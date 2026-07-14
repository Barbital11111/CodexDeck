use std::collections::HashSet;
use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt as _;

use crate::app_paths;
use crate::cli;
use crate::codex_multimodel;
use crate::models::AccountPoolConfig;
use crate::models::AppSettings;
use crate::models::AppSettingsPatch;
use crate::models::ModelRouterRouteSelection;
use crate::models::NotificationBotConfig;
use crate::models::NotificationPipelineConfig;
use crate::models::NotificationProviderConfig;
use crate::models::NotificationScheduleMode;
use crate::models::NotificationTargetConfig;
use crate::models::NotificationTemplateConfig;
use crate::state::AppState;
use crate::store::load_store;
use crate::store::load_store_read_only;
use crate::store::save_store;

pub(crate) const DEV_CONTROLLED_CODEX_LAUNCH_PATH_ENV: &str =
    "CODEXDECK_DEV_CONTROLLED_CODEX_LAUNCH_PATH";

pub(crate) fn dev_controlled_codex_launch_path() -> Option<String> {
    std::env::var(DEV_CONTROLLED_CODEX_LAUNCH_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

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

/// 读取应用设置（前端设置页使用）。
pub(crate) async fn get_app_settings_internal(
    app: &AppHandle,
    state: &AppState,
) -> Result<AppSettings, String> {
    let _guard = state.store_lock.lock().await;
    let store = load_store_read_only(app)?;
    Ok(store.settings.clone())
}

pub(crate) fn reconcile_startup_settings(app: &AppHandle) -> Result<(), String> {
    let mut store = load_store_read_only(app)?;
    let before_settings = store.settings.clone();
    reconcile_startup_settings_in_place(app, &mut store.settings)?;
    if store.settings != before_settings {
        save_store(app, &store)?;
    }
    Ok(())
}

fn reconcile_startup_settings_in_place(
    app: &AppHandle,
    settings: &mut AppSettings,
) -> Result<(), String> {
    ensure_notification_settings_shape(settings);
    if settings
        .codex_launch_path
        .as_deref()
        .is_some_and(should_discard_codex_launch_path)
    {
        settings.codex_launch_path = None;
    }
    settings.api_enhanced_launch_enabled = false;
    codex_multimodel::reconcile_settings_state(app, settings);
    if settings.codex_model_instructions_fix_enabled {
        crate::profile_files::apply_model_instructions_fix_setting(true)?;
    } else {
        crate::profile_files::clear_model_instructions_fix_setting()?;
    }
    Ok(())
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
        if patch.api_enhanced_launch_enabled.is_some() {
            store.settings.api_enhanced_launch_enabled = false;
        }
        if let Some(value) = patch.codex_multi_model_mode_enabled {
            store.settings.codex_multi_model_mode_enabled = value;
        }
        if let Some(value) = patch.codex_model_instructions_fix_enabled {
            store.settings.codex_model_instructions_fix_enabled = value;
            if value {
                crate::profile_files::apply_model_instructions_fix_setting(true)?;
            } else {
                crate::profile_files::clear_model_instructions_fix_setting()?;
            }
        }
        if let Some(value) = patch.codex_disable_gpu_acceleration {
            store.settings.codex_disable_gpu_acceleration = value;
        }
        if let Some(value) = patch.codex_multi_model_status {
            store.settings.codex_multi_model_status = normalize_optional_string(value);
        }
        if let Some(value) = patch.codex_multi_model_workspace {
            store.settings.codex_multi_model_workspace = normalize_optional_string(value);
        }
        if let Some(value) = patch.codex_multi_model_restore_point {
            store.settings.codex_multi_model_restore_point = normalize_optional_string(value);
        }
        if let Some(value) = patch.codex_multi_model_controlled_app_root {
            store.settings.codex_multi_model_controlled_app_root = normalize_optional_string(value);
        }
        if let Some(value) = patch.codex_multi_model_controlled_exe_path {
            store.settings.codex_multi_model_controlled_exe_path = normalize_optional_string(value);
        }
        if let Some(value) = patch.codex_multi_model_controlled_app_asar_path {
            store.settings.codex_multi_model_controlled_app_asar_path =
                normalize_optional_string(value);
        }
        if let Some(value) = patch.codex_multi_model_source_app_root {
            store.settings.codex_multi_model_source_app_root = normalize_optional_string(value);
        }
        if let Some(value) = patch.codex_multi_model_patch_state_path {
            store.settings.codex_multi_model_patch_state_path = normalize_optional_string(value);
        }
        if let Some(value) = patch.model_router_enabled {
            store.settings.model_router_enabled = value;
        }
        if let Some(value) = patch.model_router_account_id {
            store.settings.model_router_account_id = value;
        }
        if let Some(value) = patch.model_router_route_selections {
            store.settings.model_router_route_selections =
                normalize_model_router_route_selections(value, &store.accounts);
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
        if let Some(value) = patch.account_card_order {
            store.settings.account_card_order =
                normalize_account_card_order(value, &store.accounts);
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

        let mut settings = store.settings.clone();
        ensure_notification_settings_shape(&mut settings);
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

    let settings = load_store_read_only(app)?.settings;
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

fn normalize_model_router_route_selections(
    values: Vec<ModelRouterRouteSelection>,
    accounts: &[crate::models::StoredAccount],
) -> Vec<ModelRouterRouteSelection> {
    let mut valid_routes = HashSet::new();
    for account in accounts {
        if !matches!(account.source_kind, crate::models::AccountSourceKind::Relay) {
            continue;
        }
        for entry in account.enabled_model_catalog() {
            valid_routes.insert((account.id.clone(), entry.model));
        }
    }

    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let Some(account_id) = normalize_optional_string(Some(value.account_id)) else {
            continue;
        };
        let Some(model) = normalize_optional_string(Some(value.model)) else {
            continue;
        };
        let key = (account_id.clone(), model.clone());
        if !valid_routes.contains(&key) || !seen.insert(key) {
            continue;
        }
        result.push(ModelRouterRouteSelection { account_id, model });
    }
    result
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

fn normalize_account_card_order(
    values: Vec<String>,
    accounts: &[crate::models::StoredAccount],
) -> Vec<String> {
    let valid_keys = accounts
        .iter()
        .map(|account| account.account_key())
        .collect::<HashSet<_>>();
    normalize_account_key_list(values, &valid_keys)
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
            account_key: normalize_optional_string(value.account_key),
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
    if is_legacy_controlled_codex_launch_path(path) {
        return true;
    }

    cli::is_windows_store_codex_path(std::path::Path::new(path))
        && cli::has_windows_store_codex_app()
}

fn is_legacy_controlled_codex_launch_path(path: &str) -> bool {
    let normalized = path.replace('/', "\\").to_ascii_lowercase();
    normalized.contains("\\codexdeck-multimodel\\controlled-codex\\")
}

#[cfg(test)]
mod tests {}
