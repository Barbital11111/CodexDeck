mod account_service;
mod app_paths;
mod auth;
mod cli;
mod codex_enhanced;
mod editor_apps;
mod hybrid_relay_proxy;
mod i18n;
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
use models::SwitchAccountResult;
use models::UpdateApiAccountInput;
use models::UpdateApiAccountKeyInput;
use state::AppState;
use state::OauthCallbackListenerHandle;
#[cfg(target_os = "windows")]
use utils::new_background_command;

const OAUTH_CALLBACK_FINISHED_EVENT: &str = "oauth-callback-finished";
const AUTH_KEEPALIVE_INTERVAL_SECS: u64 = 300;

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
    use std::net::TcpListener;

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
}

#[tauri::command]
async fn switch_account_and_launch(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    workspace_path: Option<String>,
    launch_codex: Option<bool>,
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
        let refreshed_auth = match auth::refresh_chatgpt_auth_tokens_serialized(
            &account.auth_json,
            &state.auth_refresh_lock,
        )
        .await
        {
            Ok(refreshed_auth) => refreshed_auth,
            Err(error) => {
                let normalized_error = normalize_switch_refresh_error(&error);
                let should_block_refresh = normalized_error
                    == "当前账号的 refresh_token 已失效或已被轮换，请重新登录授权。"
                    || normalized_error == "当前账号授权已过期，请重新登录授权。";

                if should_block_refresh {
                    let blocked_message = "授权过期，请重新登录授权。";
                    match app_paths::app_data_dir(&app) {
                        Ok(data_dir) => {
                            let store_path = store::account_store_path_from_data_dir(&data_dir);
                            if let Err(persist_error) =
                                store::update_account_group_refresh_state_in_path(
                                    &store_path,
                                    &account.account_key(),
                                    None,
                                    true,
                                    Some(blocked_message),
                                    utils::now_unix_seconds(),
                                    true,
                                )
                            {
                                log::warn!("切换失败后写回账号停刷状态失败: {persist_error}");
                            }
                        }
                        Err(path_error) => {
                            log::warn!("切换失败后获取应用数据目录失败: {path_error}");
                        }
                    }
                }

                return Err(format!("切换账号前刷新登录令牌失败: {normalized_error}"));
            }
        };

        account.auth_json = refreshed_auth.clone();

        let refreshed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("读取系统时间失败: {error}"))?
            .as_secs() as i64;
        let _guard = state.store_lock.lock().await;
        let mut latest_store = store::load_store(&app)?;
        let stored_account = latest_store
            .accounts
            .iter_mut()
            .find(|stored| stored.id == id)
            .ok_or_else(|| "找不到要切换的账号".to_string())?;
        stored_account.auth_json = refreshed_auth;
        stored_account.updated_at = refreshed_at;
        stored_account.auth_refresh_blocked = false;
        stored_account.auth_refresh_error = None;
        clear_stale_auth_usage_error(stored_account);
        profile_files::sync_account_profile_in_store_path(
            &store::account_store_path_from_data_dir(&app_paths::app_data_dir(&app)?),
            stored_account,
        )?;
        store::save_store(&app, &latest_store)?;
    }

    let should_sync_opencode = store.settings.sync_opencode_openai_auth;
    let should_restart_opencode_desktop =
        should_sync_opencode && store.settings.restart_opencode_desktop_on_switch;
    let should_restart_editors =
        restart_editors_on_switch.unwrap_or(store.settings.restart_editors_on_switch);
    let effective_restart_targets =
        restart_editor_targets.unwrap_or_else(|| store.settings.restart_editor_targets.clone());
    let configured_codex_launch_path = store.settings.codex_launch_path.clone();
    let should_use_api_enhanced_launch =
        matches!(account.source_kind, models::AccountSourceKind::Relay)
            && store.settings.api_enhanced_launch_enabled;
    // 向后兼容：旧前端未传参数时仍按“切换并启动”处理。
    let should_launch_codex = launch_codex.unwrap_or(true);
    {
        let _guard = state.store_lock.lock().await;
        let mut latest_store = store::load_store(&app)?;
        let stored_account = latest_store
            .accounts
            .iter_mut()
            .find(|stored| stored.id == id)
            .ok_or_else(|| "找不到要切换的账号".to_string())?;
        profile_files::sync_account_profile_in_store_path(
            &store::account_store_path_from_data_dir(&app_paths::app_data_dir(&app)?),
            stored_account,
        )?;
        profile_files::apply_account_profile(stored_account)?;
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
        settings_service::apply_active_codex_context_window_setting(
            latest_store.settings.codex_context_window_k,
        )?;
        latest_store.settings.active_account_id = Some(stored_account.id.clone());
        latest_store.settings.active_hybrid_profile = None;
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

    // 切换时强制结束旧实例，避免触发“是否退出”确认弹窗。
    force_stop_running_codex();

    if should_use_api_enhanced_launch {
        let result = codex_enhanced::launch_codex_with_enhancements(
            configured_codex_launch_path.as_deref(),
            workspace_path.as_deref(),
        )
        .await?;
        return Ok(SwitchAccountResult {
            account_id: account.account_id,
            launched_app_path: result.launched_app_path,
            used_fallback_cli: result.used_fallback_cli,
            opencode_synced,
            opencode_sync_error,
            opencode_desktop_restarted,
            opencode_desktop_restart_error,
            restarted_editor_apps,
            editor_restart_error,
        });
    }

    let mut app_launch_error = None;
    if let Some(path) = cli::find_configured_codex_app_path(configured_codex_launch_path.as_deref())
        .or_else(cli::find_codex_app_path)
    {
        match launch_codex_app(&path, workspace_path.as_deref()) {
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

    #[cfg(target_os = "windows")]
    if cli::has_windows_store_codex_app() {
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

    let mut cmd = cli::new_codex_command(configured_codex_launch_path.as_deref())?;
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
async fn switch_hybrid_account_and_launch(
    app: AppHandle,
    state: State<'_, AppState>,
    chatgpt_account_id: String,
    relay_account_id: String,
    workspace_path: Option<String>,
    launch_codex: Option<bool>,
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
        let refreshed_auth = match auth::refresh_chatgpt_auth_tokens_serialized(
            &chatgpt_account.auth_json,
            &state.auth_refresh_lock,
        )
        .await
        {
            Ok(refreshed_auth) => refreshed_auth,
            Err(error) => {
                let normalized_error = normalize_switch_refresh_error(&error);
                let should_block_refresh = normalized_error
                    == "当前账号的 refresh_token 已失效或已被轮换，请重新登录授权。"
                    || normalized_error == "当前账号授权已过期，请重新登录授权。";

                if should_block_refresh {
                    let blocked_message = "授权过期，请重新登录授权。";
                    match app_paths::app_data_dir(&app) {
                        Ok(data_dir) => {
                            let store_path = store::account_store_path_from_data_dir(&data_dir);
                            if let Err(persist_error) =
                                store::update_account_group_refresh_state_in_path(
                                    &store_path,
                                    &chatgpt_account.account_key(),
                                    None,
                                    true,
                                    Some(blocked_message),
                                    utils::now_unix_seconds(),
                                    false,
                                )
                            {
                                log::warn!(
                                    "混合模式切换失败后写回账号停刷状态失败: {persist_error}"
                                );
                            }
                        }
                        Err(path_error) => {
                            log::warn!("混合模式切换失败后获取应用数据目录失败: {path_error}");
                        }
                    }
                }

                return Err(format!(
                    "切换混合模式前刷新登录令牌失败: {normalized_error}"
                ));
            }
        };

        let refreshed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("读取系统时间失败: {error}"))?
            .as_secs() as i64;
        let _guard = state.store_lock.lock().await;
        let mut latest_store = store::load_store(&app)?;
        let stored_account = latest_store
            .accounts
            .iter_mut()
            .find(|stored| stored.id == chatgpt_account_id)
            .ok_or_else(|| "找不到要用于混合模式的 ChatGPT 官方账号".to_string())?;
        stored_account.auth_json = refreshed_auth;
        stored_account.updated_at = refreshed_at;
        stored_account.auth_refresh_blocked = false;
        stored_account.auth_refresh_error = None;
        clear_stale_auth_usage_error(stored_account);
        profile_files::sync_account_profile_in_store_path(
            &store::account_store_path_from_data_dir(&app_paths::app_data_dir(&app)?),
            stored_account,
        )?;
        store::save_store(&app, &latest_store)?;
    }

    let should_restart_editors =
        restart_editors_on_switch.unwrap_or(store.settings.restart_editors_on_switch);
    let effective_restart_targets =
        restart_editor_targets.unwrap_or_else(|| store.settings.restart_editor_targets.clone());
    let configured_codex_launch_path = store.settings.codex_launch_path.clone();
    let should_launch_codex = launch_codex.unwrap_or(true);
    let relay_account;
    {
        let _guard = state.store_lock.lock().await;
        let mut latest_store = store::load_store(&app)?;
        let store_path = store::account_store_path_from_data_dir(&app_paths::app_data_dir(&app)?);
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
        let hybrid_proxy_base_url =
            hybrid_relay_proxy::ensure_hybrid_relay_proxy_for_account(state.inner(), &stored_relay)
                .await?;
        profile_files::apply_hybrid_account_profile_with_provider_base_url(
            &stored_chatgpt,
            &stored_relay,
            &hybrid_proxy_base_url,
        )?;

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
        settings_service::apply_active_codex_context_window_setting(
            latest_store.settings.codex_context_window_k,
        )?;
        latest_store.settings.active_account_id = Some(stored_relay.id.clone());
        latest_store.settings.active_hybrid_profile = Some(models::ActiveHybridProfile {
            chatgpt_account_id: stored_chatgpt.id.clone(),
            relay_account_id: stored_relay.id.clone(),
        });
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

    force_stop_running_codex();

    let mut app_launch_error = None;
    if let Some(path) = cli::find_configured_codex_app_path(configured_codex_launch_path.as_deref())
        .or_else(cli::find_codex_app_path)
    {
        match launch_codex_app(&path, workspace_path.as_deref()) {
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

    #[cfg(target_os = "windows")]
    if cli::has_windows_store_codex_app() {
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

fn launch_codex_app(path: &std::path::Path, workspace_path: Option<&str>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
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
            let _ = workspace_path;
            return cli::launch_windows_store_codex();
        }

        let mut cmd = new_background_command(path);
        if let Some(workspace) = workspace_path {
            cmd.arg(workspace);
        }
        cmd.spawn()
            .map_err(|e| format!("启动 Codex 应用失败: {e}"))?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut cmd = Command::new(path);
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
        Err("当前平台暂不支持直接启动 Codex 应用".to_string())
    }
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

fn force_stop_running_codex() {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("pkill").args(["-9", "-x", "Codex"]).status();
        let _ = Command::new("pkill")
            .args(["-9", "-x", "Codex Desktop"])
            .status();
    }

    #[cfg(target_os = "windows")]
    {
        let _ = new_background_command("taskkill")
            .args(["/F", "/IM", "Codex.exe", "/T"])
            .status();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("pkill").args(["-9", "-x", "Codex"]).status();
    }

    // 等待进程树收敛，避免新实例拉起时与旧实例短暂重叠。
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

async fn restore_hybrid_relay_profile_on_startup(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let active_profile = {
        let _guard = state.store_lock.lock().await;
        let store = store::load_store(app)?;
        let active_account_id = store.settings.active_account_id.clone();
        let Some(active_relay_id) = active_account_id else {
            return Ok(());
        };
        let Some(active_relay_account) = store
            .accounts
            .iter()
            .find(|account| {
                account.id == active_relay_id
                    && matches!(account.source_kind, models::AccountSourceKind::Relay)
            })
            .cloned()
        else {
            return Ok(());
        };

        if let Some(hybrid) = store.settings.active_hybrid_profile.as_ref() {
            if hybrid.relay_account_id == active_relay_account.id {
                let chatgpt_account = store
                    .accounts
                    .iter()
                    .find(|account| {
                        account.id == hybrid.chatgpt_account_id
                            && matches!(account.source_kind, models::AccountSourceKind::Chatgpt)
                    })
                    .cloned()
                    .ok_or_else(|| "启动时找不到混合模式的 ChatGPT 官方账号".to_string())?;
                let relay_account = store
                    .accounts
                    .iter()
                    .find(|account| {
                        account.id == hybrid.relay_account_id
                            && matches!(account.source_kind, models::AccountSourceKind::Relay)
                    })
                    .cloned()
                    .ok_or_else(|| "启动时找不到混合模式的 API 条目".to_string())?;
                Some((Some(chatgpt_account), relay_account))
            } else {
                Some((None, active_relay_account))
            }
        } else {
            Some((None, active_relay_account))
        }
    };

    let Some((chatgpt_account, relay_account)) = active_profile else {
        return Ok(());
    };
    if let Some(chatgpt_account) = chatgpt_account {
        let local_base_url = hybrid_relay_proxy::ensure_hybrid_relay_proxy_for_account(
            state.inner(),
            &relay_account,
        )
        .await?;
        profile_files::apply_hybrid_account_profile_with_provider_base_url(
            &chatgpt_account,
            &relay_account,
            &local_base_url,
        )
    } else {
        profile_files::apply_account_profile(&relay_account)
    }
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

            if let Err(err) = settings_service::sync_autostart_from_store(app.handle()) {
                log::warn!("启动时同步开机启动状态失败: {err}");
            }
            // 启动阶段先同步当前本机登录账号，再初始化状态栏，保证首次展示即一致。
            store::sync_current_auth_account_on_startup(app.handle())?;
            let account_store_path =
                store::account_store_path_from_data_dir(&app_paths::app_data_dir(app.handle())?);
            match store::sync_relay_account_profiles_on_startup_in_path(&account_store_path, false)
            {
                Ok(count) if count > 0 => {
                    log::info!("启动时已同步 {count} 个 API profile");
                }
                Ok(_) => {}
                Err(err) => {
                    log::warn!("启动时同步 API profile 失败: {err}");
                }
            }
            let setup_app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = restore_hybrid_relay_profile_on_startup(&setup_app_handle).await {
                    log::warn!("启动时恢复混合模式本地代理失败: {err}");
                }
            });
            tray::setup_system_tray(app.handle())?;
            match app_paths::codex_state_provider_backup_dir().and_then(|backup_dir| {
                session_provider_sync::cleanup_codex_state_provider_backups(&backup_dir)
            }) {
                Ok(removed) if removed > 0 => {
                    log::info!("启动时已清理 {removed} 个旧 Codex 线程 provider 备份");
                }
                Ok(_) => {}
                Err(err) => {
                    log::warn!("启动时清理 Codex 线程 provider 备份失败: {err}");
                }
            }
            start_auth_keepalive_loop(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            import_current_auth_account,
            create_api_account,
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
