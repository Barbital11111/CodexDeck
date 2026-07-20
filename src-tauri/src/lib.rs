mod account_service;
mod app_paths;
mod auth;
mod cli;
mod codex_model_picker_patch;
mod codex_multimodel;
mod editor_apps;
mod hybrid_relay_proxy;
mod i18n;
mod model_router;
mod models;
mod notification_service;
mod opencode;
mod profile_files;
mod session_provider_sync;
mod settings_service;
mod state;
mod store;
mod token_usage;
mod tray;
mod usage;
mod utils;

use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
#[cfg(any(target_os = "macos", all(unix, not(target_os = "macos"))))]
use std::process::Command;
use std::thread;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use rfd::FileDialog;
use tauri::webview::PageLoadEvent;
use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;
use tauri::State;
use tauri::WindowEvent;

use models::AccountSummary;
use models::AppSettings;
use models::AppSettingsPatch;
use models::AuthJsonImportInput;
use models::CreateApiAccountInput;
use models::EditorAppId;
use models::ImportAccountsResult;
use models::InstalledEditorApp;
use models::NotificationProviderConfig;
use models::NotificationTargetConfig;
use models::OauthCallbackFinishedEvent;
use models::PreparedOauthLogin;
use models::RelayModelCatalogEntry;
use models::SwitchAccountResult;
use models::UpdateApiAccountInput;
use models::UpdateApiAccountKeyInput;
use state::AppState;
use state::OauthCallbackListenerHandle;
#[cfg(target_os = "windows")]
use utils::new_background_command;

const OAUTH_CALLBACK_FINISHED_EVENT: &str = "oauth-callback-finished";
const AUTH_KEEPALIVE_INTERVAL_SECS: u64 = 300;
const AUTH_KEEPALIVE_INITIAL_DELAY_SECS: u64 = 60;

async fn resolve_relay_provider_base_url(
    _state: &AppState,
    relay_account: &models::StoredAccount,
) -> Result<String, String> {
    relay_account
        .api_base_url
        .clone()
        .ok_or_else(|| "API 条目资料不完整".to_string())
}
struct CodexLaunchOutcome {
    launched_app_path: Option<String>,
    used_fallback_cli: bool,
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn write_oauth_html_response(
    stream: &mut std::net::TcpStream,
    status_line: &str,
    title: &str,
    detail: &str,
) {
    let body = format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>body{{margin:0;padding:32px;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;background:#f4f7fb;color:#152033}}main{{max-width:560px;margin:0 auto;padding:24px;border-radius:20px;background:#fff;box-shadow:0 14px 34px rgba(21,32,51,.08)}}h1{{margin:0 0 10px;font-size:24px;line-height:1.2}}p{{margin:0;color:#52627b;line-height:1.6;word-break:break-word}}</style></head><body><main><h1>{}</h1><p>{}</p></main></body></html>",
        escape_html(title),
        escape_html(title),
        escape_html(detail)
    );
    let response = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn read_oauth_request_path(stream: &mut std::net::TcpStream) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .map_err(|error| format!("设置 OAuth 回调读取超时失败: {error}"))?;
    let mut buffer = [0_u8; 8192];
    let bytes_read = stream
        .read(&mut buffer)
        .map_err(|error| format!("读取 OAuth 回调请求失败: {error}"))?;
    if bytes_read == 0 {
        return Err("OAuth 回调连接已关闭".to_string());
    }

    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| "OAuth 回调请求为空".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    if method != "GET" {
        return Err(format!("不支持的 OAuth 回调请求方法: {method}"));
    }

    parts
        .next()
        .map(ToString::to_string)
        .ok_or_else(|| "OAuth 回调请求缺少路径".to_string())
}

fn build_oauth_callback_url(redirect_uri: &str, path: &str) -> Result<String, String> {
    let mut callback_url = reqwest::Url::parse(redirect_uri)
        .map_err(|error| format!("OAuth redirect_uri 无效: {error}"))?;
    let request_url = reqwest::Url::parse(&format!("http://localhost{path}"))
        .map_err(|error| format!("OAuth 回调路径无效: {error}"))?;
    callback_url.set_path(request_url.path());
    callback_url.set_query(request_url.query());
    callback_url.set_fragment(request_url.fragment());
    Ok(callback_url.to_string())
}

fn bind_oauth_callback_listener(preferred_port: u16) -> Result<(TcpListener, u16), String> {
    match TcpListener::bind(("127.0.0.1", preferred_port)) {
        Ok(listener) => Ok((listener, preferred_port)),
        Err(error) => {
            let fallback = TcpListener::bind(("127.0.0.1", 0)).map_err(|fallback_error| {
                format!(
                    "无法启动 OAuth 回调监听 127.0.0.1:{preferred_port}: {error}；自动回退到本地空闲端口也失败: {fallback_error}"
                )
            })?;
            let port = fallback
                .local_addr()
                .map_err(|addr_error| format!("无法读取 OAuth 回调监听端口: {addr_error}"))?
                .port();
            log::warn!(
                "OAuth 回调默认端口 {} 绑定失败: {}；已自动回退到本地空闲端口 {}",
                preferred_port,
                error,
                port
            );
            Ok((fallback, port))
        }
    }
}

async fn stop_oauth_callback_listener(state: &AppState) {
    let handle = {
        let mut guard = state.oauth_listener.lock().await;
        guard.take()
    };

    let Some(mut handle) = handle else {
        return;
    };

    if let Some(shutdown_tx) = handle.shutdown_tx.take() {
        let _ = shutdown_tx.send(());
    }

    if let Some(task) = handle.task.take() {
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let _ = task.join();
        })
        .await;
    }
}

async fn clear_pending_oauth_if_matches(state: &AppState, expected_state: &str) {
    let mut guard = state.pending_oauth_login.lock().await;
    if guard
        .as_ref()
        .is_some_and(|pending| pending.state.as_str() == expected_state)
    {
        *guard = None;
    }
}

async fn import_oauth_auth_json(
    app: &AppHandle,
    state: &AppState,
    auth_json: serde_json::Value,
    source: &str,
) -> Result<ImportAccountsResult, String> {
    let serialized = serde_json::to_string(&auth_json)
        .map_err(|error| format!("序列化 OAuth 登录结果失败: {error}"))?;
    let result = account_service::import_auth_json_accounts_internal(
        app,
        state,
        vec![AuthJsonImportInput {
            source: source.to_string(),
            content: serialized,
            label: None,
        }],
    )
    .await?;

    if result.imported_count > 0 || result.updated_count > 0 {
        let _ = tray::refresh_macos_tray_snapshot(app);
    }

    Ok(result)
}

async fn complete_oauth_login_internal(
    app: &AppHandle,
    state: &AppState,
    callback_url: &str,
) -> Result<ImportAccountsResult, String> {
    let pending = {
        let guard = state.pending_oauth_login.lock().await;
        guard
            .clone()
            .ok_or_else(|| "请先打开授权页面".to_string())?
    };

    let auth_json = auth::complete_oauth_callback_login(&pending, callback_url).await?;
    if let Some(account_id) = pending.reauthorize_account_id.as_deref() {
        account_service::reauthorize_account_internal(app, state, account_id, auth_json).await
    } else {
        import_oauth_auth_json(app, state, auth_json, "oauth-callback").await
    }
}

async fn emit_oauth_callback_finished(app: &AppHandle, payload: OauthCallbackFinishedEvent) {
    let _ = app.emit(OAUTH_CALLBACK_FINISHED_EVENT, payload);
}

fn run_oauth_callback_listener(
    app: AppHandle,
    listener: TcpListener,
    pending: auth::PendingOauthLogin,
    shutdown_rx: std::sync::mpsc::Receiver<()>,
) {
    loop {
        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs() as i64,
            Err(_) => 0,
        };
        if now >= pending.expires_at {
            tauri::async_runtime::block_on(async {
                let state = app.state::<AppState>();
                clear_pending_oauth_if_matches(state.inner(), &pending.state).await;
                emit_oauth_callback_finished(
                    &app,
                    OauthCallbackFinishedEvent {
                        result: None,
                        error: Some("OAuth 授权已超时，请重新打开授权页面。".to_string()),
                    },
                )
                .await;
            });
            break;
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                let path = match read_oauth_request_path(&mut stream) {
                    Ok(value) => value,
                    Err(error) => {
                        write_oauth_html_response(
                            &mut stream,
                            "400 Bad Request",
                            "授权失败",
                            &error,
                        );
                        break;
                    }
                };

                if path == "/cancel" {
                    write_oauth_html_response(
                        &mut stream,
                        "200 OK",
                        "授权已取消",
                        "当前授权监听已取消，可以关闭这个页面。",
                    );
                    break;
                }

                if !path.starts_with("/auth/callback") {
                    write_oauth_html_response(
                        &mut stream,
                        "404 Not Found",
                        "未识别的回调地址",
                        "当前地址不是 CodexDeck 的 OAuth 回调地址，可以关闭这个页面。",
                    );
                    continue;
                }

                let callback_url = match build_oauth_callback_url(&pending.redirect_uri, &path) {
                    Ok(value) => value,
                    Err(error) => {
                        write_oauth_html_response(
                            &mut stream,
                            "400 Bad Request",
                            "授权失败",
                            &error,
                        );
                        break;
                    }
                };
                let callback_result = tauri::async_runtime::block_on(async {
                    let state = app.state::<AppState>();
                    let pending_matches = {
                        let guard = state.pending_oauth_login.lock().await;
                        guard
                            .as_ref()
                            .is_some_and(|current| current.state.as_str() == pending.state.as_str())
                    };
                    if !pending_matches {
                        return Err("当前授权会话已失效，请回到应用重新打开授权页面。".to_string());
                    }

                    let result =
                        complete_oauth_login_internal(&app, state.inner(), &callback_url).await;
                    clear_pending_oauth_if_matches(state.inner(), &pending.state).await;
                    result
                });

                match callback_result {
                    Ok(result) => {
                        write_oauth_html_response(
                            &mut stream,
                            "200 OK",
                            "授权完成",
                            "账号已经写入 CodexDeck，可以回到应用继续操作。",
                        );
                        restore_main_window(&app);
                        tauri::async_runtime::block_on(async {
                            emit_oauth_callback_finished(
                                &app,
                                OauthCallbackFinishedEvent {
                                    result: Some(result),
                                    error: None,
                                },
                            )
                            .await;
                        });
                    }
                    Err(error) => {
                        write_oauth_html_response(
                            &mut stream,
                            "400 Bad Request",
                            "授权失败",
                            &error,
                        );
                        restore_main_window(&app);
                        if !error.contains("会话已失效") {
                            tauri::async_runtime::block_on(async {
                                emit_oauth_callback_finished(
                                    &app,
                                    OauthCallbackFinishedEvent {
                                        result: None,
                                        error: Some(error),
                                    },
                                )
                                .await;
                            });
                        }
                    }
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(120));
            }
            Err(error) => {
                tauri::async_runtime::block_on(async {
                    emit_oauth_callback_finished(
                        &app,
                        OauthCallbackFinishedEvent {
                            result: None,
                            error: Some(format!("OAuth 回调监听失败: {error}")),
                        },
                    )
                    .await;
                });
                break;
            }
        }
    }

    tauri::async_runtime::block_on(async {
        let state = app.state::<AppState>();
        let mut guard = state.oauth_listener.lock().await;
        *guard = None;
    });
}

async fn start_oauth_callback_listener(
    app: &AppHandle,
    state: &AppState,
    listener: TcpListener,
    pending: &auth::PendingOauthLogin,
) -> Result<(), String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("无法设置 OAuth 回调监听模式: {error}"))?;

    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    let app_handle = app.clone();
    let pending_login = pending.clone();
    let task = thread::spawn(move || {
        run_oauth_callback_listener(app_handle, listener, pending_login, shutdown_rx);
    });

    let mut guard = state.oauth_listener.lock().await;
    *guard = Some(OauthCallbackListenerHandle {
        shutdown_tx: Some(shutdown_tx),
        task: Some(task),
    });
    Ok(())
}

// ===== Tauri Commands (thin wrappers) =====
// 命令函数仅负责参数编排与跨模块调用，
// 核心业务逻辑放在 account_service/auth/store/tray 等模块。

#[tauri::command]
async fn list_accounts(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<AccountSummary>, String> {
    account_service::list_accounts_internal(&app, state.inner()).await
}

#[tauri::command]
async fn run_startup_maintenance(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<AccountSummary>, String> {
    run_startup_maintenance_internal(&app, state.inner()).await
}

#[tauri::command]
async fn import_current_auth_account(
    app: AppHandle,
    state: State<'_, AppState>,
    label: Option<String>,
) -> Result<AccountSummary, String> {
    let summary =
        account_service::import_current_auth_account_internal(&app, state.inner(), label).await?;
    let _ = tray::refresh_macos_tray_snapshot(&app);
    Ok(summary)
}

#[tauri::command]
async fn create_api_account(
    app: AppHandle,
    state: State<'_, AppState>,
    input: CreateApiAccountInput,
) -> Result<AccountSummary, String> {
    let summary = account_service::create_api_account_internal(&app, state.inner(), input).await?;
    let _ = tray::refresh_macos_tray_snapshot(&app);
    Ok(summary)
}

#[tauri::command]
async fn probe_api_models(
    base_url: String,
    api_key: String,
) -> Result<Vec<RelayModelCatalogEntry>, String> {
    account_service::probe_api_models_internal(&base_url, &api_key).await
}

#[tauri::command]
async fn probe_api_account_models(
    app: AppHandle,
    state: State<'_, AppState>,
    account_key: String,
    base_url: Option<String>,
    api_key: Option<String>,
) -> Result<Vec<RelayModelCatalogEntry>, String> {
    account_service::probe_api_account_models_internal(
        &app,
        state.inner(),
        &account_key,
        base_url,
        api_key,
    )
    .await
}

#[tauri::command]
async fn import_auth_json_accounts(
    app: AppHandle,
    state: State<'_, AppState>,
    items: Vec<AuthJsonImportInput>,
) -> Result<ImportAccountsResult, String> {
    let result =
        account_service::import_auth_json_accounts_internal(&app, state.inner(), items).await?;
    if result.imported_count > 0 || result.updated_count > 0 {
        let _ = tray::refresh_macos_tray_snapshot(&app);
    }
    Ok(result)
}

#[tauri::command]
async fn export_accounts_zip(
    app: AppHandle,
    state: State<'_, AppState>,
    account_key: Option<String>,
    account_keys: Option<Vec<String>>,
    format: Option<account_service::AccountsExportFormat>,
) -> Result<Option<String>, String> {
    account_service::export_accounts_zip_internal(
        &app,
        state.inner(),
        account_key,
        account_keys,
        format,
    )
    .await
}

#[tauri::command]
async fn delete_account(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    account_service::delete_account_internal(&app, state.inner(), &id).await?;
    let _ = tray::refresh_macos_tray_snapshot(&app);
    Ok(())
}

#[tauri::command]
async fn update_account_label(
    app: AppHandle,
    state: State<'_, AppState>,
    account_key: String,
    label: String,
) -> Result<String, String> {
    let resolved_label =
        account_service::update_account_label_internal(&app, state.inner(), &account_key, label)
            .await?;

    let _ = tray::refresh_macos_tray_snapshot(&app);
    Ok(resolved_label)
}

#[tauri::command]
async fn update_api_account(
    app: AppHandle,
    state: State<'_, AppState>,
    account_key: String,
    input: UpdateApiAccountInput,
) -> Result<AccountSummary, String> {
    let summary =
        account_service::update_api_account_internal(&app, state.inner(), &account_key, input)
            .await?;

    let _ = tray::refresh_macos_tray_snapshot(&app);
    Ok(summary)
}

#[tauri::command]
async fn update_api_account_keys(
    app: AppHandle,
    state: State<'_, AppState>,
    account_key: String,
    keys: Vec<UpdateApiAccountKeyInput>,
) -> Result<AccountSummary, String> {
    let summary =
        account_service::update_api_account_keys_internal(&app, state.inner(), &account_key, keys)
            .await?;
    Ok(summary)
}

#[tauri::command]
async fn probe_api_account_key(
    app: AppHandle,
    state: State<'_, AppState>,
    account_key: String,
    key_id: String,
) -> Result<AccountSummary, String> {
    let summary =
        account_service::probe_api_account_key_internal(&app, state.inner(), &account_key, &key_id)
            .await?;
    Ok(summary)
}

#[tauri::command]
async fn update_account_tags(
    app: AppHandle,
    state: State<'_, AppState>,
    account_key: String,
    tags: Vec<String>,
) -> Result<Vec<String>, String> {
    let resolved_tags =
        account_service::update_account_tags_internal(&app, state.inner(), &account_key, tags)
            .await?;
    let _ = tray::refresh_macos_tray_snapshot(&app);
    Ok(resolved_tags)
}

#[tauri::command]
async fn refresh_all_usage(
    app: AppHandle,
    state: State<'_, AppState>,
    force_auth_refresh: Option<bool>,
) -> Result<Vec<AccountSummary>, String> {
    let summaries = account_service::refresh_all_usage_internal(
        &app,
        state.inner(),
        force_auth_refresh.unwrap_or(false),
    )
    .await?;
    let _ = tray::update_macos_tray_snapshot(&app, &summaries);
    Ok(summaries)
}

#[tauri::command]
async fn refresh_usage_for_account_keys(
    app: AppHandle,
    state: State<'_, AppState>,
    account_keys: Vec<String>,
    force_auth_refresh: Option<bool>,
) -> Result<Vec<AccountSummary>, String> {
    let summaries = account_service::refresh_usage_for_account_keys_internal(
        &app,
        state.inner(),
        account_keys,
        force_auth_refresh.unwrap_or(false),
    )
    .await?;
    let _ = tray::update_macos_tray_snapshot(&app, &summaries);
    Ok(summaries)
}

#[tauri::command]
async fn refresh_all_api_quota(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<AccountSummary>, String> {
    let summaries = account_service::refresh_all_api_quota_internal(&app, state.inner()).await?;
    let _ = tray::update_macos_tray_snapshot(&app, &summaries);
    Ok(summaries)
}

#[tauri::command]
async fn refresh_api_quota_for_account_keys(
    app: AppHandle,
    state: State<'_, AppState>,
    account_keys: Vec<String>,
) -> Result<Vec<AccountSummary>, String> {
    let summaries = account_service::refresh_api_quota_for_account_keys_internal(
        &app,
        state.inner(),
        account_keys,
    )
    .await?;
    let _ = tray::update_macos_tray_snapshot(&app, &summaries);
    Ok(summaries)
}

#[tauri::command]
async fn get_codex_token_usage() -> Result<token_usage::CodexTokenUsageSnapshot, String> {
    tauri::async_runtime::spawn_blocking(token_usage::collect_codex_token_usage_snapshot)
        .await
        .map_err(|error| format!("统计 Codex token 用量失败: {error}"))?
}

#[tauri::command]
async fn get_app_settings(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSettings, String> {
    settings_service::get_app_settings_internal(&app, state.inner()).await
}

#[tauri::command]
async fn update_app_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: AppSettingsPatch,
) -> Result<AppSettings, String> {
    let settings =
        settings_service::update_app_settings_internal(&app, state.inner(), patch).await?;
    let _ = tray::refresh_macos_tray_snapshot(&app);
    Ok(settings)
}

#[tauri::command]
async fn enable_codex_multi_model_mode(
    app: AppHandle,
) -> Result<codex_multimodel::MultiModelModeResult, String> {
    match codex_multimodel::enable_multi_model_mode(&app) {
        Ok(result) => {
            log::info!(
                "多模型模式已启用: status={}, workspace={}",
                result.status,
                result.workspace
            );
            Ok(result)
        }
        Err(error) => {
            log::error!("多模型模式启用失败: {error}");
            Err(error)
        }
    }
}

#[tauri::command]
async fn reset_codex_multi_model_mode(
    app: AppHandle,
) -> Result<codex_multimodel::MultiModelModeResult, String> {
    match codex_multimodel::reset_multi_model_mode(&app) {
        Ok(result) => {
            log::info!(
                "多模型模式已重置: status={}, workspace={}",
                result.status,
                result.workspace
            );
            Ok(result)
        }
        Err(error) => {
            log::error!("多模型模式重置失败: {error}");
            Err(error)
        }
    }
}

#[tauri::command]
async fn test_notification_target(target: NotificationTargetConfig) -> Result<(), String> {
    notification_service::test_notification_target(target).await
}

#[tauri::command]
async fn test_notification_provider(
    provider: NotificationProviderConfig,
) -> Result<String, String> {
    notification_service::test_notification_provider(provider).await
}

#[tauri::command]
async fn test_aggregate_notification(
    target: NotificationTargetConfig,
    providers: Vec<NotificationProviderConfig>,
) -> Result<(), String> {
    notification_service::test_aggregate_notification(target, providers).await
}

#[tauri::command]
async fn discover_telegram_chats(
    bot_token: String,
) -> Result<notification_service::TelegramChatDiscoveryResult, String> {
    notification_service::discover_telegram_chats(bot_token).await
}

#[tauri::command]
fn detect_codex_app() -> Result<Option<String>, String> {
    Ok(cli::find_codex_app_path().map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
fn list_installed_editor_apps() -> Result<Vec<InstalledEditorApp>, String> {
    Ok(editor_apps::list_installed_editor_apps())
}

#[tauri::command]
fn is_opencode_desktop_app_installed() -> Result<bool, String> {
    Ok(opencode::is_opencode_desktop_app_installed())
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("仅允许打开 http/https 链接".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("打开外部链接失败: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        // Avoid `cmd /C start` here. OAuth URLs contain `&`, and cmd treats them
        // as command separators unless they are shell-escaped very carefully.
        // Prefer the Windows URL protocol handler so the link goes to the
        // user's default browser instead of opening a File Explorer window.
        let mut primary = new_background_command("rundll32.exe");
        primary
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn()
            .or_else(|primary_error| {
                let mut fallback = new_background_command("explorer.exe");
                fallback.arg(&url).spawn().map_err(|fallback_error| {
                    format!("打开外部链接失败: rundll32={primary_error}; explorer={fallback_error}")
                })
            })?;
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("打开外部链接失败: {e}"))?;
        Ok(())
    }
}

#[tauri::command]
async fn pick_codex_launch_path(
    kind: String,
    current_path: Option<String>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = FileDialog::new().set_title("选择 Codex 启动路径");

        if let Some(current_path) = current_path {
            let current_path = std::path::PathBuf::from(current_path);
            let initial_dir = if current_path.is_dir() {
                current_path
            } else {
                current_path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or(current_path)
            };
            dialog = dialog.set_directory(initial_dir);
        }

        let selected = match kind.as_str() {
            "file" => dialog.pick_file(),
            "directory" => dialog.pick_folder(),
            _ => return Err("不支持的路径选择类型".to_string()),
        };

        Ok(selected.map(|path| path.to_string_lossy().to_string()))
    })
    .await
    .map_err(|error| format!("打开 Codex 路径选择器失败: {error}"))?
}

#[tauri::command]
async fn prepare_oauth_login(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<PreparedOauthLogin, String> {
    let _oauth_guard = state.oauth_flow_lock.lock().await;
    stop_oauth_callback_listener(state.inner()).await;
    let (listener, redirect_port) = bind_oauth_callback_listener(auth::oauth_redirect_port())?;
    let (mut pending, prepared) = auth::prepare_oauth_login(redirect_port)?;
    pending.reauthorize_account_id = account_id.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    {
        let mut guard = state.pending_oauth_login.lock().await;
        *guard = Some(pending.clone());
    }
    if let Err(error) = start_oauth_callback_listener(&app, state.inner(), listener, &pending).await
    {
        let mut guard = state.pending_oauth_login.lock().await;
        *guard = None;
        return Err(error);
    }
    Ok(prepared)
}

#[tauri::command]
async fn complete_oauth_callback_login(
    app: AppHandle,
    state: State<'_, AppState>,
    callback_url: String,
) -> Result<ImportAccountsResult, String> {
    let _oauth_guard = state.oauth_flow_lock.lock().await;
    let pending = {
        let guard = state.pending_oauth_login.lock().await;
        guard
            .clone()
            .ok_or_else(|| "请先打开授权页面".to_string())?
    };
    let result = complete_oauth_login_internal(&app, state.inner(), &callback_url).await?;
    clear_pending_oauth_if_matches(state.inner(), &pending.state).await;
    stop_oauth_callback_listener(state.inner()).await;
    Ok(result)
}

#[tauri::command]
async fn cancel_oauth_login(state: State<'_, AppState>) -> Result<(), String> {
    let _oauth_guard = state.oauth_flow_lock.lock().await;
    {
        let mut guard = state.pending_oauth_login.lock().await;
        *guard = None;
    }
    stop_oauth_callback_listener(state.inner()).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::bind_oauth_callback_listener;
    use super::build_oauth_callback_url;
    use super::is_auth_related_usage_error;
    #[cfg(target_os = "windows")]
    use super::select_codex_desktop_user_data_path;
    use std::net::TcpListener;
    #[cfg(target_os = "windows")]
    use std::path::Path;

    #[test]
    fn build_oauth_callback_url_uses_redirect_origin_and_runtime_query() {
        let callback_url = build_oauth_callback_url(
            "http://localhost:17888/auth/callback",
            "/auth/callback?code=abc&state=xyz",
        )
        .expect("callback url should be built");

        assert_eq!(
            callback_url,
            "http://localhost:17888/auth/callback?code=abc&state=xyz"
        );
    }

    #[test]
    fn bind_oauth_callback_listener_falls_back_when_preferred_port_is_busy() {
        let occupied = TcpListener::bind(("127.0.0.1", 0)).expect("should bind a local test port");
        let preferred_port = occupied
            .local_addr()
            .expect("should read local addr")
            .port();

        let (_listener, resolved_port) =
            bind_oauth_callback_listener(preferred_port).expect("bind should fall back");

        assert_ne!(resolved_port, preferred_port);
    }

    #[test]
    fn auth_related_usage_error_detects_stale_oauth_failures() {
        assert!(is_auth_related_usage_error("授权过期，请重新登录授权。"));
        assert!(is_auth_related_usage_error(
            "401 Unauthorized: Encountered invalidated oauth token for user"
        ));
        assert!(is_auth_related_usage_error(
            "令牌刷新失败: refresh_token_reused"
        ));
        assert!(!is_auth_related_usage_error("连接 NewAPI 额度接口失败"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn multi_model_launch_uses_stable_isolated_electron_user_data() {
        let workspace = Path::new(r"C:\CodexDeck\codexdeck-multimodel");

        let selected = select_codex_desktop_user_data_path(
            true,
            Some(workspace),
            Some(Path::new(r"C:\Users\test\AppData\Roaming")),
            true,
        );

        assert_eq!(
            selected,
            Some(workspace.join("electron-user-data")),
            "the controlled desktop must not share the official Electron session"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn standard_launch_preserves_explicit_electron_user_data_override() {
        let selected = select_codex_desktop_user_data_path(
            false,
            None,
            Some(Path::new(r"C:\Users\test\AppData\Roaming")),
            true,
        );

        assert_eq!(selected, None);
    }
}

fn model_router_backup_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_paths::app_data_dir(app)?.join("model-router-profile-backup"))
}

fn model_router_backup_state(
    settings: &AppSettings,
) -> profile_files::ActiveCodexProfileBackupState {
    profile_files::ActiveCodexProfileBackupState {
        active_account_id: settings.active_account_id.clone(),
        active_hybrid_profile: settings.active_hybrid_profile.clone(),
        codex_auth_existed: false,
        codex_config_existed: false,
    }
}

fn select_model_router_relay_account_id(
    store: &models::AccountsStore,
    requested_id: Option<String>,
) -> Result<String, String> {
    if let Some(requested_id) = requested_id.filter(|value| !value.trim().is_empty()) {
        let account = store
            .accounts
            .iter()
            .find(|account| account.id == requested_id)
            .ok_or_else(|| "找不到要用于路由模式的 API 条目。".to_string())?;
        if !matches!(account.source_kind, models::AccountSourceKind::Relay) {
            return Err("路由模式只能选择 API 条目。".to_string());
        }
        return Ok(requested_id);
    }

    store
        .settings
        .model_router_account_id
        .as_ref()
        .and_then(|account_id| {
            store
                .accounts
                .iter()
                .find(|account| {
                    account.id == *account_id
                        && matches!(account.source_kind, models::AccountSourceKind::Relay)
                })
                .map(|account| account.id.clone())
        })
        .or_else(|| {
            store
                .settings
                .active_account_id
                .as_ref()
                .and_then(|account_id| {
                    store
                        .accounts
                        .iter()
                        .find(|account| {
                            account.id == *account_id
                                && matches!(account.source_kind, models::AccountSourceKind::Relay)
                        })
                        .map(|account| account.id.clone())
                })
        })
        .or_else(|| {
            store
                .accounts
                .iter()
                .find(|account| matches!(account.source_kind, models::AccountSourceKind::Relay))
                .map(|account| account.id.clone())
        })
        .ok_or_else(|| "路由模式需要至少一个 API 条目。".to_string())
}

async fn disable_model_router_mode_for_store(
    app: &AppHandle,
    state: &AppState,
    store: &mut models::AccountsStore,
    next_router_account_id: Option<String>,
) -> Result<(), String> {
    let preserved_router_account_id =
        next_router_account_id.or_else(|| store.settings.model_router_account_id.clone());
    model_router::stop_model_router(state).await;
    if store.settings.model_router_enabled {
        let backup_state =
            profile_files::restore_active_codex_profile_backup(&model_router_backup_dir(app)?)?;
        store.settings.active_account_id = backup_state.active_account_id;
        store.settings.active_hybrid_profile = backup_state.active_hybrid_profile;
    }
    store.settings.model_router_enabled = false;
    store.settings.model_router_account_id = preserved_router_account_id;
    Ok(())
}

async fn apply_relay_model_router_profile_for_store(
    app: &AppHandle,
    state: &AppState,
    store: &mut models::AccountsStore,
    relay_account_id: Option<String>,
    create_backup_if_needed: bool,
) -> Result<String, String> {
    let account_id = select_model_router_relay_account_id(store, relay_account_id)?;
    let relay_index = store
        .accounts
        .iter()
        .position(|account| account.id == account_id)
        .ok_or_else(|| "找不到要用于路由模式的 API 条目。".to_string())?;
    if !matches!(
        store.accounts[relay_index].source_kind,
        models::AccountSourceKind::Relay
    ) {
        return Err("路由模式只能选择 API 条目。".to_string());
    }

    if create_backup_if_needed && !store.settings.model_router_enabled {
        profile_files::create_active_codex_profile_backup(
            &model_router_backup_dir(app)?,
            &model_router_backup_state(&store.settings),
        )?;
    }

    let (router_base_url, router_entries) =
        model_router::ensure_model_router_for_store(state, store).await?;
    let store_path = store::account_store_path_for_app(app)?;
    {
        let stored_relay = &mut store.accounts[relay_index];
        profile_files::sync_account_profile_in_store_path(&store_path, stored_relay)?;
    }
    let mut router_account = store.accounts[relay_index].clone();
    if let Some(first_model) = router_entries
        .iter()
        .find(|entry| entry.enabled && !entry.model.trim().is_empty())
        .map(|entry| entry.model.clone())
    {
        router_account.model_name = Some(first_model);
    }
    profile_files::apply_relay_account_profile_with_provider_base_url(
        &router_account,
        &router_base_url,
        &router_entries,
    )?;
    profile_files::apply_model_instructions_fix_setting(
        store.settings.codex_model_instructions_fix_enabled,
    )?;
    store.settings.model_router_enabled = true;
    store.settings.model_router_account_id = Some(account_id.clone());
    store.settings.active_account_id = Some(account_id.clone());
    store.settings.active_hybrid_profile = None;
    Ok(account_id)
}

#[tauri::command]
async fn switch_account_and_launch(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    workspace_path: Option<String>,
    launch_codex: Option<bool>,
    use_model_router: Option<bool>,
    restart_editors_on_switch: Option<bool>,
    restart_editor_targets: Option<Vec<EditorAppId>>,
) -> Result<SwitchAccountResult, String> {
    let store = {
        let _guard = state.store_lock.lock().await;
        store::load_store(&app)?
    };

    let mut account = store
        .accounts
        .iter()
        .find(|account| account.id == id)
        .cloned()
        .ok_or_else(|| "找不到要切换的账号".to_string())?;

    if matches!(account.source_kind, models::AccountSourceKind::Chatgpt)
        && auth::auth_tokens_need_refresh(&account.auth_json)
    {
        account = refresh_chatgpt_account_before_switch(&app, state.inner(), &id, true).await?;
    }

    let should_sync_opencode = store.settings.sync_opencode_openai_auth;
    let should_restart_opencode_desktop =
        should_sync_opencode && store.settings.restart_opencode_desktop_on_switch;
    let should_restart_editors =
        restart_editors_on_switch.unwrap_or(store.settings.restart_editors_on_switch);
    let effective_restart_targets =
        restart_editor_targets.unwrap_or_else(|| store.settings.restart_editor_targets.clone());
    let configured_codex_launch_path = store.settings.codex_launch_path.clone();
    let disable_gpu_acceleration = store.settings.codex_disable_gpu_acceleration;
    // 向后兼容：旧前端未传参数时仍按“切换并启动”处理。
    let should_launch_codex = launch_codex.unwrap_or(true);
    let should_use_model_router = use_model_router.unwrap_or(false)
        && matches!(account.source_kind, models::AccountSourceKind::Relay);
    {
        let _guard = state.store_lock.lock().await;
        let mut latest_store = store::load_store(&app)?;
        let router_context = if should_use_model_router {
            if !latest_store.settings.model_router_enabled {
                profile_files::create_active_codex_profile_backup(
                    &model_router_backup_dir(&app)?,
                    &model_router_backup_state(&latest_store.settings),
                )?;
            }
            Some(model_router::ensure_model_router_for_store(state.inner(), &latest_store).await?)
        } else {
            disable_model_router_mode_for_store(&app, state.inner(), &mut latest_store, None)
                .await?;
            None
        };
        let stored_account = latest_store
            .accounts
            .iter_mut()
            .find(|stored| stored.id == id)
            .ok_or_else(|| "找不到要切换的账号".to_string())?;
        profile_files::sync_account_profile_in_store_path(
            &store::account_store_path_for_app(&app)?,
            stored_account,
        )?;
        if let Some((router_base_url, router_entries)) = router_context.as_ref() {
            let mut router_account = stored_account.clone();
            if let Some(first_model) = router_entries
                .iter()
                .find(|entry| entry.enabled && !entry.model.trim().is_empty())
                .map(|entry| entry.model.clone())
            {
                router_account.model_name = Some(first_model);
            }
            profile_files::apply_relay_account_profile_with_provider_base_url(
                &router_account,
                router_base_url,
                router_entries,
            )?;
        } else {
            let account_snapshot = stored_account.clone();
            match account_snapshot.source_kind {
                models::AccountSourceKind::Relay => {
                    let provider_base_url =
                        resolve_relay_provider_base_url(state.inner(), &account_snapshot).await?;
                    let model_catalog_entries = account_snapshot.enabled_model_catalog();
                    profile_files::apply_relay_account_profile_with_provider_base_url(
                        &account_snapshot,
                        &provider_base_url,
                        &model_catalog_entries,
                    )?;
                }
                models::AccountSourceKind::Chatgpt => {
                    profile_files::apply_account_profile(stored_account)?;
                }
            }
        }
        let thread_provider_backup_dir = app_paths::codex_state_provider_backup_dir()?;
        let synced_thread_count = match stored_account.source_kind {
            models::AccountSourceKind::Relay => {
                let provider_id = profile_files::relay_provider_id_for_account(stored_account)
                    .ok_or_else(|| "API 条目资料不完整".to_string())?;
                session_provider_sync::sync_codex_thread_providers_for_relay(
                    &provider_id,
                    &thread_provider_backup_dir,
                )?
            }
            models::AccountSourceKind::Chatgpt => {
                session_provider_sync::sync_codex_thread_providers_for_chatgpt(
                    &thread_provider_backup_dir,
                )?
            }
        };
        if synced_thread_count > 0 {
            log::info!("已同步 {synced_thread_count} 条 Codex 线程 provider");
        }
        profile_files::apply_model_instructions_fix_setting(
            latest_store.settings.codex_model_instructions_fix_enabled,
        )?;
        latest_store.settings.active_account_id = Some(stored_account.id.clone());
        latest_store.settings.active_hybrid_profile = None;
        latest_store.settings.model_router_enabled = should_use_model_router;
        if should_use_model_router {
            latest_store.settings.model_router_account_id = Some(stored_account.id.clone());
        }
        account = stored_account.clone();
        store::save_store(&app, &latest_store)?;
    }
    let _ = tray::refresh_macos_tray_snapshot(&app);

    let mut opencode_synced = false;
    let mut opencode_sync_error = None;
    let mut opencode_desktop_restarted = false;
    let mut opencode_desktop_restart_error = None;
    if should_sync_opencode {
        match if matches!(account.source_kind, models::AccountSourceKind::Chatgpt) {
            opencode::sync_openai_auth_from_codex_auth(&account.auth_json)
        } else {
            Err("当前条目为 API 中转站配置，无法同步为 opencode 的 OAuth 登录态。".to_string())
        } {
            Ok(()) => {
                opencode_synced = true;
                if should_restart_opencode_desktop {
                    match opencode::restart_opencode_desktop_app() {
                        Ok(()) => {
                            opencode_desktop_restarted = true;
                        }
                        Err(err) => {
                            log::warn!("重启 opencode 桌面端失败: {err}");
                            opencode_desktop_restart_error = Some(err);
                        }
                    }
                }
            }
            Err(err) => {
                log::warn!("同步 opencode OpenAI 认证失败: {err}");
                opencode_sync_error = Some(err);
            }
        }
    }

    let (restarted_editor_apps, editor_restart_error) = if should_restart_editors {
        editor_apps::restart_selected_editor_apps(&effective_restart_targets)
    } else {
        (Vec::new(), None)
    };

    if !should_launch_codex {
        return Ok(SwitchAccountResult {
            account_id: account.account_id,
            launched_app_path: None,
            used_fallback_cli: false,
            opencode_synced,
            opencode_sync_error,
            opencode_desktop_restarted,
            opencode_desktop_restart_error,
            restarted_editor_apps,
            editor_restart_error,
        });
    }

    let stopped_codex_before_prepare = stop_running_codex_before_multi_model_prepare(
        &app,
        configured_codex_launch_path.as_deref(),
    );
    let configured_codex_launch_path =
        prepare_codex_launch_path_for_current_settings(&app, configured_codex_launch_path)?;
    let multi_model_launch = is_codex_multi_model_mode_enabled(&app);

    // 切换时强制结束旧实例，避免触发“是否退出”确认弹窗。
    if !stopped_codex_before_prepare {
        force_stop_running_codex(configured_codex_launch_path.as_deref(), multi_model_launch);
    }

    let mut app_launch_error = None;
    let configured_app_path =
        cli::find_configured_codex_app_path(configured_codex_launch_path.as_deref());
    if let Some(path) = configured_app_path.clone().or_else(|| {
        (!multi_model_launch)
            .then(cli::find_codex_app_path)
            .flatten()
    }) {
        match launch_codex_app(
            &path,
            workspace_path.as_deref(),
            multi_model_launch,
            disable_gpu_acceleration,
        ) {
            Ok(()) => {
                return Ok(SwitchAccountResult {
                    account_id: account.account_id,
                    launched_app_path: Some(path.to_string_lossy().to_string()),
                    used_fallback_cli: false,
                    opencode_synced,
                    opencode_sync_error,
                    opencode_desktop_restarted,
                    opencode_desktop_restart_error,
                    restarted_editor_apps,
                    editor_restart_error,
                });
            }
            Err(error) => {
                log::warn!("通过 Codex 应用路径启动失败 {}: {}", path.display(), error);
                app_launch_error = Some(error);
            }
        }
    }

    if multi_model_launch {
        return Err(app_launch_error.unwrap_or_else(|| {
            "多模型模式只允许启动受控 Codex 副本，但当前受控启动路径不可用。请在设置中重置后重新开启多模型模式。".to_string()
        }));
    }

    #[cfg(target_os = "windows")]
    if cli::has_windows_store_codex_app() {
        app_paths::apply_codex_home_process_env()?;
        match cli::launch_windows_store_codex() {
            Ok(()) => {
                return Ok(SwitchAccountResult {
                    account_id: account.account_id,
                    launched_app_path: None,
                    used_fallback_cli: false,
                    opencode_synced,
                    opencode_sync_error,
                    opencode_desktop_restarted,
                    opencode_desktop_restart_error,
                    restarted_editor_apps,
                    editor_restart_error,
                });
            }
            Err(error) => {
                log::warn!("通过 Windows Store AUMID 启动 Codex 失败: {error}");
                app_launch_error = Some(match app_launch_error {
                    Some(previous_error) => {
                        format!("{previous_error}；且通过 Windows Store AUMID 启动失败: {error}")
                    }
                    None => format!("通过 Windows Store AUMID 启动失败: {error}"),
                });
            }
        }
    }

    launch_codex_cli_app(
        configured_codex_launch_path.as_deref(),
        workspace_path.as_deref(),
        app_launch_error.as_deref(),
    )?;

    Ok(SwitchAccountResult {
        account_id: account.account_id,
        launched_app_path: None,
        used_fallback_cli: true,
        opencode_synced,
        opencode_sync_error,
        opencode_desktop_restarted,
        opencode_desktop_restart_error,
        restarted_editor_apps,
        editor_restart_error,
    })
}

#[tauri::command]
async fn set_model_router_mode(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
    relay_account_id: Option<String>,
) -> Result<AppSettings, String> {
    let settings = {
        let _guard = state.store_lock.lock().await;
        let mut latest_store = store::load_store(&app)?;

        if enabled {
            apply_relay_model_router_profile_for_store(
                &app,
                state.inner(),
                &mut latest_store,
                relay_account_id,
                true,
            )
            .await?;
        } else {
            disable_model_router_mode_for_store(
                &app,
                state.inner(),
                &mut latest_store,
                relay_account_id,
            )
            .await?;
        }

        let settings = latest_store.settings.clone();
        store::save_store(&app, &latest_store)?;
        settings
    };
    let _ = tray::refresh_macos_tray_snapshot(&app);
    Ok(settings)
}

fn prepare_codex_launch_path_for_current_settings(
    app: &AppHandle,
    configured_codex_launch_path: Option<String>,
) -> Result<Option<String>, String> {
    let store = store::load_store(app)?;
    if !store.settings.codex_multi_model_mode_enabled {
        return Ok(configured_codex_launch_path);
    }

    let Some(managed_launch_path) = codex_multimodel::prepare_managed_codex_launch_path(app)?
    else {
        return Ok(configured_codex_launch_path);
    };

    let mut latest_store = store::load_store(app)?;
    profile_files::apply_model_instructions_fix_setting(
        latest_store.settings.codex_model_instructions_fix_enabled,
    )?;
    latest_store.settings.codex_launch_path = Some(managed_launch_path.clone());
    store::save_store(app, &latest_store)?;
    Ok(Some(managed_launch_path))
}

fn is_codex_multi_model_mode_enabled(app: &AppHandle) -> bool {
    store::load_store(app)
        .ok()
        .is_some_and(|store| store.settings.codex_multi_model_mode_enabled)
}

fn stop_running_codex_before_multi_model_prepare(
    app: &AppHandle,
    configured_codex_launch_path: Option<&str>,
) -> bool {
    if !is_codex_multi_model_mode_enabled(app) {
        return false;
    }
    let stop_path = store::load_store(app)
        .ok()
        .and_then(|store| store.settings.codex_multi_model_controlled_exe_path)
        .or_else(|| configured_codex_launch_path.map(ToString::to_string));
    let Some(stop_path) = stop_path else {
        return false;
    };
    force_stop_running_codex(Some(&stop_path), true);
    true
}

#[tauri::command]
async fn switch_hybrid_account_and_launch(
    app: AppHandle,
    state: State<'_, AppState>,
    chatgpt_account_id: String,
    relay_account_id: String,
    workspace_path: Option<String>,
    launch_codex: Option<bool>,
    use_model_router: Option<bool>,
    restart_editors_on_switch: Option<bool>,
    restart_editor_targets: Option<Vec<EditorAppId>>,
) -> Result<SwitchAccountResult, String> {
    let store = {
        let _guard = state.store_lock.lock().await;
        store::load_store(&app)?
    };

    let chatgpt_account = store
        .accounts
        .iter()
        .find(|account| account.id == chatgpt_account_id)
        .cloned()
        .ok_or_else(|| "找不到要用于混合模式的 ChatGPT 官方账号".to_string())?;
    if !matches!(
        chatgpt_account.source_kind,
        models::AccountSourceKind::Chatgpt
    ) {
        return Err("混合模式需要选择一个 ChatGPT 官方账号。".to_string());
    }
    if auth::auth_tokens_need_refresh(&chatgpt_account.auth_json) {
        let _ =
            refresh_chatgpt_account_before_switch(&app, state.inner(), &chatgpt_account_id, false)
                .await?;
    }

    let should_restart_editors =
        restart_editors_on_switch.unwrap_or(store.settings.restart_editors_on_switch);
    let effective_restart_targets =
        restart_editor_targets.unwrap_or_else(|| store.settings.restart_editor_targets.clone());
    let configured_codex_launch_path = store.settings.codex_launch_path.clone();
    let disable_gpu_acceleration = store.settings.codex_disable_gpu_acceleration;
    let should_launch_codex = launch_codex.unwrap_or(true);
    let should_use_model_router = use_model_router.unwrap_or(false);
    let relay_account;
    {
        let _guard = state.store_lock.lock().await;
        let mut latest_store = store::load_store(&app)?;
        let router_context = if should_use_model_router {
            if !latest_store.settings.model_router_enabled {
                profile_files::create_active_codex_profile_backup(
                    &model_router_backup_dir(&app)?,
                    &model_router_backup_state(&latest_store.settings),
                )?;
            }
            Some(model_router::ensure_model_router_for_store(state.inner(), &latest_store).await?)
        } else {
            disable_model_router_mode_for_store(&app, state.inner(), &mut latest_store, None)
                .await?;
            None
        };
        let store_path = store::account_store_path_for_app(&app)?;
        let chatgpt_index = latest_store
            .accounts
            .iter()
            .position(|account| account.id == chatgpt_account_id)
            .ok_or_else(|| "找不到要用于混合模式的 ChatGPT 官方账号".to_string())?;
        if !matches!(
            latest_store.accounts[chatgpt_index].source_kind,
            models::AccountSourceKind::Chatgpt
        ) {
            return Err("混合模式需要选择一个 ChatGPT 官方账号。".to_string());
        }
        let relay_index = latest_store
            .accounts
            .iter()
            .position(|account| account.id == relay_account_id)
            .ok_or_else(|| "找不到要用于混合模式的 API 条目".to_string())?;
        if !matches!(
            latest_store.accounts[relay_index].source_kind,
            models::AccountSourceKind::Relay
        ) {
            return Err("混合模式需要选择一个 API 条目。".to_string());
        }

        {
            let stored_relay = &mut latest_store.accounts[relay_index];
            profile_files::sync_account_profile_in_store_path(&store_path, stored_relay)?;
        }
        let stored_chatgpt = latest_store.accounts[chatgpt_index].clone();
        let stored_relay = latest_store.accounts[relay_index].clone();
        if let Some((router_base_url, router_entries)) = router_context.as_ref() {
            let mut router_relay = stored_relay.clone();
            if let Some(first_model) = router_entries
                .iter()
                .find(|entry| entry.enabled && !entry.model.trim().is_empty())
                .map(|entry| entry.model.clone())
            {
                router_relay.model_name = Some(first_model);
            }
            profile_files::apply_hybrid_account_profile_with_provider_base_url_and_catalog_entries(
                &stored_chatgpt,
                &router_relay,
                router_base_url,
                router_entries,
            )?;
        } else {
            profile_files::apply_hybrid_account_profile(&stored_chatgpt, &stored_relay)?;
        }

        let thread_provider_backup_dir = app_paths::codex_state_provider_backup_dir()?;
        let provider_id = profile_files::relay_provider_id_for_account(&stored_relay)
            .ok_or_else(|| "API 条目资料不完整".to_string())?;
        let synced_thread_count = session_provider_sync::sync_codex_thread_providers_for_relay(
            &provider_id,
            &thread_provider_backup_dir,
        )?;
        if synced_thread_count > 0 {
            log::info!("混合模式已同步 {synced_thread_count} 条 Codex 线程 provider");
        }
        profile_files::apply_model_instructions_fix_setting(
            latest_store.settings.codex_model_instructions_fix_enabled,
        )?;
        latest_store.settings.active_account_id = Some(stored_relay.id.clone());
        latest_store.settings.active_hybrid_profile = Some(models::ActiveHybridProfile {
            chatgpt_account_id: stored_chatgpt.id.clone(),
            relay_account_id: stored_relay.id.clone(),
        });
        latest_store.settings.model_router_enabled = should_use_model_router;
        if should_use_model_router {
            latest_store.settings.model_router_account_id = Some(stored_relay.id.clone());
        }
        relay_account = stored_relay;
        store::save_store(&app, &latest_store)?;
    }
    let _ = tray::refresh_macos_tray_snapshot(&app);

    let (restarted_editor_apps, editor_restart_error) = if should_restart_editors {
        editor_apps::restart_selected_editor_apps(&effective_restart_targets)
    } else {
        (Vec::new(), None)
    };

    if !should_launch_codex {
        return Ok(SwitchAccountResult {
            account_id: relay_account.account_id,
            launched_app_path: None,
            used_fallback_cli: false,
            opencode_synced: false,
            opencode_sync_error: None,
            opencode_desktop_restarted: false,
            opencode_desktop_restart_error: None,
            restarted_editor_apps,
            editor_restart_error,
        });
    }

    let stopped_codex_before_prepare = stop_running_codex_before_multi_model_prepare(
        &app,
        configured_codex_launch_path.as_deref(),
    );
    let configured_codex_launch_path =
        prepare_codex_launch_path_for_current_settings(&app, configured_codex_launch_path)?;
    let multi_model_launch = is_codex_multi_model_mode_enabled(&app);

    if !stopped_codex_before_prepare {
        force_stop_running_codex(configured_codex_launch_path.as_deref(), multi_model_launch);
    }

    let mut app_launch_error = None;
    let configured_app_path =
        cli::find_configured_codex_app_path(configured_codex_launch_path.as_deref());
    if let Some(path) = configured_app_path.clone().or_else(|| {
        (!multi_model_launch)
            .then(cli::find_codex_app_path)
            .flatten()
    }) {
        match launch_codex_app(
            &path,
            workspace_path.as_deref(),
            multi_model_launch,
            disable_gpu_acceleration,
        ) {
            Ok(()) => {
                return Ok(SwitchAccountResult {
                    account_id: relay_account.account_id,
                    launched_app_path: Some(path.to_string_lossy().to_string()),
                    used_fallback_cli: false,
                    opencode_synced: false,
                    opencode_sync_error: None,
                    opencode_desktop_restarted: false,
                    opencode_desktop_restart_error: None,
                    restarted_editor_apps,
                    editor_restart_error,
                });
            }
            Err(error) => {
                log::warn!("通过 Codex 应用路径启动失败 {}: {}", path.display(), error);
                app_launch_error = Some(error);
            }
        }
    }

    if multi_model_launch {
        return Err(app_launch_error.unwrap_or_else(|| {
            "多模型模式只允许启动受控 Codex 副本，但当前受控启动路径不可用。请在设置中重置后重新开启多模型模式。".to_string()
        }));
    }

    #[cfg(target_os = "windows")]
    if cli::has_windows_store_codex_app() {
        app_paths::apply_codex_home_process_env()?;
        match cli::launch_windows_store_codex() {
            Ok(()) => {
                return Ok(SwitchAccountResult {
                    account_id: relay_account.account_id,
                    launched_app_path: None,
                    used_fallback_cli: false,
                    opencode_synced: false,
                    opencode_sync_error: None,
                    opencode_desktop_restarted: false,
                    opencode_desktop_restart_error: None,
                    restarted_editor_apps,
                    editor_restart_error,
                });
            }
            Err(error) => {
                log::warn!("通过 Windows Store AUMID 启动 Codex 失败: {error}");
                app_launch_error = Some(match app_launch_error {
                    Some(previous_error) => {
                        format!("{previous_error}；且通过 Windows Store AUMID 启动失败: {error}")
                    }
                    None => format!("通过 Windows Store AUMID 启动失败: {error}"),
                });
            }
        }
    }

    let mut cmd = cli::new_codex_command(configured_codex_launch_path.as_deref())?;
    app_paths::apply_codex_home_env(&mut cmd)?;
    cmd.arg("app");
    if let Some(workspace) = workspace_path.as_deref() {
        cmd.arg(workspace);
    }
    cmd.spawn().map_err(|e| {
        if let Some(app_launch_error) = app_launch_error.as_ref() {
            format!(
                "通过 Codex 应用路径启动失败: {app_launch_error}；且通过 codex app 启动失败: {e}"
            )
        } else {
            format!("未检测到本地 Codex 应用，且通过 codex app 启动失败: {e}")
        }
    })?;

    Ok(SwitchAccountResult {
        account_id: relay_account.account_id,
        launched_app_path: None,
        used_fallback_cli: true,
        opencode_synced: false,
        opencode_sync_error: None,
        opencode_desktop_restarted: false,
        opencode_desktop_restart_error: None,
        restarted_editor_apps,
        editor_restart_error,
    })
}

#[tauri::command]
async fn launch_current_codex_config(
    app: AppHandle,
    state: State<'_, AppState>,
    workspace_path: Option<String>,
    restart_editors_on_switch: Option<bool>,
    restart_editor_targets: Option<Vec<EditorAppId>>,
) -> Result<SwitchAccountResult, String> {
    let (
        configured_codex_launch_path,
        account_id,
        should_restart_editors,
        restart_targets,
        disable_gpu_acceleration,
    ) = {
        let _guard = state.store_lock.lock().await;
        let mut latest_store = store::load_store(&app)?;
        if !latest_store.settings.model_router_enabled {
            return Err("请先开启路由模式。".to_string());
        }
        let router_account_id = latest_store.settings.model_router_account_id.clone();
        let stored_account_id = apply_relay_model_router_profile_for_store(
            &app,
            state.inner(),
            &mut latest_store,
            router_account_id,
            false,
        )
        .await?;
        let account_id = latest_store
            .accounts
            .iter()
            .find(|account| account.id == stored_account_id)
            .map(|account| account.account_id.clone())
            .ok_or_else(|| "找不到路由模式默认 API 条目。".to_string())?;
        let configured_codex_launch_path = latest_store.settings.codex_launch_path.clone();
        let should_restart_editors =
            restart_editors_on_switch.unwrap_or(latest_store.settings.restart_editors_on_switch);
        let restart_targets = restart_editor_targets
            .unwrap_or_else(|| latest_store.settings.restart_editor_targets.clone());
        let disable_gpu_acceleration = latest_store.settings.codex_disable_gpu_acceleration;
        store::save_store(&app, &latest_store)?;
        (
            configured_codex_launch_path,
            account_id,
            should_restart_editors,
            restart_targets,
            disable_gpu_acceleration,
        )
    };
    let _ = tray::refresh_macos_tray_snapshot(&app);

    let (restarted_editor_apps, editor_restart_error) = if should_restart_editors {
        editor_apps::restart_selected_editor_apps(&restart_targets)
    } else {
        (Vec::new(), None)
    };

    let stopped_codex_before_prepare = stop_running_codex_before_multi_model_prepare(
        &app,
        configured_codex_launch_path.as_deref(),
    );
    let configured_codex_launch_path =
        prepare_codex_launch_path_for_current_settings(&app, configured_codex_launch_path)?;
    let multi_model_launch = is_codex_multi_model_mode_enabled(&app);

    let launch_result = launch_codex_using_current_config(
        configured_codex_launch_path.as_deref(),
        workspace_path.as_deref(),
        multi_model_launch,
        disable_gpu_acceleration,
        stopped_codex_before_prepare,
    )
    .await?;

    Ok(SwitchAccountResult {
        account_id,
        launched_app_path: launch_result.launched_app_path,
        used_fallback_cli: launch_result.used_fallback_cli,
        opencode_synced: false,
        opencode_sync_error: None,
        opencode_desktop_restarted: false,
        opencode_desktop_restart_error: None,
        restarted_editor_apps,
        editor_restart_error,
    })
}

async fn launch_codex_using_current_config(
    configured_codex_launch_path: Option<&str>,
    workspace_path: Option<&str>,
    multi_model_launch: bool,
    disable_gpu_acceleration: bool,
    already_stopped_codex: bool,
) -> Result<CodexLaunchOutcome, String> {
    if !already_stopped_codex {
        force_stop_running_codex(configured_codex_launch_path, multi_model_launch);
    }

    let mut app_launch_error = None;
    let configured_app_path = cli::find_configured_codex_app_path(configured_codex_launch_path);
    if let Some(path) = configured_app_path.clone().or_else(|| {
        (!multi_model_launch)
            .then(cli::find_codex_app_path)
            .flatten()
    }) {
        match launch_codex_app(
            &path,
            workspace_path,
            multi_model_launch,
            disable_gpu_acceleration,
        ) {
            Ok(()) => {
                return Ok(CodexLaunchOutcome {
                    launched_app_path: Some(path.to_string_lossy().to_string()),
                    used_fallback_cli: false,
                });
            }
            Err(error) => {
                log::warn!("通过 Codex 应用路径启动失败 {}: {}", path.display(), error);
                app_launch_error = Some(error);
            }
        }
    }

    if multi_model_launch {
        return Err(app_launch_error.unwrap_or_else(|| {
            "多模型模式只允许启动受控 Codex 副本，但当前受控启动路径不可用。请在设置中重置后重新开启多模型模式。".to_string()
        }));
    }

    #[cfg(target_os = "windows")]
    if cli::has_windows_store_codex_app() {
        app_paths::apply_codex_home_process_env()?;
        match cli::launch_windows_store_codex() {
            Ok(()) => {
                return Ok(CodexLaunchOutcome {
                    launched_app_path: None,
                    used_fallback_cli: false,
                });
            }
            Err(error) => {
                log::warn!("通过 Windows Store AUMID 启动 Codex 失败: {error}");
                app_launch_error = Some(match app_launch_error {
                    Some(previous_error) => {
                        format!("{previous_error}；且通过 Windows Store AUMID 启动失败: {error}")
                    }
                    None => format!("通过 Windows Store AUMID 启动失败: {error}"),
                });
            }
        }
    }

    launch_codex_cli_app(
        configured_codex_launch_path,
        workspace_path,
        app_launch_error.as_deref(),
    )?;

    Ok(CodexLaunchOutcome {
        launched_app_path: None,
        used_fallback_cli: true,
    })
}

fn launch_codex_cli_app(
    configured_codex_launch_path: Option<&str>,
    workspace_path: Option<&str>,
    app_launch_error: Option<&str>,
) -> Result<(), String> {
    let mut cmd = cli::new_codex_command(configured_codex_launch_path)?;
    app_paths::apply_codex_home_env(&mut cmd)?;
    cmd.arg("app");
    if let Some(workspace) = workspace_path {
        cmd.arg(workspace);
    }
    cmd.spawn().map_err(|e| {
        if let Some(app_launch_error) = app_launch_error {
            format!(
                "通过 Codex 应用路径启动失败: {app_launch_error}；且通过 codex app 启动失败: {e}"
            )
        } else {
            format!("未检测到本地 Codex 应用，且通过 codex app 启动失败: {e}")
        }
    })?;
    Ok(())
}

fn launch_codex_app(
    path: &std::path::Path,
    workspace_path: Option<&str>,
    multi_model_launch: bool,
    disable_gpu_acceleration: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = multi_model_launch;
        let _ = disable_gpu_acceleration;
        let mut cmd = Command::new("open");
        app_paths::apply_codex_home_env(&mut cmd)?;
        cmd.arg("-na").arg(path);
        if let Some(workspace) = workspace_path {
            cmd.arg(workspace);
        }
        let status = cmd
            .status()
            .map_err(|e| format!("启动 Codex 应用失败: {e}"))?;
        if !status.success() {
            return Err("启动 Codex 应用失败".to_string());
        }
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        if cli::is_windows_store_codex_path(path) {
            let arguments = workspace_path
                .map(|workspace| vec![workspace.to_string()])
                .unwrap_or_default();
            app_paths::apply_codex_home_process_env()?;
            return cli::launch_windows_store_codex_with_args(&arguments);
        }

        let mut cmd = new_background_command(path);
        app_paths::apply_codex_home_env(&mut cmd)?;
        apply_codex_desktop_user_data_env(&mut cmd, multi_model_launch)?;
        if disable_gpu_acceleration {
            cmd.arg("--disable-gpu");
        }
        if let Some(workspace) = workspace_path {
            cmd.arg(workspace);
        }
        cmd.spawn()
            .map_err(|e| format!("启动 Codex 应用失败: {e}"))?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = multi_model_launch;
        let _ = disable_gpu_acceleration;
        let mut cmd = Command::new(path);
        app_paths::apply_codex_home_env(&mut cmd)?;
        if let Some(workspace) = workspace_path {
            cmd.arg(workspace);
        }
        cmd.spawn()
            .map_err(|e| format!("启动 Codex 应用失败: {e}"))?;
        return Ok(());
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = path;
        let _ = workspace_path;
        let _ = multi_model_launch;
        let _ = disable_gpu_acceleration;
        Err("当前平台暂不支持直接启动 Codex 应用".to_string())
    }
}

#[cfg(target_os = "windows")]
fn select_codex_desktop_user_data_path(
    multi_model_launch: bool,
    multi_model_workspace: Option<&std::path::Path>,
    app_data: Option<&std::path::Path>,
    has_explicit_override: bool,
) -> Option<PathBuf> {
    if multi_model_launch {
        return multi_model_workspace.map(|workspace| workspace.join("electron-user-data"));
    }

    if has_explicit_override {
        return None;
    }

    app_data.map(|path| path.join("Codex"))
}

#[cfg(target_os = "windows")]
fn apply_codex_desktop_user_data_env(
    command: &mut std::process::Command,
    multi_model_launch: bool,
) -> Result<(), String> {
    let multi_model_workspace = if multi_model_launch {
        Some(codex_multimodel::workspace_dir()?)
    } else {
        None
    };
    let app_data = std::env::var_os("APPDATA");
    let app_data_path = app_data.as_deref().map(|value| std::path::Path::new(value));
    let has_explicit_override = std::env::var_os("CODEX_ELECTRON_USER_DATA_PATH").is_some()
        || std::env::var_os("CODEX_COMMAND_DESKTOP_USER_DATA_DIR").is_some();

    if let Some(user_data_path) = select_codex_desktop_user_data_path(
        multi_model_launch,
        multi_model_workspace.as_deref(),
        app_data_path,
        has_explicit_override,
    ) {
        command.env("CODEX_ELECTRON_USER_DATA_PATH", user_data_path);
    }

    Ok(())
}

async fn refresh_chatgpt_account_before_switch(
    app: &AppHandle,
    state: &AppState,
    account_id: &str,
    sync_current_auth_on_block: bool,
) -> Result<models::StoredAccount, String> {
    let _refresh_guard = state.auth_refresh_lock.lock().await;
    let mut account = {
        let _store_guard = state.store_lock.lock().await;
        store::load_store(app)?
            .accounts
            .into_iter()
            .find(|stored| stored.id == account_id)
            .ok_or_else(|| "找不到要切换的账号".to_string())?
    };

    if !matches!(account.source_kind, models::AccountSourceKind::Chatgpt) {
        return Ok(account);
    }
    if !auth::auth_tokens_need_refresh(&account.auth_json) {
        return Ok(account);
    }
    if account.auth_refresh_blocked {
        return Err(format!(
            "切换账号前刷新登录令牌失败: {}",
            account
                .auth_refresh_error
                .clone()
                .unwrap_or_else(|| "授权过期，请重新登录授权。".to_string())
        ));
    }

    let refreshed_auth = match auth::refresh_chatgpt_auth_tokens(&account.auth_json).await {
        Ok(refreshed_auth) => refreshed_auth,
        Err(error) => {
            let normalized_error = normalize_switch_refresh_error(&error);
            let should_block_refresh = normalized_error
                == "当前账号的 refresh_token 已失效或已被轮换，请重新登录授权。"
                || normalized_error == "当前账号授权已过期，请重新登录授权。";

            if should_block_refresh {
                let blocked_message = "授权过期，请重新登录授权。";
                match store::account_store_path_for_app(app) {
                    Ok(store_path) => {
                        let _store_guard = state.store_lock.lock().await;
                        if let Err(persist_error) =
                            store::update_account_group_refresh_state_in_path(
                                &store_path,
                                &account.account_key(),
                                None,
                                true,
                                Some(blocked_message),
                                utils::now_unix_seconds(),
                                sync_current_auth_on_block,
                            )
                        {
                            log::warn!("切换失败后写回账号停刷状态失败: {persist_error}");
                        }
                    }
                    Err(path_error) => {
                        log::warn!("切换失败后获取账号存储路径失败: {path_error}");
                    }
                }
            }

            return Err(format!("切换账号前刷新登录令牌失败: {normalized_error}"));
        }
    };

    let refreshed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("读取系统时间失败: {error}"))?
        .as_secs() as i64;
    let _store_guard = state.store_lock.lock().await;
    let mut latest_store = store::load_store(app)?;
    let stored_account = latest_store
        .accounts
        .iter_mut()
        .find(|stored| stored.id == account_id)
        .ok_or_else(|| "找不到要切换的账号".to_string())?;
    stored_account.auth_json = refreshed_auth;
    stored_account.updated_at = refreshed_at;
    stored_account.auth_refresh_blocked = false;
    stored_account.auth_refresh_error = None;
    stored_account.auth_refresh_next_at = auth::auth_refresh_next_at(&stored_account.auth_json);
    clear_stale_auth_usage_error(stored_account);
    profile_files::sync_account_profile_in_store_path(
        &store::account_store_path_for_app(app)?,
        stored_account,
    )?;
    account = stored_account.clone();
    store::save_store(app, &latest_store)?;
    Ok(account)
}

fn normalize_switch_refresh_error(raw_error: &str) -> String {
    let normalized = raw_error.to_ascii_lowercase();
    if normalized.contains("refresh_token_reused")
        || normalized
            .contains("your refresh token has already been used to generate a new access token")
    {
        return "当前账号的 refresh_token 已失效或已被轮换，请重新登录授权。".to_string();
    }
    if normalized.contains("please try signing in again")
        || normalized.contains("provided authentication token is expired")
        || normalized.contains("token is expired")
    {
        return "当前账号授权已过期，请重新登录授权。".to_string();
    }
    raw_error.to_string()
}

fn clear_stale_auth_usage_error(account: &mut models::StoredAccount) {
    if account
        .usage_error
        .as_deref()
        .is_some_and(is_auth_related_usage_error)
    {
        account.usage_error = None;
    }
}

fn is_auth_related_usage_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    message.contains("授权过期")
        || normalized.contains("401")
        || normalized.contains("unauthorized")
        || normalized.contains("invalid_token")
        || normalized.contains("token_revoked")
        || normalized.contains("invalidated oauth token")
        || normalized.contains("refresh_token")
        || normalized.contains("provided authentication token is expired")
        || normalized.contains("token is expired")
}

fn force_stop_running_codex(
    configured_codex_launch_path: Option<&str>,
    only_configured_install: bool,
) {
    #[cfg(target_os = "macos")]
    {
        if let Some(app_root) = cli::configured_codex_app_install_root(configured_codex_launch_path)
        {
            let app_name = app_root
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("Codex")
                .to_string();
            let _ = Command::new("pkill").args(["-9", "-x", &app_name]).status();
        }
    }

    #[cfg(target_os = "windows")]
    {
        cli::stop_running_windows_codex_processes(
            configured_codex_launch_path,
            only_configured_install,
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("pkill").args(["-9", "-x", "Codex"]).status();
    }

    #[cfg(not(target_os = "windows"))]
    thread::sleep(Duration::from_millis(220));
}

fn handle_window_close_to_background(window: &tauri::Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        if let Err(err) = window.hide() {
            log::warn!("隐藏窗口失败: {err}");
        }
        #[cfg(target_os = "macos")]
        {
            // 仅隐藏主窗口到后台时，同时隐藏 Dock 图标；
            // 应用仍继续运行，可从状态栏再次打开。
            if let Err(err) = window.app_handle().set_dock_visibility(false) {
                log::warn!("隐藏 Dock 图标失败: {err}");
            }
        }
    }
}

pub(crate) fn restore_main_window(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    if let Err(err) = app.set_dock_visibility(true) {
        log::warn!("恢复 Dock 图标失败: {err}");
    }

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn start_auth_keepalive_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(AUTH_KEEPALIVE_INITIAL_DELAY_SECS)).await;
        loop {
            let state = app.state::<AppState>();
            match account_service::refresh_all_usage_internal(&app, state.inner(), true).await {
                Ok(summaries) => {
                    let _ = tray::update_macos_tray_snapshot(&app, &summaries);
                }
                Err(error) => {
                    log::warn!("后台账号保活失败: {error}");
                }
            }
            tokio::time::sleep(Duration::from_secs(AUTH_KEEPALIVE_INTERVAL_SECS)).await;
        }
    });
}

async fn run_startup_maintenance_internal(
    app: &AppHandle,
    state: &AppState,
) -> Result<Vec<AccountSummary>, String> {
    {
        let _guard = state.store_lock.lock().await;
        if let Err(err) = store::sync_current_auth_account_on_startup(app) {
            log::warn!("启动后同步当前本机登录账号失败: {err}");
        }
        if let Err(err) = settings_service::reconcile_startup_settings(app) {
            log::warn!("启动后校准设置失败: {err}");
        }
    }

    account_service::list_accounts_internal(app, state).await
}

fn apply_main_window_chrome(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    if let Err(err) = window.set_decorations(false) {
        log::warn!("设置主窗口无系统标题栏失败: {err}");
    }
    if let Err(err) = window.set_shadow(true) {
        log::warn!("设置主窗口阴影失败: {err}");
    }
}

// ===== App Bootstrap =====

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            log::info!("检测到重复启动请求，切换到现有实例");
            restore_main_window(app);
        }))
        .manage(AppState::default())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .on_page_load(|webview, payload| {
            if webview.label() != "main" || payload.event() != PageLoadEvent::Finished {
                return;
            }
            let window = webview.window();
            if let Err(err) = window.show() {
                log::warn!("显示主窗口失败: {err}");
                return;
            }
            if let Err(err) = window.set_focus() {
                log::warn!("聚焦主窗口失败: {err}");
            }
        })
        .on_menu_event(tray::handle_status_bar_menu_event)
        .on_window_event(handle_window_close_to_background)
        .setup(|app| {
            utils::prepare_process_path();
            apply_main_window_chrome(app.handle());

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            tray::setup_system_tray(app.handle())?;
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = settings_service::sync_autostart_from_store(&app_handle) {
                    log::warn!("启动后同步开机启动状态失败: {err}");
                }
                match app_paths::codex_state_provider_backup_dir().and_then(|backup_dir| {
                    session_provider_sync::cleanup_codex_state_provider_backups(&backup_dir)
                }) {
                    Ok(removed) if removed > 0 => {
                        log::info!("启动后已清理 {removed} 个旧 Codex 线程 provider 备份");
                    }
                    Ok(_) => {}
                    Err(err) => {
                        log::warn!("启动后清理 Codex 线程 provider 备份失败: {err}");
                    }
                }
                match session_provider_sync::cleanup_legacy_codex_state_provider_backups() {
                    Ok(removed) if removed > 0 => {
                        log::info!("启动后已清理 {removed} 个旧版 Codex 线程 provider 备份");
                    }
                    Ok(_) => {}
                    Err(err) => {
                        log::warn!("启动后清理旧版 Codex 线程 provider 备份失败: {err}");
                    }
                }
            });
            start_auth_keepalive_loop(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            import_current_auth_account,
            run_startup_maintenance,
            create_api_account,
            probe_api_models,
            probe_api_account_models,
            import_auth_json_accounts,
            export_accounts_zip,
            delete_account,
            update_account_label,
            update_api_account,
            update_api_account_keys,
            probe_api_account_key,
            update_account_tags,
            refresh_all_usage,
            refresh_usage_for_account_keys,
            refresh_all_api_quota,
            refresh_api_quota_for_account_keys,
            get_codex_token_usage,
            get_app_settings,
            update_app_settings,
            enable_codex_multi_model_mode,
            reset_codex_multi_model_mode,
            test_notification_target,
            test_notification_provider,
            test_aggregate_notification,
            discover_telegram_chats,
            detect_codex_app,
            list_installed_editor_apps,
            is_opencode_desktop_app_installed,
            open_external_url,
            pick_codex_launch_path,
            prepare_oauth_login,
            complete_oauth_callback_login,
            cancel_oauth_login,
            set_model_router_mode,
            launch_current_codex_config,
            switch_account_and_launch,
            switch_hybrid_account_and_launch
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, event| match event {
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            restore_main_window(_app_handle);
        }
        _ => {}
    });
}
