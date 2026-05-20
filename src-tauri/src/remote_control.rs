use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

#[cfg(any(target_os = "macos", all(unix, not(target_os = "macos"))))]
use std::process::Command;

use serde::Deserialize;
use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::app_paths;
use crate::utils;
use crate::utils::new_resolved_command;

const REMOTE_RUNTIME_DIR_ENV: &str = "CODEXDECK_REMOTE_RUNTIME_DIR";
const STATUS_URL: &str = "http://127.0.0.1:47992/api/state";
const RUNTIME_URL: &str = "http://127.0.0.1:47992/api/runtime";
const CAPABILITIES_URL: &str = "http://127.0.0.1:47992/api/capabilities";
const LOGS_URL: &str = "http://127.0.0.1:47992/api/logs";
const DEFAULT_PANEL_URL: &str = "http://127.0.0.1:47992/";
const START_SCRIPT: &str = "scripts/start-codex-command-console.ps1";
const STOP_SCRIPT: &str = "scripts/stop-codex-command-console.ps1";
const INSTALL_APK_SCRIPT: &str = "scripts/install-mobile-app.ps1";
const MOBILE_APK: &str = "mobile/Codex-Command-Mobile-debug.apk";
const MANIFEST_FILE: &str = "INSTALL-MANIFEST.json";
const START_READY_TIMEOUT: Duration = Duration::from_secs(120);
const START_READY_POLL: Duration = Duration::from_millis(750);

#[derive(Clone)]
struct RuntimeCandidate {
    root: PathBuf,
    source: RemoteRuntimeSource,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RemoteRuntimeSource {
    Env,
    UserDataCurrent,
    Resource,
    Missing,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteRuntimeManifest {
    pub(crate) built_at: Option<String>,
    pub(crate) repo_root: Option<String>,
    pub(crate) install_root: Option<String>,
    pub(crate) runtime_name: Option<String>,
    pub(crate) runtime_version: Option<String>,
    pub(crate) protocol_version: Option<String>,
    pub(crate) bridge_version: Option<String>,
    pub(crate) panel: Option<serde_json::Value>,
    pub(crate) scripts: Option<serde_json::Value>,
    pub(crate) ports: Option<serde_json::Value>,
    pub(crate) capabilities: Option<serde_json::Value>,
    pub(crate) mobile_apk: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteRuntimeDetection {
    pub(crate) available: bool,
    pub(crate) runtime_root: Option<String>,
    pub(crate) source: RemoteRuntimeSource,
    pub(crate) missing: Vec<String>,
    pub(crate) status_url: String,
    pub(crate) runtime_url: String,
    pub(crate) capabilities_url: String,
    pub(crate) logs_url: String,
    pub(crate) panel_url: String,
    pub(crate) manifest_path: Option<String>,
    pub(crate) mobile_apk_path: Option<String>,
    pub(crate) manifest: Option<RemoteRuntimeManifest>,
    pub(crate) checked_roots: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteCommandResult {
    pub(crate) ok: bool,
    pub(crate) code: Option<i32>,
    pub(crate) stdout_tail: String,
    pub(crate) stderr_tail: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteRuntimeState {
    pub(crate) updated_at: Option<String>,
    pub(crate) panel_status: Option<String>,
    pub(crate) bridge_status: Option<String>,
    pub(crate) relay_status: Option<String>,
    pub(crate) phone_status: Option<String>,
    pub(crate) desktop_status: Option<String>,
    pub(crate) relay_url: Option<String>,
    pub(crate) binding_code: Option<String>,
    pub(crate) manual_code: Option<String>,
    pub(crate) pairing_payload: Option<String>,
    pub(crate) expires_at: Option<String>,
    pub(crate) panel_url: Option<String>,
    pub(crate) desktop_port: Option<String>,
    pub(crate) desktop_target_id: Option<String>,
    pub(crate) last_error: Option<String>,
}

#[derive(Deserialize)]
struct RemoteStateEnvelope {
    state: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteStatusSnapshot {
    pub(crate) reachable: bool,
    pub(crate) state: Option<RemoteRuntimeState>,
    pub(crate) connection_address: Option<String>,
    pub(crate) connection_code: Option<String>,
    pub(crate) error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteJsonSnapshot {
    pub(crate) reachable: bool,
    pub(crate) data: Option<serde_json::Value>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteLogEntry {
    pub(crate) name: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) kind: Option<String>,
    #[serde(alias = "sizeBytes")]
    pub(crate) size: Option<u64>,
    pub(crate) modified_at: Option<String>,
    pub(crate) tail: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteLogsPayload {
    logs_dir: Option<String>,
    entries: Option<Vec<RemoteLogEntry>>,
    latest: Option<serde_json::Value>,
}

#[derive(Default, Deserialize)]
struct RemoteLogsEnvelope {
    logs: Option<RemoteLogsPayload>,
    #[serde(flatten)]
    payload: RemoteLogsPayload,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteLogsSnapshot {
    pub(crate) reachable: bool,
    pub(crate) logs_dir: Option<String>,
    pub(crate) entries: Vec<RemoteLogEntry>,
    pub(crate) latest: Option<serde_json::Value>,
    pub(crate) error: Option<String>,
}

pub(crate) async fn detect_runtime(app: &AppHandle) -> Result<RemoteRuntimeDetection, String> {
    let candidates = runtime_candidates(app)?;
    Ok(resolve_runtime_from_candidates(candidates))
}

pub(crate) async fn start_console(app: &AppHandle) -> Result<RemoteCommandResult, String> {
    let runtime = require_runtime(app)?;
    start_runtime_script_and_wait(
        runtime.root,
        START_SCRIPT,
        vec![
            "-NoBrowser".to_string(),
            "-TimeoutSec".to_string(),
            "90".to_string(),
        ],
    )
    .await
}

pub(crate) async fn stop_console(app: &AppHandle) -> Result<RemoteCommandResult, String> {
    let runtime = require_runtime(app)?;
    run_runtime_script(runtime.root, STOP_SCRIPT, Vec::new()).await
}

pub(crate) async fn restart_console(app: &AppHandle) -> Result<RemoteCommandResult, String> {
    let runtime = require_runtime(app)?;
    let stop_result = run_runtime_script(runtime.root.clone(), STOP_SCRIPT, Vec::new()).await;
    if let Err(error) = stop_result {
        log::warn!("停止远程控制运行时失败，仍尝试重新启动: {error}");
    }
    start_runtime_script_and_wait(
        runtime.root,
        START_SCRIPT,
        vec![
            "-NoBrowser".to_string(),
            "-TimeoutSec".to_string(),
            "90".to_string(),
        ],
    )
    .await
}

pub(crate) async fn get_status() -> Result<RemoteStatusSnapshot, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| format!("创建远程控制状态客户端失败: {error}"))?;

    let response = match client.get(STATUS_URL).send().await {
        Ok(response) => response,
        Err(error) => {
            return Ok(RemoteStatusSnapshot {
                reachable: false,
                state: None,
                connection_address: None,
                connection_code: None,
                error: Some(format!("远程控制运行时未响应: {error}")),
            });
        }
    };

    if !response.status().is_success() {
        return Ok(RemoteStatusSnapshot {
            reachable: false,
            state: None,
            connection_address: None,
            connection_code: None,
            error: Some(format!("远程控制状态接口返回 {}", response.status())),
        });
    }

    let envelope = response
        .json::<RemoteStateEnvelope>()
        .await
        .map_err(|error| format!("解析远程控制状态失败: {error}"))?;
    let state = remote_state_from_value(envelope.state)
        .map_err(|error| format!("解析远程控制状态失败: {error}"))?;
    let connection_address = relay_connection_address(state.relay_url.as_deref());
    let connection_code = preferred_connection_code(&state);

    Ok(RemoteStatusSnapshot {
        reachable: true,
        state: Some(state),
        connection_address,
        connection_code,
        error: None,
    })
}

pub(crate) async fn get_runtime_info() -> Result<RemoteJsonSnapshot, String> {
    get_json_snapshot(RUNTIME_URL, "运行时接口").await
}

pub(crate) async fn get_capabilities() -> Result<RemoteJsonSnapshot, String> {
    get_json_snapshot(CAPABILITIES_URL, "能力接口").await
}

pub(crate) async fn get_logs() -> Result<RemoteLogsSnapshot, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|error| format!("创建远程控制日志客户端失败: {error}"))?;

    let response = match client.get(LOGS_URL).send().await {
        Ok(response) => response,
        Err(error) => {
            return Ok(RemoteLogsSnapshot {
                reachable: false,
                logs_dir: None,
                entries: Vec::new(),
                latest: None,
                error: Some(format!("远程控制日志接口未响应: {error}")),
            });
        }
    };

    if !response.status().is_success() {
        return Ok(RemoteLogsSnapshot {
            reachable: false,
            logs_dir: None,
            entries: Vec::new(),
            latest: None,
            error: Some(format!("远程控制日志接口返回 {}", response.status())),
        });
    }

    match response.json::<RemoteLogsEnvelope>().await {
        Ok(envelope) => {
            let payload = envelope.logs.unwrap_or(envelope.payload);
            Ok(RemoteLogsSnapshot {
                reachable: true,
                logs_dir: payload.logs_dir,
                entries: payload.entries.unwrap_or_default(),
                latest: payload.latest,
                error: None,
            })
        }
        Err(error) => Ok(RemoteLogsSnapshot {
            reachable: false,
            logs_dir: None,
            entries: Vec::new(),
            latest: None,
            error: Some(format!("解析远程控制日志失败: {error}")),
        }),
    }
}

async fn get_json_snapshot(url: &str, label: &str) -> Result<RemoteJsonSnapshot, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|error| format!("创建远程控制{label}客户端失败: {error}"))?;

    let response = match client.get(url).send().await {
        Ok(response) => response,
        Err(error) => {
            return Ok(RemoteJsonSnapshot {
                reachable: false,
                data: None,
                error: Some(format!("远程控制{label}未响应: {error}")),
            });
        }
    };

    if !response.status().is_success() {
        return Ok(RemoteJsonSnapshot {
            reachable: false,
            data: None,
            error: Some(format!("远程控制{label}返回 {}", response.status())),
        });
    }

    match response.json::<serde_json::Value>().await {
        Ok(data) => Ok(RemoteJsonSnapshot {
            reachable: true,
            data: Some(data),
            error: None,
        }),
        Err(error) => Ok(RemoteJsonSnapshot {
            reachable: false,
            data: None,
            error: Some(format!("解析远程控制{label}失败: {error}")),
        }),
    }
}

pub(crate) async fn open_panel() -> Result<(), String> {
    let status = get_status().await?;
    let panel_url = status
        .state
        .as_ref()
        .and_then(|state| trimmed_option(state.panel_url.as_deref()))
        .unwrap_or_else(|| DEFAULT_PANEL_URL.to_string());
    open_http_url(&panel_url)
}

pub(crate) async fn install_mobile_apk(app: &AppHandle) -> Result<RemoteCommandResult, String> {
    let runtime = require_runtime(app)?;
    run_runtime_script(runtime.root, INSTALL_APK_SCRIPT, Vec::new()).await
}

pub(crate) async fn open_logs(app: &AppHandle) -> Result<(), String> {
    let runtime = require_runtime(app)?;
    let target = preferred_logs_path(app, &runtime.root)?;
    open_path(&target)
}

fn runtime_candidates(app: &AppHandle) -> Result<Vec<RuntimeCandidate>, String> {
    let mut candidates = Vec::new();

    if let Some(root) = env_runtime_root() {
        candidates.push(RuntimeCandidate {
            root,
            source: RemoteRuntimeSource::Env,
        });
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(RuntimeCandidate {
            root: resource_dir.join("codex-command-runtime"),
            source: RemoteRuntimeSource::Resource,
        });
    }

    candidates.push(RuntimeCandidate {
        root: app_paths::app_data_dir(app)?
            .join("remote-runtime")
            .join("current"),
        source: RemoteRuntimeSource::UserDataCurrent,
    });

    Ok(candidates)
}

fn env_runtime_root() -> Option<PathBuf> {
    let value = env::var(REMOTE_RUNTIME_DIR_ENV).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn resolve_runtime_from_candidates(candidates: Vec<RuntimeCandidate>) -> RemoteRuntimeDetection {
    let checked_roots = candidates
        .iter()
        .map(|candidate| candidate.root.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let mut best_missing = Vec::new();

    for candidate in candidates {
        let missing = missing_runtime_files(&candidate.root);
        if missing.is_empty() {
            let manifest_path = candidate.root.join(MANIFEST_FILE);
            let mobile_apk_path = candidate.root.join(MOBILE_APK);
            return RemoteRuntimeDetection {
                available: true,
                runtime_root: Some(candidate.root.to_string_lossy().to_string()),
                source: candidate.source,
                missing,
                status_url: STATUS_URL.to_string(),
                runtime_url: RUNTIME_URL.to_string(),
                capabilities_url: CAPABILITIES_URL.to_string(),
                logs_url: LOGS_URL.to_string(),
                panel_url: DEFAULT_PANEL_URL.to_string(),
                manifest_path: Some(manifest_path.to_string_lossy().to_string()),
                mobile_apk_path: Some(mobile_apk_path.to_string_lossy().to_string()),
                manifest: read_manifest(&candidate.root),
                checked_roots,
            };
        }
        if best_missing.is_empty() {
            best_missing = missing;
        }
    }

    RemoteRuntimeDetection {
        available: false,
        runtime_root: None,
        source: RemoteRuntimeSource::Missing,
        missing: best_missing,
        status_url: STATUS_URL.to_string(),
        runtime_url: RUNTIME_URL.to_string(),
        capabilities_url: CAPABILITIES_URL.to_string(),
        logs_url: LOGS_URL.to_string(),
        panel_url: DEFAULT_PANEL_URL.to_string(),
        manifest_path: None,
        mobile_apk_path: None,
        manifest: None,
        checked_roots,
    }
}

fn require_runtime(app: &AppHandle) -> Result<RuntimeCandidate, String> {
    for candidate in runtime_candidates(app)? {
        let missing = missing_runtime_files(&candidate.root);
        if missing.is_empty() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "未检测到 Codex Command runtime。请确认安装包包含内置运行时；开发环境也可以设置 {REMOTE_RUNTIME_DIR_ENV}。"
    ))
}

fn missing_runtime_files(root: &Path) -> Vec<String> {
    let mut missing = Vec::new();
    if !root.is_dir() {
        missing.push(root.to_string_lossy().to_string());
        return missing;
    }

    for relative in [
        START_SCRIPT,
        STOP_SCRIPT,
        INSTALL_APK_SCRIPT,
        MOBILE_APK,
        MANIFEST_FILE,
    ] {
        if !root.join(relative).is_file() {
            missing.push(relative.to_string());
        }
    }

    missing
}

fn read_manifest(root: &Path) -> Option<RemoteRuntimeManifest> {
    let content = fs::read_to_string(root.join(MANIFEST_FILE)).ok()?;
    match serde_json::from_str::<RemoteRuntimeManifest>(&content) {
        Ok(manifest) => Some(manifest),
        Err(error) => {
            log::warn!("解析远程控制安装版 manifest 失败: {error}");
            None
        }
    }
}

fn powershell_path_arg(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        value.to_string()
    }
}

async fn start_runtime_script_and_wait(
    runtime_root: PathBuf,
    relative_script: &'static str,
    args: Vec<String>,
) -> Result<RemoteCommandResult, String> {
    start_runtime_script_detached(runtime_root, relative_script, args).await?;
    wait_for_runtime_ready().await
}

async fn run_runtime_script(
    runtime_root: PathBuf,
    relative_script: &'static str,
    args: Vec<String>,
) -> Result<RemoteCommandResult, String> {
    let script_path = runtime_root.join(relative_script);
    if !script_path.is_file() {
        return Err(format!("远程控制脚本不存在: {}", script_path.display()));
    }

    tauri::async_runtime::spawn_blocking(move || {
        let script_arg = powershell_path_arg(&script_path);
        let mut command = new_resolved_command("powershell");
        command
            .current_dir(&runtime_root)
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&script_arg);
        for arg in args {
            command.arg(arg);
        }

        let output = command.output().map_err(|error| {
            format!(
                "调用远程控制脚本失败 {}: {error}。请确认 Windows PowerShell 可用。",
                script_path.display()
            )
        })?;
        command_result_from_output(output)
    })
    .await
    .map_err(|error| format!("等待远程控制脚本结束失败: {error}"))?
}

async fn start_runtime_script_detached(
    runtime_root: PathBuf,
    relative_script: &'static str,
    args: Vec<String>,
) -> Result<(), String> {
    let script_path = runtime_root.join(relative_script);
    if !script_path.is_file() {
        return Err(format!("远程控制脚本不存在: {}", script_path.display()));
    }

    tauri::async_runtime::spawn_blocking(move || {
        let script_arg = powershell_path_arg(&script_path);
        let mut command = new_resolved_command("powershell");
        command
            .current_dir(&runtime_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&script_arg);
        for arg in args {
            command.arg(arg);
        }

        command.spawn().map(|_| ()).map_err(|error| {
            format!(
                "启动远程控制脚本失败 {}: {error}。请确认 Windows PowerShell 可用。",
                script_path.display()
            )
        })
    })
    .await
    .map_err(|error| format!("启动远程控制脚本任务失败: {error}"))?
}

async fn wait_for_runtime_ready() -> Result<RemoteCommandResult, String> {
    let started = Instant::now();

    loop {
        let detail = match get_status().await {
            Ok(snapshot) if remote_snapshot_ready(&snapshot) => {
                return Ok(RemoteCommandResult {
                    ok: true,
                    code: Some(0),
                    stdout_tail: "远程控制台已启动，连接码和二维码数据已生成。".to_string(),
                    stderr_tail: String::new(),
                });
            }
            Ok(snapshot) => format_wait_snapshot(&snapshot),
            Err(error) => error,
        };

        if started.elapsed() >= START_READY_TIMEOUT {
            return Err(format!("远程控制台启动超时: {detail}"));
        }

        tokio::time::sleep(START_READY_POLL).await;
    }
}

fn remote_snapshot_ready(snapshot: &RemoteStatusSnapshot) -> bool {
    let Some(state) = snapshot.state.as_ref() else {
        return false;
    };

    let has_error = trimmed_option(state.last_error.as_deref()).is_some();
    snapshot.reachable
        && !has_error
        && status_in(state.bridge_status.as_deref(), &["running"])
        && status_in(state.relay_status.as_deref(), &["connected", "running"])
        && status_in(state.desktop_status.as_deref(), &["ready", "running"])
        && preferred_connection_code(state).is_some()
        && trimmed_option(state.pairing_payload.as_deref()).is_some()
}

fn status_in(value: Option<&str>, allowed: &[&str]) -> bool {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    allowed
        .iter()
        .any(|allowed_value| value.eq_ignore_ascii_case(allowed_value))
}

fn format_wait_snapshot(snapshot: &RemoteStatusSnapshot) -> String {
    if let Some(error) = trimmed_option(snapshot.error.as_deref()) {
        return error;
    }

    let Some(state) = snapshot.state.as_ref() else {
        return "远程控制运行时尚未返回状态".to_string();
    };

    if let Some(error) = trimmed_option(state.last_error.as_deref()) {
        return error;
    }

    format!(
        "bridge={}, relay={}, desktop={}, code={}, payload={}",
        state.bridge_status.as_deref().unwrap_or("unknown"),
        state.relay_status.as_deref().unwrap_or("unknown"),
        state.desktop_status.as_deref().unwrap_or("unknown"),
        if preferred_connection_code(state).is_some() {
            "ready"
        } else {
            "missing"
        },
        if trimmed_option(state.pairing_payload.as_deref()).is_some() {
            "ready"
        } else {
            "missing"
        }
    )
}

fn command_result_from_output(output: Output) -> Result<RemoteCommandResult, String> {
    let result = RemoteCommandResult {
        ok: output.status.success(),
        code: output.status.code(),
        stdout_tail: output_text_tail(&output.stdout),
        stderr_tail: output_text_tail(&output.stderr),
    };

    if result.ok {
        Ok(result)
    } else {
        let detail = if !result.stderr_tail.is_empty() {
            result.stderr_tail.clone()
        } else if !result.stdout_tail.is_empty() {
            result.stdout_tail.clone()
        } else {
            format!("退出码 {:?}", result.code)
        };
        Err(format!("远程控制脚本执行失败: {detail}"))
    }
}

fn output_text_tail(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes).trim().to_string();
    let redacted = utils::redact_sensitive_text(&raw);
    tail_chars(&redacted, 1200)
}

fn tail_chars(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }

    let tail = value
        .chars()
        .skip(char_count.saturating_sub(max_chars))
        .collect::<String>();
    format!("...{tail}")
}

fn relay_connection_address(relay_url: Option<&str>) -> Option<String> {
    let relay_url = trimmed_option(relay_url)?;
    let without_scheme = relay_url
        .strip_prefix("ws://")
        .or_else(|| relay_url.strip_prefix("wss://"))
        .unwrap_or(&relay_url);
    let without_suffix = without_scheme
        .strip_suffix("/relay")
        .unwrap_or(without_scheme);
    trimmed_option(Some(without_suffix))
}

fn preferred_connection_code(state: &RemoteRuntimeState) -> Option<String> {
    trimmed_option(state.manual_code.as_deref())
        .or_else(|| trimmed_option(state.binding_code.as_deref()))
}

fn remote_state_from_value(
    mut value: serde_json::Value,
) -> Result<RemoteRuntimeState, serde_json::Error> {
    if let Some(object) = value.as_object_mut() {
        let legacy_key = ["pairing", "Payload", "Json"].concat();
        if let Some(payload) = object.remove(&legacy_key) {
            object.entry("pairingPayload").or_insert(payload);
        }
    }

    serde_json::from_value(value)
}

fn trimmed_option(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn preferred_logs_path(app: &AppHandle, runtime_root: &Path) -> Result<PathBuf, String> {
    for candidate in [
        runtime_root.join("logs"),
        runtime_root.join("runtime").join("logs"),
        app_paths::app_data_dir(app)?
            .join("remote-runtime")
            .join("logs"),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Ok(runtime_root.to_path_buf())
}

fn open_http_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("仅允许打开 http/https 链接".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|error| format!("打开远程控制台失败: {error}"))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let mut primary = utils::new_background_command("rundll32.exe");
        primary
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .or_else(|primary_error| {
                let mut fallback = utils::new_background_command("explorer.exe");
                fallback.arg(url).spawn().map_err(|fallback_error| {
                    format!(
                        "打开远程控制台失败: rundll32={primary_error}; explorer={fallback_error}"
                    )
                })
            })?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|error| format!("打开远程控制台失败: {error}"))?;
        return Ok(());
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = url;
        Err("当前平台暂不支持打开远程控制台".to_string())
    }
}

fn open_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        fs::create_dir_all(path)
            .map_err(|error| format!("创建远程控制日志目录失败 {}: {error}", path.display()))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("打开远程控制日志失败: {error}"))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = utils::new_background_command("explorer.exe");
        command
            .arg(path)
            .spawn()
            .map_err(|error| format!("打开远程控制日志失败: {error}"))?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("打开远程控制日志失败: {error}"))?;
        return Ok(());
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = path;
        Err("当前平台暂不支持打开远程控制日志".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::powershell_path_arg;
    use std::path::Path;

    #[test]
    fn powershell_path_arg_removes_windows_extended_path_prefix() {
        assert_eq!(
            powershell_path_arg(Path::new(
                r"\\?\D:\CodexDeckTest\codex-command-runtime\scripts\start.ps1"
            )),
            r"D:\CodexDeckTest\codex-command-runtime\scripts\start.ps1"
        );
        assert_eq!(
            powershell_path_arg(Path::new(r"\\?\UNC\server\share\start.ps1")),
            r"\\server\share\start.ps1"
        );
    }
}
