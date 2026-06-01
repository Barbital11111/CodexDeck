use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle as ThreadJoinHandle;

use tokio::sync::Mutex;

use crate::auth::PendingOauthLogin;
use crate::hybrid_relay_proxy::HybridRelayProxyHandle;

pub(crate) struct OauthCallbackListenerHandle {
    pub(crate) shutdown_tx: Option<Sender<()>>,
    pub(crate) task: Option<ThreadJoinHandle<()>>,
}

/// 全局运行态：
/// - `store_lock` 保证账号存储读写的串行化。
/// - `pending_oauth_login` 维护当前 OAuth 授权会话。
/// - `oauth_listener` 维护本地 OAuth 回调监听线程。
/// - `hybrid_relay_proxy` 维护混合模式本地 Responses 代理。
pub(crate) struct AppState {
    pub(crate) store_lock: Arc<Mutex<()>>,
    pub(crate) auth_refresh_lock: Arc<Mutex<()>>,
    pub(crate) oauth_flow_lock: Arc<Mutex<()>>,
    pub(crate) pending_oauth_login: Mutex<Option<PendingOauthLogin>>,
    pub(crate) oauth_listener: Mutex<Option<OauthCallbackListenerHandle>>,
    pub(crate) hybrid_relay_proxy: Mutex<Option<HybridRelayProxyHandle>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            store_lock: Arc::new(Mutex::new(())),
            auth_refresh_lock: Arc::new(Mutex::new(())),
            oauth_flow_lock: Arc::new(Mutex::new(())),
            pending_oauth_login: Mutex::new(None),
            oauth_listener: Mutex::new(None),
            hybrid_relay_proxy: Mutex::new(None),
        }
    }
}
