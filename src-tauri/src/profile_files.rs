use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::Url;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use toml_edit::table;
use toml_edit::value;
use toml_edit::DocumentMut;
use uuid::Uuid;

use crate::app_paths;
use crate::auth;
use crate::cli;
use crate::models::AccountSourceKind;
use crate::models::ActiveHybridProfile;
use crate::models::ProxyEndpointCapability;
use crate::models::RelayModelCatalogEntry;
use crate::models::StoredAccount;
use crate::utils::redact_sensitive_text;
use crate::utils::set_private_permissions;

const PROFILE_DIR_NAME: &str = "profiles";
const PROFILE_AUTH_FILE_NAME: &str = "auth.json";
const PROFILE_CONFIG_FILE_NAME: &str = "config.toml";
const PROFILE_BACKUP_STATE_FILE_NAME: &str = "state.json";
const PROFILE_INCOMPLETE_MESSAGE: &str = "配置不完整";
const RELAY_INCOMPLETE_MESSAGE: &str = "API 条目资料不完整";
const VALIDATE_TIMEOUT_SECS: u64 = 18;
const CODEX_CREDENTIALS_STORE_KEY: &str = "cli_auth_credentials_store";
const CODEX_CREDENTIALS_STORE_FILE: &str = "file";
const CODEX_MODEL_KEY: &str = "model";
const CODEX_MODEL_CATALOG_JSON_KEY: &str = "model_catalog_json";
const CODEX_BASE_URL_KEY: &str = "openai_base_url";
const CODEX_MODEL_INSTRUCTIONS_FILE_KEY: &str = "model_instructions_file";
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
const OPENAI_API_KEY_AUTH_KEY: &str = "OPENAI_API_KEY";
const CODEX_FEATURES_TABLE_KEY: &str = "features";
const CODEX_RESPONSES_WEBSOCKETS_KEY: &str = "responses_websockets";
const CODEX_RESPONSES_WEBSOCKETS_V2_KEY: &str = "responses_websockets_v2";
const CODEX_PLUGINS_KEY: &str = "plugins";
const CODEX_APPS_KEY: &str = "apps";
const CODEX_IMAGE_GENERATION_KEY: &str = "image_generation";
const CODEX_LEGACY_HYBRID_TOOL_FEATURE_KEYS: &[&str] = &[
    CODEX_PLUGINS_KEY,
    CODEX_APPS_KEY,
    CODEX_IMAGE_GENERATION_KEY,
];
const CODEX_SANDBOX_MODE_KEY: &str = "sandbox_mode";
const CODEX_APPROVAL_POLICY_KEY: &str = "approval_policy";
const CODEX_SANDBOX_TABLE_KEY: &str = "sandbox";
const CODEX_WINDOWS_TABLE_KEY: &str = "windows";
const CODEX_WINDOWS_SANDBOX_KEY: &str = "sandbox";
const CODEXDECK_MODEL_CATALOG_FILE_NAME: &str = "codexdeck-model-catalog.json";
const CODEXDECK_BUNDLED_MODEL_CATALOG_FILE_NAME: &str = "codexdeck-bundled-model-catalog.json";
const CODEX_MODEL_INSTRUCTIONS_FIX_FILE_NAME: &str = "gpt-5.5-base-instructions.md";
const CODEX_MODEL_INSTRUCTIONS_FIX_CONTENT: &str =
    include_str!("../../resources/gpt-5.5-base-instructions.md");
const CODEX_MODELS_CACHE_FILE_NAME: &str = "models_cache.json";
const CODEX_MODELS_CACHE_CLIENT_VERSION_FALLBACK: &str = "0.131.0";
const MANAGED_AGENT_PREFIX: &str = "codexdeck-";
const MANAGED_AGENT_MARKER: &str = "# codexdeck-managed = true";
const CODEX_REPLACE_OR_REMOVE_ROOT_KEYS: &[&str] = &[
    CODEX_CREDENTIALS_STORE_KEY,
    CODEX_MODEL_KEY,
    CODEX_MODEL_CATALOG_JSON_KEY,
    CODEX_BASE_URL_KEY,
    CODEX_CONTEXT_WINDOW_KEY,
    CODEX_AUTO_COMPACT_LIMIT_KEY,
    CODEX_TOOL_OUTPUT_LIMIT_KEY,
    CODEX_MODEL_PROVIDER_KEY,
    CODEX_MODEL_PROVIDERS_KEY,
];
const CODEX_REPLACE_IF_PRESENT_ROOT_KEYS: &[&str] =
    &[CODEX_SANDBOX_MODE_KEY, CODEX_APPROVAL_POLICY_KEY];
const CODEX_COPIED_FEATURE_KEYS: &[&str] = &[
    CODEX_RESPONSES_WEBSOCKETS_KEY,
    CODEX_RESPONSES_WEBSOCKETS_V2_KEY,
];

pub(crate) struct RelayValidationResult {
    pub(crate) balance_text: Option<String>,
    pub(crate) endpoints: Vec<ProxyEndpointCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActiveCodexProfileBackupState {
    pub(crate) active_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) active_hybrid_profile: Option<ActiveHybridProfile>,
    #[serde(default)]
    pub(crate) codex_auth_existed: bool,
    #[serde(default)]
    pub(crate) codex_config_existed: bool,
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
        AccountSourceKind::Relay => {
            let base_url = normalize_relay_base_url(
                account
                    .api_base_url
                    .as_deref()
                    .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?,
            )?;
            let model_catalog_path = ensure_account_model_catalog_json(account)
                .map(Some)
                .or_else(|error| {
                    log::warn!(
                        "生成 API 账号模型 catalog 失败，继续使用单模型配置: {}",
                        redact_sensitive_text(&error)
                    );
                    Ok::<Option<PathBuf>, String>(None)
                })?;
            build_relay_profile_config(
                config_template.as_deref(),
                base_url.as_str(),
                account
                    .model_name
                    .as_deref()
                    .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?,
                model_catalog_path.as_deref(),
            )
        }
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
    write_file_if_changed(&auth_path, serialized_auth.as_bytes())?;
    write_file_if_changed(&config_path, config_text.as_bytes())?;

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
    let profile_config_contents = match account.source_kind {
        AccountSourceKind::Chatgpt => config_contents,
        AccountSourceKind::Relay => {
            let base_url = normalize_relay_base_url(
                account
                    .api_base_url
                    .as_deref()
                    .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?,
            )?;
            let model_catalog_path = ensure_account_model_catalog_json(account)
                .map(Some)
                .or_else(|error| {
                    log::warn!(
                        "生成 API 账号模型 catalog 失败，继续使用单模型配置: {}",
                        redact_sensitive_text(&error)
                    );
                    Ok::<Option<PathBuf>, String>(None)
                })?;
            build_relay_profile_config(
                Some(config_contents.as_str()),
                base_url.as_str(),
                account
                    .model_name
                    .as_deref()
                    .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?,
                model_catalog_path.as_deref(),
            )
        }
    };
    let active_config_path = current_codex_config_path()?;
    let parent = active_config_path
        .parent()
        .ok_or_else(|| format!("无法解析 Codex 配置目录 {}", active_config_path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 Codex 配置目录失败 {}: {error}", parent.display()))?;
    let active_config_contents = read_optional_text(&active_config_path)?;
    let merged_config_contents = merge_active_codex_profile_config(
        active_config_contents.as_deref(),
        &profile_config_contents,
    );
    write_file_atomically(&active_config_path, merged_config_contents.as_bytes())?;
    if matches!(account.source_kind, AccountSourceKind::Relay) {
        sync_codexdeck_model_assets()?;
    }
    Ok(())
}

pub(crate) fn apply_relay_account_profile_with_provider_base_url(
    account: &StoredAccount,
    provider_base_url: &str,
    model_catalog_entries: &[RelayModelCatalogEntry],
) -> Result<(), String> {
    if !matches!(account.source_kind, AccountSourceKind::Relay) {
        return Err("只有 API 条目支持本地路由模式。".to_string());
    }

    let model_name = account
        .model_name
        .as_deref()
        .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?;
    let api_key = account
        .primary_relay_api_key()
        .ok_or_else(|| RELAY_INCOMPLETE_MESSAGE.to_string())?;
    let provider_base_url = normalize_relay_base_url(provider_base_url)?;
    let auth_json = build_api_auth_json(api_key);

    let active_config_path = current_codex_config_path()?;
    let parent = active_config_path
        .parent()
        .ok_or_else(|| format!("无法解析 Codex 配置目录 {}", active_config_path.display()))?;
    let active_config_contents = read_optional_text(&active_config_path)?;
    let model_catalog_json_path = ensure_model_catalog_json_for_entries(model_catalog_entries)
        .or_else(|_| ensure_account_model_catalog_json(account))?;
    let profile_config = build_relay_profile_config(
        active_config_contents.as_deref(),
        provider_base_url.as_str(),
        model_name,
        Some(&model_catalog_json_path),
    );
    let merged_config_contents =
        merge_active_codex_profile_config(active_config_contents.as_deref(), &profile_config);

    auth::write_active_codex_auth(&auth_json)?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 Codex 配置目录失败 {}: {error}", parent.display()))?;
    write_file_atomically(&active_config_path, merged_config_contents.as_bytes())?;
    sync_codexdeck_model_assets()?;
    Ok(())
}

pub(crate) fn create_active_codex_profile_backup(
    backup_dir: &Path,
    state: &ActiveCodexProfileBackupState,
) -> Result<(), String> {
    fs::create_dir_all(backup_dir)
        .map_err(|error| format!("创建路由模式备份目录失败 {}: {error}", backup_dir.display()))?;
    let auth_path = app_paths::codex_auth_path()?;
    let config_path = app_paths::codex_config_path()?;
    let mut backup_state = state.clone();
    backup_state.codex_auth_existed = auth_path.exists();
    backup_state.codex_config_existed = config_path.exists();
    backup_optional_file(&auth_path, &backup_dir.join(PROFILE_AUTH_FILE_NAME))?;
    backup_optional_file(&config_path, &backup_dir.join(PROFILE_CONFIG_FILE_NAME))?;
    let serialized_state = serde_json::to_vec_pretty(&backup_state)
        .map_err(|error| format!("序列化路由模式备份状态失败: {error}"))?;
    write_file_atomically(
        &backup_dir.join(PROFILE_BACKUP_STATE_FILE_NAME),
        &serialized_state,
    )?;
    Ok(())
}

pub(crate) fn restore_active_codex_profile_backup(
    backup_dir: &Path,
) -> Result<ActiveCodexProfileBackupState, String> {
    let state_path = backup_dir.join(PROFILE_BACKUP_STATE_FILE_NAME);
    let has_manifest = state_path.is_file();
    let state = read_active_codex_profile_backup_state(backup_dir)?;
    restore_profile_backup_file(
        &backup_dir.join(PROFILE_AUTH_FILE_NAME),
        &app_paths::codex_auth_path()?,
        has_manifest.then_some(state.codex_auth_existed),
    )?;
    restore_profile_backup_file(
        &backup_dir.join(PROFILE_CONFIG_FILE_NAME),
        &app_paths::codex_config_path()?,
        has_manifest.then_some(state.codex_config_existed),
    )?;
    Ok(state)
}

pub(crate) fn read_active_codex_profile_backup_state(
    backup_dir: &Path,
) -> Result<ActiveCodexProfileBackupState, String> {
    let state_path = backup_dir.join(PROFILE_BACKUP_STATE_FILE_NAME);
    if !state_path.is_file() {
        return Ok(ActiveCodexProfileBackupState {
            active_account_id: None,
            active_hybrid_profile: None,
            codex_auth_existed: backup_dir.join(PROFILE_AUTH_FILE_NAME).is_file(),
            codex_config_existed: backup_dir.join(PROFILE_CONFIG_FILE_NAME).is_file(),
        });
    }
    let raw = fs::read_to_string(&state_path)
        .map_err(|error| format!("读取路由模式备份状态失败 {}: {error}", state_path.display()))?;
    serde_json::from_str(&raw).map_err(|error| {
        format!(
            "路由模式备份状态不是合法 JSON {}: {error}",
            state_path.display()
        )
    })
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
    let entries = relay_account.enabled_model_catalog();
    apply_hybrid_account_profile_with_provider_base_url_and_catalog_entries(
        chatgpt_account,
        relay_account,
        provider_base_url,
        &entries,
    )
}

pub(crate) fn apply_hybrid_account_profile_with_provider_base_url_and_catalog_entries(
    chatgpt_account: &StoredAccount,
    relay_account: &StoredAccount,
    provider_base_url: &str,
    model_catalog_entries: &[RelayModelCatalogEntry],
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
    let provider_base_url = normalize_relay_base_url(provider_base_url)?;

    let hybrid_auth_json = build_hybrid_chatgpt_auth_json(&chatgpt_account.auth_json)?;

    let active_config_path = current_codex_config_path()?;
    let parent = active_config_path
        .parent()
        .ok_or_else(|| format!("无法解析 Codex 配置目录 {}", active_config_path.display()))?;
    let active_config_contents = read_optional_text(&active_config_path)?;
    let model_catalog_json_path = ensure_model_catalog_json_for_entries(model_catalog_entries)
        .or_else(|_| ensure_account_model_catalog_json(relay_account))
        .or_else(|_| ensure_hybrid_model_catalog_json())?;
    let profile_config = build_hybrid_relay_profile_config(
        active_config_contents.as_deref(),
        provider_base_url.as_str(),
        model_name,
        api_key,
        Some(&model_catalog_json_path),
    );
    let merged_config_contents =
        merge_active_codex_profile_config(active_config_contents.as_deref(), &profile_config);
    validate_hybrid_profile_config(&merged_config_contents, provider_base_url.as_str(), api_key)?;

    auth::write_active_codex_auth(&hybrid_auth_json)?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 Codex 配置目录失败 {}: {error}", parent.display()))?;
    write_file_atomically(&active_config_path, merged_config_contents.as_bytes())?;
    sync_codexdeck_model_assets()?;
    Ok(())
}

pub(crate) fn default_model_instructions_fix_path() -> Result<PathBuf, String> {
    Ok(app_paths::codex_dir()?.join(CODEX_MODEL_INSTRUCTIONS_FIX_FILE_NAME))
}

pub(crate) fn apply_model_instructions_fix_setting(enabled: bool) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    let config_path = current_codex_config_path()?;
    let managed_path = default_model_instructions_fix_path()?;
    ensure_model_instructions_fix_file(&managed_path)?;
    apply_model_instructions_fix_to_config_path(&config_path, &managed_path, true)
}

pub(crate) fn clear_model_instructions_fix_setting() -> Result<(), String> {
    let config_path = current_codex_config_path()?;
    let managed_path = default_model_instructions_fix_path()?;
    apply_model_instructions_fix_to_config_path(&config_path, &managed_path, false)
}

pub(crate) fn is_model_instructions_fix_applied() -> Result<bool, String> {
    let config_path = current_codex_config_path()?;
    let Some(raw) = read_optional_text(&config_path)? else {
        return Ok(false);
    };
    let Ok(document) = raw.parse::<DocumentMut>() else {
        return Ok(false);
    };
    let managed_path = default_model_instructions_fix_path()?;
    let managed_path_text = managed_path.to_string_lossy();
    Ok(document
        .get(CODEX_MODEL_INSTRUCTIONS_FILE_KEY)
        .and_then(|item| item.as_str())
        == Some(managed_path_text.as_ref()))
}

fn apply_model_instructions_fix_to_config_path(
    config_path: &Path,
    managed_path: &Path,
    enabled: bool,
) -> Result<(), String> {
    if !enabled && !config_path.exists() {
        return Ok(());
    }
    if enabled && !managed_path.is_file() {
        return Err(format!(
            "降智修复说明文件不存在: {}",
            managed_path.display()
        ));
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 Codex 配置目录失败 {}: {error}", parent.display()))?;
    }

    let raw = read_optional_text(config_path)?;
    let mut document = match raw {
        Some(raw) => raw.parse::<DocumentMut>().map_err(|error| {
            format!(
                "Codex config.toml 不是合法 TOML，无法写入降智修复 {}: {error}",
                config_path.display()
            )
        })?,
        None => DocumentMut::new(),
    };
    let changed = apply_model_instructions_fix_to_document(&mut document, managed_path, enabled);
    if changed || !config_path.exists() {
        write_file_atomically(config_path, document.to_string().as_bytes())?;
    }
    Ok(())
}

fn ensure_model_instructions_fix_file(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    write_file_atomically(path, CODEX_MODEL_INSTRUCTIONS_FIX_CONTENT.as_bytes())
}

fn apply_model_instructions_fix_to_document(
    document: &mut DocumentMut,
    managed_path: &Path,
    enabled: bool,
) -> bool {
    let managed_path_text = managed_path.to_string_lossy().to_string();
    if enabled {
        if document
            .get(CODEX_MODEL_INSTRUCTIONS_FILE_KEY)
            .and_then(|item| item.as_str())
            == Some(managed_path_text.as_str())
        {
            return false;
        }
        document[CODEX_MODEL_INSTRUCTIONS_FILE_KEY] = value(managed_path_text);
        return true;
    }

    if document
        .get(CODEX_MODEL_INSTRUCTIONS_FILE_KEY)
        .and_then(|item| item.as_str())
        == Some(managed_path_text.as_str())
    {
        document.remove(CODEX_MODEL_INSTRUCTIONS_FILE_KEY);
        return true;
    }
    false
}

fn build_hybrid_chatgpt_auth_json(auth_json: &Value) -> Result<Value, String> {
    let Some(object) = auth_json.as_object() else {
        return Err("混合模式 ChatGPT auth.json 必须是 JSON 对象。".to_string());
    };

    let mut cleaned = object.clone();
    cleaned.remove(OPENAI_API_KEY_AUTH_KEY);
    Ok(Value::Object(cleaned))
}

fn ensure_hybrid_model_catalog_json() -> Result<PathBuf, String> {
    let path = app_paths::codex_dir()?.join(CODEXDECK_MODEL_CATALOG_FILE_NAME);
    match read_cached_bundled_codex_model_catalog_json() {
        Ok(catalog) => {
            write_file_if_changed(&path, catalog.as_bytes())?;
            Ok(path)
        }
        Err(error) if path.is_file() => {
            log::warn!(
                "生成 Codex bundled model catalog 失败，沿用已有文件 {}: {}",
                path.display(),
                redact_sensitive_text(&error)
            );
            Ok(path)
        }
        Err(error) => Err(format!(
            "生成混合模式模型 catalog 失败: {}",
            redact_sensitive_text(&error)
        )),
    }
}

fn ensure_account_model_catalog_json(account: &StoredAccount) -> Result<PathBuf, String> {
    let entries = account.enabled_model_catalog();
    ensure_model_catalog_json_for_entries(&entries)
}

fn ensure_model_catalog_json_for_entries(
    entries: &[RelayModelCatalogEntry],
) -> Result<PathBuf, String> {
    if entries.is_empty() {
        return Err("API 账号没有启用的模型。".to_string());
    }

    if let Some(path) = existing_model_catalog_path_if_entries_match(entries)? {
        return Ok(path);
    }

    let bundled = read_cached_bundled_codex_model_catalog_json()?;
    let catalog = build_model_catalog_json_from_entries(&bundled, &entries)?;
    let path = app_paths::codex_dir()?.join(CODEXDECK_MODEL_CATALOG_FILE_NAME);
    write_file_if_changed(&path, catalog.as_bytes())?;
    Ok(path)
}

pub(crate) fn sync_models_cache_from_model_catalog_json(catalog_json: &str) -> Result<(), String> {
    let catalog: Value = serde_json::from_str(catalog_json)
        .map_err(|error| format!("Codex 模型 catalog 不是合法 JSON: {error}"))?;
    let models = catalog
        .get("models")
        .and_then(Value::as_array)
        .filter(|models| !models.is_empty())
        .ok_or_else(|| "Codex 模型 catalog 缺少 models 数组。".to_string())?
        .clone();
    let cache_path = app_paths::codex_dir()?.join(CODEX_MODELS_CACHE_FILE_NAME);
    let existing_cache = fs::read_to_string(&cache_path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok());
    let client_version = existing_cache
        .as_ref()
        .and_then(|cache| cache.get("client_version"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(CODEX_MODELS_CACHE_CLIENT_VERSION_FALLBACK);
    if existing_cache
        .as_ref()
        .and_then(|cache| cache.get("models"))
        .is_some_and(|existing_models| existing_models == &Value::Array(models.clone()))
    {
        return Ok(());
    }

    let fetched_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("生成 Codex models cache 时间失败: {error}"))?;
    let payload = serde_json::json!({
        "fetched_at": fetched_at,
        "etag": format!("codexdeck-{}", Uuid::new_v4()),
        "client_version": client_version,
        "models": models,
    });
    let serialized = serde_json::to_vec_pretty(&payload)
        .map_err(|error| format!("序列化 Codex models cache 失败: {error}"))?;
    write_file_if_changed(&cache_path, &serialized)
}

pub(crate) fn sync_codexdeck_model_assets() -> Result<(), String> {
    let catalog_path = app_paths::codex_dir()?.join(CODEXDECK_MODEL_CATALOG_FILE_NAME);
    if !catalog_path.exists() {
        return Ok(());
    }

    let catalog = fs::read_to_string(&catalog_path).map_err(|error| {
        format!(
            "读取 CodexDeck 模型 catalog 失败 {}: {error}",
            catalog_path.display()
        )
    })?;
    sync_models_cache_from_model_catalog_json(&catalog)?;
    sync_agents_from_model_catalog_json(&catalog)
}

fn sync_agents_from_model_catalog_json(catalog_json: &str) -> Result<(), String> {
    let catalog: Value = serde_json::from_str(catalog_json)
        .map_err(|error| format!("Codex 模型 catalog 不是合法 JSON: {error}"))?;
    let Some(models) = catalog.get("models").and_then(Value::as_array) else {
        return Ok(());
    };

    let agents_dir = app_paths::codex_dir()?.join("agents");
    fs::create_dir_all(&agents_dir).map_err(|error| {
        format!(
            "创建 Codex agents 目录失败 {}: {error}",
            agents_dir.display()
        )
    })?;
    let mut expected_file_names = HashSet::new();

    for model in models {
        let Some(slug) = model.get("slug").and_then(Value::as_str) else {
            continue;
        };
        let slug = slug.trim();
        if slug.is_empty() {
            continue;
        }
        expected_file_names.insert(format!("codexdeck-{}.toml", safe_file_stem(slug)));
        let display_name = model
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(slug);
        write_agent_toml(&agents_dir, slug, display_name)?;
    }
    cleanup_stale_codexdeck_agents(&agents_dir, &expected_file_names)?;
    Ok(())
}

fn existing_model_catalog_path_if_entries_match(
    entries: &[RelayModelCatalogEntry],
) -> Result<Option<PathBuf>, String> {
    let path = app_paths::codex_dir()?.join(CODEXDECK_MODEL_CATALOG_FILE_NAME);
    if !path.is_file() {
        return Ok(None);
    }
    let Some(catalog) = fs::read_to_string(&path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
    else {
        return Ok(None);
    };
    let Some(models) = catalog.get("models").and_then(Value::as_array) else {
        return Ok(None);
    };
    let enabled_entries = entries
        .iter()
        .filter(|entry| entry.enabled && !entry.model.trim().is_empty())
        .collect::<Vec<_>>();
    if models.len() != enabled_entries.len() {
        return Ok(None);
    }
    for (model, entry) in models.iter().zip(enabled_entries) {
        let Some(slug) = model.get("slug").and_then(Value::as_str) else {
            return Ok(None);
        };
        if slug != entry.model.trim() {
            return Ok(None);
        }
        let display_name = model
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(slug);
        if display_name != entry.display_name_or_model() {
            return Ok(None);
        }
        if let Some(context_window) = entry.context_window {
            if model.get("context_window").and_then(Value::as_u64) != Some(context_window as u64) {
                return Ok(None);
            }
        }
    }
    Ok(Some(path))
}

fn cleanup_stale_codexdeck_agents(
    agents_dir: &Path,
    expected_file_names: &HashSet<String>,
) -> Result<(), String> {
    if !agents_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(agents_dir).map_err(|error| {
        format!(
            "读取 Codex agents 目录失败 {}: {error}",
            agents_dir.display()
        )
    })? {
        let entry = entry.map_err(|error| format!("读取 Codex agent 文件失败: {error}"))?;
        let path = entry.path();
        let is_expected = path
            .file_name()
            .and_then(|item| item.to_str())
            .is_some_and(|name| expected_file_names.contains(name));
        if path.is_file() && !is_expected && is_codexdeck_managed_agent(&path) {
            fs::remove_file(&path).map_err(|error| {
                format!("移除旧 CodexDeck agent 失败 {}: {error}", path.display())
            })?;
        }
    }
    Ok(())
}

fn is_codexdeck_managed_agent(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|item| item.to_str())
        .is_some_and(|name| name.starts_with(MANAGED_AGENT_PREFIX))
    {
        return true;
    }

    fs::read_to_string(path)
        .ok()
        .is_some_and(|contents| contents.contains(MANAGED_AGENT_MARKER))
}

fn write_agent_toml(agents_dir: &Path, model: &str, display_name: &str) -> Result<(), String> {
    let file_name = format!("codexdeck-{}.toml", safe_file_stem(model));
    let path = agents_dir.join(file_name);
    let agent_name = format!("codexdeck_{}", safe_agent_name(model));
    let agent_name = toml_string(&agent_name);
    let description = toml_string(&format!("CodexDeck managed agent for {display_name}."));
    let model = toml_string(model);
    let contents = format!(
        r#"{MANAGED_AGENT_MARKER}
name = {agent_name}
description = {description}
model = {model}
sandbox_mode = "read-only"
developer_instructions = """
You are a CodexDeck managed subagent. Follow the user's task and keep the response concise.
"""
"#
    );
    write_file_if_changed(&path, contents.as_bytes())
        .map_err(|error| format!("写入 CodexDeck agent 失败 {}: {error}", path.display()))
}

fn safe_file_stem(value: &str) -> String {
    let safe = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if safe.is_empty() {
        "model".to_string()
    } else {
        safe
    }
}

fn safe_agent_name(value: &str) -> String {
    let safe = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if safe.is_empty() {
        "model".to_string()
    } else {
        safe
    }
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn build_model_catalog_json_from_entries(
    bundled_catalog_json: &str,
    entries: &[RelayModelCatalogEntry],
) -> Result<String, String> {
    let mut catalog: Value = serde_json::from_str(bundled_catalog_json)
        .map_err(|error| format!("Codex bundled model catalog 不是合法 JSON: {error}"))?;
    let bundled_models = catalog
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "Codex bundled model catalog 缺少 models 数组。".to_string())?;
    let template = select_catalog_template_model(bundled_models, entries)
        .cloned()
        .ok_or_else(|| "Codex bundled model catalog 缺少可复用模型模板。".to_string())?;

    let mut models = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let mut model = template.clone();
        if let Some(object) = model.as_object_mut() {
            object.insert("slug".to_string(), Value::String(entry.model.clone()));
            object.insert(
                "display_name".to_string(),
                Value::String(entry.display_name_or_model().to_string()),
            );
            object.insert(
                "description".to_string(),
                Value::String(format!(
                    "CodexDeck API model {}",
                    entry.request_model_or_model()
                )),
            );
            object.insert(
                "priority".to_string(),
                Value::Number(serde_json::Number::from(index as u64)),
            );
            object.insert("visibility".to_string(), Value::String("list".to_string()));
            object.insert("supported_in_api".to_string(), Value::Bool(true));
            if let Some(context_window) = entry.context_window {
                object.insert(
                    "context_window".to_string(),
                    Value::Number(serde_json::Number::from(context_window)),
                );
                object.insert(
                    "max_context_window".to_string(),
                    Value::Number(serde_json::Number::from(context_window)),
                );
                if let Some(truncation_policy) = object
                    .get_mut("truncation_policy")
                    .and_then(Value::as_object_mut)
                {
                    truncation_policy.insert(
                        "limit".to_string(),
                        Value::Number(serde_json::Number::from(context_window)),
                    );
                }
            }
        }
        models.push(model);
    }

    let Some(object) = catalog.as_object_mut() else {
        return Err("Codex bundled model catalog 顶层不是 JSON 对象。".to_string());
    };
    object.insert("models".to_string(), Value::Array(models));
    serde_json::to_string_pretty(&catalog)
        .map_err(|error| format!("序列化 Codex 模型 catalog 失败: {error}"))
}

fn select_catalog_template_model<'a>(
    bundled_models: &'a [Value],
    entries: &[RelayModelCatalogEntry],
) -> Option<&'a Value> {
    for entry in entries {
        if let Some(model) = bundled_models.iter().find(|model| {
            model
                .get("slug")
                .and_then(Value::as_str)
                .is_some_and(|slug| slug == entry.model)
        }) {
            return Some(model);
        }
    }

    bundled_models
        .iter()
        .find(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.5"))
        .or_else(|| bundled_models.first())
}

fn read_cached_bundled_codex_model_catalog_json() -> Result<String, String> {
    static BUNDLED_MODEL_CATALOG: OnceLock<Result<String, String>> = OnceLock::new();
    BUNDLED_MODEL_CATALOG
        .get_or_init(load_cached_bundled_codex_model_catalog_json)
        .clone()
}

fn load_cached_bundled_codex_model_catalog_json() -> Result<String, String> {
    let cache_path = app_paths::codex_dir()?.join(CODEXDECK_BUNDLED_MODEL_CATALOG_FILE_NAME);
    if let Ok(contents) = fs::read_to_string(&cache_path) {
        if validate_model_catalog_json(&contents).is_ok() {
            return Ok(contents);
        }
    }

    let catalog = read_bundled_codex_model_catalog_json()?;
    if let Err(error) = write_file_if_changed(&cache_path, catalog.as_bytes()) {
        log::warn!(
            "写入 Codex bundled model catalog 缓存失败 {}: {}",
            cache_path.display(),
            redact_sensitive_text(&error)
        );
    }
    Ok(catalog)
}

fn read_bundled_codex_model_catalog_json() -> Result<String, String> {
    let mut command = cli::new_codex_command(None)?;
    let output = command
        .args(["debug", "models", "--bundled"])
        .output()
        .map_err(|error| format!("运行 codex debug models --bundled 失败: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "codex debug models --bundled 返回 {}: {}",
            output.status,
            truncate_message(&stderr)
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("Codex bundled model catalog 不是 UTF-8: {error}"))?;
    validate_model_catalog_json(&stdout)?;
    Ok(stdout)
}

fn validate_model_catalog_json(contents: &str) -> Result<(), String> {
    let parsed: Value = serde_json::from_str(contents)
        .map_err(|error| format!("Codex bundled model catalog 不是合法 JSON: {error}"))?;
    let has_models = parsed
        .get("models")
        .and_then(Value::as_array)
        .is_some_and(|models| !models.is_empty());
    if !has_models {
        return Err("Codex bundled model catalog 缺少 models 数组。".to_string());
    }
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
    if trimmed.chars().any(char::is_whitespace) {
        return Err("API Key 不应包含空格或换行。".to_string());
    }
    Ok(trimmed.to_string())
}

pub(crate) fn normalize_relay_base_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("请输入 Base URL。".to_string());
    }
    let mut parsed =
        Url::parse(trimmed).map_err(|error| format!("Base URL 不是有效地址: {error}"))?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return Err("Base URL 仅支持 http/https 地址。".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("Base URL 不应包含查询参数或片段。".to_string());
    }

    let normalized_path = parsed.path().trim_end_matches('/').to_string();
    if normalized_path.is_empty() {
        parsed.set_path("/v1");
    } else {
        parsed.set_path(&normalized_path);
    }

    Ok(parsed.to_string())
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
    let had_codexdeck_provider = document
        .get(CODEX_MODEL_PROVIDER_KEY)
        .and_then(|item| item.as_str())
        == Some(CODEXDECK_RELAY_PROVIDER_ID)
        || document
            .get(CODEX_MODEL_PROVIDERS_KEY)
            .and_then(|item| item.as_table())
            .is_some_and(|providers| providers.contains_key(CODEXDECK_RELAY_PROVIDER_ID));
    normalize_standard_profile_config(&mut document);
    remove_responses_websocket_flags(&mut document);
    remove_legacy_hybrid_tool_feature_disables(&mut document);
    document.remove(CODEX_BASE_URL_KEY);
    document.remove(CODEX_MODEL_PROVIDERS_KEY);
    if had_base_url || had_codexdeck_provider {
        document.remove(CODEX_MODEL_KEY);
    }
    document.to_string()
}

fn build_relay_profile_config(
    current_config: Option<&str>,
    base_url: &str,
    model_name: &str,
    model_catalog_json_path: Option<&Path>,
) -> String {
    let base_url = normalize_relay_base_url_for_profile(base_url);
    let mut document = parse_config_or_default(current_config);
    normalize_standard_profile_config(&mut document);
    disable_responses_websockets(&mut document);
    remove_legacy_hybrid_tool_feature_disables(&mut document);
    document[CODEX_BASE_URL_KEY] = value(base_url.as_str());
    document[CODEX_MODEL_KEY] = value(model_name);
    if let Some(path) = model_catalog_json_path {
        document[CODEX_MODEL_CATALOG_JSON_KEY] = value(path.to_string_lossy().as_ref());
    } else {
        document.remove(CODEX_MODEL_CATALOG_JSON_KEY);
    }
    document[CODEX_MODEL_PROVIDER_KEY] = value(CODEXDECK_RELAY_PROVIDER_ID);
    document[CODEX_MODEL_PROVIDERS_KEY] = table();
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID] = table();
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID][CODEX_PROVIDER_NAME_KEY] =
        value(CODEXDECK_RELAY_PROVIDER_ID);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID][CODEX_PROVIDER_BASE_URL_KEY] =
        value(base_url.as_str());
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
    model_catalog_json_path: Option<&Path>,
) -> String {
    let base_url = normalize_relay_base_url_for_profile(base_url);
    let mut document = parse_config_or_default(current_config);
    normalize_standard_profile_config(&mut document);
    disable_responses_websockets(&mut document);
    remove_legacy_hybrid_tool_feature_disables(&mut document);
    document.remove(CODEX_BASE_URL_KEY);
    document[CODEX_MODEL_KEY] = value(model_name);
    if let Some(path) = model_catalog_json_path {
        document[CODEX_MODEL_CATALOG_JSON_KEY] = value(path.to_string_lossy().as_ref());
    } else {
        document.remove(CODEX_MODEL_CATALOG_JSON_KEY);
    }
    document[CODEX_MODEL_PROVIDER_KEY] = value(CODEXDECK_RELAY_PROVIDER_ID);
    document[CODEX_MODEL_PROVIDERS_KEY] = table();
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID] = table();
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID][CODEX_PROVIDER_NAME_KEY] =
        value(CODEXDECK_RELAY_PROVIDER_ID);
    document[CODEX_MODEL_PROVIDERS_KEY][CODEXDECK_RELAY_PROVIDER_ID][CODEX_PROVIDER_BASE_URL_KEY] =
        value(base_url.as_str());
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

fn normalize_relay_base_url_for_profile(base_url: &str) -> String {
    normalize_relay_base_url(base_url)
        .unwrap_or_else(|_| base_url.trim().trim_end_matches('/').to_string())
}

fn validate_hybrid_profile_config(
    config_contents: &str,
    provider_base_url: &str,
    api_key: &str,
) -> Result<(), String> {
    let document = config_contents
        .parse::<DocumentMut>()
        .map_err(|error| format!("混合模式 config.toml 生成失败: {error}"))?;

    if document.get(CODEX_BASE_URL_KEY).is_some() {
        return Err("混合模式配置不应写入顶层 openai_base_url。".to_string());
    }

    let provider_id = document
        .get(CODEX_MODEL_PROVIDER_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "混合模式配置缺少 model_provider。".to_string())?;
    if provider_id != CODEXDECK_RELAY_PROVIDER_ID {
        return Err(format!(
            "混合模式配置的 model_provider 必须是 {CODEXDECK_RELAY_PROVIDER_ID}。"
        ));
    }

    let provider = document
        .get(CODEX_MODEL_PROVIDERS_KEY)
        .and_then(|item| item.as_table())
        .and_then(|providers| providers.get(provider_id))
        .and_then(|item| item.as_table())
        .ok_or_else(|| "混合模式配置缺少当前 model_provider 表。".to_string())?;

    let actual_base_url = provider
        .get(CODEX_PROVIDER_BASE_URL_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .ok_or_else(|| "混合模式 provider 缺少 base_url。".to_string())?;
    if actual_base_url != provider_base_url {
        return Err("混合模式 provider base_url 与 API 条目不一致。".to_string());
    }

    let wire_api = provider
        .get(CODEX_PROVIDER_WIRE_API_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .ok_or_else(|| "混合模式 provider 缺少 wire_api。".to_string())?;
    if wire_api != CODEX_PROVIDER_WIRE_API_RESPONSES {
        return Err("混合模式 provider wire_api 必须是 responses。".to_string());
    }

    let requires_openai_auth = provider
        .get(CODEX_PROVIDER_REQUIRES_OPENAI_AUTH_KEY)
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    if !requires_openai_auth {
        return Err("混合模式 provider 必须启用 requires_openai_auth。".to_string());
    }

    let bearer_token = provider
        .get(CODEX_PROVIDER_EXPERIMENTAL_BEARER_TOKEN_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .ok_or_else(|| "混合模式 provider 缺少 experimental_bearer_token。".to_string())?;
    if bearer_token != api_key {
        return Err("混合模式 provider token 与 API 条目不一致。".to_string());
    }

    let supports_websockets = provider
        .get(CODEX_PROVIDER_SUPPORTS_WEBSOCKETS_KEY)
        .and_then(|item| item.as_bool())
        .unwrap_or(true);
    if supports_websockets {
        return Err("混合模式 provider 必须禁用 supports_websockets。".to_string());
    }

    let catalog_path = document
        .get(CODEX_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .ok_or_else(|| "混合模式配置缺少 model_catalog_json。".to_string())?;
    if catalog_path.is_empty() {
        return Err("混合模式 model_catalog_json 不能为空。".to_string());
    }

    Ok(())
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

fn remove_legacy_hybrid_tool_feature_disables(document: &mut DocumentMut) {
    let Some(features) = document
        .get_mut(CODEX_FEATURES_TABLE_KEY)
        .and_then(|item| item.as_table_mut())
    else {
        return;
    };

    for key in CODEX_LEGACY_HYBRID_TOOL_FEATURE_KEYS {
        if features.get(*key).and_then(|item| item.as_bool()) == Some(false) {
            features.remove(*key);
        }
    }
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
    for key in CODEX_COPIED_FEATURE_KEYS {
        copy_table_value_if_present(
            &mut active_document,
            &profile_document,
            CODEX_FEATURES_TABLE_KEY,
            key,
        );
        remove_table_value_if_missing(
            &mut active_document,
            &profile_document,
            CODEX_FEATURES_TABLE_KEY,
            key,
        );
    }
    remove_legacy_hybrid_tool_feature_disables(&mut active_document);

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

fn backup_optional_file(source: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建备份目录失败 {}: {error}", parent.display()))?;
    }
    if source.exists() {
        fs::copy(source, destination).map_err(|error| {
            format!(
                "备份 Codex 文件失败 {} -> {}: {error}",
                source.display(),
                destination.display()
            )
        })?;
        set_private_permissions(destination);
    } else if destination.exists() {
        fs::remove_file(destination)
            .map_err(|error| format!("清理旧备份文件失败 {}: {error}", destination.display()))?;
    }
    Ok(())
}

fn restore_optional_file(source: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 Codex 目录失败 {}: {error}", parent.display()))?;
    }
    if source.exists() {
        fs::copy(source, destination).map_err(|error| {
            format!(
                "恢复 Codex 文件失败 {} -> {}: {error}",
                source.display(),
                destination.display()
            )
        })?;
        set_private_permissions(destination);
    } else if destination.exists() {
        fs::remove_file(destination)
            .map_err(|error| format!("移除 Codex 文件失败 {}: {error}", destination.display()))?;
    }
    Ok(())
}

fn restore_profile_backup_file(
    source: &Path,
    destination: &Path,
    source_existed_when_backed_up: Option<bool>,
) -> Result<(), String> {
    match source_existed_when_backed_up {
        Some(true) => restore_optional_file(source, destination),
        Some(false) => {
            if destination.exists() {
                fs::remove_file(destination).map_err(|error| {
                    format!("移除 Codex 文件失败 {}: {error}", destination.display())
                })?;
            }
            Ok(())
        }
        None => {
            if source.exists() {
                restore_optional_file(source, destination)?;
            }
            Ok(())
        }
    }
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

fn write_file_if_changed(path: &Path, contents: &[u8]) -> Result<(), String> {
    if fs::read(path).is_ok_and(|existing| existing == contents) {
        return Ok(());
    }
    write_file_atomically(path, contents)
}

#[cfg(test)]
mod tests {
    use super::apply_model_instructions_fix_to_document;
    use super::build_chatgpt_profile_config;
    use super::build_hybrid_chatgpt_auth_json;
    use super::build_hybrid_relay_profile_config;
    use super::build_model_catalog_json_from_entries;
    use super::build_relay_profile_config;
    use super::cleanup_orphan_profiles_in_store_path;
    use super::merge_active_codex_profile_config;
    use super::normalize_relay_api_key;
    use super::normalize_relay_base_url;
    use super::profile_dir_from_store_path;
    use super::remove_account_profile_in_store_path;
    use super::truncate_message;
    use super::validate_hybrid_profile_config;
    use super::validate_relay_target;
    use crate::models::ProxyEndpointCapability;
    use crate::models::RelayModelCatalogEntry;
    use serde_json::json;
    use serde_json::Value;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn hybrid_catalog_test_path() -> PathBuf {
        PathBuf::from("fixtures/codexdeck-model-catalog.json")
    }

    fn assert_model_catalog_json_path(config: &str, path: &PathBuf) {
        let document = config
            .parse::<toml_edit::DocumentMut>()
            .expect("parse config");
        let expected = path.to_string_lossy();
        assert_eq!(
            document
                .get("model_catalog_json")
                .and_then(|item| item.as_str()),
            Some(expected.as_ref())
        );
    }

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
    fn hybrid_chatgpt_auth_removes_openai_api_key_and_preserves_tokens() {
        let auth = json!({
            "OPENAI_API_KEY": "placeholder-stale-api-key",
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "official-access",
                "refresh_token": "official-refresh",
                "id_token": "official-id"
            }
        });

        let cleaned = build_hybrid_chatgpt_auth_json(&auth).expect("clean hybrid auth");

        assert!(cleaned.get("OPENAI_API_KEY").is_none());
        assert_eq!(cleaned.get("auth_mode"), Some(&json!("chatgpt")));
        assert_eq!(cleaned["tokens"]["access_token"], json!("official-access"));
        assert_eq!(
            cleaned["tokens"]["refresh_token"],
            json!("official-refresh")
        );
    }

    #[test]
    fn normalize_relay_base_url_adds_v1_for_root_urls() {
        assert_eq!(
            normalize_relay_base_url(" http://127.0.0.1:8787/ ").unwrap(),
            "http://127.0.0.1:8787/v1"
        );
        assert_eq!(
            normalize_relay_base_url("https://relay.example.com").unwrap(),
            "https://relay.example.com/v1"
        );
    }

    #[test]
    fn normalize_relay_base_url_preserves_explicit_api_paths() {
        assert_eq!(
            normalize_relay_base_url("https://relay.example.com/v1/").unwrap(),
            "https://relay.example.com/v1"
        );
        assert_eq!(
            normalize_relay_base_url("https://relay.example.com/api/v1/").unwrap(),
            "https://relay.example.com/api/v1"
        );
    }

    #[test]
    fn normalize_relay_base_url_rejects_query_or_fragment() {
        assert!(normalize_relay_base_url("https://relay.example.com?token=abc").is_err());
        assert!(normalize_relay_base_url("https://relay.example.com/v1#models").is_err());
    }

    #[test]
    fn normalize_relay_api_key_accepts_provider_tokens() {
        assert_eq!(
            normalize_relay_api_key(" tp-provider-token ").unwrap(),
            "tp-provider-token"
        );
        assert_eq!(
            normalize_relay_api_key("custom-provider-token").unwrap(),
            "custom-provider-token"
        );
    }

    #[test]
    fn normalize_relay_api_key_rejects_empty_or_whitespace() {
        assert!(normalize_relay_api_key("   ").is_err());
        assert!(normalize_relay_api_key("tp-token with-space").is_err());
        assert!(normalize_relay_api_key("tp-token\nnext").is_err());
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
    fn chatgpt_profile_config_clears_hybrid_model_without_legacy_base_url() {
        let current = r#"model = "gpt-5.5"
model_provider = "codexdeck_api"

[model_providers.codexdeck_api]
name = "codexdeck_api"
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "sk-hybrid-secret"
supports_websockets = false
"#;

        let config = build_chatgpt_profile_config(Some(current));

        assert!(config.contains(r#"cli_auth_credentials_store = "file""#));
        assert!(!config.contains("model ="));
        assert!(!config.contains("model_provider"));
        assert!(!config.contains("model_providers"));
        assert!(!config.contains("experimental_bearer_token"));
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
            None,
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
    fn relay_profile_config_normalizes_root_base_url_to_v1() {
        let config = build_relay_profile_config(None, "http://127.0.0.1:8787", "relay-model", None);

        assert!(config.contains(r#"openai_base_url = "http://127.0.0.1:8787/v1""#));
        assert!(config.contains(r#"base_url = "http://127.0.0.1:8787/v1""#));
        assert!(!config.contains(r#"base_url = "http://127.0.0.1:8787""#));
    }

    #[test]
    fn relay_profile_config_can_write_model_catalog_json() {
        let catalog_path = hybrid_catalog_test_path();
        let config = build_relay_profile_config(
            None,
            "https://relay.example.com/v1",
            "relay-model",
            Some(&catalog_path),
        );

        assert_model_catalog_json_path(&config, &catalog_path);
    }

    #[test]
    fn model_catalog_json_is_generated_from_current_codex_schema() {
        let bundled = json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "description": "template",
                    "default_reasoning_level": "medium",
                    "supported_reasoning_levels": [
                        { "effort": "low", "description": "Low" }
                    ],
                    "shell_type": "shell_command",
                    "visibility": "list",
                    "supported_in_api": true,
                    "priority": 0,
                    "context_window": 272000,
                    "max_context_window": 272000,
                    "truncation_policy": { "mode": "tokens", "limit": 10000 }
                }
            ]
        })
        .to_string();
        let catalog = build_model_catalog_json_from_entries(
            &bundled,
            &[
                RelayModelCatalogEntry {
                    model: "menu-fast".to_string(),
                    display_name: Some("Fast Model".to_string()),
                    request_model: Some("upstream-fast".to_string()),
                    context_window: Some(128000),
                    enabled: true,
                },
                RelayModelCatalogEntry {
                    model: "menu-heavy".to_string(),
                    display_name: None,
                    request_model: None,
                    context_window: None,
                    enabled: true,
                },
            ],
        )
        .expect("build catalog");
        let parsed: Value = serde_json::from_str(&catalog).expect("parse generated catalog");
        let models = parsed["models"].as_array().expect("models array");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["slug"], json!("menu-fast"));
        assert_eq!(models[0]["display_name"], json!("Fast Model"));
        assert_eq!(models[0]["priority"], json!(0));
        assert_eq!(models[0]["context_window"], json!(128000));
        assert_eq!(models[0]["max_context_window"], json!(128000));
        assert_eq!(models[0]["truncation_policy"]["limit"], json!(128000));
        assert_eq!(models[1]["slug"], json!("menu-heavy"));
        assert_eq!(models[1]["display_name"], json!("menu-heavy"));
        assert_eq!(models[1]["default_reasoning_level"], json!("medium"));
    }

    #[test]
    fn hybrid_profile_config_uses_codexdeck_provider_and_bearer_token() {
        let catalog_path = hybrid_catalog_test_path();
        let config = build_hybrid_relay_profile_config(
            Some(
                r#"openai_base_url = "https://old.example.com/v1"

[features]
experimental_feature = true
responses_websockets = true
responses_websockets_v2 = true
"#,
            ),
            "https://relay.example.com/v1",
            "gpt-5.5",
            "test-hybrid-token",
            Some(&catalog_path),
        );

        assert!(config.contains(r#"cli_auth_credentials_store = "file""#));
        assert!(config.contains(r#"model = "gpt-5.5""#));
        assert_model_catalog_json_path(&config, &catalog_path);
        assert!(config.contains(r#"model_provider = "codexdeck_api""#));
        assert!(!config.contains("openai_base_url"));
        assert!(config.contains("[model_providers.codexdeck_api]"));
        assert!(config.contains(r#"name = "codexdeck_api""#));
        assert!(config.contains(r#"base_url = "https://relay.example.com/v1""#));
        assert!(!config.contains("127.0.0.1"));
        assert!(config.contains(r#"wire_api = "responses""#));
        assert!(config.contains("requires_openai_auth = true"));
        assert!(config.contains(r#"experimental_bearer_token = "test-hybrid-token""#));
        assert!(config.contains("supports_websockets = false"));
        assert!(config.contains("experimental_feature = true"));
        assert!(config.contains("responses_websockets = false"));
        assert!(config.contains("responses_websockets_v2 = false"));
        assert!(!config.contains("plugins ="));
        assert!(!config.contains("apps ="));
        assert!(!config.contains("image_generation ="));
    }

    #[test]
    fn hybrid_profile_config_normalizes_root_base_url_to_v1() {
        let catalog_path = hybrid_catalog_test_path();
        let config = build_hybrid_relay_profile_config(
            None,
            "http://127.0.0.1:8787",
            "gpt-5.5",
            "test-hybrid-token",
            Some(&catalog_path),
        );

        assert!(config.contains(r#"base_url = "http://127.0.0.1:8787/v1""#));
        assert!(!config.contains(r#"base_url = "http://127.0.0.1:8787""#));
    }

    #[test]
    fn hybrid_profile_validation_requires_token_on_active_provider() {
        let config = r#"model = "gpt-5.5"
model_catalog_json = "fixtures/codexdeck-model-catalog.json"
model_provider = "codexdeck_api"

[model_providers.other_provider]
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "placeholder-wrong-provider-token"
supports_websockets = false

[model_providers.codexdeck_api]
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = false
"#;

        let error = validate_hybrid_profile_config(
            config,
            "https://relay.example.com/v1",
            "sk-hybrid-secret",
        )
        .expect_err("missing token on active provider should fail");

        assert!(error.contains("experimental_bearer_token"));
    }

    #[test]
    fn hybrid_profile_merge_preserves_user_tables_and_removes_legacy_base_url() {
        let catalog_path = hybrid_catalog_test_path();
        let active = r#"model = "old-model"
openai_base_url = "https://old.example.com/v1"
custom_setting = "keep"

[mcp_servers.filesystem]
command = "node"
args = ["server.js"]

[projects."fixtures/workspace/project"]
trust_level = "trusted"

[features]
experimental_feature = true
responses_websockets = true
plugins = false
apps = false
image_generation = false
"#;
        let profile = build_hybrid_relay_profile_config(
            Some(active),
            "https://relay.example.com/v1",
            "gpt-5.5",
            "test-hybrid-token",
            Some(&catalog_path),
        );

        let merged = merge_active_codex_profile_config(Some(active), &profile);

        assert!(merged.contains(r#"model = "gpt-5.5""#));
        assert_model_catalog_json_path(&merged, &catalog_path);
        assert!(merged.contains(r#"model_provider = "codexdeck_api""#));
        assert!(!merged.contains("openai_base_url"));
        assert!(merged.contains("[mcp_servers.filesystem]"));
        assert!(merged.contains(r#"args = ["server.js"]"#));
        assert!(merged.contains(r#"[projects."fixtures/workspace/project"]"#));
        assert!(merged.contains(r#"trust_level = "trusted""#));
        assert!(merged.contains(r#"custom_setting = "keep""#));
        assert!(merged.contains("requires_openai_auth = true"));
        assert!(merged.contains(r#"experimental_bearer_token = "test-hybrid-token""#));
        assert!(merged.contains("responses_websockets = false"));
        assert!(merged.contains("responses_websockets_v2 = false"));
        assert!(!merged.contains("plugins ="));
        assert!(!merged.contains("apps ="));
        assert!(!merged.contains("image_generation ="));
    }

    #[test]
    fn profile_config_smart_merge_updates_only_switch_owned_keys() {
        let active = r#"model = "active-model"
openai_base_url = "https://old.example.com/v1"
model_catalog_json = "fixtures/old-models.json"
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
plugins = true
apps = true
image_generation = true
"#;
        let profile = r#"cli_auth_credentials_store = "file"
openai_base_url = "https://relay.example.com/v1"
model = "relay-model"
model_catalog_json = "fixtures/codexdeck-model-catalog.json"
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
plugins = false
apps = false
image_generation = false
"#;

        let merged = merge_active_codex_profile_config(Some(active), profile);

        assert!(merged.contains(r#"cli_auth_credentials_store = "file""#));
        assert!(merged.contains(r#"openai_base_url = "https://relay.example.com/v1""#));
        assert!(merged.contains(r#"model = "relay-model""#));
        assert!(merged.contains("model_catalog_json"));
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
        assert!(merged.contains("plugins = true"));
        assert!(merged.contains("apps = true"));
        assert!(merged.contains("image_generation = true"));
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
model_catalog_json = "fixtures/codexdeck-model-catalog.json"
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
plugins = false
apps = false
image_generation = false
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
        assert!(!merged.contains("model_catalog_json"));
        assert!(!merged.contains("model_context_window"));
        assert!(!merged.contains("model_auto_compact_token_limit"));
        assert!(!merged.contains("tool_output_token_limit"));
        assert!(!merged.contains("model_provider"));
        assert!(!merged.contains("model_providers"));
        assert!(merged.contains("experimental_feature = true"));
        assert!(!merged.contains("responses_websockets"));
        assert!(!merged.contains("responses_websockets_v2"));
        assert!(!merged.contains("plugins"));
        assert!(!merged.contains("apps"));
        assert!(!merged.contains("image_generation"));
    }

    #[test]
    fn model_instructions_fix_writes_managed_path() {
        let path = PathBuf::from("fixtures")
            .join(".codex")
            .join("gpt-5.5-base-instructions.md");
        let expected = path.to_string_lossy().to_string();
        let mut document = r#"model = "gpt-5.5"
"#
        .parse::<toml_edit::DocumentMut>()
        .expect("parse config");

        assert!(apply_model_instructions_fix_to_document(
            &mut document,
            &path,
            true
        ));
        assert_eq!(
            document
                .get("model_instructions_file")
                .and_then(|item| item.as_str()),
            Some(expected.as_str())
        );
        assert!(!apply_model_instructions_fix_to_document(
            &mut document,
            &path,
            true
        ));
    }

    #[test]
    fn model_instructions_fix_disable_removes_only_managed_path() {
        let managed_path = PathBuf::from("fixtures")
            .join(".codex")
            .join("gpt-5.5-base-instructions.md");
        let managed = managed_path.to_string_lossy();
        let mut managed_document = format!(
            r#"model = "gpt-5.5"
model_instructions_file = '{}'
"#,
            managed
        )
        .parse::<toml_edit::DocumentMut>()
        .expect("parse managed config");

        assert!(apply_model_instructions_fix_to_document(
            &mut managed_document,
            &managed_path,
            false
        ));
        assert!(managed_document.get("model_instructions_file").is_none());

        let mut custom_document = r#"model = "gpt-5.5"
model_instructions_file = 'custom-model-instructions.md'
"#
        .parse::<toml_edit::DocumentMut>()
        .expect("parse custom config");
        assert!(!apply_model_instructions_fix_to_document(
            &mut custom_document,
            &managed_path,
            false
        ));
        assert_eq!(
            custom_document
                .get("model_instructions_file")
                .and_then(|item| item.as_str()),
            Some("custom-model-instructions.md")
        );
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

        let result = validate_relay_target(
            &format!("http://{addr}/v1"),
            "test-api-key-probe",
            "upstream-model",
        )
        .await
        .expect("validate responses-only relay");

        assert_eq!(result.endpoints, vec![ProxyEndpointCapability::Responses]);
    }
}
