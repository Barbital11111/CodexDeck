use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::to_bytes;
use axum::body::Body;
use axum::body::Bytes;
use axum::extract::Request;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderName;
use axum::http::HeaderValue;
use axum::http::Method;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::response::Response;
use axum::routing::any;
use axum::Router;
use futures_util::StreamExt;
use serde_json::json;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::models::AccountSourceKind;
use crate::models::ProxyEndpointCapability;
use crate::models::StoredAccount;
use crate::state::AppState;
use crate::utils::redact_sensitive_text;

const LOCAL_PROXY_HOST: &str = "127.0.0.1";
const LOCAL_PROXY_VERSION_PREFIX: &str = "/v1";
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;
const UNSUPPORTED_HOSTED_TOOL_TYPES: &[&str] = &["image_generation"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HybridRelayRoute {
    Passthrough,
    ResponsesToChatCompletions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HybridRelayEndpointRouting {
    supports_responses: bool,
    supports_responses_compact: bool,
    supports_chat_completions: bool,
}

impl HybridRelayEndpointRouting {
    fn from_capabilities(endpoints: &[ProxyEndpointCapability]) -> Self {
        if endpoints.is_empty() {
            return Self {
                supports_responses: true,
                supports_responses_compact: true,
                supports_chat_completions: false,
            };
        }

        Self {
            supports_responses: endpoints.contains(&ProxyEndpointCapability::Responses),
            supports_responses_compact: endpoints
                .contains(&ProxyEndpointCapability::ResponsesCompact),
            supports_chat_completions: endpoints
                .contains(&ProxyEndpointCapability::ChatCompletions),
        }
    }

    fn route_for_path(&self, path: &str) -> HybridRelayRoute {
        if !self.supports_chat_completions {
            return HybridRelayRoute::Passthrough;
        }

        match responses_endpoint_for_path(path) {
            Some(ResponsesEndpoint::Responses) if !self.supports_responses => {
                HybridRelayRoute::ResponsesToChatCompletions
            }
            Some(ResponsesEndpoint::ResponsesCompact) if !self.supports_responses_compact => {
                HybridRelayRoute::ResponsesToChatCompletions
            }
            _ => HybridRelayRoute::Passthrough,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponsesEndpoint {
    Responses,
    ResponsesCompact,
}

#[derive(Clone)]
struct HybridRelayProxyRuntime {
    client: reqwest::Client,
    target_base_url: String,
    api_key: String,
    endpoint_routing: HybridRelayEndpointRouting,
}

pub(crate) struct HybridRelayProxyHandle {
    local_base_url: String,
    target_base_url: String,
    api_key_fingerprint: String,
    endpoint_routing: HybridRelayEndpointRouting,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl HybridRelayProxyHandle {
    fn matches_target(
        &self,
        target_base_url: &str,
        api_key_fingerprint: &str,
        endpoint_routing: &HybridRelayEndpointRouting,
    ) -> bool {
        self.target_base_url == target_base_url
            && self.api_key_fingerprint == api_key_fingerprint
            && &self.endpoint_routing == endpoint_routing
            && !self.task.is_finished()
    }

    fn local_base_url(&self) -> String {
        self.local_base_url.clone()
    }

    fn shutdown(mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

impl Drop for HybridRelayProxyHandle {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

pub(crate) async fn ensure_hybrid_relay_proxy_for_account(
    state: &AppState,
    relay_account: &StoredAccount,
) -> Result<String, String> {
    if !matches!(relay_account.source_kind, AccountSourceKind::Relay) {
        return Err("混合模式需要选择一个 API 条目。".to_string());
    }

    let target_base_url = relay_account
        .api_base_url
        .as_deref()
        .map(normalize_base_url)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "API 条目资料不完整".to_string())?;
    let api_key = relay_account
        .primary_relay_api_key()
        .ok_or_else(|| "API 条目资料不完整".to_string())?;

    let endpoint_routing =
        HybridRelayEndpointRouting::from_capabilities(&relay_account.proxy_endpoints);
    ensure_hybrid_relay_proxy(state, &target_base_url, api_key, endpoint_routing).await
}

async fn ensure_hybrid_relay_proxy(
    state: &AppState,
    target_base_url: &str,
    api_key: &str,
    endpoint_routing: HybridRelayEndpointRouting,
) -> Result<String, String> {
    let target_base_url = normalize_base_url(target_base_url);
    if target_base_url.is_empty() {
        return Err("API 条目缺少 Base URL。".to_string());
    }
    if !(target_base_url.starts_with("https://") || target_base_url.starts_with("http://")) {
        return Err("Base URL 仅支持 http/https 地址。".to_string());
    }
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("API 条目缺少 API Key。".to_string());
    }

    let api_key_fingerprint = fingerprint_secret(api_key);
    let mut guard = state.hybrid_relay_proxy.lock().await;
    if let Some(handle) = guard.as_ref() {
        if handle.matches_target(&target_base_url, &api_key_fingerprint, &endpoint_routing) {
            return Ok(handle.local_base_url());
        }
    }

    if let Some(handle) = guard.take() {
        handle.shutdown();
    }

    let handle =
        start_hybrid_relay_proxy(target_base_url, api_key.to_string(), endpoint_routing).await?;
    let local_base_url = handle.local_base_url();
    *guard = Some(handle);
    Ok(local_base_url)
}

async fn start_hybrid_relay_proxy(
    target_base_url: String,
    api_key: String,
    endpoint_routing: HybridRelayEndpointRouting,
) -> Result<HybridRelayProxyHandle, String> {
    let client = reqwest::Client::builder()
        .build()
        .map_err(|error| format!("创建混合模式本地代理客户端失败: {error}"))?;
    let runtime = Arc::new(HybridRelayProxyRuntime {
        client,
        target_base_url: target_base_url.clone(),
        api_key: api_key.clone(),
        endpoint_routing: endpoint_routing.clone(),
    });
    let app = Router::new()
        .fallback(any(proxy_request))
        .with_state(runtime);
    let listener = TcpListener::bind((LOCAL_PROXY_HOST, 0))
        .await
        .map_err(|error| format!("启动混合模式本地代理失败: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("读取混合模式本地代理端口失败: {error}"))?
        .port();
    let local_base_url = format!("http://{LOCAL_PROXY_HOST}:{port}{LOCAL_PROXY_VERSION_PREFIX}");
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        });
        if let Err(error) = server.await {
            log::warn!("混合模式本地代理已停止: {error}");
        }
    });

    Ok(HybridRelayProxyHandle {
        local_base_url,
        target_base_url,
        api_key_fingerprint: fingerprint_secret(&api_key),
        endpoint_routing,
        shutdown_tx: Some(shutdown_tx),
        task,
    })
}

async fn proxy_request(
    State(runtime): State<Arc<HybridRelayProxyRuntime>>,
    request: Request<Body>,
) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let uri = parts.uri;
    let headers = parts.headers;
    let route = runtime.endpoint_routing.route_for_path(uri.path());
    let upstream_url = upstream_url_for_request(&runtime.target_base_url, &uri, route);

    let body_bytes = match to_bytes(body, MAX_REQUEST_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return text_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                &format!("混合模式本地代理读取请求失败: {error}"),
            );
        }
    };
    let (forward_body, request_conversion_error) =
        rewrite_request_body(&headers, body_bytes.as_ref(), route);
    if let Some(message) = request_conversion_error {
        return text_response(StatusCode::BAD_REQUEST, &message);
    }

    let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(method) => method,
        Err(_) => return text_response(StatusCode::METHOD_NOT_ALLOWED, "不支持的请求方法。"),
    };
    let mut upstream_request = runtime
        .client
        .request(reqwest_method, upstream_url)
        .bearer_auth(&runtime.api_key);
    for (name, value) in headers.iter() {
        if should_skip_request_header(name) {
            continue;
        }
        upstream_request = upstream_request.header(name, value);
    }
    if method_allows_body(&method) || !forward_body.is_empty() {
        upstream_request = upstream_request.body(forward_body);
    }

    let upstream_response = match upstream_request.send().await {
        Ok(response) => response,
        Err(error) => {
            let message = redact_sensitive_text(&format!("混合模式本地代理连接中转失败: {error}"));
            return text_response(StatusCode::BAD_GATEWAY, &message);
        }
    };

    convert_upstream_response(upstream_response, route).await
}

fn upstream_url_for_request(target_base_url: &str, uri: &Uri, route: HybridRelayRoute) -> String {
    let target_base_url = normalize_base_url(target_base_url);
    let request_path = match route {
        HybridRelayRoute::Passthrough => uri.path().to_string(),
        HybridRelayRoute::ResponsesToChatCompletions => CHAT_COMPLETIONS_PATH.to_string(),
    };
    let stripped_v1_path = request_path
        .strip_prefix(LOCAL_PROXY_VERSION_PREFIX)
        .filter(|path| path.is_empty() || path.starts_with('/'));
    let path_to_append = if target_base_url.ends_with(LOCAL_PROXY_VERSION_PREFIX) {
        stripped_v1_path.unwrap_or(request_path.as_str())
    } else {
        request_path.as_str()
    };
    let mut upstream_url = format!("{target_base_url}{path_to_append}");
    if let Some(query) = uri.query() {
        upstream_url.push('?');
        upstream_url.push_str(query);
    }
    upstream_url
}

fn rewrite_request_body(
    headers: &HeaderMap,
    body: &[u8],
    route: HybridRelayRoute,
) -> (Vec<u8>, Option<String>) {
    if body.is_empty() || !looks_like_json_request(headers, body) {
        return (body.to_vec(), None);
    }

    let Ok(mut payload) = serde_json::from_slice::<Value>(body) else {
        return (body.to_vec(), None);
    };

    strip_unsupported_hosted_tools(&mut payload);

    if route == HybridRelayRoute::ResponsesToChatCompletions {
        return match responses_to_chat_completions(payload) {
            Ok(payload) => (
                serde_json::to_vec(&payload).unwrap_or_else(|_| body.to_vec()),
                None,
            ),
            Err(message) => (
                body.to_vec(),
                Some(format!(
                    "混合模式本地代理转换 Responses 请求失败: {message}"
                )),
            ),
        };
    }

    (
        serde_json::to_vec(&payload).unwrap_or_else(|_| body.to_vec()),
        None,
    )
}

pub(crate) fn strip_unsupported_hosted_tools(payload: &mut Value) -> usize {
    let mut removed = 0usize;
    if let Some(tools) = payload.get_mut("tools").and_then(Value::as_array_mut) {
        let before = tools.len();
        tools.retain(|tool| {
            !tool
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(is_unsupported_hosted_tool_type)
        });
        removed += before.saturating_sub(tools.len());
        if tools.is_empty() {
            payload.as_object_mut().map(|object| object.remove("tools"));
        }
    }

    if payload
        .get("tool_choice")
        .and_then(|tool_choice| tool_choice.get("type"))
        .and_then(Value::as_str)
        .is_some_and(is_unsupported_hosted_tool_type)
    {
        payload
            .as_object_mut()
            .map(|object| object.remove("tool_choice"));
        removed += 1;
    }

    removed
}

fn is_unsupported_hosted_tool_type(tool_type: &str) -> bool {
    UNSUPPORTED_HOSTED_TOOL_TYPES
        .iter()
        .any(|unsupported| tool_type.eq_ignore_ascii_case(unsupported))
}

fn responses_endpoint_for_path(path: &str) -> Option<ResponsesEndpoint> {
    match path {
        "/responses" | "/v1/responses" => Some(ResponsesEndpoint::Responses),
        "/responses/compact" | "/v1/responses/compact" => Some(ResponsesEndpoint::ResponsesCompact),
        _ => None,
    }
}

fn responses_to_chat_completions(body: Value) -> Result<Value, String> {
    let mut result = json!({});

    if let Some(model) = body.get("model") {
        result["model"] = model.clone();
    }

    let mut messages = Vec::new();
    if let Some(instructions) = body.get("instructions") {
        let instructions = response_text_from_content(instructions);
        if !instructions.trim().is_empty() {
            messages.push(json!({
                "role": "system",
                "content": instructions
            }));
        }
    }

    if let Some(input) = body.get("input") {
        append_responses_input_as_chat_messages(input, &mut messages)?;
    }
    result["messages"] = json!(collapse_system_messages_to_head(messages));

    if let Some(max_tokens) = body
        .get("max_output_tokens")
        .or_else(|| body.get("max_completion_tokens"))
        .or_else(|| body.get("max_tokens"))
    {
        result["max_tokens"] = max_tokens.clone();
    }

    for key in [
        "temperature",
        "top_p",
        "stream",
        "frequency_penalty",
        "presence_penalty",
        "stop",
        "response_format",
        "seed",
        "user",
    ] {
        if let Some(value) = body.get(key) {
            result[key] = value.clone();
        }
    }

    if result
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        result["stream_options"] = json!({ "include_usage": true });
    }

    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let chat_tools = tools
            .iter()
            .filter_map(response_tool_to_chat_tool)
            .collect::<Vec<_>>();
        if !chat_tools.is_empty() {
            result["tools"] = json!(chat_tools);
        }
    }

    if let Some(tool_choice) = body.get("tool_choice") {
        result["tool_choice"] = response_tool_choice_to_chat(tool_choice);
    }

    Ok(result)
}

fn append_responses_input_as_chat_messages(
    input: &Value,
    messages: &mut Vec<Value>,
) -> Result<(), String> {
    let mut pending_tool_calls = Vec::new();
    match input {
        Value::String(text) => {
            messages.push(json!({ "role": "user", "content": text }));
        }
        Value::Array(items) => {
            for item in items {
                append_responses_item_as_chat_message(item, messages, &mut pending_tool_calls)?;
            }
        }
        Value::Object(_) => {
            append_responses_item_as_chat_message(input, messages, &mut pending_tool_calls)?;
        }
        _ => {}
    }
    flush_pending_tool_calls(messages, &mut pending_tool_calls);
    Ok(())
}

fn append_responses_item_as_chat_message(
    item: &Value,
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
) -> Result<(), String> {
    let item_type = item.get("type").and_then(Value::as_str);
    match item_type {
        Some("function_call") => {
            pending_tool_calls.push(responses_function_call_to_chat_tool_call(item));
        }
        Some("function_call_output") => {
            flush_pending_tool_calls(messages, pending_tool_calls);
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let output = item
                .get("output")
                .map(response_tool_output_text)
                .unwrap_or_default();
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": output
            }));
        }
        Some("message") | None => {
            flush_pending_tool_calls(messages, pending_tool_calls);
            if item.get("role").is_some() || item.get("content").is_some() {
                messages.push(response_message_item_to_chat_message(item));
            }
        }
        _ => {
            flush_pending_tool_calls(messages, pending_tool_calls);
            if item.get("role").is_some() || item.get("content").is_some() {
                messages.push(response_message_item_to_chat_message(item));
            }
        }
    }
    Ok(())
}

fn flush_pending_tool_calls(messages: &mut Vec<Value>, pending_tool_calls: &mut Vec<Value>) {
    if pending_tool_calls.is_empty() {
        return;
    }

    messages.push(json!({
        "role": "assistant",
        "content": null,
        "tool_calls": std::mem::take(pending_tool_calls)
    }));
}

fn responses_function_call_to_chat_tool_call(item: &Value) -> Value {
    let call_id = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("call_0");
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown_tool");
    let arguments = canonicalize_tool_arguments(item.get("arguments"));

    json!({
        "id": call_id,
        "type": "function",
        "function": {
            "name": name,
            "arguments": arguments
        }
    })
}

fn response_message_item_to_chat_message(item: &Value) -> Value {
    let role = item
        .get("role")
        .and_then(Value::as_str)
        .map(responses_role_to_chat_role)
        .unwrap_or("user");
    let content = item
        .get("content")
        .map(response_text_from_content)
        .unwrap_or_default();

    json!({
        "role": role,
        "content": content
    })
}

fn responses_role_to_chat_role(role: &str) -> &'static str {
    match role {
        "system" | "developer" => "system",
        "assistant" => "assistant",
        "tool" => "tool",
        "user" | "latest_reminder" => "user",
        _ => "user",
    }
}

fn canonicalize_tool_arguments(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "{}".to_string();
    };
    match value {
        Value::String(text) => canonicalize_json_string_if_parseable(text),
        Value::Object(_) | Value::Array(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
        }
        Value::Null => "{}".to_string(),
        _ => value.to_string(),
    }
}

fn canonicalize_tool_arguments_str(value: &str) -> String {
    if value.trim().is_empty() {
        "{}".to_string()
    } else {
        canonicalize_json_string_if_parseable(value)
    }
}

fn canonicalize_json_string_if_parseable(value: &str) -> String {
    match serde_json::from_str::<Value>(value) {
        Ok(parsed) => serde_json::to_string(&parsed).unwrap_or_else(|_| value.to_string()),
        Err(_) => value.to_string(),
    }
}

fn response_text_from_content(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .or_else(|| part.get("output_text"))
                    .or_else(|| part.get("input_text"))
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(_) => content
            .get("text")
            .or_else(|| content.get("output_text"))
            .or_else(|| content.get("input_text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => content.as_str().unwrap_or_default().to_string(),
    }
}

fn response_tool_output_text(output: &Value) -> String {
    match output {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(output).unwrap_or_default(),
    }
}

fn collapse_system_messages_to_head(messages: Vec<Value>) -> Vec<Value> {
    let mut system_chunks = Vec::new();
    let mut rest = Vec::with_capacity(messages.len());

    for message in messages {
        if message.get("role").and_then(Value::as_str) == Some("system") {
            if let Some(content) = message.get("content").and_then(Value::as_str) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    system_chunks.push(trimmed.to_string());
                }
                continue;
            }
        }
        rest.push(message);
    }

    let mut out = Vec::with_capacity(rest.len() + 1);
    if !system_chunks.is_empty() {
        out.push(json!({
            "role": "system",
            "content": system_chunks.join("\n\n")
        }));
    }
    out.extend(rest);
    out
}

fn response_tool_to_chat_tool(tool: &Value) -> Option<Value> {
    let tool_type = tool.get("type").and_then(Value::as_str).unwrap_or_default();
    if tool_type.eq_ignore_ascii_case("function") {
        let mut function = json!({});
        if let Some(name) = tool.get("name").and_then(Value::as_str) {
            function["name"] = json!(name);
        }
        if let Some(description) = tool.get("description").and_then(Value::as_str) {
            function["description"] = json!(description);
        }
        if let Some(parameters) = tool.get("parameters") {
            function["parameters"] = parameters.clone();
        } else if let Some(parameters) = tool.get("input_schema") {
            function["parameters"] = parameters.clone();
        }
        if function.get("name").is_none() {
            return None;
        }
        return Some(json!({
            "type": "function",
            "function": function
        }));
    }

    None
}

fn response_tool_choice_to_chat(tool_choice: &Value) -> Value {
    if let Some(choice) = tool_choice.as_str() {
        return json!(choice);
    }
    let Some(choice_type) = tool_choice.get("type").and_then(Value::as_str) else {
        return tool_choice.clone();
    };
    if choice_type.eq_ignore_ascii_case("function") {
        if let Some(name) = tool_choice.get("name").and_then(Value::as_str) {
            return json!({
                "type": "function",
                "function": { "name": name }
            });
        }
    }
    tool_choice.clone()
}

fn looks_like_json_request(headers: &HeaderMap, body: &[u8]) -> bool {
    let content_type_is_json = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let mime = value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            mime == "application/json" || mime.ends_with("+json")
        });
    content_type_is_json
        || body
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|byte| matches!(byte, b'{' | b'['))
}

fn should_skip_request_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "host"
            | "content-length"
            | "authorization"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn should_skip_response_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "content-length"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn method_allows_body(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

async fn convert_upstream_response(
    upstream_response: reqwest::Response,
    route: HybridRelayRoute,
) -> Response<Body> {
    let status = StatusCode::from_u16(upstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let is_event_stream = upstream_response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"));

    if route == HybridRelayRoute::ResponsesToChatCompletions && status.is_success() {
        if is_event_stream {
            return stream_chat_sse_as_responses(upstream_response);
        }
        return json_chat_completion_as_response(upstream_response).await;
    }

    stream_upstream_response(upstream_response, status)
}

fn stream_upstream_response(
    upstream_response: reqwest::Response,
    status: StatusCode,
) -> Response<Body> {
    let mut response = Response::builder().status(status);
    for (name, value) in upstream_response.headers().iter() {
        if should_skip_response_header(name) {
            continue;
        }
        let Ok(header_name) = HeaderName::from_bytes(name.as_str().as_bytes()) else {
            continue;
        };
        let Ok(header_value) = HeaderValue::from_bytes(value.as_bytes()) else {
            continue;
        };
        response = response.header(header_name, header_value);
    }

    let stream = upstream_response
        .bytes_stream()
        .map(|chunk| chunk.map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error)));
    response
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| {
            text_response(StatusCode::BAD_GATEWAY, "混合模式本地代理构造响应失败。")
        })
}

async fn json_chat_completion_as_response(upstream_response: reqwest::Response) -> Response<Body> {
    let status = StatusCode::from_u16(upstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let headers = upstream_response.headers().clone();
    let bytes = match upstream_response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            return text_response(
                StatusCode::BAD_GATEWAY,
                &format!("混合模式本地代理读取 Chat Completions 响应失败: {error}"),
            );
        }
    };

    if !status.is_success() {
        return bytes_response(status, &headers, bytes);
    }

    let Ok(chat_response) = serde_json::from_slice::<Value>(&bytes) else {
        return bytes_response(status, &headers, bytes);
    };
    let responses_payload = chat_completion_to_response(chat_response);
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&responses_payload).unwrap_or_else(|_| bytes.to_vec()),
        ))
        .unwrap_or_else(|_| {
            text_response(
                StatusCode::BAD_GATEWAY,
                "混合模式本地代理构造 JSON 响应失败。",
            )
        })
}

fn bytes_response(status: StatusCode, headers: &HeaderMap, bytes: Bytes) -> Response<Body> {
    let mut response = Response::builder().status(status);
    for (name, value) in headers.iter() {
        if should_skip_response_header(name) {
            continue;
        }
        response = response.header(name, value);
    }
    response.body(Body::from(bytes)).unwrap_or_else(|_| {
        text_response(StatusCode::BAD_GATEWAY, "混合模式本地代理构造响应失败。")
    })
}

fn stream_chat_sse_as_responses(upstream_response: reqwest::Response) -> Response<Body> {
    let stream = upstream_response
        .bytes_stream()
        .map(|chunk| chunk.map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error)));
    let converted = chat_sse_to_responses_sse_stream(stream);
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(converted))
        .unwrap_or_else(|_| {
            text_response(
                StatusCode::BAD_GATEWAY,
                "混合模式本地代理构造 SSE 响应失败。",
            )
        })
}

fn chat_completion_to_response(body: Value) -> Value {
    let choice = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let message = choice.get("message").cloned().unwrap_or_else(|| json!({}));
    let response_id = response_id_from_chat_id(body.get("id").and_then(Value::as_str));
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let created_at = body.get("created").and_then(Value::as_u64).unwrap_or(0);
    let finish_reason = choice.get("finish_reason").and_then(Value::as_str);
    let output = chat_message_to_response_output_items(&message, &response_id);

    let mut response = json!({
        "id": response_id,
        "object": "response",
        "created_at": created_at,
        "status": response_status_from_finish_reason(finish_reason),
        "model": model,
        "output": output,
        "usage": chat_usage_to_responses_usage(body.get("usage"))
    });

    if finish_reason == Some("length") {
        response["incomplete_details"] = json!({ "reason": "max_output_tokens" });
    }

    response
}

fn chat_message_to_response_output_items(message: &Value, response_id: &str) -> Vec<Value> {
    let mut output = Vec::new();
    let mut content = Vec::new();

    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            content.push(json!({
                "type": "output_text",
                "text": text,
                "annotations": []
            }));
        }
    } else if let Some(parts) = message.get("content").and_then(Value::as_array) {
        for part in parts {
            let Some(text) = part
                .get("text")
                .or_else(|| part.get("output_text"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if !text.is_empty() {
                content.push(json!({
                    "type": "output_text",
                    "text": text,
                    "annotations": []
                }));
            }
        }
    }

    if !content.is_empty() {
        output.push(json!({
            "id": format!("{response_id}_msg"),
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": content
        }));
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, tool_call) in tool_calls.iter().enumerate() {
            output.push(chat_tool_call_to_response_item(tool_call, index));
        }
    } else if let Some(function_call) = message.get("function_call") {
        output.push(chat_legacy_function_call_to_response_item(function_call));
    }

    output
}

fn chat_tool_call_to_response_item(tool_call: &Value, index: usize) -> Value {
    let call_id = tool_call
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("call_{index}"));
    let function = tool_call.get("function").unwrap_or(&Value::Null);
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let arguments = function
        .get("arguments")
        .map(|value| canonicalize_tool_arguments(Some(value)))
        .unwrap_or_else(|| "{}".to_string());

    response_function_call_item(
        format!("fc_{call_id}"),
        "completed",
        call_id,
        name.to_string(),
        arguments,
    )
}

fn chat_legacy_function_call_to_response_item(function_call: &Value) -> Value {
    let call_id = function_call
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("call_0");
    let name = function_call
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let arguments = canonicalize_tool_arguments(function_call.get("arguments"));

    response_function_call_item(
        format!("fc_{call_id}"),
        "completed",
        call_id.to_string(),
        name.to_string(),
        arguments,
    )
}

fn response_function_call_item(
    item_id: String,
    status: &str,
    call_id: String,
    name: String,
    arguments: String,
) -> Value {
    json!({
        "id": item_id,
        "type": "function_call",
        "status": status,
        "call_id": call_id,
        "name": name,
        "arguments": arguments
    })
}

fn chat_usage_to_responses_usage(usage: Option<&Value>) -> Value {
    let Some(usage) = usage.filter(|value| value.is_object() && !value.is_null()) else {
        return json!({
            "input_tokens": 0,
            "output_tokens": 0,
            "total_tokens": 0
        });
    };

    let input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(input_tokens + output_tokens);

    let mut result = json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": total_tokens
    });

    if let Some(cached_tokens) = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
    {
        result["input_tokens_details"] = json!({ "cached_tokens": cached_tokens });
    }

    if let Some(details) = usage.get("completion_tokens_details") {
        result["output_tokens_details"] = details.clone();
    }

    result
}

fn response_id_from_chat_id(id: Option<&str>) -> String {
    let id = id.unwrap_or("codexdeck");
    if id.starts_with("resp_") {
        id.to_string()
    } else {
        format!("resp_{id}")
    }
}

fn response_status_from_finish_reason(finish_reason: Option<&str>) -> &'static str {
    match finish_reason {
        Some("length") => "incomplete",
        _ => "completed",
    }
}

fn chat_sse_to_responses_sse_stream<E: std::error::Error + Send + 'static>(
    stream: impl futures_util::Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut state = ChatSseToResponsesState::default();
        let mut stream_failed = false;

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                    while let Some(block) = take_sse_block(&mut buffer) {
                        if block.trim().is_empty() {
                            continue;
                        }

                        let mut data_parts = Vec::new();
                        let mut is_error_event = false;
                        for line in block.lines() {
                            if let Some(event) = strip_sse_field(line, "event") {
                                is_error_event = event.trim() == "error";
                            }
                            if let Some(data) = strip_sse_field(line, "data") {
                                data_parts.push(data.to_string());
                            }
                        }

                        if data_parts.is_empty() {
                            continue;
                        }

                        let data = data_parts.join("\n");
                        if data.trim() == "[DONE]" {
                            for event in state.finalize() {
                                yield Ok(event);
                            }
                            continue;
                        }

                        let Ok(chunk) = serde_json::from_str::<Value>(&data) else {
                            continue;
                        };
                        if is_error_event || chunk.get("error").is_some() {
                            yield Ok(state.failed_event(error_message_from_sse_value(&chunk)));
                            stream_failed = true;
                            break;
                        }
                        for event in state.handle_chat_chunk(&chunk) {
                            yield Ok(event);
                        }
                    }

                    if stream_failed {
                        break;
                    }
                }
                Err(error) => {
                    yield Ok(state.failed_event(format!("Stream error: {error}")));
                    stream_failed = true;
                    break;
                }
            }
        }

        if !stream_failed {
            for event in state.finalize() {
                yield Ok(event);
            }
        }
    }
}

#[derive(Debug)]
struct ChatSseToResponsesState {
    response_started: bool,
    completed: bool,
    response_id: String,
    model: String,
    created_at: u64,
    next_output_index: u32,
    text: ChatSseTextState,
    tools: BTreeMap<usize, ChatSseToolCallState>,
    output_items: Vec<(u32, Value)>,
    latest_usage: Option<Value>,
    finish_reason: Option<String>,
}

#[derive(Debug, Default)]
struct ChatSseTextState {
    added: bool,
    done: bool,
    item_id: String,
    output_index: u32,
    text: String,
}

#[derive(Debug, Default)]
struct ChatSseToolCallState {
    added: bool,
    done: bool,
    call_id: String,
    name: String,
    arguments: String,
    item_id: String,
    output_index: u32,
}

impl Default for ChatSseToResponsesState {
    fn default() -> Self {
        Self {
            response_started: false,
            completed: false,
            response_id: "resp_codexdeck".to_string(),
            model: String::new(),
            created_at: 0,
            next_output_index: 0,
            text: ChatSseTextState::default(),
            tools: BTreeMap::new(),
            output_items: Vec::new(),
            latest_usage: None,
            finish_reason: None,
        }
    }
}

impl ChatSseToResponsesState {
    fn handle_chat_chunk(&mut self, chunk: &Value) -> Vec<Bytes> {
        let mut events = Vec::new();

        if let Some(id) = chunk.get("id").and_then(Value::as_str) {
            self.response_id = response_id_from_chat_id(Some(id));
        }
        if let Some(model) = chunk.get("model").and_then(Value::as_str) {
            self.model = model.to_string();
        }
        if let Some(created) = chunk.get("created").and_then(Value::as_u64) {
            self.created_at = created;
        }

        events.extend(self.ensure_response_started());

        if let Some(usage) = chunk.get("usage").filter(|value| !value.is_null()) {
            self.latest_usage = Some(chat_usage_to_responses_usage(Some(usage)));
        }

        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return events;
        };

        if let Some(delta) = choice.get("delta") {
            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    events.extend(self.push_text_delta(content));
                }
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for (fallback_index, tool_call) in tool_calls.iter().enumerate() {
                    let index = tool_call
                        .get("index")
                        .and_then(Value::as_u64)
                        .map(|value| value as usize)
                        .unwrap_or(fallback_index);
                    let function = tool_call.get("function").unwrap_or(&Value::Null);
                    events.extend(self.push_tool_call_delta(
                        index,
                        tool_call.get("id").and_then(Value::as_str),
                        function.get("name").and_then(Value::as_str),
                        function.get("arguments").and_then(Value::as_str),
                    ));
                }
            }
            if let Some(function_call) = delta.get("function_call") {
                events.extend(self.push_tool_call_delta(
                    0,
                    function_call.get("id").and_then(Value::as_str),
                    function_call.get("name").and_then(Value::as_str),
                    function_call.get("arguments").and_then(Value::as_str),
                ));
            }
        }

        if let Some(finish_reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(finish_reason.to_string());
        }

        events
    }

    fn ensure_response_started(&mut self) -> Vec<Bytes> {
        if self.response_started {
            return Vec::new();
        }

        self.response_started = true;
        vec![
            sse_event(
                "response.created",
                json!({
                    "type": "response.created",
                    "response": self.base_response("in_progress", Vec::new())
                }),
            ),
            sse_event(
                "response.in_progress",
                json!({
                    "type": "response.in_progress",
                    "response": self.base_response("in_progress", Vec::new())
                }),
            ),
        ]
    }

    fn allocate_output_index(&mut self) -> u32 {
        let output_index = self.next_output_index;
        self.next_output_index = self.next_output_index.saturating_add(1);
        output_index
    }

    fn push_text_delta(&mut self, delta: &str) -> Vec<Bytes> {
        let mut events = Vec::new();
        if !self.text.added {
            self.text.added = true;
            self.text.item_id = format!("{}_msg", self.response_id);
            let output_index = self.allocate_output_index();
            self.text.output_index = output_index;
            events.push(sse_event(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": self.text.output_index,
                    "item": {
                        "id": self.text.item_id,
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": []
                    }
                }),
            ));
            events.push(sse_event(
                "response.content_part.added",
                json!({
                    "type": "response.content_part.added",
                    "item_id": self.text.item_id,
                    "output_index": self.text.output_index,
                    "content_index": 0,
                    "part": {
                        "type": "output_text",
                        "text": "",
                        "annotations": []
                    }
                }),
            ));
        }

        self.text.text.push_str(delta);
        events.push(sse_event(
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "item_id": self.text.item_id,
                "output_index": self.text.output_index,
                "content_index": 0,
                "delta": delta
            }),
        ));
        events
    }

    fn push_tool_call_delta(
        &mut self,
        index: usize,
        call_id: Option<&str>,
        name: Option<&str>,
        arguments_delta: Option<&str>,
    ) -> Vec<Bytes> {
        let mut events = Vec::new();
        if !self.tools.contains_key(&index) {
            let default_call_id = call_id
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("call_{index}"));
            let output_index = self.allocate_output_index();
            self.tools.insert(
                index,
                ChatSseToolCallState {
                    call_id: default_call_id,
                    item_id: format!("{}_fc_{index}", self.response_id),
                    output_index,
                    ..Default::default()
                },
            );
        }

        if let Some(tool) = self.tools.get_mut(&index) {
            if let Some(call_id) = call_id.filter(|value| !value.is_empty()) {
                tool.call_id = call_id.to_string();
            }
            if let Some(name) = name.filter(|value| !value.is_empty()) {
                if tool.name.is_empty() || tool.name == name {
                    tool.name = name.to_string();
                } else {
                    tool.name.push_str(name);
                }
            }
            if !tool.added {
                tool.added = true;
                events.push(sse_event(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": tool.output_index,
                        "item": response_function_call_item(
                            tool.item_id.clone(),
                            "in_progress",
                            tool.call_id.clone(),
                            tool.name.clone(),
                            tool.arguments.clone(),
                        )
                    }),
                ));
            }
            if let Some(delta) = arguments_delta.filter(|value| !value.is_empty()) {
                tool.arguments.push_str(delta);
                events.push(sse_event(
                    "response.function_call_arguments.delta",
                    json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": tool.item_id,
                        "output_index": tool.output_index,
                        "delta": delta
                    }),
                ));
            }
        }

        events
    }

    fn finalize(&mut self) -> Vec<Bytes> {
        if self.completed {
            return Vec::new();
        }

        let mut events = self.ensure_response_started();
        if self.text.added && !self.text.done {
            self.text.done = true;
            let item = json!({
                "id": self.text.item_id,
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": self.text.text,
                    "annotations": []
                }]
            });
            self.output_items
                .push((self.text.output_index, item.clone()));
            events.push(sse_event(
                "response.output_text.done",
                json!({
                    "type": "response.output_text.done",
                    "item_id": self.text.item_id,
                    "output_index": self.text.output_index,
                    "content_index": 0,
                    "text": self.text.text
                }),
            ));
            events.push(sse_event(
                "response.content_part.done",
                json!({
                    "type": "response.content_part.done",
                    "item_id": self.text.item_id,
                    "output_index": self.text.output_index,
                    "content_index": 0,
                    "part": {
                        "type": "output_text",
                        "text": self.text.text,
                        "annotations": []
                    }
                }),
            ));
            events.push(sse_event(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": self.text.output_index,
                    "item": item
                }),
            ));
        }
        for tool in self.tools.values_mut() {
            if !tool.added || tool.done {
                continue;
            }
            tool.done = true;
            let arguments = canonicalize_tool_arguments_str(&tool.arguments);
            let item = response_function_call_item(
                tool.item_id.clone(),
                "completed",
                tool.call_id.clone(),
                tool.name.clone(),
                arguments.clone(),
            );
            self.output_items.push((tool.output_index, item.clone()));
            events.push(sse_event(
                "response.function_call_arguments.done",
                json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": tool.item_id,
                    "output_index": tool.output_index,
                    "arguments": arguments
                }),
            ));
            events.push(sse_event(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": tool.output_index,
                    "item": item
                }),
            ));
        }
        self.output_items
            .sort_by_key(|(output_index, _item)| *output_index);
        let output = self
            .output_items
            .iter()
            .map(|(_output_index, item)| item.clone())
            .collect::<Vec<_>>();

        let status = response_status_from_finish_reason(self.finish_reason.as_deref());
        let mut response = self.base_response(status, output);
        if status == "incomplete" {
            response["incomplete_details"] = json!({ "reason": "max_output_tokens" });
        }
        events.push(sse_event(
            "response.completed",
            json!({
                "type": "response.completed",
                "response": response
            }),
        ));
        self.completed = true;
        events
    }

    fn failed_event(&mut self, message: String) -> Bytes {
        self.completed = true;
        let mut response = self.base_response("failed", Vec::new());
        response["error"] = json!({ "message": message });
        sse_event(
            "response.failed",
            json!({
                "type": "response.failed",
                "response": response
            }),
        )
    }

    fn base_response(&self, status: &str, output: Vec<Value>) -> Value {
        json!({
            "id": self.response_id,
            "object": "response",
            "created_at": self.created_at,
            "status": status,
            "model": self.model,
            "output": output,
            "usage": self.latest_usage.clone().unwrap_or_else(|| {
                json!({
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "total_tokens": 0
                })
            })
        })
    }
}

fn append_utf8_safe(buffer: &mut String, remainder: &mut Vec<u8>, bytes: &[u8]) {
    let mut combined = Vec::with_capacity(remainder.len() + bytes.len());
    combined.extend_from_slice(remainder);
    combined.extend_from_slice(bytes);
    match std::str::from_utf8(&combined) {
        Ok(text) => {
            buffer.push_str(text);
            remainder.clear();
        }
        Err(error) => {
            let valid_up_to = error.valid_up_to();
            if valid_up_to > 0 {
                if let Ok(text) = std::str::from_utf8(&combined[..valid_up_to]) {
                    buffer.push_str(text);
                }
            }
            remainder.clear();
            remainder.extend_from_slice(&combined[valid_up_to..]);
        }
    }
}

fn take_sse_block(buffer: &mut String) -> Option<String> {
    if let Some(index) = buffer.find("\n\n") {
        let block = buffer[..index].trim_end_matches('\r').to_string();
        buffer.drain(..index + 2);
        return Some(block);
    }
    if let Some(index) = buffer.find("\r\n\r\n") {
        let block = buffer[..index].to_string();
        buffer.drain(..index + 4);
        return Some(block);
    }
    None
}

fn strip_sse_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field)?;
    let rest = rest.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn sse_event(event: &str, data: Value) -> Bytes {
    Bytes::from(format!(
        "event: {event}\ndata: {}\n\n",
        serde_json::to_string(&data).unwrap_or_default()
    ))
}

fn error_message_from_sse_value(value: &Value) -> String {
    let error = value.get("error").unwrap_or(value);
    error
        .as_str()
        .map(ToString::to_string)
        .or_else(|| {
            error
                .get("message")
                .or_else(|| error.get("detail"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| error.to_string())
}

fn text_response(status: StatusCode, message: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Body::from(message.to_string()))
        .unwrap_or_else(|_| Response::new(Body::from("混合模式本地代理异常。")))
}

fn normalize_base_url(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_string()
}

fn fingerprint_secret(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::chat_completion_to_response;
    use super::chat_sse_to_responses_sse_stream;
    use super::responses_to_chat_completions;
    use super::strip_unsupported_hosted_tools;
    use super::upstream_url_for_request;
    use super::HybridRelayEndpointRouting;
    use super::HybridRelayRoute;
    use crate::models::ProxyEndpointCapability;
    use axum::http::Uri;
    use futures_util::stream;
    use futures_util::StreamExt;
    use serde_json::json;

    #[test]
    fn strips_image_generation_tool_from_responses_payload() {
        let mut payload = json!({
            "model": "gpt-5.5",
            "tools": [
                { "type": "function", "name": "shell" },
                { "type": "image_generation" }
            ],
            "tool_choice": "auto"
        });

        let removed = strip_unsupported_hosted_tools(&mut payload);

        assert_eq!(removed, 1);
        assert_eq!(
            payload["tools"],
            json!([{ "type": "function", "name": "shell" }])
        );
        assert_eq!(payload["tool_choice"], "auto");
    }

    #[test]
    fn removes_empty_tools_and_unsupported_tool_choice() {
        let mut payload = json!({
            "model": "gpt-5.5",
            "tools": [
                { "type": "image_generation" }
            ],
            "tool_choice": { "type": "image_generation" }
        });

        let removed = strip_unsupported_hosted_tools(&mut payload);

        assert_eq!(removed, 2);
        assert!(payload.get("tools").is_none());
        assert!(payload.get("tool_choice").is_none());
    }

    #[test]
    fn maps_local_v1_path_to_upstream_v1_base() {
        let uri: Uri = "/v1/responses?stream=true".parse().expect("parse uri");

        let upstream = upstream_url_for_request(
            "https://relay.example.com/v1",
            &uri,
            HybridRelayRoute::Passthrough,
        );

        assert_eq!(
            upstream,
            "https://relay.example.com/v1/responses?stream=true"
        );
    }

    #[test]
    fn maps_local_v1_path_to_upstream_root_base() {
        let uri: Uri = "/v1/responses/compact".parse().expect("parse uri");

        let upstream = upstream_url_for_request(
            "https://relay.example.com",
            &uri,
            HybridRelayRoute::Passthrough,
        );

        assert_eq!(upstream, "https://relay.example.com/v1/responses/compact");
    }

    #[test]
    fn chat_only_account_routes_responses_to_chat_completions() {
        let routing = HybridRelayEndpointRouting::from_capabilities(&[
            ProxyEndpointCapability::ChatCompletions,
        ]);
        assert_eq!(
            routing.route_for_path("/v1/responses"),
            HybridRelayRoute::ResponsesToChatCompletions
        );

        let uri: Uri = "/v1/responses?stream=true".parse().expect("parse uri");
        let upstream = upstream_url_for_request(
            "https://relay.example.com/v1",
            &uri,
            routing.route_for_path(uri.path()),
        );

        assert_eq!(
            upstream,
            "https://relay.example.com/v1/chat/completions?stream=true"
        );
    }

    #[test]
    fn chat_only_account_routes_root_base_to_chat_completions() {
        let routing = HybridRelayEndpointRouting::from_capabilities(&[
            ProxyEndpointCapability::ChatCompletions,
        ]);
        let uri: Uri = "/v1/responses?stream=true".parse().expect("parse uri");

        let upstream = upstream_url_for_request(
            "https://relay.example.com",
            &uri,
            routing.route_for_path(uri.path()),
        );

        assert_eq!(
            upstream,
            "https://relay.example.com/v1/chat/completions?stream=true"
        );
    }

    #[test]
    fn responses_capable_account_keeps_responses_route() {
        let routing = HybridRelayEndpointRouting::from_capabilities(&[
            ProxyEndpointCapability::Responses,
            ProxyEndpointCapability::ChatCompletions,
        ]);

        assert_eq!(
            routing.route_for_path("/v1/responses"),
            HybridRelayRoute::Passthrough
        );
    }

    #[test]
    fn converts_responses_request_to_chat_completions() {
        let payload = json!({
            "model": "gpt-5.5",
            "instructions": "Follow policy.",
            "input": [
                {
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "ping" }]
                }
            ],
            "tools": [
                {
                    "type": "function",
                    "name": "lookup",
                    "description": "Lookup",
                    "parameters": { "type": "object" }
                },
                { "type": "image_generation" }
            ],
            "max_output_tokens": 9,
            "stream": true
        });
        let mut sanitized = payload;
        strip_unsupported_hosted_tools(&mut sanitized);

        let converted = responses_to_chat_completions(sanitized).expect("convert request");

        assert_eq!(converted["model"], "gpt-5.5");
        assert_eq!(converted["messages"][0]["role"], "system");
        assert_eq!(converted["messages"][1]["role"], "user");
        assert_eq!(converted["messages"][1]["content"], "ping");
        assert_eq!(converted["tools"][0]["function"]["name"], "lookup");
        assert_eq!(converted["max_tokens"], 9);
        assert_eq!(converted["stream_options"]["include_usage"], true);
        assert_eq!(converted["tools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn converts_responses_tool_calls_to_chat_messages() {
        let payload = json!({
            "model": "gpt-5.5",
            "input": [
                {
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "look up" }]
                },
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "lookup",
                    "arguments": { "query": "ping" }
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": { "result": "pong" }
                }
            ]
        });

        let converted = responses_to_chat_completions(payload).expect("convert request");

        assert_eq!(converted["messages"][0]["role"], "user");
        assert_eq!(converted["messages"][1]["role"], "assistant");
        assert_eq!(converted["messages"][1]["content"], json!(null));
        assert_eq!(converted["messages"][1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            converted["messages"][1]["tool_calls"][0]["function"]["name"],
            "lookup"
        );
        assert_eq!(
            converted["messages"][1]["tool_calls"][0]["function"]["arguments"],
            r#"{"query":"ping"}"#
        );
        assert_eq!(converted["messages"][2]["role"], "tool");
        assert_eq!(converted["messages"][2]["tool_call_id"], "call_1");
        assert_eq!(converted["messages"][2]["content"], r#"{"result":"pong"}"#);
    }

    #[test]
    fn converts_chat_completion_response_to_responses() {
        let chat = json!({
            "id": "chatcmpl_1",
            "created": 123,
            "model": "gpt-5.5",
            "choices": [{
                "message": { "role": "assistant", "content": "pong" },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 4,
                "completion_tokens": 2,
                "total_tokens": 6
            }
        });

        let response = chat_completion_to_response(chat);

        assert_eq!(response["id"], "resp_chatcmpl_1");
        assert_eq!(response["status"], "completed");
        assert_eq!(response["output"][0]["type"], "message");
        assert_eq!(response["output"][0]["content"][0]["text"], "pong");
        assert_eq!(response["usage"]["input_tokens"], 4);
        assert_eq!(response["usage"]["output_tokens"], 2);
    }

    #[test]
    fn converts_chat_completion_tool_calls_to_responses() {
        let chat = json!({
            "id": "chatcmpl_1",
            "created": 123,
            "model": "gpt-5.5",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "lookup",
                            "arguments": "{\"query\":\"ping\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });

        let response = chat_completion_to_response(chat);

        assert_eq!(response["output"][0]["type"], "function_call");
        assert_eq!(response["output"][0]["call_id"], "call_1");
        assert_eq!(response["output"][0]["name"], "lookup");
        assert_eq!(response["output"][0]["arguments"], r#"{"query":"ping"}"#);
    }

    #[tokio::test]
    async fn converts_chat_sse_to_responses_sse() {
        let chunks = vec![
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                b"data: {\"id\":\"chatcmpl_1\",\"created\":123,\"model\":\"gpt-5.5\",\"choices\":[{\"delta\":{\"content\":\"po\"}}]}\n\n",
            )),
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                b"data: {\"id\":\"chatcmpl_1\",\"created\":123,\"model\":\"gpt-5.5\",\"choices\":[{\"delta\":{\"content\":\"ng\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2,\"total_tokens\":6}}\n\n",
            )),
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"data: [DONE]\n\n")),
        ];
        let output = chat_sse_to_responses_sse_stream(stream::iter(chunks))
            .map(|chunk| chunk.expect("sse chunk"))
            .collect::<Vec<_>>()
            .await;
        let output = String::from_utf8(output.concat()).expect("utf8");

        assert!(output.contains("event: response.created"));
        assert!(output.contains("event: response.output_text.delta"));
        assert!(output.contains("\"text\":\"pong\""));
        assert!(output.contains("event: response.completed"));
        assert!(output.contains("\"input_tokens\":4"));
    }

    #[tokio::test]
    async fn converts_chat_sse_tool_calls_to_responses_sse() {
        let chunks = vec![
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                b"data: {\"id\":\"chatcmpl_1\",\"created\":123,\"model\":\"gpt-5.5\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"query\\\":\"}}]}}]}\n\n",
            )),
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                b"data: {\"id\":\"chatcmpl_1\",\"created\":123,\"model\":\"gpt-5.5\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"ping\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2,\"total_tokens\":6}}\n\n",
            )),
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"data: [DONE]\n\n")),
        ];

        let output = chat_sse_to_responses_sse_stream(stream::iter(chunks))
            .map(|chunk| chunk.expect("sse chunk"))
            .collect::<Vec<_>>()
            .await;
        let output = String::from_utf8(output.concat()).expect("utf8");

        assert!(output.contains("event: response.output_item.added"));
        assert!(output.contains("\"type\":\"function_call\""));
        assert!(output.contains("event: response.function_call_arguments.delta"));
        assert!(output.contains("event: response.function_call_arguments.done"));
        assert!(output.contains("\"arguments\":\"{\\\"query\\\":\\\"ping\\\"}\""));
        assert!(output.contains("event: response.output_item.done"));
        assert!(output.contains("\"call_id\":\"call_1\""));
        assert!(output.contains("\"name\":\"lookup\""));
        assert!(output.contains("\"input_tokens\":4"));
    }
}
