use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use rusqlite::params;
use rusqlite::Connection;
use rusqlite::Transaction;
use serde_json::Map;
use serde_json::Value;

use crate::app_paths;
use crate::utils::set_private_permissions;

const CODEX_STATE_DB_NAME: &str = "state_5.sqlite";
const CODEX_GLOBAL_STATE_NAME: &str = ".codex-global-state.json";
const CODEX_GLOBAL_STATE_BACKUP_NAME: &str = ".codex-global-state.json.bak";
const CHATGPT_PROVIDER_ID: &str = "openai";
const PROVIDER_SYNC_NAMESPACE: &str = "provider-sync";
const PROVIDER_SYNC_BACKUP_PREFIX: &str = "state_5.sqlite.provider-sync-";
const PROVIDER_SYNC_BACKUP_SUFFIX: &str = ".bak";
const MANAGED_BACKUP_PREFIX: &str = "provider-sync-";
const MAX_PROVIDER_SYNC_BACKUPS: usize = 1;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];

pub(crate) fn sync_codex_thread_providers_for_relay(
    provider_id: &str,
    backup_dir: &Path,
) -> Result<usize, String> {
    sync_codex_thread_providers(provider_id, backup_dir)
}

pub(crate) fn sync_codex_thread_providers_for_chatgpt(backup_dir: &Path) -> Result<usize, String> {
    sync_codex_thread_providers(CHATGPT_PROVIDER_ID, backup_dir)
}

fn sync_codex_thread_providers(provider_id: &str, backup_dir: &Path) -> Result<usize, String> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return Ok(0);
    }

    let codex_home = app_paths::codex_dir()?;
    sync_codex_thread_visibility_in_home(provider_id, &codex_home, backup_dir)
}

fn sync_codex_thread_visibility_in_home(
    provider_id: &str,
    codex_home: &Path,
    backup_dir: &Path,
) -> Result<usize, String> {
    let session_scan = collect_session_changes(codex_home, provider_id)?;
    let db_path = codex_home.join(CODEX_STATE_DB_NAME);
    let sqlite_pending = count_sqlite_visibility_changes(
        &db_path,
        provider_id,
        &session_scan.user_event_thread_ids,
        &session_scan.thread_cwd_by_id,
    )?;
    let workspace_plan = plan_workspace_roots_sync(codex_home, &session_scan.thread_cwd_by_id)?;
    let pending_changes = session_scan.changes.len()
        + sqlite_pending.total()
        + usize::from(workspace_plan.as_ref().is_some_and(|plan| plan.changed));

    if pending_changes == 0 {
        return Ok(0);
    }

    let backup_path =
        create_provider_sync_backup(codex_home, backup_dir, provider_id, &session_scan.changes)?;
    log::info!(
        "已创建 Codex 线程可见性备份 {}，待修复 {} 项",
        backup_path.display(),
        pending_changes
    );

    assert_session_changes_writable(&session_scan.changes)?;
    let apply_result = apply_visibility_changes(
        codex_home,
        &db_path,
        provider_id,
        &session_scan.user_event_thread_ids,
        &session_scan.thread_cwd_by_id,
        &session_scan.changes,
        workspace_plan.as_ref(),
    )?;

    if let Err(error) = cleanup_codex_state_provider_backups(backup_dir) {
        log::warn!("清理旧 Codex 线程 provider 备份失败: {error}");
    }
    if let Err(error) = cleanup_legacy_codex_state_provider_backups() {
        log::warn!("清理旧版 ~/.codex 线程 provider 备份失败: {error}");
    }

    let changed = apply_result.sqlite.total()
        + apply_result.session.applied_changes
        + usize::from(apply_result.workspace_changed);
    Ok(changed)
}

#[derive(Clone, Debug)]
struct FirstLineRecord {
    first_line: String,
    separator: String,
    offset: u64,
}

#[derive(Clone, Debug)]
struct SessionChange {
    path: PathBuf,
    original_first_line: String,
    original_separator: String,
    original_offset: u64,
    updated_first_line: String,
}

#[derive(Debug, Default)]
struct SessionScan {
    changes: Vec<SessionChange>,
    user_event_thread_ids: HashSet<String>,
    thread_cwd_by_id: HashMap<String, String>,
}

#[derive(Debug, Default)]
struct SqliteChangeCounts {
    provider_rows: usize,
    user_event_rows: usize,
    cwd_rows: usize,
}

impl SqliteChangeCounts {
    fn total(&self) -> usize {
        self.provider_rows + self.user_event_rows + self.cwd_rows
    }
}

#[derive(Debug, Default)]
struct SessionApplyResult {
    applied_changes: usize,
    applied: Vec<SessionChange>,
}

#[derive(Debug, Default)]
struct VisibilityApplyResult {
    sqlite: SqliteChangeCounts,
    session: SessionApplyResult,
    workspace_changed: bool,
}

#[derive(Debug)]
struct WorkspaceRootsPlan {
    changed: bool,
    original_text: String,
    next_text: String,
}

fn collect_session_changes(codex_home: &Path, provider_id: &str) -> Result<SessionScan, String> {
    let mut scan = SessionScan::default();

    for dir_name in SESSION_DIRS {
        let root = codex_home.join(dir_name);
        if !root.exists() {
            continue;
        }
        let rollout_files = list_rollout_files(&root)?;
        for rollout_path in rollout_files {
            let record = read_first_line_record(&rollout_path)?;
            let Some(mut parsed) = parse_session_meta_record(&record.first_line) else {
                continue;
            };
            let Some(payload) = parsed.get_mut("payload").and_then(Value::as_object_mut) else {
                continue;
            };
            let thread_id = payload
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);

            if let (Some(thread_id), Some(cwd)) = (
                thread_id.as_ref(),
                payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty()),
            ) {
                scan.thread_cwd_by_id
                    .insert(thread_id.clone(), to_desktop_workspace_path(cwd));
            }

            if let Some(thread_id) = thread_id.as_ref() {
                if file_has_user_event(&rollout_path, &record.first_line, record.offset)? {
                    scan.user_event_thread_ids.insert(thread_id.clone());
                }
            }

            let current_provider = payload.get("model_provider").and_then(Value::as_str);
            if current_provider != Some(provider_id) {
                payload.insert(
                    "model_provider".to_string(),
                    Value::String(provider_id.to_string()),
                );
                let updated_first_line = serde_json::to_string(&parsed).map_err(|error| {
                    format!(
                        "序列化 Codex rollout 元数据失败 {}: {error}",
                        rollout_path.display()
                    )
                })?;
                scan.changes.push(SessionChange {
                    path: rollout_path,
                    original_first_line: record.first_line,
                    original_separator: record.separator,
                    original_offset: record.offset,
                    updated_first_line,
                });
            }
        }
    }

    scan.changes
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(scan)
}

fn list_rollout_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "读取 Codex 会话目录失败 {}: {error}",
                    dir.display()
                ))
            }
        };

        for entry in entries {
            let entry = entry.map_err(|error| format!("读取 Codex 会话目录条目失败: {error}"))?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                format!("读取 Codex 会话文件类型失败 {}: {error}", path.display())
            })?;
            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with("rollout-") && file_name.ends_with(".jsonl") {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn read_first_line_record(path: &Path) -> Result<FirstLineRecord, String> {
    let file = File::open(path)
        .map_err(|error| format!("读取 Codex rollout 文件失败 {}: {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    let read = reader
        .read_until(b'\n', &mut bytes)
        .map_err(|error| format!("读取 Codex rollout 首行失败 {}: {error}", path.display()))?;

    if read == 0 {
        return Ok(FirstLineRecord {
            first_line: String::new(),
            separator: String::new(),
            offset: 0,
        });
    }

    let offset = bytes.len() as u64;
    let separator = if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len().saturating_sub(2));
        "\r\n".to_string()
    } else if bytes.ends_with(b"\n") {
        bytes.truncate(bytes.len().saturating_sub(1));
        "\n".to_string()
    } else {
        String::new()
    };
    let first_line = String::from_utf8(bytes)
        .map_err(|error| format!("Codex rollout 首行不是 UTF-8 {}: {error}", path.display()))?;

    Ok(FirstLineRecord {
        first_line,
        separator,
        offset,
    })
}

fn parse_session_meta_record(line: &str) -> Option<Value> {
    let parsed: Value = serde_json::from_str(line).ok()?;
    if parsed.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    if !parsed.get("payload").is_some_and(Value::is_object) {
        return None;
    }
    Some(parsed)
}

fn file_has_user_event(path: &Path, first_line: &str, offset: u64) -> Result<bool, String> {
    if line_has_user_event(first_line) {
        return Ok(true);
    }

    let mut file = File::open(path)
        .map_err(|error| format!("读取 Codex rollout 文件失败 {}: {error}", path.display()))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| format!("跳转 Codex rollout 文件失败 {}: {error}", path.display()))?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line.map_err(|error| {
            format!(
                "扫描 Codex rollout 用户消息失败 {}: {error}",
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        if line_has_user_event(&line) {
            return Ok(true);
        }
    }

    Ok(false)
}

fn line_has_user_event(line: &str) -> bool {
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return false;
    };

    if record.get("type").and_then(Value::as_str) == Some("event_msg")
        && record
            .get("payload")
            .and_then(|payload| payload.get("type"))
            .and_then(Value::as_str)
            == Some("user_message")
    {
        return true;
    }

    for key in ["payload", "item", "msg"] {
        let Some(value) = record.get(key) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("message")
            && value.get("role").and_then(Value::as_str) == Some("user")
        {
            return true;
        }
    }

    false
}

fn assert_session_changes_writable(changes: &[SessionChange]) -> Result<(), String> {
    for change in changes {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&change.path)
            .map_err(|error| {
                format!(
                    "Codex rollout 文件正在被占用或不可写，请关闭 Codex 后重试 {}: {error}",
                    change.path.display()
                )
            })?;
    }
    Ok(())
}

fn apply_session_changes(changes: &[SessionChange]) -> Result<SessionApplyResult, String> {
    let mut applied = 0usize;
    let mut applied_changes = Vec::new();
    for change in changes {
        let current = match read_first_line_record(&change.path) {
            Ok(current) => current,
            Err(error) => {
                restore_session_changes(&applied_changes);
                return Err(error);
            }
        };
        if current.first_line != change.original_first_line
            || current.offset != change.original_offset
        {
            log::warn!("跳过已变化的 Codex rollout 文件 {}", change.path.display());
            continue;
        }
        if let Err(error) = rewrite_first_line(change) {
            restore_session_changes(&applied_changes);
            return Err(error);
        }
        applied += 1;
        applied_changes.push(change.clone());
    }

    Ok(SessionApplyResult {
        applied_changes: applied,
        applied: applied_changes,
    })
}

fn restore_session_changes(changes: &[SessionChange]) {
    for change in changes.iter().rev() {
        let restore = SessionChange {
            path: change.path.clone(),
            original_first_line: change.updated_first_line.clone(),
            original_separator: change.original_separator.clone(),
            original_offset: change.updated_first_line.len() as u64
                + change.original_separator.len() as u64,
            updated_first_line: change.original_first_line.clone(),
        };
        if let Err(error) = rewrite_first_line(&restore) {
            log::warn!(
                "回滚 Codex rollout 文件失败 {}: {error}",
                change.path.display()
            );
        }
    }
}

fn rewrite_first_line(change: &SessionChange) -> Result<(), String> {
    let timestamp = current_timestamp_millis()?;
    let tmp_path = change.path.with_extension(format!(
        "{}.provider-sync.{}.tmp",
        change
            .path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("jsonl"),
        timestamp
    ));
    let rollback_path = change.path.with_extension(format!(
        "{}.provider-sync.{}.rollback",
        change
            .path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("jsonl"),
        timestamp
    ));
    let mut source = File::open(&change.path).map_err(|error| {
        format!(
            "读取 Codex rollout 文件失败 {}: {error}",
            change.path.display()
        )
    })?;
    source
        .seek(SeekFrom::Start(change.original_offset))
        .map_err(|error| {
            format!(
                "跳转 Codex rollout 文件失败 {}: {error}",
                change.path.display()
            )
        })?;

    {
        let mut target = File::create(&tmp_path).map_err(|error| {
            format!(
                "创建 Codex rollout 临时文件失败 {}: {error}",
                tmp_path.display()
            )
        })?;
        target
            .write_all(change.updated_first_line.as_bytes())
            .map_err(|error| format!("写入 Codex rollout 首行失败: {error}"))?;
        target
            .write_all(change.original_separator.as_bytes())
            .map_err(|error| format!("写入 Codex rollout 换行符失败: {error}"))?;
        io::copy(&mut source, &mut target)
            .map_err(|error| format!("复制 Codex rollout 剩余内容失败: {error}"))?;
        target
            .sync_all()
            .map_err(|error| format!("同步 Codex rollout 临时文件失败: {error}"))?;
    }

    replace_file_with_rollback(&tmp_path, &change.path, &rollback_path).map_err(|error| {
        let _ = fs::remove_file(&tmp_path);
        let _ = fs::remove_file(&rollback_path);
        error
    })?;
    let _ = fs::remove_file(&tmp_path);
    Ok(())
}

fn replace_file_with_rollback(
    source_path: &Path,
    destination_path: &Path,
    rollback_path: &Path,
) -> Result<(), String> {
    if rollback_path.exists() {
        fs::remove_file(rollback_path).map_err(|error| {
            format!(
                "清理 Codex rollout 回滚文件失败 {}: {error}",
                rollback_path.display()
            )
        })?;
    }

    fs::rename(destination_path, rollback_path).map_err(|error| {
        format!(
            "准备替换 Codex rollout 文件失败 {}: {error}",
            destination_path.display()
        )
    })?;

    if let Err(error) = fs::rename(source_path, destination_path) {
        let restore_result = fs::rename(rollback_path, destination_path);
        return Err(match restore_result {
            Ok(_) => format!(
                "替换 Codex rollout 文件失败 {}: {error}",
                destination_path.display()
            ),
            Err(restore_error) => format!(
                "替换 Codex rollout 文件失败且回滚失败 {}: {error}; 回滚错误: {restore_error}",
                destination_path.display()
            ),
        });
    }

    if let Err(error) = fs::remove_file(rollback_path) {
        log::warn!(
            "删除 Codex rollout 回滚文件失败 {}: {error}",
            rollback_path.display()
        );
    }
    Ok(())
}

fn count_sqlite_visibility_changes(
    db_path: &Path,
    provider_id: &str,
    user_event_thread_ids: &HashSet<String>,
    thread_cwd_by_id: &HashMap<String, String>,
) -> Result<SqliteChangeCounts, String> {
    if !db_path.is_file() {
        return Ok(SqliteChangeCounts::default());
    }

    let connection = open_state_db(db_path)?;
    if !table_exists(&connection, "threads")? {
        return Ok(SqliteChangeCounts::default());
    }

    let provider_rows = if table_has_column(&connection, "threads", "model_provider")? {
        count_model_provider_rows(&connection, provider_id)?
    } else {
        0
    };
    let user_event_rows = if table_has_column(&connection, "threads", "has_user_event")? {
        count_user_event_rows(&connection, user_event_thread_ids)?
    } else {
        0
    };
    let cwd_rows = if table_has_column(&connection, "threads", "cwd")? {
        count_cwd_rows(&connection, thread_cwd_by_id)?
    } else {
        0
    };

    Ok(SqliteChangeCounts {
        provider_rows,
        user_event_rows,
        cwd_rows,
    })
}

fn apply_visibility_changes(
    codex_home: &Path,
    db_path: &Path,
    provider_id: &str,
    user_event_thread_ids: &HashSet<String>,
    thread_cwd_by_id: &HashMap<String, String>,
    session_changes: &[SessionChange],
    workspace_plan: Option<&WorkspaceRootsPlan>,
) -> Result<VisibilityApplyResult, String> {
    if !db_path.is_file() {
        let session = apply_session_changes(session_changes)?;
        let workspace_changed = match apply_workspace_roots_plan(codex_home, workspace_plan) {
            Ok(changed) => changed,
            Err(error) => {
                restore_session_changes(&session.applied);
                return Err(error);
            }
        };
        return Ok(VisibilityApplyResult {
            sqlite: SqliteChangeCounts::default(),
            session,
            workspace_changed,
        });
    }

    let mut connection = open_state_db(db_path)?;
    if !table_exists(&connection, "threads")? {
        let session = apply_session_changes(session_changes)?;
        let workspace_changed = match apply_workspace_roots_plan(codex_home, workspace_plan) {
            Ok(changed) => changed,
            Err(error) => {
                restore_session_changes(&session.applied);
                return Err(error);
            }
        };
        return Ok(VisibilityApplyResult {
            sqlite: SqliteChangeCounts::default(),
            session,
            workspace_changed,
        });
    }

    let has_provider = table_has_column(&connection, "threads", "model_provider")?;
    let has_user_event = table_has_column(&connection, "threads", "has_user_event")?;
    let has_cwd = table_has_column(&connection, "threads", "cwd")?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开启 Codex 线程数据库事务失败: {error}"))?;
    let sqlite = apply_sqlite_visibility_changes_in_transaction(
        &transaction,
        provider_id,
        user_event_thread_ids,
        thread_cwd_by_id,
        has_provider,
        has_user_event,
        has_cwd,
    )?;
    let session = apply_session_changes(session_changes)?;
    let workspace_changed = match apply_workspace_roots_plan(codex_home, workspace_plan) {
        Ok(changed) => changed,
        Err(error) => {
            restore_session_changes(&session.applied);
            return Err(error);
        }
    };
    if let Err(error) = transaction.commit() {
        restore_session_changes(&session.applied);
        if let Err(restore_error) = restore_workspace_roots_plan(codex_home, workspace_plan) {
            log::warn!("回滚 Codex 全局状态失败: {restore_error}");
        }
        return Err(format!("提交 Codex 线程数据库事务失败: {error}"));
    }

    Ok(VisibilityApplyResult {
        sqlite,
        session,
        workspace_changed,
    })
}

fn apply_sqlite_visibility_changes_in_transaction(
    transaction: &Transaction<'_>,
    provider_id: &str,
    user_event_thread_ids: &HashSet<String>,
    thread_cwd_by_id: &HashMap<String, String>,
    has_provider: bool,
    has_user_event: bool,
    has_cwd: bool,
) -> Result<SqliteChangeCounts, String> {
    let provider_rows = if has_provider {
        transaction
            .execute(
                "UPDATE threads SET model_provider = ?1 WHERE COALESCE(model_provider, '') <> ?1",
                params![provider_id],
            )
            .map_err(|error| format!("同步 Codex 线程 provider 失败: {error}"))?
    } else {
        0
    };

    let mut user_event_rows = 0usize;
    if has_user_event {
        let mut statement = transaction
            .prepare(
                "UPDATE threads SET has_user_event = 1 WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
            )
            .map_err(|error| format!("准备 Codex 线程用户消息修复语句失败: {error}"))?;
        for thread_id in user_event_thread_ids {
            user_event_rows += statement
                .execute(params![thread_id])
                .map_err(|error| format!("修复 Codex 线程用户消息标记失败: {error}"))?;
        }
    }

    let mut cwd_rows = 0usize;
    if has_cwd {
        let mut statement = transaction
            .prepare("UPDATE threads SET cwd = ?1 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1")
            .map_err(|error| format!("准备 Codex 线程工作目录修复语句失败: {error}"))?;
        for (thread_id, cwd) in thread_cwd_by_id {
            if thread_id.trim().is_empty() || cwd.trim().is_empty() {
                continue;
            }
            cwd_rows += statement
                .execute(params![cwd, thread_id])
                .map_err(|error| format!("修复 Codex 线程工作目录失败: {error}"))?;
        }
    }

    Ok(SqliteChangeCounts {
        provider_rows,
        user_event_rows,
        cwd_rows,
    })
}

fn open_state_db(db_path: &Path) -> Result<Connection, String> {
    let connection = Connection::open(db_path)
        .map_err(|error| format!("打开 Codex 线程数据库失败 {}: {error}", db_path.display()))?;
    connection
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(|error| format!("设置 Codex 线程数据库等待锁超时失败: {error}"))?;
    Ok(connection)
}

fn count_model_provider_rows(connection: &Connection, provider_id: &str) -> Result<usize, String> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE COALESCE(model_provider, '') <> ?1",
            params![provider_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count.max(0) as usize)
        .map_err(|error| format!("统计待同步 Codex 线程 provider 失败: {error}"))
}

fn count_user_event_rows(
    connection: &Connection,
    user_event_thread_ids: &HashSet<String>,
) -> Result<usize, String> {
    let mut statement = connection
        .prepare("SELECT has_user_event FROM threads WHERE id = ?1")
        .map_err(|error| format!("准备 Codex 线程用户消息统计语句失败: {error}"))?;
    let mut pending = 0usize;
    for thread_id in user_event_thread_ids {
        let value = statement
            .query_row(params![thread_id], |row| row.get::<_, Option<i64>>(0))
            .unwrap_or(None);
        if value.unwrap_or(0) != 1 {
            pending += 1;
        }
    }
    Ok(pending)
}

fn count_cwd_rows(
    connection: &Connection,
    thread_cwd_by_id: &HashMap<String, String>,
) -> Result<usize, String> {
    let mut statement = connection
        .prepare("SELECT cwd FROM threads WHERE id = ?1")
        .map_err(|error| format!("准备 Codex 线程工作目录统计语句失败: {error}"))?;
    let mut pending = 0usize;
    for (thread_id, cwd) in thread_cwd_by_id {
        if thread_id.trim().is_empty() || cwd.trim().is_empty() {
            continue;
        }
        let value = statement
            .query_row(params![thread_id], |row| row.get::<_, Option<String>>(0))
            .unwrap_or(None);
        if value.as_deref().unwrap_or_default() != cwd {
            pending += 1;
        }
    }
    Ok(pending)
}

fn table_exists(connection: &Connection, table_name: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(|error| format!("读取 Codex 线程数据库表失败: {error}"))
}

fn table_has_column(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, String> {
    let mut statement = connection
        .prepare(&format!(
            "PRAGMA table_info({})",
            quote_sql_identifier(table_name)
        ))
        .map_err(|error| format!("读取 Codex 线程表结构失败: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("读取 Codex 线程字段失败: {error}"))?;

    for row in rows {
        let column = row.map_err(|error| format!("读取 Codex 线程字段失败: {error}"))?;
        if column == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn quote_sql_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn plan_workspace_roots_sync(
    codex_home: &Path,
    thread_cwd_by_id: &HashMap<String, String>,
) -> Result<Option<WorkspaceRootsPlan>, String> {
    let path = codex_home.join(CODEX_GLOBAL_STATE_NAME);
    if !path.is_file() {
        return Ok(None);
    }

    let original_text = fs::read_to_string(&path)
        .map_err(|error| format!("读取 Codex 全局状态失败 {}: {error}", path.display()))?;
    let mut state: Value = serde_json::from_str(&original_text)
        .map_err(|error| format!("解析 Codex 全局状态失败 {}: {error}", path.display()))?;
    let Some(object) = state.as_object_mut() else {
        return Ok(None);
    };

    let cwd_stats = build_cwd_stats(thread_cwd_by_id.values());
    let existing_saved_roots = path_array(object.get("electron-saved-workspace-roots"));
    let existing_project_order = path_array(object.get("project-order"));
    let existing_active_roots = path_array(object.get("active-workspace-roots"));

    let next_saved_roots = dedupe_paths(
        if existing_project_order.is_empty() {
            existing_saved_roots
                .iter()
                .chain(existing_active_roots.iter())
                .cloned()
                .collect()
        } else {
            existing_project_order
                .iter()
                .chain(existing_saved_roots.iter())
                .chain(existing_active_roots.iter())
                .cloned()
                .collect()
        },
        &cwd_stats,
    );
    let next_project_order = dedupe_paths(
        if existing_project_order.is_empty() {
            next_saved_roots.clone()
        } else {
            existing_project_order
                .iter()
                .chain(existing_saved_roots.iter())
                .cloned()
                .collect()
        },
        &cwd_stats,
    );
    let next_active_roots = dedupe_paths(existing_active_roots.clone(), &cwd_stats);
    let original_active = object.get("active-workspace-roots").cloned();
    let next_active_value = match original_active.as_ref() {
        Some(Value::Array(_)) => strings_value(next_active_roots.clone()),
        Some(_) => next_active_roots
            .first()
            .map(|value| Value::String(value.clone()))
            .unwrap_or_else(|| original_active.clone().unwrap_or(Value::Null)),
        None => Value::Null,
    };
    let next_labels = resolve_object_keys(object.get("electron-workspace-root-labels"), &cwd_stats);
    let next_open_targets =
        resolve_open_target_preferences(object.get("open-in-target-preferences"), &cwd_stats);

    let mut changed = false;
    changed |= object.get("electron-saved-workspace-roots")
        != Some(&strings_value(next_saved_roots.clone()));
    changed |= object.get("project-order") != Some(&strings_value(next_project_order.clone()));
    changed |= original_active.as_ref() != Some(&next_active_value);
    if let Some(next_labels) = next_labels.as_ref() {
        changed |= object.get("electron-workspace-root-labels") != Some(next_labels);
    }
    if let Some(next_open_targets) = next_open_targets.as_ref() {
        changed |= object.get("open-in-target-preferences") != Some(next_open_targets);
    }

    let backup_missing = !codex_home.join(CODEX_GLOBAL_STATE_BACKUP_NAME).is_file();
    if !changed && !backup_missing {
        return Ok(None);
    }

    object.insert(
        "electron-saved-workspace-roots".to_string(),
        strings_value(next_saved_roots),
    );
    object.insert(
        "project-order".to_string(),
        strings_value(next_project_order),
    );
    if !next_active_value.is_null() {
        object.insert("active-workspace-roots".to_string(), next_active_value);
    }
    if let Some(next_labels) = next_labels {
        object.insert("electron-workspace-root-labels".to_string(), next_labels);
    }
    if let Some(next_open_targets) = next_open_targets {
        object.insert("open-in-target-preferences".to_string(), next_open_targets);
    }

    let next_text = format!(
        "{}\n",
        serde_json::to_string_pretty(&state)
            .map_err(|error| format!("序列化 Codex 全局状态失败: {error}"))?
    );

    Ok(Some(WorkspaceRootsPlan {
        changed,
        original_text,
        next_text,
    }))
}

fn apply_workspace_roots_plan(
    codex_home: &Path,
    plan: Option<&WorkspaceRootsPlan>,
) -> Result<bool, String> {
    let Some(plan) = plan else {
        return Ok(false);
    };

    let path = codex_home.join(CODEX_GLOBAL_STATE_NAME);
    let backup_path = codex_home.join(CODEX_GLOBAL_STATE_BACKUP_NAME);
    fs::write(&path, &plan.next_text)
        .map_err(|error| format!("写入 Codex 全局状态失败 {}: {error}", path.display()))?;
    fs::write(&backup_path, &plan.next_text).map_err(|error| {
        format!(
            "写入 Codex 全局状态备份失败 {}: {error}",
            backup_path.display()
        )
    })?;
    Ok(plan.changed)
}

fn restore_workspace_roots_plan(
    codex_home: &Path,
    plan: Option<&WorkspaceRootsPlan>,
) -> Result<(), String> {
    let Some(plan) = plan else {
        return Ok(());
    };
    let path = codex_home.join(CODEX_GLOBAL_STATE_NAME);
    fs::write(&path, &plan.original_text)
        .map_err(|error| format!("回滚 Codex 全局状态失败 {}: {error}", path.display()))
}

#[derive(Clone, Debug)]
struct CwdStat {
    cwd: String,
    normalized_cwd: String,
    count: usize,
}

fn build_cwd_stats<'a>(values: impl Iterator<Item = &'a String>) -> Vec<CwdStat> {
    let mut by_path: HashMap<String, CwdStat> = HashMap::new();
    for value in values {
        let Some(normalized_cwd) = normalize_comparable_path(value) else {
            continue;
        };
        let entry = by_path.entry(normalized_cwd.clone()).or_insert(CwdStat {
            cwd: to_desktop_workspace_path(value),
            normalized_cwd,
            count: 0,
        });
        entry.count += 1;
    }
    by_path.into_values().collect()
}

fn path_array(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        Some(Value::String(value)) if !value.trim().is_empty() => vec![value.trim().to_string()],
        _ => Vec::new(),
    }
}

fn strings_value(values: Vec<String>) -> Value {
    Value::Array(values.into_iter().map(Value::String).collect())
}

fn dedupe_paths(values: Vec<String>, cwd_stats: &[CwdStat]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let resolved = resolve_stored_path(&value, cwd_stats);
        let Some(comparable) = normalize_comparable_path(&resolved) else {
            continue;
        };
        if seen.insert(comparable) {
            result.push(resolved);
        }
    }
    result
}

fn resolve_object_keys(value: Option<&Value>, cwd_stats: &[CwdStat]) -> Option<Value> {
    let Value::Object(object) = value? else {
        return value.cloned();
    };

    let mut next = Map::new();
    for (key, value) in object {
        let resolved = resolve_stored_path(key, cwd_stats);
        if !next.contains_key(&resolved) || resolved == *key {
            next.insert(resolved, value.clone());
        }
    }
    Some(Value::Object(next))
}

fn resolve_open_target_preferences(value: Option<&Value>, cwd_stats: &[CwdStat]) -> Option<Value> {
    let Value::Object(object) = value? else {
        return value.cloned();
    };

    let mut next = object.clone();
    if let Some(per_path) = object.get("perPath") {
        if let Some(resolved) = resolve_object_keys(Some(per_path), cwd_stats) {
            next.insert("perPath".to_string(), resolved);
        }
    }
    Some(Value::Object(next))
}

fn resolve_stored_path(value: &str, cwd_stats: &[CwdStat]) -> String {
    let Some(comparable) = normalize_comparable_path(value) else {
        return value.to_string();
    };

    let mut matches: Vec<&CwdStat> = cwd_stats
        .iter()
        .filter(|entry| entry.normalized_cwd == comparable)
        .collect();
    if matches.is_empty() {
        return to_desktop_workspace_path(value);
    }

    matches.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.cwd.cmp(&right.cwd))
    });
    to_desktop_workspace_path(&matches[0].cwd)
}

fn normalize_comparable_path(value: &str) -> Option<String> {
    let mut normalized = value.trim().to_string();
    if normalized.is_empty() {
        return None;
    }

    if let Some(rest) = normalized.strip_prefix(r"\\?\UNC\") {
        normalized = format!(r"\\{rest}");
    } else if let Some(rest) = normalized.strip_prefix(r"\\?\") {
        normalized = rest.to_string();
    }

    normalized = normalized.replace('/', r"\");
    while normalized.ends_with('\\') && !is_drive_root(&normalized) {
        normalized.pop();
    }
    if normalized.len() == 2 && normalized.as_bytes()[1] == b':' {
        normalized.push('\\');
    }
    Some(normalized.to_lowercase())
}

fn is_drive_root(value: &str) -> bool {
    value.len() == 3
        && value.as_bytes()[1] == b':'
        && (value.as_bytes()[2] == b'\\' || value.as_bytes()[2] == b'/')
}

fn to_desktop_workspace_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return value.to_string();
    }

    let upper = trimmed.to_uppercase();
    if upper.starts_with(r"\\?\UNC\") {
        return format!(r"\\{}", trimmed[8..].replace('/', r"\"));
    }
    if upper.starts_with(r"\\?\") {
        return trimmed[4..].replace('/', r"\");
    }
    trimmed.to_string()
}

fn create_provider_sync_backup(
    codex_home: &Path,
    backup_root: &Path,
    target_provider: &str,
    session_changes: &[SessionChange],
) -> Result<PathBuf, String> {
    fs::create_dir_all(backup_root).map_err(|error| {
        format!(
            "创建 Codex 线程备份目录失败 {}: {error}",
            backup_root.display()
        )
    })?;
    let timestamp = current_timestamp_millis()?;
    let backup_dir = backup_root.join(format!("{MANAGED_BACKUP_PREFIX}{timestamp}"));
    let db_dir = backup_dir.join("db");
    fs::create_dir_all(&db_dir).map_err(|error| {
        format!(
            "创建 Codex 线程数据库备份目录失败 {}: {error}",
            db_dir.display()
        )
    })?;

    let mut db_files = Vec::new();
    for suffix in ["", "-shm", "-wal"] {
        let file_name = format!("{CODEX_STATE_DB_NAME}{suffix}");
        let source = codex_home.join(&file_name);
        if source.is_file() {
            let destination = db_dir.join(&file_name);
            fs::copy(&source, &destination).map_err(|error| {
                format!(
                    "备份 Codex 线程数据库失败 {} -> {}: {error}",
                    source.display(),
                    destination.display()
                )
            })?;
            set_private_permissions(&destination);
            db_files.push(file_name);
        }
    }

    for file_name in [CODEX_GLOBAL_STATE_NAME, CODEX_GLOBAL_STATE_BACKUP_NAME] {
        let source = codex_home.join(file_name);
        if source.is_file() {
            let destination = backup_dir.join(file_name);
            fs::copy(&source, &destination).map_err(|error| {
                format!(
                    "备份 Codex 全局状态失败 {} -> {}: {error}",
                    source.display(),
                    destination.display()
                )
            })?;
            set_private_permissions(&destination);
        }
    }

    let session_manifest = serde_json::json!({
        "version": 1,
        "namespace": PROVIDER_SYNC_NAMESPACE,
        "codexHome": codex_home.to_string_lossy(),
        "targetProvider": target_provider,
        "createdAtMs": timestamp,
        "files": session_changes.iter().map(|change| {
            serde_json::json!({
                "path": change.path.to_string_lossy(),
                "originalFirstLine": change.original_first_line,
                "originalSeparator": change.original_separator,
            })
        }).collect::<Vec<_>>(),
    });
    fs::write(
        backup_dir.join("session-meta-backup.json"),
        serde_json::to_string_pretty(&session_manifest)
            .map_err(|error| format!("序列化 Codex 会话备份清单失败: {error}"))?,
    )
    .map_err(|error| format!("写入 Codex 会话备份清单失败: {error}"))?;

    let metadata = serde_json::json!({
        "version": 1,
        "namespace": PROVIDER_SYNC_NAMESPACE,
        "codexHome": codex_home.to_string_lossy(),
        "targetProvider": target_provider,
        "createdAtMs": timestamp,
        "dbFiles": db_files,
        "changedSessionFiles": session_changes.len(),
    });
    fs::write(
        backup_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata)
            .map_err(|error| format!("序列化 Codex 线程备份元数据失败: {error}"))?,
    )
    .map_err(|error| format!("写入 Codex 线程备份元数据失败: {error}"))?;

    set_private_permissions(&backup_dir);
    Ok(backup_dir)
}

pub(crate) fn cleanup_codex_state_provider_backups(backup_dir: &Path) -> Result<usize, String> {
    cleanup_codex_state_provider_backups_keep(backup_dir, MAX_PROVIDER_SYNC_BACKUPS)
}

pub(crate) fn cleanup_legacy_codex_state_provider_backups() -> Result<usize, String> {
    let codex_dir = app_paths::codex_dir()?;
    let mut removed = cleanup_codex_state_provider_backups_keep(&codex_dir, 0)?;
    let cockpit_backup_dir = codex_dir
        .join("backups_state")
        .join(PROVIDER_SYNC_NAMESPACE);
    removed += cleanup_codex_state_provider_backups_keep(&cockpit_backup_dir, 0)?;
    remove_empty_dir_best_effort(&cockpit_backup_dir);
    remove_empty_dir_best_effort(&codex_dir.join("backups_state"));
    Ok(removed)
}

fn cleanup_codex_state_provider_backups_keep(
    backup_dir: &Path,
    keep_latest: usize,
) -> Result<usize, String> {
    if !backup_dir.exists() {
        return Ok(0);
    }
    if !backup_dir.is_dir() {
        return Err(format!(
            "Codex 线程备份路径不是目录 {}",
            backup_dir.display()
        ));
    }

    let mut backups = Vec::new();
    for entry in fs::read_dir(backup_dir).map_err(|error| {
        format!(
            "读取 Codex 线程备份目录失败 {}: {error}",
            backup_dir.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "读取 Codex 线程备份目录条目失败 {}: {error}",
                backup_dir.display()
            )
        })?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() && is_managed_backup_dir(&path) {
            if let Some(timestamp) = managed_backup_timestamp(&file_name) {
                backups.push((timestamp, path, true));
            }
            continue;
        }
        if path.is_file() {
            if let Some(timestamp) = legacy_provider_sync_backup_timestamp(&file_name) {
                backups.push((timestamp, path, false));
            }
        }
    }

    if backups.len() <= keep_latest {
        return Ok(0);
    }

    backups.sort_by_key(|(timestamp, _, _)| *timestamp);
    let remove_count = backups.len().saturating_sub(keep_latest);
    let mut removed = 0usize;
    for (_, path, is_dir) in backups.into_iter().take(remove_count) {
        if is_dir {
            fs::remove_dir_all(&path).map_err(|error| {
                format!("删除旧 Codex 线程备份失败 {}: {error}", path.display())
            })?;
        } else {
            fs::remove_file(&path).map_err(|error| {
                format!("删除旧 Codex 线程备份失败 {}: {error}", path.display())
            })?;
        }
        removed += 1;
    }
    Ok(removed)
}

fn is_managed_backup_dir(path: &Path) -> bool {
    let metadata_path = path.join("metadata.json");
    let Ok(text) = fs::read_to_string(metadata_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    value.get("namespace").and_then(Value::as_str) == Some(PROVIDER_SYNC_NAMESPACE)
}

fn managed_backup_timestamp(file_name: &str) -> Option<u128> {
    file_name
        .strip_prefix(MANAGED_BACKUP_PREFIX)?
        .parse::<u128>()
        .ok()
}

fn legacy_provider_sync_backup_timestamp(file_name: &str) -> Option<u128> {
    file_name
        .strip_prefix(PROVIDER_SYNC_BACKUP_PREFIX)?
        .strip_suffix(PROVIDER_SYNC_BACKUP_SUFFIX)?
        .parse::<u128>()
        .ok()
}

fn remove_empty_dir_best_effort(path: &Path) {
    if !path.is_dir() {
        return;
    }
    let Ok(mut entries) = fs::read_dir(path) else {
        return;
    };
    if entries.next().is_none() {
        let _ = fs::remove_dir(path);
    }
}

fn current_timestamp_millis() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| format!("读取系统时间失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::cleanup_codex_state_provider_backups_keep;
    use super::sync_codex_thread_visibility_in_home;
    use super::table_has_column;
    use super::to_desktop_workspace_path;
    use rusqlite::Connection;
    use serde_json::Value;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("codex-state-sync-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_rollout(path: &Path, id: &str, provider: &str, cwd: &str, user_event: bool) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create rollout dir");
        }
        let meta = serde_json::json!({
            "type": "session_meta",
            "payload": {
                "id": id,
                "model_provider": provider,
                "cwd": cwd
            }
        });
        let user_line = if user_event {
            serde_json::json!({
                "type": "event_msg",
                "payload": { "type": "user_message" }
            })
        } else {
            serde_json::json!({
                "type": "event_msg",
                "payload": { "type": "agent_message" }
            })
        };
        fs::write(path, format!("{meta}\n{user_line}\n")).expect("write rollout");
    }

    fn first_line_provider(path: &Path) -> String {
        let text = fs::read_to_string(path).expect("read rollout");
        let first_line = text.lines().next().expect("first line");
        let value: Value = serde_json::from_str(first_line).expect("parse first line");
        value["payload"]["model_provider"]
            .as_str()
            .expect("provider")
            .to_string()
    }

    #[test]
    fn detects_model_provider_column() {
        let connection = Connection::open_in_memory().expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .expect("create threads");

        assert!(table_has_column(&connection, "threads", "model_provider").expect("detect column"));
    }

    #[test]
    fn syncs_cockpit_visibility_fields_and_session_meta() {
        let codex_home = temp_dir();
        let backup_dir = temp_dir().join("backups");
        let db_path = codex_home.join("state_5.sqlite");
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    cwd TEXT,
                    archived INTEGER
                )",
                [],
            )
            .expect("create threads");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, has_user_event, cwd, archived)
                 VALUES
                    ('thread-one', 'openai', 0, '', 0),
                    ('thread-two', 'openai', 0, '', 1)",
                [],
            )
            .expect("insert threads");
        drop(connection);

        let extended_cwd = r"\\?\C:\Workspace\Project";
        let active_rollout = codex_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("17")
            .join("rollout-thread-one.jsonl");
        write_rollout(&active_rollout, "thread-one", "openai", extended_cwd, true);
        let archived_rollout = codex_home
            .join("archived_sessions")
            .join("2026")
            .join("05")
            .join("17")
            .join("rollout-thread-two.jsonl");
        write_rollout(
            &archived_rollout,
            "thread-two",
            "openai",
            extended_cwd,
            false,
        );
        fs::write(
            codex_home.join(".codex-global-state.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "electron-saved-workspace-roots": [extended_cwd],
                "project-order": [],
                "active-workspace-roots": extended_cwd,
                "electron-workspace-root-labels": {
                    extended_cwd: "Project"
                },
                "open-in-target-preferences": {
                    "perPath": {
                        extended_cwd: "codex"
                    }
                }
            }))
            .expect("serialize global state"),
        )
        .expect("write global state");

        let changed =
            sync_codex_thread_visibility_in_home("codexdeck_api", &codex_home, &backup_dir)
                .expect("sync visibility");

        assert!(changed >= 6);
        assert_eq!(first_line_provider(&active_rollout), "codexdeck_api");
        assert_eq!(first_line_provider(&archived_rollout), "codexdeck_api");

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let row: (String, i64, String) = connection
            .query_row(
                "SELECT model_provider, has_user_event, cwd FROM threads WHERE id = 'thread-one'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read thread");
        assert_eq!(row.0, "codexdeck_api");
        assert_eq!(row.1, 1);
        assert_eq!(row.2, r"C:\Workspace\Project");

        let global_state: Value = serde_json::from_str(
            &fs::read_to_string(codex_home.join(".codex-global-state.json"))
                .expect("read global state"),
        )
        .expect("parse global state");
        assert_eq!(
            global_state["electron-saved-workspace-roots"][0].as_str(),
            Some(r"C:\Workspace\Project")
        );
        assert!(codex_home.join(".codex-global-state.json.bak").is_file());

        let backup_entries: Vec<_> = fs::read_dir(&backup_dir)
            .expect("read backup dir")
            .filter_map(Result::ok)
            .collect();
        assert_eq!(backup_entries.len(), 1);
        assert!(backup_entries[0].path().join("db/state_5.sqlite").is_file());
        assert!(backup_entries[0]
            .path()
            .join("session-meta-backup.json")
            .is_file());
    }

    #[test]
    fn does_not_backup_when_visibility_is_already_synced() {
        let codex_home = temp_dir();
        let backup_dir = temp_dir().join("backups");
        let db_path = codex_home.join("state_5.sqlite");
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    cwd TEXT
                )",
                [],
            )
            .expect("create threads");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, has_user_event, cwd)
                 VALUES ('thread-one', 'codexdeck_api', 1, 'C:\\Workspace\\Project')",
                [],
            )
            .expect("insert thread");
        drop(connection);

        let rollout = codex_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("17")
            .join("rollout-thread-one.jsonl");
        write_rollout(
            &rollout,
            "thread-one",
            "codexdeck_api",
            r"C:\Workspace\Project",
            true,
        );

        let changed =
            sync_codex_thread_visibility_in_home("codexdeck_api", &codex_home, &backup_dir)
                .expect("sync visibility");

        assert_eq!(changed, 0);
        assert!(!backup_dir.exists());
    }

    #[test]
    fn cleanup_provider_backups_keeps_only_latest_matching_items() {
        let backup_dir = temp_dir().join("backups");
        fs::create_dir_all(&backup_dir).expect("create backup dir");
        fs::write(
            backup_dir.join("state_5.sqlite.provider-sync-200.bak"),
            "old file backup",
        )
        .expect("write file backup");
        for timestamp in [100u128, 300] {
            let dir = backup_dir.join(format!("provider-sync-{timestamp}"));
            fs::create_dir_all(&dir).expect("create managed dir");
            fs::write(
                dir.join("metadata.json"),
                serde_json::json!({ "namespace": "provider-sync" }).to_string(),
            )
            .expect("write metadata");
        }
        fs::write(backup_dir.join("notes.txt"), "keep").expect("write non backup");

        let removed =
            cleanup_codex_state_provider_backups_keep(&backup_dir, 1).expect("cleanup backups");

        assert_eq!(removed, 2);
        assert!(!backup_dir
            .join("state_5.sqlite.provider-sync-200.bak")
            .exists());
        assert!(!backup_dir.join("provider-sync-100").exists());
        assert!(backup_dir.join("provider-sync-300").exists());
        assert!(backup_dir.join("notes.txt").exists());
    }

    #[test]
    fn converts_extended_windows_paths_to_desktop_paths() {
        assert_eq!(
            to_desktop_workspace_path(r"\\?\C:\Workspace\Project"),
            r"C:\Workspace\Project"
        );
        assert_eq!(
            to_desktop_workspace_path(r"\\?\UNC\server\share"),
            r"\\server\share"
        );
    }
}
