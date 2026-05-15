use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt as _;

use crate::app_paths;
use crate::cli;
use crate::models::AccountPoolConfig;
use crate::models::AppSettings;
use crate::models::AppSettingsPatch;
use crate::models::NotificationBotConfig;
use crate::models::NotificationPipelineConfig;
use crate::models::NotificationProviderConfig;
use crate::models::NotificationScheduleMode;
use crate::models::NotificationTargetConfig;
use crate::models::NotificationTemplateConfig;
use crate::profile_files;
use crate::state::AppState;
use crate::store;
use crate::store::load_store;
use crate::store::save_store;

const CONTEXT_WINDOW_MIN_K: u16 = 272;
const CONTEXT_WINDOW_MAX_K: u16 = 1000;
const CONTEXT_WINDOW_KEY: &str = "model_context_window";
const AUTO_COMPACT_LIMIT_KEY: &str = "model_auto_compact_token_limit";
const TOOL_OUTPUT_LIMIT_KEY: &str = "tool_output_token_limit";
const MODEL_KEY: &str = "model";
const MODEL_REASONING_EFFORT_KEY: &str = "model_reasoning_effort";
const MODELS_CACHE_FILE_NAME: &str = "models_cache.json";
const AUTO_COMPACT_RATIO_NUMERATOR: u32 = 95;
const AUTO_COMPACT_RATIO_DENOMINATOR: u32 = 100;

fn normalize_notification_schedule_mode(
    mode: NotificationScheduleMode,
    schedule_date: Option<&str>,
    schedule_time: Option<&str>,
    interval_minutes: Option<u16>,
) -> NotificationScheduleMode {
    match mode {
        NotificationScheduleMode::Manual => {
            if schedule_date.is_some() {
                NotificationScheduleMode::Date
            } else if interval_minutes.is_some() {
                NotificationScheduleMode::Interval
            } else if schedule_time.is_some() {
                NotificationScheduleMode::Daily
            } else {
                NotificationScheduleMode::Manual
            }
        }
        other => other,
    }
}

fn normalize_notification_interval_minutes(value: Option<u16>) -> Option<u16> {
    value.map(|minutes| minutes.clamp(1, 1440))
}

#[derive(Debug, Clone, Copy, Default)]
struct CodexContextWindowLimits {
    hard_limit_k: Option<u16>,
    effective_limit_k: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct CachedModelsFile {
    #[serde(default)]
    models: Vec<CachedModelEntry>,
}

#[derive(Debug, Deserialize)]
struct CachedModelEntry {
    slug: String,
    #[serde(default)]
    context_window: Option<u32>,
    #[serde(default)]
    max_context_window: Option<u32>,
    #[serde(default)]
    effective_context_window_percent: Option<u8>,
}

/// 读取应用设置（前端设置页使用）。
pub(crate) async fn get_app_settings_internal(
    app: &AppHandle,
    state: &AppState,
) -> Result<AppSettings, String> {
    let _guard = state.store_lock.lock().await;
    let mut store = load_store(app)?;
    let mut store_changed = ensure_notification_settings_shape(&mut store.settings);
    if store
        .settings
        .codex_launch_path
        .as_deref()
        .is_some_and(should_discard_codex_launch_path)
    {
        store.settings.codex_launch_path = None;
        store_changed = true;
    }
    let config_path = app_paths::codex_config_path()?;
    let requested_context_window_k = read_codex_context_window_k_from_config(&config_path)?;
    let (context_window_model, limits) =
        read_codex_context_window_limits_for_current_model(&config_path)?;
    let context_window_k = requested_context_window_k.map(normalize_context_window_k);
    if store.settings.codex_context_window_k != context_window_k {
        store.settings.codex_context_window_k = context_window_k;
        store_changed = true;
    }
    if store_changed {
        save_store(app, &store)?;
    }
    let mut settings = store.settings.clone();
    populate_codex_context_window_metadata(&mut settings, context_window_model, limits);
    Ok(settings)
}

/// 更新应用设置并持久化：
/// - 存储到 `accounts.json.settings`
/// - 若涉及开机启动开关，立即同步到系统。
pub(crate) async fn update_app_settings_internal(
    app: &AppHandle,
    state: &AppState,
    patch: AppSettingsPatch,
) -> Result<AppSettings, String> {
    let mut launch_at_startup_to_apply = None;
    let mut context_window_to_apply = None;
    let config_path = app_paths::codex_config_path()?;
    let (context_window_model, limits) =
        read_codex_context_window_limits_for_current_model(&config_path)?;
    let settings = {
        let _guard = state.store_lock.lock().await;
        let mut store = load_store(app)?;
        ensure_notification_settings_shape(&mut store.settings);

        if let Some(value) = patch.launch_at_startup {
            store.settings.launch_at_startup = value;
            launch_at_startup_to_apply = Some(value);
        }
        if let Some(value) = patch.tray_usage_display_mode {
            store.settings.tray_usage_display_mode = value;
        }
        if let Some(value) = patch.launch_codex_after_switch {
            store.settings.launch_codex_after_switch = value;
        }
        if let Some(value) = patch.smart_switch_include_api {
            store.settings.smart_switch_include_api = value;
        }
        if let Some(value) = patch.usage_auto_refresh_enabled {
            store.settings.usage_auto_refresh_enabled = value;
        }
        if let Some(value) = patch.usage_auto_refresh_interval_secs {
            store.settings.usage_auto_refresh_interval_secs = value.clamp(15, 600);
        }
        if let Some(value) = patch.api_quota_auto_refresh_enabled {
            store.settings.api_quota_auto_refresh_enabled = value;
        }
        if let Some(value) = patch.api_quota_auto_refresh_interval_secs {
            store.settings.api_quota_auto_refresh_interval_secs = value.clamp(60, 3600);
        }
        if let Some(value) = patch.quota_alert_enabled {
            store.settings.quota_alert_enabled = value;
        }
        if let Some(value) = patch.show_provider_badge {
            store.settings.show_provider_badge = value;
        }
        if let Some(value) = patch.codex_context_window_k {
            let normalized = value.map(normalize_context_window_k);
            store.settings.codex_context_window_k = normalized;
            context_window_to_apply = Some(normalized);
        }
        if let Some(value) = patch.quota_alert_five_hour_threshold {
            store.settings.quota_alert_five_hour_threshold = value.min(100);
        }
        if let Some(value) = patch.quota_alert_one_week_threshold {
            store.settings.quota_alert_one_week_threshold = value.min(100);
        }
        if let Some(value) = patch.codex_launch_path {
            store.settings.codex_launch_path = normalize_codex_launch_path_for_storage(value)?;
        }
        if let Some(value) = patch.sync_opencode_openai_auth {
            store.settings.sync_opencode_openai_auth = value;
        }
        if let Some(value) = patch.restart_opencode_desktop_on_switch {
            store.settings.restart_opencode_desktop_on_switch = value;
        }
        if let Some(value) = patch.restart_editors_on_switch {
            store.settings.restart_editors_on_switch = value;
        }
        if let Some(value) = patch.restart_editor_targets {
            store.settings.restart_editor_targets = value;
        }
        if let Some(value) = patch.account_pools {
            store.settings.account_pools = normalize_account_pools(value, &store.accounts);
        }
        if let Some(value) = patch.notification_providers {
            store.settings.notification_providers = normalize_notification_providers(value);
        }
        if let Some(value) = patch.notification_targets {
            store.settings.notification_targets = normalize_notification_targets(value);
        }
        if let Some(value) = patch.notification_bots {
            store.settings.notification_bots = normalize_notification_bots(value);
        }
        if let Some(value) = patch.notification_templates {
            store.settings.notification_templates = normalize_notification_templates(value);
        }
        if let Some(value) = patch.notification_pipelines {
            store.settings.notification_pipelines = normalize_notification_pipelines(value);
            store.settings.notification_schema_version = 1;
        }
        if let Some(value) = patch.notification_schema_version {
            store.settings.notification_schema_version = value.max(1);
        }
        if let Some(value) = patch.locale {
            store.settings.locale = value;
        }
        if let Some(value) = patch.skipped_update_version {
            store.settings.skipped_update_version = value;
        }

        if let Some(value) = context_window_to_apply {
            sync_codex_context_window_setting(app, &store, &config_path, value)?;
        }

        let mut settings = store.settings.clone();
        ensure_notification_settings_shape(&mut settings);
        populate_codex_context_window_metadata(&mut settings, context_window_model.clone(), limits);
        save_store(app, &store)?;
        settings
    };

    if let Some(value) = launch_at_startup_to_apply {
        set_system_autostart(app, value)?;
    }

    Ok(settings)
}

/// 启动时根据本地设置校准系统开机启动状态，避免“设置与系统实际状态不一致”。
pub(crate) fn sync_autostart_from_store(app: &AppHandle) -> Result<(), String> {
    if app_paths::is_dev_runtime() {
        log::info!("开发预览环境跳过系统开机启动同步");
        return Ok(());
    }

    let settings = load_store(app)?.settings;
    let current_enabled = app
        .autolaunch()
        .is_enabled()
        .map_err(|e| format!("读取开机启动状态失败: {e}"))?;

    if current_enabled != settings.launch_at_startup {
        set_system_autostart(app, settings.launch_at_startup)?;
    }

    Ok(())
}

fn set_system_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if app_paths::is_dev_runtime() {
        log::info!("开发预览环境跳过系统开机启动写入");
        return Ok(());
    }

    if enabled {
        app.autolaunch()
            .enable()
            .map_err(|e| format!("开启开机启动失败: {e}"))
    } else {
        app.autolaunch()
            .disable()
            .map_err(|e| format!("关闭开机启动失败: {e}"))
    }
}

fn normalize_codex_launch_path(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        let unquoted = trimmed
            .strip_prefix('"')
            .and_then(|item| item.strip_suffix('"'))
            .or_else(|| {
                trimmed
                    .strip_prefix('\'')
                    .and_then(|item| item.strip_suffix('\''))
            })
            .unwrap_or(trimmed)
            .trim();

        if unquoted.is_empty() {
            None
        } else {
            Some(unquoted.to_string())
        }
    })
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_context_window_k(value: u16) -> u16 {
    value.clamp(CONTEXT_WINDOW_MIN_K, CONTEXT_WINDOW_MAX_K)
}

fn sync_codex_context_window_setting(
    app: &AppHandle,
    store: &crate::models::AccountsStore,
    config_path: &Path,
    context_window_k: Option<u16>,
) -> Result<(), String> {
    apply_codex_context_window_setting(config_path, context_window_k, true)?;

    let store_path = store::account_store_path_from_data_dir(&app_paths::app_data_dir(app)?);
    for account in &store.accounts {
        let profile_config_path =
            profile_files::profile_config_path_from_store_path(&store_path, &account.id);
        apply_codex_context_window_setting(&profile_config_path, context_window_k, false)?;
    }

    Ok(())
}

pub(crate) fn apply_active_codex_context_window_setting(
    context_window_k: Option<u16>,
) -> Result<(), String> {
    let config_path = app_paths::codex_config_path()?;
    apply_codex_context_window_setting(&config_path, context_window_k, true)
}

fn read_codex_context_window_k_from_config(path: &Path) -> Result<Option<u16>, String> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| format!("读取 Codex config.toml 失败 {}: {error}", path.display()))?;
    Ok(parse_codex_context_window_k(&raw))
}

fn parse_codex_context_window_k(raw: &str) -> Option<u16> {
    let mut context_tokens = None;
    let mut compact_tokens = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.split('#').next().unwrap_or_default().trim();
        let Ok(parsed) = value.parse::<u32>() else {
            continue;
        };
        if key == CONTEXT_WINDOW_KEY {
            context_tokens = Some(parsed);
        } else if key == AUTO_COMPACT_LIMIT_KEY {
            compact_tokens = Some(parsed);
        }
    }

    context_tokens
        .or(compact_tokens)
        .and_then(tokens_to_context_window_k)
}

fn read_codex_model_slug_from_config(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| format!("读取 Codex config.toml 失败 {}: {error}", path.display()))?;
    Ok(parse_codex_model_slug(&raw))
}

fn parse_codex_model_slug(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != MODEL_KEY {
            continue;
        }
        let normalized = value
            .split('#')
            .next()
            .unwrap_or_default()
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim();
        if normalized.is_empty() {
            return None;
        }
        return Some(normalized.to_string());
    }

    None
}

fn tokens_to_context_window_k(tokens: u32) -> Option<u16> {
    let normalized = tokens_to_context_window_metadata_k(tokens)?;
    Some(normalize_context_window_k(normalized))
}

fn tokens_to_context_window_metadata_k(tokens: u32) -> Option<u16> {
    if tokens < 1_000 {
        return None;
    }

    Some((tokens / 1_000) as u16)
}

fn read_codex_context_window_limits_for_current_model(
    config_path: &Path,
) -> Result<(Option<String>, CodexContextWindowLimits), String> {
    let Some(model_slug) = read_codex_model_slug_from_config(config_path)? else {
        return Ok((None, CodexContextWindowLimits::default()));
    };

    let limits = read_codex_context_window_limits_from_models_cache(&model_slug)?;
    Ok((Some(model_slug), limits))
}

fn read_codex_context_window_limits_from_models_cache(
    model_slug: &str,
) -> Result<CodexContextWindowLimits, String> {
    let path = app_paths::codex_dir()?.join(MODELS_CACHE_FILE_NAME);
    if !path.exists() {
        return Ok(CodexContextWindowLimits::default());
    }

    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("读取 Codex models cache 失败 {}: {error}", path.display()))?;
    Ok(parse_codex_context_window_limits_from_models_cache(
        &raw, model_slug,
    ))
}

fn parse_codex_context_window_limits_from_models_cache(
    raw: &str,
    model_slug: &str,
) -> CodexContextWindowLimits {
    let Ok(cache) = serde_json::from_str::<CachedModelsFile>(raw) else {
        return CodexContextWindowLimits::default();
    };

    let Some(model) = cache
        .models
        .into_iter()
        .find(|item| item.slug == model_slug)
    else {
        return CodexContextWindowLimits::default();
    };

    let hard_limit_tokens = model.max_context_window.or(model.context_window);
    let hard_limit_k = hard_limit_tokens.and_then(tokens_to_context_window_metadata_k);
    let effective_limit_tokens = match (hard_limit_tokens, model.effective_context_window_percent) {
        (Some(tokens), Some(percent)) if percent > 0 => {
            let scaled = tokens.saturating_mul(u32::from(percent)) / 100;
            Some(scaled.min(tokens))
        }
        (Some(tokens), _) => Some(tokens),
        _ => None,
    };
    let effective_limit_k = effective_limit_tokens
        .and_then(tokens_to_context_window_metadata_k)
        .or(hard_limit_k);

    CodexContextWindowLimits {
        hard_limit_k,
        effective_limit_k,
    }
}

fn populate_codex_context_window_metadata(
    settings: &mut AppSettings,
    model_slug: Option<String>,
    limits: CodexContextWindowLimits,
) {
    settings.codex_context_window_model = model_slug;
    settings.codex_context_window_limit_k = limits.hard_limit_k;
    settings.codex_context_window_effective_limit_k = limits.effective_limit_k;
}

fn apply_codex_context_window_setting(
    path: &Path,
    context_window_k: Option<u16>,
    create_if_missing: bool,
) -> Result<(), String> {
    if !path.exists() && !create_if_missing {
        return Ok(());
    }

    let original =
        if path.exists() {
            Some(fs::read_to_string(path).map_err(|error| {
                format!("读取 Codex config.toml 失败 {}: {error}", path.display())
            })?)
        } else {
            None
        };

    let next = merge_codex_context_window_config(
        original.as_deref(),
        context_window_k.map(normalize_context_window_k),
    );

    if original.as_deref() == Some(next.as_str()) {
        return Ok(());
    }

    if next.is_empty() && !path.exists() {
        return Ok(());
    }

    write_text_file(path, &next)
}

fn merge_codex_context_window_config(raw: Option<&str>, context_window_k: Option<u16>) -> String {
    let mut retained_lines = raw
        .unwrap_or_default()
        .lines()
        .filter(|line| {
            !matches!(
                config_key_for_line(line),
                Some(CONTEXT_WINDOW_KEY | AUTO_COMPACT_LIMIT_KEY | TOOL_OUTPUT_LIMIT_KEY)
            )
        })
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    while retained_lines
        .last()
        .is_some_and(|line| line.trim().is_empty())
    {
        retained_lines.pop();
    }
    while retained_lines
        .first()
        .is_some_and(|line| line.trim().is_empty())
    {
        retained_lines.remove(0);
    }

    if let Some(value_k) = context_window_k {
        let context_tokens = u32::from(value_k) * 1_000;
        let auto_compact_tokens = auto_compact_limit_tokens_for_context_window_k(value_k);
        let insert_index = retained_lines
            .iter()
            .position(|line| matches!(config_key_for_line(line), Some(MODEL_KEY)))
            .map(|index| index + 1)
            .or_else(|| {
                retained_lines.iter().position(|line| {
                    matches!(config_key_for_line(line), Some(MODEL_REASONING_EFFORT_KEY))
                })
            })
            .unwrap_or(retained_lines.len());

        retained_lines.insert(
            insert_index,
            format!("{CONTEXT_WINDOW_KEY} = {context_tokens}"),
        );
        retained_lines.insert(
            insert_index + 1,
            format!("{AUTO_COMPACT_LIMIT_KEY} = {auto_compact_tokens}"),
        );
    }

    if retained_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", retained_lines.join("\n"))
    }
}

fn config_key_for_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('[') {
        return None;
    }

    line.split_once('=').map(|(key, _)| key.trim())
}

fn auto_compact_limit_tokens_for_context_window_k(context_window_k: u16) -> u32 {
    let context_tokens = u32::from(context_window_k) * 1_000;
    let auto_compact_tokens = context_tokens.saturating_mul(AUTO_COMPACT_RATIO_NUMERATOR)
        / AUTO_COMPACT_RATIO_DENOMINATOR;
    auto_compact_tokens.clamp(1_000, context_tokens)
}

fn write_text_file(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析配置目录 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建配置目录失败 {}: {error}", parent.display()))?;

    let target = PathBuf::from(path);
    fs::write(&target, contents)
        .map_err(|error| format!("写入 Codex config.toml 失败 {}: {error}", target.display()))
}

fn normalize_account_key_list(values: Vec<String>, valid_keys: &HashSet<String>) -> Vec<String> {
    let mut result = Vec::new();
    for value in values {
        let Some(normalized) = normalize_optional_string(Some(value)) else {
            continue;
        };
        if !valid_keys.contains(&normalized) {
            continue;
        }
        if !result.iter().any(|existing| existing == &normalized) {
            result.push(normalized);
        }
    }
    result
}

fn normalize_account_pools(
    values: Vec<AccountPoolConfig>,
    accounts: &[crate::models::StoredAccount],
) -> Vec<AccountPoolConfig> {
    let valid_keys = accounts
        .iter()
        .map(|account| account.account_key())
        .collect::<HashSet<_>>();
    let mut seen_ids = HashSet::new();
    let mut result = Vec::new();

    for value in values {
        let Some(id) = normalize_optional_string(Some(value.id)) else {
            continue;
        };
        if seen_ids.contains(&id) {
            continue;
        }
        seen_ids.insert(id.clone());

        let name =
            normalize_optional_string(Some(value.name)).unwrap_or_else(|| "账号池".to_string());
        let account_keys = normalize_account_key_list(value.account_keys, &valid_keys);

        result.push(AccountPoolConfig {
            id,
            name,
            account_keys,
            collapsed: value.collapsed,
        });
    }

    result
}

fn normalize_notification_targets(
    values: Vec<NotificationTargetConfig>,
) -> Vec<NotificationTargetConfig> {
    let mut seen_ids = HashSet::new();
    let mut result = Vec::new();

    for value in values {
        let Some(id) = normalize_optional_string(Some(value.id)) else {
            continue;
        };
        if seen_ids.contains(&id) {
            continue;
        }
        seen_ids.insert(id.clone());

        let name =
            normalize_optional_string(Some(value.name)).unwrap_or_else(|| "通知通道".to_string());
        let telegram_bot_token = normalize_optional_string(value.telegram_bot_token);
        let telegram_chat_id = normalize_optional_string(value.telegram_chat_id);
        let webhook_url = normalize_optional_string(value.webhook_url);
        let provider_ids = normalize_string_id_list(value.provider_ids);

        result.push(NotificationTargetConfig {
            id,
            name,
            kind: value.kind,
            enabled: value.enabled,
            aggregate_enabled: value.aggregate_enabled,
            provider_ids,
            template_preset: value.template_preset,
            message_template: normalize_optional_string(Some(value.message_template))
                .unwrap_or_else(crate::models::default_notification_message_template),
            schedule_date: normalize_optional_string(value.schedule_date),
            schedule_time: normalize_optional_string(value.schedule_time),
            telegram_bot_token,
            telegram_chat_id,
            webhook_url,
            created_at: value.created_at,
            updated_at: value.updated_at,
            last_test_at: value.last_test_at,
            last_test_error: normalize_optional_string(value.last_test_error),
        });
    }

    result
}

fn ensure_notification_settings_shape(settings: &mut AppSettings) -> bool {
    settings.notification_providers =
        normalize_notification_providers(std::mem::take(&mut settings.notification_providers));
    settings.notification_targets =
        normalize_notification_targets(std::mem::take(&mut settings.notification_targets));
    settings.notification_bots =
        normalize_notification_bots(std::mem::take(&mut settings.notification_bots));
    settings.notification_templates =
        normalize_notification_templates(std::mem::take(&mut settings.notification_templates));
    settings.notification_pipelines =
        normalize_notification_pipelines(std::mem::take(&mut settings.notification_pipelines));

    if settings.notification_schema_version > 0 {
        return false;
    }

    settings.notification_schema_version = 1;
    if settings.notification_targets.is_empty() {
        return true;
    }

    if settings.notification_bots.is_empty() {
        settings.notification_bots =
            derive_notification_bots_from_targets(&settings.notification_targets);
    }
    if settings.notification_templates.is_empty() {
        settings.notification_templates =
            derive_notification_templates_from_targets(&settings.notification_targets);
    }
    if settings.notification_pipelines.is_empty() {
        settings.notification_pipelines =
            derive_notification_pipelines_from_targets(&settings.notification_targets);
    }
    true
}

fn normalize_notification_bots(values: Vec<NotificationBotConfig>) -> Vec<NotificationBotConfig> {
    let mut seen_ids = HashSet::new();
    let mut result = Vec::new();

    for value in values {
        let Some(id) = normalize_optional_string(Some(value.id)) else {
            continue;
        };
        if !seen_ids.insert(id.clone()) {
            continue;
        }

        let name =
            normalize_optional_string(Some(value.name)).unwrap_or_else(|| "推送机器人".to_string());

        result.push(NotificationBotConfig {
            id,
            name,
            kind: value.kind,
            enabled: value.enabled,
            telegram_bot_token: normalize_optional_string(value.telegram_bot_token),
            telegram_chat_id: normalize_optional_string(value.telegram_chat_id),
            webhook_url: normalize_optional_string(value.webhook_url),
            created_at: value.created_at,
            updated_at: value.updated_at,
            last_test_at: value.last_test_at,
            last_test_error: normalize_optional_string(value.last_test_error),
        });
    }

    result
}

fn normalize_notification_templates(
    values: Vec<NotificationTemplateConfig>,
) -> Vec<NotificationTemplateConfig> {
    let mut seen_ids = HashSet::new();
    let mut result = Vec::new();

    for value in values {
        let Some(id) = normalize_optional_string(Some(value.id)) else {
            continue;
        };
        if !seen_ids.insert(id.clone()) {
            continue;
        }

        let name =
            normalize_optional_string(Some(value.name)).unwrap_or_else(|| "通知模板".to_string());

        result.push(NotificationTemplateConfig {
            id,
            name,
            preset: value.preset,
            message_template: normalize_optional_string(Some(value.message_template))
                .unwrap_or_else(crate::models::default_notification_message_template),
            created_at: value.created_at,
            updated_at: value.updated_at,
        });
    }

    result
}

fn normalize_notification_pipelines(
    values: Vec<NotificationPipelineConfig>,
) -> Vec<NotificationPipelineConfig> {
    let mut seen_ids = HashSet::new();
    let mut result = Vec::new();

    for value in values {
        let Some(id) = normalize_optional_string(Some(value.id)) else {
            continue;
        };
        if !seen_ids.insert(id.clone()) {
            continue;
        }

        let name =
            normalize_optional_string(Some(value.name)).unwrap_or_else(|| "通知链路".to_string());

        let schedule_date = normalize_optional_string(value.schedule_date);
        let schedule_time = normalize_optional_string(value.schedule_time);
        let schedule_interval_minutes =
            normalize_notification_interval_minutes(value.schedule_interval_minutes);
        let schedule_mode = normalize_notification_schedule_mode(
            value.schedule_mode,
            schedule_date.as_deref(),
            schedule_time.as_deref(),
            schedule_interval_minutes,
        );

        result.push(NotificationPipelineConfig {
            id,
            name,
            enabled: value.enabled,
            aggregate_enabled: value.aggregate_enabled,
            provider_ids: normalize_string_id_list(value.provider_ids),
            bot_ids: normalize_string_id_list(value.bot_ids),
            template_id: normalize_optional_string(value.template_id),
            template_override: normalize_optional_string(value.template_override),
            schedule_mode,
            schedule_date,
            schedule_time,
            schedule_interval_minutes,
            created_at: value.created_at,
            updated_at: value.updated_at,
            last_run_at: value.last_run_at,
            last_test_at: value.last_test_at,
            last_test_error: normalize_optional_string(value.last_test_error),
        });
    }

    result
}

fn derive_notification_bots_from_targets(
    targets: &[NotificationTargetConfig],
) -> Vec<NotificationBotConfig> {
    targets
        .iter()
        .map(|target| NotificationBotConfig {
            id: format!("bot-{}", target.id),
            name: target.name.clone(),
            kind: target.kind.clone(),
            enabled: target.enabled,
            telegram_bot_token: target.telegram_bot_token.clone(),
            telegram_chat_id: target.telegram_chat_id.clone(),
            webhook_url: target.webhook_url.clone(),
            created_at: target.created_at,
            updated_at: target.updated_at,
            last_test_at: target.last_test_at,
            last_test_error: target.last_test_error.clone(),
        })
        .collect()
}

fn derive_notification_templates_from_targets(
    targets: &[NotificationTargetConfig],
) -> Vec<NotificationTemplateConfig> {
    targets
        .iter()
        .map(|target| NotificationTemplateConfig {
            id: format!("template-{}", target.id),
            name: format!("{} 模板", target.name),
            preset: target.template_preset.clone(),
            message_template: target.message_template.clone(),
            created_at: target.created_at,
            updated_at: target.updated_at,
        })
        .collect()
}

fn derive_notification_pipelines_from_targets(
    targets: &[NotificationTargetConfig],
) -> Vec<NotificationPipelineConfig> {
    targets
        .iter()
        .map(|target| NotificationPipelineConfig {
            id: format!("pipeline-{}", target.id),
            name: target.name.clone(),
            enabled: target.enabled,
            aggregate_enabled: target.aggregate_enabled,
            provider_ids: target.provider_ids.clone(),
            bot_ids: vec![format!("bot-{}", target.id)],
            template_id: Some(format!("template-{}", target.id)),
            template_override: None,
            schedule_mode: normalize_notification_schedule_mode(
                NotificationScheduleMode::Manual,
                target.schedule_date.as_deref(),
                target.schedule_time.as_deref(),
                None,
            ),
            schedule_date: target.schedule_date.clone(),
            schedule_time: target.schedule_time.clone(),
            schedule_interval_minutes: None,
            created_at: target.created_at,
            updated_at: target.updated_at,
            last_run_at: None,
            last_test_at: target.last_test_at,
            last_test_error: target.last_test_error.clone(),
        })
        .collect()
}

fn normalize_string_id_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for value in values {
        let Some(id) = normalize_optional_string(Some(value)) else {
            continue;
        };
        if seen.insert(id.clone()) {
            result.push(id);
        }
    }

    result
}

fn normalize_notification_providers(
    values: Vec<NotificationProviderConfig>,
) -> Vec<NotificationProviderConfig> {
    let mut seen_ids = HashSet::new();
    let mut result = Vec::new();

    for value in values {
        let Some(id) = normalize_optional_string(Some(value.id)) else {
            continue;
        };
        if seen_ids.contains(&id) {
            continue;
        }
        seen_ids.insert(id.clone());

        let Some(base_url) = normalize_optional_string(Some(value.base_url)) else {
            continue;
        };
        let Some(email) = normalize_optional_string(Some(value.email)) else {
            continue;
        };
        let name =
            normalize_optional_string(Some(value.name)).unwrap_or_else(|| "API 平台".to_string());

        result.push(NotificationProviderConfig {
            id,
            name,
            kind: value.kind,
            enabled: value.enabled,
            cost_multiplier: normalize_notification_cost_multiplier(value.cost_multiplier),
            base_url: normalize_sub2api_base_url(&base_url),
            email,
            password: normalize_optional_string(value.password),
            created_at: value.created_at,
            updated_at: value.updated_at,
            last_test_at: value.last_test_at,
            last_test_error: normalize_optional_string(value.last_test_error),
        });
    }

    result
}

fn normalize_notification_cost_multiplier(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value.clamp(0.0001, 1000.0)
    } else {
        crate::models::default_notification_cost_multiplier()
    }
}

fn normalize_sub2api_base_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    trimmed.to_string()
}

fn normalize_codex_launch_path_for_storage(
    value: Option<String>,
) -> Result<Option<String>, String> {
    let normalized = normalize_codex_launch_path(value);
    if normalized
        .as_deref()
        .is_some_and(should_discard_codex_launch_path)
    {
        return Ok(None);
    }

    cli::validate_configured_codex_path(normalized.as_deref())?;
    Ok(normalized)
}

fn should_discard_codex_launch_path(path: &str) -> bool {
    cli::is_windows_store_codex_path(std::path::Path::new(path))
        && cli::has_windows_store_codex_app()
}

#[cfg(test)]
mod tests {
    use super::auto_compact_limit_tokens_for_context_window_k;
    use super::merge_codex_context_window_config;
    use super::parse_codex_context_window_limits_from_models_cache;
    use super::parse_codex_model_slug;

    #[test]
    fn parse_codex_model_slug_reads_top_level_model_key() {
        let raw = r#"
model = "gpt-5.4"
model_reasoning_effort = "xhigh"
"#;

        assert_eq!(parse_codex_model_slug(raw).as_deref(), Some("gpt-5.4"));
    }

    #[test]
    fn parse_models_cache_uses_effective_percent_limit() {
        let raw = r#"
{
  "models": [
    {
      "slug": "gpt-5.4",
      "context_window": 272000,
      "max_context_window": 272000,
      "effective_context_window_percent": 95
    }
  ]
}
"#;

        let limits = parse_codex_context_window_limits_from_models_cache(raw, "gpt-5.4");

        assert_eq!(limits.hard_limit_k, Some(272));
        assert_eq!(limits.effective_limit_k, Some(258));
    }

    #[test]
    fn auto_compact_limit_uses_configured_ratio() {
        assert_eq!(auto_compact_limit_tokens_for_context_window_k(400), 380_000);
        assert_eq!(auto_compact_limit_tokens_for_context_window_k(272), 258_400);
    }

    #[test]
    fn merge_context_window_inserts_below_model_before_reasoning_effort() {
        let raw = r#"
model = "gpt-5.4"
model_reasoning_effort = "xhigh"
openai_base_url = "https://api.example.com/v1"
[windows]
sandbox = "elevated"
"#;

        let merged = merge_codex_context_window_config(Some(raw), Some(400));

        assert_eq!(
            merged,
            r#"model = "gpt-5.4"
model_context_window = 400000
model_auto_compact_token_limit = 380000
model_reasoning_effort = "xhigh"
openai_base_url = "https://api.example.com/v1"
[windows]
sandbox = "elevated"
"#
        );
    }

    #[test]
    fn merge_context_window_removes_existing_tool_output_limit() {
        let raw = r#"
model = "gpt-5.4"
tool_output_token_limit = 50000
model_reasoning_effort = "xhigh"
"#;

        let merged = merge_codex_context_window_config(Some(raw), Some(400));

        assert_eq!(
            merged,
            r#"model = "gpt-5.4"
model_context_window = 400000
model_auto_compact_token_limit = 380000
model_reasoning_effort = "xhigh"
"#
        );
    }

    #[test]
    fn merge_context_window_repairs_missing_compact_and_tool_output_when_context_exists() {
        let raw = r#"
model = "gpt-5.4"
model_context_window = 400000
model_reasoning_effort = "xhigh"
"#;

        let merged = merge_codex_context_window_config(Some(raw), Some(400));

        assert_eq!(
            merged,
            r#"model = "gpt-5.4"
model_context_window = 400000
model_auto_compact_token_limit = 380000
model_reasoning_effort = "xhigh"
"#
        );
    }

    #[test]
    fn merge_context_window_removes_tool_output_limit_for_subsequent_lines() {
        let raw = r#"
model = "gpt-5.4"
tool_output_token_limit = 100000
model_reasoning_effort = "xhigh"
"#;

        let merged = merge_codex_context_window_config(Some(raw), Some(400));

        assert_eq!(
            merged,
            r#"model = "gpt-5.4"
model_context_window = 400000
model_auto_compact_token_limit = 380000
model_reasoning_effort = "xhigh"
"#
        );
    }

    #[test]
    fn merge_context_window_default_removes_all_managed_lines() {
        let raw = r#"
model = "gpt-5.4"
model_context_window = 400000
model_auto_compact_token_limit = 380000
tool_output_token_limit = 100000
model_reasoning_effort = "xhigh"
"#;

        let merged = merge_codex_context_window_config(Some(raw), None);

        assert_eq!(
            merged,
            r#"model = "gpt-5.4"
model_reasoning_effort = "xhigh"
"#
        );
    }
}
