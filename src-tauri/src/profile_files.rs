use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::Value;
use toml_edit::table;
use toml_edit::value;
use toml_edit::DocumentMut;
use uuid::Uuid;

use crate::app_paths;
use crate::auth;
use crate::models::AccountSourceKind;
use crate::models::ProxyEndpointCapability;
use crate::models::StoredAccount;
use crate::utils::redact_sensitive_text;
use crate::utils::set_private_permissions;

const PROFILE_DIR_NAME: &str = "profiles";
const PROFILE_AUTH_FILE_NAME: &str = "auth.json";
const PROFILE_CONFIG_FILE_NAME: &str = "config.toml";
const PROFILE_INCOMPLETE_MESSAGE: &str = "配置不完整";
const RELAY_INCOMPLETE_MESSAGE: &str = "API 条目资料不完整";
const VALIDATE_TIMEOUT_SECS: u64 = 18;
const CODEX_CREDENTIALS_STORE_KEY: &str = "cli_auth_credentials_store";
const CODEX_CREDENTIALS_STORE_FILE: &str = "file";
const CODEX_MODEL_KEY: &str = "model";
const CODEX_BASE_URL_KEY: &str = "openai_base_url";
const CODEX_CONTEXT_WINDOW_KEY: &str = "model_context_window";
const CODEX_AUTO_COMPACT_LIMIT_KEY: &str = "model_auto_compact_token_limit";
const CODEX_TOOL_OUTPUT_LIMIT_KEY: &str = "tool_output_token_limit";
const CODEX_MODEL_PROVIDER_KEY: &str = "model_provider";
const CODEX_MODEL_PROVIDERS_KEY: &str = "model_providers";
const CODEX_PROVIDER_NAME_KEY: &str = "name";
const CODEX_PROVIDER_BASE_URL_KEY: &str = "base_url";
const CODEX_PROVIDER_WIRE_API_KEY: &str = "wire_api";
const CODEX_PROVIDER_WIRE_API_RESPONSES: &str = "responses";
const CODEX_PROVIDER_REQUIRES_OPENAI_AUTH_KEY: &str = "requires_openai_auth";
const CODEX_PROVIDER_SUPPORTS_WEBSOCKETS_KEY: &str = "supports_websockets";
const CODEX_PROVIDER_EXPERIMENTAL_BEARER_TOKEN_KEY: &str = "experimental_bearer_token";
const CODEXDECK_RELAY_PROVIDER_ID: &str = "codexdeck_api";
const CODEX_FEATURES_TABLE_KEY: &str = "features";
const CODEX_RESPONSES_WEBSOCKETS_KEY: &str = "responses_websockets";
const CODEX_RESPONSES_WEBSOCKETS_V2_KEY: &str = "responses_websockets_v2";
const CODEX_SANDBOX_MODE_KEY: &str = "sandbox_mode";
const CODEX_APPROVAL_POLICY_KEY: &str = "approval_policy";
const CODEX_SANDBOX_TABLE_KEY: &str = "sandbox";
const CODEX_WINDOWS_TABLE_KEY: &str = "windows";
const CODEX_WINDOWS_SANDBOX_KEY: &str = "sandbox";
const CODEX_REPLACE_OR_REMOVE_ROOT_KEYS: &[&str] = &[
    CODEX_CREDENTIALS_STORE_KEY,
    CODEX_MODEL_KEY,
    CODEX_BASE_URL_KEY,
    CODEX_CONTEXT_WINDOW_KEY,
    CODEX_AUTO_COMPACT_LIMIT_KEY,
    CODEX_TOOL_OUTPUT_LIMIT_KEY,
    CODEX_MODEL_PROVIDER_KEY,
    CODEX_MODEL_PROVIDERS_KEY,
];
const CODEX_REPLACE_IF_PRESENT_ROOT_KEYS: &[&str] =
    &[CODEX_SANDBOX_MODE_KEY, CODEX_APPROVAL_POLICY_KEY];

pub(crate) struct RelayValidationResult {
    pub(crate) balance_text: Option<String>,
    pub(crate) endpoints: Vec<ProxyEndpointCapability>,
}

enum RelayEndpointProbeResult {
    Supported,
    Unsupported(String),
    Fatal(String),
}

pub(crate) fn profile_dir_from_store_path(store_path: &Path, id: &str) -> PathBuf {
    profile_root_from_store_path(store_path).join(id)
}

fn profile_root_from_store_path(store_path: &Path) -> PathBuf {
    store_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(PROFILE_DIR_NAME)
}

pub(crate) fn profile_auth_path_from_store_path(store_path: &Path, id: &str) -> PathBuf {
    profile_dir_from_store_path(store_path, id).join(PROFILE_AUTH_FILE_NAME)
}

pub(crate) fn profile_config_path_from_store_path(store_path: &Path, id: &str) -> PathBuf {
    profile_dir_from_store_path(store_path, id).join(PROFILE_CONFIG_FILE_NAME)
}

pub(crate) fn ensure_profile_metadata(store_path: &Path, account: &mut StoredAccount) -> bool {
    let mut changed = false;
    let auth_path = profile_auth_path_from_store_path(store_path, &account.id);
    let config_path = profile_config_path_from_store_path(store_path, &account.id);
    let auth_path_string = auth_path.to_string_lossy().to_string();
    let config_path_string = config_path.to_string_lossy().to_string();

    if account.profile_auth_path.as_deref() != Some(auth_path_string.as_str()) {
        account.profile_auth_path = Some(auth_path_string);
        changed = true;
    }
    if account.profile_config_path.as_deref() != Some(config_path_string.as_str()) {
        account.profile_config_path = Some(config_path_string);
        changed = true;
    }

    let auth_ready = auth_path.is_file();
    let config_ready = config_path.is_file();
    if account.profile_auth_ready != auth_ready {
        account.profile_auth_ready = auth_ready;
        changed = true;
    }
    if account.profile_config_ready != config_ready {
        account.profile_config_ready = config_ready;
        changed = true;
    }

    let integrity_error = compute_profile_integrity_error(account, auth_ready, config_ready);
    if account.profile_integrity_error != integrity_error {
        account.profile_integrity_error = integrity_error;
        changed = true;
    }

    changed
}

pub(crate) fn remove_account_profile_in_store_path(
    store_path: &Path,
    id: &str,
) -> Result<(), String> {
    let profile_dir = profile_dir_from_store_path(store_path, id);
    if !profile_dir.exists() {
        return Ok(());
    }
    if !profile_dir.is_dir() {
        return Err(format!(
            "账号 profile 路径不是目录，未删除 {}",
            profile_dir.display()
        ));
    }
    fs::remove_dir_all(&profile_dir).map_err(|error| {
        format!(
            "移除账号 profile 目录失败 {}: {error}",
            profile_dir.display()
        )
    })
}

pub(crate) fn cleanup_orphan_profiles_in_store_path(
    store_path: &Path,
    valid_ids: &HashSet<String>,
) -> Result<usize, String> {
    let profile_root = profile_root_from_store_path(store_path);
    if !profile_root.exists() {
        return Ok(0);
    }
    if !profile_root.is_dir() {
        return Err(format!(
            "账号 profile 根路径不是目录，未清理 {}",
            profile_root.display()
        ));
    }

    let entries = fs::read_dir(&profile_root).map_err(|error| {
        format!(
            "读取账号 profile 目录失败 {}: {error}",
            profile_root.display()
        )
    })?;
    let mut removed_count = 0usize;

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "读取账号 profile 条目失败 {}: {error}",
                profile_root.display()
            )
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if valid_ids.contains(id) {
            continue;
        }
        fs::remove_dir_all(&path).map_err(|error| {
            format!("移除孤儿账号 profile 目录失败 {}: {error}", path.display())
        })?;
        removed_count += 1;
    }

    Ok(removed_count)
}

pub(crate) fn sync_account_profile_in_store_path(
    store_path: &Path,
    account: &mut StoredAccount,
) -> Result<(), String> {
    let auth_path = profile_auth_path_from_store_path(store_path, &account.id);
    let config_path = profile_config_path_from_store_path(store_path, &account.id);
    let profile_dir = auth_path
        .parent()
        .ok_or_else(|| format!("无法解析账号 profile 目录 {}", auth_path.display()))?;
    fs::create_dir_all(profile_dir).map_err(|error| {
        format!(
            "创建账号 profile 目录失败 {}: {error}",
            profile_dir.display()
        )
    })?;

    let config_template =
        read_optional_text(&config_path)?.or(read_current_codex_config_optional()?);
    let config_text = match account.source_kind {
        AccountSourceKind::Chatgpt => build_chatgpt_profile_config(config_template.as_deref()),
        AccountSourceKind::Relay => build_relay_profile_config(
            config_template.as_deref(),
            account
                .api_base_url
                .as_deref()
                .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?,
            account
                .model_name
                .as_deref()
                .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?,
        ),
    };

    let auth_json = match account.source_kind {
        AccountSourceKind::Chatgpt => account.auth_json.clone(),
        AccountSourceKind::Relay => build_api_auth_json(
            account
                .primary_relay_api_key()
                .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?,
        ),
    };

    let serialized_auth = serde_json::to_string_pretty(&auth_json)
        .map_err(|error| format!("序列化账号 profile auth.json 失败: {error}"))?;
    write_file_atomically(&auth_path, serialized_auth.as_bytes())?;
    write_file_atomically(&config_path, config_text.as_bytes())?;

    account.profile_auth_path = Some(auth_path.to_string_lossy().to_string());
    account.profile_config_path = Some(config_path.to_string_lossy().to_string());
    account.profile_auth_ready = true;
    account.profile_config_ready = true;
    account.profile_integrity_error = None;
    Ok(())
}

pub(crate) fn apply_account_profile(account: &StoredAccount) -> Result<(), String> {
    let auth_path = account
        .profile_auth_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| PROFILE_INCOMPLETE_MESSAGE.to_string())?;
    let config_path = account
        .profile_config_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| PROFILE_INCOMPLETE_MESSAGE.to_string())?;

    if !auth_path.is_file() || !config_path.is_file() {
        return Err(account
            .profile_integrity_error
            .clone()
            .unwrap_or_else(|| PROFILE_INCOMPLETE_MESSAGE.to_string()));
    }

    let auth_contents = fs::read_to_string(&auth_path).map_err(|error| {
        format!(
            "读取账号 profile auth.json 失败 {}: {error}",
            auth_path.display()
        )
    })?;
    let auth_json: Value = serde_json::from_str(&auth_contents).map_err(|error| {
        format!(
            "账号 profile auth.json 不是合法 JSON {}: {error}",
            auth_path.display()
        )
    })?;
    auth::write_active_codex_auth(&auth_json)?;

    let config_contents = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "读取账号 profile config.toml 失败 {}: {error}",
            config_path.display()
        )
    })?;
    let active_config_path = current_codex_config_path()?;
    let parent = active_config_path
        .parent()
        .ok_or_else(|| format!("无法解析 Codex 配置目录 {}", active_config_path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 Codex 配置目录失败 {}: {error}", parent.display()))?;
    let active_config_contents = read_optional_text(&active_config_path)?;
    let merged_config_contents =
        merge_active_codex_profile_config(active_config_contents.as_deref(), &config_contents);
    write_file_atomically(&active_config_path, merged_config_contents.as_bytes())?;
    Ok(())
}

pub(crate) fn apply_hybrid_account_profile(
    chatgpt_account: &StoredAccount,
    relay_account: &StoredAccount,
) -> Result<(), String> {
    let base_url = relay_account
        .api_base_url
        .as_deref()
        .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?;
    apply_hybrid_account_profile_with_provider_base_url(chatgpt_account, relay_account, base_url)
}

pub(crate) fn apply_hybrid_account_profile_with_provider_base_url(
    chatgpt_account: &StoredAccount,
    relay_account: &StoredAccount,
    provider_base_url: &str,
) -> Result<(), String> {
    if !matches!(chatgpt_account.source_kind, AccountSourceKind::Chatgpt) {
        return Err("混合模式需要选择一个 ChatGPT 官方账号。".to_string());
    }
    if !matches!(relay_account.source_kind, AccountSourceKind::Relay) {
        return Err("混合模式需要选择一个 API 条目。".to_string());
    }

    let model_name = relay_account
        .model_name
        .as_deref()
        .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?;
    let api_key = relay_account
        .primary_relay_api_key()
        .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?;

    auth::write_active_codex_auth(&chatgpt_account.auth_json)?;

    let active_config_path = current_codex_config_path()?;
    let parent = active_config_path
        .parent()
        .ok_or_else(|| format!("无法解析 Codex 配置目录 {}", active_config_path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 Codex 配置目录失败 {}: {error}", parent.display()))?;
    let active_config_contents = read_optional_text(&active_config_path)?;
    let profile_config = build_hybrid_relay_profile_config(
        active_config_contents.as_deref(),
        provider_base_url,
        model_name,
        api_key,
    );
    let merged_config_contents =
        merge_active_codex_profile_config(active_config_contents.as_deref(), &profile_config);
    write_file_atomically(&active_config_path, merged_config_contents.as_bytes())?;
    Ok(())
}

pub(crate) fn build_api_auth_json(api_key: &str) -> Value {
    serde_json::json!({
        "OPENAI_API_KEY": api_key,
        "auth_mode": "apikey"
    })
}

pub(crate) fn relay_account_id(id: &str) -> String {
    format!("relay:{id}")
}

pub(crate) fn normalize_relay_label(label: &str) -> Result<String, String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err("请输入 API 名称。".to_string());
    }
    Ok(trimmed.to_string())
}

pub(crate) fn normalize_relay_model_name(model_name: &str) -> Result<String, String> {
    let trimmed = model_name.trim();
    if trimmed.is_empty() {
        return Err("请输入模型名称。".to_string());
    }
    Ok(trimmed.to_string())
}

pub(crate) fn normalize_relay_api_key(api_key: &str) -> Result<String, String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("请输入 API Key。".to_string());
    }
    if !trimmed.starts_with("sk-") {
        return Err("仅支持 OpenAI 格式 API Key，例如 sk-...".to_string());
    }
    Ok(trimmed.to_string())
}

pub(crate) fn normalize_relay_base_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("请输入 Base URL。".to_string());
    }
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err("Base URL 仅支持 http/https 地址。".to_string());
    }
    Ok(trimmed.to_string())
}

pub(crate) async fn validate_relay_target(
    base_url: &str,
    api_key: &str,
    model_name: &str,
) -> Result<RelayValidationResult, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(VALIDATE_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("创建 API 检测客户端失败: {error}"))?;

    let mut endpoints = Vec::new();
    let mut diagnostics = Vec::new();
    for endpoint in [
        ProxyEndpointCapability::Responses,
        ProxyEndpointCapability::ResponsesCompact,
        ProxyEndpointCapability::ChatCompletions,
    ] {
        match probe_relay_endpoint_capability(&client, base_url, api_key, model_name, endpoint)
            .await?
        {
            RelayEndpointProbeResult::Supported => endpoints.push(endpoint),
            RelayEndpointProbeResult::Unsupported(message) => diagnostics.push(message),
            RelayEndpointProbeResult::Fatal(message) => {
                if endpoints.is_empty() {
                    return Err(message);
                }
                diagnostics.push(message);
            }
        }
    }

    if endpoints.is_empty() {
        let detail = diagnostics
            .into_iter()
            .filter(|message| !message.trim().is_empty())
            .take(3)
            .collect::<Vec<_>>()
            .join("；");
        let suffix = if detail.is_empty() {
            "未检测到可用于 CodexDeck 的 OpenAI 兼容接口。".to_string()
        } else {
            format!("未检测到可用于 CodexDeck 的 OpenAI 兼容接口：{detail}")
        };
        return Err(format!("{suffix} 请确认 Base URL 已填写到 /v1。"));
    }

    Ok(RelayValidationResult {
        balance_text: fetch_relay_balance_best_effort(&client, base_url, api_key).await,
        endpoints,
    })
}

async fn probe_relay_endpoint_capability(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model_name: &str,
    endpoint: ProxyEndpointCapability,
) -> Result<RelayEndpointProbeResult, String> {
    let normalized = base_url.trim_end_matches('/');
    let (path, payload) = match endpoint {
        ProxyEndpointCapability::Responses => (
            "responses",
            serde_json::json!({
                "model": model_name,
                "input": "ping",
                "max_output_tokens": 1
            }),
        ),
        ProxyEndpointCapability::ResponsesCompact => (
            "responses/compact",
            serde_json::json!({
                "model": model_name,
                "previous_response_id": "resp_codex_tools_probe"
            }),
        ),
        ProxyEndpointCapability::ChatCompletions => (
            "chat/completions",
            serde_json::json!({
                "model": model_name,
                "messages": [
                    { "role": "user", "content": "ping" }
                ],
                "max_tokens": 1,
                "stream": false
            }),
        ),
        ProxyEndpointCapability::Realtime => {
            return Ok(RelayEndpointProbeResult::Unsupported(
                "Realtime 暂未自动探测。".to_string(),
            ));
        }
    };
    let response = client
        .post(format!("{normalized}/{path}"))
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
        .map_err(format_relay_validation_transport_error)?;

    let status = response.status();
    if status.is_success() {
        return Ok(RelayEndpointProbeResult::Supported);
    }

    let body = redact_sensitive_text(&response.text().await.unwrap_or_default());
    let normalized_body = body.to_ascii_lowercase();
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return Ok(RelayEndpointProbeResult::Fatal(
            "API Key 无效或无权访问该接口。".to_string(),
        ));
    }
    if response_body_mentions_endpoint_unsupported(status, &normalized_body) {
        return Ok(RelayEndpointProbeResult::Unsupported(format!(
            "/{path} 不可用: {}",
            truncate_message(&body)
        )));
    }
    if endpoint == ProxyEndpointCapability::ResponsesCompact
        && response_body_mentions_compact_probe_placeholder(&normalized_body)
    {
        return Ok(RelayEndpointProbeResult::Supported);
    }
    if status == StatusCode::BAD_REQUEST && normalized_body.contains("model") {
        return Ok(RelayEndpointProbeResult::Fatal(format!(
            "模型名称不可用: {}",
            truncate_message(&body)
        )));
    }
    Ok(RelayEndpointProbeResult::Fatal(format!(
        "/{path} 检测返回 {status}: {}",
        truncate_message(&body)
    )))
}

fn response_body_mentions_endpoint_unsupported(status: StatusCode, body: &str) -> bool {
    matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) || body.contains("endpoint not supported")
        || body.contains("endpoint is not supported")
        || body.contains("unsupported endpoint")
        || body.contains("unknown endpoint")
        || body.contains("unknown route")
        || body.contains("method not allowed")
        || body.contains("cannot post")
        || body.contains("not support")
        || body.contains("does not support")
}

fn response_body_mentions_compact_probe_placeholder(body: &str) -> bool {
    body.contains("resp_codex_tools_probe")
        || body.contains("previous_response_id")
        || body.contains("previous response")
        || body.contains("response not found")
        || body.contains("no response")
}

async fn fetch_relay_balance_best_effort(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Option<String> {
    let mut candidates = Vec::new();
    let normalized = base_url.trim_end_matches('/');
    candidates.push(format!("{normalized}/dashboard/billing/credit_grants"));
    if let Some(stripped) = normalized.strip_suffix("/v1") {
        candidates.push(format!("{stripped}/dashboard/billing/credit_grants"));
    }

    for endpoint in candidates {
        let Ok(response) = client.get(&endpoint).bearer_auth(api_key).send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(payload) = response.json::<Value>().await else {
            continue;
        };
        if let Some(value) = payload
            .get("total_available")
            .and_then(Value::as_f64)
            .map(|number| format!("${number:.2}"))
        {
            return Some(value);
        }
        if let Some(value) = payload
            .get("balance")
            .and_then(Value::as_str)
            .map(ToString::to_string)
        {
            return Some(value);
        }
    }

    None
}

fn compute_profile_integrity_error(
    account: &StoredAccount,
    auth_ready: bool,
    config_ready: bool,
) -> Option<String> {
    if matches!(account.source_kind, AccountSourceKind::Relay)
        && (account.api_base_url.as_deref().is_none()
            || account.primary_relay_api_key().is_none()
            || account.model_name.as_deref().is_none())
    {
        return Some(RELAY_INCOMPLETE_MESSAGE.to_string());
    }

    if auth_ready && config_ready {
        None
    } else {
        Some(PROFILE_INCOMPLETE_MESSAGE.to_string())
    }
}

fn build_chatgpt_profile_config(current_config: Option<&str>) -> String {
    let mut document = parse_config_or_default(current_config);
    let had_base_url = document.get(CODEX_BASE_URL_KEY).is_some();
    normalize_standard_profile_config(&mut document);
    remove_responses_websocket_flags(&mut document);
    document.remove(CODEX_BASE_URL_KEY);
    document.remove(CODEX_MODEL_PROVIDERS_KEY);
    if had_base_url {
        document.remove(CODEX_MODEL_KEY);
    }
    document.to_string()
}

fn build_relay_profile_config(
    current_config: Option<&str>,
    base_url: &str,
    model_name: &str,
) -> String {
    let mut document = parse_config_or_default(current_config);
    normalize_standard_profile_config(&mut document);
    disable_responses_websockets(&mut document);
    document[CODEX_BASE_URL_KEY] = value(base_url);
    document[CODEX_MODEL_KEY] = value(model_name);
    document[CODEX_MODEL_PROVIDER_KEY] = value(CODEXDECK_RELAY_PROVIDER_ID);
    document[CODEX_MODEL_PROVIDERS_KEY] = table();
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID] = table();
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID][CODEX_PROVIDER_NAME_KEY] =
        value(CODEXDECK_RELAY_PROVIDER_ID);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID][CODEX_PROVIDER_BASE_URL_KEY] =
        value(base_url);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID][CODEX_PROVIDER_WIRE_API_KEY] =
        value(CODEX_PROVIDER_WIRE_API_RESPONSES);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID]
        [CODEX_PROVIDER_REQUIRES_OPENAI_AUTH_KEY] = value(true);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID]
        [CODEX_PROVIDER_SUPPORTS_WEBSOCKETS_KEY] = value(false);
    document.to_string()
}

fn build_hybrid_relay_profile_config(
    current_config: Option<&str>,
    base_url: &str,
    model_name: &str,
    api_key: &str,
) -> String {
    let mut document = parse_config_or_default(current_config);
    normalize_standard_profile_config(&mut document);
    disable_responses_websockets(&mut document);
    document.remove(CODEX_BASE_URL_KEY);
    document[CODEX_MODEL_KEY] = value(model_name);
    document[CODEX_MODEL_PROVIDER_KEY] = value(CODEXDECK_RELAY_PROVIDER_ID);
    document[CODEX_MODEL_PROVIDERS_KEY] = table();
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID] = table();
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID][CODEX_PROVIDER_NAME_KEY] =
        value(CODEXDECK_RELAY_PROVIDER_ID);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID][CODEX_PROVIDER_BASE_URL_KEY] =
        value(base_url);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID][CODEX_PROVIDER_WIRE_API_KEY] =
        value(CODEX_PROVIDER_WIRE_API_RESPONSES);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID]
        [CODEX_PROVIDER_REQUIRES_OPENAI_AUTH_KEY] = value(true);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID]
        [CODEX_PROVIDER_EXPERIMENTAL_BEARER_TOKEN_KEY] = value(api_key);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID]
        [CODEX_PROVIDER_SUPPORTS_WEBSOCKETS_KEY] = value(false);
    document.to_string()
}

fn normalize_standard_profile_config(document: &mut DocumentMut) {
    document[CODEX_CREDENTIALS_STORE_KEY] = value(CODEX_CREDENTIALS_STORE_FILE);
    document.remove(CODEX_MODEL_PROVIDER_KEY);
}

pub(crate) fn relay_provider_id_for_account(account: &StoredAccount) -> Option<String> {
    account.api_base_url.as_deref()?;
    Some(CODEXDECK_RELAY_PROVIDER_ID.to_string())
}

fn disable_responses_websockets(document: &mut DocumentMut) {
    if document
        .get(CODEX_FEATURES_TABLE_KEY)
        .and_then(|item| item.as_table())
        .is_none()
    {
        document[CODEX_FEATURES_TABLE_KEY] = table();
    }

    document[CODEX_FEATURES_TABLE_KEY][CODEX_RESPONSES_WEBSOCKETS_KEY] = value(false);
    document[CODEX_FEATURES_TABLE_KEY][CODEX_RESPONSES_WEBSOCKETS_V2_KEY] = value(false);
}

fn remove_responses_websocket_flags(document: &mut DocumentMut) {
    let Some(features) = document
        .get_mut(CODEX_FEATURES_TABLE_KEY)
        .and_then(|item| item.as_table_mut())
    else {
        return;
    };

    features.remove(CODEX_RESPONSES_WEBSOCKETS_KEY);
    features.remove(CODEX_RESPONSES_WEBSOCKETS_V2_KEY);
    if features.is_empty() {
        document.remove(CODEX_FEATURES_TABLE_KEY);
    }
}

fn merge_active_codex_profile_config(active_config: Option<&str>, profile_config: &str) -> String {
    let Some(active_config) = active_config else {
        return profile_config.to_string();
    };
    let Ok(mut active_document) = active_config.parse::<DocumentMut>() else {
        return profile_config.to_string();
    };
    let Ok(profile_document) = profile_config.parse::<DocumentMut>() else {
        return profile_config.to_string();
    };

    copy_or_remove_root_keys(
        &mut active_document,
        &profile_document,
        CODEX_REPLACE_OR_REMOVE_ROOT_KEYS,
    );
    copy_if_present_root_keys(
        &mut active_document,
        &profile_document,
        CODEX_REPLACE_IF_PRESENT_ROOT_KEYS,
    );
    copy_table_if_present(
        &mut active_document,
        &profile_document,
        CODEX_SANDBOX_TABLE_KEY,
    );
    copy_table_value_if_present(
        &mut active_document,
        &profile_document,
        CODEX_WINDOWS_TABLE_KEY,
        CODEX_WINDOWS_SANDBOX_KEY,
    );
    copy_table_value_if_present(
        &mut active_document,
        &profile_document,
        CODEX_FEATURES_TABLE_KEY,
        CODEX_RESPONSES_WEBSOCKETS_KEY,
    );
    copy_table_value_if_present(
        &mut active_document,
        &profile_document,
        CODEX_FEATURES_TABLE_KEY,
        CODEX_RESPONSES_WEBSOCKETS_V2_KEY,
    );
    remove_table_value_if_missing(
        &mut active_document,
        &profile_document,
        CODEX_FEATURES_TABLE_KEY,
        CODEX_RESPONSES_WEBSOCKETS_KEY,
    );
    remove_table_value_if_missing(
        &mut active_document,
        &profile_document,
        CODEX_FEATURES_TABLE_KEY,
        CODEX_RESPONSES_WEBSOCKETS_V2_KEY,
    );

    active_document.to_string()
}

fn copy_or_remove_root_keys(target: &mut DocumentMut, source: &DocumentMut, keys: &[&str]) {
    for key in keys {
        if let Some(item) = source.get(key).cloned() {
            target[*key] = item;
        } else {
            target.remove(key);
        }
    }
}

fn copy_if_present_root_keys(target: &mut DocumentMut, source: &DocumentMut, keys: &[&str]) {
    for key in keys {
        if let Some(item) = source.get(key).cloned() {
            target[*key] = item;
        }
    }
}

fn copy_table_if_present(target: &mut DocumentMut, source: &DocumentMut, table_key: &str) {
    if let Some(item) = source.get(table_key).cloned() {
        target[table_key] = item;
    }
}

fn copy_table_value_if_present(
    target: &mut DocumentMut,
    source: &DocumentMut,
    table_key: &str,
    value_key: &str,
) {
    let Some(value_item) = source
        .get(table_key)
        .and_then(|item| item.as_table())
        .and_then(|table| table.get(value_key))
        .cloned()
    else {
        return;
    };

    if target
        .get(table_key)
        .and_then(|item| item.as_table())
        .is_none()
    {
        target[table_key] = table();
    }
    target[table_key][value_key] = value_item;
}

fn remove_table_value_if_missing(
    target: &mut DocumentMut,
    source: &DocumentMut,
    table_key: &str,
    value_key: &str,
) {
    let source_has_value = source
        .get(table_key)
        .and_then(|item| item.as_table())
        .and_then(|table| table.get(value_key))
        .is_some();
    if source_has_value {
        return;
    }

    let Some(table) = target
        .get_mut(table_key)
        .and_then(|item| item.as_table_mut())
    else {
        return;
    };
    table.remove(value_key);
    if table.is_empty() {
        target.remove(table_key);
    }
}

fn parse_config_or_default(current_config: Option<&str>) -> DocumentMut {
    current_config
        .and_then(|raw| raw.parse::<DocumentMut>().ok())
        .unwrap_or_default()
}

fn read_current_codex_config_optional() -> Result<Option<String>, String> {
    let path = current_codex_config_path()?;
    read_optional_text(&path)
}

fn current_codex_config_path() -> Result<PathBuf, String> {
    app_paths::codex_config_path()
}

fn truncate_message(message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.chars().count() <= 160 {
        trimmed.to_string()
    } else {
        let truncated = trimmed.chars().take(157).collect::<String>();
        format!("{truncated}...")
    }
}

fn format_relay_validation_transport_error(error: reqwest::Error) -> String {
    let detail = redact_sensitive_text(&error.to_string());
    format!("检测 API 失败 [已隐藏接口地址]: {detail}")
}

fn read_optional_text(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("读取文件失败 {}: {error}", path.display()))?;
    Ok(Some(raw))
}

fn write_file_atomically(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析目标目录 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建目标目录失败 {}: {error}", parent.display()))?;

    let temp_path = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("profile"),
        Uuid::new_v4()
    ));

    let write_result = (|| -> Result<(), String> {
        let mut temp_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|error| format!("创建临时文件失败 {}: {error}", temp_path.display()))?;
        temp_file
            .write_all(contents)
            .map_err(|error| format!("写入临时文件失败 {}: {error}", temp_path.display()))?;
        temp_file
            .sync_all()
            .map_err(|error| format!("刷新临时文件失败 {}: {error}", temp_path.display()))?;
        drop(temp_file);
        set_private_permissions(&temp_path);

        #[cfg(target_family = "unix")]
        {
            fs::rename(&temp_path, path).map_err(|error| {
                format!(
                    "替换目标文件失败 {} -> {}: {error}",
                    temp_path.display(),
                    path.display()
                )
            })?;

            let parent_dir = fs::File::open(parent)
                .map_err(|error| format!("打开目标目录失败 {}: {error}", parent.display()))?;
            parent_dir
                .sync_all()
                .map_err(|error| format!("刷新目标目录失败 {}: {error}", parent.display()))?;
        }

        #[cfg(not(target_family = "unix"))]
        {
            if path.exists() {
                fs::remove_file(path)
                    .map_err(|error| format!("移除旧文件失败 {}: {error}", path.display()))?;
            }
            fs::rename(&temp_path, path).map_err(|error| {
                format!(
                    "替换目标文件失败 {} -> {}: {error}",
                    temp_path.display(),
                    path.display()
                )
            })?;
        }

        set_private_permissions(path);
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }

    write_result
}

#[cfg(test)]
mod tests {
    use super::build_chatgpt_profile_config;
    use super::build_hybrid_relay_profile_config;
    use super::build_relay_profile_config;
    use super::cleanup_orphan_profiles_in_store_path;
    use super::merge_active_codex_profile_config;
    use super::profile_dir_from_store_path;
    use super::remove_account_profile_in_store_path;
    use super::truncate_message;
    use super::validate_relay_target;
    use crate::models::ProxyEndpointCapability;
    use serde_json::Value;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_store_path() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("codex-tools-profile-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir.join("accounts.json")
    }

    #[test]
    fn remove_account_profile_deletes_only_target_directory() {
        let store_path = temp_store_path();
        let target = profile_dir_from_store_path(&store_path, "target");
        let other = profile_dir_from_store_path(&store_path, "other");
        fs::create_dir_all(&target).expect("create target profile");
        fs::create_dir_all(&other).expect("create other profile");

        remove_account_profile_in_store_path(&store_path, "target").expect("remove profile");

        assert!(!target.exists());
        assert!(other.exists());
    }

    #[test]
    fn cleanup_orphan_profiles_keeps_known_ids() {
        let store_path = temp_store_path();
        let keep = profile_dir_from_store_path(&store_path, "keep");
        let orphan = profile_dir_from_store_path(&store_path, "orphan");
        fs::create_dir_all(&keep).expect("create kept profile");
        fs::create_dir_all(&orphan).expect("create orphan profile");
        let valid_ids = HashSet::from(["keep".to_string()]);

        let removed =
            cleanup_orphan_profiles_in_store_path(&store_path, &valid_ids).expect("cleanup");

        assert_eq!(removed, 1);
        assert!(keep.exists());
        assert!(!orphan.exists());
    }

    #[test]
    fn chatgpt_profile_config_uses_file_auth_and_clears_relay_provider_state() {
        let current = r#"model = "relay-model"
openai_base_url = "https://relay.example.com/v1"
model_provider = "old_proxy"

[model_providers.old_proxy]
name = "old_proxy"
base_url = "http://127.0.0.1:8787/v1"
wire_api = "responses"
"#;

        let config = build_chatgpt_profile_config(Some(current));

        assert!(config.contains(r#"cli_auth_credentials_store = "file""#));
        assert!(!config.contains("openai_base_url"));
        assert!(!config.contains("model ="));
        assert!(!config.contains("model_provider"));
        assert!(!config.contains("model_providers"));
        assert!(!config.contains("responses_websockets"));
        assert!(!config.contains("responses_websockets_v2"));
    }

    #[test]
    fn relay_profile_config_disables_responses_websockets() {
        let config = build_relay_profile_config(
            Some(
                r#"[features]
experimental_feature = true
responses_websockets = true
responses_websockets_v2 = true
"#,
            ),
            "https://relay.example.com/v1",
            "relay-model",
        );

        assert!(config.contains("[features]"));
        assert!(config.contains("experimental_feature = true"));
        assert!(config.contains("responses_websockets = false"));
        assert!(config.contains("responses_websockets_v2 = false"));
        assert!(config.contains(r#"openai_base_url = "https://relay.example.com/v1""#));
        assert!(config.contains(r#"model_provider = "codexdeck_api""#));
        assert!(config.contains("[model_providers.codexdeck_api]"));
        assert!(config.contains(r#"name = "codexdeck_api""#));
        assert!(config.contains(r#"base_url = "https://relay.example.com/v1""#));
        assert!(config.contains(r#"wire_api = "responses""#));
        assert!(config.contains("requires_openai_auth = true"));
        assert!(config.contains("supports_websockets = false"));
    }

    #[test]
    fn hybrid_profile_config_uses_codexdeck_provider_and_bearer_token() {
        let config = build_hybrid_relay_profile_config(
            Some(
                r#"openai_base_url = "https://old.example.com/v1"

[features]
experimental_feature = true
responses_websockets = true
responses_websockets_v2 = true
"#,
            ),
            "http://127.0.0.1:45123/v1",
            "gpt-5.5",
            "test-hybrid-token",
        );

        assert!(config.contains(r#"cli_auth_credentials_store = "file""#));
        assert!(config.contains(r#"model = "gpt-5.5""#));
        assert!(config.contains(r#"model_provider = "codexdeck_api""#));
        assert!(!config.contains("openai_base_url"));
        assert!(config.contains("[model_providers.codexdeck_api]"));
        assert!(config.contains(r#"name = "codexdeck_api""#));
        assert!(config.contains(r#"base_url = "http://127.0.0.1:45123/v1""#));
        assert!(!config.contains("relay.example.com"));
        assert!(config.contains(r#"wire_api = "responses""#));
        assert!(config.contains("requires_openai_auth = true"));
        assert!(config.contains(r#"experimental_bearer_token = "test-hybrid-token""#));
        assert!(config.contains("supports_websockets = false"));
        assert!(config.contains("experimental_feature = true"));
        assert!(config.contains("responses_websockets = false"));
        assert!(config.contains("responses_websockets_v2 = false"));
    }

    #[test]
    fn hybrid_profile_merge_preserves_user_tables_and_removes_legacy_base_url() {
        let active = r#"model = "old-model"
openai_base_url = "https://old.example.com/v1"
custom_setting = "keep"

[mcp_servers.filesystem]
command = "node"
args = ["server.js"]

[projects."C:\\Workspace\\Project"]
trust_level = "trusted"

[features]
experimental_feature = true
responses_websockets = true
"#;
        let profile = build_hybrid_relay_profile_config(
            Some(active),
            "https://relay.example.com/v1",
            "gpt-5.5",
            "test-hybrid-token",
        );

        let merged = merge_active_codex_profile_config(Some(active), &profile);

        assert!(merged.contains(r#"model = "gpt-5.5""#));
        assert!(merged.contains(r#"model_provider = "codexdeck_api""#));
        assert!(!merged.contains("openai_base_url"));
        assert!(merged.contains("[mcp_servers.filesystem]"));
        assert!(merged.contains(r#"args = ["server.js"]"#));
        assert!(merged.contains(r#"[projects."C:\\Workspace\\Project"]"#));
        assert!(merged.contains(r#"trust_level = "trusted""#));
        assert!(merged.contains(r#"custom_setting = "keep""#));
        assert!(merged.contains(r#"experimental_bearer_token = "test-hybrid-token""#));
        assert!(merged.contains("responses_websockets = false"));
        assert!(merged.contains("responses_websockets_v2 = false"));
    }

    #[test]
    fn profile_config_smart_merge_updates_only_switch_owned_keys() {
        let active = r#"model = "active-model"
openai_base_url = "https://old.example.com/v1"
model_context_window = 272000
model_auto_compact_token_limit = 258400
tool_output_token_limit = 50000
sandbox_mode = "workspace-write"
approval_policy = "on-request"
custom_setting = "keep-active"
model_provider = "old_proxy"

[mcp_servers.playwright]
command = "npx"
args = ["@playwright/mcp@latest"]

[mcp_servers.filesystem]
command = "node"
args = ["server.js"]

[model_providers.old_proxy]
name = "old_proxy"

[windows]
sandbox = "old-windows"
other = "keep-window"

[features]
experimental_feature = true
responses_websockets = true
responses_websockets_v2 = true
"#;
        let profile = r#"cli_auth_credentials_store = "file"
openai_base_url = "https://relay.example.com/v1"
model = "relay-model"
model_provider = "codexdeck_api"
model_context_window = 400000
model_auto_compact_token_limit = 380000
tool_output_token_limit = 100000
sandbox_mode = "danger-full-access"
approval_policy = "never"
custom_setting = "drop-profile"

[model_providers.codexdeck_api]
name = "codexdeck_api"
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = false

[mcp_servers.stale]
command = "stale-mcp"

[windows]
sandbox = "new-windows"
unrelated = "drop-profile-window"

[features]
responses_websockets = false
responses_websockets_v2 = false
"#;

        let merged = merge_active_codex_profile_config(Some(active), profile);

        assert!(merged.contains(r#"cli_auth_credentials_store = "file""#));
        assert!(merged.contains(r#"openai_base_url = "https://relay.example.com/v1""#));
        assert!(merged.contains(r#"model = "relay-model""#));
        assert!(merged.contains("model_context_window = 400000"));
        assert!(merged.contains("model_auto_compact_token_limit = 380000"));
        assert!(merged.contains("tool_output_token_limit = 100000"));
        assert!(merged.contains(r#"sandbox_mode = "danger-full-access""#));
        assert!(merged.contains(r#"approval_policy = "never""#));
        assert!(merged.contains(r#"custom_setting = "keep-active""#));
        assert!(merged.contains("[mcp_servers.playwright]"));
        assert!(merged.contains(r#"args = ["@playwright/mcp@latest"]"#));
        assert!(merged.contains("[mcp_servers.filesystem]"));
        assert!(merged.contains("[windows]"));
        assert!(merged.contains(r#"sandbox = "new-windows""#));
        assert!(merged.contains(r#"other = "keep-window""#));
        assert!(merged.contains("[features]"));
        assert!(merged.contains("experimental_feature = true"));
        assert!(merged.contains("responses_websockets = false"));
        assert!(merged.contains("responses_websockets_v2 = false"));
        assert!(!merged.contains("[mcp_servers.stale]"));
        assert!(!merged.contains("stale-mcp"));
        assert!(!merged.contains(r#"custom_setting = "drop-profile""#));
        assert!(merged.contains(r#"model_provider = "codexdeck_api""#));
        assert!(merged.contains("[model_providers.codexdeck_api]"));
        assert!(!merged.contains("drop-profile-window"));
    }

    #[test]
    fn chatgpt_profile_merge_removes_relay_keys_without_touching_other_config() {
        let active = r#"model = "relay-model"
openai_base_url = "https://relay.example.com/v1"
model_context_window = 400000
model_auto_compact_token_limit = 380000
tool_output_token_limit = 100000
custom_setting = "keep-active"
model_provider = "old_proxy"

[mcp_servers.current]
command = "current-mcp"

[model_providers.old_proxy]
name = "old_proxy"

[features]
experimental_feature = true
responses_websockets = false
responses_websockets_v2 = false
"#;
        let profile = r#"cli_auth_credentials_store = "file"
"#;

        let merged = merge_active_codex_profile_config(Some(active), profile);

        assert!(merged.contains(r#"cli_auth_credentials_store = "file""#));
        assert!(merged.contains("[mcp_servers.current]"));
        assert!(merged.contains(r#"command = "current-mcp""#));
        assert!(merged.contains(r#"custom_setting = "keep-active""#));
        assert!(!merged.contains("openai_base_url"));
        assert!(!merged.contains("model ="));
        assert!(!merged.contains("model_context_window"));
        assert!(!merged.contains("model_auto_compact_token_limit"));
        assert!(!merged.contains("tool_output_token_limit"));
        assert!(!merged.contains("model_provider"));
        assert!(!merged.contains("model_providers"));
        assert!(merged.contains("experimental_feature = true"));
        assert!(!merged.contains("responses_websockets"));
        assert!(!merged.contains("responses_websockets_v2"));
    }

    #[test]
    fn profile_sandbox_is_preserved_when_profile_does_not_define_it() {
        let active = r#"cli_auth_credentials_store = "file"
model = "gpt-5.5"
sandbox_mode = "workspace-write"
approval_policy = "on-request"

[sandbox]
network_access = true

[windows]
sandbox = "old-windows"
"#;
        let profile = r#"cli_auth_credentials_store = "file"
openai_base_url = "https://relay.example.com/v1"
model = "relay-model"
"#;

        let merged = merge_active_codex_profile_config(Some(active), profile);

        assert!(merged.contains(r#"openai_base_url = "https://relay.example.com/v1""#));
        assert!(merged.contains(r#"model = "relay-model""#));
        assert!(merged.contains(r#"sandbox_mode = "workspace-write""#));
        assert!(merged.contains(r#"approval_policy = "on-request""#));
        assert!(merged.contains("[sandbox]"));
        assert!(merged.contains("network_access = true"));
        assert!(merged.contains("[windows]"));
        assert!(merged.contains(r#"sandbox = "old-windows""#));
    }

    #[test]
    fn relay_validation_error_body_is_redacted_before_truncation() {
        let secret_key = ["sk", "profile-secret-1234567890"].join("-");
        let local_path = ["D:", "\\workspace\\secret"].concat();
        let upstream = ["https://", "api.example.invalid/v1"].concat();
        let redacted = crate::utils::redact_sensitive_text(&format!(
            "failed {secret_key} {local_path} {upstream}"
        ));
        let message = truncate_message(&redacted);

        assert!(!message.contains(&secret_key));
        assert!(!message.contains(&local_path));
        assert!(!message.contains("api.example.invalid"));
        assert!(message.contains("[已隐藏密钥]"));
        assert!(message.contains("[已隐藏本地路径]"));
    }

    #[test]
    fn relay_validation_transport_error_message_hides_endpoint_details() {
        let url = "not a valid url";
        let error = reqwest::Client::new()
            .get(url)
            .build()
            .expect_err("invalid url should produce a request build error");
        let message = super::format_relay_validation_transport_error(error);

        assert!(!message.contains(url));
        assert!(message.contains("[已隐藏接口地址]"));
    }

    #[tokio::test]
    async fn relay_validation_detects_chat_completions_only_endpoint() {
        use axum::extract::Request;
        use axum::http::StatusCode;
        use axum::routing::post;
        use axum::Router;
        use serde_json::json;
        use tokio::net::TcpListener;

        async fn responses_unsupported() -> StatusCode {
            StatusCode::NOT_FOUND
        }

        async fn compact_unsupported() -> StatusCode {
            StatusCode::METHOD_NOT_ALLOWED
        }

        async fn chat_completions(request: Request) -> (StatusCode, axum::Json<Value>) {
            let authorized = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                == Some("Bearer test-api-key-probe");
            if !authorized {
                return (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(json!({ "error": { "message": "unauthorized" } })),
                );
            }

            (
                StatusCode::OK,
                axum::Json(json!({
                    "id": "chatcmpl_probe",
                    "choices": [
                        { "message": { "role": "assistant", "content": "ok" } }
                    ]
                })),
            )
        }

        let app = Router::new()
            .route("/v1/responses", post(responses_unsupported))
            .route("/v1/responses/compact", post(compact_unsupported))
            .route("/v1/chat/completions", post(chat_completions));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe upstream");
        let addr = listener.local_addr().expect("probe upstream addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let result = validate_relay_target(
            &format!("http://{addr}/v1"),
            "test-api-key-probe",
            "upstream-chat-model",
        )
        .await
        .expect("validate chat-completions-only relay");

        assert_eq!(
            result.endpoints,
            vec![ProxyEndpointCapability::ChatCompletions]
        );
    }

    #[tokio::test]
    async fn relay_validation_does_not_treat_generic_compact_400_as_supported() {
        use axum::extract::Request;
        use axum::http::StatusCode;
        use axum::routing::post;
        use axum::Router;
        use serde_json::json;
        use tokio::net::TcpListener;

        async fn responses(request: Request) -> (StatusCode, axum::Json<Value>) {
            let authorized = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                == Some("Bearer test-api-key-probe");
            if !authorized {
                return (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(json!({ "error": { "message": "unauthorized" } })),
                );
            }

            (
                StatusCode::OK,
                axum::Json(json!({
                    "id": "resp_probe",
                    "output": []
                })),
            )
        }

        async fn compact_generic_bad_request() -> (StatusCode, axum::Json<Value>) {
            (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({
                    "error": {
                        "message": "invalid request payload"
                    }
                })),
            )
        }

        async fn chat_completions_unsupported() -> StatusCode {
            StatusCode::NOT_FOUND
        }

        let app = Router::new()
            .route("/v1/responses", post(responses))
            .route("/v1/responses/compact", post(compact_generic_bad_request))
            .route("/v1/chat/completions", post(chat_completions_unsupported));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe upstream");
        let addr = listener.local_addr().expect("probe upstream addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let result =
            validate_relay_target(
                &format!("http://{addr}/v1"),
                "test-api-key-probe",
                "upstream-model",
            )
                .await
                .expect("validate responses-only relay");

        assert_eq!(result.endpoints, vec![ProxyEndpointCapability::Responses]);
    }
}
