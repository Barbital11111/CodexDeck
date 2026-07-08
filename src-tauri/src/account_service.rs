use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use futures_util::stream;
use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::Url;
use rfd::FileDialog;
use serde::Deserialize;
use serde::Serialize;
use tauri::AppHandle;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use zip::write::FileOptions;
use zip::CompressionMethod;

use crate::auth::account_group_key;
use crate::auth::account_variant_key;
use crate::auth::auth_refresh_next_at;
use crate::auth::auth_tokens_expire_within;
use crate::auth::current_auth_account_key;
use crate::auth::current_auth_variant_key;
use crate::auth::extract_auth;
use crate::auth::normalize_imported_auth_json;
use crate::auth::normalize_plan_type_key;
use crate::auth::read_current_codex_auth;
use crate::auth::read_current_codex_auth_optional;
use crate::auth::refresh_chatgpt_auth_tokens;
use crate::model_router;
use crate::models::dedupe_account_variants;
use crate::models::infer_provider_metadata_from_base_url;
use crate::models::normalize_relay_model_catalog;
use crate::models::AccountSourceKind;
use crate::models::AccountSummary;
use crate::models::AccountsStore;
use crate::models::AuthJsonImportInput;
use crate::models::CreateApiAccountInput;
use crate::models::ImportAccountFailure;
use crate::models::ImportAccountsResult;
use crate::models::ProxyEndpointCapability;
use crate::models::ProxyHealthStatus;
use crate::models::ProxyKey;
use crate::models::RelayModelCatalogEntry;
use crate::models::StoredAccount;
use crate::models::UpdateApiAccountKeyInput;
use crate::models::UsageSnapshot;
use crate::notification_service;
use crate::profile_files;
use crate::state::AppState;
use crate::store::account_store_path_for_app;
use crate::store::load_store;
use crate::store::load_store_read_only;
use crate::store::save_store;
use crate::store::update_account_group_refresh_state_in_path;
use crate::usage::fetch_usage_snapshot;
use crate::utils::now_unix_seconds;
use crate::utils::redact_sensitive_text;
use crate::utils::set_private_permissions;
use crate::utils::short_account;

const DEACTIVATED_WORKSPACE_NOTICE: &str = "该账号已被踢出 team 组织，请重新授权后再刷新。";
const DEACTIVATED_ACCOUNT_NOTICE: &str = "账号被封禁，请检查邮箱";
const AUTH_EXPIRED_NOTICE: &str = "授权过期，请重新登录授权。";
const EXPORT_ARCHIVE_ENTRY_NAME: &str = "accounts.json";
const SUB2API_EXPORT_TYPE: &str = "sub2api-data";
const SUB2API_EXPORT_VERSION: u8 = 1;
const OPENAI_CODEX_CLI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const KEEPALIVE_REFRESH_WINDOW_SECS: i64 = 24 * 60 * 60;
const KEEPALIVE_REFRESH_INTERVAL_SECS: i64 = 7 * 24 * 60 * 60;
const RELAY_KEY_PROBE_TIMEOUT_SECS: u64 = 12;
const RELAY_MODEL_PROBE_TIMEOUT_SECS: u64 = 12;
const USAGE_REFRESH_CONCURRENCY: usize = 4;
const API_QUOTA_REFRESH_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AccountsExportFormat {
    CodexDeck,
    Sub2api,
}

impl Default for AccountsExportFormat {
    fn default() -> Self {
        Self::CodexDeck
    }
}

#[derive(Debug, Clone)]
struct ResolvedApiQuotaFields {
    mode: crate::models::ApiQuotaMode,
    today_used_text: Option<String>,
    remaining_text: Option<String>,
    total_remaining_text: Option<String>,
    total_tokens_text: Option<String>,
    today_tokens_text: Option<String>,
    daily_window: Option<crate::models::UsageWindow>,
    total_window: Option<crate::models::UsageWindow>,
    subscription_expires_at: Option<i64>,
    subscription_name: Option<String>,
}

impl ResolvedApiQuotaFields {
    fn empty() -> Self {
        Self {
            mode: Default::default(),
            today_used_text: None,
            remaining_text: None,
            total_remaining_text: None,
            total_tokens_text: None,
            today_tokens_text: None,
            daily_window: None,
            total_window: None,
            subscription_expires_at: None,
            subscription_name: None,
        }
    }
}

#[derive(Debug, Clone)]
enum ImportCandidate {
    Chatgpt(ChatgptImportCandidate),
    Relay(RelayImportCandidate),
}

#[derive(Debug, Clone)]
struct ChatgptImportCandidate {
    source: String,
    auth_json: serde_json::Value,
    label: Option<String>,
    usage: Option<UsageSnapshot>,
    plan_type: Option<String>,
    email: Option<String>,
}

#[derive(Debug, Clone)]
struct RelayImportCandidate {
    source: String,
    label: String,
    base_url: String,
    api_key: String,
    model_name: Option<String>,
}

#[derive(Debug, Clone)]
enum PreparedImport {
    Chatgpt(PreparedChatgptImport),
    Relay(PreparedRelayImport),
}

#[derive(Debug, Clone)]
struct PreparedRelayImport {
    label: String,
    base_url: String,
    api_key: String,
    model_name: String,
}

#[derive(Debug, Clone)]
struct PreparedChatgptImport {
    principal_id: String,
    auth_json: serde_json::Value,
    account_id: String,
    email: Option<String>,
    plan_type: Option<String>,
    usage: Option<UsageSnapshot>,
    label: Option<String>,
}

impl ImportCandidate {
    fn source(&self) -> &str {
        match self {
            Self::Chatgpt(candidate) => candidate.source.as_str(),
            Self::Relay(candidate) => candidate.source.as_str(),
        }
    }
}

struct RelayKeyProbeFailure {
    status: Option<StatusCode>,
    message: String,
}

#[derive(Debug, Serialize)]
struct Sub2apiDataPayload {
    #[serde(rename = "type")]
    data_type: &'static str,
    version: u8,
    exported_at: String,
    proxies: Vec<Sub2apiDataProxy>,
    accounts: Vec<Sub2apiDataAccount>,
}

#[derive(Debug, Serialize)]
struct Sub2apiDataProxy {
    proxy_key: String,
    name: String,
    protocol: String,
    host: String,
    port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    status: String,
}

#[derive(Debug, Serialize)]
struct Sub2apiDataAccount {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
    platform: String,
    #[serde(rename = "type")]
    account_type: String,
    credentials: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy_key: Option<String>,
    concurrency: u16,
    priority: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_multiplier: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_pause_on_expired: Option<bool>,
}

fn build_account_summaries_for_store(
    store: &AccountsStore,
    current_account_key: Option<&str>,
    current_variant_key: Option<&str>,
) -> Vec<AccountSummary> {
    let mut summaries = store
        .accounts
        .iter()
        .map(|account| account.to_summary(current_account_key, current_variant_key))
        .collect::<Vec<_>>();

    if let Some(hybrid) = store.settings.active_hybrid_profile.as_ref() {
        let has_valid_hybrid_pair = store.accounts.iter().any(|account| {
            account.id == hybrid.chatgpt_account_id
                && matches!(account.source_kind, AccountSourceKind::Chatgpt)
        }) && store.accounts.iter().any(|account| {
            account.id == hybrid.relay_account_id
                && matches!(account.source_kind, AccountSourceKind::Relay)
        });
        if has_valid_hybrid_pair {
            for summary in &mut summaries {
                summary.is_current = summary.id == hybrid.relay_account_id;
            }
            return summaries;
        }
    }

    if !summaries.iter().any(|account| account.is_current) {
        if let Some(active_id) = store.settings.active_account_id.as_deref() {
            if let Some(account) = summaries.iter_mut().find(|account| account.id == active_id) {
                account.is_current = true;
            }
        }
    }

    summaries
}

fn current_hybrid_pair_for_relay(
    store: &AccountsStore,
    relay_account_id: &str,
) -> Option<(StoredAccount, StoredAccount)> {
    let hybrid = store.settings.active_hybrid_profile.as_ref()?;
    if hybrid.relay_account_id != relay_account_id {
        return None;
    }
    let chatgpt_account = store
        .accounts
        .iter()
        .find(|account| {
            account.id == hybrid.chatgpt_account_id
                && matches!(account.source_kind, AccountSourceKind::Chatgpt)
        })?
        .clone();
    let relay_account = store
        .accounts
        .iter()
        .find(|account| {
            account.id == hybrid.relay_account_id
                && matches!(account.source_kind, AccountSourceKind::Relay)
        })?
        .clone();
    Some((chatgpt_account, relay_account))
}

async fn apply_current_relay_profile(
    store: &AccountsStore,
    relay_account: &StoredAccount,
) -> Result<(), String> {
    if let Some((chatgpt_account, relay_account)) =
        current_hybrid_pair_for_relay(store, &relay_account.id)
    {
        profile_files::apply_hybrid_account_profile(&chatgpt_account, &relay_account)?;
    } else {
        profile_files::apply_account_profile(relay_account)?;
    }
    profile_files::apply_model_instructions_fix_setting(
        store.settings.codex_model_instructions_fix_enabled,
    )
}

pub(crate) async fn list_accounts_internal(
    app: &AppHandle,
    state: &AppState,
) -> Result<Vec<AccountSummary>, String> {
    let _guard = state.store_lock.lock().await;
    let store = load_store_read_only(app)?;
    let current_account_key = current_auth_account_key();
    let current_variant_key = current_auth_variant_key();
    Ok(build_account_summaries_for_store(
        &store,
        current_account_key.as_deref(),
        current_variant_key.as_deref(),
    ))
}

pub(crate) async fn import_current_auth_account_internal(
    app: &AppHandle,
    state: &AppState,
    label: Option<String>,
) -> Result<AccountSummary, String> {
    let auth_json = read_current_codex_auth()?;
    let prepared = prepare_auth_json_import(auth_json, label).await?;
    commit_prepared_import(app, state, prepared).await
}

pub(crate) async fn create_api_account_internal(
    app: &AppHandle,
    state: &AppState,
    input: CreateApiAccountInput,
) -> Result<AccountSummary, String> {
    let label = profile_files::normalize_relay_label(&input.label)?;
    let base_url = profile_files::normalize_relay_base_url(&input.base_url)?;
    let api_key = profile_files::normalize_relay_api_key(&input.api_key)?;
    let model_name = profile_files::normalize_relay_model_name(&input.model_name)?;
    let model_catalog =
        normalize_relay_model_catalog(Some(model_name.as_str()), &input.model_catalog);
    let (provider_id, provider_name) =
        infer_provider_metadata_from_base_url(Some(base_url.as_str()));
    let quota_fields = resolve_create_api_quota_fields(&input, &label, &base_url, &api_key).await;
    let tags = normalize_account_tags(input.tags);

    let (last_validated_at, balance_text, profile_last_validation_error, proxy_endpoints) = if input
        .force_save
    {
        (
                None,
                None,
                Some("已跳过接口探测，仅启用 /v1/chat/completions；如需 Responses/Compact，请在 Key 池中手动开启。".to_string()),
                vec![ProxyEndpointCapability::ChatCompletions],
            )
    } else {
        let validation =
            profile_files::validate_relay_target(&base_url, &api_key, &model_name).await?;
        (
            Some(now_unix_seconds()),
            if input.balance_display_enabled {
                validation.balance_text
            } else {
                None
            },
            None,
            validation.endpoints,
        )
    };

    let current_account_key = current_auth_account_key();
    let current_variant_key = current_auth_variant_key();
    let summary = {
        let mut _guard = state.store_lock.lock().await;
        let mut store = load_store(app)?;
        let now = now_unix_seconds();
        let id = uuid::Uuid::new_v4().to_string();
        let account_id = profile_files::relay_account_id(&id);
        let mut stored = StoredAccount {
            id: id.clone(),
            label: label.clone(),
            source_kind: AccountSourceKind::Relay,
            principal_id: None,
            email: None,
            account_id,
            plan_type: None,
            auth_json: profile_files::build_api_auth_json(&api_key),
            api_base_url: Some(base_url),
            api_key: Some(api_key.clone()),
            api_keys: vec![relay_proxy_key(&id, &label, &api_key, now)],
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints,
            model_name: Some(model_name),
            model_catalog,
            model_routing_enabled: false,
            balance_text,
            balance_display_enabled: input.balance_display_enabled,
            api_quota_mode: quota_fields.mode,
            api_quota_today_used_text: quota_fields.today_used_text,
            api_quota_remaining_text: quota_fields.remaining_text,
            api_quota_total_remaining_text: quota_fields.total_remaining_text,
            api_quota_total_tokens_text: quota_fields.total_tokens_text,
            api_quota_today_tokens_text: quota_fields.today_tokens_text,
            api_quota_daily_window: quota_fields.daily_window,
            api_quota_total_window: quota_fields.total_window,
            api_quota_subscription_expires_at: quota_fields.subscription_expires_at,
            api_quota_subscription_name: quota_fields.subscription_name,
            provider_id,
            provider_name,
            tags,
            profile_auth_path: None,
            profile_config_path: None,
            profile_auth_ready: false,
            profile_config_ready: false,
            profile_integrity_error: None,
            profile_last_validated_at: last_validated_at,
            profile_last_validation_error,
            added_at: now,
            updated_at: now,
            usage: None,
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        };
        profile_files::sync_account_profile_in_store_path(
            &account_store_path_for_app(app)?,
            &mut stored,
        )?;

        let summary = stored.to_summary(
            current_account_key.as_deref(),
            current_variant_key.as_deref(),
        );
        store.accounts.push(stored);
        save_store(app, &store)?;
        summary
    };

    Ok(summary)
}

pub(crate) async fn probe_api_models_internal(
    base_url: &str,
    api_key: &str,
) -> Result<Vec<RelayModelCatalogEntry>, String> {
    let base_url = profile_files::normalize_relay_base_url(base_url)?;
    let api_key = profile_files::normalize_relay_api_key(api_key)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(RELAY_MODEL_PROBE_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("创建模型探测客户端失败: {error}"))?;
    let mut last_error: Option<String> = None;
    for endpoint in build_model_probe_url_candidates(&base_url) {
        let response = client
            .get(&endpoint)
            .bearer_auth(&api_key)
            .send()
            .await
            .map_err(|error| {
                format!(
                    "探测模型失败 [已隐藏接口地址]: {}",
                    redact_sensitive_text(&error.to_string())
                )
            })?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("读取模型探测响应失败: {error}"))?;
        if !status.is_success() {
            let message = format!(
                "模型探测返回 {status}: {}",
                truncate_probe_message(&redact_sensitive_text(&body))
            );
            if matches!(
                status,
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ) {
                last_error = Some(message);
                continue;
            }
            return Err(message);
        }
        let payload: serde_json::Value = serde_json::from_str(&body)
            .map_err(|error| format!("模型探测响应不是合法 JSON: {error}"))?;
        let entries = parse_probe_model_catalog(&payload);
        if entries.is_empty() {
            last_error = Some("模型探测响应里没有可用模型。".to_string());
            continue;
        }
        return Ok(entries);
    }

    Err(last_error.unwrap_or_else(|| "模型探测没有可用端点。".to_string()))
}

pub(crate) async fn probe_api_account_models_internal(
    app: &AppHandle,
    state: &AppState,
    account_key: &str,
    base_url: Option<String>,
    api_key: Option<String>,
) -> Result<Vec<RelayModelCatalogEntry>, String> {
    let account_key = account_key.trim();
    if account_key.is_empty() {
        return Err("未找到要探测模型的 API 账号。".to_string());
    }

    let normalized_input_base_url = base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let normalized_input_api_key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (resolved_base_url, resolved_api_key) = {
        let _guard = state.store_lock.lock().await;
        let store = load_store(app)?;
        let account = store
            .accounts
            .iter()
            .find(|candidate| candidate.account_key() == account_key)
            .ok_or_else(|| "未找到要探测模型的 API 账号。".to_string())?;
        if !matches!(account.source_kind, AccountSourceKind::Relay) {
            return Err("只有 API 账号支持模型探测。".to_string());
        }

        let resolved_base_url = normalized_input_base_url
            .map(str::to_string)
            .or_else(|| {
                account
                    .api_base_url
                    .as_ref()
                    .map(|value| value.trim().to_string())
            })
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "请先填写 Base URL。".to_string())?;
        let resolved_api_key = normalized_input_api_key
            .map(str::to_string)
            .or_else(|| {
                account
                    .primary_relay_api_key()
                    .map(|value| value.trim().to_string())
            })
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "当前账号没有可用 API Key，请临时输入 API Key 后再探测。".to_string())?;
        (resolved_base_url, resolved_api_key)
    };

    probe_api_models_internal(&resolved_base_url, &resolved_api_key).await
}

const MODEL_PROBE_COMPAT_SUFFIXES: &[&str] = &[
    "/api/claudecode",
    "/api/anthropic",
    "/apps/anthropic",
    "/api/coding",
    "/claudecode",
    "/anthropic",
    "/step_plan",
    "/coding",
    "/claude",
];

fn build_model_probe_url_candidates(base_url: &str) -> Vec<String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let mut candidates = Vec::new();
    push_unique_model_probe_candidate(&mut candidates, format!("{trimmed}/models"));

    if let Some(stripped) = trimmed.strip_suffix("/v1") {
        push_unique_model_probe_candidate(&mut candidates, format!("{stripped}/models"));
    }

    if let Some(stripped) = strip_model_probe_compat_suffix(trimmed) {
        let root = stripped.trim_end_matches('/');
        if !root.is_empty() && root.contains("://") {
            push_unique_model_probe_candidate(&mut candidates, format!("{root}/v1/models"));
            push_unique_model_probe_candidate(&mut candidates, format!("{root}/models"));
        }
    }

    candidates
}

fn push_unique_model_probe_candidate(candidates: &mut Vec<String>, candidate: String) {
    if Url::parse(&candidate).is_ok() && !candidates.iter().any(|item| item == &candidate) {
        candidates.push(candidate);
    }
}

fn strip_model_probe_compat_suffix(base_url: &str) -> Option<&str> {
    MODEL_PROBE_COMPAT_SUFFIXES
        .iter()
        .find_map(|suffix| base_url.strip_suffix(suffix))
}

fn parse_probe_model_catalog(payload: &serde_json::Value) -> Vec<RelayModelCatalogEntry> {
    let candidates = payload
        .get("data")
        .or_else(|| payload.get("models"))
        .unwrap_or(payload);

    let Some(items) = candidates.as_array() else {
        return Vec::new();
    };

    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for item in items {
        let (model, display_name) = if let Some(value) = item.as_str() {
            (value.trim().to_string(), None)
        } else if let Some(object) = item.as_object() {
            let model = object
                .get("id")
                .or_else(|| object.get("model"))
                .or_else(|| object.get("name"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            let display_name = object
                .get("display_name")
                .or_else(|| object.get("displayName"))
                .or_else(|| object.get("name"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty() && *value != model)
                .map(ToString::to_string);
            (model, display_name)
        } else {
            continue;
        };

        if model.is_empty() || !seen.insert(model.clone()) {
            continue;
        }

        entries.push(RelayModelCatalogEntry {
            model,
            display_name,
            request_model: None,
            context_window: None,
            enabled: true,
        });
    }

    entries
}

pub(crate) async fn reauthorize_account_internal(
    app: &AppHandle,
    state: &AppState,
    id: &str,
    auth_json: serde_json::Value,
) -> Result<ImportAccountsResult, String> {
    let prepared = prepare_auth_json_import(auth_json, None).await?;

    let mut _guard = state.store_lock.lock().await;
    let mut store = load_store(app)?;
    let Some(existing) = store.accounts.iter_mut().find(|account| account.id == id) else {
        return Err("未找到要重新授权的账号".to_string());
    };

    validate_reauthorization_target(existing, &prepared)?;
    apply_reauthorized_account(existing, prepared);
    let store_path = account_store_path_for_app(app)?;
    profile_files::sync_account_profile_in_store_path(&store_path, existing)?;
    dedupe_account_variants(&mut store.accounts);
    save_store(app, &store)?;

    Ok(ImportAccountsResult {
        total_count: 1,
        imported_count: 0,
        updated_count: 1,
        failures: Vec::new(),
    })
}

pub(crate) async fn import_auth_json_accounts_internal(
    app: &AppHandle,
    state: &AppState,
    items: Vec<AuthJsonImportInput>,
) -> Result<ImportAccountsResult, String> {
    if items.is_empty() {
        return Err("请至少提供一个 JSON 文件或 JSON 文本".to_string());
    }

    let total_count = items.len();
    let mut prepared_imports = Vec::with_capacity(total_count);
    let mut failures = Vec::new();

    for item in items {
        let source = normalize_import_source(&item.source);
        let candidates =
            match expand_import_json_content(&item.content, &source, item.label.as_deref()) {
                Ok(value) => value,
                Err(error) => {
                    failures.push(ImportAccountFailure { source, error });
                    continue;
                }
            };

        for candidate in candidates {
            let candidate_source = candidate.source().to_string();
            match prepare_import_candidate(candidate).await {
                Ok(prepared) => prepared_imports.push(prepared),
                Err(error) => failures.push(ImportAccountFailure {
                    source: candidate_source,
                    error,
                }),
            }
        }
    }

    if prepared_imports.is_empty() {
        return Ok(ImportAccountsResult {
            total_count,
            imported_count: 0,
            updated_count: 0,
            failures,
        });
    }

    let current_account_key = current_auth_account_key();
    let current_variant_key = current_auth_variant_key();
    let (imported_count, updated_count) = {
        let mut _guard = state.store_lock.lock().await;
        let mut store = load_store(app)?;
        let mut imported_count = 0usize;
        let mut updated_count = 0usize;
        let mut touched_ids = HashSet::new();
        let store_path = account_store_path_for_app(app)?;

        for prepared in prepared_imports {
            let (summary, updated_existing) = match prepared {
                PreparedImport::Chatgpt(prepared) => upsert_prepared_import(
                    &mut store,
                    prepared,
                    current_account_key.as_deref(),
                    current_variant_key.as_deref(),
                ),
                PreparedImport::Relay(prepared) => upsert_prepared_relay_import(
                    &mut store,
                    prepared,
                    current_account_key.as_deref(),
                    current_variant_key.as_deref(),
                ),
            };
            touched_ids.insert(summary.id);
            if updated_existing {
                updated_count += 1;
            } else {
                imported_count += 1;
            }
        }

        for account in store
            .accounts
            .iter_mut()
            .filter(|account| touched_ids.contains(&account.id))
        {
            profile_files::sync_account_profile_in_store_path(&store_path, account)?;
        }

        save_store(app, &store)?;
        (imported_count, updated_count)
    };

    Ok(ImportAccountsResult {
        total_count,
        imported_count,
        updated_count,
        failures,
    })
}

pub(crate) async fn export_accounts_zip_internal(
    app: &AppHandle,
    state: &AppState,
    account_key: Option<String>,
    account_keys: Option<Vec<String>>,
    format: Option<AccountsExportFormat>,
) -> Result<Option<String>, String> {
    let format = format.unwrap_or_default();
    let mut selected_account_keys = account_keys
        .unwrap_or_default()
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>();
    if let Some(account_key) = account_key
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        selected_account_keys.insert(account_key.to_string());
    }
    let (default_file_name, export_payload) = {
        let _guard = state.store_lock.lock().await;
        let mut store = load_store(app)?;
        if !selected_account_keys.is_empty() {
            store
                .accounts
                .retain(|account| selected_account_keys.contains(&account.account_key()));
        }
        match format {
            AccountsExportFormat::CodexDeck => (
                format!("codexdeck-accounts-{}.zip", now_unix_seconds()),
                serde_json::to_vec_pretty(&store)
                    .map_err(|error| format!("序列化账号列表失败: {error}"))?,
            ),
            AccountsExportFormat::Sub2api => {
                let payload = build_sub2api_data_payload(&store)?;
                (
                    format!("sub2api-accounts-{}.json", now_unix_seconds()),
                    serde_json::to_vec_pretty(&payload)
                        .map_err(|error| format!("序列化 Sub2API 导出失败: {error}"))?,
                )
            }
        }
    };

    tauri::async_runtime::spawn_blocking(move || match format {
        AccountsExportFormat::CodexDeck => {
            export_accounts_zip_sync(&default_file_name, &export_payload)
        }
        AccountsExportFormat::Sub2api => {
            export_accounts_json_sync("导出 Sub2API 账号数据", &default_file_name, &export_payload)
        }
    })
    .await
    .map_err(|error| format!("导出账号列表失败: {error}"))?
}

pub(crate) async fn delete_account_internal(
    app: &AppHandle,
    state: &AppState,
    id: &str,
) -> Result<(), String> {
    let _guard = state.store_lock.lock().await;
    let store_path = account_store_path_for_app(app)?;
    let mut store = load_store(app)?;
    let removed_current = store.settings.active_account_id.as_deref() == Some(id);
    let original_len = store.accounts.len();
    store.accounts.retain(|account| account.id != id);

    if original_len == store.accounts.len() {
        return Err("未找到要删除的账号".to_string());
    }

    if removed_current {
        store.settings.active_account_id = None;
    }
    if store
        .settings
        .active_hybrid_profile
        .as_ref()
        .is_some_and(|hybrid| hybrid.chatgpt_account_id == id || hybrid.relay_account_id == id)
    {
        store.settings.active_hybrid_profile = None;
    }
    if store.settings.model_router_account_id.as_deref() == Some(id) {
        store.settings.model_router_account_id = None;
    }
    store
        .settings
        .model_router_route_selections
        .retain(|selection| selection.account_id != id);
    let valid_account_keys = store
        .accounts
        .iter()
        .map(|account| account.account_key())
        .collect::<HashSet<_>>();
    for pool in &mut store.settings.account_pools {
        pool.account_keys
            .retain(|account_key| valid_account_keys.contains(account_key));
    }

    save_store(app, &store)?;
    profile_files::remove_account_profile_in_store_path(&store_path, id)?;
    Ok(())
}

fn build_sub2api_data_payload(store: &AccountsStore) -> Result<Sub2apiDataPayload, String> {
    let exported_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("生成 Sub2API 导出时间失败: {error}"))?;
    let accounts = store
        .accounts
        .iter()
        .filter_map(account_to_sub2api_data_account)
        .collect::<Vec<_>>();

    Ok(Sub2apiDataPayload {
        data_type: SUB2API_EXPORT_TYPE,
        version: SUB2API_EXPORT_VERSION,
        exported_at,
        proxies: Vec::new(),
        accounts,
    })
}

fn account_to_sub2api_data_account(account: &StoredAccount) -> Option<Sub2apiDataAccount> {
    match account.source_kind {
        AccountSourceKind::Chatgpt => chatgpt_account_to_sub2api_data_account(account),
        AccountSourceKind::Relay => relay_account_to_sub2api_data_account(account),
    }
}

fn chatgpt_account_to_sub2api_data_account(account: &StoredAccount) -> Option<Sub2apiDataAccount> {
    let tokens = account.auth_json.get("tokens")?.as_object()?;
    let credentials = sub2api_openai_oauth_credentials(account, tokens);
    if credentials
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
        && credentials
            .get("refresh_token")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return None;
    }

    Some(Sub2apiDataAccount {
        name: account.label.clone(),
        notes: Some("Imported from CodexDeck".to_string()),
        platform: "openai".to_string(),
        account_type: "oauth".to_string(),
        credentials,
        extra: sub2api_openai_oauth_extra(),
        proxy_key: None,
        concurrency: 3,
        priority: 100,
        rate_multiplier: None,
        expires_at: None,
        auto_pause_on_expired: Some(true),
    })
}

fn relay_account_to_sub2api_data_account(account: &StoredAccount) -> Option<Sub2apiDataAccount> {
    let api_key = account.primary_relay_api_key()?.trim();
    if api_key.is_empty() {
        return None;
    }
    let base_url = account
        .api_base_url
        .as_deref()?
        .trim()
        .trim_end_matches('/');
    if base_url.is_empty() {
        return None;
    }

    let mut credentials = serde_json::Map::new();
    credentials.insert(
        "api_key".to_string(),
        serde_json::Value::String(api_key.to_string()),
    );
    credentials.insert(
        "base_url".to_string(),
        serde_json::Value::String(base_url.to_string()),
    );
    if let Some(model_name) = account
        .model_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        credentials.insert(
            "default_model".to_string(),
            serde_json::Value::String(model_name.to_string()),
        );
    }

    Some(Sub2apiDataAccount {
        name: account.label.clone(),
        notes: Some("Imported from CodexDeck".to_string()),
        platform: "openai".to_string(),
        account_type: "apikey".to_string(),
        credentials,
        extra: sub2api_openai_apikey_extra(account),
        proxy_key: None,
        concurrency: 3,
        priority: 100,
        rate_multiplier: None,
        expires_at: None,
        auto_pause_on_expired: Some(true),
    })
}

fn sub2api_openai_oauth_credentials(
    account: &StoredAccount,
    tokens: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut credentials = serde_json::Map::new();
    copy_string_field(tokens, &mut credentials, "access_token");
    copy_string_field(tokens, &mut credentials, "refresh_token");
    copy_string_field(tokens, &mut credentials, "id_token");
    copy_string_field(tokens, &mut credentials, "account_id");

    let id_token_claims = credentials
        .get("id_token")
        .and_then(serde_json::Value::as_str)
        .and_then(jwt_claims);
    let access_token_claims = credentials
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .and_then(jwt_claims);

    let client_id = id_token_claims
        .as_ref()
        .and_then(client_id_from_jwt_claims)
        .unwrap_or(OPENAI_CODEX_CLI_CLIENT_ID);
    insert_string_if_absent(&mut credentials, "client_id", client_id);

    if let Some(access_token) = credentials
        .get("access_token")
        .and_then(serde_json::Value::as_str)
    {
        if let Some(expires_at) = jwt_expiration_rfc3339(access_token) {
            credentials.insert(
                "expires_at".to_string(),
                serde_json::Value::String(expires_at),
            );
        }
    }

    if let Some(claims) = id_token_claims.as_ref() {
        if let Some(email) = claims.get("email").and_then(serde_json::Value::as_str) {
            insert_string_if_absent(&mut credentials, "email", email);
        }
        if let Some(auth_claims) = openai_auth_claims(claims) {
            if let Some(value) = auth_claims
                .get("chatgpt_account_id")
                .and_then(serde_json::Value::as_str)
            {
                insert_string_if_absent(&mut credentials, "chatgpt_account_id", value);
            }
            if let Some(value) = auth_claims
                .get("chatgpt_user_id")
                .or_else(|| auth_claims.get("user_id"))
                .and_then(serde_json::Value::as_str)
            {
                insert_string_if_absent(&mut credentials, "chatgpt_user_id", value);
            }
            if let Some(value) = auth_claims
                .get("chatgpt_plan_type")
                .and_then(serde_json::Value::as_str)
            {
                insert_string_if_absent(&mut credentials, "plan_type", value);
            }
            if let Some(value) = auth_claims.get("poid").and_then(serde_json::Value::as_str) {
                insert_string_if_absent(&mut credentials, "organization_id", value);
            }
        }
    }
    if let Some(claims) = access_token_claims.as_ref() {
        if let Some(value) = openai_auth_claims(claims)
            .and_then(|auth_claims| auth_claims.get("poid"))
            .and_then(serde_json::Value::as_str)
        {
            insert_string_if_absent(&mut credentials, "organization_id", value);
        }
    }

    if let Some(email) = account
        .email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        credentials.insert(
            "email".to_string(),
            serde_json::Value::String(email.to_string()),
        );
    }
    if let Some(plan_type) = account
        .resolved_plan_type()
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        credentials.insert(
            "plan_type".to_string(),
            serde_json::Value::String(plan_type.to_string()),
        );
    }
    if !account.account_id.trim().is_empty() {
        credentials.insert(
            "chatgpt_account_id".to_string(),
            serde_json::Value::String(account.account_id.trim().to_string()),
        );
    }
    if let Some(expires_at) = account.api_quota_subscription_expires_at {
        if let Ok(expires_at) = OffsetDateTime::from_unix_timestamp(expires_at) {
            if let Ok(subscription_expires_at) = expires_at.format(&Rfc3339) {
                insert_string_if_absent(
                    &mut credentials,
                    "subscription_expires_at",
                    &subscription_expires_at,
                );
            }
        }
    }

    credentials
}

fn sub2api_openai_oauth_extra() -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "openai_passthrough".to_string(),
        serde_json::Value::Bool(true),
    );
    extra.insert("codex_cli_only".to_string(), serde_json::Value::Bool(true));
    Some(extra)
}

fn sub2api_openai_apikey_extra(
    account: &StoredAccount,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "openai_passthrough".to_string(),
        serde_json::Value::Bool(true),
    );
    if let Some(model_name) = account
        .model_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        extra.insert(
            "default_model".to_string(),
            serde_json::Value::String(model_name.to_string()),
        );
    }
    Some(extra)
}

fn copy_string_field(
    from: &serde_json::Map<String, serde_json::Value>,
    to: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) {
    if let Some(value) = from
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        to.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
}

fn insert_string_if_absent(
    target: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: &str,
) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if target
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|existing| !existing.is_empty())
        .is_some()
    {
        return;
    }
    target.insert(
        key.to_string(),
        serde_json::Value::String(value.to_string()),
    );
}

fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| {
            let remainder = payload.len() % 4;
            let padded = if remainder == 0 {
                payload.to_string()
            } else {
                format!("{payload}{}", "=".repeat(4 - remainder))
            };
            base64::engine::general_purpose::URL_SAFE.decode(padded)
        })
        .ok()?;
    serde_json::from_slice::<serde_json::Value>(&decoded).ok()
}

fn openai_auth_claims(
    claims: &serde_json::Value,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    claims
        .get("https://api.openai.com/auth")
        .and_then(serde_json::Value::as_object)
}

fn client_id_from_jwt_claims(claims: &serde_json::Value) -> Option<&str> {
    match claims.get("aud")? {
        serde_json::Value::String(value) => {
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
        }
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::trim)
            .find(|value| !value.is_empty()),
        _ => None,
    }
}

fn jwt_expiration_rfc3339(token: &str) -> Option<String> {
    let claims = jwt_claims(token)?;
    let exp = claims.get("exp").and_then(serde_json::Value::as_i64)?;
    OffsetDateTime::from_unix_timestamp(exp)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

pub(crate) async fn update_account_label_internal(
    app: &AppHandle,
    state: &AppState,
    account_key: &str,
    label: String,
) -> Result<String, String> {
    let resolved_label =
        normalize_custom_label(Some(label)).ok_or_else(|| "账号别名不能为空".to_string())?;
    let now = now_unix_seconds();

    let _guard = state.store_lock.lock().await;
    let mut store = load_store(app)?;
    let mut updated = false;

    for account in store
        .accounts
        .iter_mut()
        .filter(|account| account.account_key() == account_key)
    {
        account.label = resolved_label.clone();
        account.updated_at = now;
        updated = true;
    }

    if !updated {
        return Err("未找到要设置别名的账号".to_string());
    }

    save_store(app, &store)?;
    Ok(resolved_label)
}

pub(crate) async fn update_api_account_internal(
    app: &AppHandle,
    state: &AppState,
    account_key: &str,
    input: crate::models::UpdateApiAccountInput,
) -> Result<AccountSummary, String> {
    let resolved_label = profile_files::normalize_relay_label(&input.label)?;
    let resolved_base_url = profile_files::normalize_relay_base_url(&input.base_url)?;
    let resolved_model_name = profile_files::normalize_relay_model_name(&input.model_name)?;
    let resolved_model_catalog =
        normalize_relay_model_catalog(Some(resolved_model_name.as_str()), &input.model_catalog);
    let now = now_unix_seconds();
    let current_account_key = current_auth_account_key();
    let current_variant_key = current_auth_variant_key();

    let _guard = state.store_lock.lock().await;
    let mut store = load_store(app)?;
    let store_path = account_store_path_for_app(app)?;
    let active_account_id = store.settings.active_account_id.clone();

    let updated_account = {
        let account = store
            .accounts
            .iter_mut()
            .find(|account| account.account_key() == account_key)
            .ok_or_else(|| "未找到要更新的 API 账号".to_string())?;

        if !matches!(account.source_kind, AccountSourceKind::Relay) {
            return Err("只有 API 账号支持编辑接口信息".to_string());
        }

        let maybe_new_api_key = match input
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(api_key) => Some(profile_files::normalize_relay_api_key(api_key)?),
            None => None,
        };

        let (provider_id, provider_name) =
            infer_provider_metadata_from_base_url(Some(resolved_base_url.as_str()));
        let quota_api_key = maybe_new_api_key
            .as_deref()
            .or_else(|| account.primary_relay_api_key());
        let quota_fields = resolve_update_api_quota_fields(
            &input,
            &resolved_label,
            &resolved_base_url,
            quota_api_key,
        )
        .await;
        let reset_proxy_capabilities = relay_profile_change_requires_proxy_reset(
            account,
            resolved_base_url.as_str(),
            resolved_model_name.as_str(),
            maybe_new_api_key.as_deref(),
        );

        account.label = resolved_label.clone();
        account.api_base_url = Some(resolved_base_url);
        if let Some(resolved_api_key) = maybe_new_api_key {
            account.auth_json = profile_files::build_api_auth_json(&resolved_api_key);
            account.api_key = Some(resolved_api_key.clone());
            replace_primary_relay_proxy_key(account, &resolved_label, &resolved_api_key, now);
        }
        sync_primary_api_key_from_relay_key_pool(account);
        account.model_name = Some(resolved_model_name);
        account.model_catalog = resolved_model_catalog;
        account.model_routing_enabled = false;
        let balance_display_enabled = input.balance_display_enabled.unwrap_or(true);
        account.balance_display_enabled = balance_display_enabled;
        if balance_display_enabled {
            account.api_quota_mode = quota_fields.mode;
            account.api_quota_today_used_text = quota_fields.today_used_text;
            account.api_quota_remaining_text = quota_fields.remaining_text;
            account.api_quota_total_remaining_text = quota_fields.total_remaining_text;
            account.api_quota_total_tokens_text = quota_fields.total_tokens_text;
            account.api_quota_today_tokens_text = quota_fields.today_tokens_text;
            account.api_quota_daily_window = quota_fields.daily_window;
            account.api_quota_total_window = quota_fields.total_window;
            account.api_quota_subscription_expires_at = quota_fields.subscription_expires_at;
            account.api_quota_subscription_name = quota_fields.subscription_name;
        } else {
            clear_api_quota_fields(account);
        }
        account.provider_id = provider_id;
        account.provider_name = provider_name;
        account.updated_at = now;
        account.profile_last_validated_at = None;
        account.profile_last_validation_error = if reset_proxy_capabilities {
            account.proxy_endpoints = vec![ProxyEndpointCapability::ChatCompletions];
            Some(
                "接口能力已重置为仅 /v1/chat/completions；如需 Responses/Compact，请重新探测或手动开启。"
                    .to_string(),
            )
        } else {
            None
        };
        account.profile_integrity_error = None;

        profile_files::sync_account_profile_in_store_path(&store_path, account)?;
        account.clone()
    };

    if active_account_id.as_deref() == Some(updated_account.id.as_str())
        || store
            .settings
            .active_hybrid_profile
            .as_ref()
            .is_some_and(|hybrid| hybrid.relay_account_id == updated_account.id)
    {
        model_router::stop_model_router(state).await;
        apply_current_relay_profile(&store, &updated_account).await?;
    }

    let summary = build_account_summaries_for_store(
        &store,
        current_account_key.as_deref(),
        current_variant_key.as_deref(),
    )
    .into_iter()
    .find(|account| account.id == updated_account.id)
    .unwrap_or_else(|| {
        updated_account.to_summary(
            current_account_key.as_deref(),
            current_variant_key.as_deref(),
        )
    });
    save_store(app, &store)?;
    Ok(summary)
}

pub(crate) async fn update_account_tags_internal(
    app: &AppHandle,
    state: &AppState,
    account_key: &str,
    tags: Vec<String>,
) -> Result<Vec<String>, String> {
    let resolved_tags = normalize_account_tags(tags);
    let now = now_unix_seconds();

    let _guard = state.store_lock.lock().await;
    let mut store = load_store(app)?;
    let mut updated = false;

    for account in store
        .accounts
        .iter_mut()
        .filter(|account| account.account_key() == account_key)
    {
        account.tags = resolved_tags.clone();
        account.updated_at = now;
        updated = true;
    }

    if !updated {
        return Err("未找到要更新标签的账号".to_string());
    }

    save_store(app, &store)?;
    Ok(resolved_tags)
}

pub(crate) async fn update_api_account_keys_internal(
    app: &AppHandle,
    state: &AppState,
    account_key: &str,
    keys: Vec<UpdateApiAccountKeyInput>,
) -> Result<AccountSummary, String> {
    if keys.is_empty() {
        return Err("至少保留一个 API Key。".to_string());
    }

    let now = now_unix_seconds();
    let current_account_key = current_auth_account_key();
    let current_variant_key = current_auth_variant_key();

    let _guard = state.store_lock.lock().await;
    let mut store = load_store(app)?;
    let store_path = account_store_path_for_app(app)?;
    let active_account_id = store.settings.active_account_id.clone();

    let updated_account = {
        let account = store
            .accounts
            .iter_mut()
            .find(|account| account.account_key() == account_key)
            .ok_or_else(|| "未找到要更新 Key 池的 API 账号".to_string())?;

        if !matches!(account.source_kind, AccountSourceKind::Relay) {
            return Err("只有 API 账号支持 Key 池管理。".to_string());
        }

        account.sync_relay_api_keys_from_legacy();
        account.api_keys = normalize_relay_key_pool(account, keys, now)?;
        sync_primary_api_key_from_relay_key_pool(account);
        account.updated_at = now;
        account.profile_last_validated_at = None;
        account.profile_last_validation_error = None;
        account.profile_integrity_error = None;

        profile_files::sync_account_profile_in_store_path(&store_path, account)?;
        account.clone()
    };

    if active_account_id.as_deref() == Some(updated_account.id.as_str())
        || store
            .settings
            .active_hybrid_profile
            .as_ref()
            .is_some_and(|hybrid| hybrid.relay_account_id == updated_account.id)
    {
        model_router::stop_model_router(state).await;
        apply_current_relay_profile(&store, &updated_account).await?;
    }

    let summary = build_account_summaries_for_store(
        &store,
        current_account_key.as_deref(),
        current_variant_key.as_deref(),
    )
    .into_iter()
    .find(|account| account.id == updated_account.id)
    .unwrap_or_else(|| {
        updated_account.to_summary(
            current_account_key.as_deref(),
            current_variant_key.as_deref(),
        )
    });
    save_store(app, &store)?;
    Ok(summary)
}

pub(crate) async fn probe_api_account_key_internal(
    app: &AppHandle,
    state: &AppState,
    account_key: &str,
    key_id: &str,
) -> Result<AccountSummary, String> {
    let now = now_unix_seconds();
    let current_account_key = current_auth_account_key();
    let current_variant_key = current_auth_variant_key();
    let (base_url, api_key) = {
        let _guard = state.store_lock.lock().await;
        let mut store = load_store(app)?;
        let account = store
            .accounts
            .iter_mut()
            .find(|account| account.account_key() == account_key)
            .ok_or_else(|| "未找到要检测的 API 账号".to_string())?;

        if !matches!(account.source_kind, AccountSourceKind::Relay) {
            return Err("只有 API 账号支持 Key 探测。".to_string());
        }

        account.sync_relay_api_keys_from_legacy();
        let base_url = account
            .api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "API 条目缺少 Base URL。".to_string())?
            .to_string();
        let api_key = account
            .api_keys
            .iter()
            .find(|key| key.id == key_id)
            .and_then(|key| key.secret.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "未找到要检测的 API Key。".to_string())?
            .to_string();
        save_store(app, &store)?;
        (base_url, api_key)
    };

    let probe_result = probe_relay_api_key(&base_url, &api_key).await;

    let summary = {
        let _guard = state.store_lock.lock().await;
        let mut store = load_store(app)?;
        let store_path = account_store_path_for_app(app)?;
        let active_account_id = store.settings.active_account_id.clone();
        let account = store
            .accounts
            .iter_mut()
            .find(|account| account.account_key() == account_key)
            .ok_or_else(|| "未找到要检测的 API 账号".to_string())?;
        let current_base_url = account
            .api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "API 条目缺少 Base URL。".to_string())?
            .to_string();
        if current_base_url != base_url {
            return Err("API 条目的 Base URL 已变更，请重新检测。".to_string());
        }

        let key = account
            .api_keys
            .iter_mut()
            .find(|key| key.id == key_id)
            .ok_or_else(|| "未找到要检测的 API Key。".to_string())?;
        let current_api_key = key
            .secret
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "未找到要检测的 API Key。".to_string())?
            .to_string();
        if current_api_key != api_key {
            return Err("API Key 已变更，请重新检测。".to_string());
        }

        match &probe_result {
            Ok(()) => {
                key.health_status = ProxyHealthStatus::Healthy;
                key.last_error = None;
                key.cooldown_until = None;
                key.failure_count = 0;
            }
            Err(error) => {
                key.health_status = probe_failure_health_status(error.status);
                key.last_error = Some(redact_sensitive_text(&error.message));
                key.cooldown_until = probe_failure_cooldown_until(error.status, now);
                key.failure_count = key.failure_count.saturating_add(1);
            }
        }
        key.updated_at = Some(now);
        account.updated_at = now;
        sync_primary_api_key_from_relay_key_pool(account);
        profile_files::sync_account_profile_in_store_path(&store_path, account)?;
        let updated_account = account.clone();

        if active_account_id.as_deref() == Some(updated_account.id.as_str())
            || store
                .settings
                .active_hybrid_profile
                .as_ref()
                .is_some_and(|hybrid| hybrid.relay_account_id == updated_account.id)
        {
            model_router::stop_model_router(state).await;
            apply_current_relay_profile(&store, &updated_account).await?;
        }

        let summary = build_account_summaries_for_store(
            &store,
            current_account_key.as_deref(),
            current_variant_key.as_deref(),
        )
        .into_iter()
        .find(|account| account.id == updated_account.id)
        .unwrap_or_else(|| {
            updated_account.to_summary(
                current_account_key.as_deref(),
                current_variant_key.as_deref(),
            )
        });
        save_store(app, &store)?;
        summary
    };

    match probe_result {
        Ok(()) => Ok(summary),
        Err(error) => Err(error.message),
    }
}

/// 拉取并刷新所有账号用量，返回可直接用于前端/状态栏显示的摘要。
///
/// 为避免“后台刷新覆盖新增账号”的竞态：
/// 1) 先拿快照用于网络请求；
/// 2) 请求完成后重新加载最新 store 并按账号组写回。
#[derive(Debug)]
struct RefreshTarget {
    account_key: String,
    auth_json: serde_json::Value,
    auth_is_current: bool,
    auth_refresh_blocked: bool,
    auth_refresh_error: Option<String>,
    auth_refresh_next_at: Option<i64>,
    source_auth_last_refresh: Option<i64>,
    updated_at: i64,
}

#[derive(Debug)]
struct RefreshOutcome {
    usage: Option<crate::models::UsageSnapshot>,
    usage_error: Option<String>,
    updated_at: i64,
    auth_plan_type: Option<String>,
    auth_email: Option<String>,
    auth_json: serde_json::Value,
    auth_is_current: bool,
    auth_refreshed: bool,
    auth_refresh_blocked: bool,
    auth_refresh_error: Option<String>,
    auth_refresh_next_at: Option<i64>,
    source_updated_at: i64,
    source_auth_last_refresh: Option<i64>,
}

pub(crate) async fn refresh_all_usage_internal(
    app: &AppHandle,
    state: &AppState,
    force_auth_refresh: bool,
) -> Result<Vec<AccountSummary>, String> {
    refresh_usage_internal(app, state, force_auth_refresh, None).await
}

pub(crate) async fn refresh_usage_for_account_keys_internal(
    app: &AppHandle,
    state: &AppState,
    account_keys: Vec<String>,
    force_auth_refresh: bool,
) -> Result<Vec<AccountSummary>, String> {
    let requested_keys = account_keys
        .into_iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<HashSet<_>>();
    if requested_keys.is_empty() {
        return list_accounts_internal(app, state).await;
    }

    refresh_usage_internal(app, state, force_auth_refresh, Some(requested_keys)).await
}

pub(crate) async fn refresh_api_quota_for_account_keys_internal(
    app: &AppHandle,
    state: &AppState,
    account_keys: Vec<String>,
) -> Result<Vec<AccountSummary>, String> {
    let requested_keys = account_keys
        .into_iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<HashSet<_>>();
    if requested_keys.is_empty() {
        return list_accounts_internal(app, state).await;
    }

    refresh_api_quota_internal(app, state, Some(requested_keys)).await
}

pub(crate) async fn refresh_all_api_quota_internal(
    app: &AppHandle,
    state: &AppState,
) -> Result<Vec<AccountSummary>, String> {
    refresh_api_quota_internal(app, state, None).await
}

async fn refresh_api_quota_internal(
    app: &AppHandle,
    state: &AppState,
    account_key_filter: Option<HashSet<String>>,
) -> Result<Vec<AccountSummary>, String> {
    let targets = {
        let _guard = state.store_lock.lock().await;
        let store = load_store(app)?;
        build_api_quota_refresh_targets(&store, account_key_filter.as_ref())
    };

    let snapshots: HashMap<String, Result<notification_service::ApiQuotaSnapshot, String>> =
        stream::iter(targets.into_iter().map(|(account_key, target)| async move {
            let result = fetch_api_quota_refresh_target(target).await;
            (account_key, result)
        }))
        .buffer_unordered(API_QUOTA_REFRESH_CONCURRENCY)
        .collect()
        .await;

    let store = {
        let _guard = state.store_lock.lock().await;
        let mut store = load_store(app)?;
        let now = now_unix_seconds();

        for account in &mut store.accounts {
            let Some(result) = snapshots.get(&account.account_key()) else {
                continue;
            };

            match result {
                Ok(snapshot) => {
                    apply_api_quota_snapshot(account, snapshot, now);
                    account.profile_last_validation_error = None;
                }
                Err(error) => {
                    clear_api_quota_snapshot_fields(account);
                    account.profile_last_validation_error =
                        Some(redact_sensitive_text(error.as_str()));
                    account.updated_at = now;
                }
            }
        }

        save_store(app, &store)?;
        store
    };

    let current_account_key = current_auth_account_key();
    let current_variant_key = current_auth_variant_key();
    Ok(build_account_summaries_for_store(
        &store,
        current_account_key.as_deref(),
        current_variant_key.as_deref(),
    ))
}

async fn fetch_api_quota_refresh_target(
    target: ApiQuotaRefreshTarget,
) -> Result<notification_service::ApiQuotaSnapshot, String> {
    match target {
        ApiQuotaRefreshTarget::PlatformProvider { provider, fallback } => {
            let platform_result = notification_service::fetch_api_quota_snapshot(provider).await;
            fetch_api_quota_refresh_target_with_platform_fallback(platform_result, fallback).await
        }
        ApiQuotaRefreshTarget::NewapiToken { base_url, api_key } => {
            fetch_api_key_quota_snapshot(&base_url, &api_key).await
        }
    }
}

async fn fetch_api_quota_refresh_target_with_platform_fallback(
    platform_result: Result<notification_service::ApiQuotaSnapshot, String>,
    fallback: Option<Box<ApiQuotaRefreshTarget>>,
) -> Result<notification_service::ApiQuotaSnapshot, String> {
    match platform_result {
        Ok(snapshot) => Ok(snapshot),
        Err(platform_error) => {
            if let Some(ApiQuotaRefreshTarget::NewapiToken { base_url, api_key }) =
                fallback.map(|target| *target)
            {
                match fetch_api_key_quota_snapshot(&base_url, &api_key).await {
                    Ok(snapshot) => Ok(snapshot),
                    Err(api_key_error) => Err(format!(
                        "{platform_error}；API Key 回退失败：{api_key_error}"
                    )),
                }
            } else {
                Err(platform_error)
            }
        }
    }
}

fn build_api_quota_refresh_targets(
    store: &AccountsStore,
    account_key_filter: Option<&HashSet<String>>,
) -> Vec<(String, ApiQuotaRefreshTarget)> {
    let platform_base_url_account_counts = platform_api_quota_account_counts_by_base_url(store);
    store
        .accounts
        .iter()
        .filter(|account| matches!(account.source_kind, AccountSourceKind::Relay))
        .filter_map(|account| {
            let account_key = account.account_key();
            if account_key_filter.is_some_and(|filter| !filter.contains(&account_key)) {
                return None;
            }
            if !account.balance_display_enabled {
                return None;
            }
            let api_key_target = build_api_key_quota_refresh_target(account_key.clone(), account);
            if is_api_only_quota_account(account) {
                if supports_api_key_quota_refresh(account) {
                    return api_key_target;
                }
                return None;
            }

            if let Some(provider) = find_api_quota_provider_for_account(
                account,
                &store.settings.notification_providers,
                &platform_base_url_account_counts,
            ) {
                let fallback = api_key_target.map(|(_, target)| Box::new(target));
                return Some((
                    account_key,
                    ApiQuotaRefreshTarget::PlatformProvider { provider, fallback },
                ));
            }

            if supports_provider_api_key_quota_refresh(account) {
                return api_key_target;
            }

            None
        })
        .collect()
}

fn platform_api_quota_account_counts_by_base_url(store: &AccountsStore) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for account in store
        .accounts
        .iter()
        .filter(|account| matches!(account.source_kind, AccountSourceKind::Relay))
        .filter(|account| account.balance_display_enabled)
        .filter(|account| !is_api_only_quota_account(account))
    {
        let base_url = normalize_api_quota_provider_base_url(account.api_base_url.as_deref());
        if base_url.is_empty() {
            continue;
        }
        *counts.entry(base_url).or_insert(0) += 1;
    }
    counts
}

enum ApiQuotaRefreshTarget {
    PlatformProvider {
        provider: crate::models::NotificationProviderConfig,
        fallback: Option<Box<ApiQuotaRefreshTarget>>,
    },
    NewapiToken {
        base_url: String,
        api_key: String,
    },
}

async fn fetch_api_key_quota_snapshot(
    base_url: &str,
    api_key: &str,
) -> Result<notification_service::ApiQuotaSnapshot, String> {
    let allow_newapi_fallback = is_newapi_quota_base_url(base_url);
    match notification_service::fetch_provider_api_key_quota_snapshot(base_url, api_key).await {
        Ok(Some(snapshot)) => Ok(snapshot),
        Ok(None) if allow_newapi_fallback => {
            notification_service::fetch_newapi_token_quota_snapshot(base_url, api_key).await
        }
        Ok(None) => Err("当前官方供应商没有已支持的 API Key 余额接口。".to_string()),
        Err(provider_error) if !allow_newapi_fallback => Err(provider_error),
        Err(provider_error) => {
            match notification_service::fetch_newapi_token_quota_snapshot(base_url, api_key).await {
                Ok(snapshot) => Ok(snapshot),
                Err(newapi_error) => {
                    Err(format!("{provider_error}；NewAPI 回退失败：{newapi_error}"))
                }
            }
        }
    }
}

fn is_api_only_quota_account(account: &StoredAccount) -> bool {
    matches!(account.api_quota_mode, crate::models::ApiQuotaMode::ApiOnly)
        && account.balance_display_enabled
        && account.primary_relay_api_key().is_some()
}

fn supports_api_key_quota_refresh(account: &StoredAccount) -> bool {
    account.balance_display_enabled
        && account.primary_relay_api_key().is_some()
        && account.api_base_url.as_deref().is_some_and(|base_url| {
            notification_service::supports_provider_api_key_quota(base_url)
                || is_newapi_quota_base_url(base_url)
        })
}

fn supports_provider_api_key_quota_refresh(account: &StoredAccount) -> bool {
    account.balance_display_enabled
        && account.primary_relay_api_key().is_some()
        && account
            .api_base_url
            .as_deref()
            .is_some_and(notification_service::supports_provider_api_key_quota)
}

fn is_newapi_quota_base_url(base_url: &str) -> bool {
    let normalized = base_url.trim().to_ascii_lowercase();
    !(normalized.contains("xiaomimimo.com")
        || normalized.contains("api.deepseek.com")
        || normalized.contains("api.moonshot.cn")
        || normalized.contains("api.moonshot.ai")
        || normalized.contains("api.moonshot.com")
        || normalized.contains("api.kimi.com")
        || normalized.contains("api.z.ai")
        || normalized.contains("bigmodel.cn")
        || normalized.contains("api.minimaxi.com")
        || normalized.contains("minimaxi.com")
        || normalized.contains("api.minimax.io")
        || normalized.contains("minimax.io"))
}

fn build_api_key_quota_refresh_target(
    account_key: String,
    account: &StoredAccount,
) -> Option<(String, ApiQuotaRefreshTarget)> {
    account
        .primary_relay_api_key()
        .filter(|api_key| !api_key.trim().is_empty())
        .map(|api_key| {
            (
                account_key,
                ApiQuotaRefreshTarget::NewapiToken {
                    base_url: account.api_base_url.clone().unwrap_or_default(),
                    api_key: api_key.to_string(),
                },
            )
        })
}

fn find_api_quota_provider_for_account(
    account: &StoredAccount,
    providers: &[crate::models::NotificationProviderConfig],
    platform_base_url_account_counts: &HashMap<String, usize>,
) -> Option<crate::models::NotificationProviderConfig> {
    let account_base_url = normalize_api_quota_provider_base_url(account.api_base_url.as_deref());
    if account_base_url.is_empty() {
        return None;
    }

    let account_key = account.account_key();
    let matching_providers = providers
        .iter()
        .filter(|provider| {
            provider.email.trim().len() > 0
                && provider
                    .password
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|password| !password.is_empty())
        })
        .filter(|provider| {
            normalize_api_quota_provider_base_url(Some(&provider.base_url)) == account_base_url
        })
        .collect::<Vec<_>>();

    if let Some(provider) = matching_providers
        .iter()
        .find(|provider| provider.account_key.as_deref() == Some(account_key.as_str()))
    {
        return Some((*provider).clone());
    }

    let mut unbound_providers = matching_providers.into_iter().filter(|provider| {
        provider
            .account_key
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
    });
    let provider = unbound_providers.next()?;
    if unbound_providers.next().is_some() {
        return None;
    }
    if platform_base_url_account_counts
        .get(&account_base_url)
        .copied()
        .unwrap_or(0)
        > 1
    {
        return None;
    }

    Some(provider.clone())
}

fn normalize_api_quota_provider_base_url(value: Option<&str>) -> String {
    value
        .unwrap_or_default()
        .trim()
        .trim_end_matches('/')
        .to_ascii_lowercase()
        .trim_end_matches("/api/v1")
        .trim_end_matches("/v1")
        .to_string()
}

fn apply_api_quota_snapshot(
    account: &mut StoredAccount,
    snapshot: &notification_service::ApiQuotaSnapshot,
    now: i64,
) {
    account.balance_display_enabled = true;
    account.api_quota_mode = snapshot.mode;
    account.api_quota_today_used_text = snapshot.today_used_text.clone();
    account.api_quota_remaining_text = snapshot.remaining_text.clone();
    account.api_quota_total_remaining_text = snapshot.total_remaining_text.clone();
    account.api_quota_total_tokens_text = snapshot.total_tokens_text.clone();
    account.api_quota_today_tokens_text = snapshot.today_tokens_text.clone();
    account.api_quota_daily_window = snapshot.daily_window.clone();
    account.api_quota_total_window = snapshot.total_window.clone();
    account.api_quota_subscription_expires_at = snapshot.subscription_expires_at;
    account.api_quota_subscription_name = account
        .api_quota_subscription_name
        .clone()
        .filter(|value| is_manual_api_quota_subscription_label(value))
        .or_else(|| snapshot.subscription_name.clone());
    account.balance_text = snapshot
        .remaining_text
        .clone()
        .or_else(|| snapshot.total_remaining_text.clone());
    account.updated_at = now;
}

fn clear_api_quota_fields(account: &mut StoredAccount) {
    account.balance_text = None;
    account.api_quota_mode = Default::default();
    clear_api_quota_snapshot_fields(account);
    if account
        .profile_last_validation_error
        .as_deref()
        .is_some_and(is_api_quota_error_message)
    {
        account.profile_last_validation_error = None;
    }
}

fn is_manual_api_quota_subscription_label(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "adagio"
            | "allegretto"
            | "allegro"
            | "lite"
            | "max"
            | "moderato"
            | "plus"
            | "pro"
            | "standard"
            | "ultra"
            | "vivace"
    )
}

fn clear_api_quota_snapshot_fields(account: &mut StoredAccount) {
    account.balance_text = None;
    account.api_quota_today_used_text = None;
    account.api_quota_remaining_text = None;
    account.api_quota_total_remaining_text = None;
    account.api_quota_total_tokens_text = None;
    account.api_quota_today_tokens_text = None;
    account.api_quota_daily_window = None;
    account.api_quota_total_window = None;
    account.api_quota_subscription_expires_at = None;
    account.api_quota_subscription_name = None;
}

fn is_api_quota_error_message(message: &str) -> bool {
    message.contains("NewAPI 额度接口")
        || message.contains("连接 NewAPI 额度接口失败")
        || message.contains("API 平台连接失败")
        || message.contains("API 平台接口失败")
        || message.contains("API 平台接口返回格式异常")
        || message.contains("API 平台用户接口失败")
        || message.contains("API 平台用量统计接口失败")
        || message.contains("API 平台 URL 无效")
}

async fn refresh_usage_internal(
    app: &AppHandle,
    state: &AppState,
    force_auth_refresh: bool,
    account_key_filter: Option<HashSet<String>>,
) -> Result<Vec<AccountSummary>, String> {
    let current_auth_override: Option<(String, serde_json::Value)> =
        read_current_codex_auth_optional()
            .ok()
            .flatten()
            .and_then(|auth_json| {
                extract_auth(&auth_json).ok().map(|auth| {
                    (
                        account_group_key(&auth.principal_id, &auth.account_id),
                        auth_json,
                    )
                })
            });

    let refresh_targets: Vec<RefreshTarget> = {
        let _guard = state.store_lock.lock().await;
        let store = load_store(app)?;
        build_refresh_targets(
            store.accounts,
            current_auth_override.as_ref(),
            account_key_filter.as_ref(),
        )
    };

    let outcomes: HashMap<String, RefreshOutcome> =
        stream::iter(refresh_targets.into_iter().map(|target| async move {
            let account_key = target.account_key.clone();
            let outcome = refresh_usage_for_target(app, state, &target, force_auth_refresh).await;
            (account_key, outcome)
        }))
        .buffer_unordered(USAGE_REFRESH_CONCURRENCY)
        .collect()
        .await;

    let store = {
        let _guard = state.store_lock.lock().await;
        let mut latest_store = load_store(app)?;
        let store_path = account_store_path_for_app(app)?;

        for account in &mut latest_store.accounts {
            let Some(outcome) = outcomes.get(&account.account_key()) else {
                continue;
            };

            let should_apply_auth = should_apply_auth_outcome(account, outcome);
            if should_apply_auth {
                account.auth_json = outcome.auth_json.clone();
                account.auth_refresh_blocked = outcome.auth_refresh_blocked;
                account.auth_refresh_error = outcome.auth_refresh_error.clone();
                account.auth_refresh_next_at = outcome.auth_refresh_next_at;
            }
            account.updated_at = account.updated_at.max(outcome.updated_at);
            account.email = outcome.auth_email.clone().or(account.email.clone());
            let preferred_auth_plan_type = if outcome.auth_is_current || outcome.auth_refreshed {
                outcome.auth_plan_type.clone()
            } else {
                outcome.auth_plan_type.clone().or(account.plan_type.clone())
            };
            if let Some(snapshot) = outcome.usage.clone() {
                let mut resolved_snapshot = snapshot;
                let resolved_plan_type = preferred_auth_plan_type
                    .clone()
                    .or(resolved_snapshot.plan_type.clone());
                resolved_snapshot.plan_type = resolved_plan_type.clone();
                account.plan_type = resolved_plan_type;
                account.usage = Some(resolved_snapshot);
            }
            if let Some(err) = outcome.usage_error.clone() {
                if preferred_auth_plan_type.is_some() {
                    account.plan_type = preferred_auth_plan_type;
                }
                account.usage_error = Some(err);
            } else if outcome.usage.is_some() {
                account.usage_error = None;
            }
            if !outcome.auth_refresh_blocked
                && (outcome.auth_is_current || outcome.auth_refreshed)
                && matches!(account.source_kind, AccountSourceKind::Chatgpt)
                && should_apply_auth
            {
                profile_files::sync_account_profile_in_store_path(&store_path, account)?;
            }
        }

        dedupe_account_variants(&mut latest_store.accounts);
        save_store(app, &latest_store)?;
        latest_store
    };

    // 与当前 auth 文件重新对齐，确保 current 标签准确。
    let current_account_key = current_auth_account_key();
    let current_variant_key = current_auth_variant_key();
    Ok(build_account_summaries_for_store(
        &store,
        current_account_key.as_deref(),
        current_variant_key.as_deref(),
    ))
}

fn build_refresh_targets(
    accounts: Vec<StoredAccount>,
    current_auth_override: Option<&(String, serde_json::Value)>,
    account_key_filter: Option<&HashSet<String>>,
) -> Vec<RefreshTarget> {
    let mut targets_by_account_key: HashMap<String, RefreshTarget> = HashMap::new();

    for account in accounts {
        if matches!(account.source_kind, AccountSourceKind::Relay) {
            continue;
        }
        if account.auth_refresh_blocked {
            continue;
        }

        let account_key = account.account_key();
        if let Some(filter) = account_key_filter {
            if !filter.contains(&account_key) {
                continue;
            }
        }

        let current_override = current_auth_override
            .filter(|(current_account_key, _)| current_account_key == &account_key);
        let stored_auth_json = account.auth_json;
        let should_use_current_auth = current_override
            .map(|(_, auth_json)| auth_json_is_at_least_as_fresh(auth_json, &stored_auth_json))
            .unwrap_or(false);
        let auth_is_current = current_override.is_some() && should_use_current_auth;
        let auth_json = current_override
            .filter(|_| should_use_current_auth)
            .map(|(_, auth_json)| auth_json.clone())
            .unwrap_or(stored_auth_json);
        let source_auth_last_refresh = auth_json_last_refresh_unix(&auth_json);

        let candidate = RefreshTarget {
            account_key: account_key.clone(),
            auth_json,
            auth_is_current,
            auth_refresh_blocked: account.auth_refresh_blocked,
            auth_refresh_error: account.auth_refresh_error.clone(),
            auth_refresh_next_at: account.auth_refresh_next_at,
            source_auth_last_refresh,
            updated_at: account.updated_at,
        };

        match targets_by_account_key.get_mut(&account_key) {
            Some(existing) => {
                if should_replace_refresh_target(existing, &candidate) {
                    *existing = candidate;
                } else if existing.auth_refresh_error.is_none() {
                    existing.auth_refresh_error = candidate.auth_refresh_error.clone();
                }
            }
            None => {
                targets_by_account_key.insert(account_key, candidate);
            }
        }
    }

    let mut targets = targets_by_account_key.into_values().collect::<Vec<_>>();
    targets.sort_by(|left, right| {
        right
            .auth_is_current
            .cmp(&left.auth_is_current)
            .then(right.updated_at.cmp(&left.updated_at))
            .then(left.account_key.cmp(&right.account_key))
    });
    targets
}

fn should_replace_refresh_target(existing: &RefreshTarget, candidate: &RefreshTarget) -> bool {
    if candidate.auth_is_current != existing.auth_is_current {
        return candidate.auth_is_current;
    }
    if candidate.auth_refresh_blocked != existing.auth_refresh_blocked {
        return !candidate.auth_refresh_blocked;
    }
    candidate.updated_at > existing.updated_at
}

fn auth_json_is_at_least_as_fresh(
    candidate: &serde_json::Value,
    reference: &serde_json::Value,
) -> bool {
    match (
        auth_json_last_refresh_unix(candidate),
        auth_json_last_refresh_unix(reference),
    ) {
        (Some(candidate_refresh), Some(reference_refresh)) => {
            candidate_refresh >= reference_refresh
        }
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => true,
    }
}

fn auth_json_is_newer_than_target(
    candidate: &serde_json::Value,
    candidate_updated_at: i64,
    target_last_refresh: Option<i64>,
    target_updated_at: i64,
) -> bool {
    match (auth_json_last_refresh_unix(candidate), target_last_refresh) {
        (Some(candidate_refresh), Some(target_refresh)) => candidate_refresh > target_refresh,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => candidate_updated_at > target_updated_at,
    }
}

fn should_apply_auth_outcome(account: &StoredAccount, outcome: &RefreshOutcome) -> bool {
    if outcome.auth_refreshed {
        return auth_json_is_newer_or_same_for_updated_at(
            &outcome.auth_json,
            outcome.updated_at,
            &account.auth_json,
            account.updated_at,
        );
    }
    let current_last_refresh = auth_json_last_refresh_unix(&account.auth_json);
    current_last_refresh == outcome.source_auth_last_refresh
        && account.updated_at <= outcome.source_updated_at
}

fn auth_json_is_newer_or_same_for_updated_at(
    candidate: &serde_json::Value,
    candidate_updated_at: i64,
    reference: &serde_json::Value,
    reference_updated_at: i64,
) -> bool {
    match (
        auth_json_last_refresh_unix(candidate),
        auth_json_last_refresh_unix(reference),
    ) {
        (Some(candidate_refresh), Some(reference_refresh)) => {
            candidate_refresh > reference_refresh
                || (candidate_refresh == reference_refresh
                    && candidate_updated_at >= reference_updated_at)
        }
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => candidate_updated_at >= reference_updated_at,
    }
}

fn auth_json_last_refresh_unix(auth_json: &serde_json::Value) -> Option<i64> {
    let value = auth_json.get("last_refresh")?;
    match value {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(timestamp) = trimmed.parse::<i64>() {
                return Some(if timestamp.abs() >= 1_000_000_000_000 {
                    timestamp / 1000
                } else {
                    timestamp
                });
            }
            OffsetDateTime::parse(trimmed, &Rfc3339)
                .ok()
                .map(|datetime| datetime.unix_timestamp())
        }
        _ => None,
    }
}

fn auth_tokens_need_keepalive_refresh(auth_json: &serde_json::Value) -> bool {
    auth_tokens_expire_within(auth_json, KEEPALIVE_REFRESH_WINDOW_SECS)
        || auth_json_last_refresh_unix(auth_json)
            .map(|last_refresh| {
                now_unix_seconds().saturating_sub(last_refresh) >= KEEPALIVE_REFRESH_INTERVAL_SECS
            })
            .unwrap_or(true)
}

async fn refresh_usage_for_target(
    app: &AppHandle,
    state: &AppState,
    target: &RefreshTarget,
    force_auth_refresh: bool,
) -> RefreshOutcome {
    let mut working_auth_json = target.auth_json.clone();
    let mut refresh_error: Option<String> = None;
    let mut proactive_refresh_error: Option<String> = None;
    let mut auth_refreshed = false;
    let mut auth_refresh_blocked = target.auth_refresh_blocked;
    let mut auth_refresh_error = target.auth_refresh_error.clone();

    if force_auth_refresh
        && !auth_refresh_blocked
        && should_refresh_auth_now(&working_auth_json, target.auth_refresh_next_at)
    {
        match refresh_latest_auth_for_target(app, state, target).await {
            Ok(refreshed) => {
                working_auth_json = refreshed;
                auth_refreshed = true;
                auth_refresh_blocked = false;
                auth_refresh_error = None;
            }
            Err(err) => {
                // 预刷新失败时，先继续沿用当前 access_token 拉取用量；
                // 只有当真实请求也失败时，才把账号标记为需要重新授权。
                proactive_refresh_error = Some(err);
            }
        }
    }

    let mut extracted = extract_auth(&working_auth_json);
    let mut fetch_result = match &extracted {
        Ok(auth) => fetch_usage_snapshot(&auth.access_token, &auth.account_id).await,
        Err(err) => Err(err.clone()),
    };

    if !auth_refresh_blocked && should_retry_with_token_refresh(&fetch_result) {
        match refresh_latest_auth_for_target(app, state, target).await {
            Ok(refreshed) => {
                working_auth_json = refreshed;
                auth_refreshed = true;
                auth_refresh_blocked = false;
                auth_refresh_error = None;
                extracted = extract_auth(&working_auth_json);
                fetch_result = match &extracted {
                    Ok(auth) => fetch_usage_snapshot(&auth.access_token, &auth.account_id).await,
                    Err(err) => Err(err.clone()),
                };
            }
            Err(err) => {
                handle_refresh_failure(
                    app,
                    state,
                    target,
                    &err,
                    &mut auth_refresh_blocked,
                    &mut auth_refresh_error,
                    &mut refresh_error,
                )
                .await;
            }
        }
    }

    let (auth_plan_type, auth_email) = match &extracted {
        Ok(auth) => (auth.plan_type.clone(), auth.email.clone()),
        Err(_) => (None, None),
    };

    let updated_at = now_unix_seconds();
    let usage = fetch_result.as_ref().ok().cloned();
    if usage.is_some() {
        auth_refresh_blocked = false;
        auth_refresh_error = None;
    }
    let usage_error = match fetch_result {
        Ok(_) => refresh_error.as_deref().map(normalize_usage_error_message),
        Err(err) => {
            let combined_error = if let Some(refresh_err) = refresh_error
                .as_deref()
                .or(proactive_refresh_error.as_deref())
            {
                format!("{err} | 令牌刷新失败: {refresh_err}")
            } else {
                err
            };
            Some(normalize_usage_error_message(&combined_error))
        }
    };

    let auth_refresh_next_at = if auth_refresh_blocked {
        None
    } else {
        auth_refresh_next_at(&working_auth_json)
    };

    RefreshOutcome {
        usage,
        usage_error,
        updated_at,
        auth_plan_type,
        auth_email,
        auth_json: working_auth_json,
        auth_is_current: target.auth_is_current,
        auth_refreshed,
        auth_refresh_blocked,
        auth_refresh_error,
        auth_refresh_next_at,
        source_updated_at: target.updated_at,
        source_auth_last_refresh: target.source_auth_last_refresh,
    }
}

fn should_refresh_auth_now(auth_json: &serde_json::Value, next_at: Option<i64>) -> bool {
    if next_at.is_some_and(|timestamp| timestamp <= now_unix_seconds()) {
        return true;
    }
    auth_tokens_need_keepalive_refresh(auth_json)
}

async fn refresh_latest_auth_for_target(
    app: &AppHandle,
    state: &AppState,
    target: &RefreshTarget,
) -> Result<serde_json::Value, String> {
    let _refresh_guard = state.auth_refresh_lock.lock().await;
    let latest = latest_refresh_account_for_target(app, state, target).await?;

    let Some(latest) = latest else {
        let refreshed = refresh_chatgpt_auth_tokens(&target.auth_json).await?;
        persist_account_refresh_state(
            app,
            state,
            &target.account_key,
            Some(&refreshed),
            false,
            None,
        )
        .await?;
        return Ok(refreshed);
    };

    if latest.auth_refresh_blocked {
        return Err(latest
            .auth_refresh_error
            .unwrap_or_else(|| "授权过期，请重新登录授权。".to_string()));
    }

    if auth_json_is_newer_than_target(
        &latest.auth_json,
        latest.updated_at,
        target.source_auth_last_refresh,
        target.updated_at,
    ) && !should_refresh_auth_now(&latest.auth_json, latest.auth_refresh_next_at)
    {
        return Ok(latest.auth_json);
    }

    let refreshed = refresh_chatgpt_auth_tokens(&latest.auth_json).await?;
    persist_account_refresh_state(
        app,
        state,
        &target.account_key,
        Some(&refreshed),
        false,
        None,
    )
    .await?;
    Ok(refreshed)
}

async fn latest_refresh_account_for_target(
    app: &AppHandle,
    state: &AppState,
    target: &RefreshTarget,
) -> Result<Option<StoredAccount>, String> {
    let _store_guard = state.store_lock.lock().await;
    let store = load_store(app)?;
    Ok(store
        .accounts
        .into_iter()
        .filter(|account| {
            matches!(account.source_kind, AccountSourceKind::Chatgpt)
                && account.account_key() == target.account_key
        })
        .max_by(|left, right| {
            auth_json_last_refresh_unix(&left.auth_json)
                .cmp(&auth_json_last_refresh_unix(&right.auth_json))
                .then(left.updated_at.cmp(&right.updated_at))
        }))
}

async fn persisted_auth_newer_than_target(
    app: &AppHandle,
    state: &AppState,
    target: &RefreshTarget,
) -> Result<bool, String> {
    let latest = latest_refresh_account_for_target(app, state, target).await?;
    Ok(latest.is_some_and(|account| {
        auth_json_is_newer_than_target(
            &account.auth_json,
            account.updated_at,
            target.source_auth_last_refresh,
            target.updated_at,
        )
    }))
}

async fn handle_refresh_failure(
    app: &AppHandle,
    state: &AppState,
    target: &RefreshTarget,
    raw_error: &str,
    auth_refresh_blocked: &mut bool,
    auth_refresh_error: &mut Option<String>,
    refresh_error: &mut Option<String>,
) {
    if should_suspend_auth_keepalive(raw_error) {
        if persisted_auth_newer_than_target(app, state, target)
            .await
            .unwrap_or(false)
        {
            return;
        }
        let normalized_error = normalize_usage_error_message(raw_error);
        *auth_refresh_blocked = true;
        *auth_refresh_error = Some(normalized_error.clone());
        if let Err(err) = persist_account_refresh_state(
            app,
            state,
            &target.account_key,
            None,
            true,
            Some(normalized_error.as_str()),
        )
        .await
        {
            *refresh_error = Some(err);
        }
        return;
    }

    *refresh_error = Some(raw_error.to_string());
}

async fn persist_account_refresh_state(
    app: &AppHandle,
    state: &AppState,
    account_key: &str,
    auth_json: Option<&serde_json::Value>,
    auth_refresh_blocked: bool,
    auth_refresh_error: Option<&str>,
) -> Result<(), String> {
    let _guard = state.store_lock.lock().await;
    let store_path = account_store_path_for_app(app)?;
    update_account_group_refresh_state_in_path(
        &store_path,
        account_key,
        auth_json,
        auth_refresh_blocked,
        auth_refresh_error,
        now_unix_seconds(),
        true,
    )?;
    Ok(())
}

fn should_retry_with_token_refresh(
    fetch_result: &Result<crate::models::UsageSnapshot, String>,
) -> bool {
    match fetch_result {
        Ok(snapshot) => snapshot.plan_type.is_none(),
        Err(err) => {
            let normalized = err.to_ascii_lowercase();
            normalized.contains("401")
                || normalized.contains("unauthorized")
                || normalized.contains("invalid_token")
                || normalized.contains("deactivated_workspace")
        }
    }
}

fn should_suspend_auth_keepalive(raw_error: &str) -> bool {
    let normalized = raw_error.to_ascii_lowercase();
    normalized.contains("refresh_token_reused")
        || normalized.contains("provided authentication token is expired")
        || normalized
            .contains("your refresh token has already been used to generate a new access token")
        || normalized.contains("please try signing in again")
        || normalized.contains("token is expired")
        || normalized.contains("deactivated_workspace")
        || normalized.contains("your openai account has been deactivated")
        || normalized.contains("account has been deactivated")
        || normalized.contains("account deactivated")
        || normalized.contains("deactivated_user")
        || normalized.contains("auth.json 缺少 refresh_token")
}

fn normalize_usage_error_message(raw_error: &str) -> String {
    let normalized = raw_error.to_ascii_lowercase();
    if normalized.contains("deactivated_workspace") {
        return DEACTIVATED_WORKSPACE_NOTICE.to_string();
    }
    if normalized.contains("your openai account has been deactivated")
        || normalized.contains("account has been deactivated")
        || normalized.contains("account deactivated")
        || normalized.contains("deactivated_user")
        || (normalized.contains("deactivated") && normalized.contains("check your email"))
    {
        return DEACTIVATED_ACCOUNT_NOTICE.to_string();
    }
    if normalized.contains("refresh_token_reused")
        || normalized.contains("provided authentication token is expired")
        || normalized
            .contains("your refresh token has already been used to generate a new access token")
        || normalized.contains("please try signing in again")
        || normalized.contains("token is expired")
        || normalized.contains("auth.json 缺少 refresh_token")
    {
        return AUTH_EXPIRED_NOTICE.to_string();
    }
    raw_error.to_string()
}

fn relay_proxy_key(account_id: &str, label: &str, api_key: &str, updated_at: i64) -> ProxyKey {
    ProxyKey {
        id: format!("key:{account_id}:primary"),
        label: Some(label.to_string()),
        secret: Some(api_key.to_string()),
        enabled: true,
        priority: 100,
        weight: 100,
        health_status: ProxyHealthStatus::Healthy,
        last_error: None,
        cooldown_until: None,
        failure_count: 0,
        last_used_at: None,
        updated_at: Some(updated_at),
    }
}

fn replace_primary_relay_proxy_key(
    account: &mut StoredAccount,
    label: &str,
    api_key: &str,
    updated_at: i64,
) {
    let primary_key_id = format!("key:{}:primary", account.id);
    if let Some(existing) = account
        .api_keys
        .iter_mut()
        .find(|key| key.id == primary_key_id)
    {
        existing.label = Some(label.to_string());
        existing.secret = Some(api_key.to_string());
        existing.enabled = true;
        existing.health_status = ProxyHealthStatus::Healthy;
        existing.last_error = None;
        existing.cooldown_until = None;
        existing.updated_at = Some(updated_at);
        return;
    }

    account
        .api_keys
        .insert(0, relay_proxy_key(&account.id, label, api_key, updated_at));
}

fn relay_profile_change_requires_proxy_reset(
    account: &StoredAccount,
    next_base_url: &str,
    next_model_name: &str,
    next_api_key: Option<&str>,
) -> bool {
    let base_url_changed = account.api_base_url.as_deref() != Some(next_base_url);
    let model_changed = account.model_name.as_deref() != Some(next_model_name);
    let api_key_changed = next_api_key
        .map(|value| account.api_key.as_deref() != Some(value))
        .unwrap_or(false);

    base_url_changed || model_changed || api_key_changed
}

fn sync_primary_api_key_from_relay_key_pool(account: &mut StoredAccount) {
    let now = now_unix_seconds();
    let primary_secret = account
        .api_keys
        .iter()
        .find(|key| {
            key.enabled
                && key
                    .secret
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some()
                && matches!(
                    key.health_status,
                    ProxyHealthStatus::Healthy | ProxyHealthStatus::Degraded
                )
                && key.cooldown_until.map(|until| until <= now).unwrap_or(true)
        })
        .or_else(|| {
            account.api_keys.iter().find(|key| {
                key.enabled
                    && key
                        .secret
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_some()
            })
        })
        .and_then(|key| key.secret.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    if let Some(api_key) = primary_secret {
        account.auth_json = profile_files::build_api_auth_json(&api_key);
        account.api_key = Some(api_key);
    }
}

fn normalize_relay_key_pool(
    account: &StoredAccount,
    inputs: Vec<UpdateApiAccountKeyInput>,
    updated_at: i64,
) -> Result<Vec<ProxyKey>, String> {
    let existing_by_id = account
        .api_keys
        .iter()
        .map(|key| (key.id.as_str(), key))
        .collect::<HashMap<_, _>>();
    let mut seen_ids = HashSet::new();
    let mut output = Vec::with_capacity(inputs.len());

    for (index, input) in inputs.into_iter().enumerate() {
        let id = input
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("key:{}:{}", account.id, uuid::Uuid::new_v4()));

        if !seen_ids.insert(id.clone()) {
            return Err("API Key ID 重复，请检查 Key 池。".to_string());
        }

        let existing = existing_by_id.get(id.as_str()).copied();
        let new_secret = input
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let secret = match new_secret {
            Some(value) => profile_files::normalize_relay_api_key(value)?,
            None => existing
                .and_then(|key| key.secret.clone())
                .or_else(|| {
                    if index == 0 {
                        account.api_key.clone()
                    } else {
                        None
                    }
                })
                .ok_or_else(|| "新增 API Key 必须填写 Key 内容。".to_string())?,
        };
        let existing_health = existing.map(|key| key.health_status);
        let health_status = if !input.enabled {
            ProxyHealthStatus::Disabled
        } else if new_secret.is_some()
            || matches!(existing_health, Some(ProxyHealthStatus::Disabled))
        {
            ProxyHealthStatus::Healthy
        } else {
            existing_health.unwrap_or(ProxyHealthStatus::Healthy)
        };

        output.push(ProxyKey {
            id,
            label: normalize_custom_label(input.label)
                .or_else(|| existing.and_then(|key| key.label.clone())),
            secret: Some(secret),
            enabled: input.enabled,
            priority: input.priority,
            weight: if input.weight == 0 { 100 } else { input.weight },
            health_status,
            last_error: existing.and_then(|key| key.last_error.clone()),
            cooldown_until: existing.and_then(|key| key.cooldown_until),
            failure_count: existing.map(|key| key.failure_count).unwrap_or(0),
            last_used_at: existing.and_then(|key| key.last_used_at),
            updated_at: Some(updated_at),
        });
    }

    if !output.iter().any(|key| key.enabled) {
        return Err("至少需要启用一个 API Key。".to_string());
    }

    Ok(output)
}

async fn probe_relay_api_key(base_url: &str, api_key: &str) -> Result<(), RelayKeyProbeFailure> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(RELAY_KEY_PROBE_TIMEOUT_SECS))
        .build()
        .map_err(|error| RelayKeyProbeFailure {
            status: None,
            message: format!("创建 Key 探测客户端失败: {error}"),
        })?;
    let endpoint = format!("{}/models", base_url.trim_end_matches('/'));
    let response = client
        .get(&endpoint)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|error| RelayKeyProbeFailure {
            status: None,
            message: format!("Key 探测请求失败: {error}"),
        })?;
    let status = response.status();

    if status.is_success() {
        return Ok(());
    }

    let body = response.text().await.unwrap_or_default();
    Err(RelayKeyProbeFailure {
        status: Some(status),
        message: probe_failure_message(status, &body),
    })
}

fn probe_failure_message(status: StatusCode, body: &str) -> String {
    let detail = truncate_probe_message(&redact_sensitive_text(body));
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            "Key 探测失败：鉴权被拒绝。".to_string()
        }
        StatusCode::TOO_MANY_REQUESTS => "Key 探测失败：上游限速，已进入冷却。".to_string(),
        StatusCode::NOT_FOUND => {
            "Key 探测失败：Base URL 不支持 /models，请确认填写到 /v1。".to_string()
        }
        _ if detail.is_empty() => format!("Key 探测失败：上游返回 {status}。"),
        _ => format!("Key 探测失败：上游返回 {status}: {detail}"),
    }
}

fn truncate_probe_message(message: &str) -> String {
    let collapsed = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let truncated = chars.by_ref().take(180).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn probe_failure_health_status(status: Option<StatusCode>) -> ProxyHealthStatus {
    match status {
        Some(StatusCode::UNAUTHORIZED) | Some(StatusCode::FORBIDDEN) => {
            ProxyHealthStatus::AuthFailed
        }
        Some(StatusCode::TOO_MANY_REQUESTS) => ProxyHealthStatus::CoolingDown,
        Some(status) if status.is_server_error() => ProxyHealthStatus::Degraded,
        _ => ProxyHealthStatus::Degraded,
    }
}

fn probe_failure_cooldown_until(status: Option<StatusCode>, now: i64) -> Option<i64> {
    match status {
        Some(StatusCode::TOO_MANY_REQUESTS) => Some(now + 5 * 60),
        Some(status) if status.is_server_error() => Some(now + 60),
        _ => None,
    }
}

async fn prepare_auth_json_import(
    auth_json: serde_json::Value,
    label: Option<String>,
) -> Result<PreparedChatgptImport, String> {
    let extracted = extract_auth(&auth_json)?;

    // 用量拉取失败不阻断导入流程，避免账号无法入库。
    let usage = fetch_usage_snapshot(&extracted.access_token, &extracted.account_id)
        .await
        .ok();

    Ok(PreparedChatgptImport {
        principal_id: extracted.principal_id,
        auth_json,
        account_id: extracted.account_id,
        email: extracted.email,
        plan_type: extracted.plan_type,
        usage,
        label,
    })
}

async fn prepare_import_candidate(candidate: ImportCandidate) -> Result<PreparedImport, String> {
    match candidate {
        ImportCandidate::Chatgpt(candidate) => {
            let extracted = extract_auth(&candidate.auth_json)?;

            // 用量拉取失败不阻断导入流程；若来自账号库备份，则保留备份内已有快照。
            let usage = fetch_usage_snapshot(&extracted.access_token, &extracted.account_id)
                .await
                .ok()
                .or(candidate.usage);

            Ok(PreparedImport::Chatgpt(PreparedChatgptImport {
                principal_id: extracted.principal_id,
                auth_json: candidate.auth_json,
                account_id: extracted.account_id,
                email: extracted.email.or(candidate.email),
                plan_type: extracted.plan_type.or(candidate.plan_type),
                usage,
                label: candidate.label,
            }))
        }
        ImportCandidate::Relay(candidate) => {
            let label = profile_files::normalize_relay_label(&candidate.label)?;
            let base_url = profile_files::normalize_relay_base_url(&candidate.base_url)?;
            let api_key = profile_files::normalize_relay_api_key(&candidate.api_key)?;
            let model_name = profile_files::normalize_relay_model_name(
                candidate.model_name.as_deref().unwrap_or("gpt-5"),
            )?;

            Ok(PreparedImport::Relay(PreparedRelayImport {
                label,
                base_url,
                api_key,
                model_name,
            }))
        }
    }
}

async fn commit_prepared_import(
    app: &AppHandle,
    state: &AppState,
    prepared: PreparedChatgptImport,
) -> Result<AccountSummary, String> {
    let current_account_key = current_auth_account_key();
    let current_variant_key = current_auth_variant_key();
    let summary = {
        let mut _guard = state.store_lock.lock().await;
        let mut store = load_store(app)?;
        let (summary, _) = upsert_prepared_import(
            &mut store,
            prepared,
            current_account_key.as_deref(),
            current_variant_key.as_deref(),
        );
        let store_path = account_store_path_for_app(app)?;
        if let Some(account) = store
            .accounts
            .iter_mut()
            .find(|account| account.id == summary.id)
        {
            profile_files::sync_account_profile_in_store_path(&store_path, account)?;
        }
        save_store(app, &store)?;
        store
            .accounts
            .iter()
            .find(|account| account.id == summary.id)
            .map(|account| {
                account.to_summary(
                    current_account_key.as_deref(),
                    current_variant_key.as_deref(),
                )
            })
            .unwrap_or(summary)
    };

    Ok(summary)
}

fn export_accounts_zip_sync(
    default_file_name: &str,
    export_payload: &[u8],
) -> Result<Option<String>, String> {
    let Some(selected_path) = FileDialog::new()
        .set_title("导出账号列表")
        .add_filter("ZIP archive", &["zip"])
        .set_file_name(default_file_name)
        .save_file()
    else {
        return Ok(None);
    };

    let export_path = ensure_zip_extension(selected_path);
    write_accounts_zip_archive(&export_path, export_payload)?;
    Ok(Some(export_path.to_string_lossy().to_string()))
}

fn export_accounts_json_sync(
    title: &str,
    default_file_name: &str,
    export_payload: &[u8],
) -> Result<Option<String>, String> {
    let Some(selected_path) = FileDialog::new()
        .set_title(title)
        .add_filter("JSON", &["json"])
        .set_file_name(default_file_name)
        .save_file()
    else {
        return Ok(None);
    };

    let export_path = ensure_json_extension(selected_path);
    write_export_file_atomically(&export_path, export_payload)?;
    Ok(Some(export_path.to_string_lossy().to_string()))
}

fn write_accounts_zip_archive(path: &Path, export_payload: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析导出目录 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建导出目录失败 {}: {error}", parent.display()))?;

    let temp_path = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("accounts.zip"),
        uuid::Uuid::new_v4()
    ));

    let write_result = (|| -> Result<(), String> {
        let archive_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|error| format!("创建导出临时文件失败 {}: {error}", temp_path.display()))?;
        let mut archive = zip::ZipWriter::new(archive_file);
        let options = FileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o600);
        archive
            .start_file(EXPORT_ARCHIVE_ENTRY_NAME, options)
            .map_err(|error| format!("创建压缩包内容失败: {error}"))?;
        archive
            .write_all(export_payload)
            .map_err(|error| format!("写入压缩包失败: {error}"))?;
        let archive_file = archive
            .finish()
            .map_err(|error| format!("完成压缩包写入失败: {error}"))?;
        archive_file
            .sync_all()
            .map_err(|error| format!("刷新导出文件失败 {}: {error}", temp_path.display()))?;
        drop(archive_file);
        set_private_permissions(&temp_path);

        #[cfg(target_family = "unix")]
        {
            fs::rename(&temp_path, path).map_err(|error| {
                format!(
                    "写入导出文件失败 {} -> {}: {error}",
                    temp_path.display(),
                    path.display()
                )
            })?;

            let parent_dir = fs::File::open(parent)
                .map_err(|error| format!("打开导出目录失败 {}: {error}", parent.display()))?;
            parent_dir
                .sync_all()
                .map_err(|error| format!("刷新导出目录失败 {}: {error}", parent.display()))?;
        }

        #[cfg(not(target_family = "unix"))]
        {
            if path.exists() {
                fs::remove_file(path)
                    .map_err(|error| format!("删除旧导出文件失败 {}: {error}", path.display()))?;
            }
            fs::rename(&temp_path, path).map_err(|error| {
                format!(
                    "写入导出文件失败 {} -> {}: {error}",
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

fn write_export_file_atomically(path: &Path, export_payload: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析导出目录 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建导出目录失败 {}: {error}", parent.display()))?;

    let temp_path = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("accounts.json"),
        uuid::Uuid::new_v4()
    ));

    let write_result = (|| -> Result<(), String> {
        let mut export_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|error| format!("创建导出临时文件失败 {}: {error}", temp_path.display()))?;
        export_file
            .write_all(export_payload)
            .map_err(|error| format!("写入导出文件失败 {}: {error}", temp_path.display()))?;
        export_file
            .sync_all()
            .map_err(|error| format!("刷新导出文件失败 {}: {error}", temp_path.display()))?;
        drop(export_file);
        set_private_permissions(&temp_path);

        #[cfg(target_family = "unix")]
        {
            fs::rename(&temp_path, path).map_err(|error| {
                format!(
                    "写入导出文件失败 {} -> {}: {error}",
                    temp_path.display(),
                    path.display()
                )
            })?;

            let parent_dir = fs::File::open(parent)
                .map_err(|error| format!("打开导出目录失败 {}: {error}", parent.display()))?;
            parent_dir
                .sync_all()
                .map_err(|error| format!("刷新导出目录失败 {}: {error}", parent.display()))?;
        }

        #[cfg(not(target_family = "unix"))]
        {
            if path.exists() {
                fs::remove_file(path)
                    .map_err(|error| format!("删除旧导出文件失败 {}: {error}", path.display()))?;
            }
            fs::rename(&temp_path, path).map_err(|error| {
                format!(
                    "写入导出文件失败 {} -> {}: {error}",
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

fn ensure_zip_extension(path: PathBuf) -> PathBuf {
    let has_zip_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("zip"))
        .unwrap_or(false);

    if has_zip_extension {
        path
    } else {
        path.with_extension("zip")
    }
}

fn ensure_json_extension(path: PathBuf) -> PathBuf {
    let has_json_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    if has_json_extension {
        path
    } else {
        path.with_extension("json")
    }
}

fn upsert_prepared_import(
    store: &mut AccountsStore,
    prepared: PreparedChatgptImport,
    current_account_key: Option<&str>,
    current_variant_key: Option<&str>,
) -> (AccountSummary, bool) {
    let PreparedChatgptImport {
        principal_id,
        auth_json,
        account_id,
        email,
        plan_type,
        usage,
        label,
    } = prepared;

    let now = now_unix_seconds();
    let resolved_label = normalize_custom_label(label)
        .unwrap_or_else(|| fallback_account_label(email.as_deref(), &account_id));
    let resolved_plan_type = plan_type.or_else(|| {
        usage
            .as_ref()
            .and_then(|snapshot| snapshot.plan_type.clone())
    });
    let resolved_account_key = account_group_key(&principal_id, &account_id);
    let resolved_plan_key = normalize_plan_type_key(resolved_plan_type.as_deref());
    let resolved_variant_key =
        account_variant_key(&principal_id, &account_id, resolved_plan_type.as_deref());

    if let Some(existing) = store
        .accounts
        .iter_mut()
        .find(|account| account.variant_key() == resolved_variant_key)
    {
        apply_prepared_import_to_account(
            existing,
            principal_id.clone(),
            resolved_label,
            email,
            resolved_plan_type.clone(),
            auth_json,
            usage,
            now,
        );
        (
            existing.to_summary(current_account_key, current_variant_key),
            true,
        )
    } else if resolved_plan_key != "unknown" {
        if let Some(existing) = store.accounts.iter_mut().find(|account| {
            account.account_key() == resolved_account_key
                && normalize_plan_type_key(account.resolved_plan_type().as_deref()) == "unknown"
        }) {
            apply_prepared_import_to_account(
                existing,
                principal_id.clone(),
                resolved_label,
                email,
                resolved_plan_type.clone(),
                auth_json,
                usage,
                now,
            );
            return (
                existing.to_summary(current_account_key, current_variant_key),
                true,
            );
        }

        let auth_refresh_next_at = auth_refresh_next_at(&auth_json);
        let stored = StoredAccount {
            id: uuid::Uuid::new_v4().to_string(),
            label: resolved_label,
            source_kind: AccountSourceKind::Chatgpt,
            principal_id: Some(principal_id.clone()),
            email,
            account_id,
            plan_type: resolved_plan_type,
            auth_json,
            api_base_url: None,
            api_key: None,
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: None,
            model_catalog: Vec::new(),
            model_routing_enabled: false,
            balance_text: None,
            balance_display_enabled: false,
            api_quota_mode: Default::default(),
            api_quota_today_used_text: None,
            api_quota_remaining_text: None,
            api_quota_total_remaining_text: None,
            api_quota_total_tokens_text: None,
            api_quota_today_tokens_text: None,
            api_quota_daily_window: None,
            api_quota_total_window: None,
            api_quota_subscription_expires_at: None,
            api_quota_subscription_name: None,
            provider_id: None,
            provider_name: None,
            tags: Vec::new(),
            profile_auth_path: None,
            profile_config_path: None,
            profile_auth_ready: false,
            profile_config_ready: false,
            profile_integrity_error: None,
            profile_last_validated_at: None,
            profile_last_validation_error: None,
            added_at: now,
            updated_at: now,
            usage,
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at,
        };
        let summary = stored.to_summary(current_account_key, current_variant_key);
        store.accounts.push(stored);
        (summary, false)
    } else {
        let auth_refresh_next_at = auth_refresh_next_at(&auth_json);
        let stored = StoredAccount {
            id: uuid::Uuid::new_v4().to_string(),
            label: resolved_label,
            source_kind: AccountSourceKind::Chatgpt,
            principal_id: Some(principal_id),
            email,
            account_id,
            plan_type: resolved_plan_type,
            auth_json,
            api_base_url: None,
            api_key: None,
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: None,
            model_catalog: Vec::new(),
            model_routing_enabled: false,
            balance_text: None,
            balance_display_enabled: false,
            api_quota_mode: Default::default(),
            api_quota_today_used_text: None,
            api_quota_remaining_text: None,
            api_quota_total_remaining_text: None,
            api_quota_total_tokens_text: None,
            api_quota_today_tokens_text: None,
            api_quota_daily_window: None,
            api_quota_total_window: None,
            api_quota_subscription_expires_at: None,
            api_quota_subscription_name: None,
            provider_id: None,
            provider_name: None,
            tags: Vec::new(),
            profile_auth_path: None,
            profile_config_path: None,
            profile_auth_ready: false,
            profile_config_ready: false,
            profile_integrity_error: None,
            profile_last_validated_at: None,
            profile_last_validation_error: None,
            added_at: now,
            updated_at: now,
            usage,
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at,
        };
        let summary = stored.to_summary(current_account_key, current_variant_key);
        store.accounts.push(stored);
        (summary, false)
    }
}

fn upsert_prepared_relay_import(
    store: &mut AccountsStore,
    prepared: PreparedRelayImport,
    current_account_key: Option<&str>,
    current_variant_key: Option<&str>,
) -> (AccountSummary, bool) {
    let PreparedRelayImport {
        label,
        base_url,
        api_key,
        model_name,
    } = prepared;

    let now = now_unix_seconds();
    let (provider_id, provider_name) =
        infer_provider_metadata_from_base_url(Some(base_url.as_str()));

    if let Some(existing) = store.accounts.iter_mut().find(|account| {
        matches!(account.source_kind, AccountSourceKind::Relay)
            && account.api_base_url.as_deref() == Some(base_url.as_str())
            && account.primary_relay_api_key() == Some(api_key.as_str())
    }) {
        existing.label = label.clone();
        existing.auth_json = profile_files::build_api_auth_json(&api_key);
        existing.api_base_url = Some(base_url);
        existing.api_key = Some(api_key.clone());
        existing.model_name = Some(model_name);
        existing.provider_id = provider_id;
        existing.provider_name = provider_name;
        existing.profile_last_validated_at = None;
        existing.profile_last_validation_error = Some(
            "已从 Sub2API 导入并跳过接口探测，仅启用 /v1/chat/completions；如需 Responses/Compact，请在 Key 池中手动开启。"
                .to_string(),
        );
        existing.proxy_endpoints = vec![ProxyEndpointCapability::ChatCompletions];
        existing.updated_at = now;
        replace_primary_relay_proxy_key(existing, &label, &api_key, now);
        sync_primary_api_key_from_relay_key_pool(existing);
        return (
            existing.to_summary(current_account_key, current_variant_key),
            true,
        );
    }

    let id = uuid::Uuid::new_v4().to_string();
    let stored = StoredAccount {
        id: id.clone(),
        label: label.clone(),
        source_kind: AccountSourceKind::Relay,
        principal_id: None,
        email: None,
        account_id: profile_files::relay_account_id(&id),
        plan_type: None,
        auth_json: profile_files::build_api_auth_json(&api_key),
        api_base_url: Some(base_url),
        api_key: Some(api_key.clone()),
        api_keys: vec![relay_proxy_key(&id, &label, &api_key, now)],
        proxy_priority: None,
        proxy_weight: None,
        proxy_key_selection_mode: None,
        proxy_endpoints: vec![ProxyEndpointCapability::ChatCompletions],
        model_name: Some(model_name),
        model_catalog: Vec::new(),
        model_routing_enabled: false,
        balance_text: None,
        balance_display_enabled: false,
        api_quota_mode: Default::default(),
        api_quota_today_used_text: None,
        api_quota_remaining_text: None,
        api_quota_total_remaining_text: None,
        api_quota_total_tokens_text: None,
        api_quota_today_tokens_text: None,
        api_quota_daily_window: None,
        api_quota_total_window: None,
        api_quota_subscription_expires_at: None,
        api_quota_subscription_name: None,
        provider_id,
        provider_name,
        tags: Vec::new(),
        profile_auth_path: None,
        profile_config_path: None,
        profile_auth_ready: false,
        profile_config_ready: false,
        profile_integrity_error: None,
        profile_last_validated_at: None,
        profile_last_validation_error: Some(
            "已从 Sub2API 导入并跳过接口探测，仅启用 /v1/chat/completions；如需 Responses/Compact，请在 Key 池中手动开启。"
                .to_string(),
        ),
        added_at: now,
        updated_at: now,
        usage: None,
        usage_error: None,
        auth_refresh_blocked: false,
        auth_refresh_error: None,
        auth_refresh_next_at: None,
    };
    let summary = stored.to_summary(current_account_key, current_variant_key);
    store.accounts.push(stored);
    (summary, false)
}

fn apply_prepared_import_to_account(
    existing: &mut StoredAccount,
    principal_id: String,
    label: String,
    email: Option<String>,
    plan_type: Option<String>,
    auth_json: serde_json::Value,
    usage: Option<UsageSnapshot>,
    now: i64,
) {
    existing.label = label;
    existing.source_kind = AccountSourceKind::Chatgpt;
    existing.principal_id = Some(principal_id);
    existing.email = email;
    existing.plan_type = plan_type.or(existing.plan_type.clone());
    existing.auth_json = auth_json;
    existing.api_base_url = None;
    existing.api_key = None;
    existing.model_name = None;
    existing.balance_text = None;
    existing.provider_id = None;
    existing.provider_name = None;
    existing.updated_at = now;
    existing.usage = usage;
    existing.usage_error = None;
    existing.auth_refresh_blocked = false;
    existing.auth_refresh_error = None;
    existing.auth_refresh_next_at = auth_refresh_next_at(&existing.auth_json);
}

fn apply_reauthorized_account(existing: &mut StoredAccount, prepared: PreparedChatgptImport) {
    let PreparedChatgptImport {
        principal_id,
        auth_json,
        account_id,
        email,
        plan_type,
        usage,
        ..
    } = prepared;

    let now = now_unix_seconds();
    let resolved_plan_type = plan_type.or_else(|| {
        usage
            .as_ref()
            .and_then(|snapshot| snapshot.plan_type.clone())
    });

    existing.principal_id = Some(principal_id);
    existing.source_kind = AccountSourceKind::Chatgpt;
    existing.email = email.or_else(|| existing.email.clone());
    existing.account_id = account_id;
    existing.plan_type = resolved_plan_type;
    existing.auth_json = auth_json;
    existing.api_base_url = None;
    existing.api_key = None;
    existing.model_name = None;
    existing.balance_text = None;
    existing.provider_id = None;
    existing.provider_name = None;
    existing.updated_at = now;
    existing.usage = usage;
    existing.usage_error = None;
    existing.auth_refresh_blocked = false;
    existing.auth_refresh_error = None;
    existing.auth_refresh_next_at = auth_refresh_next_at(&existing.auth_json);
}

fn validate_reauthorization_target(
    existing: &StoredAccount,
    prepared: &PreparedChatgptImport,
) -> Result<(), String> {
    if let (Some(existing_email), Some(new_email)) =
        (existing.email.as_deref(), prepared.email.as_deref())
    {
        if existing_email.trim().eq_ignore_ascii_case(new_email.trim()) {
            return Ok(());
        }
    }

    if existing.principal_id.as_deref().is_some_and(|value| {
        value
            .trim()
            .eq_ignore_ascii_case(prepared.principal_id.trim())
    }) || existing.account_id == prepared.account_id
    {
        return Ok(());
    }

    let target_label = existing.email.as_deref().unwrap_or(existing.label.as_str());
    let new_label = prepared
        .email
        .as_deref()
        .unwrap_or_else(|| prepared.account_id.as_str());
    Err(format!(
        "重新授权得到的账号与目标账号不一致。目标账号: {target_label}；新账号: {new_label}。请确认浏览器登录的是同一个账号。"
    ))
}

fn expand_import_json_content(
    raw: &str,
    source: &str,
    label_override: Option<&str>,
) -> Result<Vec<ImportCandidate>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("JSON 内容为空".to_string());
    }

    let normalized = trimmed.strip_prefix('\u{feff}').unwrap_or(trimmed);
    let parsed =
        serde_json::from_str(normalized).map_err(|error| format!("JSON 格式无效: {error}"))?;
    expand_import_value(parsed, source, label_override)
}

fn expand_import_value(
    parsed: serde_json::Value,
    source: &str,
    label_override: Option<&str>,
) -> Result<Vec<ImportCandidate>, String> {
    if looks_like_sub2api_data_payload(&parsed) {
        return expand_sub2api_data_import(parsed, source, label_override);
    }

    if looks_like_accounts_store(&parsed) {
        return expand_accounts_store_import(parsed, source, label_override);
    }

    if looks_like_stored_account(&parsed) {
        return import_stored_account_candidates(parsed, source, label_override);
    }

    if let Some(items) = parsed.as_array() {
        if items.is_empty() {
            return Err("JSON 内没有可导入的账号".to_string());
        }

        let mut expanded = Vec::with_capacity(items.len());
        for (index, item) in items.iter().cloned().enumerate() {
            let item_source = format!("{source} / #{}", index + 1);
            let candidates = if looks_like_stored_account(&item) {
                import_stored_account_candidates(item, &item_source, label_override)?
            } else {
                vec![ImportCandidate::Chatgpt(ChatgptImportCandidate {
                    source: item_source,
                    auth_json: normalize_imported_auth_json(item),
                    label: normalize_custom_label(label_override.map(ToString::to_string)),
                    usage: None,
                    plan_type: None,
                    email: None,
                })]
            };
            expanded.extend(candidates);
        }
        return Ok(expanded);
    }

    Ok(vec![ImportCandidate::Chatgpt(ChatgptImportCandidate {
        source: source.to_string(),
        auth_json: normalize_imported_auth_json(parsed),
        label: normalize_custom_label(label_override.map(ToString::to_string)),
        usage: None,
        plan_type: None,
        email: None,
    })])
}

fn expand_accounts_store_import(
    parsed: serde_json::Value,
    source: &str,
    label_override: Option<&str>,
) -> Result<Vec<ImportCandidate>, String> {
    let Some(root) = parsed.as_object() else {
        return Err("账号库备份格式无效（根节点不是对象）".to_string());
    };
    let Some(accounts) = root.get("accounts").and_then(serde_json::Value::as_array) else {
        return Err("账号库备份缺少 accounts 数组".to_string());
    };
    if accounts.is_empty() {
        return Err("账号库备份里没有可导入的账号".to_string());
    }

    let mut expanded = Vec::with_capacity(accounts.len());
    for (index, account) in accounts.iter().cloned().enumerate() {
        let item_source = format!("{source} / #{}", index + 1);
        let candidates = import_stored_account_candidates(account, &item_source, label_override)?;
        expanded.extend(candidates);
    }
    Ok(expanded)
}

fn import_stored_account_candidates(
    parsed: serde_json::Value,
    source: &str,
    label_override: Option<&str>,
) -> Result<Vec<ImportCandidate>, String> {
    let Some(root) = parsed.as_object() else {
        return Err("账号备份格式无效（不是对象）".to_string());
    };

    let auth_json = root
        .get("authJson")
        .or_else(|| root.get("auth_json"))
        .cloned()
        .ok_or_else(|| "账号备份缺少 authJson".to_string())?;

    let stored_label = root
        .get("label")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let email = root
        .get("email")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let plan_type = root
        .get("planType")
        .or_else(|| root.get("plan_type"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let usage = root
        .get("usage")
        .cloned()
        .and_then(|value| serde_json::from_value::<UsageSnapshot>(value).ok());
    let account_id = root
        .get("accountId")
        .or_else(|| root.get("account_id"))
        .and_then(serde_json::Value::as_str);

    Ok(vec![ImportCandidate::Chatgpt(ChatgptImportCandidate {
        source: describe_account_backup_source(
            source,
            normalize_custom_label(stored_label.clone())
                .as_deref()
                .or(email.as_deref())
                .or(account_id),
        ),
        auth_json: normalize_imported_auth_json(auth_json),
        label: normalize_custom_label(label_override.map(ToString::to_string))
            .or_else(|| normalize_custom_label(stored_label)),
        usage,
        plan_type,
        email,
    })])
}

fn looks_like_sub2api_data_payload(value: &serde_json::Value) -> bool {
    let Some(root) = value.as_object() else {
        return false;
    };

    if root
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(SUB2API_EXPORT_TYPE))
    {
        return true;
    }

    root.get("accounts")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|accounts| accounts.iter().any(looks_like_sub2api_account))
}

fn looks_like_sub2api_account(value: &serde_json::Value) -> bool {
    let Some(root) = value.as_object() else {
        return false;
    };
    root.get("credentials")
        .and_then(serde_json::Value::as_object)
        .is_some()
        && root
            .get("platform")
            .and_then(serde_json::Value::as_str)
            .is_some()
        && root
            .get("type")
            .and_then(serde_json::Value::as_str)
            .is_some()
}

fn expand_sub2api_data_import(
    parsed: serde_json::Value,
    source: &str,
    label_override: Option<&str>,
) -> Result<Vec<ImportCandidate>, String> {
    let Some(root) = parsed.as_object() else {
        return Err("Sub2API 数据格式无效（根节点不是对象）".to_string());
    };
    let Some(accounts) = root.get("accounts").and_then(serde_json::Value::as_array) else {
        return Err("Sub2API 数据缺少 accounts 数组".to_string());
    };
    if accounts.is_empty() {
        return Err("Sub2API 数据里没有可导入的账号".to_string());
    }

    let mut expanded = Vec::with_capacity(accounts.len());
    for (index, account) in accounts.iter().enumerate() {
        let item_source = format!("{source} / #{}", index + 1);
        expanded.push(import_sub2api_account_candidate(
            account,
            &item_source,
            label_override,
        )?);
    }
    Ok(expanded)
}

fn import_sub2api_account_candidate(
    account: &serde_json::Value,
    source: &str,
    label_override: Option<&str>,
) -> Result<ImportCandidate, String> {
    let Some(root) = account.as_object() else {
        return Err("Sub2API 账号格式无效（不是对象）".to_string());
    };
    let platform = root
        .get("platform")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !platform.eq_ignore_ascii_case("openai") {
        return Err(format!("暂不支持导入 Sub2API 平台 {platform}"));
    }

    let account_type = root
        .get("type")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let credentials = root
        .get("credentials")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "Sub2API 账号缺少 credentials".to_string())?;
    let stored_label = root
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let label = normalize_custom_label(label_override.map(ToString::to_string))
        .or_else(|| normalize_custom_label(stored_label.clone()));
    let described_source = describe_account_backup_source(
        source,
        label.as_deref().or_else(|| {
            credentials
                .get("email")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        }),
    );

    if account_type.eq_ignore_ascii_case("oauth") {
        return import_sub2api_oauth_candidate(credentials, described_source, label);
    }

    if account_type.eq_ignore_ascii_case("apikey") || account_type.eq_ignore_ascii_case("api_key") {
        return import_sub2api_apikey_candidate(credentials, described_source, label);
    }

    Err(format!("暂不支持导入 Sub2API 账号类型 {account_type}"))
}

fn import_sub2api_oauth_candidate(
    credentials: &serde_json::Map<String, serde_json::Value>,
    source: String,
    label: Option<String>,
) -> Result<ImportCandidate, String> {
    let access_token = required_sub2api_string(credentials, "access_token")?;
    let id_token = required_sub2api_string(credentials, "id_token")?;
    let refresh_token = optional_sub2api_string(credentials, "refresh_token");
    let account_id = optional_sub2api_string(credentials, "chatgpt_account_id")
        .or_else(|| optional_sub2api_string(credentials, "account_id"));

    let mut tokens = serde_json::Map::new();
    tokens.insert(
        "access_token".to_string(),
        serde_json::Value::String(access_token),
    );
    tokens.insert("id_token".to_string(), serde_json::Value::String(id_token));
    if let Some(refresh_token) = refresh_token {
        tokens.insert(
            "refresh_token".to_string(),
            serde_json::Value::String(refresh_token),
        );
    }
    if let Some(account_id) = account_id {
        tokens.insert(
            "account_id".to_string(),
            serde_json::Value::String(account_id),
        );
    }

    let mut auth_json = serde_json::Map::new();
    auth_json.insert(
        "auth_mode".to_string(),
        serde_json::Value::String("chatgpt".to_string()),
    );
    auth_json.insert("tokens".to_string(), serde_json::Value::Object(tokens));
    if let Some(last_refresh) = credentials.get("last_refresh").cloned() {
        auth_json.insert("last_refresh".to_string(), last_refresh);
    }

    Ok(ImportCandidate::Chatgpt(ChatgptImportCandidate {
        source,
        auth_json: normalize_imported_auth_json(serde_json::Value::Object(auth_json)),
        label,
        usage: None,
        plan_type: optional_sub2api_string(credentials, "plan_type"),
        email: optional_sub2api_string(credentials, "email"),
    }))
}

fn import_sub2api_apikey_candidate(
    credentials: &serde_json::Map<String, serde_json::Value>,
    source: String,
    label: Option<String>,
) -> Result<ImportCandidate, String> {
    let api_key = required_sub2api_string(credentials, "api_key")?;
    let base_url = required_sub2api_string(credentials, "base_url")?;
    let model_name = optional_sub2api_string(credentials, "default_model")
        .or_else(|| optional_sub2api_string(credentials, "model"));

    Ok(ImportCandidate::Relay(RelayImportCandidate {
        source,
        label: label.unwrap_or_else(|| "Sub2API API".to_string()),
        base_url,
        api_key,
        model_name,
    }))
}

fn required_sub2api_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, String> {
    optional_sub2api_string(object, key).ok_or_else(|| format!("Sub2API 账号缺少 {key}"))
}

fn optional_sub2api_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn looks_like_accounts_store(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .and_then(|root| root.get("accounts"))
        .and_then(serde_json::Value::as_array)
        .is_some()
}

fn looks_like_stored_account(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .map(|root| root.contains_key("authJson") || root.contains_key("auth_json"))
        .unwrap_or(false)
}

fn describe_account_backup_source(source: &str, hint: Option<&str>) -> String {
    let Some(hint) = hint.map(str::trim).filter(|value| !value.is_empty()) else {
        return source.to_string();
    };
    format!("{source} / {hint}")
}

fn normalize_custom_label(label: Option<String>) -> Option<String> {
    label.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_optional_quota_display_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.chars().take(40).collect())
        }
    })
}

async fn resolve_create_api_quota_fields(
    input: &crate::models::CreateApiAccountInput,
    label: &str,
    base_url: &str,
    api_key: &str,
) -> ResolvedApiQuotaFields {
    let fallback = ResolvedApiQuotaFields {
        mode: input.api_quota_mode,
        today_used_text: normalize_optional_quota_display_text(
            input.api_quota_today_used_text.clone(),
        ),
        remaining_text: normalize_optional_quota_display_text(
            input.api_quota_remaining_text.clone(),
        ),
        total_remaining_text: normalize_optional_quota_display_text(
            input.api_quota_total_remaining_text.clone(),
        ),
        total_tokens_text: normalize_optional_quota_display_text(
            input.api_quota_total_tokens_text.clone(),
        ),
        today_tokens_text: normalize_optional_quota_display_text(
            input.api_quota_today_tokens_text.clone(),
        ),
        daily_window: input.api_quota_daily_window.clone(),
        total_window: input.api_quota_total_window.clone(),
        subscription_expires_at: input.api_quota_subscription_expires_at,
        subscription_name: normalize_optional_quota_display_text(
            input.api_quota_subscription_name.clone(),
        ),
    };

    resolve_platform_api_quota_fields(
        fallback,
        label,
        base_url,
        Some(api_key),
        input.platform_login_email.as_deref(),
        input.platform_login_password.as_deref(),
        input.balance_display_enabled,
    )
    .await
}

async fn resolve_update_api_quota_fields(
    input: &crate::models::UpdateApiAccountInput,
    label: &str,
    base_url: &str,
    api_key: Option<&str>,
) -> ResolvedApiQuotaFields {
    let fallback = ResolvedApiQuotaFields {
        mode: input.api_quota_mode,
        today_used_text: normalize_optional_quota_display_text(
            input.api_quota_today_used_text.clone(),
        ),
        remaining_text: normalize_optional_quota_display_text(
            input.api_quota_remaining_text.clone(),
        ),
        total_remaining_text: normalize_optional_quota_display_text(
            input.api_quota_total_remaining_text.clone(),
        ),
        total_tokens_text: normalize_optional_quota_display_text(
            input.api_quota_total_tokens_text.clone(),
        ),
        today_tokens_text: normalize_optional_quota_display_text(
            input.api_quota_today_tokens_text.clone(),
        ),
        daily_window: input.api_quota_daily_window.clone(),
        total_window: input.api_quota_total_window.clone(),
        subscription_expires_at: input.api_quota_subscription_expires_at,
        subscription_name: normalize_optional_quota_display_text(
            input.api_quota_subscription_name.clone(),
        ),
    };

    resolve_platform_api_quota_fields(
        fallback,
        label,
        base_url,
        api_key,
        input.platform_login_email.as_deref(),
        input.platform_login_password.as_deref(),
        input.balance_display_enabled.unwrap_or(true),
    )
    .await
}

async fn resolve_platform_api_quota_fields(
    fallback: ResolvedApiQuotaFields,
    label: &str,
    base_url: &str,
    api_key: Option<&str>,
    email: Option<&str>,
    password: Option<&str>,
    balance_display_enabled: bool,
) -> ResolvedApiQuotaFields {
    if !balance_display_enabled {
        return ResolvedApiQuotaFields::empty();
    }

    let email = email.map(str::trim).filter(|value| !value.is_empty());
    let password = password.map(str::trim).filter(|value| !value.is_empty());

    if let Some(api_key) = api_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|_| email.is_none() || password.is_none())
    {
        if let Ok(snapshot) = fetch_api_key_quota_snapshot(base_url, api_key).await {
            return ResolvedApiQuotaFields {
                mode: snapshot.mode,
                today_used_text: snapshot.today_used_text.or(fallback.today_used_text),
                remaining_text: snapshot.remaining_text.or(fallback.remaining_text),
                total_remaining_text: snapshot
                    .total_remaining_text
                    .or(fallback.total_remaining_text),
                total_tokens_text: snapshot.total_tokens_text.or(fallback.total_tokens_text),
                today_tokens_text: snapshot.today_tokens_text.or(fallback.today_tokens_text),
                daily_window: snapshot.daily_window.or(fallback.daily_window),
                total_window: snapshot.total_window.or(fallback.total_window),
                subscription_expires_at: snapshot
                    .subscription_expires_at
                    .or(fallback.subscription_expires_at),
                subscription_name: fallback.subscription_name.or(snapshot.subscription_name),
            };
        }
    }

    let Some(email) = email else {
        return fallback;
    };
    let Some(password) = password else {
        return fallback;
    };

    let provider = crate::models::NotificationProviderConfig {
        id: "api-quota-probe".to_string(),
        name: label.to_string(),
        account_key: None,
        kind: Default::default(),
        enabled: true,
        cost_multiplier: crate::models::default_notification_cost_multiplier(),
        base_url: base_url.to_string(),
        email: email.to_string(),
        password: Some(password.to_string()),
        created_at: now_unix_seconds(),
        updated_at: now_unix_seconds(),
        last_test_at: None,
        last_test_error: None,
    };

    match notification_service::fetch_api_quota_snapshot(provider).await {
        Ok(snapshot) => ResolvedApiQuotaFields {
            mode: snapshot.mode,
            today_used_text: snapshot.today_used_text.or(fallback.today_used_text),
            remaining_text: snapshot.remaining_text.or(fallback.remaining_text),
            total_remaining_text: snapshot
                .total_remaining_text
                .or(fallback.total_remaining_text),
            total_tokens_text: snapshot.total_tokens_text.or(fallback.total_tokens_text),
            today_tokens_text: snapshot.today_tokens_text.or(fallback.today_tokens_text),
            daily_window: snapshot.daily_window.or(fallback.daily_window),
            total_window: snapshot.total_window.or(fallback.total_window),
            subscription_expires_at: snapshot
                .subscription_expires_at
                .or(fallback.subscription_expires_at),
            subscription_name: fallback.subscription_name.or(snapshot.subscription_name),
        },
        Err(_) => fallback,
    }
}

fn normalize_account_tags(tags: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        let clipped = trimmed.chars().take(24).collect::<String>();
        if normalized
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&clipped))
        {
            continue;
        }
        normalized.push(clipped);
        if normalized.len() >= 12 {
            break;
        }
    }
    normalized
}

fn fallback_account_label(email: Option<&str>, account_id: &str) -> String {
    email
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Codex {}", short_account(account_id)))
}

fn normalize_import_source(source: &str) -> String {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        "未命名 JSON".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::auth_tokens_need_keepalive_refresh;
    use super::build_account_summaries_for_store;
    use super::build_api_quota_refresh_targets;
    use super::build_refresh_targets;
    use super::build_sub2api_data_payload;
    use super::clear_api_quota_snapshot_fields;
    use super::expand_import_json_content;
    use super::is_api_quota_error_message;
    use super::is_manual_api_quota_subscription_label;
    use super::is_newapi_quota_base_url;
    use super::probe_api_models_internal;
    use super::probe_failure_message;
    use super::relay_profile_change_requires_proxy_reset;
    use super::sync_primary_api_key_from_relay_key_pool;
    use super::upsert_prepared_import;
    use super::upsert_prepared_relay_import;
    use super::ChatgptImportCandidate;
    use super::ImportCandidate;
    use super::PreparedChatgptImport;
    use super::PreparedRelayImport;
    use crate::models::AccountSourceKind;
    use crate::models::AccountsStore;
    use crate::models::ActiveHybridProfile;
    use crate::models::ApiQuotaMode;
    use crate::models::NotificationProviderConfig;
    use crate::models::ProxyEndpointCapability;
    use crate::models::ProxyHealthStatus;
    use crate::models::ProxyKey;
    use crate::models::StoredAccount;
    use crate::models::UsageSnapshot;
    use crate::models::UsageWindow;
    use crate::notification_service;
    use crate::utils::now_unix_seconds;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use reqwest::StatusCode;
    use serde_json::json;

    fn jwt_with_exp(exp: i64) -> String {
        let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#));
        format!("header.{payload}.signature")
    }

    fn usage_snapshot(plan_type: &str) -> UsageSnapshot {
        UsageSnapshot {
            fetched_at: 10,
            plan_type: Some(plan_type.to_string()),
            five_hour: Some(UsageWindow {
                used_percent: 10.0,
                total_percent: None,
                window_seconds: 18_000,
                reset_at: Some(20),
            }),
            one_week: Some(UsageWindow {
                used_percent: 20.0,
                total_percent: None,
                window_seconds: 604_800,
                reset_at: Some(30),
            }),
            credits: None,
        }
    }

    #[tokio::test]
    async fn probe_api_models_accepts_provider_token_prefixes() {
        use axum::extract::Request;
        use axum::http::StatusCode as AxumStatusCode;
        use axum::routing::get;
        use axum::Router;
        use tokio::net::TcpListener;

        async fn models(request: Request) -> (AxumStatusCode, axum::Json<serde_json::Value>) {
            let authorization = request
                .headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok());
            if authorization != Some("Bearer tp-provider-token") {
                return (
                    AxumStatusCode::UNAUTHORIZED,
                    axum::Json(json!({ "error": { "message": "unauthorized" } })),
                );
            }

            (
                AxumStatusCode::OK,
                axum::Json(json!({
                    "data": [
                        { "id": "provider-model", "object": "model" }
                    ]
                })),
            )
        }

        let app = Router::new().route("/v1/models", get(models));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind model probe server");
        let addr = listener.local_addr().expect("model probe server addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let entries = probe_api_models_internal(&format!("http://{addr}/v1"), "tp-provider-token")
            .await
            .expect("probe provider token models");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].model, "provider-model");
    }

    #[tokio::test]
    async fn probe_api_models_falls_back_from_v1_to_root_models() {
        use axum::extract::Request;
        use axum::http::StatusCode as AxumStatusCode;
        use axum::routing::get;
        use axum::Router;
        use tokio::net::TcpListener;

        async fn models(request: Request) -> (AxumStatusCode, axum::Json<serde_json::Value>) {
            let authorization = request
                .headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok());
            if authorization != Some("Bearer root-model-token") {
                return (
                    AxumStatusCode::UNAUTHORIZED,
                    axum::Json(json!({ "error": { "message": "unauthorized" } })),
                );
            }

            (
                AxumStatusCode::OK,
                axum::Json(json!({
                    "models": [
                        { "id": "root-model", "display_name": "Root Model" }
                    ]
                })),
            )
        }

        let app = Router::new().route("/models", get(models));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind root model probe server");
        let addr = listener.local_addr().expect("root model probe server addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let entries = probe_api_models_internal(&format!("http://{addr}/v1"), "root-model-token")
            .await
            .expect("probe root model endpoint");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].model, "root-model");
        assert_eq!(entries[0].display_name.as_deref(), Some("Root Model"));
    }

    fn chatgpt_account_for_export() -> StoredAccount {
        StoredAccount {
            id: "chatgpt-export".to_string(),
            label: "ChatGPT Export".to_string(),
            source_kind: AccountSourceKind::Chatgpt,
            principal_id: Some("export@example.com".to_string()),
            email: Some("export@example.com".to_string()),
            account_id: "workspace-export".to_string(),
            plan_type: Some("pro".to_string()),
            auth_json: json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "access_token": jwt_with_exp(1_800_000_000),
                    "refresh_token": "refresh-export",
                    "id_token": "id-export",
                    "account_id": "workspace-export"
                }
            }),
            api_base_url: None,
            api_key: None,
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: None,
            model_catalog: Vec::new(),
            model_routing_enabled: false,
            balance_text: None,
            balance_display_enabled: false,
            api_quota_mode: Default::default(),
            api_quota_today_used_text: None,
            api_quota_remaining_text: None,
            api_quota_total_remaining_text: None,
            api_quota_total_tokens_text: None,
            api_quota_today_tokens_text: None,
            api_quota_daily_window: None,
            api_quota_total_window: None,
            api_quota_subscription_expires_at: None,
            api_quota_subscription_name: None,
            provider_id: None,
            provider_name: None,
            tags: Vec::new(),
            profile_auth_path: None,
            profile_config_path: None,
            profile_auth_ready: false,
            profile_config_ready: false,
            profile_integrity_error: None,
            profile_last_validated_at: None,
            profile_last_validation_error: None,
            added_at: 1,
            updated_at: 2,
            usage: None,
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        }
    }

    fn prepared_import(
        principal_id: &str,
        account_id: &str,
        email: &str,
        label: &str,
        plan_type: &str,
    ) -> PreparedChatgptImport {
        PreparedChatgptImport {
            principal_id: principal_id.to_string(),
            auth_json: json!({ "kind": label }),
            account_id: account_id.to_string(),
            email: Some(email.to_string()),
            plan_type: Some(plan_type.to_string()),
            usage: Some(usage_snapshot(plan_type)),
            label: Some(label.to_string()),
        }
    }

    fn only_chatgpt_candidate(candidates: &[ImportCandidate]) -> &ChatgptImportCandidate {
        match candidates {
            [ImportCandidate::Chatgpt(candidate)] => candidate,
            _ => panic!("expected one ChatGPT import candidate"),
        }
    }

    fn relay_account_for_tests() -> StoredAccount {
        StoredAccount {
            id: "relay-settings".to_string(),
            label: "Relay Settings".to_string(),
            source_kind: AccountSourceKind::Relay,
            principal_id: Some("relay:settings".to_string()),
            email: None,
            account_id: "relay-settings".to_string(),
            plan_type: Some("api".to_string()),
            auth_json: json!({}),
            api_base_url: Some("https://api.example.com/v1".to_string()),
            api_key: Some("sk-settings".to_string()),
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: Some("gpt-5.4".to_string()),
            model_catalog: Vec::new(),
            model_routing_enabled: false,
            balance_text: None,
            balance_display_enabled: false,
            api_quota_mode: Default::default(),
            api_quota_today_used_text: None,
            api_quota_remaining_text: None,
            api_quota_total_remaining_text: None,
            api_quota_total_tokens_text: None,
            api_quota_today_tokens_text: None,
            api_quota_daily_window: None,
            api_quota_total_window: None,
            api_quota_subscription_expires_at: None,
            api_quota_subscription_name: None,
            provider_id: None,
            provider_name: None,
            tags: Vec::new(),
            profile_auth_path: None,
            profile_config_path: None,
            profile_auth_ready: false,
            profile_config_ready: false,
            profile_integrity_error: None,
            profile_last_validated_at: None,
            profile_last_validation_error: None,
            added_at: 1,
            updated_at: 1,
            usage: None,
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        }
    }

    #[test]
    fn sub2api_export_maps_chatgpt_oauth_account() {
        let mut store = AccountsStore::default();
        store.accounts.push(chatgpt_account_for_export());

        let payload = build_sub2api_data_payload(&store).expect("payload");
        let value = serde_json::to_value(payload).expect("json");
        let account = value["accounts"][0].as_object().expect("account");

        assert_eq!(value["type"], "sub2api-data");
        assert_eq!(value["version"], 1);
        assert_eq!(account["name"], "ChatGPT Export");
        assert_eq!(account["platform"], "openai");
        assert_eq!(account["type"], "oauth");
        assert_eq!(account["concurrency"], 3);
        assert_eq!(account["priority"], 100);
        assert_eq!(
            account["credentials"]["access_token"],
            jwt_with_exp(1_800_000_000)
        );
        assert_eq!(account["credentials"]["refresh_token"], "refresh-export");
        assert_eq!(account["credentials"]["id_token"], "id-export");
        assert_eq!(
            account["credentials"]["client_id"],
            "app_EMoamEEZ73f0CkXaXp7hrann"
        );
        assert_eq!(account["credentials"]["email"], "export@example.com");
        assert_eq!(account["credentials"]["plan_type"], "pro");
        assert_eq!(
            account["credentials"]["chatgpt_account_id"],
            "workspace-export"
        );
        assert_eq!(account["credentials"]["expires_at"], "2027-01-15T08:00:00Z");
        assert_eq!(account["extra"]["openai_passthrough"], true);
        assert_eq!(account["extra"]["codex_cli_only"], true);
    }

    #[test]
    fn sub2api_export_maps_relay_account_as_openai_apikey() {
        let mut store = AccountsStore::default();
        store.accounts.push(relay_account_for_tests());

        let payload = build_sub2api_data_payload(&store).expect("payload");
        let value = serde_json::to_value(payload).expect("json");
        let account = value["accounts"][0].as_object().expect("account");

        assert_eq!(account["name"], "Relay Settings");
        assert_eq!(account["platform"], "openai");
        assert_eq!(account["type"], "apikey");
        assert_eq!(account["concurrency"], 3);
        assert_eq!(account["credentials"]["api_key"], "sk-settings");
        assert_eq!(
            account["credentials"]["base_url"],
            "https://api.example.com/v1"
        );
        assert_eq!(account["credentials"]["default_model"], "gpt-5.4");
        assert_eq!(account["extra"]["openai_passthrough"], true);
    }

    fn notification_provider_for_tests(base_url: &str) -> NotificationProviderConfig {
        NotificationProviderConfig {
            id: "provider-1".to_string(),
            name: "Provider".to_string(),
            account_key: None,
            kind: Default::default(),
            enabled: true,
            cost_multiplier: crate::models::default_notification_cost_multiplier(),
            base_url: base_url.to_string(),
            email: "api@example.com".to_string(),
            password: Some("secret".to_string()),
            created_at: 1,
            updated_at: 1,
            last_test_at: None,
            last_test_error: None,
        }
    }

    #[test]
    fn api_quota_refresh_prefers_sub2api_provider_when_bound() {
        let mut account = relay_account_for_tests();
        account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        account.balance_display_enabled = true;
        account.balance_text = None;

        let mut store = AccountsStore::default();
        store
            .settings
            .notification_providers
            .push(notification_provider_for_tests("https://api.example.com"));
        store.accounts.push(account);

        let targets = build_api_quota_refresh_targets(&store, None);

        assert_eq!(targets.len(), 1);
        match &targets[0].1 {
            super::ApiQuotaRefreshTarget::PlatformProvider { fallback, .. } => {
                assert!(fallback.is_some());
            }
            _ => panic!("expected platform provider refresh target"),
        }
    }

    #[test]
    fn api_quota_refresh_uses_newapi_for_api_key_balance_accounts_before_first_snapshot() {
        let mut account = relay_account_for_tests();
        account.api_quota_mode = ApiQuotaMode::ApiOnly;
        account.balance_display_enabled = true;
        account.balance_text = None;

        let mut store = AccountsStore::default();
        store
            .settings
            .notification_providers
            .push(notification_provider_for_tests("https://api.example.com"));
        store.accounts.push(account);

        let targets = build_api_quota_refresh_targets(&store, None);

        assert_eq!(targets.len(), 1);
        assert!(matches!(
            targets[0].1,
            super::ApiQuotaRefreshTarget::NewapiToken { .. }
        ));
    }

    #[test]
    fn api_quota_refresh_uses_provider_api_key_for_supported_subscription_account() {
        let mut account = relay_account_for_tests();
        account.api_base_url = Some("https://api.minimaxi.com/v1".to_string());
        account.api_quota_mode = ApiQuotaMode::PlatformSubscription;
        account.balance_display_enabled = true;
        account.balance_text = None;

        let mut store = AccountsStore::default();
        store.accounts.push(account);

        let targets = build_api_quota_refresh_targets(&store, None);

        assert_eq!(targets.len(), 1);
        assert!(matches!(
            targets[0].1,
            super::ApiQuotaRefreshTarget::NewapiToken { .. }
        ));
    }

    #[test]
    fn api_quota_refresh_skips_mimo_token_plan_newapi_fallback() {
        let mut account = relay_account_for_tests();
        account.api_base_url = Some("https://token-plan-cn.xiaomimimo.com/v1".to_string());
        account.api_quota_mode = ApiQuotaMode::ApiOnly;
        account.balance_display_enabled = true;
        account.balance_text = None;

        let mut store = AccountsStore::default();
        store.accounts.push(account);

        let targets = build_api_quota_refresh_targets(&store, None);

        assert!(
            targets.is_empty(),
            "MiMo Token Plan does not expose a known API-key quota endpoint and must not be probed as NewAPI"
        );
    }

    #[test]
    fn official_provider_quota_urls_are_not_treated_as_newapi() {
        for base_url in [
            "https://api.deepseek.com/v1",
            "https://api.minimaxi.com/v1",
            "https://api.minimax.io/v1",
            "https://api.moonshot.cn/v1",
            "https://api.z.ai/api/coding/paas/v4",
            "https://token-plan-cn.xiaomimimo.com/v1",
        ] {
            assert!(
                !is_newapi_quota_base_url(base_url),
                "{base_url} must use provider-specific quota logic only"
            );
        }

        assert!(is_newapi_quota_base_url("https://newapi.example.com/v1"));
    }

    #[test]
    fn api_quota_refresh_prefers_bound_platform_login_for_supported_provider_account() {
        let mut account = relay_account_for_tests();
        account.api_base_url = Some("https://api.minimaxi.com/v1".to_string());
        account.api_quota_mode = ApiQuotaMode::PlatformSubscription;
        account.balance_display_enabled = true;
        let account_key = account.account_key();

        let mut provider = notification_provider_for_tests("https://api.minimaxi.com");
        provider.account_key = Some(account_key);

        let mut store = AccountsStore::default();
        store.accounts.push(account);
        store.settings.notification_providers.push(provider);

        let targets = build_api_quota_refresh_targets(&store, None);

        assert_eq!(targets.len(), 1);
        assert!(matches!(
            targets[0].1,
            super::ApiQuotaRefreshTarget::PlatformProvider { .. }
        ));
    }

    #[test]
    fn api_quota_refresh_keeps_manual_platform_account_without_api_key_fallback() {
        let mut account = relay_account_for_tests();
        account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        account.balance_display_enabled = true;
        account.api_key = None;
        account.api_keys.clear();

        let mut store = AccountsStore::default();
        store
            .settings
            .notification_providers
            .push(notification_provider_for_tests("https://api.example.com"));
        store.accounts.push(account);

        let targets = build_api_quota_refresh_targets(&store, None);

        assert_eq!(targets.len(), 1);
        match &targets[0].1 {
            super::ApiQuotaRefreshTarget::PlatformProvider { fallback, .. } => {
                assert!(fallback.is_none());
            }
            _ => panic!("expected platform provider refresh target"),
        }
    }

    #[test]
    fn api_quota_refresh_skips_ambiguous_unbound_platform_providers() {
        let mut first_account = relay_account_for_tests();
        first_account.id = "relay-first".to_string();
        first_account.account_id = "relay-first".to_string();
        first_account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        first_account.balance_display_enabled = true;

        let mut second_account = relay_account_for_tests();
        second_account.id = "relay-second".to_string();
        second_account.account_id = "relay-second".to_string();
        second_account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        second_account.balance_display_enabled = true;

        let mut first_provider = notification_provider_for_tests("https://api.example.com");
        first_provider.id = "provider-first".to_string();
        first_provider.email = "user-a@example.com".to_string();

        let mut second_provider = notification_provider_for_tests("https://api.example.com");
        second_provider.id = "provider-second".to_string();
        second_provider.email = "user-b@example.com".to_string();

        let mut store = AccountsStore::default();
        store.accounts.push(first_account);
        store.accounts.push(second_account);
        store.settings.notification_providers.push(first_provider);
        store.settings.notification_providers.push(second_provider);

        let targets = build_api_quota_refresh_targets(&store, None);

        assert!(
            targets.is_empty(),
            "ambiguous unbound providers must not be shared across same-base-url accounts"
        );
    }

    #[test]
    fn api_quota_refresh_skips_single_unbound_provider_for_multiple_same_base_accounts() {
        let mut first_account = relay_account_for_tests();
        first_account.id = "relay-first".to_string();
        first_account.account_id = "relay-first".to_string();
        first_account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        first_account.balance_display_enabled = true;

        let mut second_account = relay_account_for_tests();
        second_account.id = "relay-second".to_string();
        second_account.account_id = "relay-second".to_string();
        second_account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        second_account.balance_display_enabled = true;

        let mut store = AccountsStore::default();
        store.accounts.push(first_account);
        store.accounts.push(second_account);
        store
            .settings
            .notification_providers
            .push(notification_provider_for_tests("https://api.example.com"));

        let targets = build_api_quota_refresh_targets(&store, None);

        assert!(
            targets.is_empty(),
            "single legacy provider must not be shared across same-base-url accounts"
        );
    }

    #[test]
    fn api_quota_refresh_uses_provider_bound_to_account_key() {
        let mut first_account = relay_account_for_tests();
        first_account.id = "relay-first".to_string();
        first_account.account_id = "relay-first".to_string();
        first_account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        first_account.balance_display_enabled = true;
        let first_account_key = first_account.account_key();

        let mut second_account = relay_account_for_tests();
        second_account.id = "relay-second".to_string();
        second_account.account_id = "relay-second".to_string();
        second_account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        second_account.balance_display_enabled = true;
        let second_account_key = second_account.account_key();

        let mut first_provider = notification_provider_for_tests("https://api.example.com");
        first_provider.id = "provider-first".to_string();
        first_provider.email = "user-a@example.com".to_string();
        first_provider.account_key = Some(first_account_key.clone());

        let mut second_provider = notification_provider_for_tests("https://api.example.com");
        second_provider.id = "provider-second".to_string();
        second_provider.email = "user-b@example.com".to_string();
        second_provider.account_key = Some(second_account_key.clone());

        let mut store = AccountsStore::default();
        store.accounts.push(first_account);
        store.accounts.push(second_account);
        store.settings.notification_providers.push(first_provider);
        store.settings.notification_providers.push(second_provider);

        let targets = build_api_quota_refresh_targets(&store, None);

        assert_eq!(targets.len(), 2);
        for (account_key, target) in targets {
            let super::ApiQuotaRefreshTarget::PlatformProvider { provider, .. } = target else {
                panic!("expected platform provider refresh target");
            };
            match account_key.as_str() {
                value if value == first_account_key => {
                    assert_eq!(provider.email, "user-a@example.com");
                    assert_eq!(
                        provider.account_key.as_deref(),
                        Some(first_account_key.as_str())
                    );
                }
                value if value == second_account_key => {
                    assert_eq!(provider.email, "user-b@example.com");
                    assert_eq!(
                        provider.account_key.as_deref(),
                        Some(second_account_key.as_str())
                    );
                }
                other => panic!("unexpected account key {other}"),
            }
        }
    }

    #[test]
    fn api_quota_refresh_does_not_probe_newapi_for_manual_or_sub2api_accounts() {
        let mut account = relay_account_for_tests();
        account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        account.balance_text = None;

        let mut store = AccountsStore::default();
        store.accounts.push(account);

        let targets = build_api_quota_refresh_targets(&store, None);

        assert!(targets.is_empty());
    }

    #[test]
    fn api_quota_refresh_skips_disabled_balance_display_even_with_stale_balance() {
        let mut account = relay_account_for_tests();
        account.api_quota_mode = ApiQuotaMode::ApiOnly;
        account.balance_display_enabled = false;
        account.balance_text = Some("750000".to_string());

        let mut store = AccountsStore::default();
        store
            .settings
            .notification_providers
            .push(notification_provider_for_tests("https://api.example.com"));
        store.accounts.push(account);

        let targets = build_api_quota_refresh_targets(&store, None);

        assert!(targets.is_empty());
    }

    #[test]
    fn api_quota_snapshot_apply_replaces_stale_balance_with_zero() {
        let mut account = relay_account_for_tests();
        account.balance_display_enabled = true;
        account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        account.balance_text = Some("$144.77".to_string());
        account.api_quota_remaining_text = Some("$144.77".to_string());
        account.api_quota_today_used_text = Some("$1.23".to_string());
        account.api_quota_total_tokens_text = Some("999".to_string());

        let snapshot = notification_service::ApiQuotaSnapshot {
            mode: ApiQuotaMode::PlatformBasic,
            today_used_text: Some("$0.00".to_string()),
            remaining_text: Some("$0.00".to_string()),
            total_remaining_text: Some("$0.00".to_string()),
            total_tokens_text: Some("0".to_string()),
            today_tokens_text: Some("0".to_string()),
            daily_window: None,
            total_window: None,
            subscription_expires_at: None,
            subscription_name: None,
        };

        super::apply_api_quota_snapshot(&mut account, &snapshot, 42);

        assert_eq!(account.balance_text.as_deref(), Some("$0.00"));
        assert_eq!(account.api_quota_remaining_text.as_deref(), Some("$0.00"));
        assert_eq!(account.api_quota_today_used_text.as_deref(), Some("$0.00"));
        assert_eq!(account.api_quota_total_tokens_text.as_deref(), Some("0"));
        assert_eq!(account.api_quota_mode, ApiQuotaMode::PlatformBasic);
        assert_eq!(account.updated_at, 42);
    }

    #[test]
    fn api_quota_snapshot_apply_clears_stale_balance_when_snapshot_has_no_balance() {
        let mut account = relay_account_for_tests();
        account.balance_display_enabled = true;
        account.api_quota_mode = ApiQuotaMode::PlatformBasic;
        account.balance_text = Some("$144.77".to_string());
        account.api_quota_remaining_text = Some("$144.77".to_string());

        let snapshot = notification_service::ApiQuotaSnapshot {
            mode: ApiQuotaMode::PlatformBasic,
            today_used_text: None,
            remaining_text: None,
            total_remaining_text: None,
            total_tokens_text: None,
            today_tokens_text: None,
            daily_window: None,
            total_window: None,
            subscription_expires_at: None,
            subscription_name: None,
        };

        super::apply_api_quota_snapshot(&mut account, &snapshot, 43);

        assert_eq!(account.balance_text, None);
        assert_eq!(account.api_quota_remaining_text, None);
        assert_eq!(account.updated_at, 43);
    }

    #[test]
    fn api_quota_snapshot_apply_keeps_manual_subscription_label() {
        let mut account = relay_account_for_tests();
        account.balance_display_enabled = true;
        account.api_quota_mode = ApiQuotaMode::PlatformSubscription;
        account.api_quota_subscription_name = Some("Plus".to_string());

        let snapshot = notification_service::ApiQuotaSnapshot {
            mode: ApiQuotaMode::PlatformSubscription,
            today_used_text: None,
            remaining_text: None,
            total_remaining_text: None,
            total_tokens_text: None,
            today_tokens_text: None,
            daily_window: None,
            total_window: None,
            subscription_expires_at: None,
            subscription_name: None,
        };

        super::apply_api_quota_snapshot(&mut account, &snapshot, 44);

        assert_eq!(account.api_quota_subscription_name.as_deref(), Some("Plus"));
        assert_eq!(account.updated_at, 44);
    }

    #[test]
    fn api_quota_snapshot_apply_replaces_unknown_subscription_label() {
        let mut account = relay_account_for_tests();
        account.balance_display_enabled = true;
        account.api_quota_mode = ApiQuotaMode::PlatformSubscription;
        account.api_quota_subscription_name = Some("Enterprise".to_string());

        let snapshot = notification_service::ApiQuotaSnapshot {
            mode: ApiQuotaMode::PlatformSubscription,
            today_used_text: None,
            remaining_text: None,
            total_remaining_text: None,
            total_tokens_text: None,
            today_tokens_text: None,
            daily_window: None,
            total_window: None,
            subscription_expires_at: None,
            subscription_name: Some("Max".to_string()),
        };

        super::apply_api_quota_snapshot(&mut account, &snapshot, 45);

        assert_eq!(account.api_quota_subscription_name.as_deref(), Some("Max"));
        assert_eq!(account.updated_at, 45);
    }

    #[test]
    fn api_quota_refresh_failure_clears_snapshot_but_keeps_mode() {
        let mut account = relay_account_for_tests();
        account.balance_display_enabled = true;
        account.api_quota_mode = ApiQuotaMode::PlatformSubscription;
        account.balance_text = Some("$144.77".to_string());
        account.api_quota_remaining_text = Some("$144.77".to_string());
        account.api_quota_today_used_text = Some("$1.23".to_string());

        clear_api_quota_snapshot_fields(&mut account);

        assert_eq!(account.balance_text, None);
        assert_eq!(account.api_quota_remaining_text, None);
        assert_eq!(account.api_quota_today_used_text, None);
        assert_eq!(account.api_quota_mode, ApiQuotaMode::PlatformSubscription);
    }

    #[test]
    fn api_quota_error_filter_recognizes_platform_endpoint_errors() {
        assert!(is_api_quota_error_message(
            "API 平台用量统计接口失败: API 平台接口失败 500: boom"
        ));
        assert!(is_api_quota_error_message(
            "API 平台用户接口失败: API 平台接口返回格式异常"
        ));
    }

    #[test]
    fn api_quota_manual_subscription_labels_cover_known_provider_plans() {
        for label in [
            "Plus",
            "Max",
            "Ultra",
            "Lite",
            "Standard",
            "Pro",
            "Adagio",
            "Moderato",
            "Allegretto",
            "Allegro",
            "Vivace",
        ] {
            assert!(
                is_manual_api_quota_subscription_label(label),
                "{label} should be kept as a manual quota subscription label"
            );
        }

        assert!(!is_manual_api_quota_subscription_label("Enterprise"));
    }

    fn chatgpt_account_for_tests() -> StoredAccount {
        StoredAccount {
            id: "chatgpt-settings".to_string(),
            label: "ChatGPT Settings".to_string(),
            source_kind: AccountSourceKind::Chatgpt,
            principal_id: Some("chatgpt@example.com".to_string()),
            email: Some("chatgpt@example.com".to_string()),
            account_id: "chatgpt-account".to_string(),
            plan_type: Some("pro".to_string()),
            auth_json: json!({ "kind": "chatgpt" }),
            api_base_url: None,
            api_key: None,
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: None,
            model_catalog: Vec::new(),
            model_routing_enabled: false,
            balance_text: None,
            balance_display_enabled: false,
            api_quota_mode: Default::default(),
            api_quota_today_used_text: None,
            api_quota_remaining_text: None,
            api_quota_total_remaining_text: None,
            api_quota_total_tokens_text: None,
            api_quota_today_tokens_text: None,
            api_quota_daily_window: None,
            api_quota_total_window: None,
            api_quota_subscription_expires_at: None,
            api_quota_subscription_name: None,
            provider_id: None,
            provider_name: None,
            tags: Vec::new(),
            profile_auth_path: None,
            profile_config_path: None,
            profile_auth_ready: false,
            profile_config_ready: false,
            profile_integrity_error: None,
            profile_last_validated_at: None,
            profile_last_validation_error: None,
            added_at: 1,
            updated_at: 1,
            usage: None,
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        }
    }

    #[test]
    fn hybrid_summaries_mark_relay_as_current_even_when_auth_matches_chatgpt() {
        let chatgpt = chatgpt_account_for_tests();
        let relay = relay_account_for_tests();
        let chatgpt_variant_key = chatgpt.variant_key();
        let mut store = AccountsStore::default();
        store.settings.active_account_id = Some(relay.id.clone());
        store.settings.active_hybrid_profile = Some(ActiveHybridProfile {
            chatgpt_account_id: chatgpt.id.clone(),
            relay_account_id: relay.id.clone(),
        });
        store.accounts = vec![chatgpt.clone(), relay.clone()];

        let summaries =
            build_account_summaries_for_store(&store, None, Some(chatgpt_variant_key.as_str()));

        let current_ids = summaries
            .iter()
            .filter(|account| account.is_current)
            .map(|account| account.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(current_ids, vec![relay.id.as_str()]);
    }

    #[test]
    fn sync_primary_api_key_prefers_healthy_enabled_key() {
        let now = now_unix_seconds();
        let mut account = relay_account_for_tests();
        account.api_keys = vec![
            ProxyKey {
                id: "bad".to_string(),
                label: Some("bad".to_string()),
                secret: Some("sk-bad".to_string()),
                enabled: true,
                priority: 100,
                weight: 100,
                health_status: ProxyHealthStatus::AuthFailed,
                last_error: Some("401".to_string()),
                cooldown_until: Some(now + 300),
                failure_count: 2,
                last_used_at: None,
                updated_at: Some(now),
            },
            ProxyKey {
                id: "good".to_string(),
                label: Some("good".to_string()),
                secret: Some("sk-good".to_string()),
                enabled: true,
                priority: 100,
                weight: 100,
                health_status: ProxyHealthStatus::Healthy,
                last_error: None,
                cooldown_until: None,
                failure_count: 0,
                last_used_at: None,
                updated_at: Some(now),
            },
        ];

        sync_primary_api_key_from_relay_key_pool(&mut account);

        assert_eq!(account.api_key.as_deref(), Some("sk-good"));
    }

    #[test]
    fn relay_profile_change_detection_matches_gateway_reset_expectations() {
        let account = relay_account_for_tests();

        assert!(!relay_profile_change_requires_proxy_reset(
            &account,
            "https://api.example.com/v1",
            "gpt-5.4",
            None,
        ));
        assert!(relay_profile_change_requires_proxy_reset(
            &account,
            "https://api.changed.example.com/v1",
            "gpt-5.4",
            None,
        ));
        assert!(relay_profile_change_requires_proxy_reset(
            &account,
            "https://api.example.com/v1",
            "gpt-5.5",
            None,
        ));
        assert!(relay_profile_change_requires_proxy_reset(
            &account,
            "https://api.example.com/v1",
            "gpt-5.4",
            Some("sk-changed"),
        ));
    }

    #[test]
    fn expand_import_json_content_supports_exported_accounts_store() {
        let raw = json!({
            "version": 1,
            "accounts": [
                {
                    "id": "stored-1",
                    "label": "Alpha",
                    "email": "alpha@example.com",
                    "accountId": "account-1",
                    "planType": "team",
                    "authJson": {
                        "auth_mode": "chatgpt",
                        "tokens": {
                            "access_token": "access-1",
                            "id_token": "id-1",
                            "refresh_token": "refresh-1"
                        }
                    },
                    "addedAt": 1,
                    "updatedAt": 2,
                    "usage": {
                        "fetchedAt": 3,
                        "planType": "team",
                        "fiveHour": null,
                        "oneWeek": null,
                        "credits": null
                    },
                    "usageError": null
                }
            ],
            "settings": {}
        })
        .to_string();

        let candidates = expand_import_json_content(&raw, "accounts.json", None).unwrap();

        assert_eq!(candidates.len(), 1);
        let candidate = only_chatgpt_candidate(&candidates);
        assert_eq!(candidate.label.as_deref(), Some("Alpha"));
        assert_eq!(candidate.email.as_deref(), Some("alpha@example.com"));
        assert_eq!(candidate.plan_type.as_deref(), Some("team"));
        assert_eq!(
            candidate
                .usage
                .as_ref()
                .and_then(|usage| usage.plan_type.as_deref()),
            Some("team")
        );
        assert_eq!(candidate.source, "accounts.json / #1 / Alpha");
        assert_eq!(
            candidate
                .auth_json
                .get("tokens")
                .and_then(serde_json::Value::as_object)
                .and_then(|tokens| tokens.get("access_token"))
                .and_then(serde_json::Value::as_str),
            Some("access-1")
        );
    }

    #[test]
    fn expand_import_json_content_supports_single_stored_account_backup() {
        let raw = json!({
            "id": "stored-1",
            "label": "Solo",
            "email": "solo@example.com",
            "accountId": "account-1",
            "authJson": {
                "auth_mode": "chatgpt",
                "tokens": {
                    "access_token": "access-1",
                    "id_token": "id-1",
                    "refresh_token": "refresh-1"
                }
            },
            "addedAt": 1,
            "updatedAt": 2
        })
        .to_string();

        let candidates = expand_import_json_content(&raw, "account.json", None).unwrap();

        assert_eq!(candidates.len(), 1);
        let candidate = only_chatgpt_candidate(&candidates);
        assert_eq!(candidate.label.as_deref(), Some("Solo"));
        assert_eq!(candidate.source, "account.json / Solo");
    }

    #[test]
    fn expand_import_json_content_normalizes_flat_auth_json() {
        let raw = json!({
            "auth_mode": "chatgpt",
            "access_token": "access-1",
            "id_token": "id-1",
            "refresh_token": "refresh-1"
        })
        .to_string();

        let candidates = expand_import_json_content(&raw, "auth.json", None).unwrap();

        assert_eq!(candidates.len(), 1);
        let candidate = only_chatgpt_candidate(&candidates);
        assert_eq!(candidate.source, "auth.json");
        assert_eq!(
            candidate
                .auth_json
                .get("tokens")
                .and_then(serde_json::Value::as_object)
                .and_then(|tokens| tokens.get("access_token"))
                .and_then(serde_json::Value::as_str),
            Some("access-1")
        );
    }

    #[test]
    fn expand_import_json_content_supports_sub2api_oauth_data() {
        let raw = json!({
            "exported_at": "2026-05-24T00:00:00Z",
            "proxies": [],
            "accounts": [
                {
                    "name": "Sub2 OAuth",
                    "platform": "openai",
                    "type": "oauth",
                    "credentials": {
                        "access_token": "access-sub2",
                        "refresh_token": "refresh-sub2",
                        "id_token": "id-sub2",
                        "chatgpt_account_id": "workspace-sub2",
                        "email": "sub2@example.com",
                        "plan_type": "team"
                    }
                }
            ]
        })
        .to_string();

        let candidates = expand_import_json_content(&raw, "sub2api.json", None).unwrap();

        assert_eq!(candidates.len(), 1);
        let candidate = only_chatgpt_candidate(&candidates);
        assert_eq!(candidate.source, "sub2api.json / #1 / Sub2 OAuth");
        assert_eq!(candidate.label.as_deref(), Some("Sub2 OAuth"));
        assert_eq!(candidate.email.as_deref(), Some("sub2@example.com"));
        assert_eq!(candidate.plan_type.as_deref(), Some("team"));
        assert_eq!(
            candidate
                .auth_json
                .get("tokens")
                .and_then(serde_json::Value::as_object)
                .and_then(|tokens| tokens.get("account_id"))
                .and_then(serde_json::Value::as_str),
            Some("workspace-sub2")
        );
        assert_eq!(
            candidate
                .auth_json
                .get("tokens")
                .and_then(serde_json::Value::as_object)
                .and_then(|tokens| tokens.get("refresh_token"))
                .and_then(serde_json::Value::as_str),
            Some("refresh-sub2")
        );
    }

    #[test]
    fn expand_import_json_content_supports_sub2api_apikey_data() {
        let raw = json!({
            "type": "sub2api-data",
            "version": 1,
            "accounts": [
                {
                    "name": "Sub2 API",
                    "platform": "openai",
                    "type": "apikey",
                    "credentials": {
                        "api_key": "sk-sub2",
                        "base_url": "https://api.example.com/v1",
                        "default_model": "gpt-5.4"
                    }
                }
            ]
        })
        .to_string();

        let candidates = expand_import_json_content(&raw, "sub2api.json", None).unwrap();

        assert_eq!(candidates.len(), 1);
        let candidate = match &candidates[..] {
            [ImportCandidate::Relay(candidate)] => candidate,
            _ => panic!("expected one Relay import candidate"),
        };
        assert_eq!(candidate.source, "sub2api.json / #1 / Sub2 API");
        assert_eq!(candidate.label, "Sub2 API");
        assert_eq!(candidate.api_key, "sk-sub2");
        assert_eq!(candidate.base_url, "https://api.example.com/v1");
        assert_eq!(candidate.model_name.as_deref(), Some("gpt-5.4"));
    }

    #[test]
    fn upsert_prepared_import_reuses_unknown_variant_placeholder() {
        let mut store = AccountsStore::default();
        store.accounts.push(StoredAccount {
            id: "existing".to_string(),
            label: "placeholder".to_string(),
            source_kind: Default::default(),
            principal_id: Some("fresh@example.com".to_string()),
            email: Some("fresh@example.com".to_string()),
            account_id: "account-1".to_string(),
            plan_type: None,
            auth_json: json!({ "kind": "old" }),
            api_base_url: None,
            api_key: None,
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: None,
            model_catalog: Vec::new(),
            model_routing_enabled: false,
            balance_text: None,
            balance_display_enabled: false,
            api_quota_mode: Default::default(),
            api_quota_today_used_text: None,
            api_quota_remaining_text: None,
            api_quota_total_remaining_text: None,
            api_quota_total_tokens_text: None,
            api_quota_today_tokens_text: None,
            api_quota_daily_window: None,
            api_quota_total_window: None,
            api_quota_subscription_expires_at: None,
            api_quota_subscription_name: None,
            provider_id: None,
            provider_name: None,
            tags: Vec::new(),
            profile_auth_path: None,
            profile_config_path: None,
            profile_auth_ready: false,
            profile_config_ready: false,
            profile_integrity_error: None,
            profile_last_validated_at: None,
            profile_last_validation_error: None,
            added_at: 1,
            updated_at: 1,
            usage: None,
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        });

        let prepared = prepared_import(
            "fresh@example.com",
            "account-1",
            "fresh@example.com",
            "fresh",
            "team",
        );

        let (summary, updated_existing) = upsert_prepared_import(&mut store, prepared, None, None);

        assert!(updated_existing);
        assert_eq!(store.accounts.len(), 1);
        assert_eq!(summary.id, "existing");
        assert_eq!(store.accounts[0].label, "fresh");
        assert_eq!(store.accounts[0].plan_type.as_deref(), Some("team"));
        assert_eq!(
            store.accounts[0]
                .usage
                .as_ref()
                .and_then(|usage| usage.plan_type.as_deref()),
            Some("team")
        );
    }

    #[test]
    fn upsert_prepared_import_prefers_auth_plan_type_over_usage_plan_type() {
        let mut store = AccountsStore::default();
        let prepared = PreparedChatgptImport {
            principal_id: "shared@example.com".to_string(),
            auth_json: json!({ "kind": "team-auth" }),
            account_id: "account-1".to_string(),
            email: Some("shared@example.com".to_string()),
            plan_type: Some("team".to_string()),
            usage: Some(usage_snapshot("plus")),
            label: Some("team".to_string()),
        };

        let (summary, updated_existing) = upsert_prepared_import(&mut store, prepared, None, None);

        assert!(!updated_existing);
        assert_eq!(summary.plan_type.as_deref(), Some("team"));
        assert_eq!(store.accounts[0].plan_type.as_deref(), Some("team"));
        assert_eq!(
            store.accounts[0]
                .usage
                .as_ref()
                .and_then(|usage| usage.plan_type.as_deref()),
            Some("plus")
        );
    }

    #[test]
    fn upsert_prepared_relay_import_creates_relay_account() {
        let mut store = AccountsStore::default();
        let prepared = PreparedRelayImport {
            label: "Sub2 API".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: "sk-sub2".to_string(),
            model_name: "gpt-5.4".to_string(),
        };

        let (summary, updated_existing) =
            upsert_prepared_relay_import(&mut store, prepared, None, None);

        assert!(!updated_existing);
        assert!(matches!(summary.source_kind, AccountSourceKind::Relay));
        assert_eq!(summary.label, "Sub2 API");
        assert_eq!(
            summary.api_base_url.as_deref(),
            Some("https://api.example.com/v1")
        );
        assert_eq!(summary.model_name.as_deref(), Some("gpt-5.4"));
        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.accounts[0].api_key.as_deref(), Some("sk-sub2"));
        assert_eq!(
            store.accounts[0].proxy_endpoints,
            vec![ProxyEndpointCapability::ChatCompletions]
        );
    }

    #[test]
    fn upsert_prepared_import_keeps_same_workspace_different_users_separate() {
        let mut store = AccountsStore::default();

        let first = prepared_import(
            "first@example.com",
            "workspace-1",
            "first@example.com",
            "first",
            "team",
        );
        let second = prepared_import(
            "second@example.com",
            "workspace-1",
            "second@example.com",
            "second",
            "team",
        );

        let (_, updated_first) = upsert_prepared_import(&mut store, first, None, None);
        let (_, updated_second) = upsert_prepared_import(&mut store, second, None, None);

        assert!(!updated_first);
        assert!(!updated_second);
        assert_eq!(store.accounts.len(), 2);
        assert_ne!(
            store.accounts[0].account_key(),
            store.accounts[1].account_key()
        );
        assert_ne!(
            store.accounts[0].variant_key(),
            store.accounts[1].variant_key()
        );
    }

    #[test]
    fn build_refresh_targets_ignores_stale_current_auth_override() {
        let mut store = AccountsStore::default();
        store.accounts.push(StoredAccount {
            id: "existing".to_string(),
            label: "fresh".to_string(),
            source_kind: Default::default(),
            principal_id: Some("fresh@example.com".to_string()),
            email: Some("fresh@example.com".to_string()),
            account_id: "account-1".to_string(),
            plan_type: Some("team".to_string()),
            auth_json: json!({
                "auth_mode": "chatgpt",
                "last_refresh": "2026-05-12T16:07:42Z",
                "tokens": {
                    "access_token": "fresh-access",
                    "id_token": "fresh-id",
                    "refresh_token": "fresh-refresh",
                    "account_id": "account-1"
                }
            }),
            api_base_url: None,
            api_key: None,
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: None,
            model_catalog: Vec::new(),
            model_routing_enabled: false,
            balance_text: None,
            balance_display_enabled: false,
            api_quota_mode: Default::default(),
            api_quota_today_used_text: None,
            api_quota_remaining_text: None,
            api_quota_total_remaining_text: None,
            api_quota_total_tokens_text: None,
            api_quota_today_tokens_text: None,
            api_quota_daily_window: None,
            api_quota_total_window: None,
            api_quota_subscription_expires_at: None,
            api_quota_subscription_name: None,
            provider_id: None,
            provider_name: None,
            tags: Vec::new(),
            profile_auth_path: None,
            profile_config_path: None,
            profile_auth_ready: false,
            profile_config_ready: false,
            profile_integrity_error: None,
            profile_last_validated_at: None,
            profile_last_validation_error: None,
            added_at: 1,
            updated_at: 20,
            usage: None,
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        });
        let account_key = store.accounts[0].account_key();
        let stale_current_auth = json!({
            "auth_mode": "chatgpt",
            "last_refresh": "2026-05-12T15:32:53Z",
            "tokens": {
                "access_token": "stale-access",
                "id_token": "stale-id",
                "refresh_token": "stale-refresh",
                "account_id": "account-1"
            }
        });

        let targets = build_refresh_targets(
            store.accounts,
            Some(&(account_key, stale_current_auth)),
            None,
        );

        assert_eq!(targets.len(), 1);
        assert!(!targets[0].auth_is_current);
        assert_eq!(
            targets[0]
                .auth_json
                .get("tokens")
                .and_then(serde_json::Value::as_object)
                .and_then(|tokens| tokens.get("access_token"))
                .and_then(serde_json::Value::as_str),
            Some("fresh-access")
        );
    }

    #[test]
    fn build_refresh_targets_accepts_fresh_current_auth_override() {
        let mut store = AccountsStore::default();
        store.accounts.push(StoredAccount {
            id: "existing".to_string(),
            label: "stale".to_string(),
            source_kind: Default::default(),
            principal_id: Some("fresh@example.com".to_string()),
            email: Some("fresh@example.com".to_string()),
            account_id: "account-1".to_string(),
            plan_type: Some("team".to_string()),
            auth_json: json!({
                "auth_mode": "chatgpt",
                "last_refresh": "2026-05-12T15:32:53Z",
                "tokens": {
                    "access_token": "stale-access",
                    "id_token": "stale-id",
                    "refresh_token": "stale-refresh",
                    "account_id": "account-1"
                }
            }),
            api_base_url: None,
            api_key: None,
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: None,
            model_catalog: Vec::new(),
            model_routing_enabled: false,
            balance_text: None,
            balance_display_enabled: false,
            api_quota_mode: Default::default(),
            api_quota_today_used_text: None,
            api_quota_remaining_text: None,
            api_quota_total_remaining_text: None,
            api_quota_total_tokens_text: None,
            api_quota_today_tokens_text: None,
            api_quota_daily_window: None,
            api_quota_total_window: None,
            api_quota_subscription_expires_at: None,
            api_quota_subscription_name: None,
            provider_id: None,
            provider_name: None,
            tags: Vec::new(),
            profile_auth_path: None,
            profile_config_path: None,
            profile_auth_ready: false,
            profile_config_ready: false,
            profile_integrity_error: None,
            profile_last_validated_at: None,
            profile_last_validation_error: None,
            added_at: 1,
            updated_at: 20,
            usage: None,
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        });
        let account_key = store.accounts[0].account_key();
        let fresh_current_auth = json!({
            "auth_mode": "chatgpt",
            "last_refresh": "2026-05-12T16:07:42Z",
            "tokens": {
                "access_token": "fresh-access",
                "id_token": "fresh-id",
                "refresh_token": "fresh-refresh",
                "account_id": "account-1"
            }
        });

        let targets = build_refresh_targets(
            store.accounts,
            Some(&(account_key, fresh_current_auth)),
            None,
        );

        assert_eq!(targets.len(), 1);
        assert!(targets[0].auth_is_current);
        assert_eq!(
            targets[0]
                .auth_json
                .get("tokens")
                .and_then(serde_json::Value::as_object)
                .and_then(|tokens| tokens.get("access_token"))
                .and_then(serde_json::Value::as_str),
            Some("fresh-access")
        );
    }

    #[test]
    fn keepalive_refreshes_one_day_before_access_token_expiry() {
        let now = now_unix_seconds();
        let soon_expiring_auth = json!({
            "auth_mode": "chatgpt",
            "last_refresh": now,
            "tokens": {
                "access_token": jwt_with_exp(now + 23 * 60 * 60),
                "id_token": jwt_with_exp(now + 23 * 60 * 60),
                "refresh_token": "old-refresh",
                "account_id": "account-1"
            }
        });
        let fresh_auth = json!({
            "auth_mode": "chatgpt",
            "last_refresh": now,
            "tokens": {
                "access_token": jwt_with_exp(now + 25 * 60 * 60),
                "id_token": jwt_with_exp(now + 25 * 60 * 60),
                "refresh_token": "old-refresh",
                "account_id": "account-1"
            }
        });

        assert!(auth_tokens_need_keepalive_refresh(&soon_expiring_auth));
        assert!(!auth_tokens_need_keepalive_refresh(&fresh_auth));
    }

    #[test]
    fn keepalive_rotates_refresh_token_after_seven_days_even_when_access_token_is_fresh() {
        let now = now_unix_seconds();
        let stale_rotation_auth = json!({
            "auth_mode": "chatgpt",
            "last_refresh": now - 7 * 24 * 60 * 60,
            "tokens": {
                "access_token": jwt_with_exp(now + 10 * 24 * 60 * 60),
                "id_token": jwt_with_exp(now + 10 * 24 * 60 * 60),
                "refresh_token": "old-refresh",
                "account_id": "account-1"
            }
        });
        let recent_rotation_auth = json!({
            "auth_mode": "chatgpt",
            "last_refresh": now - 6 * 24 * 60 * 60,
            "tokens": {
                "access_token": jwt_with_exp(now + 10 * 24 * 60 * 60),
                "id_token": jwt_with_exp(now + 10 * 24 * 60 * 60),
                "refresh_token": "old-refresh",
                "account_id": "account-1"
            }
        });
        let missing_rotation_timestamp_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": jwt_with_exp(now + 10 * 24 * 60 * 60),
                "id_token": jwt_with_exp(now + 10 * 24 * 60 * 60),
                "refresh_token": "old-refresh",
                "account_id": "account-1"
            }
        });

        assert!(auth_tokens_need_keepalive_refresh(&stale_rotation_auth));
        assert!(!auth_tokens_need_keepalive_refresh(&recent_rotation_auth));
        assert!(auth_tokens_need_keepalive_refresh(
            &missing_rotation_timestamp_auth
        ));
    }

    #[test]
    fn probe_failure_message_redacts_upstream_body() {
        let key_prefix = "s";
        let secret_value = format!("{key_prefix}k-probe-secret-1234567890");
        let local_path = ["D:", "\\workspace\\secret"].concat();
        let upstream = ["https://", "api.example.invalid/v1"].concat();
        let body = format!("failed with {secret_value} {local_path} {upstream}");

        let message = probe_failure_message(StatusCode::BAD_GATEWAY, &body);

        assert!(!message.contains(&secret_value));
        assert!(!message.contains(&local_path));
        assert!(!message.contains("api.example.invalid"));
        assert!(message.contains("[已隐藏密钥]"));
        assert!(message.contains("[已隐藏本地路径]"));
    }
}
