use std::collections::HashSet;
use std::fs;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::app_paths;
use crate::utils::now_unix_seconds;

const DAY_SECONDS: i64 = 24 * 60 * 60;
const TOKEN_USAGE_CACHE_FILE: &str = "token-usage-daily-cache.json";
const MAX_REASONABLE_SINGLE_TOKEN_DELTA: u64 = 10_000_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexTokenTotals {
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_output_tokens: u64,
    pub(crate) total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexTokenUsageSnapshot {
    pub(crate) updated_at: i64,
    pub(crate) source_path_count: usize,
    pub(crate) failed_path_count: usize,
    pub(crate) event_count: usize,
    pub(crate) last_7d: CodexTokenTotals,
    pub(crate) last_30d: CodexTokenTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexTokenUsageCache {
    cache_day: i64,
    snapshot: CodexTokenUsageSnapshot,
}

#[derive(Debug, Clone)]
struct ParsedTokenEvent {
    timestamp: i64,
    last: Option<CodexTokenTotals>,
    total: Option<CodexTokenTotals>,
}

pub(crate) fn collect_codex_token_usage_snapshot() -> Result<CodexTokenUsageSnapshot, String> {
    let now = now_unix_seconds();
    if let Some(snapshot) = read_cached_snapshot(now) {
        return Ok(snapshot);
    }

    let codex_dir = app_paths::codex_dir()?;
    let roots = [
        codex_dir.join("sessions"),
        codex_dir.join("archived_sessions"),
    ];
    let snapshot = scan_codex_token_usage_roots(&roots, now);
    if let Err(error) = write_cached_snapshot(&snapshot) {
        log::warn!("写入 Codex token 用量缓存失败: {error}");
    }
    Ok(snapshot)
}

fn read_cached_snapshot(now: i64) -> Option<CodexTokenUsageSnapshot> {
    let cache_path = token_usage_cache_path().ok()?;
    let content = fs::read_to_string(cache_path).ok()?;
    let cache = serde_json::from_str::<CodexTokenUsageCache>(&content).ok()?;
    if cache.cache_day == cache_day(now) {
        return Some(cache.snapshot);
    }
    None
}

fn write_cached_snapshot(snapshot: &CodexTokenUsageSnapshot) -> Result<(), String> {
    let cache_path = token_usage_cache_path()?;
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 Codex token 用量缓存目录失败: {error}"))?;
    }
    let cache = CodexTokenUsageCache {
        cache_day: cache_day(snapshot.updated_at),
        snapshot: snapshot.clone(),
    };
    let serialized = serde_json::to_string_pretty(&cache)
        .map_err(|error| format!("序列化 Codex token 用量缓存失败: {error}"))?;
    fs::write(cache_path, serialized)
        .map_err(|error| format!("写入 Codex token 用量缓存失败: {error}"))
}

fn token_usage_cache_path() -> Result<PathBuf, String> {
    let data_dir = app_data_dir_without_tauri_handle()?;
    Ok(data_dir.join(TOKEN_USAGE_CACHE_FILE))
}

fn app_data_dir_without_tauri_handle() -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    {
        if let Some(path) = env_path("CODEX_SWITCH_DEV_DATA_DIR")
            .or_else(|| env_path("CODEX_TOOLS_DEV_DATA_DIR"))
        {
            return Ok(path);
        }
    }

    dirs::data_dir()
        .map(|path| path.join("com.carry.codex-tools"))
        .ok_or_else(|| "无法读取应用数据目录".to_string())
}

fn env_path(name: &str) -> Option<PathBuf> {
    let value = std::env::var(name).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

fn cache_day(timestamp: i64) -> i64 {
    timestamp.div_euclid(DAY_SECONDS)
}

fn scan_codex_token_usage_roots(roots: &[PathBuf], now: i64) -> CodexTokenUsageSnapshot {
    let mut files = Vec::new();
    let mut failed_path_count = 0;
    for root in roots {
        collect_jsonl_files(root, &mut files, &mut failed_path_count);
    }

    let mut snapshot = CodexTokenUsageSnapshot {
        updated_at: now,
        source_path_count: files.len(),
        failed_path_count,
        event_count: 0,
        last_7d: CodexTokenTotals::default(),
        last_30d: CodexTokenTotals::default(),
    };

    let last_7d_start = now.saturating_sub(7 * DAY_SECONDS);
    let last_30d_start = now.saturating_sub(30 * DAY_SECONDS);

    let mut seen_session_keys = HashSet::new();
    for file in files {
        match parse_token_session_file(&file) {
            Ok(session) => {
                let session_key = session
                    .session_id
                    .clone()
                    .unwrap_or_else(|| fallback_session_key(&file));
                if !seen_session_keys.insert(session_key) {
                    continue;
                }
                snapshot.event_count += session.events.len();
                let events = dedupe_token_events(session.events);

                if !session.is_subagent_session {
                    add_window_delta_from_events(
                        &events,
                        last_7d_start,
                        now,
                        &mut snapshot.last_7d,
                    );
                    add_window_delta_from_events(
                        &events,
                        last_30d_start,
                        now,
                        &mut snapshot.last_30d,
                    );
                }
            }
            Err(_) => {
                snapshot.failed_path_count += 1;
            }
        }
    }

    snapshot
}

struct ParsedTokenSessionFile {
    session_id: Option<String>,
    events: Vec<ParsedTokenEvent>,
    is_subagent_session: bool,
}

fn parse_token_session_file(path: &Path) -> Result<ParsedTokenSessionFile, String> {
    let file = fs::File::open(path).map_err(|error| format!("读取 Codex 日志失败: {error}"))?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    let mut session_id = None;
    let mut is_subagent_session = false;

    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        if line_contains_subagent_session_meta(&line) {
            is_subagent_session = true;
        }
        if session_id.is_none() {
            session_id = parse_session_id_line(&line);
        }
        let Some(event) = parse_token_event_line(&line) else {
            continue;
        };

        events.push(event);
    }

    Ok(ParsedTokenSessionFile {
        session_id,
        events,
        is_subagent_session,
    })
}

fn parse_session_id_line(line: &str) -> Option<String> {
    let root = serde_json::from_str::<Value>(line).ok()?;
    if root.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }

    root.get("payload")
        .and_then(|payload| payload.get("id").or_else(|| payload.get("session_id")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn parse_token_event_line(line: &str) -> Option<ParsedTokenEvent> {
    let root = serde_json::from_str::<Value>(line).ok()?;
    if root.get("type")?.as_str()? != "event_msg" {
        return None;
    }

    let payload = root.get("payload")?;
    if payload.get("type")?.as_str()? != "token_count" {
        return None;
    }

    let timestamp = parse_timestamp(root.get("timestamp")?.as_str()?)?;
    let info = payload.get("info")?;
    let last = info.get("last_token_usage").and_then(parse_token_totals);
    let total = info.get("total_token_usage").and_then(parse_token_totals);
    if last.is_none() && total.is_none() {
        return None;
    }

    Some(ParsedTokenEvent {
        timestamp,
        last,
        total,
    })
}

fn line_contains_subagent_session_meta(line: &str) -> bool {
    let root = match serde_json::from_str::<Value>(line) {
        Ok(root) => root,
        Err(_) => return false,
    };

    if root.get("type").and_then(Value::as_str) != Some("session_meta") {
        return false;
    }

    let Some(payload) = root.get("payload") else {
        return false;
    };

    payload
        .get("source")
        .and_then(|source| source.get("subagent"))
        .is_some()
        || payload.get("subagent").is_some()
        || payload.get("parent_thread_id").is_some()
        || payload.get("thread_spawn").is_some()
}

fn parse_token_totals(value: &Value) -> Option<CodexTokenTotals> {
    if !value.is_object() {
        return None;
    }

    let input_tokens = field_u64(value, "input_tokens");
    let cached_input_tokens = field_u64(value, "cached_input_tokens");
    let output_tokens = field_u64(value, "output_tokens");
    let reasoning_output_tokens = field_u64(value, "reasoning_output_tokens");
    let total_tokens = field_u64(value, "total_tokens").unwrap_or_else(|| {
        input_tokens
            .unwrap_or(0)
            .saturating_add(output_tokens.unwrap_or(0))
    });

    Some(CodexTokenTotals {
        input_tokens: input_tokens.unwrap_or(0),
        cached_input_tokens: cached_input_tokens.unwrap_or(0),
        output_tokens: output_tokens.unwrap_or(0),
        reasoning_output_tokens: reasoning_output_tokens.unwrap_or(0),
        total_tokens,
    })
}

fn field_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key)?.as_u64()
}

fn parse_timestamp(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .map(|timestamp| timestamp.unix_timestamp())
}

fn collect_jsonl_files(path: &Path, files: &mut Vec<PathBuf>, failed_path_count: &mut usize) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        if path.exists() {
            *failed_path_count += 1;
        }
        return;
    };

    if metadata.is_file() {
        if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
            files.push(path.to_path_buf());
        }
        return;
    }

    if !metadata.is_dir() {
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        *failed_path_count += 1;
        return;
    };

    for entry in entries {
        match entry {
            Ok(entry) => collect_jsonl_files(&entry.path(), files, failed_path_count),
            Err(_) => *failed_path_count += 1,
        }
    }
}

fn is_duplicate_token_count_event(
    event: &ParsedTokenEvent,
    previous_total_tokens: &mut Option<u64>,
) -> bool {
    let Some(total) = event.total.as_ref() else {
        return false;
    };

    let current_total_tokens = total.total_tokens;
    if previous_total_tokens
        .map(|previous| previous == current_total_tokens)
        .unwrap_or(false)
    {
        return true;
    }

    *previous_total_tokens = Some(current_total_tokens);
    false
}

fn dedupe_token_events(events: Vec<ParsedTokenEvent>) -> Vec<ParsedTokenEvent> {
    let mut previous_total_tokens = None;
    let mut deduped = Vec::with_capacity(events.len());
    for event in events {
        if is_duplicate_token_count_event(&event, &mut previous_total_tokens) {
            continue;
        }
        deduped.push(event);
    }
    deduped
}

fn add_window_delta_from_events(
    events: &[ParsedTokenEvent],
    window_start: i64,
    window_end: i64,
    target: &mut CodexTokenTotals,
) {
    let mut previous_total = None;
    let has_cumulative_totals = events.iter().any(|event| event.total.is_some());

    for event in events {
        if let Some(total) = event.total.as_ref() {
            if event.timestamp >= window_start && event.timestamp <= window_end {
                let delta = previous_total
                    .as_ref()
                    .map(|previous| total.saturating_sub_totals(previous))
                    .or_else(|| {
                        event
                            .last
                            .as_ref()
                            .filter(|last| last.is_reasonable_single_delta())
                            .cloned()
                    });
                if let Some(delta) = delta {
                    target.add(&delta);
                }
            }
            if event.timestamp <= window_end {
                previous_total = Some(total.clone());
            }
            continue;
        }

        // Fallback for older or partial logs that only include per-event usage.
        if !has_cumulative_totals
            && event.timestamp >= window_start
            && event.timestamp <= window_end
        {
            if let Some(last) = event
                .last
                .as_ref()
                .filter(|last| last.is_reasonable_single_delta())
            {
                target.add(last);
            }
        }
    }
}

fn fallback_session_key(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

impl CodexTokenTotals {
    fn add(&mut self, other: &CodexTokenTotals) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(other.cached_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.reasoning_output_tokens = self
            .reasoning_output_tokens
            .saturating_add(other.reasoning_output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }

    fn saturating_sub_totals(&self, other: &CodexTokenTotals) -> CodexTokenTotals {
        CodexTokenTotals {
            input_tokens: self.input_tokens.saturating_sub(other.input_tokens),
            cached_input_tokens: self
                .cached_input_tokens
                .saturating_sub(other.cached_input_tokens),
            output_tokens: self.output_tokens.saturating_sub(other.output_tokens),
            reasoning_output_tokens: self
                .reasoning_output_tokens
                .saturating_sub(other.reasoning_output_tokens),
            total_tokens: self.total_tokens.saturating_sub(other.total_tokens),
        }
    }

    fn is_reasonable_single_delta(&self) -> bool {
        self.total_tokens <= MAX_REASONABLE_SINGLE_TOKEN_DELTA
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    fn event_line(timestamp: &str, total: u64, last: u64) -> String {
        serde_json::json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": total,
                        "cached_input_tokens": 10,
                        "output_tokens": 20,
                        "reasoning_output_tokens": 5,
                        "total_tokens": total
                    },
                    "last_token_usage": {
                        "input_tokens": last,
                        "cached_input_tokens": 1,
                        "output_tokens": 2,
                        "reasoning_output_tokens": 1,
                        "total_tokens": last
                    }
                }
            }
        })
        .to_string()
    }

    fn subagent_session_meta_line(session_id: &str, parent_thread_id: &str) -> String {
        serde_json::json!({
            "timestamp": "2026-04-28T05:59:59Z",
            "type": "session_meta",
            "payload": {
                "id": session_id,
                "source": {
                    "subagent": {
                        "thread_spawn": {
                            "parent_thread_id": parent_thread_id,
                            "depth": 1
                        }
                    }
                }
            }
        })
        .to_string()
    }

    fn simple_session_meta_line(session_id: &str) -> String {
        serde_json::json!({
            "timestamp": "2026-04-28T05:59:59Z",
            "type": "session_meta",
            "payload": {
                "id": session_id
            }
        })
        .to_string()
    }

    fn alternate_subagent_session_meta_line(session_id: &str, parent_thread_id: &str) -> String {
        serde_json::json!({
            "timestamp": "2026-04-28T05:59:59Z",
            "type": "session_meta",
            "payload": {
                "id": session_id,
                "parent_thread_id": parent_thread_id
            }
        })
        .to_string()
    }

    #[test]
    fn parses_codex_token_event_lines() {
        let event =
            parse_token_event_line(&event_line("2026-04-28T06:37:43.263Z", 40902952, 206498))
                .expect("token event");

        assert_eq!(event.timestamp, 1_777_358_263);
        assert_eq!(event.last.expect("last usage").total_tokens, 206_498);
        assert_eq!(event.total.expect("total usage").input_tokens, 40_902_952);
    }

    #[test]
    fn scans_windows_from_known_roots() {
        let root = unique_temp_dir();
        let sessions = root.join("sessions").join("2026").join("04").join("28");
        fs::create_dir_all(&sessions).expect("create sessions dir");
        fs::write(
            sessions.join("rollout-test.jsonl"),
            [
                event_line("2026-04-27T06:00:00Z", 100, 100),
                event_line("2026-04-28T06:00:00Z", 350, 250),
            ]
            .join("\n"),
        )
        .expect("write log");

        let snapshot = scan_codex_token_usage_roots(
            &[root.join("sessions"), root.join("archived_sessions")],
            1_777_361_000,
        );

        assert_eq!(snapshot.source_path_count, 1);
        assert_eq!(snapshot.event_count, 2);
        assert_eq!(snapshot.last_7d.total_tokens, 350);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ignores_duplicate_token_events_with_same_total() {
        let root = unique_temp_dir();
        let sessions = root.join("sessions").join("2026").join("04").join("28");
        fs::create_dir_all(&sessions).expect("create sessions dir");
        fs::write(
            sessions.join("rollout-duplicate-token-count.jsonl"),
            [
                event_line("2026-04-27T05:00:00Z", 100, 100),
                event_line("2026-04-28T05:30:00Z", 350, 250),
                event_line("2026-04-28T05:35:00Z", 350, 250),
            ]
            .join("\n"),
        )
        .expect("write log");

        let snapshot = scan_codex_token_usage_roots(
            &[root.join("sessions"), root.join("archived_sessions")],
            1_777_361_000,
        );

        assert_eq!(snapshot.event_count, 3);
        assert_eq!(snapshot.last_7d.total_tokens, 350);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn counts_only_cumulative_delta_inside_window_for_long_sessions() {
        let root = unique_temp_dir();
        let sessions = root.join("sessions").join("2026").join("04").join("28");
        fs::create_dir_all(&sessions).expect("create sessions dir");
        fs::write(
            sessions.join("long-running-session.jsonl"),
            [
                event_line("2026-04-27T05:00:00Z", 7_200_000_000, 7_200_000_000),
                event_line("2026-04-28T06:00:00Z", 7_200_001_200, 1_200),
            ]
            .join("\n"),
        )
        .expect("write log");

        let snapshot = scan_codex_token_usage_roots(
            &[root.join("sessions"), root.join("archived_sessions")],
            1_777_361_000,
        );

        assert_eq!(snapshot.event_count, 2);
        assert_eq!(snapshot.last_7d.total_tokens, 1_200);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uses_reasonable_first_event_last_usage_when_window_has_no_baseline() {
        let root = unique_temp_dir();
        let sessions = root.join("sessions").join("2026").join("04").join("28");
        fs::create_dir_all(&sessions).expect("create sessions dir");
        fs::write(
            sessions.join("new-session.jsonl"),
            [
                event_line("2026-04-28T05:00:00Z", 100, 100),
                event_line("2026-04-28T06:00:00Z", 350, 250),
            ]
            .join("\n"),
        )
        .expect("write log");

        let snapshot = scan_codex_token_usage_roots(
            &[root.join("sessions"), root.join("archived_sessions")],
            1_777_361_000,
        );

        assert_eq!(snapshot.last_7d.total_tokens, 350);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ignores_unreasonable_first_event_total_without_window_baseline() {
        let root = unique_temp_dir();
        let sessions = root.join("sessions").join("2026").join("04").join("28");
        fs::create_dir_all(&sessions).expect("create sessions dir");
        fs::write(
            sessions.join("huge-historical-session.jsonl"),
            [event_line(
                "2026-04-28T06:00:00Z",
                7_200_000_000,
                7_200_000_000,
            )]
            .join("\n"),
        )
        .expect("write log");

        let snapshot = scan_codex_token_usage_roots(
            &[root.join("sessions"), root.join("archived_sessions")],
            1_777_361_000,
        );

        assert_eq!(snapshot.last_7d.total_tokens, 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dedupes_same_session_across_active_and_archived_roots() {
        let root = unique_temp_dir();
        let active = root.join("sessions").join("2026").join("04").join("28");
        let archived = root
            .join("archived_sessions")
            .join("2026")
            .join("04")
            .join("28");
        fs::create_dir_all(&active).expect("create active dir");
        fs::create_dir_all(&archived).expect("create archived dir");
        let log = [
            simple_session_meta_line("same-session"),
            event_line("2026-04-28T05:00:00Z", 100, 100),
            event_line("2026-04-28T06:00:00Z", 350, 250),
        ]
        .join("\n");
        fs::write(active.join("active.jsonl"), &log).expect("write active");
        fs::write(archived.join("archived-copy.jsonl"), log).expect("write archived");

        let snapshot = scan_codex_token_usage_roots(
            &[root.join("sessions"), root.join("archived_sessions")],
            1_777_361_000,
        );

        assert_eq!(snapshot.source_path_count, 2);
        assert_eq!(snapshot.event_count, 2);
        assert_eq!(snapshot.last_7d.total_tokens, 350);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ignores_future_token_events() {
        let root = unique_temp_dir();
        let sessions = root.join("sessions").join("2026").join("04").join("28");
        fs::create_dir_all(&sessions).expect("create sessions dir");
        fs::write(
            sessions.join("future-session.jsonl"),
            [
                event_line("2026-04-28T06:00:00Z", 350, 350),
                event_line("2026-04-29T06:00:00Z", 9_000, 8_650),
            ]
            .join("\n"),
        )
        .expect("write log");

        let snapshot = scan_codex_token_usage_roots(
            &[root.join("sessions"), root.join("archived_sessions")],
            1_777_361_000,
        );

        assert_eq!(snapshot.last_7d.total_tokens, 350);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn excludes_alternate_subagent_session_meta_shape_from_window_totals() {
        let root = unique_temp_dir();
        let sessions = root.join("sessions").join("2026").join("04").join("28");
        fs::create_dir_all(&sessions).expect("create sessions dir");
        fs::write(
            sessions.join("alternate-subagent.jsonl"),
            [
                alternate_subagent_session_meta_line("subagent-session", "parent-thread"),
                event_line("2026-04-28T05:30:00Z", 200, 200),
                event_line("2026-04-28T06:30:00Z", 700, 500),
            ]
            .join("\n"),
        )
        .expect("write subagent log");

        let snapshot = scan_codex_token_usage_roots(
            &[root.join("sessions"), root.join("archived_sessions")],
            1_777_361_000,
        );

        assert_eq!(snapshot.last_7d.total_tokens, 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn excludes_subagent_sessions_from_window_totals() {
        let root = unique_temp_dir();
        let sessions = root.join("sessions").join("2026").join("04").join("28");
        fs::create_dir_all(&sessions).expect("create sessions dir");
        fs::write(
            sessions.join("top-level.jsonl"),
            [
                event_line("2026-04-27T06:00:00Z", 100, 100),
                event_line("2026-04-28T06:00:00Z", 350, 250),
            ]
            .join("\n"),
        )
        .expect("write top-level log");
        fs::write(
            sessions.join("subagent.jsonl"),
            [
                subagent_session_meta_line("subagent-session", "parent-thread"),
                event_line("2026-04-28T05:30:00Z", 200, 200),
                event_line("2026-04-28T06:30:00Z", 700, 500),
            ]
            .join("\n"),
        )
        .expect("write subagent log");

        let snapshot = scan_codex_token_usage_roots(
            &[root.join("sessions"), root.join("archived_sessions")],
            1_777_361_000,
        );

        assert_eq!(snapshot.source_path_count, 2);
        assert_eq!(snapshot.event_count, 4);
        assert_eq!(snapshot.last_7d.total_tokens, 350);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cache_day_uses_utc_day_boundaries() {
        assert_eq!(cache_day(0), 0);
        assert_eq!(cache_day(DAY_SECONDS - 1), 0);
        assert_eq!(cache_day(DAY_SECONDS), 1);
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "codex-tools-token-usage-{}-{nanos}",
            std::process::id()
        ))
    }
}
