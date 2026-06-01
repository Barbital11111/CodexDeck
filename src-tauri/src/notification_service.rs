use std::collections::BTreeMap;
use std::collections::HashSet;

use serde::Serialize;
use serde_json::json;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::models::NotificationProviderConfig;
use crate::models::NotificationProviderKind;
use crate::models::NotificationTargetConfig;
use crate::models::NotificationTargetKind;
use crate::models::UsageWindow;

#[derive(Debug, Clone)]
struct ProviderUsageSnapshot {
    provider_name: String,
    account_label: String,
    multiplier: f64,
    effective_balance: f64,
    today_cost: f64,
    effective_today_cost: f64,
    effective_total_cost: f64,
    today_requests: i64,
    today_tokens: i64,
    total_requests: i64,
    total_tokens: i64,
    models: Vec<ModelUsageSnapshot>,
}

#[derive(Debug, Clone)]
struct ModelUsageSnapshot {
    model: String,
    requests: i64,
    effective_cost: f64,
}

#[derive(Debug, Clone)]
struct Sub2apiAuthSession {
    base_url: String,
    access_token: String,
}

#[derive(Debug, Clone)]
struct Sub2apiEndpointError {
    message: String,
    retry_next_base: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiQuotaSnapshot {
    pub(crate) mode: crate::models::ApiQuotaMode,
    pub(crate) today_used_text: Option<String>,
    pub(crate) remaining_text: Option<String>,
    pub(crate) total_remaining_text: Option<String>,
    pub(crate) total_tokens_text: Option<String>,
    pub(crate) today_tokens_text: Option<String>,
    pub(crate) daily_window: Option<UsageWindow>,
    pub(crate) total_window: Option<UsageWindow>,
    pub(crate) subscription_expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TelegramChatCandidate {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) chat_type: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TelegramChatDiscoveryResult {
    pub(crate) bot_username: Option<String>,
    pub(crate) chats: Vec<TelegramChatCandidate>,
}

pub(crate) async fn discover_telegram_chats(
    bot_token: String,
) -> Result<TelegramChatDiscoveryResult, String> {
    let token = required_field(Some(bot_token.as_str()), "请填写 Telegram Bot Token")?;
    let client = build_client()?;

    let get_me_payload = telegram_get_json(
        &client,
        build_telegram_api_url(token, "getMe")?,
        "Telegram Bot Token 验证失败",
    )
    .await?;
    let get_me_result = ensure_telegram_ok(&get_me_payload, "Telegram Bot Token 验证失败")?;
    let bot_username = get_me_result
        .get("username")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let mut updates_url = build_telegram_api_url(token, "getUpdates")?;
    updates_url
        .query_pairs_mut()
        .append_pair("limit", "50")
        .append_pair(
            "allowed_updates",
            r#"["message","edited_message","channel_post","edited_channel_post","my_chat_member","chat_member"]"#,
        );
    let updates_payload =
        telegram_get_json(&client, updates_url, "读取 Telegram 最近会话失败").await?;
    let updates_result = ensure_telegram_ok(&updates_payload, "读取 Telegram 最近会话失败")?;
    let Some(updates) = updates_result.as_array() else {
        return Err("Telegram getUpdates 返回格式异常".to_string());
    };

    let mut chats = BTreeMap::new();
    for update in updates {
        for pointer in [
            "/message/chat",
            "/edited_message/chat",
            "/channel_post/chat",
            "/edited_channel_post/chat",
            "/my_chat_member/chat",
            "/chat_member/chat",
        ] {
            let Some(chat) = update.pointer(pointer) else {
                continue;
            };
            if let Some(candidate) = parse_telegram_chat_candidate(chat) {
                chats.entry(candidate.id.clone()).or_insert(candidate);
            }
        }
    }

    Ok(TelegramChatDiscoveryResult {
        bot_username,
        chats: chats.into_values().collect(),
    })
}

pub(crate) async fn test_notification_provider(
    provider: NotificationProviderConfig,
) -> Result<String, String> {
    let base_url = normalize_sub2api_base_url(required_field(
        Some(provider.base_url.as_str()),
        "请填写 API 平台访问 URL",
    )?);
    let email = required_field(Some(provider.email.as_str()), "请填写 API 平台登录账号")?;
    let password = required_field(provider.password.as_deref(), "请填写 API 平台登录密码")?;
    let base =
        reqwest::Url::parse(&base_url).map_err(|error| format!("API 平台 URL 无效: {error}"))?;
    if !matches!(base.scheme(), "http" | "https") {
        return Err("API 平台 URL 仅支持 http/https".to_string());
    }

    let client = build_client()?;
    let session = login_sub2api_provider(&client, &base_url, email, password).await?;
    let user =
        fetch_sub2api_current_user(&client, &session.base_url, &session.access_token).await?;
    let account_label = user_label(&user);
    fetch_sub2api_dashboard_stats(&client, &session.base_url, &session.access_token).await?;

    Ok(format!(
        "API 平台连接成功：{account_label}，账号与用量接口均可访问。"
    ))
}

pub(crate) async fn fetch_api_quota_snapshot(
    provider: NotificationProviderConfig,
) -> Result<ApiQuotaSnapshot, String> {
    let base_url = normalize_sub2api_base_url(required_field(
        Some(provider.base_url.as_str()),
        "请填写 API 平台访问 URL",
    )?);
    let email = required_field(Some(provider.email.as_str()), "请填写 API 平台登录账号")?;
    let password = required_field(provider.password.as_deref(), "请填写 API 平台登录密码")?;
    let base =
        reqwest::Url::parse(&base_url).map_err(|error| format!("API 平台 URL 无效: {error}"))?;
    if !matches!(base.scheme(), "http" | "https") {
        return Err("API 平台 URL 仅支持 http/https".to_string());
    }

    let client = build_client()?;
    let session = login_sub2api_provider(&client, &base_url, email, password).await?;
    let user = fetch_sub2api_current_user(&client, &session.base_url, &session.access_token)
        .await
        .unwrap_or(Value::Null);
    let stats = fetch_sub2api_dashboard_stats(&client, &session.base_url, &session.access_token)
        .await
        .unwrap_or(Value::Null);
    let progress =
        fetch_sub2api_subscription_progress(&client, &session.base_url, &session.access_token)
            .await
            .unwrap_or_default();

    Ok(build_api_quota_snapshot(
        &provider, &user, &stats, &progress,
    ))
}

pub(crate) async fn fetch_newapi_token_quota_snapshot(
    base_url: &str,
    api_key: &str,
) -> Result<ApiQuotaSnapshot, String> {
    let base_url = normalize_openai_compatible_base_url(required_field(
        Some(base_url),
        "请填写 API 平台访问 URL",
    )?);
    let api_key = required_field(Some(api_key), "请填写 API Key")?;
    let base =
        reqwest::Url::parse(&base_url).map_err(|error| format!("API 平台 URL 无效: {error}"))?;
    if !matches!(base.scheme(), "http" | "https") {
        return Err("API 平台 URL 仅支持 http/https".to_string());
    }

    let client = build_client()?;
    let mut last_error = None;
    for endpoint in [
        format!("{base_url}/api/usage/token/"),
        format!("{base_url}/api/usage/token"),
    ] {
        let response = client
            .get(&endpoint)
            .bearer_auth(api_key)
            .send()
            .await
            .map_err(|error| {
                format!(
                    "连接 NewAPI 额度接口失败: {}",
                    sanitize_reqwest_error(error)
                )
            })?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            last_error = Some(format!(
                "NewAPI 额度接口失败 {status}: {}",
                summarize_api_error_body(&body).unwrap_or_else(|| summarize_response_body(&body))
            ));
            continue;
        }

        let payload = parse_json_body(&body, "NewAPI 额度接口返回格式异常")?;
        let data = unwrap_data(&payload);
        return build_newapi_token_quota_snapshot(data)
            .ok_or_else(|| "NewAPI 额度接口返回缺少 total_available / total_used".to_string());
    }

    Err(last_error.unwrap_or_else(|| "NewAPI 额度接口失败: 未收到响应".to_string()))
}

pub(crate) async fn test_notification_target(
    target: NotificationTargetConfig,
) -> Result<(), String> {
    match target.kind {
        NotificationTargetKind::Telegram => test_telegram_target(&target).await,
        NotificationTargetKind::Webhook => test_webhook_target(&target).await,
    }
}

pub(crate) async fn test_aggregate_notification(
    target: NotificationTargetConfig,
    providers: Vec<NotificationProviderConfig>,
) -> Result<(), String> {
    let selected_provider_ids = target.provider_ids.iter().collect::<HashSet<_>>();
    let has_provider_filter = !selected_provider_ids.is_empty();
    let active_providers = providers
        .into_iter()
        .filter(|provider| provider.enabled)
        .filter(|provider| matches!(provider.kind, NotificationProviderKind::Sub2api))
        .filter(|provider| !has_provider_filter || selected_provider_ids.contains(&provider.id))
        .collect::<Vec<_>>();

    if active_providers.is_empty() {
        return Err("没有匹配且启用的 Sub2API 平台，无法生成聚合推送。".to_string());
    }

    let client = build_client()?;
    let mut snapshots = Vec::new();
    for provider in active_providers {
        let provider_name = provider_display_name(&provider);
        let snapshot = collect_sub2api_usage_snapshot(&client, &provider)
            .await
            .map_err(|error| format!("{provider_name} 查询失败：{error}"))?;
        snapshots.push(snapshot);
    }

    let message = if target.aggregate_enabled {
        render_usage_template(&target, &snapshots)
    } else {
        render_usage_template(&target, &snapshots[..1])
    };
    send_notification_message(&target, "CodexDeck 聚合额度日报", &message).await
}

async fn login_sub2api_provider(
    client: &reqwest::Client,
    base_url: &str,
    email: &str,
    password: &str,
) -> Result<Sub2apiAuthSession, String> {
    let candidates = sub2api_base_url_candidates(base_url)?;
    let mut retry_errors = Vec::new();
    for candidate in candidates {
        match login_sub2api_provider_at_base(client, &candidate, email, password).await {
            Ok(access_token) => {
                return Ok(Sub2apiAuthSession {
                    base_url: candidate,
                    access_token,
                });
            }
            Err(error) if error.retry_next_base => {
                retry_errors.push(format!("{candidate}: {}", error.message));
            }
            Err(error) => {
                return Err(format!(
                    "API 平台登录失败（{}）：{}",
                    candidate, error.message
                ));
            }
        }
    }

    if retry_errors.is_empty() {
        Err("API 平台登录失败：未找到可用的 Sub2API API 基址。".to_string())
    } else {
        Err(format!(
            "API 平台登录失败：未找到可用的 Sub2API API 基址。已尝试：{}",
            retry_errors.join("；")
        ))
    }
}

async fn login_sub2api_provider_at_base(
    client: &reqwest::Client,
    base_url: &str,
    email: &str,
    password: &str,
) -> Result<String, Sub2apiEndpointError> {
    let response = client
        .post(format!("{base_url}/auth/login"))
        .json(&json!({
            "email": email,
            "password": password
        }))
        .send()
        .await
        .map_err(|error| Sub2apiEndpointError {
            message: format!("连接 API 平台失败: {}", sanitize_reqwest_error(error)),
            retry_next_base: false,
        })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(Sub2apiEndpointError {
            message: format!(
                "{status}: {}",
                summarize_api_error_body(&body).unwrap_or_else(|| summarize_response_body(&body))
            ),
            retry_next_base: should_retry_sub2api_base_candidate(status, &body),
        });
    }

    let payload =
        parse_json_body(&body, "认证接口返回格式异常").map_err(|error| Sub2apiEndpointError {
            message: error,
            retry_next_base: body_looks_like_non_api_response(&body),
        })?;
    let data = unwrap_data(&payload);
    if data.get("temp_token").is_some()
        || data
            .get("requires_2fa")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Err(Sub2apiEndpointError {
            message: "该账号启用了 2FA/TOTP，当前通知测试暂不自动处理。".to_string(),
            retry_next_base: false,
        });
    }

    data.get("access_token")
        .or_else(|| data.get("accessToken"))
        .or_else(|| data.get("token"))
        .or_else(|| data.get("jwt"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| Sub2apiEndpointError {
            message: summarize_api_error_body(&body)
                .unwrap_or_else(|| "认证接口未返回 access_token".to_string()),
            retry_next_base: !looks_like_sub2api_auth_payload(&payload),
        })
}

async fn fetch_sub2api_current_user(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> Result<Value, String> {
    let data = authorized_sub2api_get(client, base_url, token, "/auth/me").await?;
    let user = data
        .get("user")
        .and_then(Value::as_object)
        .map_or(&data, |_| data.get("user").unwrap_or(&data));
    if !user.is_object() {
        return Err("用户接口返回格式异常".to_string());
    }

    Ok(user.clone())
}

async fn fetch_sub2api_dashboard_stats(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> Result<Value, String> {
    let data = authorized_sub2api_get(client, base_url, token, "/usage/dashboard/stats").await?;
    if !data.is_object() {
        return Err("用量统计接口返回格式异常".to_string());
    }
    Ok(data)
}

async fn fetch_sub2api_model_stats(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    report_date: &str,
) -> Result<Vec<Value>, String> {
    let endpoint =
        format!("/usage/dashboard/models?start_date={report_date}&end_date={report_date}");
    let data = authorized_sub2api_get(client, base_url, token, &endpoint).await?;
    if let Some(models) = data.get("models").and_then(Value::as_array) {
        return Ok(models
            .iter()
            .filter(|item| item.is_object())
            .cloned()
            .collect());
    }
    if let Some(models) = data.as_array() {
        return Ok(models
            .iter()
            .filter(|item| item.is_object())
            .cloned()
            .collect());
    }

    Err("模型统计接口返回格式异常".to_string())
}

async fn fetch_sub2api_subscription_progress(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> Result<Vec<Value>, String> {
    let data = authorized_sub2api_get(client, base_url, token, "/subscriptions/progress").await?;
    if let Some(items) = data.as_array() {
        return Ok(items
            .iter()
            .filter(|item| item.is_object())
            .cloned()
            .collect());
    }
    Err("订阅进度接口返回格式异常".to_string())
}

async fn collect_sub2api_usage_snapshot(
    client: &reqwest::Client,
    provider: &NotificationProviderConfig,
) -> Result<ProviderUsageSnapshot, String> {
    let base_url = normalize_sub2api_base_url(required_field(
        Some(provider.base_url.as_str()),
        "请填写 API 平台访问 URL",
    )?);
    let email = required_field(Some(provider.email.as_str()), "请填写 API 平台登录账号")?;
    let password = required_field(provider.password.as_deref(), "请填写 API 平台登录密码")?;
    let base =
        reqwest::Url::parse(&base_url).map_err(|error| format!("API 平台 URL 无效: {error}"))?;
    if !matches!(base.scheme(), "http" | "https") {
        return Err("API 平台 URL 仅支持 http/https".to_string());
    }

    let session = login_sub2api_provider(client, &base_url, email, password).await?;
    let user = fetch_sub2api_current_user(client, &session.base_url, &session.access_token).await?;
    let stats =
        fetch_sub2api_dashboard_stats(client, &session.base_url, &session.access_token).await?;
    let report_date = report_date_text();
    let model_values = fetch_sub2api_model_stats(
        client,
        &session.base_url,
        &session.access_token,
        &report_date,
    )
    .await
    .unwrap_or_default();
    let multiplier = normalize_multiplier(provider.cost_multiplier);
    let balance = json_number(user.get("balance"));
    let today_cost = json_number(
        stats
            .get("today_actual_cost")
            .or_else(|| stats.get("today_cost")),
    );
    let total_cost = json_number(
        stats
            .get("total_actual_cost")
            .or_else(|| stats.get("total_cost")),
    );

    Ok(ProviderUsageSnapshot {
        provider_name: provider_display_name(provider),
        account_label: user_label(&user),
        multiplier,
        effective_balance: balance * multiplier,
        today_cost,
        effective_today_cost: today_cost * multiplier,
        effective_total_cost: total_cost * multiplier,
        today_requests: json_i64(stats.get("today_requests")),
        today_tokens: json_i64(stats.get("today_tokens")),
        total_requests: json_i64(stats.get("total_requests")),
        total_tokens: json_i64(stats.get("total_tokens")),
        models: model_values
            .iter()
            .map(|item| {
                let raw_cost = json_number(item.get("actual_cost").or_else(|| item.get("cost")));
                ModelUsageSnapshot {
                    model: item
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    requests: json_i64(item.get("requests")),
                    effective_cost: raw_cost * multiplier,
                }
            })
            .collect(),
    })
}

async fn authorized_sub2api_get(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    endpoint: &str,
) -> Result<Value, String> {
    let response = client
        .get(format!("{base_url}{endpoint}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| format!("连接 API 平台失败: {}", sanitize_reqwest_error(error)))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "API 平台接口失败 {status}: {}",
            summarize_response_body(&body)
        ));
    }

    let payload = parse_json_body(&body, "API 平台接口返回格式异常")?;
    Ok(unwrap_data(&payload).clone())
}

async fn test_telegram_target(target: &NotificationTargetConfig) -> Result<(), String> {
    let message = render_test_message(target);
    send_telegram_message(target, &message).await
}

async fn send_telegram_message(
    target: &NotificationTargetConfig,
    message: &str,
) -> Result<(), String> {
    let token = required_field(
        target.telegram_bot_token.as_deref(),
        "请填写 Telegram Bot Token",
    )?;
    let chat_id = required_field(
        target.telegram_chat_id.as_deref(),
        "请填写 Telegram Chat ID",
    )?;
    let url = reqwest::Url::parse(&format!("https://api.telegram.org/bot{token}/sendMessage"))
        .map_err(|_| "Telegram Bot Token 格式不正确".to_string())?;

    let client = build_client()?;
    let response = client
        .post(url)
        .json(&json!({
            "chat_id": chat_id,
            "text": message,
            "disable_web_page_preview": true
        }))
        .send()
        .await
        .map_err(|error| format!("连接 Telegram 失败: {}", sanitize_reqwest_error(error)))?;

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!(
        "Telegram 返回失败状态 {status}: {}",
        summarize_response_body(&body)
    ))
}

fn build_telegram_api_url(token: &str, method: &str) -> Result<reqwest::Url, String> {
    reqwest::Url::parse(&format!("https://api.telegram.org/bot{token}/{method}"))
        .map_err(|_| "Telegram Bot Token 格式不正确".to_string())
}

async fn telegram_get_json(
    client: &reqwest::Client,
    url: reqwest::Url,
    context: &str,
) -> Result<Value, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("{context}: {}", sanitize_reqwest_error(error)))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let description = serde_json::from_str::<Value>(&body)
            .ok()
            .as_ref()
            .and_then(telegram_description)
            .unwrap_or_else(|| summarize_response_body(&body));
        return Err(format!("{context} {status}: {description}"));
    }

    parse_json_body(&body, context)
}

fn ensure_telegram_ok<'a>(payload: &'a Value, context: &str) -> Result<&'a Value, String> {
    if payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return payload
            .get("result")
            .ok_or_else(|| format!("{context}: Telegram 返回缺少 result"));
    }

    let description =
        telegram_description(payload).unwrap_or_else(|| "Telegram 返回 ok=false".to_string());
    Err(format!("{context}: {description}"))
}

fn telegram_description(payload: &Value) -> Option<String> {
    payload
        .get("description")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            payload
                .get("message")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
}

fn parse_telegram_chat_candidate(chat: &Value) -> Option<TelegramChatCandidate> {
    let id = chat
        .get("id")
        .and_then(|value| {
            value
                .as_i64()
                .map(|id| id.to_string())
                .or_else(|| value.as_u64().map(|id| id.to_string()))
                .or_else(|| value.as_str().map(ToString::to_string))
        })
        .filter(|value| !value.trim().is_empty())?;
    let chat_type = chat
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let first_name = chat.get("first_name").and_then(Value::as_str).unwrap_or("");
    let last_name = chat.get("last_name").and_then(Value::as_str).unwrap_or("");
    let full_name = format!("{first_name} {last_name}").trim().to_string();
    let title = chat
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| chat.get("username").and_then(Value::as_str))
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            if full_name.is_empty() {
                None
            } else {
                Some(full_name)
            }
        })
        .unwrap_or_else(|| format!("Telegram {chat_type}"));

    Some(TelegramChatCandidate {
        id,
        title,
        chat_type,
    })
}

async fn test_webhook_target(target: &NotificationTargetConfig) -> Result<(), String> {
    let message = render_test_message(target);
    send_webhook_message(target, "CodexDeck 通知测试", &message).await
}

async fn send_webhook_message(
    target: &NotificationTargetConfig,
    title: &str,
    message: &str,
) -> Result<(), String> {
    let webhook_url = required_field(target.webhook_url.as_deref(), "请填写 Webhook URL")?;
    let url =
        reqwest::Url::parse(webhook_url).map_err(|error| format!("Webhook URL 无效: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Webhook URL 仅支持 http/https".to_string());
    }

    let client = build_client()?;
    let response = client
        .post(url)
        .json(&json!({
            "title": title,
            "message": message,
            "source": "CodexDeck"
        }))
        .send()
        .await
        .map_err(|error| format!("连接 Webhook 失败: {}", sanitize_reqwest_error(error)))?;

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!(
        "Webhook 返回失败状态 {status}: {}",
        summarize_response_body(&body)
    ))
}

async fn send_notification_message(
    target: &NotificationTargetConfig,
    title: &str,
    message: &str,
) -> Result<(), String> {
    match target.kind {
        NotificationTargetKind::Telegram => send_telegram_message(target, message).await,
        NotificationTargetKind::Webhook => send_webhook_message(target, title, message).await,
    }
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| format!("创建通知客户端失败: {error}"))
}

fn required_field<'a>(value: Option<&'a str>, message: &str) -> Result<&'a str, String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| message.to_string())
}

fn unwrap_data(payload: &Value) -> &Value {
    let data = payload.get("data");
    if payload
        .get("code")
        .is_some_and(|value| value == 0 || value == "0" || value == true || value == "true")
        && data.is_some()
    {
        return data.unwrap_or(payload);
    }

    if data.is_some()
        && (payload
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || payload.get("ok").and_then(Value::as_bool).unwrap_or(false)
            || payload
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case("success")))
    {
        return data.unwrap_or(payload);
    }

    if data.is_some()
        && payload
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|value| {
                let value = value.trim();
                value.eq_ignore_ascii_case("ok") || value.eq_ignore_ascii_case("success")
            })
    {
        return data.unwrap_or(payload);
    }

    payload
}

fn parse_json_body(body: &str, message: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(body).map_err(|error| format!("{message}: {error}"))
}

fn mask_email(value: &str) -> String {
    let email = value.trim();
    let Some((name, domain)) = email.split_once('@') else {
        if email.chars().count() <= 2 {
            return "***".to_string();
        }
        return format!("{}***", email.chars().take(2).collect::<String>());
    };
    let masked_name = if name.chars().count() <= 2 {
        format!("{}***", name.chars().next().unwrap_or('*'))
    } else {
        let first = name.chars().take(2).collect::<String>();
        let last = name.chars().last().unwrap_or('*');
        format!("{first}***{last}")
    };
    format!("{masked_name}@{domain}")
}

fn user_label(user: &Value) -> String {
    user.get("email")
        .or_else(|| user.get("username"))
        .or_else(|| user.get("id"))
        .and_then(Value::as_str)
        .map(mask_email)
        .unwrap_or_else(|| "已登录账号".to_string())
}

fn provider_display_name(provider: &NotificationProviderConfig) -> String {
    let name = provider.name.trim();
    if name.is_empty() {
        "API 平台".to_string()
    } else {
        name.to_string()
    }
}

fn normalize_sub2api_base_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    trimmed.to_string()
}

fn normalize_openai_compatible_base_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    trimmed
        .strip_suffix("/api/v1")
        .or_else(|| trimmed.strip_suffix("/v1"))
        .unwrap_or(trimmed)
        .to_string()
}

fn sub2api_base_url_candidates(value: &str) -> Result<Vec<String>, String> {
    let normalized = normalize_sub2api_base_url(value);
    let mut url =
        reqwest::Url::parse(&normalized).map_err(|error| format!("API 平台 URL 无效: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("API 平台 URL 仅支持 http/https".to_string());
    }
    url.set_query(None);
    url.set_fragment(None);

    let mut candidates = Vec::new();
    push_unique_candidate(&mut candidates, clean_base_url(url.clone()));

    let path = url.path().trim_end_matches('/');
    if path.is_empty() || path == "/" {
        push_unique_candidate(&mut candidates, origin_with_path(&url, "/api/v1"));
        push_unique_candidate(&mut candidates, origin_with_path(&url, "/v1"));
    } else if path.eq_ignore_ascii_case("/v1") {
        push_unique_candidate(&mut candidates, origin_with_path(&url, "/api/v1"));
    } else if !path.eq_ignore_ascii_case("/api/v1") {
        push_unique_candidate(&mut candidates, origin_with_path(&url, "/api/v1"));
        push_unique_candidate(&mut candidates, origin_with_path(&url, "/v1"));
    }

    Ok(candidates)
}

fn push_unique_candidate(candidates: &mut Vec<String>, candidate: String) {
    if !candidates
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        candidates.push(candidate);
    }
}

fn origin_with_path(url: &reqwest::Url, path: &str) -> String {
    let mut candidate = url.clone();
    candidate.set_path(path);
    candidate.set_query(None);
    candidate.set_fragment(None);
    clean_base_url(candidate)
}

fn clean_base_url(url: reqwest::Url) -> String {
    url.as_str().trim_end_matches('/').to_string()
}

fn should_retry_sub2api_base_candidate(status: reqwest::StatusCode, body: &str) -> bool {
    matches!(
        status,
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
    ) || body_looks_like_non_api_response(body)
}

fn body_looks_like_non_api_response(body: &str) -> bool {
    let trimmed = body.trim_start();
    trimmed.starts_with("<!doctype")
        || trimmed.starts_with("<html")
        || !(trimmed.starts_with('{') || trimmed.starts_with('['))
}

fn looks_like_sub2api_auth_payload(payload: &Value) -> bool {
    let data = unwrap_data(payload);
    data.get("access_token").is_some()
        || data.get("accessToken").is_some()
        || data.get("token").is_some()
        || data.get("jwt").is_some()
        || data.get("temp_token").is_some()
        || data.get("requires_2fa").is_some()
        || payload.get("message").is_some()
        || payload.get("error").is_some()
}

fn normalize_multiplier(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value.clamp(0.0001, 1000.0)
    } else {
        crate::models::default_notification_cost_multiplier()
    }
}

fn json_number(value: Option<&Value>) -> f64 {
    value
        .and_then(|item| {
            item.as_f64()
                .or_else(|| item.as_i64().map(|number| number as f64))
                .or_else(|| item.as_u64().map(|number| number as f64))
                .or_else(|| item.as_str().and_then(|raw| raw.trim().parse::<f64>().ok()))
        })
        .unwrap_or(0.0)
}

fn json_i64(value: Option<&Value>) -> i64 {
    value
        .and_then(|item| {
            item.as_i64()
                .or_else(|| item.as_u64().and_then(|number| i64::try_from(number).ok()))
                .or_else(|| {
                    item.as_f64().map(|number| {
                        if number.is_finite() {
                            number.round() as i64
                        } else {
                            0
                        }
                    })
                })
                .or_else(|| {
                    item.as_str()
                        .and_then(|raw| raw.trim().parse::<f64>().ok())
                        .map(|number| number.round() as i64)
                })
        })
        .unwrap_or(0)
}

fn json_optional_number(value: Option<&Value>) -> Option<f64> {
    value.and_then(|item| {
        item.as_f64()
            .or_else(|| item.as_i64().map(|number| number as f64))
            .or_else(|| item.as_u64().map(|number| number as f64))
            .or_else(|| item.as_str().and_then(|raw| raw.trim().parse::<f64>().ok()))
            .filter(|number| number.is_finite())
    })
}

fn json_optional_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(|item| {
        item.as_i64()
            .or_else(|| item.as_u64().and_then(|number| i64::try_from(number).ok()))
            .or_else(|| {
                item.as_f64().and_then(|number| {
                    if number.is_finite() {
                        Some(number.round() as i64)
                    } else {
                        None
                    }
                })
            })
            .or_else(|| {
                item.as_str()
                    .and_then(|raw| raw.trim().parse::<f64>().ok())
                    .map(|number| number.round() as i64)
            })
    })
}

fn money_text(value: f64) -> String {
    format!("${:.2}", value.max(0.0))
}

fn quota_text(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{:.0}", value.max(0.0))
    } else {
        let text = format!("{:.4}", value.max(0.0));
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn parse_api_time_to_unix(value: Option<&Value>) -> Option<i64> {
    let item = value?;
    if let Some(number) = json_optional_i64(Some(item)) {
        if number > 1_000_000_000_000 {
            return Some(number / 1000);
        }
        if number > 1_000_000_000 {
            return Some(number);
        }
    }

    let raw = item.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    OffsetDateTime::parse(raw, &Rfc3339)
        .ok()
        .map(|time| time.unix_timestamp())
}

fn subscription_payload(item: &Value) -> &Value {
    item.get("subscription")
        .and_then(|value| value.as_object().map(|_| value))
        .unwrap_or(item)
}

fn subscription_progress_payload(item: &Value) -> Option<&Value> {
    item.get("progress")
        .and_then(|value| value.as_object().map(|_| value))
        .or_else(|| item.as_object().map(|_| item))
}

fn progress_window(progress: Option<&Value>, key: &str) -> Option<UsageWindow> {
    let payload = progress?.get(key)?;
    if payload.is_null() {
        return None;
    }
    let used_percent = json_optional_number(payload.get("percentage")).unwrap_or_else(|| {
        let used = json_optional_number(payload.get("used"))
            .or_else(|| json_optional_number(payload.get("used_usd")))
            .unwrap_or(0.0);
        let limit = json_optional_number(payload.get("limit"))
            .or_else(|| json_optional_number(payload.get("limit_usd")))
            .unwrap_or(0.0);
        if limit > 0.0 {
            (used / limit) * 100.0
        } else {
            0.0
        }
    });
    let reset_at = json_optional_i64(payload.get("reset_in_seconds"))
        .or_else(|| json_optional_i64(payload.get("resets_in_seconds")))
        .filter(|seconds| *seconds > 0)
        .map(|seconds| crate::utils::now_unix_seconds() + seconds)
        .or_else(|| parse_api_time_to_unix(payload.get("resets_at")))
        .or_else(|| parse_api_time_to_unix(payload.get("reset_at")));

    Some(UsageWindow {
        used_percent: used_percent.clamp(0.0, 100.0),
        window_seconds: 0,
        reset_at,
    })
}

fn choose_best_subscription(progress_items: &[Value]) -> Option<&Value> {
    progress_items
        .iter()
        .find(|item| {
            subscription_payload(item)
                .get("status")
                .and_then(Value::as_str)
                .is_none_or(|status| status.eq_ignore_ascii_case("active"))
        })
        .or_else(|| progress_items.first())
}

fn build_api_quota_snapshot(
    provider: &NotificationProviderConfig,
    user: &Value,
    stats: &Value,
    progress_items: &[Value],
) -> ApiQuotaSnapshot {
    let multiplier = normalize_multiplier(provider.cost_multiplier);
    let balance = json_optional_number(user.get("balance"))
        .map(|value| value * multiplier)
        .or_else(|| {
            json_optional_number(stats.get("balance"))
                .or_else(|| json_optional_number(stats.get("remaining_balance")))
                .map(|value| value * multiplier)
        });
    let today_cost = json_optional_number(
        stats
            .get("today_actual_cost")
            .or_else(|| stats.get("today_cost")),
    )
    .map(|value| value * multiplier);
    let base_snapshot = ApiQuotaSnapshot {
        mode: crate::models::ApiQuotaMode::PlatformBasic,
        today_used_text: today_cost.map(money_text),
        remaining_text: balance.map(money_text),
        total_remaining_text: balance.map(money_text),
        total_tokens_text: Some(format_tokens(json_i64(stats.get("total_tokens")))),
        today_tokens_text: Some(format_tokens(json_i64(stats.get("today_tokens")))),
        daily_window: None,
        total_window: None,
        subscription_expires_at: None,
    };

    let Some(subscription_item) = choose_best_subscription(progress_items) else {
        return base_snapshot;
    };

    let subscription = subscription_payload(subscription_item);
    let progress = subscription_progress_payload(subscription_item);
    let daily_window = progress_window(progress, "daily");
    let total_window = progress_window(progress, "monthly")
        .or_else(|| progress_window(progress, "weekly"))
        .or_else(|| {
            let monthly_used = json_optional_number(subscription.get("monthly_usage_usd"));
            let monthly_limit = subscription
                .get("group")
                .and_then(|group| json_optional_number(group.get("monthly_limit_usd")))
                .or_else(|| json_optional_number(subscription.get("monthly_limit_usd")));
            match (monthly_used, monthly_limit) {
                (Some(used), Some(limit)) if limit > 0.0 => Some(UsageWindow {
                    used_percent: ((used / limit) * 100.0).clamp(0.0, 100.0),
                    window_seconds: 0,
                    reset_at: None,
                }),
                _ => None,
            }
        });
    let subscription_expires_at = progress
        .and_then(|payload| parse_api_time_to_unix(payload.get("expires_at")))
        .or_else(|| parse_api_time_to_unix(subscription_item.get("expires_at")))
        .or_else(|| parse_api_time_to_unix(subscription.get("expires_at")));

    if daily_window.is_none() && total_window.is_none() && subscription_expires_at.is_none() {
        return base_snapshot;
    }

    ApiQuotaSnapshot {
        mode: crate::models::ApiQuotaMode::PlatformSubscription,
        today_used_text: base_snapshot.today_used_text,
        remaining_text: base_snapshot.remaining_text,
        total_remaining_text: base_snapshot.total_remaining_text,
        total_tokens_text: base_snapshot.total_tokens_text,
        today_tokens_text: base_snapshot.today_tokens_text,
        daily_window,
        total_window,
        subscription_expires_at,
    }
}

fn build_newapi_token_quota_snapshot(payload: &Value) -> Option<ApiQuotaSnapshot> {
    let total_used = json_optional_number(payload.get("total_used"))
        .or_else(|| json_optional_number(payload.get("used_quota")))
        .or_else(|| json_optional_number(payload.get("used")));
    let total_granted = json_optional_number(payload.get("total_granted"))
        .or_else(|| json_optional_number(payload.get("total_quota")))
        .or_else(|| json_optional_number(payload.get("quota")));
    let total_available = json_optional_number(payload.get("total_available"))
        .or_else(|| json_optional_number(payload.get("remaining_quota")))
        .or_else(|| json_optional_number(payload.get("available_quota")))
        .or_else(|| json_optional_number(payload.get("balance")));
    let has_any_quota =
        total_used.is_some() || total_granted.is_some() || total_available.is_some();
    if !has_any_quota {
        return None;
    }

    let used = total_used.unwrap_or(0.0).max(0.0);
    let granted = total_granted.unwrap_or_else(|| used + total_available.unwrap_or(0.0));
    let available = total_available.unwrap_or_else(|| (granted - used).max(0.0));
    let unlimited = payload
        .get("unlimited_quota")
        .or_else(|| payload.get("unlimited"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let used_percent = if unlimited || granted <= 0.0 {
        0.0
    } else {
        ((used / granted) * 100.0).clamp(0.0, 100.0)
    };

    Some(ApiQuotaSnapshot {
        mode: crate::models::ApiQuotaMode::ApiOnly,
        today_used_text: None,
        remaining_text: Some(if unlimited {
            "不限量".to_string()
        } else {
            quota_text(available)
        }),
        total_remaining_text: Some(if unlimited {
            "不限量".to_string()
        } else {
            quota_text(available)
        }),
        total_tokens_text: total_granted.map(quota_text),
        today_tokens_text: total_used.map(quota_text),
        daily_window: None,
        total_window: Some(UsageWindow {
            used_percent,
            window_seconds: 0,
            reset_at: parse_api_time_to_unix(payload.get("expires_at"))
                .or_else(|| parse_api_time_to_unix(payload.get("expire_time")))
                .or_else(|| parse_api_time_to_unix(payload.get("expired_at"))),
        }),
        subscription_expires_at: parse_api_time_to_unix(payload.get("expires_at"))
            .or_else(|| parse_api_time_to_unix(payload.get("expire_time")))
            .or_else(|| parse_api_time_to_unix(payload.get("expired_at"))),
    })
}

fn report_date_text() -> String {
    let generated_time = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "2026-05-06T00:00:00Z".to_string());
    generated_time.get(..10).unwrap_or("2026-05-06").to_string()
}

fn build_aggregate_report(snapshots: &[ProviderUsageSnapshot]) -> String {
    let generated_time = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "当前时间".to_string());
    let report_date = generated_time.get(..10).unwrap_or("今日");
    let total_today_cost = snapshots
        .iter()
        .map(|snapshot| snapshot.effective_today_cost)
        .sum::<f64>();
    let total_cost = snapshots
        .iter()
        .map(|snapshot| snapshot.effective_total_cost)
        .sum::<f64>();
    let total_balance = snapshots
        .iter()
        .map(|snapshot| snapshot.effective_balance)
        .sum::<f64>();
    let today_requests = snapshots
        .iter()
        .map(|snapshot| snapshot.today_requests)
        .sum::<i64>();
    let today_tokens = snapshots
        .iter()
        .map(|snapshot| snapshot.today_tokens)
        .sum::<i64>();
    let total_requests = snapshots
        .iter()
        .map(|snapshot| snapshot.total_requests)
        .sum::<i64>();
    let total_tokens = snapshots
        .iter()
        .map(|snapshot| snapshot.total_tokens)
        .sum::<i64>();
    let available_total = total_cost + total_balance;
    let progress = if available_total > 0.0 {
        total_cost / available_total
    } else {
        0.0
    };
    let balance_ratio = if total_balance > 0.0 {
        total_today_cost / total_balance
    } else {
        0.0
    };

    let provider_lines = snapshots
        .iter()
        .take(12)
        .map(|snapshot| {
            format!(
                "- {} x{}：今日 {} -> {} / 余额 {} / {}",
                snapshot.provider_name,
                format_multiplier(snapshot.multiplier),
                money(snapshot.today_cost),
                money(snapshot.effective_today_cost),
                money(snapshot.effective_balance),
                snapshot.account_label
            )
        })
        .chain(
            (snapshots.len() > 12)
                .then(|| format!("- 其余 {} 个平台已纳入总计", snapshots.len() - 12)),
        )
        .collect::<Vec<_>>();

    let mut model_totals = BTreeMap::<String, (f64, i64)>::new();
    for snapshot in snapshots {
        for model in &snapshot.models {
            let entry = model_totals.entry(model.model.clone()).or_insert((0.0, 0));
            entry.0 += model.effective_cost;
            entry.1 += model.requests;
        }
    }
    let mut model_lines = model_totals
        .into_iter()
        .map(|(model, (cost, requests))| (model, cost, requests))
        .collect::<Vec<_>>();
    model_lines.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let model_lines = model_lines
        .into_iter()
        .take(5)
        .map(|(model, cost, requests)| {
            format!(
                "- {model}: {} / {}次",
                money(cost),
                format_requests(requests)
            )
        })
        .collect::<Vec<_>>();
    let model_lines = if model_lines.is_empty() {
        vec!["- 暂无模型统计".to_string()]
    } else {
        model_lines
    };

    [
        format!("📊 CodexDeck 聚合额度日报 · {report_date}"),
        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
        format!("平台数：{}", snapshots.len()),
        format!(
            "{} 开销进度 {}",
            progress_bar(progress, 24),
            ratio(progress)
        ),
        format!(
            "统一倍率后累计开销: {} / 可用总额 {}",
            money(total_cost),
            money(available_total)
        ),
        format!("统一倍率后今日开销: {}", money(total_today_cost)),
        format!("统一倍率后当前余额: {}", money(total_balance)),
        format!(
            "今日请求/Token: {} / {}",
            format_requests(today_requests),
            format_tokens(today_tokens)
        ),
        format!(
            "累计请求/Token: {} / {}",
            format_requests(total_requests),
            format_tokens(total_tokens)
        ),
        format!("今日开销占当前余额比例: {}", ratio(balance_ratio)),
        "平台明细:".to_string(),
        provider_lines.join("\n"),
        "主要模型开销:".to_string(),
        model_lines.join("\n"),
        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
        format!("生成时间: {generated_time}"),
    ]
    .join("\n")
}

fn render_usage_template(
    target: &NotificationTargetConfig,
    snapshots: &[ProviderUsageSnapshot],
) -> String {
    let fallback = build_aggregate_report(snapshots);
    let template = target.message_template.trim();
    if template.is_empty() {
        return fallback;
    }

    let generated_time = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "当前时间".to_string());
    let report_date = generated_time.get(..10).unwrap_or("今日");
    let total_today_cost = snapshots
        .iter()
        .map(|snapshot| snapshot.effective_today_cost)
        .sum::<f64>();
    let total_cost = snapshots
        .iter()
        .map(|snapshot| snapshot.effective_total_cost)
        .sum::<f64>();
    let total_balance = snapshots
        .iter()
        .map(|snapshot| snapshot.effective_balance)
        .sum::<f64>();
    let today_requests = snapshots
        .iter()
        .map(|snapshot| snapshot.today_requests)
        .sum::<i64>();
    let today_tokens = snapshots
        .iter()
        .map(|snapshot| snapshot.today_tokens)
        .sum::<i64>();
    let total_requests = snapshots
        .iter()
        .map(|snapshot| snapshot.total_requests)
        .sum::<i64>();
    let total_tokens = snapshots
        .iter()
        .map(|snapshot| snapshot.total_tokens)
        .sum::<i64>();
    let available_total = total_cost + total_balance;
    let progress = if available_total > 0.0 {
        total_cost / available_total
    } else {
        0.0
    };
    let balance_ratio = if total_balance > 0.0 {
        total_today_cost / total_balance
    } else {
        0.0
    };
    let primary = snapshots.first();
    let provider_name = if snapshots.len() == 1 {
        primary
            .map(|snapshot| snapshot.provider_name.clone())
            .unwrap_or_else(|| "API 平台".to_string())
    } else {
        format!("{} 个平台聚合", snapshots.len())
    };

    let mut model_totals = BTreeMap::<String, (f64, i64)>::new();
    for snapshot in snapshots {
        for model in &snapshot.models {
            let entry = model_totals.entry(model.model.clone()).or_insert((0.0, 0));
            entry.0 += model.effective_cost;
            entry.1 += model.requests;
        }
    }
    let mut model_lines = model_totals
        .into_iter()
        .map(|(model, (cost, requests))| (model, cost, requests))
        .collect::<Vec<_>>();
    model_lines.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let model_cost_lines = model_lines
        .into_iter()
        .take(5)
        .map(|(model, cost, requests)| {
            format!(
                "- {model}: {} / {}次",
                money(cost),
                format_requests(requests)
            )
        })
        .collect::<Vec<_>>();
    let model_cost_lines = if model_cost_lines.is_empty() {
        "- 暂无模型统计".to_string()
    } else {
        model_cost_lines.join("\n")
    };

    template
        .replace("{target}", target.name.trim())
        .replace("{time}", &generated_time)
        .replace(
            "{reportTitle}",
            if target.aggregate_enabled {
                "CodexDeck 聚合额度日报"
            } else {
                "CodexDeck 额度日报"
            },
        )
        .replace("{reportDate}", report_date)
        .replace("{providerName}", &provider_name)
        .replace("{progressBar}", &progress_bar(progress, 24))
        .replace("{usageProgress}", &ratio(progress))
        .replace("{totalCost}", &money(total_cost))
        .replace("{availableTotal}", &money(available_total))
        .replace("{todayCost}", &money(total_today_cost))
        .replace("{balance}", &format!("{:.2}", total_balance))
        .replace("{todayRequests}", &format_requests(today_requests))
        .replace("{todayTokens}", &format_tokens(today_tokens))
        .replace("{totalRequests}", &format_requests(total_requests))
        .replace("{totalTokens}", &format_tokens(total_tokens))
        .replace("{todayBalanceRatio}", &ratio(balance_ratio))
        .replace("{previousDelta}", "$0.00")
        .replace("{modelCostLines}", &model_cost_lines)
        .replace("{generatedTime}", &generated_time)
        .replace(
            "{account}",
            primary
                .map(|snapshot| snapshot.account_label.as_str())
                .unwrap_or("API 平台"),
        )
        .replace("{window}", "今日")
        .replace("{remaining}", &money(total_balance))
        .replace("{resetTime}", "不适用")
        .replace("{error}", "")
}

fn money(value: f64) -> String {
    format!("${value:.2}")
}

fn ratio(value: f64) -> String {
    format!("{:.1}%", value.clamp(0.0, 1000.0) * 100.0)
}

fn progress_bar(progress: f64, width: usize) -> String {
    let normalized = progress.clamp(0.0, 1.0);
    let filled = (normalized * width as f64).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(width.saturating_sub(filled))
    )
}

fn format_multiplier(value: f64) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn format_requests(value: i64) -> String {
    value.max(0).to_string()
}

fn format_tokens(value: i64) -> String {
    let number = value.max(0) as f64;
    if number >= 100_000_000.0 {
        return format!("{:.2}亿", number / 100_000_000.0);
    }
    if number >= 10_000.0 {
        return format!("{:.1}万", number / 10_000.0);
    }
    format_requests(value)
}

fn render_test_message(target: &NotificationTargetConfig) -> String {
    let template = if target.message_template.trim().is_empty() {
        crate::models::default_notification_message_template()
    } else {
        target.message_template.clone()
    };
    let time = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "当前时间".to_string());

    template
        .replace("{target}", target.name.trim())
        .replace("{time}", &time)
        .replace("{reportTitle}", "Sub2API 消耗日报")
        .replace("{reportDate}", "2026-05-06")
        .replace("{providerName}", "示例中转平台")
        .replace("{progressBar}", "██████░░░░░░░░░░░░░░░░░░")
        .replace("{usageProgress}", "25.0%")
        .replace("{totalCost}", "$12.34")
        .replace("{availableTotal}", "$49.36")
        .replace("{todayCost}", "$1.23")
        .replace("{balance}", "$37.02")
        .replace("{todayRequests}", "128")
        .replace("{todayTokens}", "45.8万")
        .replace("{totalRequests}", "1,024")
        .replace("{totalTokens}", "320.4万")
        .replace("{todayBalanceRatio}", "3.3%")
        .replace("{previousDelta}", "$0.88")
        .replace(
            "{modelCostLines}",
            "- gpt-5.5: $0.92 / 18次\n- gpt-5.4: $0.31 / 9次",
        )
        .replace("{generatedTime}", "12:00:00 +0800")
        .replace("{account}", "演示账号")
        .replace("{window}", "5h")
        .replace("{remaining}", "12%")
        .replace("{resetTime}", "约 2 小时后")
        .replace("{error}", "演示错误：授权刷新失败")
}

fn sanitize_reqwest_error(error: reqwest::Error) -> String {
    error.without_url().to_string()
}

fn summarize_response_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "响应体为空".to_string();
    }

    const MAX_LEN: usize = 240;
    if trimmed.chars().count() <= MAX_LEN {
        return trimmed.to_string();
    }

    let summary = trimmed.chars().take(MAX_LEN).collect::<String>();
    format!("{summary}...")
}

fn summarize_api_error_body(body: &str) -> Option<String> {
    let payload = serde_json::from_str::<Value>(body).ok()?;
    ["message", "error", "detail", "reason"]
        .iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::models::NotificationProviderConfig;

    use super::body_looks_like_non_api_response;
    use super::build_api_quota_snapshot;
    use super::build_newapi_token_quota_snapshot;
    use super::normalize_openai_compatible_base_url;
    use super::should_retry_sub2api_base_candidate;
    use super::sub2api_base_url_candidates;

    #[test]
    fn sub2api_candidates_try_api_v1_for_root_site_url() {
        let candidates = sub2api_base_url_candidates("https://gateway.example.invalid").unwrap();

        assert_eq!(
            candidates,
            vec![
                "https://gateway.example.invalid",
                "https://gateway.example.invalid/api/v1",
                "https://gateway.example.invalid/v1",
            ]
        );
    }

    #[test]
    fn sub2api_candidates_keep_explicit_api_v1_first() {
        let candidates = sub2api_base_url_candidates("https://gateway.example.invalid/api/v1/").unwrap();

        assert_eq!(candidates, vec!["https://gateway.example.invalid/api/v1"]);
    }

    #[test]
    fn sub2api_candidates_try_api_v1_sibling_for_v1_url() {
        let candidates = sub2api_base_url_candidates("https://gateway.example.invalid/v1").unwrap();

        assert_eq!(
            candidates,
            vec!["https://gateway.example.invalid/v1", "https://gateway.example.invalid/api/v1"]
        );
    }

    #[test]
    fn sub2api_retry_detects_html_site_page_as_wrong_base() {
        assert!(body_looks_like_non_api_response(
            "<!doctype html><html></html>"
        ));
        assert!(should_retry_sub2api_base_candidate(
            reqwest::StatusCode::OK,
            "<!doctype html><html></html>"
        ));
    }

    #[test]
    fn sub2api_retry_keeps_json_unauthorized_as_real_auth_error() {
        assert!(!body_looks_like_non_api_response(
            r#"{"code":"INVALID_CREDENTIALS","message":"invalid email or password"}"#
        ));
        assert!(!should_retry_sub2api_base_candidate(
            reqwest::StatusCode::UNAUTHORIZED,
            r#"{"code":"INVALID_CREDENTIALS","message":"invalid email or password"}"#
        ));
    }

    #[test]
    fn newapi_quota_uses_root_site_for_openai_compatible_base_url() {
        assert_eq!(
            normalize_openai_compatible_base_url("https://newapi.example.com/v1/"),
            "https://newapi.example.com"
        );
        assert_eq!(
            normalize_openai_compatible_base_url("https://newapi.example.com/api/v1"),
            "https://newapi.example.com"
        );
    }

    #[test]
    fn newapi_token_usage_maps_total_available_payload() {
        let snapshot = build_newapi_token_quota_snapshot(&json!({
            "token_name": "CodexDeck",
            "total_granted": 1000000,
            "total_used": 250000,
            "total_available": 750000,
            "unlimited_quota": false,
            "expires_at": "2099-01-02T03:04:05Z"
        }))
        .expect("newapi quota snapshot");

        assert_eq!(snapshot.mode, crate::models::ApiQuotaMode::ApiOnly);
        assert_eq!(snapshot.remaining_text.as_deref(), Some("750000"));
        assert_eq!(snapshot.total_remaining_text.as_deref(), Some("750000"));
        assert_eq!(snapshot.total_tokens_text.as_deref(), Some("1000000"));
        assert_eq!(snapshot.today_tokens_text.as_deref(), Some("250000"));
        assert_eq!(snapshot.total_window.unwrap().used_percent, 25.0);
        assert!(snapshot.subscription_expires_at.is_some());
    }

    #[test]
    fn unwrap_data_accepts_newapi_boolean_code_payload() {
        let payload = json!({
            "code": true,
            "message": "ok",
            "data": {
                "total_granted": 500000,
                "total_used": 12003529,
                "total_available": -11503529,
                "unlimited_quota": true
            }
        });

        let data = super::unwrap_data(&payload);
        let snapshot = build_newapi_token_quota_snapshot(data).expect("newapi quota snapshot");

        assert_eq!(snapshot.mode, crate::models::ApiQuotaMode::ApiOnly);
        assert_eq!(snapshot.remaining_text.as_deref(), Some("不限量"));
        assert_eq!(snapshot.total_tokens_text.as_deref(), Some("500000"));
        assert_eq!(snapshot.today_tokens_text.as_deref(), Some("12003529"));
    }

    #[test]
    fn quota_snapshot_maps_subscription_progress_to_usage_windows() {
        let provider = NotificationProviderConfig {
            id: "provider-1".to_string(),
            name: "Alex".to_string(),
            kind: Default::default(),
            enabled: true,
            cost_multiplier: 1.0,
            base_url: "https://gateway.example.invalid/api/v1".to_string(),
            email: "alex@example.com".to_string(),
            password: Some("secret".to_string()),
            created_at: 0,
            updated_at: 0,
            last_test_at: None,
            last_test_error: None,
        };
        let user = json!({ "balance": 37.02 });
        let stats = json!({
            "today_actual_cost": 4.28,
            "today_tokens": 486000,
            "total_tokens": 12800000
        });
        let progress = vec![json!({
            "subscription": {
                "status": "active",
                "expires_at": "2099-01-02T03:04:05Z"
            },
            "progress": {
                "daily": {
                    "used": 5.7,
                    "limit": 10.0,
                    "percentage": 57.0,
                    "reset_in_seconds": 3600
                },
                "monthly": {
                    "used": 38.0,
                    "limit": 100.0,
                    "percentage": 38.0,
                    "reset_in_seconds": null
                },
                "expires_at": "2099-01-02T03:04:05Z"
            }
        })];

        let snapshot = build_api_quota_snapshot(&provider, &user, &stats, &progress);

        assert_eq!(
            snapshot.mode,
            crate::models::ApiQuotaMode::PlatformSubscription
        );
        assert_eq!(snapshot.today_used_text.as_deref(), Some("$4.28"));
        assert_eq!(snapshot.remaining_text.as_deref(), Some("$37.02"));
        assert_eq!(snapshot.daily_window.unwrap().used_percent, 57.0);
        assert_eq!(snapshot.total_window.unwrap().used_percent, 38.0);
        assert!(snapshot.subscription_expires_at.is_some());
        assert_eq!(snapshot.today_tokens_text.as_deref(), Some("48.6万"));
        assert_eq!(snapshot.total_tokens_text.as_deref(), Some("1280.0万"));
    }

    #[test]
    fn quota_snapshot_accepts_sub2api_progress_field_names() {
        let provider = NotificationProviderConfig {
            id: "provider-1".to_string(),
            name: "Alex".to_string(),
            kind: Default::default(),
            enabled: true,
            cost_multiplier: 1.0,
            base_url: "https://gateway.example.invalid/api/v1".to_string(),
            email: "alex@example.com".to_string(),
            password: Some("secret".to_string()),
            created_at: 0,
            updated_at: 0,
            last_test_at: None,
            last_test_error: None,
        };
        let user = json!({ "balance": 18.64 });
        let stats = json!({
            "today_actual_cost": 4.28,
            "today_tokens": 486000,
            "total_tokens": 12800000
        });
        let progress = vec![json!({
            "subscription": {
                "status": "active",
                "expires_at": "2099-01-31T12:59:14Z"
            },
            "progress": {
                "id": 7,
                "group_name": "Pro",
                "expires_at": "2099-01-31T12:59:14Z",
                "daily": {
                    "limit_usd": 10.0,
                    "used_usd": 5.7,
                    "remaining_usd": 4.3,
                    "percentage": 57.0,
                    "resets_in_seconds": 3600
                },
                "monthly": {
                    "limit_usd": 100.0,
                    "used_usd": 38.0,
                    "remaining_usd": 62.0,
                    "percentage": 38.0,
                    "resets_in_seconds": 86400
                }
            }
        })];

        let snapshot = build_api_quota_snapshot(&provider, &user, &stats, &progress);

        assert_eq!(
            snapshot.mode,
            crate::models::ApiQuotaMode::PlatformSubscription
        );
        assert_eq!(snapshot.today_used_text.as_deref(), Some("$4.28"));
        assert_eq!(snapshot.remaining_text.as_deref(), Some("$18.64"));
        assert_eq!(snapshot.daily_window.unwrap().used_percent, 57.0);
        assert_eq!(snapshot.total_window.unwrap().used_percent, 38.0);
        assert!(snapshot.subscription_expires_at.is_some());
    }
}
