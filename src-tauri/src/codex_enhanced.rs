use std::collections::HashSet;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use rusqlite::types::{ToSqlOutput, Value as SqlValue, ValueRef};
use rusqlite::{Connection, ToSql};
use serde::Deserialize;
use serde_json::Map;
use serde_json::{json, Value};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::app_paths;
use crate::cli;
use crate::utils::new_background_command;

const DEFAULT_DEBUG_PORT: u16 = 9229;
const CDP_RETRY_COUNT: usize = 24;
const CDP_RETRY_DELAY_MS: u64 = 500;
const CDP_COMMAND_TIMEOUT_SECS: u64 = 5;
const BRIDGE_BINDING_NAME: &str = "codexDeckNativeBridge";

pub(crate) struct EnhancedLaunchResult {
    pub(crate) launched_app_path: Option<String>,
    pub(crate) used_fallback_cli: bool,
}

#[derive(Debug, Deserialize)]
struct CdpTarget {
    #[serde(default, rename = "type")]
    target_type: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default, rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionRef {
    session_id: String,
    title: String,
}

#[derive(Debug, Clone)]
struct OwnedSqlValue(SqlValue);

impl ToSql for OwnedSqlValue {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(self.0.clone()))
    }
}

pub(crate) async fn launch_codex_with_enhancements(
    configured_codex_launch_path: Option<&str>,
    workspace_path: Option<&str>,
) -> Result<EnhancedLaunchResult, String> {
    let debug_port = select_debug_port(DEFAULT_DEBUG_PORT);
    let mut app_launch_error = None;

    if let Some(path) = cli::find_configured_codex_app_path(configured_codex_launch_path)
        .or_else(cli::find_codex_app_path)
    {
        match launch_codex_app_with_debug_port(&path, workspace_path, debug_port) {
            Ok(()) => {
                inject_enhancement_script(debug_port)
                    .await
                    .map_err(|error| format!("Codex 已启动，但增强注入失败: {error}"))?;
                return Ok(EnhancedLaunchResult {
                    launched_app_path: Some(path.to_string_lossy().to_string()),
                    used_fallback_cli: false,
                });
            }
            Err(error) => {
                log::warn!(
                    "通过增强模式启动 Codex 应用失败 {}: {}",
                    path.display(),
                    error
                );
                app_launch_error = Some(error);
            }
        }
    }

    #[cfg(target_os = "windows")]
    if cli::has_windows_store_codex_app() {
        match cli::launch_windows_store_codex_with_args(&debug_arguments(
            debug_port,
            workspace_path,
        )) {
            Ok(()) => {
                inject_enhancement_script(debug_port)
                    .await
                    .map_err(|error| format!("Codex 已启动，但增强注入失败: {error}"))?;
                return Ok(EnhancedLaunchResult {
                    launched_app_path: None,
                    used_fallback_cli: false,
                });
            }
            Err(error) => {
                log::warn!("通过 Windows Store AUMID 增强启动 Codex 失败: {error}");
                app_launch_error = Some(match app_launch_error {
                    Some(previous_error) => {
                        format!(
                            "{previous_error}；且通过 Windows Store AUMID 增强启动失败: {error}"
                        )
                    }
                    None => format!("通过 Windows Store AUMID 增强启动失败: {error}"),
                });
            }
        }
    }

    let mut cmd = cli::new_codex_command(configured_codex_launch_path)?;
    cmd.arg("app");
    append_debug_arguments(&mut cmd, debug_port, workspace_path);
    cmd.spawn().map_err(|error| {
        if let Some(app_launch_error) = app_launch_error.as_ref() {
            format!(
                "通过 Codex 应用路径增强启动失败: {app_launch_error}；且通过 codex app 增强启动失败: {error}"
            )
        } else {
            format!("未检测到本地 Codex 应用，且通过 codex app 增强启动失败: {error}")
        }
    })?;
    inject_enhancement_script(debug_port)
        .await
        .map_err(|error| format!("Codex 已启动，但增强注入失败: {error}"))?;

    Ok(EnhancedLaunchResult {
        launched_app_path: None,
        used_fallback_cli: true,
    })
}

fn launch_codex_app_with_debug_port(
    path: &Path,
    workspace_path: Option<&str>,
    debug_port: u16,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = new_background_command("open");
        cmd.arg("-na").arg(path).arg("--args");
        append_debug_arguments(&mut cmd, debug_port, workspace_path);
        cmd.spawn()
            .map_err(|error| format!("增强启动 Codex 应用失败: {error}"))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        if cli::is_windows_store_codex_path(path) {
            return cli::launch_windows_store_codex_with_args(&debug_arguments(
                debug_port,
                workspace_path,
            ));
        }

        let mut cmd = new_background_command(path);
        append_debug_arguments(&mut cmd, debug_port, workspace_path);
        cmd.spawn()
            .map_err(|error| format!("增强启动 Codex 应用失败: {error}"))?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut cmd = new_background_command(path);
        append_debug_arguments(&mut cmd, debug_port, workspace_path);
        cmd.spawn()
            .map_err(|error| format!("增强启动 Codex 应用失败: {error}"))?;
        return Ok(());
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = path;
        let _ = workspace_path;
        let _ = debug_port;
        Err("当前平台暂不支持增强启动 Codex 应用".to_string())
    }
}

fn append_debug_arguments(
    cmd: &mut std::process::Command,
    debug_port: u16,
    workspace_path: Option<&str>,
) {
    for arg in debug_arguments(debug_port, workspace_path) {
        cmd.arg(arg);
    }
}

fn debug_arguments(debug_port: u16, workspace_path: Option<&str>) -> Vec<String> {
    let mut args = vec![
        format!("--remote-debugging-port={debug_port}"),
        format!("--remote-allow-origins=http://127.0.0.1:{debug_port}"),
    ];
    if let Some(workspace) = workspace_path {
        args.push(workspace.to_string());
    }
    args
}

fn select_debug_port(preferred: u16) -> u16 {
    if TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
        return preferred;
    }
    TcpListener::bind(("127.0.0.1", 0))
        .ok()
        .and_then(|listener| listener.local_addr().ok())
        .map(|addr| addr.port())
        .unwrap_or(preferred)
}

async fn inject_enhancement_script(debug_port: u16) -> Result<(), String> {
    let target = wait_for_cdp_target(debug_port).await?;
    let websocket_url = target
        .web_socket_debugger_url
        .ok_or_else(|| "CDP 页面目标缺少 websocket 地址".to_string())?;
    let (mut socket, _) = connect_async(&websocket_url)
        .await
        .map_err(|error| format!("连接 CDP websocket 失败: {error}"))?;
    let bridge_script = bridge_runtime_script();
    let script = enhanced_renderer_script();

    let _ = send_cdp_command(&mut socket, 1, "Runtime.enable", json!({})).await;
    let _ = send_cdp_command(&mut socket, 2, "Page.enable", json!({})).await;
    let _ = send_cdp_command(
        &mut socket,
        3,
        "Runtime.removeBinding",
        json!({ "name": BRIDGE_BINDING_NAME }),
    )
    .await;
    send_cdp_command(
        &mut socket,
        4,
        "Runtime.addBinding",
        json!({ "name": BRIDGE_BINDING_NAME }),
    )
    .await?;
    send_cdp_command(
        &mut socket,
        5,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": bridge_script }),
    )
    .await?;
    send_cdp_command(
        &mut socket,
        6,
        "Runtime.evaluate",
        json!({
            "expression": bridge_script,
            "allowUnsafeEvalBlockedByCSP": true
        }),
    )
    .await?;
    send_cdp_command(
        &mut socket,
        7,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": script }),
    )
    .await?;
    send_cdp_command(
        &mut socket,
        8,
        "Runtime.evaluate",
        json!({
            "expression": script,
            "allowUnsafeEvalBlockedByCSP": true
        }),
    )
    .await?;
    start_bridge_event_loop(socket);
    Ok(())
}

async fn wait_for_cdp_target(debug_port: u16) -> Result<CdpTarget, String> {
    let mut last_error = None;
    for _ in 0..CDP_RETRY_COUNT {
        match list_cdp_targets(debug_port).await {
            Ok(targets) => {
                if let Some(target) = pick_page_target(targets) {
                    return Ok(target);
                }
                last_error = Some("CDP 已响应，但没有可注入的页面目标".to_string());
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        tokio::time::sleep(Duration::from_millis(CDP_RETRY_DELAY_MS)).await;
    }
    Err(last_error.unwrap_or_else(|| "等待 Codex CDP 页面目标超时".to_string()))
}

async fn list_cdp_targets(debug_port: u16) -> Result<Vec<CdpTarget>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|error| format!("创建 CDP HTTP 客户端失败: {error}"))?;
    let url = format!("http://127.0.0.1:{debug_port}/json");
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("读取 CDP 目标失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("读取 CDP 目标失败: HTTP {}", response.status()));
    }
    response
        .json::<Vec<CdpTarget>>()
        .await
        .map_err(|error| format!("解析 CDP 目标失败: {error}"))
}

fn pick_page_target(targets: Vec<CdpTarget>) -> Option<CdpTarget> {
    targets.into_iter().find(|target| {
        target.web_socket_debugger_url.is_some()
            && target
                .target_type
                .as_deref()
                .is_none_or(|target_type| target_type == "page")
            && target
                .url
                .as_deref()
                .is_none_or(|url| !url.starts_with("devtools://"))
    })
}

async fn send_cdp_command<S>(
    socket: &mut S,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String>
where
    S: SinkExt<Message>
        + StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    socket
        .send(Message::Text(
            json!({
                "id": id,
                "method": method,
                "params": params
            })
            .to_string(),
        ))
        .await
        .map_err(|error| format!("发送 CDP 命令 {method} 失败: {error}"))?;

    tokio::time::timeout(Duration::from_secs(CDP_COMMAND_TIMEOUT_SECS), async {
        while let Some(message) = socket.next().await {
            let message = message.map_err(|error| format!("读取 CDP 响应失败: {error}"))?;
            let Message::Text(text) = message else {
                continue;
            };
            let value = serde_json::from_str::<Value>(&text)
                .map_err(|error| format!("解析 CDP 响应失败: {error}"))?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(format!("CDP 命令 {method} 返回错误: {error}"));
            }
            return Ok(value);
        }
        Err(format!("CDP websocket 在命令 {method} 返回前关闭"))
    })
    .await
    .map_err(|_| format!("等待 CDP 命令 {method} 响应超时"))?
}

fn start_bridge_event_loop<S>(mut socket: S)
where
    S: SinkExt<Message>
        + StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin
        + Send
        + 'static,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    tokio::spawn(async move {
        let mut next_id = 1_000_u64;
        while let Some(message) = socket.next().await {
            let Ok(Message::Text(text)) = message else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if value.get("method").and_then(Value::as_str) != Some("Runtime.bindingCalled") {
                continue;
            }
            let params = &value["params"];
            if params.get("name").and_then(Value::as_str) != Some(BRIDGE_BINDING_NAME) {
                continue;
            }
            let Some(payload_text) = params.get("payload").and_then(Value::as_str) else {
                continue;
            };
            let Ok(request) = serde_json::from_str::<Value>(payload_text) else {
                continue;
            };
            let request_id = request
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let path = request
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let payload = request.get("payload").cloned().unwrap_or(Value::Null);
            let result = handle_bridge_request(&path, payload).await;
            let expression = bridge_resolve_expression(&request_id, &result);
            next_id += 1;
            let _ = socket
                .send(Message::Text(
                    json!({
                        "id": next_id,
                        "method": "Runtime.evaluate",
                        "params": {
                            "expression": expression,
                            "allowUnsafeEvalBlockedByCSP": true
                        }
                    })
                    .to_string(),
                ))
                .await;
        }
    });
}

fn bridge_resolve_expression(request_id: &str, result: &Value) -> String {
    let request_id = serde_json::to_string(request_id).unwrap_or_else(|_| "\"\"".to_string());
    let result = serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string());
    format!("window.__codexDeckBridgeResolve({request_id}, {result})")
}

async fn handle_bridge_request(path: &str, payload: Value) -> Value {
    match path {
        "/backend/status" | "/backend/repair" => json!({
            "status": "ok",
            "message": "CodexDeck 增强后端已连接",
            "version": env!("CARGO_PKG_VERSION"),
        }),
        "/settings/get" => codexdeck_bridge_settings(),
        "/settings/set" => {
            let mut settings = codexdeck_bridge_settings();
            if let Some(object) = settings.as_object_mut() {
                if let Some(incoming) = payload.as_object() {
                    for (key, value) in incoming {
                        object.insert(key.clone(), value.clone());
                    }
                }
            }
            settings
        }
        "/delete" => delete_local_session(session_from_payload(&payload)),
        "/undo" => undo_local_session(
            payload
                .get("undo_token")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        "/export-markdown" => export_markdown(session_from_payload(&payload)),
        "/archived-thread" => find_archived_thread_by_title(
            payload
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        "/move-thread-workspace" => move_thread_workspace(
            session_from_payload(&payload),
            payload
                .get("target_cwd")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        "/thread-sort-key" => thread_sort_key(session_from_payload(&payload)),
        "/thread-sort-keys" => thread_sort_keys(sessions_from_payload(&payload)),
        "/diagnostics/log" => json!({"status": "ok", "message": "日志已记录"}),
        "/user-scripts/list" | "/user-scripts/reload" => {
            json!({"enabled": false, "scripts": [], "builtin_dir": "", "user_dir": ""})
        }
        "/user-scripts/set-enabled" | "/user-scripts/set-script-enabled" => {
            json!({"enabled": false, "scripts": [], "builtin_dir": "", "user_dir": ""})
        }
        "/codex-model-catalog" | "/codex-config-model" => codex_model_catalog().await,
        "/devtools/open"
        | "/manager/open"
        | "/ads"
        | "/zed-remote/status"
        | "/zed-remote/resolve-host"
        | "/zed-remote/fallback-request"
        | "/zed-remote/open" => {
            json!({"status": "failed", "message": "当前 CodexDeck 增强版暂未启用该外部集成"})
        }
        _ => json!({"status": "failed", "message": format!("未知 CodexDeck bridge 路径: {path}")}),
    }
}

fn codexdeck_bridge_settings() -> Value {
    json!({
        "providerSyncEnabled": false,
        "enhancementsEnabled": true,
        "launchMode": "codexdeck",
    })
}

fn session_from_payload(payload: &Value) -> SessionRef {
    SessionRef {
        session_id: payload
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        title: payload
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

fn sessions_from_payload(payload: &Value) -> Vec<SessionRef> {
    payload
        .get("sessions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_object())
                .map(|item| SessionRef {
                    session_id: item
                        .get("session_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    title: item
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn codex_state_db_path() -> Result<PathBuf, String> {
    Ok(app_paths::codex_dir()?.join("state_5.sqlite"))
}

fn enhanced_delete_backup_root() -> Result<PathBuf, String> {
    Ok(app_paths::codex_state_provider_backup_dir()?.join("enhanced-delete"))
}

fn normalize_session_id(session_id: &str) -> String {
    session_id
        .strip_prefix("local:")
        .unwrap_or(session_id)
        .to_string()
}

fn delete_local_session(session: SessionRef) -> Value {
    let thread_id = normalize_session_id(&session.session_id);
    let result = (|| -> Result<Value, String> {
        if thread_id.is_empty() {
            return Err("会话 ID 为空".to_string());
        }
        let db_path = codex_state_db_path()?;
        if !db_path.is_file() {
            return Err(format!("数据库不存在：{}", db_path.display()));
        }
        let mut db = Connection::open(&db_path).map_err(|error| error.to_string())?;
        if !has_columns(&db, "threads", &["id", "title", "rollout_path"])? {
            return Err("不支持当前 Codex 本地存储结构".to_string());
        }

        let thread_rows = select_dicts(&db, "SELECT * FROM threads WHERE id = ?1", &[&thread_id])?;
        if thread_rows.is_empty() {
            return Err("本地存储中未找到该会话".to_string());
        }

        let mut tables = Map::new();
        tables.insert("threads".to_string(), Value::Array(thread_rows));
        backup_related_rows(
            &db,
            &mut tables,
            "thread_dynamic_tools",
            "thread_id = ?1",
            &[&thread_id],
        )?;
        backup_related_rows(
            &db,
            &mut tables,
            "thread_goals",
            "thread_id = ?1",
            &[&thread_id],
        )?;
        backup_related_rows(
            &db,
            &mut tables,
            "thread_spawn_edges",
            "parent_thread_id = ?1 OR child_thread_id = ?1",
            &[&thread_id],
        )?;
        backup_related_rows(
            &db,
            &mut tables,
            "stage1_outputs",
            "thread_id = ?1",
            &[&thread_id],
        )?;
        backup_related_rows(
            &db,
            &mut tables,
            "agent_job_items",
            "assigned_thread_id = ?1",
            &[&thread_id],
        )?;
        let files = rollout_file_backups(tables.get("threads").and_then(Value::as_array));
        if !files.is_empty() {
            tables.insert("__files".to_string(), Value::Array(files.clone()));
        }

        let (token, backup_path) =
            write_delete_backup(&thread_id, &db_path, Value::Object(tables))?;
        let delete_result = (|| -> Result<(), String> {
            let tx = db.transaction().map_err(|error| error.to_string())?;
            delete_related_rows(&tx, "thread_dynamic_tools", "thread_id = ?1", &[&thread_id])?;
            delete_related_rows(&tx, "thread_goals", "thread_id = ?1", &[&thread_id])?;
            delete_related_rows(
                &tx,
                "thread_spawn_edges",
                "parent_thread_id = ?1 OR child_thread_id = ?1",
                &[&thread_id],
            )?;
            delete_related_rows(&tx, "stage1_outputs", "thread_id = ?1", &[&thread_id])?;
            if has_columns(&tx, "agent_job_items", &["assigned_thread_id"])? {
                tx.execute(
                    "UPDATE agent_job_items SET assigned_thread_id = NULL WHERE assigned_thread_id = ?1",
                    [&thread_id],
                )
                .map_err(|error| error.to_string())?;
            }
            tx.execute("DELETE FROM threads WHERE id = ?1", [&thread_id])
                .map_err(|error| error.to_string())?;
            tx.commit().map_err(|error| error.to_string())?;
            Ok(())
        })();
        if let Err(error) = delete_result {
            return Ok(json!({
                "status": "failed",
                "session_id": thread_id,
                "message": error,
                "undo_token": token,
                "backup_path": backup_path.to_string_lossy()
            }));
        }

        let mut file_errors = Vec::new();
        for file in files {
            if let Some(path) = file.get("path").and_then(Value::as_str) {
                if let Err(error) = fs::remove_file(path) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        file_errors.push(format!("{path}: {error}"));
                    }
                }
            }
        }
        if !file_errors.is_empty() {
            return Ok(json!({
                "status": "failed",
                "session_id": thread_id,
                "message": format!("数据库已删除，但 rollout 文件删除失败：{}", file_errors.join("; ")),
                "undo_token": token,
                "backup_path": backup_path.to_string_lossy()
            }));
        }

        Ok(json!({
            "status": "local_deleted",
            "session_id": thread_id,
            "message": "已从本地存储删除",
            "undo_token": token,
            "backup_path": backup_path.to_string_lossy()
        }))
    })();
    result.unwrap_or_else(|error| {
        json!({
            "status": "failed",
            "session_id": thread_id,
            "message": error,
            "undo_token": Value::Null,
            "backup_path": Value::Null
        })
    })
}

fn undo_local_session(token: &str) -> Value {
    let result = (|| -> Result<Value, String> {
        let backup_path =
            enhanced_delete_backup_root()?.join(format!("{}.json", safe_token(token)));
        let backup_text = fs::read_to_string(&backup_path).map_err(|error| error.to_string())?;
        let backup: Value =
            serde_json::from_str(&backup_text).map_err(|error| error.to_string())?;
        let db_path = backup
            .get("source_db")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .ok_or_else(|| "备份缺少数据库路径".to_string())?;
        let session_id = backup
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut db = Connection::open(&db_path).map_err(|error| error.to_string())?;
        let tx = db.transaction().map_err(|error| error.to_string())?;
        if let Some(tables) = backup.get("tables").and_then(Value::as_object) {
            for (table, rows) in tables {
                if table.starts_with("__") {
                    continue;
                }
                let Some(rows) = rows.as_array() else {
                    continue;
                };
                for row in rows {
                    if let Some(row) = row.as_object() {
                        insert_row(&tx, table, row)?;
                    }
                }
            }
            if let Some(files) = tables.get("__files").and_then(Value::as_array) {
                for file in files {
                    let Some(path) = file.get("path").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(content) = file.get("content_b64").and_then(Value::as_str) else {
                        continue;
                    };
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(content)
                        .map_err(|error| error.to_string())?;
                    if let Some(parent) = Path::new(path).parent() {
                        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                    }
                    fs::write(path, bytes).map_err(|error| error.to_string())?;
                }
            }
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(json!({
            "status": "undone",
            "session_id": session_id,
            "message": "已从 CodexDeck 备份恢复",
            "undo_token": token,
            "backup_path": backup_path.to_string_lossy()
        }))
    })();
    result.unwrap_or_else(|error| {
        json!({
            "status": "failed",
            "session_id": "",
            "message": error,
            "undo_token": token,
            "backup_path": Value::Null
        })
    })
}

fn export_markdown(session: SessionRef) -> Value {
    let thread_id = normalize_session_id(&session.session_id);
    let result = (|| -> Result<Value, String> {
        let db_path = codex_state_db_path()?;
        let db = Connection::open(&db_path).map_err(|error| error.to_string())?;
        if !has_columns(&db, "threads", &["id", "title", "rollout_path"])? {
            return Err("不支持当前 Codex 本地存储结构".to_string());
        }
        let (title, rollout_path) = db
            .query_row(
                "SELECT title, rollout_path FROM threads WHERE id = ?1",
                [&thread_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => "未找到对应会话".to_string(),
                other => other.to_string(),
            })?;
        let title = display_title(title.as_deref().unwrap_or(&session.title));
        let rollout_path = rollout_path
            .filter(|path| !path.is_empty())
            .ok_or_else(|| "会话缺少 rollout 文件路径".to_string())?;
        let messages = load_rollout_messages(Path::new(&rollout_path))?;
        if messages.is_empty() {
            return Err("未找到可导出的用户或助手消息".to_string());
        }
        let filename = build_markdown_filename(&title, &thread_id);
        Ok(json!({
            "status": "exported",
            "session_id": thread_id,
            "message": format!("已导出为 Markdown：{filename}"),
            "filename": filename,
            "markdown": render_markdown(&title, &messages)
        }))
    })();
    result.unwrap_or_else(|error| {
        json!({
            "status": "failed",
            "session_id": thread_id,
            "message": error,
            "filename": Value::Null,
            "markdown": Value::Null
        })
    })
}

fn find_archived_thread_by_title(title: &str) -> Value {
    let result = (|| -> Result<Value, String> {
        let db = Connection::open(codex_state_db_path()?).map_err(|error| error.to_string())?;
        if !has_columns(&db, "threads", &["id", "title", "archived"])? {
            return Ok(json!({"session_id": "", "title": ""}));
        }
        let mut stmt = db
            .prepare(
                "SELECT id, title FROM threads
                 WHERE archived = 1 AND (title = ?1 OR title LIKE ?2 OR ?1 LIKE '%' || title || '%')
                 ORDER BY archived_at DESC LIMIT 1",
            )
            .map_err(|error| error.to_string())?;
        let mut rows = stmt
            .query((title, format!("%{title}%")))
            .map_err(|error| error.to_string())?;
        let Some(row) = rows.next().map_err(|error| error.to_string())? else {
            return Ok(json!({"session_id": "", "title": ""}));
        };
        Ok(json!({
            "session_id": row.get::<_, String>(0).map_err(|error| error.to_string())?,
            "title": row.get::<_, Option<String>>(1).map_err(|error| error.to_string())?.unwrap_or_default()
        }))
    })();
    result.unwrap_or_else(|error| json!({"status": "failed", "message": error}))
}

fn move_thread_workspace(session: SessionRef, target_cwd: &str) -> Value {
    let thread_id = normalize_session_id(&session.session_id);
    let target = target_cwd.trim();
    if target.is_empty() {
        return json!({"status": "failed", "session_id": thread_id, "message": "目标项目路径为空"});
    }
    let result = (|| -> Result<Value, String> {
        let db = Connection::open(codex_state_db_path()?).map_err(|error| error.to_string())?;
        if !has_columns(&db, "threads", &["id", "cwd", "rollout_path"])? {
            return Err("不支持当前 Codex 本地存储结构".to_string());
        }
        let (previous_cwd, rollout_path) = db
            .query_row(
                "SELECT cwd, rollout_path FROM threads WHERE id = ?1",
                [&thread_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?;
        db.execute(
            "UPDATE threads SET cwd = ?1 WHERE id = ?2",
            (target, &thread_id),
        )
        .map_err(|error| error.to_string())?;
        let (rollout_updated, rollout_error) = update_rollout_session_meta_cwd(
            rollout_path.as_deref().unwrap_or(""),
            &thread_id,
            target,
        );
        Ok(json!({
            "status": "moved",
            "session_id": thread_id,
            "message": "已移动对话",
            "previous_cwd": previous_cwd.unwrap_or_default(),
            "target_cwd": target,
            "rollout_updated": rollout_updated,
            "rollout_error": rollout_error
        }))
    })();
    result.unwrap_or_else(
        |error| json!({"status": "failed", "session_id": thread_id, "message": error}),
    )
}

fn thread_sort_key(session: SessionRef) -> Value {
    let thread_id = normalize_session_id(&session.session_id);
    let result = (|| -> Result<Value, String> {
        let db = Connection::open(codex_state_db_path()?).map_err(|error| error.to_string())?;
        match fetch_thread_timestamp_payload(&db, &thread_id)? {
            Some(mut payload) => {
                payload.insert("status".to_string(), json!("ok"));
                payload.insert("session_id".to_string(), json!(thread_id));
                Ok(Value::Object(payload))
            }
            None => Ok(
                json!({"status": "failed", "session_id": thread_id, "message": "Thread not found"}),
            ),
        }
    })();
    result.unwrap_or_else(
        |error| json!({"status": "failed", "session_id": thread_id, "message": error}),
    )
}

fn thread_sort_keys(sessions: Vec<SessionRef>) -> Value {
    let result = (|| -> Result<Value, String> {
        let db = Connection::open(codex_state_db_path()?).map_err(|error| error.to_string())?;
        let mut sort_keys = Vec::new();
        for session in sessions.into_iter().take(200) {
            let thread_id = normalize_session_id(&session.session_id);
            if let Some(mut payload) = fetch_thread_timestamp_payload(&db, &thread_id)? {
                payload.insert("session_id".to_string(), json!(thread_id));
                sort_keys.push(Value::Object(payload));
            }
        }
        Ok(json!({"status": "ok", "sort_keys": sort_keys}))
    })();
    result.unwrap_or_else(|error| json!({"status": "failed", "message": error, "sort_keys": []}))
}

async fn codex_model_catalog() -> Value {
    let Ok(config_text) = fs::read_to_string(app_paths::codex_config_path().unwrap_or_default())
    else {
        return json!({"status": "failed", "models": [], "message": "未读取到 Codex config"});
    };
    let mut base_url = String::new();
    let mut token = String::new();
    for line in config_text.lines() {
        let trimmed = line.trim();
        if base_url.is_empty() && trimmed.starts_with("base_url") {
            base_url = toml_string_value(trimmed).unwrap_or_default();
        }
        if token.is_empty() && trimmed.starts_with("experimental_bearer_token") {
            token = toml_string_value(trimmed).unwrap_or_default();
        }
    }
    if base_url.is_empty() || token.is_empty() {
        return json!({"status": "failed", "models": [], "message": "当前配置缺少 base_url 或 bearer token"});
    }
    let endpoint = models_endpoint(&base_url);
    let result = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|error| error.to_string());
    let Ok(client) = result else {
        return json!({"status": "failed", "models": [], "message": "创建 HTTP 客户端失败"});
    };
    match client.get(endpoint).bearer_auth(token).send().await {
        Ok(response) if response.status().is_success() => match response.json::<Value>().await {
            Ok(payload) => {
                let models = parse_model_payload(&payload);
                json!({"status": "ok", "models": models, "default_model": models.first().cloned().unwrap_or_default()})
            }
            Err(error) => json!({"status": "failed", "models": [], "message": error.to_string()}),
        },
        Ok(response) => {
            json!({"status": "failed", "models": [], "message": format!("模型接口返回 HTTP {}", response.status())})
        }
        Err(error) => json!({"status": "failed", "models": [], "message": error.to_string()}),
    }
}

fn write_delete_backup(
    session_id: &str,
    source_db: &Path,
    tables: Value,
) -> Result<(String, PathBuf), String> {
    let root = enhanced_delete_backup_root()?;
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let token = format!("{epoch}-{}", uuid::Uuid::new_v4().simple());
    let path = root.join(format!("{token}.json"));
    let payload = json!({
        "token": token,
        "session_id": session_id,
        "source_db": source_db.to_string_lossy(),
        "tables": tables,
    });
    fs::write(
        &path,
        serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok((token, path))
}

fn safe_token(token: &str) -> String {
    token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .collect()
}

fn has_table(db: &Connection, table: &str) -> Result<bool, String> {
    Ok(db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .is_ok())
}

fn has_columns(db: &Connection, table: &str, columns: &[&str]) -> Result<bool, String> {
    if !has_table(db, table)? {
        return Ok(false);
    }
    let existing: HashSet<String> = table_columns(db, table)?.into_iter().collect();
    Ok(columns.iter().all(|column| existing.contains(*column)))
}

fn table_columns(db: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut stmt = db
        .prepare(&format!(
            "PRAGMA table_info(\"{}\")",
            table.replace('"', "\"\"")
        ))
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn select_dicts(db: &Connection, sql: &str, params: &[&dyn ToSql]) -> Result<Vec<Value>, String> {
    let mut stmt = db.prepare(sql).map_err(|error| error.to_string())?;
    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let rows = stmt
        .query_map(params, |row| {
            let mut data = Map::new();
            for (index, column) in columns.iter().enumerate() {
                data.insert(column.clone(), sql_value_to_json(row.get_ref(index)?));
            }
            Ok(Value::Object(data))
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn backup_related_rows(
    db: &Connection,
    tables: &mut Map<String, Value>,
    table: &str,
    where_clause: &str,
    params: &[&dyn ToSql],
) -> Result<(), String> {
    if has_table(db, table)? {
        let rows = select_dicts(
            db,
            &format!("SELECT * FROM \"{table}\" WHERE {where_clause}"),
            params,
        )?;
        tables.insert(table.to_string(), Value::Array(rows));
    }
    Ok(())
}

fn delete_related_rows(
    db: &Connection,
    table: &str,
    where_clause: &str,
    params: &[&dyn ToSql],
) -> Result<(), String> {
    if has_table(db, table)? {
        db.execute(
            &format!("DELETE FROM \"{table}\" WHERE {where_clause}"),
            params,
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn insert_row(db: &Connection, table: &str, row: &Map<String, Value>) -> Result<(), String> {
    let columns: Vec<&String> = row.keys().collect();
    if columns.is_empty() {
        return Ok(());
    }
    let quoted = columns
        .iter()
        .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let marks = (0..columns.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let values = columns
        .iter()
        .map(|column| OwnedSqlValue(json_to_sql_value(&row[*column])))
        .collect::<Vec<_>>();
    let refs = values
        .iter()
        .map(|value| value as &dyn ToSql)
        .collect::<Vec<_>>();
    db.execute(
        &format!("INSERT INTO \"{table}\" ({quoted}) VALUES ({marks})"),
        refs.as_slice(),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn rollout_file_backups(thread_rows: Option<&Vec<Value>>) -> Vec<Value> {
    thread_rows
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("rollout_path").and_then(Value::as_str))
        .filter_map(|path| {
            let bytes = fs::read(path).ok()?;
            Some(json!({
                "path": path,
                "content_b64": base64::engine::general_purpose::STANDARD.encode(bytes),
            }))
        })
        .collect()
}

fn update_rollout_session_meta_cwd(
    rollout_path: &str,
    thread_id: &str,
    target_cwd: &str,
) -> (bool, String) {
    if rollout_path.is_empty() || !Path::new(rollout_path).is_file() {
        return (false, String::new());
    }
    let result = (|| -> Result<bool, String> {
        let text = fs::read_to_string(rollout_path).map_err(|error| error.to_string())?;
        let mut changed = false;
        let mut output = String::new();
        for line in text.split_inclusive('\n') {
            let (body, end) = line
                .strip_suffix('\n')
                .map_or((line, ""), |body| (body, "\n"));
            let mut raw = line.to_string();
            if let Ok(mut item) = serde_json::from_str::<Value>(body) {
                if item.get("type") == Some(&json!("session_meta"))
                    && item["payload"]["id"] == thread_id
                    && item["payload"]["cwd"] != target_cwd
                {
                    if let Some(payload) = item.get_mut("payload").and_then(Value::as_object_mut) {
                        payload.insert("cwd".to_string(), json!(target_cwd));
                        raw =
                            serde_json::to_string(&item).map_err(|error| error.to_string())? + end;
                        changed = true;
                    }
                }
            }
            output.push_str(&raw);
        }
        if changed {
            fs::write(rollout_path, output).map_err(|error| error.to_string())?;
        }
        Ok(changed)
    })();
    match result {
        Ok(changed) => (changed, String::new()),
        Err(error) => (false, error),
    }
}

fn fetch_thread_timestamp_payload(
    db: &Connection,
    thread_id: &str,
) -> Result<Option<Map<String, Value>>, String> {
    if !has_table(db, "threads")? {
        return Ok(None);
    }
    let existing: HashSet<String> = table_columns(db, "threads")?.into_iter().collect();
    let timestamp_columns = ["updated_at", "updated_at_ms", "created_at_ms"]
        .iter()
        .filter(|column| existing.contains(**column))
        .map(|column| column.to_string())
        .collect::<Vec<_>>();
    let mut columns = vec!["id".to_string()];
    columns.extend(timestamp_columns);
    let sql = format!("SELECT {} FROM threads WHERE id = ?1", columns.join(", "));
    let mut stmt = db.prepare(&sql).map_err(|error| error.to_string())?;
    let row = stmt.query_row([thread_id], |row| {
        let mut selected = Map::new();
        for (index, column) in columns.iter().enumerate() {
            selected.insert(column.clone(), sql_value_to_json(row.get_ref(index)?));
        }
        Ok(selected)
    });
    match row {
        Ok(row) => {
            let mut payload = Map::new();
            for column in ["updated_at", "updated_at_ms", "created_at_ms"] {
                payload.insert(
                    column.to_string(),
                    row.get(column).cloned().unwrap_or(Value::Null),
                );
            }
            Ok(Some(payload))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn sql_value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => json!(value),
        ValueRef::Real(value) => json!(value),
        ValueRef::Text(value) => json!(String::from_utf8_lossy(value).to_string()),
        ValueRef::Blob(value) => {
            json!(base64::engine::general_purpose::STANDARD.encode(value))
        }
    }
}

fn json_to_sql_value(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(value) => SqlValue::Integer(i64::from(*value)),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                SqlValue::Integer(value)
            } else if let Some(value) = number.as_f64() {
                SqlValue::Real(value)
            } else {
                SqlValue::Text(number.to_string())
            }
        }
        Value::String(value) => SqlValue::Text(value.clone()),
        other => SqlValue::Text(other.to_string()),
    }
}

#[derive(Debug)]
struct MarkdownMessage {
    speaker: &'static str,
    timestamp: Option<String>,
    body: String,
}

fn load_rollout_messages(path: &Path) -> Result<Vec<MarkdownMessage>, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut messages = Vec::new();
    for raw in text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let event: Value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if event.get("type") != Some(&Value::String("response_item".to_string())) {
            continue;
        }
        let payload = &event["payload"];
        if payload.get("type") != Some(&Value::String("message".to_string())) {
            continue;
        }
        let speaker = match payload.get("role").and_then(Value::as_str).unwrap_or("") {
            "user" => "User",
            "assistant" => "Assistant",
            _ => continue,
        };
        let body = serialize_message_content(&payload["content"]);
        if body.is_empty() {
            continue;
        }
        messages.push(MarkdownMessage {
            speaker,
            timestamp: event
                .get("timestamp")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            body,
        });
    }
    Ok(messages)
}

fn serialize_message_content(content: &Value) -> String {
    let Some(items) = content.as_array() else {
        return String::new();
    };
    items
        .iter()
        .filter_map(|block| {
            let block_type = block.get("type").and_then(Value::as_str)?;
            match block_type {
                "input_text" | "output_text" => {
                    let text =
                        normalize_newlines(block.get("text").and_then(Value::as_str).unwrap_or(""))
                            .trim_matches('\n')
                            .to_string();
                    (!text.trim().is_empty()).then_some(text)
                }
                "input_image" => {
                    let image_url = block
                        .get("image_url")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    if image_url.is_empty() || image_url.starts_with("data:") {
                        Some("> Image attachment".to_string())
                    } else {
                        Some(format!("> Image attachment\n[Image link](<{image_url}>)"))
                    }
                }
                _ => None,
            }
        })
        .filter(|block| !block.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

fn render_markdown(title: &str, messages: &[MarkdownMessage]) -> String {
    let mut lines = vec![format!("# {title}"), String::new()];
    for message in messages {
        lines.push(format!("### {}", message.speaker));
        if let Some(timestamp) = &message.timestamp {
            lines.push(format!("_{timestamp}_"));
        }
        lines.push(String::new());
        lines.push(message.body.trim_end().to_string());
        lines.push(String::new());
    }
    format!("{}\n", lines.join("\n").trim_end())
}

fn display_title(value: &str) -> String {
    let normalized = normalize_newlines(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        "Untitled session".to_string()
    } else {
        normalized
    }
}

fn build_markdown_filename(title: &str, thread_id: &str) -> String {
    let cleaned = collapse_whitespace(&replace_windows_filename_chars(title, " "))
        .trim_matches([' ', '.'])
        .to_string();
    let mut safe_title = cleaned
        .chars()
        .take(80)
        .collect::<String>()
        .trim_matches([' ', '.'])
        .to_string();
    if safe_title.is_empty() {
        safe_title = "Untitled session".to_string();
    }
    let safe_thread_id = replace_windows_filename_chars(thread_id, "-");
    format!("{safe_title}-{}.md", safe_thread_id.trim())
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn replace_windows_filename_chars(value: &str, replacement: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch.is_control() {
            output.push_str(replacement);
        } else {
            output.push(ch);
        }
    }
    output
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn toml_string_value(line: &str) -> Option<String> {
    let (_, value) = line.split_once('=')?;
    let trimmed = value.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        Some(trimmed[1..trimmed.len() - 1].replace("\\\"", "\""))
    } else {
        Some(trimmed.to_string())
    }
}

fn models_endpoint(base_url: &str) -> String {
    let cleaned = base_url.trim().trim_end_matches('/');
    if cleaned.ends_with("/models") {
        return cleaned.to_string();
    }
    if cleaned.ends_with("/v1") {
        return format!("{cleaned}/models");
    }
    format!("{cleaned}/v1/models")
}

fn parse_model_payload(payload: &Value) -> Vec<String> {
    let arrays = [
        payload.get("data"),
        payload.get("models"),
        payload.get("items"),
        Some(payload),
    ];
    let mut models = Vec::new();
    for array in arrays.into_iter().flatten().filter_map(Value::as_array) {
        for item in array {
            let model = item
                .get("id")
                .or_else(|| item.get("name"))
                .or_else(|| item.get("model"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if !model.is_empty() && !models.iter().any(|existing| existing == model) {
                models.push(model.to_string());
            }
        }
    }
    models
}

fn bridge_runtime_script() -> &'static str {
    r#"
(() => {
  if (window.__codexDeckBridgeVersion === "1") return;
  window.__codexDeckBridgeVersion = "1";
  window.__codexDeckBridgeCallbacks = new Map();
  window.__codexDeckBridgeSeq = 0;
  window.__codexDeckBridgeResolve = (id, result) => {
    const callback = window.__codexDeckBridgeCallbacks.get(id);
    if (!callback) return;
    window.__codexDeckBridgeCallbacks.delete(id);
    callback.resolve(result);
  };
  window.__codexDeckBridge = (path, payload) => new Promise((resolve) => {
    const id = String(++window.__codexDeckBridgeSeq);
    window.__codexDeckBridgeCallbacks.set(id, { resolve });
    window.codexDeckNativeBridge(JSON.stringify({ id, path, payload }));
  });
})();
"#
}

fn enhanced_renderer_script() -> &'static str {
    r#"
(() => {
  if (window.__codexDeckEnhancedLaunchVersion === "2") return;
  window.__codexDeckEnhancedLaunchVersion = "2";

  const settingsKey = "codexDeckEnhancedSettings";
  const menuId = "codexdeck-enhanced-menu";
  const styleId = "codexdeck-enhanced-style";
  const selectors = {
    sidebarThread: "[data-app-action-sidebar-thread-id]",
    threadTitle: "[data-thread-title]",
    pluginNavButton: 'nav[role="navigation"] button.h-token-nav-row.w-full',
    pluginSvgPath: 'svg path[d^="M7.94562 14.0277"]',
    disabledInstallButton: 'button:disabled, button[aria-disabled="true"], [role="button"][aria-disabled="true"], button[data-disabled], [role="button"][data-disabled], button.cursor-not-allowed, [role="button"].cursor-not-allowed, button.pointer-events-none, [role="button"].pointer-events-none',
  };

  function defaultSettings() {
    return {
      pluginEntryUnlock: true,
      forcePluginInstall: true,
      sessionDelete: true,
      markdownExport: true,
    };
  }

  function settings() {
    try {
      return { ...defaultSettings(), ...JSON.parse(localStorage.getItem(settingsKey) || "{}") };
    } catch (_) {
      return defaultSettings();
    }
  }

  function setSetting(key, value) {
    const next = { ...settings(), [key]: value };
    localStorage.setItem(settingsKey, JSON.stringify(next));
    renderMenu();
    scan();
  }

  function escapeHtml(value) {
    return String(value ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;");
  }

  function bridge(path, payload = {}) {
    if (typeof window.__codexDeckBridge !== "function") {
      return Promise.resolve({ status: "failed", message: "CodexDeck bridge 未连接" });
    }
    return window.__codexDeckBridge(path, payload);
  }

  function toast(message, action) {
    document.querySelectorAll(".codexdeck-toast").forEach((node) => node.remove());
    const node = document.createElement("div");
    node.className = "codexdeck-toast";
    node.textContent = message;
    if (action) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = action.label;
      button.addEventListener("click", action.onClick);
      node.appendChild(button);
    }
    document.documentElement.appendChild(node);
    setTimeout(() => node.remove(), action ? 9000 : 3600);
  }

  function installStyle() {
    const existing = document.getElementById(styleId);
    if (existing?.dataset.version === "2") return;
    existing?.remove();
    const style = document.createElement("style");
    style.id = styleId;
    style.dataset.version = "2";
    style.textContent = `
      #${menuId} {
        position: fixed;
        top: 8px;
        right: 132px;
        z-index: 2147483000;
        display: inline-flex;
        align-items: center;
        gap: 6px;
        height: 30px;
        -webkit-app-region: no-drag;
      }
      #${menuId} button {
        border: 1px solid rgba(255,255,255,.12);
        border-radius: 7px;
        background: rgba(31,31,35,.86);
        color: #f4f4f5;
        font: 12px system-ui, sans-serif;
        line-height: 1;
        padding: 6px 9px;
        cursor: pointer;
      }
      .codexdeck-modal-overlay {
        position: fixed;
        inset: 0;
        z-index: 2147483200;
        display: flex;
        align-items: center;
        justify-content: center;
        background: rgba(15,23,42,.34);
        -webkit-app-region: no-drag;
      }
      .codexdeck-modal {
        width: min(520px, calc(100vw - 44px));
        max-height: min(680px, calc(100vh - 44px));
        overflow: hidden;
        border: 1px solid rgba(255,255,255,.12);
        border-radius: 14px;
        background: #27272a;
        color: #f4f4f5;
        box-shadow: 0 24px 80px rgba(0,0,0,.46);
        font: 14px system-ui, sans-serif;
      }
      .codexdeck-modal-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 16px 18px 10px;
      }
      .codexdeck-modal-title { font-size: 17px; font-weight: 650; }
      .codexdeck-modal-close {
        border: 0;
        background: transparent;
        color: #d4d4d8;
        font-size: 20px;
        cursor: pointer;
      }
      .codexdeck-modal-body { padding: 0 18px 18px; }
      .codexdeck-row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 14px;
        border-top: 1px solid rgba(255,255,255,.10);
        padding: 12px 0;
      }
      .codexdeck-row-title { font-weight: 600; line-height: 1.35; }
      .codexdeck-row-desc { margin-top: 3px; color: #a1a1aa; font-size: 12px; line-height: 1.45; }
      .codexdeck-toggle {
        width: 42px;
        height: 24px;
        flex: 0 0 auto;
        border: 0;
        border-radius: 999px;
        background: #52525b;
        padding: 2px;
        cursor: pointer;
      }
      .codexdeck-toggle span {
        display: block;
        width: 20px;
        height: 20px;
        border-radius: 999px;
        background: #fff;
        transition: transform .12s ease;
      }
      .codexdeck-toggle[data-enabled="true"] { background: #10a37f; }
      .codexdeck-toggle[data-enabled="true"] span { transform: translateX(18px); }
      [data-codexdeck-enhanced-row="true"] { position: relative; }
      .codexdeck-session-actions {
        position: absolute;
        right: 28px;
        top: 50%;
        z-index: 20;
        display: inline-flex;
        align-items: center;
        gap: 5px;
        opacity: 0;
        transform: translateY(-50%);
      }
      [data-codexdeck-enhanced-row="true"]:hover .codexdeck-session-actions { opacity: 1; }
      .codexdeck-action-button {
        min-width: 28px;
        height: 26px;
        border: 0;
        border-radius: 6px;
        background: transparent;
        color: #d1d5db;
        font: 12px system-ui, sans-serif;
        cursor: pointer;
      }
      .codexdeck-action-button:hover,
      .codexdeck-action-button:focus-visible {
        background: #363839;
        color: #f4f4f5;
        outline: none;
      }
      .codexdeck-confirm-overlay {
        position: fixed;
        inset: 0;
        z-index: 2147483201;
        display: flex;
        align-items: center;
        justify-content: center;
        background: rgba(15,23,42,.28);
      }
      .codexdeck-confirm {
        width: min(420px, calc(100vw - 48px));
        border: 1px solid rgba(15,23,42,.12);
        border-radius: 12px;
        background: #fff;
        color: #111827;
        box-shadow: 0 24px 80px rgba(15,23,42,.22);
        padding: 18px;
        font: 14px system-ui, sans-serif;
      }
      .codexdeck-confirm-title { font-size: 16px; font-weight: 650; }
      .codexdeck-confirm-message { margin-top: 8px; color: #4b5563; line-height: 1.45; }
      .codexdeck-confirm-actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 18px; }
      .codexdeck-confirm-actions button {
        border: 1px solid #d1d5db;
        border-radius: 7px;
        background: #fff;
        color: #111827;
        cursor: pointer;
        font: 13px system-ui, sans-serif;
        padding: 6px 12px;
      }
      .codexdeck-confirm-actions [data-danger="true"] {
        border-color: #dc2626;
        background: #dc2626;
        color: #fff;
      }
      .codexdeck-toast {
        position: fixed;
        right: 18px;
        bottom: 18px;
        z-index: 2147483300;
        max-width: min(460px, calc(100vw - 36px));
        border-radius: 8px;
        background: #111827;
        color: #fff;
        box-shadow: 0 8px 30px rgba(0,0,0,.25);
        font: 13px system-ui, sans-serif;
        line-height: 18px;
        padding: 10px 12px;
      }
      .codexdeck-toast button { margin-left: 10px; pointer-events: auto; }
      .codexdeck-force-install-unlocked {
        opacity: 1 !important;
        pointer-events: auto !important;
        cursor: pointer !important;
      }
    `;
    document.documentElement.appendChild(style);
  }

  function installMenu() {
    installStyle();
    let menu = document.getElementById(menuId);
    if (menu) return;
    menu = document.createElement("div");
    menu.id = menuId;
    menu.innerHTML = `<button type="button" data-codexdeck-open="true">CodexDeck</button>`;
    menu.addEventListener("click", (event) => {
      if (event.target?.closest?.("[data-codexdeck-open]")) openModal();
    });
    document.documentElement.appendChild(menu);
  }

  function openModal() {
    document.querySelectorAll(".codexdeck-modal-overlay").forEach((node) => node.remove());
    const current = settings();
    const overlay = document.createElement("div");
    overlay.className = "codexdeck-modal-overlay";
    overlay.innerHTML = `
      <section class="codexdeck-modal" role="dialog" aria-modal="true" aria-label="CodexDeck 增强">
        <div class="codexdeck-modal-header">
          <div class="codexdeck-modal-title">CodexDeck 增强</div>
          <button type="button" class="codexdeck-modal-close" aria-label="关闭">×</button>
        </div>
        <div class="codexdeck-modal-body">
          ${settingRow("pluginEntryUnlock", "插件入口解锁", "显示并启用官方账号态才开放的插件入口。", current.pluginEntryUnlock)}
          ${settingRow("forcePluginInstall", "插件强制安装", "解除前端不可用状态导致的安装按钮禁用。", current.forcePluginInstall)}
          ${settingRow("sessionDelete", "会话删除按钮", "在侧边栏会话行添加删除按钮，删除前会写入 CodexDeck 备份。", current.sessionDelete)}
          ${settingRow("markdownExport", "Markdown 导出", "在侧边栏会话行添加 Markdown 导出按钮。", current.markdownExport)}
        </div>
      </section>
    `;
    overlay.addEventListener("click", (event) => {
      const target = event.target;
      if (target === overlay || target?.closest?.(".codexdeck-modal-close")) overlay.remove();
      const toggle = target?.closest?.("[data-codexdeck-setting]");
      if (toggle) {
        const key = toggle.getAttribute("data-codexdeck-setting");
        setSetting(key, !settings()[key]);
      }
    });
    document.documentElement.appendChild(overlay);
    renderMenu();
  }

  function settingRow(key, title, desc, enabled) {
    return `
      <div class="codexdeck-row">
        <div>
          <div class="codexdeck-row-title">${escapeHtml(title)}</div>
          <div class="codexdeck-row-desc">${escapeHtml(desc)}</div>
        </div>
        <button type="button" class="codexdeck-toggle" data-codexdeck-setting="${escapeHtml(key)}" data-enabled="${String(!!enabled)}"><span></span></button>
      </div>
    `;
  }

  function renderMenu() {
    const current = settings();
    document.querySelectorAll("[data-codexdeck-setting]").forEach((button) => {
      const key = button.getAttribute("data-codexdeck-setting");
      button.dataset.enabled = String(!!current[key]);
    });
  }

  function reactFiberFrom(element) {
    const key = Object.keys(element).find((item) => item.startsWith("__reactFiber"));
    return key ? element[key] : null;
  }

  function authContextValueFrom(element) {
    for (let fiber = reactFiberFrom(element); fiber; fiber = fiber.return) {
      for (const value of [fiber.memoizedProps?.value, fiber.pendingProps?.value]) {
        if (value && typeof value === "object" && typeof value.setAuthMethod === "function" && "authMethod" in value) {
          return value;
        }
      }
    }
    return null;
  }

  function spoofChatGPTAuthMethod(element) {
    const auth = authContextValueFrom(element);
    if (!auth || auth.authMethod === "chatgpt") return;
    auth.setAuthMethod("chatgpt");
  }

  function pluginEntryButton() {
    const byIcon = document.querySelector(`${selectors.pluginNavButton} ${selectors.pluginSvgPath}`)?.closest("button");
    if (byIcon) return byIcon;
    return Array.from(document.querySelectorAll(selectors.pluginNavButton))
      .find((button) => /^(插件|Plugins)(\s+-\s+.*)?$/i.test((button.textContent || "").trim())) || null;
  }

  function patchReactDisabledProps(element) {
    Object.keys(element)
      .filter((key) => key.startsWith("__reactProps"))
      .forEach((key) => {
        const props = element[key];
        if (!props || typeof props !== "object") return;
        props.disabled = false;
        props["aria-disabled"] = false;
        props["data-disabled"] = undefined;
      });
  }

  function clearDisabledState(element) {
    if (!(element instanceof HTMLElement)) return;
    if ("disabled" in element) element.disabled = false;
    element.removeAttribute("disabled");
    element.removeAttribute("aria-disabled");
    element.removeAttribute("data-disabled");
    element.removeAttribute("inert");
    element.classList.remove("disabled", "opacity-50", "cursor-not-allowed", "pointer-events-none");
    element.classList.add("codexdeck-force-install-unlocked");
    element.style.pointerEvents = "auto";
    element.style.cursor = "pointer";
    element.tabIndex = 0;
    patchReactDisabledProps(element);
  }

  function labelPluginButton(button) {
    const textNode = Array.from(button.querySelectorAll("span, div")).reverse()
      .flatMap((node) => Array.from(node.childNodes))
      .find((node) => node.nodeType === 3 && /^(插件|Plugins)( - 已解锁| - Unlocked)?$/i.test((node.nodeValue || "").trim()));
    if (!textNode) return;
    const current = (textNode.nodeValue || "").trim();
    textNode.nodeValue = /^Plugins/i.test(current) ? "Plugins - Unlocked" : "插件 - 已解锁";
  }

  function enablePluginEntry() {
    if (!settings().pluginEntryUnlock) return;
    const button = pluginEntryButton();
    if (!button) return;
    spoofChatGPTAuthMethod(button);
    clearDisabledState(button);
    button.style.display = "";
    button.querySelectorAll("*").forEach((node) => {
      if (node instanceof HTMLElement) node.style.display = "";
    });
    labelPluginButton(button);
    if (button.dataset.codexDeckPluginEnabled === "true") return;
    button.dataset.codexDeckPluginEnabled = "true";
    button.addEventListener("click", () => spoofChatGPTAuthMethod(button), true);
  }

  function installButtonLabel(element) {
    return (element.textContent || "").trim();
  }

  function isInstallButtonLabel(text) {
    return /^安装\s*/.test(text) || /^Install\s*/i.test(text) || text === "强制安装";
  }

  function unlockNodes(button) {
    const nodes = [button];
    button.querySelectorAll?.("button, [role='button'], [disabled], [aria-disabled], [data-disabled], .cursor-not-allowed, .pointer-events-none")
      .forEach((node) => nodes.push(node));
    let parent = button.parentElement;
    for (let depth = 0; parent && depth < 3; depth += 1, parent = parent.parentElement) {
      if (parent.matches?.("button, [role='button'], [disabled], [aria-disabled], [data-disabled], .cursor-not-allowed, .pointer-events-none")) {
        nodes.push(parent);
      }
    }
    return Array.from(new Set(nodes));
  }

  function labelForcedInstallButton(button) {
    const walker = document.createTreeWalker(button, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      if (isInstallButtonLabel((node.nodeValue || "").trim())) {
        node.nodeValue = "强制安装";
        return;
      }
    }
  }

  function unlockInstallButton(button) {
    unlockNodes(button).forEach(clearDisabledState);
    labelForcedInstallButton(button);
    if (button.dataset.codexDeckForceInstallUnlocked === "true") return;
    button.dataset.codexDeckForceInstallUnlocked = "true";
    const keepUnlocked = () => unlockNodes(button).forEach(clearDisabledState);
    ["pointerdown", "mousedown", "mouseup", "click", "focus"].forEach((eventName) => {
      button.addEventListener(eventName, keepUnlocked, true);
    });
  }

  function unblockPluginInstallButtons() {
    if (!settings().forcePluginInstall) return;
    const nodes = Array.from(document.querySelectorAll(selectors.disabledInstallButton));
    Array.from(new Set(nodes.map((node) => node.closest?.("button, [role='button']") || node)))
      .forEach((button) => {
        if (!isInstallButtonLabel(installButtonLabel(button))) return;
        unlockInstallButton(button);
      });
  }

  function sessionRows() {
    return Array.from(document.querySelectorAll(selectors.sidebarThread))
      .filter((row) => row instanceof HTMLElement);
  }

  function sessionRef(row) {
    const sessionId = row.getAttribute("data-app-action-sidebar-thread-id") || row.dataset.appActionSidebarThreadId || "";
    const title = (row.querySelector(selectors.threadTitle)?.textContent || row.textContent || "Untitled session")
      .replace(/\s+/g, " ")
      .trim();
    return { session_id: sessionId, title };
  }

  function attachRowActions(row) {
    const current = settings();
    const wantsActions = current.sessionDelete || current.markdownExport;
    row.dataset.codexdeckEnhancedRow = String(wantsActions);
    let group = row.querySelector(":scope > .codexdeck-session-actions");
    if (!wantsActions) {
      group?.remove();
      return;
    }
    if (!group) {
      group = document.createElement("div");
      group.className = "codexdeck-session-actions";
      row.appendChild(group);
    }
    group.innerHTML = `
      ${current.markdownExport ? '<button type="button" class="codexdeck-action-button" data-codexdeck-export="true" title="导出 Markdown">MD</button>' : ""}
      ${current.sessionDelete ? '<button type="button" class="codexdeck-action-button" data-codexdeck-delete="true" title="删除会话">删</button>' : ""}
    `;
    group.querySelector("[data-codexdeck-export]")?.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      exportSession(row);
    });
    group.querySelector("[data-codexdeck-delete]")?.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      confirmDelete(row);
    });
  }

  function confirmDelete(row) {
    const ref = sessionRef(row);
    if (!ref.session_id) {
      toast("未识别到会话 ID");
      return;
    }
    document.querySelectorAll(".codexdeck-confirm-overlay").forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = "codexdeck-confirm-overlay";
    overlay.innerHTML = `
      <section class="codexdeck-confirm" role="dialog" aria-modal="true" aria-label="删除会话">
        <div class="codexdeck-confirm-title">删除这个会话？</div>
        <div class="codexdeck-confirm-message">${escapeHtml(ref.title)}<br>删除前会创建 CodexDeck 本地备份。</div>
        <div class="codexdeck-confirm-actions">
          <button type="button" data-cancel="true">取消</button>
          <button type="button" data-danger="true">删除</button>
        </div>
      </section>
    `;
    overlay.addEventListener("click", async (event) => {
      const target = event.target;
      if (target === overlay || target?.closest?.("[data-cancel]")) {
        overlay.remove();
        return;
      }
      if (!target?.closest?.("[data-danger]")) return;
      const button = target.closest("[data-danger]");
      button.disabled = true;
      button.textContent = "删除中";
      const result = await bridge("/delete", ref);
      overlay.remove();
      if (result?.status === "local_deleted") {
        row.remove();
        toast(result.message || "已删除", result.undo_token ? {
          label: "撤销",
          onClick: async () => {
            const undo = await bridge("/undo", { undo_token: result.undo_token });
            toast(undo?.message || "撤销完成");
          },
        } : null);
      } else {
        toast(result?.message || "删除失败");
      }
    });
    document.documentElement.appendChild(overlay);
  }

  async function exportSession(row) {
    const ref = sessionRef(row);
    if (!ref.session_id) {
      toast("未识别到会话 ID");
      return;
    }
    const result = await bridge("/export-markdown", ref);
    if (result?.status !== "exported" || !result.markdown) {
      toast(result?.message || "导出失败");
      return;
    }
    const blob = new Blob([result.markdown], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = result.filename || `${ref.session_id}.md`;
    document.documentElement.appendChild(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    toast(result.message || "已导出 Markdown");
  }

  function refresh() {
    installStyle();
    installMenu();
    enablePluginEntry();
    unblockPluginInstallButtons();
    sessionRows().forEach(attachRowActions);
  }

  function scan() {
    try {
      refresh();
    } catch (error) {
      console.warn("CodexDeck enhanced scan failed", error);
    }
  }

  scan();
  setInterval(scan, 1000);
  const observer = new MutationObserver(refresh);
  const startObserver = () => observer.observe(document.documentElement, { childList: true, subtree: true });
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", startObserver, { once: true });
  } else {
    startObserver();
  }
})();
"#
}
