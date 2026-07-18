use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::extract::Path;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::post;
use axum::Router;
use futures_util::StreamExt;
use reqwest::Url;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::models::AccountsStore;
use crate::models::ModelRouterRouteSelection;
use crate::models::RelayModelCatalogEntry;
use crate::models::StoredAccount;
use crate::profile_files;
use crate::state::AppState;
use crate::utils::redact_sensitive_text;

const ROUTER_REQUEST_TIMEOUT_SECS: u64 = 300;
const MAX_MODEL_ROUTER_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct ModelRouterHandle {
    pub(crate) base_url: String,
    pub(crate) routes: Vec<ModelRouterRoute>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl ModelRouterHandle {
    pub(crate) fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for ModelRouterHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelRouterRoute {
    pub(crate) model: String,
    pub(crate) display_name: Option<String>,
    pub(crate) request_model: String,
    pub(crate) context_window: Option<u32>,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) source_label: String,
}

#[derive(Clone)]
struct RouterState {
    routes_by_model: HashMap<String, ModelRouterRoute>,
    ordered_routes: Vec<ModelRouterRoute>,
    client: reqwest::Client,
}

pub(crate) fn build_routes_from_accounts(
    accounts: &[StoredAccount],
    route_selections: &[ModelRouterRouteSelection],
) -> Vec<ModelRouterRoute> {
    let selected_routes = route_selections
        .iter()
        .map(|selection| (selection.account_id.as_str(), selection.model.as_str()))
        .collect::<HashSet<_>>();
    let mut routes = Vec::new();
    for account in accounts {
        if !matches!(account.source_kind, crate::models::AccountSourceKind::Relay) {
            continue;
        }
        let Some(base_url) = account
            .api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| profile_files::normalize_relay_base_url(value).ok())
        else {
            continue;
        };
        let Some(api_key) = account.primary_relay_api_key() else {
            continue;
        };
        for entry in account.enabled_model_catalog() {
            if !selected_routes.is_empty()
                && !selected_routes.contains(&(account.id.as_str(), entry.model.as_str()))
            {
                continue;
            }
            routes.push(ModelRouterRoute {
                model: entry.model.clone(),
                display_name: entry.display_name.clone(),
                request_model: entry.request_model_or_model().to_string(),
                context_window: entry.context_window,
                base_url: base_url.clone(),
                api_key: api_key.to_string(),
                source_label: account.label.clone(),
            });
        }
    }
    routes
}

pub(crate) async fn start_model_router(
    routes: Vec<ModelRouterRoute>,
) -> Result<ModelRouterHandle, String> {
    if routes.is_empty() {
        return Err("路由模式没有可用模型。".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(ROUTER_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("创建模型路由客户端失败: {error}"))?;
    let mut routes_by_model = HashMap::new();
    let mut ordered_routes = Vec::new();
    for route in routes {
        if routes_by_model.contains_key(&route.model) {
            log::warn!("模型路由存在重复菜单模型 {}，已保留第一条", route.model);
            continue;
        }
        routes_by_model.insert(route.model.clone(), route.clone());
        ordered_routes.push(route);
    }
    if ordered_routes.is_empty() {
        return Err("路由模式没有可用模型。".to_string());
    }

    let state = RouterState {
        routes_by_model,
        ordered_routes: ordered_routes.clone(),
        client,
    };
    let app = Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/responses", post(forward_responses))
        .route("/v1/responses/compact", post(forward_responses_compact))
        .route("/v1/chat/completions", post(forward_chat_completions))
        .route("/v1/:endpoint", post(forward_single_segment_endpoint))
        .layer(DefaultBodyLimit::max(MAX_MODEL_ROUTER_REQUEST_BODY_BYTES))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("启动本地模型路由失败: {error}"))?;
    let addr = listener
        .local_addr()
        .map_err(|error| format!("读取本地模型路由地址失败: {error}"))?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    tauri::async_runtime::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });
        if let Err(error) = server.await {
            log::warn!("本地模型路由已退出: {error}");
        }
    });

    Ok(ModelRouterHandle {
        base_url: format!("http://{addr}/v1"),
        routes: ordered_routes,
        shutdown_tx: Some(shutdown_tx),
    })
}

pub(crate) async fn stop_model_router(state: &AppState) {
    let mut guard = state.model_router.lock().await;
    if let Some(mut handle) = guard.take() {
        handle.shutdown();
    }
}

pub(crate) async fn ensure_model_router_for_store(
    state: &AppState,
    store: &AccountsStore,
) -> Result<(String, Vec<RelayModelCatalogEntry>), String> {
    let routes = build_routes_from_accounts(
        &store.accounts,
        &store.settings.model_router_route_selections,
    );
    if routes.is_empty() {
        return Err("路由模式没有可用模型，请至少启用一个模型映射。".to_string());
    }
    let handle = start_model_router(routes).await?;
    let base_url = handle.base_url.clone();
    let entries = routes_to_model_catalog_entries(&handle.routes);

    let mut guard = state.model_router.lock().await;
    if let Some(mut previous) = guard.take() {
        previous.shutdown();
    }
    *guard = Some(handle);
    Ok((base_url, entries))
}

async fn list_models(State(state): State<RouterState>) -> impl IntoResponse {
    axum::Json(model_list_payload(&state.ordered_routes))
}

fn model_list_payload(routes: &[ModelRouterRoute]) -> Value {
    let data = routes
        .iter()
        .map(|route| {
            serde_json::json!({
                "id": route.model,
                "object": "model",
                "owned_by": "codexdeck",
                "display_name": route.display_name.as_deref().unwrap_or(route.model.as_str())
            })
        })
        .collect::<Vec<_>>();
    let models = routes
        .iter()
        .enumerate()
        .map(|(index, route)| codex_client_model_entry(route, index))
        .collect::<Vec<_>>();
    serde_json::json!({
        "object": "list",
        "data": data,
        "models": models
    })
}

fn codex_client_model_entry(route: &ModelRouterRoute, index: usize) -> Value {
    let display_name = route
        .display_name
        .as_deref()
        .unwrap_or(route.model.as_str());
    let context_window = route.context_window.unwrap_or(128_000);
    serde_json::json!({
        "id": route.model,
        "model": route.model,
        "slug": route.model,
        "name": display_name,
        "displayName": display_name,
        "display_name": display_name,
        "description": format!("CodexDeck route to {}", route.source_label),
        "visibility": "list",
        "hidden": false,
        "supported_in_api": true,
        "context_window": context_window,
        "max_context_window": context_window,
        "priority": 1000 + index,
        "additional_speed_tiers": [],
        "service_tiers": []
    })
}

async fn forward_responses(
    State(state): State<RouterState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    forward_openai_request(state, headers, body, "responses").await
}

async fn forward_responses_compact(
    State(state): State<RouterState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    forward_openai_request(state, headers, body, "responses/compact").await
}

async fn forward_chat_completions(
    State(state): State<RouterState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    forward_openai_request(state, headers, body, "chat/completions").await
}

async fn forward_single_segment_endpoint(
    State(_state): State<RouterState>,
    Path(endpoint): Path<String>,
) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({
            "error": {
                "message": format!("CodexDeck model router does not support /v1/{endpoint}.")
            }
        })),
    )
}

async fn forward_openai_request(
    state: RouterState,
    headers: HeaderMap,
    body: axum::body::Bytes,
    upstream_path: &str,
) -> axum::response::Response {
    let mut payload: Value = match serde_json::from_slice(&body) {
        Ok(payload) => payload,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "error": { "message": format!("Invalid JSON body: {error}") }
                })),
            )
                .into_response();
        }
    };
    let Some(menu_model) = payload
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
    else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": { "message": "Request body must include model." }
            })),
        )
            .into_response();
    };
    let Some(route) = state.routes_by_model.get(&menu_model).cloned() else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": { "message": format!("No CodexDeck route for model {menu_model}.") }
            })),
        )
            .into_response();
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "model".to_string(),
            Value::String(route.request_model.clone()),
        );
    }

    let upstream_url = match join_upstream_url(&route.base_url, upstream_path) {
        Ok(url) => url,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "error": { "message": error }
                })),
            )
                .into_response();
        }
    };
    let request = state
        .client
        .post(upstream_url)
        .bearer_auth(&route.api_key)
        .headers(forward_headers(&headers))
        .json(&payload);
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            let message = redact_sensitive_text(&error.to_string());
            return (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "error": { "message": format!("Upstream request failed: {message}") }
                })),
            )
                .into_response();
        }
    };

    let status = response.status();
    let mut response_builder = axum::response::Response::builder().status(status);
    for (name, value) in response.headers() {
        if should_forward_response_header(name.as_str()) {
            response_builder = response_builder.header(name, value);
        }
    }
    let stream = response
        .bytes_stream()
        .map(|chunk| chunk.map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error)));
    response_builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|error| {
            (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "error": { "message": format!("Build router response failed: {error}") }
                })),
            )
                .into_response()
        })
}

fn join_upstream_url(base_url: &str, path: &str) -> Result<Url, String> {
    let mut base = Url::parse(base_url).map_err(|error| format!("上游 Base URL 无效: {error}"))?;
    let next_path = format!("{}/{}", base.path().trim_end_matches('/'), path);
    base.set_path(&next_path);
    base.set_query(None);
    base.set_fragment(None);
    Ok(base)
}

fn forward_headers(headers: &HeaderMap) -> HeaderMap {
    let mut output = HeaderMap::new();
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "authorization" | "host" | "content-length" | "connection" | "accept-encoding"
        ) {
            continue;
        }
        output.insert(name.clone(), value.clone());
    }
    output
}

fn should_forward_response_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !matches!(
        lower.as_str(),
        "connection" | "content-length" | "transfer-encoding" | "content-encoding"
    )
}

pub(crate) fn routes_to_model_catalog_entries(
    routes: &[ModelRouterRoute],
) -> Vec<RelayModelCatalogEntry> {
    routes
        .iter()
        .map(|route| RelayModelCatalogEntry {
            model: route.model.clone(),
            display_name: route.display_name.clone(),
            request_model: Some(route.request_model.clone()).filter(|value| value != &route.model),
            context_window: route.context_window,
            enabled: true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::build_routes_from_accounts;
    use super::model_list_payload;
    use super::routes_to_model_catalog_entries;
    use super::start_model_router;
    use super::ModelRouterRoute;
    use crate::models::AccountSourceKind;
    use crate::models::ApiQuotaMode;
    use crate::models::ModelRouterRouteSelection;
    use crate::models::ProxyEndpointCapability;
    use crate::models::ProxyKey;
    use crate::models::RelayModelCatalogEntry;
    use crate::models::StoredAccount;
    use serde_json::json;

    fn relay_account_with_id(id: &str, label: &str, model: &str) -> StoredAccount {
        StoredAccount {
            id: id.to_string(),
            label: label.to_string(),
            source_kind: AccountSourceKind::Relay,
            principal_id: Some(format!("relay:{id}")),
            email: None,
            account_id: id.to_string(),
            plan_type: Some("api".to_string()),
            auth_json: json!({}),
            api_base_url: Some("https://api.example.com".to_string()),
            api_key: Some("sk-route".to_string()),
            api_keys: vec![ProxyKey {
                id: "key:route".to_string(),
                label: Some("primary".to_string()),
                secret: Some("sk-route".to_string()),
                enabled: true,
                priority: 100,
                weight: 100,
                health_status: Default::default(),
                last_error: None,
                cooldown_until: None,
                failure_count: 0,
                last_used_at: None,
                updated_at: None,
            }],
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: vec![ProxyEndpointCapability::Responses],
            model_name: Some(model.to_string()),
            model_catalog: vec![
                RelayModelCatalogEntry {
                    model: model.to_string(),
                    display_name: Some(format!("Menu {model}")),
                    request_model: Some(format!("upstream-{model}")),
                    context_window: Some(262144),
                    enabled: true,
                },
                RelayModelCatalogEntry {
                    model: "disabled".to_string(),
                    display_name: None,
                    request_model: None,
                    context_window: None,
                    enabled: false,
                },
            ],
            model_routing_enabled: false,
            balance_text: None,
            balance_display_enabled: false,
            api_quota_mode: ApiQuotaMode::ApiOnly,
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

    fn relay_account() -> StoredAccount {
        relay_account_with_id("relay-route", "Relay Route", "menu-main")
    }

    #[test]
    fn routes_use_enabled_menu_model_and_request_model_mapping() {
        let routes = build_routes_from_accounts(&[relay_account()], &[]);

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].model, "menu-main");
        assert_eq!(routes[0].display_name.as_deref(), Some("Menu menu-main"));
        assert_eq!(routes[0].request_model, "upstream-menu-main");
        assert_eq!(routes[0].context_window, Some(262144));
        assert_eq!(routes[0].base_url, "https://api.example.com/v1");
        assert_eq!(routes[0].api_key, "sk-route");

        let catalog = routes_to_model_catalog_entries(&routes);
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].model, "menu-main");
        assert_eq!(
            catalog[0].request_model.as_deref(),
            Some("upstream-menu-main")
        );
        assert_eq!(catalog[0].context_window, Some(262144));
    }

    #[test]
    fn route_selections_filter_router_models() {
        let first = relay_account_with_id("relay-first", "Relay First", "menu-first");
        let second = relay_account_with_id("relay-second", "Relay Second", "menu-second");
        let routes = build_routes_from_accounts(
            &[first, second],
            &[ModelRouterRouteSelection {
                account_id: "relay-second".to_string(),
                model: "menu-second".to_string(),
            }],
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].model, "menu-second");
        assert_eq!(routes[0].request_model, "upstream-menu-second");
        assert_eq!(routes[0].source_label, "Relay Second");
    }

    #[test]
    fn model_list_payload_includes_openai_and_codex_catalog_shapes() {
        let routes = build_routes_from_accounts(&[relay_account()], &[]);
        let payload = model_list_payload(&routes);

        assert_eq!(payload["data"][0]["id"], "menu-main");
        assert_eq!(payload["models"][0]["slug"], "menu-main");
        assert_eq!(payload["models"][0]["display_name"], "Menu menu-main");
        assert_eq!(payload["models"][0]["context_window"], 262144);
    }

    #[tokio::test]
    async fn model_router_accepts_requests_above_axum_default_body_limit() {
        let handle = start_model_router(vec![ModelRouterRoute {
            model: "known-model".to_string(),
            display_name: None,
            request_model: "known-model".to_string(),
            context_window: None,
            base_url: "https://api.example.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            source_label: "test".to_string(),
        }])
        .await
        .expect("start model router");
        let oversized_input = "x".repeat(3 * 1024 * 1024);
        let response = reqwest::Client::new()
            .post(format!("{}/responses", handle.base_url))
            .json(&json!({
                "model": "unrouted-model",
                "input": oversized_input,
            }))
            .send()
            .await
            .expect("send oversized request");

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let payload: serde_json::Value = response.json().await.expect("parse router error");
        assert_eq!(
            payload["error"]["message"],
            "No CodexDeck route for model unrouted-model."
        );
    }
}
