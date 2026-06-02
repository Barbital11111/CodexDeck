use std::collections::HashMap;
use std::collections::HashSet;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::auth::account_group_key;
use crate::auth::account_variant_key;
use crate::auth::extract_auth;

fn default_usage_auto_refresh_enabled() -> bool {
    true
}

fn default_usage_auto_refresh_interval_secs() -> u16 {
    30
}

fn default_api_quota_auto_refresh_enabled() -> bool {
    true
}

fn default_api_quota_auto_refresh_interval_secs() -> u16 {
    600
}

fn default_quota_alert_enabled() -> bool {
    true
}

fn default_show_provider_badge() -> bool {
    false
}

fn default_codex_context_window_k() -> Option<u16> {
    None
}

fn default_quota_alert_five_hour_threshold() -> u8 {
    15
}

fn default_quota_alert_one_week_threshold() -> u8 {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AccountsStore {
    #[serde(default = "default_store_version")]
    pub(crate) version: u8,
    #[serde(default)]
    pub(crate) accounts: Vec<StoredAccount>,
    #[serde(default, rename = "proxyUpstreams")]
    pub(crate) proxy_upstreams: Vec<ProxyUpstream>,
    #[serde(default, rename = "proxyRouteBindings")]
    pub(crate) proxy_route_bindings: Vec<ProxyRouteBinding>,
    #[serde(default)]
    pub(crate) settings: AppSettings,
}

fn default_store_version() -> u8 {
    2
}

impl Default for AccountsStore {
    fn default() -> Self {
        Self {
            version: default_store_version(),
            accounts: Vec::new(),
            proxy_upstreams: Vec::new(),
            proxy_route_bindings: Vec::new(),
            settings: AppSettings::default(),
        }
    }
}

impl AccountsStore {
    pub(crate) fn sync_proxy_upstream_snapshot(&mut self) -> bool {
        let mut changed = false;
        for account in &mut self.accounts {
            if account.sync_relay_api_keys_from_legacy() {
                changed = true;
            }
        }

        let next_upstreams = self
            .accounts
            .iter()
            .filter_map(StoredAccount::to_proxy_upstream)
            .map(ProxyUpstream::without_secrets)
            .collect::<Vec<_>>();

        let valid_upstream_ids = next_upstreams
            .iter()
            .map(|upstream| upstream.id.clone())
            .collect::<HashSet<_>>();
        let valid_channel_ids = next_upstreams
            .iter()
            .flat_map(|upstream| upstream.channels.iter().map(|channel| channel.id.clone()))
            .collect::<HashSet<_>>();

        if self.proxy_upstreams != next_upstreams {
            self.proxy_upstreams = next_upstreams;
            changed = true;
        }

        let before_bindings = self.proxy_route_bindings.len();
        self.proxy_route_bindings.retain(|binding| {
            valid_upstream_ids.contains(&binding.upstream_id)
                && valid_channel_ids.contains(&binding.channel_id)
        });
        changed || before_bindings != self.proxy_route_bindings.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AccountSourceKind {
    Chatgpt,
    Relay,
}

impl Default for AccountSourceKind {
    fn default() -> Self {
        Self::Chatgpt
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ProxyKeySelectionMode {
    #[default]
    Random,
    RoundRobin,
    FixedPriority,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProxyHealthStatus {
    #[default]
    Healthy,
    Degraded,
    CoolingDown,
    Disabled,
    AuthFailed,
    QuotaExhausted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ProxyUpstreamKind {
    #[default]
    Chatgpt,
    RelayApi,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ApiQuotaMode {
    #[default]
    ApiOnly,
    PlatformBasic,
    PlatformSubscription,
    Admin,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ProxyEndpointCapability {
    #[default]
    Responses,
    ResponsesCompact,
    ChatCompletions,
    Realtime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyKey {
    pub(crate) id: String,
    pub(crate) label: Option<String>,
    #[serde(default)]
    pub(crate) secret: Option<String>,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) priority: i32,
    #[serde(default)]
    pub(crate) weight: u32,
    #[serde(default)]
    pub(crate) health_status: ProxyHealthStatus,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
    #[serde(default)]
    pub(crate) cooldown_until: Option<i64>,
    #[serde(default)]
    pub(crate) failure_count: u32,
    #[serde(default)]
    pub(crate) last_used_at: Option<i64>,
    #[serde(default)]
    pub(crate) updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyChannel {
    pub(crate) id: String,
    pub(crate) upstream_id: String,
    pub(crate) source_account_id: String,
    pub(crate) source_account_key: String,
    pub(crate) source_label: String,
    pub(crate) upstream_kind: ProxyUpstreamKind,
    #[serde(default)]
    pub(crate) base_url: Option<String>,
    #[serde(default)]
    pub(crate) model_name: Option<String>,
    #[serde(default)]
    pub(crate) plan_type: Option<String>,
    #[serde(default)]
    pub(crate) priority: i32,
    #[serde(default)]
    pub(crate) weight: u32,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) key_selection_mode: ProxyKeySelectionMode,
    #[serde(default)]
    pub(crate) health_status: ProxyHealthStatus,
    #[serde(default)]
    pub(crate) cooldown_until: Option<i64>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) endpoints: Vec<ProxyEndpointCapability>,
    #[serde(default)]
    pub(crate) keys: Vec<ProxyKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyRouteBinding {
    pub(crate) binding_key: String,
    pub(crate) upstream_id: String,
    pub(crate) channel_id: String,
    pub(crate) source_account_id: String,
    pub(crate) source_label: String,
    #[serde(default)]
    pub(crate) model_name: Option<String>,
    #[serde(default)]
    pub(crate) last_seen_at: Option<i64>,
    #[serde(default)]
    pub(crate) expires_at: Option<i64>,
    #[serde(default)]
    pub(crate) manual: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyUpstream {
    pub(crate) id: String,
    pub(crate) source_account_id: String,
    pub(crate) source_account_key: String,
    pub(crate) source_label: String,
    pub(crate) upstream_kind: ProxyUpstreamKind,
    #[serde(default)]
    pub(crate) provider_id: Option<String>,
    #[serde(default)]
    pub(crate) provider_name: Option<String>,
    #[serde(default)]
    pub(crate) plan_type: Option<String>,
    #[serde(default)]
    pub(crate) priority: i32,
    #[serde(default)]
    pub(crate) weight: u32,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) privacy_hidden: bool,
    #[serde(default)]
    pub(crate) health_status: ProxyHealthStatus,
    #[serde(default)]
    pub(crate) cooldown_until: Option<i64>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) channels: Vec<ProxyChannel>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) updated_at: i64,
}

impl ProxyUpstream {
    pub(crate) fn without_secrets(mut self) -> Self {
        for channel in &mut self.channels {
            for key in &mut channel.keys {
                key.secret = None;
            }
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredAccount {
    pub(crate) id: String,
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) source_kind: AccountSourceKind,
    #[serde(default)]
    pub(crate) principal_id: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) account_id: String,
    pub(crate) plan_type: Option<String>,
    pub(crate) auth_json: Value,
    #[serde(default)]
    pub(crate) api_base_url: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) api_keys: Vec<ProxyKey>,
    #[serde(default)]
    pub(crate) proxy_priority: Option<i32>,
    #[serde(default)]
    pub(crate) proxy_weight: Option<u32>,
    #[serde(default)]
    pub(crate) proxy_key_selection_mode: Option<ProxyKeySelectionMode>,
    #[serde(default)]
    pub(crate) proxy_endpoints: Vec<ProxyEndpointCapability>,
    #[serde(default)]
    pub(crate) model_name: Option<String>,
    #[serde(default)]
    pub(crate) balance_text: Option<String>,
    #[serde(default)]
    pub(crate) balance_display_enabled: bool,
    #[serde(default)]
    pub(crate) api_quota_mode: ApiQuotaMode,
    #[serde(default)]
    pub(crate) api_quota_today_used_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_remaining_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_total_remaining_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_total_tokens_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_today_tokens_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_daily_window: Option<UsageWindow>,
    #[serde(default)]
    pub(crate) api_quota_total_window: Option<UsageWindow>,
    #[serde(default)]
    pub(crate) api_quota_subscription_expires_at: Option<i64>,
    #[serde(default)]
    pub(crate) provider_id: Option<String>,
    #[serde(default)]
    pub(crate) provider_name: Option<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) profile_auth_path: Option<String>,
    #[serde(default)]
    pub(crate) profile_config_path: Option<String>,
    #[serde(default)]
    pub(crate) profile_auth_ready: bool,
    #[serde(default)]
    pub(crate) profile_config_ready: bool,
    #[serde(default)]
    pub(crate) profile_integrity_error: Option<String>,
    #[serde(default)]
    pub(crate) profile_last_validated_at: Option<i64>,
    #[serde(default)]
    pub(crate) profile_last_validation_error: Option<String>,
    pub(crate) added_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) usage: Option<UsageSnapshot>,
    pub(crate) usage_error: Option<String>,
    #[serde(default)]
    pub(crate) auth_refresh_blocked: bool,
    #[serde(default)]
    pub(crate) auth_refresh_error: Option<String>,
    #[serde(default)]
    pub(crate) auth_refresh_next_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountSummary {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) source_kind: AccountSourceKind,
    pub(crate) email: Option<String>,
    pub(crate) account_key: String,
    pub(crate) account_id: String,
    pub(crate) plan_type: Option<String>,
    pub(crate) api_base_url: Option<String>,
    pub(crate) model_name: Option<String>,
    pub(crate) balance_text: Option<String>,
    pub(crate) balance_display_enabled: bool,
    pub(crate) api_quota_mode: ApiQuotaMode,
    pub(crate) api_quota_today_used_text: Option<String>,
    pub(crate) api_quota_remaining_text: Option<String>,
    pub(crate) api_quota_total_remaining_text: Option<String>,
    pub(crate) api_quota_total_tokens_text: Option<String>,
    pub(crate) api_quota_today_tokens_text: Option<String>,
    pub(crate) api_quota_daily_window: Option<UsageWindow>,
    pub(crate) api_quota_total_window: Option<UsageWindow>,
    pub(crate) api_quota_subscription_expires_at: Option<i64>,
    pub(crate) provider_id: Option<String>,
    pub(crate) provider_name: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) profile_auth_ready: bool,
    pub(crate) profile_config_ready: bool,
    pub(crate) profile_integrity_error: Option<String>,
    pub(crate) profile_last_validated_at: Option<i64>,
    pub(crate) profile_last_validation_error: Option<String>,
    pub(crate) added_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) usage: Option<UsageSnapshot>,
    pub(crate) usage_error: Option<String>,
    pub(crate) auth_refresh_blocked: bool,
    pub(crate) auth_refresh_error: Option<String>,
    pub(crate) auth_refresh_next_at: Option<i64>,
    pub(crate) is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageSnapshot {
    pub(crate) fetched_at: i64,
    pub(crate) plan_type: Option<String>,
    pub(crate) five_hour: Option<UsageWindow>,
    pub(crate) one_week: Option<UsageWindow>,
    pub(crate) credits: Option<CreditSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageWindow {
    pub(crate) used_percent: f64,
    pub(crate) window_seconds: i64,
    pub(crate) reset_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreditSnapshot {
    pub(crate) has_credits: bool,
    pub(crate) unlimited: bool,
    pub(crate) balance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SwitchAccountResult {
    pub(crate) account_id: String,
    pub(crate) launched_app_path: Option<String>,
    pub(crate) used_fallback_cli: bool,
    pub(crate) opencode_synced: bool,
    pub(crate) opencode_sync_error: Option<String>,
    pub(crate) opencode_desktop_restarted: bool,
    pub(crate) opencode_desktop_restart_error: Option<String>,
    pub(crate) restarted_editor_apps: Vec<EditorAppId>,
    pub(crate) editor_restart_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedOauthLogin {
    pub(crate) auth_url: String,
    pub(crate) redirect_uri: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ExtractedAuth {
    pub(crate) principal_id: String,
    pub(crate) account_id: String,
    pub(crate) access_token: String,
    pub(crate) email: Option<String>,
    pub(crate) plan_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthJsonImportInput {
    pub(crate) source: String,
    pub(crate) content: String,
    pub(crate) label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateApiAccountInput {
    pub(crate) label: String,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) model_name: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) force_save: bool,
    #[serde(default)]
    pub(crate) balance_display_enabled: bool,
    #[serde(default)]
    pub(crate) api_quota_mode: ApiQuotaMode,
    #[serde(default)]
    pub(crate) api_quota_today_used_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_remaining_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_total_remaining_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_total_tokens_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_today_tokens_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_daily_window: Option<UsageWindow>,
    #[serde(default)]
    pub(crate) api_quota_total_window: Option<UsageWindow>,
    #[serde(default)]
    pub(crate) api_quota_subscription_expires_at: Option<i64>,
    #[serde(default)]
    pub(crate) platform_login_email: Option<String>,
    #[serde(default)]
    pub(crate) platform_login_password: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateApiAccountInput {
    pub(crate) label: String,
    pub(crate) base_url: String,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    pub(crate) model_name: String,
    #[serde(default)]
    pub(crate) balance_display_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) api_quota_mode: ApiQuotaMode,
    #[serde(default)]
    pub(crate) api_quota_today_used_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_remaining_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_total_remaining_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_total_tokens_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_today_tokens_text: Option<String>,
    #[serde(default)]
    pub(crate) api_quota_daily_window: Option<UsageWindow>,
    #[serde(default)]
    pub(crate) api_quota_total_window: Option<UsageWindow>,
    #[serde(default)]
    pub(crate) api_quota_subscription_expires_at: Option<i64>,
    #[serde(default)]
    pub(crate) platform_login_email: Option<String>,
    #[serde(default)]
    pub(crate) platform_login_password: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateApiAccountKeyInput {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) label: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) priority: i32,
    #[serde(default)]
    pub(crate) weight: u32,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportAccountFailure {
    pub(crate) source: String,
    pub(crate) error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportAccountsResult {
    pub(crate) total_count: usize,
    pub(crate) imported_count: usize,
    pub(crate) updated_count: usize,
    pub(crate) failures: Vec<ImportAccountFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthCallbackFinishedEvent {
    pub(crate) result: Option<ImportAccountsResult>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountPoolConfig {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) account_keys: Vec<String>,
    #[serde(default)]
    pub(crate) collapsed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NotificationTargetKind {
    Telegram,
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NotificationProviderKind {
    Sub2api,
}

impl Default for NotificationProviderKind {
    fn default() -> Self {
        Self::Sub2api
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NotificationTemplatePreset {
    Test,
    UsageReport,
    QuotaLow,
    QuotaRecovered,
    AccountError,
}

impl Default for NotificationTemplatePreset {
    fn default() -> Self {
        Self::Test
    }
}

pub(crate) fn default_notification_message_template() -> String {
    "CodexDeck 通知测试成功。\n通道：{target}\n时间：{time}".to_string()
}

pub(crate) fn default_notification_cost_multiplier() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NotificationScheduleMode {
    #[default]
    Manual,
    Daily,
    Interval,
    Date,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationProviderConfig {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) kind: NotificationProviderKind,
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default = "default_notification_cost_multiplier")]
    pub(crate) cost_multiplier: f64,
    pub(crate) base_url: String,
    pub(crate) email: String,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) created_at: i64,
    #[serde(default)]
    pub(crate) updated_at: i64,
    #[serde(default)]
    pub(crate) last_test_at: Option<i64>,
    #[serde(default)]
    pub(crate) last_test_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationTargetConfig {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: NotificationTargetKind,
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) aggregate_enabled: bool,
    #[serde(default)]
    pub(crate) provider_ids: Vec<String>,
    #[serde(default)]
    pub(crate) template_preset: NotificationTemplatePreset,
    #[serde(default = "default_notification_message_template")]
    pub(crate) message_template: String,
    #[serde(default)]
    pub(crate) schedule_date: Option<String>,
    #[serde(default)]
    pub(crate) schedule_time: Option<String>,
    #[serde(default)]
    pub(crate) telegram_bot_token: Option<String>,
    #[serde(default)]
    pub(crate) telegram_chat_id: Option<String>,
    #[serde(default)]
    pub(crate) webhook_url: Option<String>,
    #[serde(default)]
    pub(crate) created_at: i64,
    #[serde(default)]
    pub(crate) updated_at: i64,
    #[serde(default)]
    pub(crate) last_test_at: Option<i64>,
    #[serde(default)]
    pub(crate) last_test_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationBotConfig {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: NotificationTargetKind,
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) telegram_bot_token: Option<String>,
    #[serde(default)]
    pub(crate) telegram_chat_id: Option<String>,
    #[serde(default)]
    pub(crate) webhook_url: Option<String>,
    #[serde(default)]
    pub(crate) created_at: i64,
    #[serde(default)]
    pub(crate) updated_at: i64,
    #[serde(default)]
    pub(crate) last_test_at: Option<i64>,
    #[serde(default)]
    pub(crate) last_test_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationTemplateConfig {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) preset: NotificationTemplatePreset,
    #[serde(default = "default_notification_message_template")]
    pub(crate) message_template: String,
    #[serde(default)]
    pub(crate) created_at: i64,
    #[serde(default)]
    pub(crate) updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationPipelineConfig {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) aggregate_enabled: bool,
    #[serde(default)]
    pub(crate) provider_ids: Vec<String>,
    #[serde(default)]
    pub(crate) bot_ids: Vec<String>,
    #[serde(default)]
    pub(crate) template_id: Option<String>,
    #[serde(default)]
    pub(crate) template_override: Option<String>,
    #[serde(default)]
    pub(crate) schedule_mode: NotificationScheduleMode,
    #[serde(default)]
    pub(crate) schedule_date: Option<String>,
    #[serde(default)]
    pub(crate) schedule_time: Option<String>,
    #[serde(default)]
    pub(crate) schedule_interval_minutes: Option<u16>,
    #[serde(default)]
    pub(crate) created_at: i64,
    #[serde(default)]
    pub(crate) updated_at: i64,
    #[serde(default)]
    pub(crate) last_run_at: Option<i64>,
    #[serde(default)]
    pub(crate) last_test_at: Option<i64>,
    #[serde(default)]
    pub(crate) last_test_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TrayUsageDisplayMode {
    Used,
    Hidden,
    #[default]
    Remaining,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub(crate) enum EditorAppId {
    Vscode,
    VscodeInsiders,
    Cursor,
    Antigravity,
    Kiro,
    Trae,
    Qoder,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) enum AppLocale {
    #[default]
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en-US")]
    EnUs,
    #[serde(rename = "ja-JP")]
    JaJp,
    #[serde(rename = "ko-KR")]
    KoKr,
    #[serde(rename = "ru-RU")]
    RuRu,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledEditorApp {
    pub(crate) id: EditorAppId,
    pub(crate) label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActiveHybridProfile {
    pub(crate) chatgpt_account_id: String,
    pub(crate) relay_account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AppSettings {
    pub(crate) launch_at_startup: bool,
    pub(crate) tray_usage_display_mode: TrayUsageDisplayMode,
    pub(crate) launch_codex_after_switch: bool,
    #[serde(default)]
    pub(crate) smart_switch_include_api: bool,
    #[serde(default)]
    pub(crate) api_enhanced_launch_enabled: bool,
    #[serde(default = "default_usage_auto_refresh_enabled")]
    pub(crate) usage_auto_refresh_enabled: bool,
    #[serde(default = "default_usage_auto_refresh_interval_secs")]
    pub(crate) usage_auto_refresh_interval_secs: u16,
    #[serde(default = "default_api_quota_auto_refresh_enabled")]
    pub(crate) api_quota_auto_refresh_enabled: bool,
    #[serde(default = "default_api_quota_auto_refresh_interval_secs")]
    pub(crate) api_quota_auto_refresh_interval_secs: u16,
    #[serde(default = "default_quota_alert_enabled")]
    pub(crate) quota_alert_enabled: bool,
    #[serde(default = "default_show_provider_badge")]
    pub(crate) show_provider_badge: bool,
    #[serde(default = "default_codex_context_window_k")]
    pub(crate) codex_context_window_k: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) codex_context_window_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) codex_context_window_limit_k: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) codex_context_window_effective_limit_k: Option<u16>,
    #[serde(default = "default_quota_alert_five_hour_threshold")]
    pub(crate) quota_alert_five_hour_threshold: u8,
    #[serde(default = "default_quota_alert_one_week_threshold")]
    pub(crate) quota_alert_one_week_threshold: u8,
    pub(crate) codex_launch_path: Option<String>,
    #[serde(default)]
    pub(crate) active_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) active_hybrid_profile: Option<ActiveHybridProfile>,
    pub(crate) sync_opencode_openai_auth: bool,
    pub(crate) restart_opencode_desktop_on_switch: bool,
    pub(crate) restart_editors_on_switch: bool,
    pub(crate) restart_editor_targets: Vec<EditorAppId>,
    #[serde(default)]
    pub(crate) account_pools: Vec<AccountPoolConfig>,
    #[serde(default)]
    pub(crate) notification_providers: Vec<NotificationProviderConfig>,
    #[serde(default)]
    pub(crate) notification_targets: Vec<NotificationTargetConfig>,
    #[serde(default)]
    pub(crate) notification_bots: Vec<NotificationBotConfig>,
    #[serde(default)]
    pub(crate) notification_templates: Vec<NotificationTemplateConfig>,
    #[serde(default)]
    pub(crate) notification_pipelines: Vec<NotificationPipelineConfig>,
    #[serde(default)]
    pub(crate) notification_schema_version: u8,
    pub(crate) locale: AppLocale,
    pub(crate) skipped_update_version: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            launch_at_startup: false,
            tray_usage_display_mode: TrayUsageDisplayMode::Remaining,
            launch_codex_after_switch: true,
            smart_switch_include_api: false,
            api_enhanced_launch_enabled: false,
            usage_auto_refresh_enabled: default_usage_auto_refresh_enabled(),
            usage_auto_refresh_interval_secs: default_usage_auto_refresh_interval_secs(),
            api_quota_auto_refresh_enabled: default_api_quota_auto_refresh_enabled(),
            api_quota_auto_refresh_interval_secs: default_api_quota_auto_refresh_interval_secs(),
            quota_alert_enabled: default_quota_alert_enabled(),
            show_provider_badge: default_show_provider_badge(),
            codex_context_window_k: default_codex_context_window_k(),
            codex_context_window_model: None,
            codex_context_window_limit_k: None,
            codex_context_window_effective_limit_k: None,
            quota_alert_five_hour_threshold: default_quota_alert_five_hour_threshold(),
            quota_alert_one_week_threshold: default_quota_alert_one_week_threshold(),
            codex_launch_path: None,
            active_account_id: None,
            active_hybrid_profile: None,
            sync_opencode_openai_auth: false,
            restart_opencode_desktop_on_switch: false,
            restart_editors_on_switch: false,
            restart_editor_targets: Vec::new(),
            account_pools: Vec::new(),
            notification_providers: Vec::new(),
            notification_targets: Vec::new(),
            notification_bots: Vec::new(),
            notification_templates: Vec::new(),
            notification_pipelines: Vec::new(),
            notification_schema_version: 0,
            locale: AppLocale::default(),
            skipped_update_version: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSettingsPatch {
    pub(crate) launch_at_startup: Option<bool>,
    pub(crate) tray_usage_display_mode: Option<TrayUsageDisplayMode>,
    pub(crate) launch_codex_after_switch: Option<bool>,
    pub(crate) smart_switch_include_api: Option<bool>,
    pub(crate) api_enhanced_launch_enabled: Option<bool>,
    pub(crate) usage_auto_refresh_enabled: Option<bool>,
    pub(crate) usage_auto_refresh_interval_secs: Option<u16>,
    pub(crate) api_quota_auto_refresh_enabled: Option<bool>,
    pub(crate) api_quota_auto_refresh_interval_secs: Option<u16>,
    pub(crate) quota_alert_enabled: Option<bool>,
    pub(crate) show_provider_badge: Option<bool>,
    pub(crate) codex_context_window_k: Option<Option<u16>>,
    pub(crate) quota_alert_five_hour_threshold: Option<u8>,
    pub(crate) quota_alert_one_week_threshold: Option<u8>,
    pub(crate) codex_launch_path: Option<Option<String>>,
    pub(crate) sync_opencode_openai_auth: Option<bool>,
    pub(crate) restart_opencode_desktop_on_switch: Option<bool>,
    pub(crate) restart_editors_on_switch: Option<bool>,
    pub(crate) restart_editor_targets: Option<Vec<EditorAppId>>,
    pub(crate) account_pools: Option<Vec<AccountPoolConfig>>,
    pub(crate) notification_providers: Option<Vec<NotificationProviderConfig>>,
    pub(crate) notification_targets: Option<Vec<NotificationTargetConfig>>,
    pub(crate) notification_bots: Option<Vec<NotificationBotConfig>>,
    pub(crate) notification_templates: Option<Vec<NotificationTemplateConfig>>,
    pub(crate) notification_pipelines: Option<Vec<NotificationPipelineConfig>>,
    pub(crate) notification_schema_version: Option<u8>,
    pub(crate) locale: Option<AppLocale>,
    pub(crate) skipped_update_version: Option<Option<String>>,
}

impl StoredAccount {
    fn proxy_priority_or_default(&self) -> i32 {
        self.proxy_priority.unwrap_or(100)
    }

    fn proxy_weight_or_default(&self) -> u32 {
        self.proxy_weight.filter(|value| *value > 0).unwrap_or(100)
    }

    fn proxy_key_selection_mode_or_default(&self) -> ProxyKeySelectionMode {
        self.proxy_key_selection_mode
            .unwrap_or(match self.source_kind {
                AccountSourceKind::Chatgpt => ProxyKeySelectionMode::FixedPriority,
                AccountSourceKind::Relay => ProxyKeySelectionMode::RoundRobin,
            })
    }

    fn default_proxy_endpoint_capabilities(&self) -> Vec<ProxyEndpointCapability> {
        if !self.proxy_endpoints.is_empty() {
            return self.proxy_endpoints.clone();
        }

        match self.source_kind {
            AccountSourceKind::Chatgpt => vec![
                ProxyEndpointCapability::Responses,
                ProxyEndpointCapability::ResponsesCompact,
            ],
            AccountSourceKind::Relay => vec![
                ProxyEndpointCapability::Responses,
                ProxyEndpointCapability::ChatCompletions,
            ],
        }
    }

    pub(crate) fn to_proxy_upstream(&self) -> Option<ProxyUpstream> {
        let source_account_key = self.account_key();
        let provider = self.resolved_provider_metadata();
        let upstream_kind = match self.source_kind {
            AccountSourceKind::Chatgpt => ProxyUpstreamKind::Chatgpt,
            AccountSourceKind::Relay => ProxyUpstreamKind::RelayApi,
        };

        let channel = match self.source_kind {
            AccountSourceKind::Chatgpt => {
                let extracted = extract_auth(&self.auth_json).ok()?;
                ProxyChannel {
                    id: format!("channel:{}", self.id),
                    upstream_id: format!("upstream:{}", self.id),
                    source_account_id: self.id.clone(),
                    source_account_key: source_account_key.clone(),
                    source_label: self.label.clone(),
                    upstream_kind,
                    base_url: None,
                    model_name: None,
                    plan_type: self
                        .usage
                        .as_ref()
                        .and_then(|usage| usage.plan_type.clone())
                        .or(self.plan_type.clone())
                        .or(extracted.plan_type.clone()),
                    priority: self.proxy_priority_or_default(),
                    weight: self.proxy_weight_or_default(),
                    enabled: !self.auth_refresh_blocked,
                    key_selection_mode: self.proxy_key_selection_mode_or_default(),
                    health_status: if self.auth_refresh_blocked {
                        ProxyHealthStatus::AuthFailed
                    } else {
                        ProxyHealthStatus::Healthy
                    },
                    cooldown_until: None,
                    last_error: self.auth_refresh_error.clone(),
                    models: Vec::new(),
                    endpoints: self.default_proxy_endpoint_capabilities(),
                    keys: vec![ProxyKey {
                        id: format!("key:{}", self.id),
                        label: Some("access_token".to_string()),
                        secret: Some(extracted.access_token),
                        enabled: !self.auth_refresh_blocked,
                        priority: 100,
                        weight: 100,
                        health_status: if self.auth_refresh_blocked {
                            ProxyHealthStatus::AuthFailed
                        } else {
                            ProxyHealthStatus::Healthy
                        },
                        last_error: self.auth_refresh_error.clone(),
                        cooldown_until: None,
                        failure_count: 0,
                        last_used_at: None,
                        updated_at: Some(self.updated_at),
                    }],
                }
            }
            AccountSourceKind::Relay => {
                let base_url = self.api_base_url.as_deref()?.trim().trim_end_matches('/');
                let model_name = self.model_name.as_deref()?.trim();
                let keys = self.resolved_relay_proxy_keys();
                if base_url.is_empty() || keys.is_empty() || model_name.is_empty() {
                    return None;
                }
                ProxyChannel {
                    id: format!("channel:{}", self.id),
                    upstream_id: format!("upstream:{}", self.id),
                    source_account_id: self.id.clone(),
                    source_account_key: source_account_key.clone(),
                    source_label: self.label.clone(),
                    upstream_kind,
                    base_url: Some(base_url.to_string()),
                    model_name: Some(model_name.to_string()),
                    plan_type: Some(self.plan_type.clone().unwrap_or_else(|| "api".to_string())),
                    priority: self.proxy_priority_or_default(),
                    weight: self.proxy_weight_or_default(),
                    enabled: true,
                    key_selection_mode: self.proxy_key_selection_mode_or_default(),
                    health_status: ProxyHealthStatus::Healthy,
                    cooldown_until: None,
                    last_error: None,
                    models: vec![model_name.to_string()],
                    endpoints: self.default_proxy_endpoint_capabilities(),
                    keys,
                }
            }
        };

        Some(ProxyUpstream {
            id: format!("upstream:{}", self.id),
            source_account_id: self.id.clone(),
            source_account_key,
            source_label: self.label.clone(),
            upstream_kind,
            provider_id: provider.0,
            provider_name: provider.1,
            plan_type: channel.plan_type.clone(),
            priority: self.proxy_priority_or_default(),
            weight: self.proxy_weight_or_default(),
            enabled: channel.enabled,
            privacy_hidden: false,
            health_status: channel.health_status,
            cooldown_until: channel.cooldown_until,
            last_error: channel.last_error.clone(),
            models: channel.models.clone(),
            channels: vec![channel],
            tags: self.tags.clone(),
            updated_at: self.updated_at,
        })
    }

    pub(crate) fn resolved_relay_proxy_keys(&self) -> Vec<ProxyKey> {
        if !matches!(self.source_kind, AccountSourceKind::Relay) {
            return Vec::new();
        }

        let now = crate::utils::now_unix_seconds();
        let mut keys = self
            .api_keys
            .iter()
            .filter_map(|key| {
                key.secret
                    .as_deref()
                    .map(str::trim)
                    .filter(|secret| !secret.is_empty())
                    .map(|secret| {
                        let mut key = key.clone();
                        key.secret = Some(secret.to_string());
                        key.weight = if key.weight == 0 { 100 } else { key.weight };
                        if matches!(
                            key.health_status,
                            ProxyHealthStatus::CoolingDown | ProxyHealthStatus::QuotaExhausted
                        ) && key
                            .cooldown_until
                            .map(|cooldown_until| cooldown_until <= now)
                            .unwrap_or(false)
                        {
                            key.health_status = ProxyHealthStatus::Healthy;
                            key.cooldown_until = None;
                        }
                        key
                    })
            })
            .collect::<Vec<_>>();

        if keys.is_empty() {
            if let Some(api_key) = self
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                keys.push(ProxyKey {
                    id: format!("key:{}", self.id),
                    label: Some(self.label.clone()),
                    secret: Some(api_key.to_string()),
                    enabled: true,
                    priority: 100,
                    weight: 100,
                    health_status: ProxyHealthStatus::Healthy,
                    last_error: None,
                    cooldown_until: None,
                    failure_count: 0,
                    last_used_at: None,
                    updated_at: Some(self.updated_at),
                });
            }
        }

        keys
    }

    pub(crate) fn sync_relay_api_keys_from_legacy(&mut self) -> bool {
        if !matches!(self.source_kind, AccountSourceKind::Relay) || !self.api_keys.is_empty() {
            return false;
        }

        let Some(api_key) = self
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return false;
        };

        self.api_keys.push(ProxyKey {
            id: format!("key:{}:primary", self.id),
            label: Some(self.label.clone()),
            secret: Some(api_key.to_string()),
            enabled: true,
            priority: 100,
            weight: 100,
            health_status: ProxyHealthStatus::Healthy,
            last_error: None,
            cooldown_until: None,
            failure_count: 0,
            last_used_at: None,
            updated_at: Some(self.updated_at),
        });
        true
    }

    pub(crate) fn primary_relay_api_key(&self) -> Option<&str> {
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                self.api_keys
                    .iter()
                    .find(|key| key.enabled)
                    .and_then(|key| key.secret.as_deref())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
    }

    pub(crate) fn resolved_provider_metadata(&self) -> (Option<String>, Option<String>) {
        let (derived_id, derived_name) =
            infer_provider_metadata_from_base_url(self.api_base_url.as_deref());
        (
            self.provider_id.clone().or(derived_id),
            self.provider_name.clone().or(derived_name),
        )
    }

    pub(crate) fn principal_key(&self) -> String {
        if matches!(self.source_kind, AccountSourceKind::Relay) {
            return format!("relay:{}", self.id);
        }

        normalized_identity_key(self.principal_id.as_deref())
            .or_else(|| {
                extract_auth(&self.auth_json)
                    .ok()
                    .map(|auth| auth.principal_id)
            })
            .or_else(|| normalized_email_key(self.email.as_deref()))
            .unwrap_or_else(|| self.account_id.clone())
    }

    pub(crate) fn account_key(&self) -> String {
        if matches!(self.source_kind, AccountSourceKind::Relay) {
            return relay_account_key(&self.id);
        }

        account_group_key(&self.principal_key(), &self.account_id)
    }

    pub(crate) fn resolved_plan_type(&self) -> Option<String> {
        if matches!(self.source_kind, AccountSourceKind::Relay) {
            return self.plan_type.clone();
        }

        self.plan_type
            .clone()
            .or_else(|| {
                extract_auth(&self.auth_json)
                    .ok()
                    .and_then(|auth| auth.plan_type)
            })
            .or_else(|| {
                self.usage
                    .as_ref()
                    .and_then(|usage| usage.plan_type.clone())
            })
    }

    pub(crate) fn variant_key(&self) -> String {
        if matches!(self.source_kind, AccountSourceKind::Relay) {
            return self.account_key();
        }

        account_variant_key(
            &self.principal_key(),
            &self.account_id,
            self.resolved_plan_type().as_deref(),
        )
    }

    pub(crate) fn to_summary(
        &self,
        current_account_key: Option<&str>,
        current_variant_key: Option<&str>,
    ) -> AccountSummary {
        let account_key = self.account_key();
        let is_current = current_variant_key
            .map(|variant_key| variant_key == self.variant_key())
            .unwrap_or_else(|| {
                current_account_key
                    .map(|key| key == account_key)
                    .unwrap_or(false)
            });

        AccountSummary {
            provider_id: self.resolved_provider_metadata().0,
            provider_name: self.resolved_provider_metadata().1,
            tags: self.tags.clone(),
            id: self.id.clone(),
            label: self.label.clone(),
            source_kind: self.source_kind.clone(),
            email: self.email.clone(),
            account_key,
            account_id: self.account_id.clone(),
            plan_type: self.plan_type.clone(),
            api_base_url: self.api_base_url.clone(),
            model_name: self.model_name.clone(),
            balance_text: self.balance_text.clone(),
            balance_display_enabled: self.balance_display_enabled,
            api_quota_mode: self.api_quota_mode,
            api_quota_today_used_text: self.api_quota_today_used_text.clone(),
            api_quota_remaining_text: self.api_quota_remaining_text.clone(),
            api_quota_total_remaining_text: self.api_quota_total_remaining_text.clone(),
            api_quota_total_tokens_text: self.api_quota_total_tokens_text.clone(),
            api_quota_today_tokens_text: self.api_quota_today_tokens_text.clone(),
            api_quota_daily_window: self.api_quota_daily_window.clone(),
            api_quota_total_window: self.api_quota_total_window.clone(),
            api_quota_subscription_expires_at: self.api_quota_subscription_expires_at,
            profile_auth_ready: self.profile_auth_ready,
            profile_config_ready: self.profile_config_ready,
            profile_integrity_error: self.profile_integrity_error.clone(),
            profile_last_validated_at: self.profile_last_validated_at,
            profile_last_validation_error: self.profile_last_validation_error.clone(),
            added_at: self.added_at,
            updated_at: self.updated_at,
            usage: self.usage.clone(),
            usage_error: self.usage_error.clone(),
            auth_refresh_blocked: self.auth_refresh_blocked,
            auth_refresh_error: self.auth_refresh_error.clone(),
            auth_refresh_next_at: self.auth_refresh_next_at,
            is_current,
        }
    }
}

pub(crate) fn relay_account_key(id: &str) -> String {
    format!("relay|{id}")
}

pub(crate) fn infer_provider_metadata_from_base_url(
    base_url: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(host) = normalized_provider_host(base_url) else {
        return (None, None);
    };

    let host_lower = host.to_ascii_lowercase();
    let (provider_id, provider_name) = if host_lower.contains("openrouter") {
        ("openrouter", "OpenRouter")
    } else if host_lower.contains("openai") {
        ("openai", "OpenAI")
    } else if host_lower.contains("anthropic") {
        ("anthropic", "Anthropic")
    } else if host_lower.contains("gemini")
        || host_lower.contains("googleapis")
        || host_lower.contains("generativelanguage")
    {
        ("google", "Google AI")
    } else if host_lower.contains("deepseek") {
        ("deepseek", "DeepSeek")
    } else if host_lower.contains("siliconflow") {
        ("siliconflow", "SiliconFlow")
    } else if host_lower.contains("moonshot") || host_lower.contains("kimi") {
        ("moonshot", "Moonshot")
    } else if host_lower.contains("dashscope") || host_lower.contains("aliyuncs") {
        ("dashscope", "DashScope")
    } else if host_lower.contains("volcengine")
        || host_lower.contains("volces")
        || host_lower.contains("ark")
    {
        ("ark", "Volcengine Ark")
    } else {
        return (Some("custom".to_string()), Some(host));
    };

    (
        Some(provider_id.to_string()),
        Some(provider_name.to_string()),
    )
}

fn normalized_provider_host(base_url: Option<&str>) -> Option<String> {
    let trimmed = base_url?.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_scheme = trimmed.split("://").nth(1).unwrap_or(trimmed);
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host = host_port.split('@').last().unwrap_or(host_port);
    let normalized = host.split(':').next().unwrap_or(host).trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

fn normalized_email_key(email: Option<&str>) -> Option<String> {
    email
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn normalized_identity_key(value: Option<&str>) -> Option<String> {
    let trimmed = value.map(str::trim).filter(|value| !value.is_empty())?;
    if trimmed.contains('@') {
        Some(trimmed.to_ascii_lowercase())
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn dedupe_account_variants(accounts: &mut Vec<StoredAccount>) -> bool {
    let mut changed = false;
    let mut merged_accounts: Vec<StoredAccount> = Vec::with_capacity(accounts.len());
    let mut index_by_variant: HashMap<String, usize> = HashMap::new();

    for account in std::mem::take(accounts) {
        let variant_key = account.variant_key();
        if let Some(existing_index) = index_by_variant.get(&variant_key).copied() {
            let merged =
                merge_duplicate_account_variant(merged_accounts[existing_index].clone(), account);
            merged_accounts[existing_index] = merged;
            changed = true;
        } else {
            index_by_variant.insert(variant_key, merged_accounts.len());
            merged_accounts.push(account);
        }
    }

    *accounts = merged_accounts;

    changed
}

fn merge_duplicate_account_variant(left: StoredAccount, right: StoredAccount) -> StoredAccount {
    let left_score = duplicate_account_merge_score(&left);
    let right_score = duplicate_account_merge_score(&right);
    let (mut preferred, alternate) = if right_score > left_score {
        (right, left)
    } else {
        (left, right)
    };

    preferred.added_at = preferred.added_at.min(alternate.added_at);
    preferred.updated_at = preferred.updated_at.max(alternate.updated_at);

    if preferred.email.is_none() {
        preferred.email = alternate.email.clone();
    }
    if preferred.plan_type.is_none() {
        preferred.plan_type = alternate.plan_type.clone();
    }
    if preferred.usage.is_none() {
        preferred.usage = alternate.usage.clone();
    }
    if preferred.usage.is_none() && preferred.usage_error.is_none() {
        preferred.usage_error = alternate.usage_error.clone();
    }
    if preferred.auth_refresh_blocked && preferred.auth_refresh_error.is_none() {
        preferred.auth_refresh_error = alternate.auth_refresh_error.clone();
    }
    if preferred.auth_refresh_next_at.is_none() {
        preferred.auth_refresh_next_at = alternate.auth_refresh_next_at;
    }
    if preferred.auth_json.is_null() && !alternate.auth_json.is_null() {
        preferred.auth_json = alternate.auth_json.clone();
    }
    if preferred.api_base_url.is_none() {
        preferred.api_base_url = alternate.api_base_url.clone();
    }
    if preferred.api_key.is_none() {
        preferred.api_key = alternate.api_key.clone();
    }
    merge_proxy_keys(&mut preferred.api_keys, &alternate.api_keys);
    if preferred.model_name.is_none() {
        preferred.model_name = alternate.model_name.clone();
    }
    if preferred.balance_text.is_none() {
        preferred.balance_text = alternate.balance_text.clone();
    }
    if !preferred.balance_display_enabled && alternate.balance_display_enabled {
        preferred.balance_display_enabled = true;
    }
    if preferred.api_quota_mode == ApiQuotaMode::ApiOnly {
        preferred.api_quota_mode = alternate.api_quota_mode;
    }
    if preferred.api_quota_today_used_text.is_none() {
        preferred.api_quota_today_used_text = alternate.api_quota_today_used_text.clone();
    }
    if preferred.api_quota_remaining_text.is_none() {
        preferred.api_quota_remaining_text = alternate.api_quota_remaining_text.clone();
    }
    if preferred.api_quota_total_remaining_text.is_none() {
        preferred.api_quota_total_remaining_text = alternate.api_quota_total_remaining_text.clone();
    }
    if preferred.api_quota_total_tokens_text.is_none() {
        preferred.api_quota_total_tokens_text = alternate.api_quota_total_tokens_text.clone();
    }
    if preferred.api_quota_today_tokens_text.is_none() {
        preferred.api_quota_today_tokens_text = alternate.api_quota_today_tokens_text.clone();
    }
    if preferred.api_quota_daily_window.is_none() {
        preferred.api_quota_daily_window = alternate.api_quota_daily_window.clone();
    }
    if preferred.api_quota_total_window.is_none() {
        preferred.api_quota_total_window = alternate.api_quota_total_window.clone();
    }
    if preferred.api_quota_subscription_expires_at.is_none() {
        preferred.api_quota_subscription_expires_at = alternate.api_quota_subscription_expires_at;
    }
    if preferred.provider_id.is_none() {
        preferred.provider_id = alternate.provider_id.clone();
    }
    if preferred.provider_name.is_none() {
        preferred.provider_name = alternate.provider_name.clone();
    }
    merge_account_tags(&mut preferred.tags, &alternate.tags);
    if preferred.profile_auth_path.is_none() {
        preferred.profile_auth_path = alternate.profile_auth_path.clone();
    }
    if preferred.profile_config_path.is_none() {
        preferred.profile_config_path = alternate.profile_config_path.clone();
    }
    preferred.profile_auth_ready = preferred.profile_auth_ready || alternate.profile_auth_ready;
    preferred.profile_config_ready =
        preferred.profile_config_ready || alternate.profile_config_ready;
    if preferred.profile_integrity_error.is_none() {
        preferred.profile_integrity_error = alternate.profile_integrity_error.clone();
    }
    if preferred.profile_last_validated_at.is_none() {
        preferred.profile_last_validated_at = alternate.profile_last_validated_at;
    }
    if preferred.profile_last_validation_error.is_none() {
        preferred.profile_last_validation_error = alternate.profile_last_validation_error.clone();
    }

    preferred
}

fn merge_account_tags(preferred: &mut Vec<String>, alternate: &[String]) {
    for tag in alternate {
        if preferred
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(tag))
        {
            continue;
        }
        preferred.push(tag.clone());
    }
}

fn merge_proxy_keys(preferred: &mut Vec<ProxyKey>, alternate: &[ProxyKey]) {
    for key in alternate {
        if preferred.iter().any(|existing| existing.id == key.id) {
            continue;
        }
        preferred.push(key.clone());
    }
}

fn duplicate_account_merge_score(account: &StoredAccount) -> (u8, u8, u8, u8, i64, i64) {
    (
        u8::from(account.usage.is_some() && account.usage_error.is_none()),
        u8::from(!account.auth_refresh_blocked),
        u8::from(account.resolved_plan_type().is_some()),
        u8::from(
            account
                .email
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some(),
        ),
        account.updated_at,
        account.added_at,
    )
}

#[cfg(test)]
mod tests {
    use super::dedupe_account_variants;
    use super::AccountSourceKind;
    use super::ProxyHealthStatus;
    use super::ProxyKeySelectionMode;
    use super::ProxyUpstreamKind;
    use super::StoredAccount;
    use super::UsageSnapshot;
    use super::UsageWindow;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use serde_json::json;

    fn usage_snapshot(plan_type: &str) -> UsageSnapshot {
        UsageSnapshot {
            fetched_at: 10,
            plan_type: Some(plan_type.to_string()),
            five_hour: Some(UsageWindow {
                used_percent: 10.0,
                window_seconds: 18_000,
                reset_at: Some(20),
            }),
            one_week: Some(UsageWindow {
                used_percent: 20.0,
                window_seconds: 604_800,
                reset_at: Some(30),
            }),
            credits: None,
        }
    }

    fn jwt_with_plan(plan_type: &str) -> String {
        let payload = URL_SAFE_NO_PAD.encode(format!(
            r#"{{"email":"shared@example.com","https://api.openai.com/auth":{{"chatgpt_account_id":"account-1","chatgpt_plan_type":"{plan_type}"}}}}"#
        ));
        format!("header.{payload}.signature")
    }

    fn stored_account(
        id: &str,
        label: &str,
        account_id: &str,
        plan_type: Option<&str>,
        usage_plan_type: Option<&str>,
        updated_at: i64,
    ) -> StoredAccount {
        StoredAccount {
            id: id.to_string(),
            label: label.to_string(),
            source_kind: Default::default(),
            principal_id: Some("shared@example.com".to_string()),
            email: Some("shared@example.com".to_string()),
            account_id: account_id.to_string(),
            plan_type: plan_type.map(ToString::to_string),
            auth_json: json!({ "id": id }),
            api_base_url: None,
            api_key: None,
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: None,
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
            added_at: updated_at - 1,
            updated_at,
            usage: usage_plan_type.map(usage_snapshot),
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        }
    }

    #[test]
    fn dedupe_account_variants_keeps_newest_variant_record() {
        let mut accounts = vec![
            stored_account(
                "old",
                "legacy",
                "account-1",
                Some("team"),
                Some("team"),
                100,
            ),
            stored_account("new", "fresh", "account-1", Some("team"), Some("team"), 200),
        ];

        let changed = dedupe_account_variants(&mut accounts);

        assert!(changed);
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "new");
        assert_eq!(accounts[0].label, "fresh");
        assert_eq!(accounts[0].added_at, 99);
        assert_eq!(accounts[0].updated_at, 200);
    }

    #[test]
    fn dedupe_account_variants_merges_when_usage_reveals_same_variant() {
        let mut accounts = vec![
            stored_account("unknown", "legacy", "account-1", None, Some("team"), 100),
            stored_account(
                "team",
                "current",
                "account-1",
                Some("team"),
                Some("team"),
                200,
            ),
        ];

        let changed = dedupe_account_variants(&mut accounts);

        assert!(changed);
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "team");
    }

    #[test]
    fn dedupe_account_variants_does_not_restore_stale_auth_error() {
        let mut stale = stored_account("stale", "stale", "account-1", Some("team"), None, 100);
        stale.usage_error = Some("授权过期，请重新登录授权。".to_string());
        stale.auth_refresh_blocked = true;
        stale.auth_refresh_error = Some("授权过期，请重新登录授权。".to_string());

        let mut healthy = stored_account(
            "healthy",
            "healthy",
            "account-1",
            Some("team"),
            Some("team"),
            200,
        );
        healthy.auth_refresh_next_at = Some(1234);

        let mut accounts = vec![stale, healthy];

        let changed = dedupe_account_variants(&mut accounts);

        assert!(changed);
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "healthy");
        assert!(accounts[0].usage.is_some());
        assert_eq!(accounts[0].usage_error, None);
        assert!(!accounts[0].auth_refresh_blocked);
        assert_eq!(accounts[0].auth_refresh_error, None);
        assert_eq!(accounts[0].auth_refresh_next_at, Some(1234));
    }

    #[test]
    fn resolved_plan_type_prefers_stored_plan_type_over_usage_plan_type() {
        let account = StoredAccount {
            id: "mixed".to_string(),
            label: "mixed".to_string(),
            source_kind: Default::default(),
            principal_id: Some("shared@example.com".to_string()),
            email: Some("shared@example.com".to_string()),
            account_id: "account-1".to_string(),
            plan_type: Some("team".to_string()),
            auth_json: json!({ "kind": "mixed" }),
            api_base_url: None,
            api_key: None,
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: None,
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
            usage: Some(usage_snapshot("plus")),
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        };

        assert_eq!(account.resolved_plan_type().as_deref(), Some("team"));
        assert_eq!(account.variant_key(), "shared@example.com|account-1|team");
    }

    #[test]
    fn resolved_plan_type_falls_back_to_auth_claim_before_usage() {
        let account = StoredAccount {
            id: "auth".to_string(),
            label: "auth".to_string(),
            source_kind: Default::default(),
            principal_id: Some("shared@example.com".to_string()),
            email: Some("shared@example.com".to_string()),
            account_id: "account-1".to_string(),
            plan_type: None,
            auth_json: json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "access_token": "token",
                    "id_token": jwt_with_plan("team")
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
            usage: Some(usage_snapshot("plus")),
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        };

        assert_eq!(account.resolved_plan_type().as_deref(), Some("team"));
    }

    #[test]
    fn persisted_principal_id_keeps_same_workspace_different_users_separate() {
        let mut accounts = vec![
            StoredAccount {
                id: "first".to_string(),
                label: "first".to_string(),
                source_kind: Default::default(),
                principal_id: Some("first@example.com".to_string()),
                email: None,
                account_id: "workspace-1".to_string(),
                plan_type: Some("team".to_string()),
                auth_json: json!({ "kind": "legacy" }),
                api_base_url: None,
                api_key: None,
                api_keys: Vec::new(),
                proxy_priority: None,
                proxy_weight: None,
                proxy_key_selection_mode: None,
                proxy_endpoints: Vec::new(),
                model_name: None,
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
            },
            StoredAccount {
                id: "second".to_string(),
                label: "second".to_string(),
                source_kind: Default::default(),
                principal_id: Some("second@example.com".to_string()),
                email: None,
                account_id: "workspace-1".to_string(),
                plan_type: Some("team".to_string()),
                auth_json: json!({ "kind": "legacy" }),
                api_base_url: None,
                api_key: None,
                api_keys: Vec::new(),
                proxy_priority: None,
                proxy_weight: None,
                proxy_key_selection_mode: None,
                proxy_endpoints: Vec::new(),
                model_name: None,
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
                added_at: 2,
                updated_at: 2,
                usage: None,
                usage_error: None,
                auth_refresh_blocked: false,
                auth_refresh_error: None,
                auth_refresh_next_at: None,
            },
        ];

        let changed = dedupe_account_variants(&mut accounts);

        assert!(!changed);
        assert_eq!(accounts.len(), 2);
        assert_ne!(accounts[0].account_key(), accounts[1].account_key());
    }

    #[test]
    fn chatgpt_account_converts_to_proxy_upstream() {
        let account = StoredAccount {
            id: "chatgpt-1".to_string(),
            label: "ChatGPT".to_string(),
            source_kind: AccountSourceKind::Chatgpt,
            principal_id: Some("shared@example.com".to_string()),
            email: Some("shared@example.com".to_string()),
            account_id: "workspace-1".to_string(),
            plan_type: Some("team".to_string()),
            auth_json: json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "access_token": "atk",
                    "id_token": jwt_with_plan("team")
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
            provider_id: None,
            provider_name: None,
            tags: vec!["核心".to_string()],
            profile_auth_path: None,
            profile_config_path: None,
            profile_auth_ready: false,
            profile_config_ready: false,
            profile_integrity_error: None,
            profile_last_validated_at: None,
            profile_last_validation_error: None,
            added_at: 1,
            updated_at: 2,
            usage: Some(usage_snapshot("team")),
            usage_error: None,
            auth_refresh_blocked: false,
            auth_refresh_error: None,
            auth_refresh_next_at: None,
        };

        let upstream = account.to_proxy_upstream().expect("proxy upstream");

        assert_eq!(upstream.upstream_kind, ProxyUpstreamKind::Chatgpt);
        assert_eq!(upstream.plan_type.as_deref(), Some("team"));
        assert_eq!(upstream.priority, 100);
        assert_eq!(upstream.weight, 100);
        assert_eq!(upstream.channels.len(), 1);
        assert_eq!(
            upstream.channels[0].key_selection_mode,
            ProxyKeySelectionMode::FixedPriority
        );
        assert_eq!(upstream.channels[0].keys.len(), 1);
        assert_eq!(upstream.channels[0].keys[0].secret.as_deref(), Some("atk"));
        assert_eq!(
            upstream.channels[0].health_status,
            ProxyHealthStatus::Healthy
        );
    }

    #[test]
    fn relay_account_converts_to_proxy_upstream() {
        let account = StoredAccount {
            id: "relay-1".to_string(),
            label: "Relay".to_string(),
            source_kind: AccountSourceKind::Relay,
            principal_id: Some("relay:relay-1".to_string()),
            email: None,
            account_id: "relay-account".to_string(),
            plan_type: Some("api".to_string()),
            auth_json: json!({}),
            api_base_url: Some("https://api.example.com/v1/".to_string()),
            api_key: Some("sk-relay".to_string()),
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: Some("gpt-5.4".to_string()),
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
            provider_id: Some("custom".to_string()),
            provider_name: Some("Custom".to_string()),
            tags: vec!["API".to_string()],
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
        };

        let upstream = account.to_proxy_upstream().expect("proxy upstream");

        assert_eq!(upstream.upstream_kind, ProxyUpstreamKind::RelayApi);
        assert_eq!(upstream.plan_type.as_deref(), Some("api"));
        assert_eq!(upstream.channels.len(), 1);
        assert_eq!(
            upstream.channels[0].base_url.as_deref(),
            Some("https://api.example.com/v1")
        );
        assert_eq!(upstream.channels[0].model_name.as_deref(), Some("gpt-5.4"));
        assert_eq!(
            upstream.channels[0].keys[0].secret.as_deref(),
            Some("sk-relay")
        );
    }
}
