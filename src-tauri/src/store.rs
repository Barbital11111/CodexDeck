#[cfg(feature = "desktop")]
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use uuid::Uuid;

#[cfg(feature = "desktop")]
use tauri::AppHandle;

#[cfg(feature = "desktop")]
use crate::app_paths;
use crate::auth::account_variant_key;
use crate::auth::auth_refresh_next_at;
use crate::auth::current_auth_account_key;
use crate::auth::extract_auth;
#[cfg(feature = "desktop")]
use crate::auth::read_current_codex_auth_optional;
use crate::auth::write_active_codex_auth;
use crate::models::dedupe_account_variants;
use crate::models::AccountSourceKind;
use crate::models::AccountsStore;
#[cfg(test)]
use crate::models::ProxyHealthStatus;
use crate::models::StoredAccount;
#[cfg(feature = "desktop")]
use crate::profile_files;
use crate::utils::now_unix_seconds;
#[cfg(test)]
use crate::utils::redact_sensitive_text;
use crate::utils::set_private_permissions;
use crate::utils::short_account;

const LAST_GOOD_BACKUP_FILE_NAME: &str = "accounts.json.last-good.json";
const PREVIOUS_GOOD_BACKUP_FILE_NAME: &str = "accounts.json.prev-good.json";
const EMPTY_STORE_MARKER_FILE_NAME: &str = "accounts.empty.json";

#[derive(Clone)]
struct RecoveryCandidate {
    source: String,
    modified_at: i64,
    store: AccountsStore,
}

#[cfg(feature = "desktop")]
#[derive(Clone)]
struct StoreMigrationCandidate {
    dir: PathBuf,
    store_path: PathBuf,
    account_count: usize,
    modified_at: i64,
    store: AccountsStore,
}

#[cfg(feature = "desktop")]
pub(crate) fn load_store(app: &AppHandle) -> Result<AccountsStore, String> {
    load_store_from_path(&account_store_path(app)?)
}

#[cfg(feature = "desktop")]
pub(crate) fn load_store_read_only(app: &AppHandle) -> Result<AccountsStore, String> {
    let path = account_store_path_read_only(app)?;
    load_store_from_path_read_only(&path)
}

#[cfg(feature = "desktop")]
pub(crate) fn save_store(app: &AppHandle, store: &AccountsStore) -> Result<(), String> {
    save_store_to_path(&account_store_path(app)?, store)
}

#[cfg(feature = "desktop")]
pub(crate) fn account_store_path_for_app(app: &AppHandle) -> Result<PathBuf, String> {
    account_store_path(app)
}

/// 启动时自动同步当前登录账号：
/// 若本机已有 `~/.codex/auth.json` 且相同“账号 + 套餐态”不在列表中，则自动写入存储。
#[cfg(feature = "desktop")]
pub(crate) fn sync_current_auth_account_on_startup(app: &AppHandle) -> Result<(), String> {
    sync_current_auth_account_on_startup_in_path(&account_store_path(app)?)
}

pub(crate) fn load_store_from_path(path: &Path) -> Result<AccountsStore, String> {
    if !path.exists() {
        return Ok(AccountsStore::default());
    }

    let raw = fs::read_to_string(path)
        .map_err(|e| format!("读取账号存储文件失败 {}: {e}", path.display()))?;

    match serde_json::from_str::<AccountsStore>(&raw) {
        Ok(store) => Ok(normalize_loaded_store(path, store)),
        Err(primary_err) => {
            if let Some((recovered, recovered_sources)) =
                recover_store_from_available_sources(path, &raw)
            {
                log::warn!(
                    "账号存储文件格式无效，已从可恢复数据重建 {}: {}; 来源: {}",
                    path.display(),
                    primary_err,
                    recovered_sources.join(", ")
                );
                if let Err(backup_err) = backup_corrupted_store_file(path, &raw) {
                    log::warn!(
                        "重建前备份损坏账号存储文件失败 {}: {}",
                        path.display(),
                        backup_err
                    );
                }
                if let Err(repair_err) = write_store_file(path, &recovered) {
                    return Err(format!(
                        "账号存储文件恢复后重写失败 {}: {}; {}",
                        path.display(),
                        primary_err,
                        repair_err
                    ));
                }
                return Ok(normalize_loaded_store(path, recovered));
            }

            if let Err(backup_err) = backup_corrupted_store_file(path, &raw) {
                log::warn!(
                    "账号存储文件损坏，备份失败 {}: {}",
                    path.display(),
                    backup_err
                );
            }

            Err(format!(
                "账号存储文件格式无效且没有可用备份，已保留原文件并停止加载 {}: {}",
                path.display(),
                primary_err
            ))
        }
    }
}

pub(crate) fn load_store_from_path_read_only(path: &Path) -> Result<AccountsStore, String> {
    if !path.exists() {
        return Ok(AccountsStore::default());
    }

    let raw = fs::read_to_string(path)
        .map_err(|e| format!("读取账号存储文件失败 {}: {e}", path.display()))?;
    serde_json::from_str::<AccountsStore>(&raw)
        .map(normalize_loaded_store_in_memory)
        .map_err(|error| format!("账号存储文件格式无效 {}: {error}", path.display()))
}

pub(crate) fn save_store_to_path(path: &Path, store: &AccountsStore) -> Result<(), String> {
    let mut store = store.clone();
    store.sync_proxy_upstream_snapshot();
    write_store_file(path, &store)
}

#[cfg(feature = "desktop")]
pub(crate) fn sync_current_auth_account_on_startup_in_path(path: &Path) -> Result<(), String> {
    let auth_json = match read_current_codex_auth_optional()? {
        Some(value) => value,
        None => return Ok(()),
    };

    let extracted = match extract_auth(&auth_json) {
        Ok(value) => value,
        Err(err) => {
            log::warn!("跳过启动自动导入当前账号: {err}");
            return Ok(());
        }
    };

    let mut store = load_store_from_path_read_only(path)?;
    let extracted_variant_key = account_variant_key(
        &extracted.principal_id,
        &extracted.account_id,
        extracted.plan_type.as_deref(),
    );
    let already_exists = store
        .accounts
        .iter()
        .any(|account| account.variant_key() == extracted_variant_key);
    if already_exists {
        return Ok(());
    }

    let now = now_unix_seconds();
    let label = extracted
        .email
        .clone()
        .unwrap_or_else(|| format!("Codex {}", short_account(&extracted.account_id)));

    let auth_refresh_next_at = auth_refresh_next_at(&auth_json);
    let stored = StoredAccount {
        id: Uuid::new_v4().to_string(),
        label,
        source_kind: Default::default(),
        principal_id: Some(extracted.principal_id),
        email: extracted.email,
        account_id: extracted.account_id,
        plan_type: extracted.plan_type,
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
        usage: None,
        usage_error: None,
        auth_refresh_blocked: false,
        auth_refresh_error: None,
        auth_refresh_next_at,
    };
    let mut stored = stored;
    let _ = profile_files::sync_account_profile_in_store_path(path, &mut stored);
    store.accounts.push(stored);
    save_store_to_path(path, &store)?;
    Ok(())
}

#[cfg(feature = "desktop")]
pub(crate) fn sync_relay_account_profiles_on_startup_in_path(
    path: &Path,
    apply_active_profile: bool,
) -> Result<usize, String> {
    let mut store = load_store_from_path(path)?;
    let active_account_id = store.settings.active_account_id.clone();
    let mut changed = false;
    let mut synced_count = 0usize;
    let mut active_relay_account = None;

    for account in store
        .accounts
        .iter_mut()
        .filter(|account| matches!(account.source_kind, AccountSourceKind::Relay))
    {
        let previous_auth_path = account.profile_auth_path.clone();
        let previous_config_path = account.profile_config_path.clone();
        let previous_auth_ready = account.profile_auth_ready;
        let previous_config_ready = account.profile_config_ready;
        let previous_integrity_error = account.profile_integrity_error.clone();

        match profile_files::sync_account_profile_in_store_path(path, account) {
            Ok(()) => {
                synced_count += 1;
                if active_account_id.as_deref() == Some(account.id.as_str()) {
                    active_relay_account = Some(account.clone());
                }
            }
            Err(error) => {
                log::warn!("启动时同步 API profile 失败 {}: {}", account.label, error);
                account.profile_integrity_error = Some(error);
            }
        }

        if account.profile_auth_path != previous_auth_path
            || account.profile_config_path != previous_config_path
            || account.profile_auth_ready != previous_auth_ready
            || account.profile_config_ready != previous_config_ready
            || account.profile_integrity_error != previous_integrity_error
        {
            changed = true;
        }
    }

    if changed {
        save_store_to_path(path, &store)?;
    }

    if apply_active_profile {
        if let Some(account) = active_relay_account {
            if let Some((chatgpt_account, relay_account)) =
                current_hybrid_pair_for_relay(&store, &account.id)
            {
                profile_files::apply_hybrid_account_profile(&chatgpt_account, &relay_account)?;
            } else {
                profile_files::apply_account_profile(&account)?;
            }
        }
    }
    profile_files::apply_model_instructions_fix_setting(
        store.settings.codex_model_instructions_fix_enabled,
    )?;

    Ok(synced_count)
}

#[cfg(feature = "desktop")]
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

pub(crate) fn update_account_group_refresh_state_in_path(
    path: &Path,
    account_key: &str,
    auth_json: Option<&serde_json::Value>,
    auth_refresh_blocked: bool,
    auth_refresh_error: Option<&str>,
    updated_at: i64,
    sync_current_auth: bool,
) -> Result<bool, String> {
    let mut store = load_store_from_path(path)?;
    let mut changed = false;

    for account in store
        .accounts
        .iter_mut()
        .filter(|account| account.account_key() == account_key)
    {
        if let Some(value) = auth_json {
            account.auth_json = value.clone();
        }
        account.auth_refresh_blocked = auth_refresh_blocked;
        account.auth_refresh_error = auth_refresh_error.map(ToString::to_string);
        account.auth_refresh_next_at = if auth_refresh_blocked {
            None
        } else {
            auth_refresh_next_at(&account.auth_json)
        };
        account.updated_at = updated_at;
        #[cfg(feature = "desktop")]
        if auth_json.is_some() && !auth_refresh_blocked {
            profile_files::sync_account_profile_in_store_path(path, account)?;
        }
        changed = true;
    }

    if !changed {
        return Ok(false);
    }

    save_store_to_path(path, &store)?;

    if sync_current_auth
        && !auth_refresh_blocked
        && auth_json.is_some()
        && current_auth_account_key().as_deref() == Some(account_key)
    {
        write_active_codex_auth(auth_json.expect("checked is_some above"))?;
        #[cfg(feature = "desktop")]
        reapply_active_hybrid_profile_if_needed(&store)?;
    }

    Ok(true)
}

#[cfg(feature = "desktop")]
fn reapply_active_hybrid_profile_if_needed(store: &AccountsStore) -> Result<(), String> {
    let Some(hybrid) = store.settings.active_hybrid_profile.as_ref() else {
        return Ok(());
    };
    let Some((chatgpt_account, relay_account)) =
        current_hybrid_pair_for_relay(store, &hybrid.relay_account_id)
    else {
        return Ok(());
    };
    if current_auth_account_key().as_deref() == Some(chatgpt_account.account_key().as_str()) {
        profile_files::apply_hybrid_account_profile(&chatgpt_account, &relay_account)?;
        profile_files::apply_model_instructions_fix_setting(
            store.settings.codex_model_instructions_fix_enabled,
        )?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn update_relay_key_health_in_path(
    path: &Path,
    account_id: &str,
    key_id: &str,
    health_status: ProxyHealthStatus,
    last_error: Option<&str>,
    cooldown_until: Option<i64>,
    updated_at: i64,
) -> Result<bool, String> {
    let mut store = load_store_from_path(path)?;
    let Some(account) = store
        .accounts
        .iter_mut()
        .find(|account| account.id == account_id)
    else {
        return Ok(false);
    };
    let Some(key) = account.api_keys.iter_mut().find(|key| key.id == key_id) else {
        return Ok(false);
    };

    key.health_status = health_status;
    key.last_error = last_error.map(redact_sensitive_text);
    key.cooldown_until = cooldown_until;
    key.failure_count = if matches!(health_status, ProxyHealthStatus::Healthy) {
        0
    } else {
        key.failure_count.saturating_add(1)
    };
    key.updated_at = Some(updated_at);
    account.updated_at = updated_at;
    save_store_to_path(path, &store)?;
    Ok(true)
}

#[cfg(test)]
pub(crate) fn record_relay_key_success_in_path(
    path: &Path,
    account_id: &str,
    key_id: &str,
    updated_at: i64,
) -> Result<bool, String> {
    let mut store = load_store_from_path(path)?;
    let Some(account) = store
        .accounts
        .iter_mut()
        .find(|account| account.id == account_id)
    else {
        return Ok(false);
    };
    let Some(key) = account.api_keys.iter_mut().find(|key| key.id == key_id) else {
        return Ok(false);
    };

    key.health_status = ProxyHealthStatus::Healthy;
    key.last_error = None;
    key.cooldown_until = None;
    key.failure_count = 0;
    key.last_used_at = Some(updated_at);
    key.updated_at = Some(updated_at);
    account.updated_at = updated_at;
    save_store_to_path(path, &store)?;
    Ok(true)
}

#[cfg(feature = "desktop")]
fn account_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app_paths::app_data_dir(app)?;
    ensure_account_store_available_in_dir(&dir)?;
    Ok(account_store_path_from_data_dir(&dir))
}

#[cfg(feature = "desktop")]
fn account_store_path_read_only(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app_paths::app_data_dir(app)?;
    Ok(account_store_path_from_data_dir(&dir))
}

pub(crate) fn account_store_path_from_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("accounts.json")
}

#[cfg(feature = "desktop")]
fn ensure_account_store_available_in_dir(data_dir: &Path) -> Result<(), String> {
    if app_paths::is_dev_runtime() {
        return Ok(());
    }

    let store_path = account_store_path_from_data_dir(data_dir);
    let current_state = inspect_store_file(&store_path);
    match current_state {
        StoreFileState::Valid { account_count: 0 } => {
            if empty_store_marker_path(&store_path).exists() {
                return Ok(());
            }
            if recover_current_store_from_shadow_backups(&store_path, "empty")? {
                return Ok(());
            }
            if let Some(candidate) = select_best_migration_candidate(data_dir)? {
                migrate_account_store_from_candidate(data_dir, &store_path, &candidate)?;
            }
        }
        StoreFileState::Valid { .. } => return Ok(()),
        StoreFileState::Missing => {
            if recover_current_store_from_shadow_backups(&store_path, "missing")? {
                return Ok(());
            }
            if let Some(candidate) = select_best_migration_candidate(data_dir)? {
                migrate_account_store_from_candidate(data_dir, &store_path, &candidate)?;
            }
        }
        StoreFileState::Invalid => {
            if recover_current_store_from_shadow_backups(&store_path, "corrupt")? {
                return Ok(());
            }
            if let Some(candidate) = select_best_migration_candidate(data_dir)? {
                backup_existing_store_before_migration(&store_path, "corrupt")?;
                migrate_account_store_from_candidate(data_dir, &store_path, &candidate)?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "desktop")]
fn recover_current_store_from_shadow_backups(path: &Path, reason: &str) -> Result<bool, String> {
    let raw = if path.exists() {
        fs::read_to_string(path)
            .map_err(|e| format!("读取账号存储文件失败 {}: {e}", path.display()))?
    } else {
        String::new()
    };

    let Some((recovered, recovered_sources)) = recover_store_from_available_sources(path, &raw)
    else {
        return Ok(false);
    };

    if path.exists() {
        backup_existing_store_before_migration(path, reason)?;
    }
    write_store_file(path, &recovered)?;
    log::warn!(
        "已从账号存储备份恢复 {}，来源: {}",
        path.display(),
        recovered_sources.join(", ")
    );
    Ok(true)
}

#[cfg(feature = "desktop")]
enum StoreFileState {
    Missing,
    Valid { account_count: usize },
    Invalid,
}

#[cfg(feature = "desktop")]
fn inspect_store_file(path: &Path) -> StoreFileState {
    if !path.exists() {
        return StoreFileState::Missing;
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return StoreFileState::Invalid;
    };
    match serde_json::from_str::<AccountsStore>(&raw) {
        Ok(store) => StoreFileState::Valid {
            account_count: store.accounts.len(),
        },
        Err(_) => StoreFileState::Invalid,
    }
}

#[cfg(feature = "desktop")]
fn select_best_migration_candidate(
    data_dir: &Path,
) -> Result<Option<StoreMigrationCandidate>, String> {
    let mut candidates = Vec::new();
    for candidate_dir in app_paths::account_data_dir_migration_candidates(data_dir) {
        if candidate_dir == data_dir {
            continue;
        }
        let store_path = account_store_path_from_data_dir(&candidate_dir);
        let Ok(raw) = fs::read_to_string(&store_path) else {
            continue;
        };
        let parsed_candidates = collect_recovery_candidates(&store_path, &raw);
        let Some(best_store) = parsed_candidates
            .iter()
            .max_by_key(|candidate| {
                (
                    candidate.store.accounts.len(),
                    candidate.store.version,
                    candidate.modified_at,
                )
            })
            .filter(|candidate| !candidate.store.accounts.is_empty())
        else {
            continue;
        };
        candidates.push(StoreMigrationCandidate {
            dir: candidate_dir,
            store_path,
            account_count: best_store.store.accounts.len(),
            modified_at: best_store.modified_at,
            store: best_store.store.clone(),
        });
    }

    candidates.sort_by_key(|candidate| {
        (
            candidate.account_count,
            candidate.modified_at,
            candidate.store_path.to_string_lossy().to_string(),
        )
    });
    candidates.reverse();

    let Some(best) = candidates.first().cloned() else {
        return Ok(None);
    };
    if candidates
        .get(1)
        .is_some_and(|second| second.account_count == best.account_count)
    {
        let paths = candidates
            .iter()
            .filter(|candidate| candidate.account_count == best.account_count)
            .map(|candidate| candidate.store_path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "发现多个账号数相同的可恢复账号存储，无法自动选择，请先手动确认: {paths}"
        ));
    }

    Ok(Some(best))
}

#[cfg(feature = "desktop")]
fn migrate_account_store_from_candidate(
    data_dir: &Path,
    store_path: &Path,
    candidate: &StoreMigrationCandidate,
) -> Result<(), String> {
    if let StoreFileState::Valid { account_count: 0 } = inspect_store_file(store_path) {
        backup_existing_store_before_migration(store_path, "empty")?;
    }

    fs::create_dir_all(data_dir)
        .map_err(|e| format!("创建账号数据目录失败 {}: {e}", data_dir.display()))?;
    write_store_file(store_path, &candidate.store)?;
    copy_optional_migration_file(
        &candidate.dir.join(LAST_GOOD_BACKUP_FILE_NAME),
        &data_dir.join(LAST_GOOD_BACKUP_FILE_NAME),
    )?;
    copy_optional_migration_file(
        &candidate.dir.join(PREVIOUS_GOOD_BACKUP_FILE_NAME),
        &data_dir.join(PREVIOUS_GOOD_BACKUP_FILE_NAME),
    )?;
    copy_migration_dir_contents_if_present(
        &candidate.dir.join("profiles"),
        &data_dir.join("profiles"),
    )?;
    log::warn!(
        "已从已有 CodexDeck 数据目录迁移账号存储 {} -> {}，账号数 {}",
        candidate.store_path.display(),
        store_path.display(),
        candidate.account_count
    );
    Ok(())
}

#[cfg(feature = "desktop")]
fn backup_existing_store_before_migration(path: &Path, reason: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析存储目录 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("创建存储目录失败 {}: {e}", parent.display()))?;
    let backup_path = parent.join(format!("accounts.{reason}-{}.json", now_unix_seconds()));
    fs::copy(path, &backup_path).map_err(|e| {
        format!(
            "备份现有账号存储失败 {} -> {}: {e}",
            path.display(),
            backup_path.display()
        )
    })?;
    set_private_permissions(&backup_path);
    Ok(())
}

#[cfg(feature = "desktop")]
fn copy_optional_migration_file(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_file() || destination.exists() {
        return Ok(());
    }
    copy_migration_file(source, destination)
}

#[cfg(feature = "desktop")]
fn copy_migration_file(source: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建迁移目录失败 {}: {e}", parent.display()))?;
    }
    fs::copy(source, destination).map_err(|e| {
        format!(
            "迁移账号存储失败 {} -> {}: {e}",
            source.display(),
            destination.display()
        )
    })?;
    set_private_permissions(destination);
    Ok(())
}

#[cfg(feature = "desktop")]
fn copy_migration_dir_contents_if_present(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Ok(());
    }
    copy_dir_contents_missing_only(source, destination).map_err(|e| {
        format!(
            "迁移账号 profile 目录失败 {} -> {}: {e}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(feature = "desktop")]
fn copy_dir_contents_missing_only(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_contents_missing_only(&source_path, &destination_path)?;
        } else if source_path.is_file() && !destination_path.exists() {
            fs::copy(&source_path, &destination_path)?;
            set_private_permissions(&destination_path);
        }
    }
    Ok(())
}

fn write_store_file(path: &Path, store: &AccountsStore) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析存储目录 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("创建存储目录失败 {}: {e}", parent.display()))?;

    let serialized =
        serde_json::to_string_pretty(store).map_err(|e| format!("序列化账号存储失败: {e}"))?;
    write_file_atomically(path, serialized.as_bytes())?;
    sync_empty_store_marker(path, store)?;
    if let Err(err) = write_store_shadow_backups(path, serialized.as_bytes()) {
        log::warn!("写入账号存储滚动备份失败 {}: {}", path.display(), err);
    }
    Ok(())
}

fn sync_empty_store_marker(path: &Path, store: &AccountsStore) -> Result<(), String> {
    let marker_path = empty_store_marker_path(path);
    if store.accounts.is_empty() {
        let parent = marker_path
            .parent()
            .ok_or_else(|| format!("无法解析空账号标记目录 {}", marker_path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建空账号标记目录失败 {}: {e}", parent.display()))?;
        fs::write(&marker_path, b"{\"empty\":true}\n")
            .map_err(|e| format!("写入空账号标记失败 {}: {e}", marker_path.display()))?;
        set_private_permissions(&marker_path);
    } else if marker_path.exists() {
        fs::remove_file(&marker_path)
            .map_err(|e| format!("移除空账号标记失败 {}: {e}", marker_path.display()))?;
    }
    Ok(())
}

fn empty_store_marker_path(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(EMPTY_STORE_MARKER_FILE_NAME)
}

fn normalize_loaded_store(path: &Path, mut store: AccountsStore) -> AccountsStore {
    let mut changed = false;

    if normalize_loaded_store_core(&mut store) {
        changed = true;
    }

    #[cfg(feature = "desktop")]
    for account in &mut store.accounts {
        if profile_files::ensure_profile_metadata(path, account) {
            changed = true;
        }
    }

    #[cfg(feature = "desktop")]
    {
        let valid_profile_ids = store
            .accounts
            .iter()
            .map(|account| account.id.clone())
            .collect::<HashSet<_>>();
        match profile_files::cleanup_orphan_profiles_in_store_path(path, &valid_profile_ids) {
            Ok(removed_count) if removed_count > 0 => {
                log::warn!(
                    "已清理 {} 个孤儿账号 profile 目录 {}",
                    removed_count,
                    path.display()
                );
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!(
                    "清理孤儿账号 profile 目录失败 {}: {}",
                    path.display(),
                    error
                );
            }
        }
    }

    if changed {
        if let Err(repair_err) = write_store_file(path, &store) {
            log::warn!(
                "修正账号存储后重写文件失败 {}: {}",
                path.display(),
                repair_err
            );
        }
    }

    store
}

fn normalize_loaded_store_in_memory(mut store: AccountsStore) -> AccountsStore {
    normalize_loaded_store_core(&mut store);
    store
}

fn normalize_loaded_store_core(store: &mut AccountsStore) -> bool {
    let mut changed = false;

    for account in &mut store.accounts {
        let expected_refresh_next_at = match account.source_kind {
            AccountSourceKind::Chatgpt if !account.auth_refresh_blocked => {
                auth_refresh_next_at(&account.auth_json)
            }
            AccountSourceKind::Relay => None,
            _ => None,
        };
        if account.auth_refresh_next_at != expected_refresh_next_at {
            account.auth_refresh_next_at = expected_refresh_next_at;
            changed = true;
        }

        if account
            .principal_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            account.principal_id = Some(account.principal_key());
            changed = true;
        }
    }

    if store
        .settings
        .active_hybrid_profile
        .as_ref()
        .is_some_and(|hybrid| {
            !store.accounts.iter().any(|account| {
                account.id == hybrid.chatgpt_account_id
                    && matches!(account.source_kind, AccountSourceKind::Chatgpt)
            }) || !store.accounts.iter().any(|account| {
                account.id == hybrid.relay_account_id
                    && matches!(account.source_kind, AccountSourceKind::Relay)
            })
        })
    {
        store.settings.active_hybrid_profile = None;
        changed = true;
    }

    if dedupe_account_variants(&mut store.accounts) {
        log::warn!("账号存储存在重复账号变体，已自动合并");
        changed = true;
    }

    if store.sync_proxy_upstream_snapshot() {
        changed = true;
    }

    changed
}

fn write_file_atomically(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析存储目录 {}", path.display()))?;
    let temp_path = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("accounts.json"),
        Uuid::new_v4()
    ));

    let write_result = (|| -> Result<(), String> {
        let mut temp_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|e| format!("创建临时存储文件失败 {}: {e}", temp_path.display()))?;
        temp_file
            .write_all(contents)
            .map_err(|e| format!("写入临时存储文件失败 {}: {e}", temp_path.display()))?;
        temp_file
            .sync_all()
            .map_err(|e| format!("刷新临时存储文件失败 {}: {e}", temp_path.display()))?;
        drop(temp_file);
        set_private_permissions(&temp_path);

        #[cfg(target_family = "unix")]
        {
            fs::rename(&temp_path, path).map_err(|e| {
                format!(
                    "替换账号存储文件失败 {} -> {}: {e}",
                    temp_path.display(),
                    path.display()
                )
            })?;

            let parent_dir = fs::File::open(parent)
                .map_err(|e| format!("打开存储目录失败 {}: {e}", parent.display()))?;
            parent_dir
                .sync_all()
                .map_err(|e| format!("刷新存储目录失败 {}: {e}", parent.display()))?;
        }

        #[cfg(not(target_family = "unix"))]
        {
            if path.exists() {
                fs::remove_file(path)
                    .map_err(|e| format!("移除旧账号存储文件失败 {}: {e}", path.display()))?;
            }
            fs::rename(&temp_path, path).map_err(|e| {
                format!(
                    "替换账号存储文件失败 {} -> {}: {e}",
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

fn write_store_shadow_backups(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析存储目录 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("创建存储目录失败 {}: {e}", parent.display()))?;

    let latest_backup = parent.join(LAST_GOOD_BACKUP_FILE_NAME);
    let previous_backup = parent.join(PREVIOUS_GOOD_BACKUP_FILE_NAME);

    if latest_backup.exists() {
        let latest_contents = fs::read(&latest_backup)
            .map_err(|e| format!("读取最新备份失败 {}: {e}", latest_backup.display()))?;
        fs::write(&previous_backup, latest_contents)
            .map_err(|e| format!("写入上一个备份失败 {}: {e}", previous_backup.display()))?;
        set_private_permissions(&previous_backup);
    }

    fs::write(&latest_backup, contents)
        .map_err(|e| format!("写入最新备份失败 {}: {e}", latest_backup.display()))?;
    set_private_permissions(&latest_backup);
    Ok(())
}

fn recover_store_from_available_sources(
    path: &Path,
    raw: &str,
) -> Option<(AccountsStore, Vec<String>)> {
    let candidates = collect_recovery_candidates(path, raw);
    if candidates.is_empty() {
        return None;
    }

    let best = candidates.iter().max_by_key(|candidate| {
        (
            usize::from(!candidate.store.accounts.is_empty()),
            candidate.store.accounts.len(),
            candidate.modified_at,
        )
    })?;

    let mut merged_accounts = Vec::new();
    let mut recovered_sources = Vec::new();
    for candidate in &candidates {
        if !candidate.store.accounts.is_empty() {
            recovered_sources.push(candidate.source.clone());
        }
        merged_accounts.extend(candidate.store.accounts.clone());
    }
    dedupe_account_variants(&mut merged_accounts);

    if merged_accounts.is_empty() {
        return None;
    }

    let mut recovered = best.store.clone();
    recovered.accounts = merged_accounts;
    Some((recovered, recovered_sources))
}

fn collect_recovery_candidates(path: &Path, raw: &str) -> Vec<RecoveryCandidate> {
    let mut candidates = parse_store_candidates_from_text(
        raw,
        format!("{} (current damaged file)", path.display()),
        file_modified_at(path),
    );

    let Some(parent) = path.parent() else {
        return candidates;
    };

    let Ok(entries) = fs::read_dir(parent) else {
        return candidates;
    };

    for entry in entries.flatten() {
        let candidate_path = entry.path();
        if candidate_path == path || !candidate_path.is_file() {
            continue;
        }
        if !is_store_backup_candidate(&candidate_path) {
            continue;
        }

        let Ok(candidate_raw) = fs::read_to_string(&candidate_path) else {
            continue;
        };
        candidates.extend(parse_store_candidates_from_text(
            &candidate_raw,
            candidate_path.display().to_string(),
            file_modified_at(&candidate_path),
        ));
    }

    candidates
}

fn parse_store_candidates_from_text(
    raw: &str,
    source: String,
    modified_at: i64,
) -> Vec<RecoveryCandidate> {
    let mut candidates = Vec::new();

    if let Ok(store) = serde_json::from_str::<AccountsStore>(raw) {
        candidates.push(RecoveryCandidate {
            source,
            modified_at,
            store,
        });
        return candidates;
    }

    let mut stream = serde_json::Deserializer::from_str(raw).into_iter::<AccountsStore>();
    let mut recovered_index = 0usize;
    while let Some(result) = stream.next() {
        match result {
            Ok(store) => {
                recovered_index += 1;
                candidates.push(RecoveryCandidate {
                    source: format!("{source}#{recovered_index}"),
                    modified_at,
                    store,
                });
            }
            Err(_) => break,
        }
    }

    if candidates.is_empty() {
        if let Ok(accounts) = serde_json::from_str::<Vec<StoredAccount>>(raw) {
            candidates.push(RecoveryCandidate {
                source,
                modified_at,
                store: AccountsStore {
                    version: 1,
                    accounts,
                    proxy_upstreams: Vec::new(),
                    proxy_route_bindings: Vec::new(),
                    settings: Default::default(),
                },
            });
        }
    }

    candidates
}

fn is_store_backup_candidate(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    name.starts_with("accounts.")
        || name.starts_with("accounts.json.")
        || name.starts_with(".accounts.json.tmp-")
}

fn file_modified_at(path: &Path) -> i64 {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| {
            time.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_secs() as i64)
        })
        .unwrap_or_default()
}

fn backup_corrupted_store_file(path: &Path, raw: &str) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析存储目录 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("创建存储目录失败 {}: {e}", parent.display()))?;

    let backup_path = parent.join(format!("accounts.corrupt-{}.json", now_unix_seconds()));
    fs::write(&backup_path, raw)
        .map_err(|e| format!("写入损坏备份文件失败 {}: {e}", backup_path.display()))?;
    set_private_permissions(&backup_path);
    Ok(backup_path)
}

#[cfg(test)]
mod tests {
    use super::ensure_account_store_available_in_dir;
    use super::empty_store_marker_path;
    use super::load_store_from_path;
    use super::record_relay_key_success_in_path;
    use super::save_store_to_path;
    use super::sync_relay_account_profiles_on_startup_in_path;
    use super::update_account_group_refresh_state_in_path;
    use super::update_relay_key_health_in_path;
    use super::LAST_GOOD_BACKUP_FILE_NAME;
    use super::PREVIOUS_GOOD_BACKUP_FILE_NAME;
    use crate::models::AccountSourceKind;
    use crate::models::AccountsStore;
    use crate::models::ProxyHealthStatus;
    use crate::models::StoredAccount;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use serde_json::json;
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use uuid::Uuid;

    struct EnvVarGuard {
        name: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set_path(name: &'static str, value: &std::path::Path) -> Self {
            let original = std::env::var_os(name);
            std::env::set_var(name, value);
            Self { name, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.original.as_ref() {
                std::env::set_var(self.name, value);
            } else {
                std::env::remove_var(self.name);
            }
        }
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("codex-tools-store-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn jwt_with_claims(claims: serde_json::Value) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("encode claims"));
        format!("header.{payload}.signature")
    }

    fn chatgpt_auth(access: &str, refresh: &str, exp: i64) -> serde_json::Value {
        json!({
            "auth_mode": "chatgpt",
            "last_refresh": "2026-02-01T00:00:00Z",
            "tokens": {
                "access_token": jwt_with_claims(json!({ "exp": exp })),
                "id_token": jwt_with_claims(json!({
                    "email": "hybrid@example.com",
                    "exp": exp,
                    "https://api.openai.com/auth": {
                        "chatgpt_account_id": "workspace-1",
                        "chatgpt_plan_type": "pro"
                    }
                })),
                "refresh_token": refresh,
                "account_id": "workspace-1"
            },
            "marker": access
        })
    }

    fn sample_store(label: &str, account_id: &str, updated_at: i64) -> AccountsStore {
        AccountsStore {
            version: 1,
            accounts: vec![StoredAccount {
                id: format!("id-{label}"),
                label: label.to_string(),
                source_kind: Default::default(),
                principal_id: Some(format!("{label}@example.com")),
                email: Some(format!("{label}@example.com")),
                account_id: account_id.to_string(),
                plan_type: Some("team".to_string()),
                auth_json: json!({ "kind": label }),
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
                added_at: updated_at - 1,
                updated_at,
                usage: None,
                usage_error: None,
                auth_refresh_blocked: false,
                auth_refresh_error: None,
                auth_refresh_next_at: None,
            }],
            proxy_upstreams: Vec::new(),
            proxy_route_bindings: Vec::new(),
            settings: Default::default(),
        }
    }

    #[test]
    fn load_store_recovers_from_backup_candidates_instead_of_resetting() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");
        fs::write(&store_path, "{not valid json").expect("write damaged store");

        let backup_path = dir.join("accounts.json.manual-backup-1");
        let backup_store = sample_store("restored", "workspace-1", 10);
        fs::write(
            &backup_path,
            serde_json::to_string_pretty(&backup_store).expect("serialize backup"),
        )
        .expect("write backup");

        let loaded = load_store_from_path(&store_path).expect("recover store");

        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.accounts[0].label, "restored");

        let persisted: AccountsStore =
            serde_json::from_str(&fs::read_to_string(&store_path).expect("read repaired store"))
                .expect("parse repaired store");
        assert_eq!(persisted.accounts.len(), 1);
        assert_eq!(persisted.accounts[0].label, "restored");
    }

    #[test]
    fn load_store_returns_error_when_damaged_store_has_no_recovery_source() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");
        fs::write(&store_path, "{not valid json").expect("write damaged store");

        let err = load_store_from_path(&store_path).expect_err("damaged store should fail");

        assert!(err.contains("账号存储文件格式无效且没有可用备份"));
        assert!(fs::read_to_string(&store_path)
            .expect("read original damaged store")
            .contains("{not valid json"));
    }

    #[test]
    fn account_store_migration_copies_best_legacy_store_into_current_dir() {
        let _guard = env_lock().lock().expect("lock env");
        let root = temp_dir();
        let current = root.join("empty-current");
        let old_small = root.join("io.github.barbital11111.codexdeck");
        let old_large = root.join("com.carry.codex-tools");
        let _appdata_guard = EnvVarGuard::set_path("APPDATA", &root);
        fs::create_dir_all(&current).expect("create current");
        fs::create_dir_all(&old_small).expect("create small legacy");
        fs::create_dir_all(&old_large).expect("create large legacy");

        let small_store = sample_store("small", "workspace-small", 10);
        let mut large_store = sample_store("large-a", "workspace-large-a", 20);
        large_store
            .accounts
            .push(sample_store("large-b", "workspace-large-b", 21).accounts.remove(0));
        fs::write(
            old_small.join("accounts.json"),
            serde_json::to_string_pretty(&small_store).expect("serialize small"),
        )
        .expect("write small");
        fs::write(
            old_large.join("accounts.json"),
            serde_json::to_string_pretty(&large_store).expect("serialize large"),
        )
        .expect("write large");

        ensure_account_store_available_in_dir(&current).expect("migrate store");

        let migrated = load_store_from_path(&current.join("accounts.json")).expect("load migrated");
        assert_eq!(migrated.accounts.len(), 2);
        assert_eq!(migrated.accounts[0].label, "large-a");
        let latest_backup: AccountsStore = serde_json::from_str(
            &fs::read_to_string(current.join(LAST_GOOD_BACKUP_FILE_NAME))
                .expect("read migrated latest backup"),
        )
        .expect("parse migrated latest backup");
        assert_eq!(latest_backup.accounts.len(), 2);
        assert!(old_large.join("accounts.json").is_file());
    }

    #[test]
    fn account_store_migration_keeps_existing_non_empty_current_store() {
        let _guard = env_lock().lock().expect("lock env");
        let root = temp_dir();
        let current = root.join("com.carry.codex-tools");
        let legacy = root.join("io.github.barbital11111.codexdeck");
        let _appdata_guard = EnvVarGuard::set_path("APPDATA", &root);
        fs::create_dir_all(&current).expect("create current");
        fs::create_dir_all(&legacy).expect("create legacy");
        let current_store = sample_store("current", "workspace-current", 30);
        let legacy_store = sample_store("legacy", "workspace-legacy", 10);
        save_store_to_path(&current.join("accounts.json"), &current_store).expect("save current");
        save_store_to_path(&legacy.join("accounts.json"), &legacy_store).expect("save legacy");

        ensure_account_store_available_in_dir(&current).expect("inspect store");

        let loaded = load_store_from_path(&current.join("accounts.json")).expect("load current");
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.accounts[0].label, "current");
    }

    #[test]
    fn account_store_migration_can_restore_from_legacy_backup() {
        let _guard = env_lock().lock().expect("lock env");
        let root = temp_dir();
        let current = root.join("fresh-current");
        let legacy = root.join("com.carry.codex-tools");
        let _appdata_guard = EnvVarGuard::set_path("APPDATA", &root);
        fs::create_dir_all(&current).expect("create current");
        fs::create_dir_all(&legacy).expect("create legacy");
        fs::write(legacy.join("accounts.json"), "{broken").expect("write broken legacy");
        let backup_store = sample_store("backup", "workspace-backup", 40);
        fs::write(
            legacy.join(LAST_GOOD_BACKUP_FILE_NAME),
            serde_json::to_string_pretty(&backup_store).expect("serialize backup"),
        )
        .expect("write backup");

        ensure_account_store_available_in_dir(&current).expect("migrate backup");

        let loaded = load_store_from_path(&current.join("accounts.json")).expect("load migrated");
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.accounts[0].label, "backup");
    }

    #[test]
    fn account_store_recovery_prefers_current_shadow_backup_for_empty_store() {
        let _guard = env_lock().lock().expect("lock env");
        let root = temp_dir();
        let current = root.join("com.carry.codex-tools");
        let _appdata_guard = EnvVarGuard::set_path("APPDATA", &root);
        fs::create_dir_all(&current).expect("create current");
        fs::write(
            current.join("accounts.json"),
            serde_json::to_string_pretty(&AccountsStore::default()).expect("serialize empty"),
        )
        .expect("write legacy empty current");
        let backup_store = sample_store("shadow", "workspace-shadow", 50);
        fs::write(
            current.join(LAST_GOOD_BACKUP_FILE_NAME),
            serde_json::to_string_pretty(&backup_store).expect("serialize backup"),
        )
        .expect("write backup");

        ensure_account_store_available_in_dir(&current).expect("recover shadow");

        let loaded = load_store_from_path(&current.join("accounts.json")).expect("load recovered");
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.accounts[0].label, "shadow");
    }

    #[test]
    fn account_store_recovery_respects_intentional_empty_marker() {
        let _guard = env_lock().lock().expect("lock env");
        let root = temp_dir();
        let current = root.join("com.carry.codex-tools");
        let legacy = root.join("io.github.barbital11111.codexdeck");
        let _appdata_guard = EnvVarGuard::set_path("APPDATA", &root);
        fs::create_dir_all(&current).expect("create current");
        fs::create_dir_all(&legacy).expect("create legacy");
        save_store_to_path(&current.join("accounts.json"), &AccountsStore::default())
            .expect("save empty current");
        assert!(empty_store_marker_path(&current.join("accounts.json")).is_file());

        let backup_store = sample_store("shadow", "workspace-shadow", 50);
        fs::write(
            current.join(LAST_GOOD_BACKUP_FILE_NAME),
            serde_json::to_string_pretty(&backup_store).expect("serialize backup"),
        )
        .expect("write current backup");
        fs::write(
            legacy.join("accounts.json"),
            serde_json::to_string_pretty(&backup_store).expect("serialize legacy"),
        )
        .expect("write legacy");

        ensure_account_store_available_in_dir(&current).expect("inspect empty current");

        let loaded = load_store_from_path(&current.join("accounts.json")).expect("load current");
        assert!(loaded.accounts.is_empty());
    }

    #[test]
    fn account_store_recovery_prefers_current_shadow_backup_for_missing_store() {
        let _guard = env_lock().lock().expect("lock env");
        let root = temp_dir();
        let current = root.join("com.carry.codex-tools");
        let _appdata_guard = EnvVarGuard::set_path("APPDATA", &root);
        fs::create_dir_all(&current).expect("create current");
        let backup_store = sample_store("missing-shadow", "workspace-missing-shadow", 60);
        fs::write(
            current.join(LAST_GOOD_BACKUP_FILE_NAME),
            serde_json::to_string_pretty(&backup_store).expect("serialize backup"),
        )
        .expect("write backup");

        ensure_account_store_available_in_dir(&current).expect("recover shadow");

        let loaded = load_store_from_path(&current.join("accounts.json")).expect("load recovered");
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.accounts[0].label, "missing-shadow");
    }

    #[test]
    fn save_store_writes_rolling_good_backups() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");

        let first = sample_store("first", "workspace-1", 10);
        save_store_to_path(&store_path, &first).expect("save first");

        let latest_backup = dir.join(LAST_GOOD_BACKUP_FILE_NAME);
        assert!(latest_backup.exists());

        let second = sample_store("second", "workspace-2", 20);
        save_store_to_path(&store_path, &second).expect("save second");

        let previous_backup = dir.join(PREVIOUS_GOOD_BACKUP_FILE_NAME);
        assert!(previous_backup.exists());

        let previous: AccountsStore =
            serde_json::from_str(&fs::read_to_string(&previous_backup).expect("read previous"))
                .expect("parse previous");
        let latest: AccountsStore =
            serde_json::from_str(&fs::read_to_string(&latest_backup).expect("read latest"))
                .expect("parse latest");

        assert_eq!(previous.accounts[0].label, "first");
        assert_eq!(latest.accounts[0].label, "second");
    }

    #[test]
    fn load_store_backfills_missing_principal_id() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");
        let legacy_store = AccountsStore {
            version: 1,
            accounts: vec![StoredAccount {
                id: "legacy".to_string(),
                label: "legacy".to_string(),
                source_kind: Default::default(),
                principal_id: None,
                email: Some("legacy@example.com".to_string()),
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
            }],
            proxy_upstreams: Vec::new(),
            proxy_route_bindings: Vec::new(),
            settings: Default::default(),
        };
        fs::write(
            &store_path,
            serde_json::to_string_pretty(&legacy_store).expect("serialize legacy store"),
        )
        .expect("write legacy store");

        let loaded = load_store_from_path(&store_path).expect("load legacy store");

        assert_eq!(
            loaded.accounts[0].principal_id.as_deref(),
            Some("legacy@example.com")
        );
    }

    #[test]
    fn update_refresh_state_syncs_profile_auth_and_config() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");
        let mut store = sample_store("fresh", "workspace-1", 10);
        store.accounts[0].auth_json = json!({
            "auth_mode": "chatgpt",
            "last_refresh": "2026-01-01T00:00:00Z",
            "tokens": {
                "access_token": "old-access",
                "id_token": "old-id",
                "refresh_token": "old-refresh",
                "account_id": "workspace-1"
            }
        });
        fs::write(
            &store_path,
            serde_json::to_string_pretty(&store).expect("serialize store"),
        )
        .expect("write store");

        let account_key = store.accounts[0].account_key();
        let refreshed_auth = json!({
            "auth_mode": "chatgpt",
            "last_refresh": "2026-02-01T00:00:00Z",
            "tokens": {
                "access_token": "new-access",
                "id_token": "new-id",
                "refresh_token": "new-refresh",
                "account_id": "workspace-1"
            }
        });

        let changed = update_account_group_refresh_state_in_path(
            &store_path,
            &account_key,
            Some(&refreshed_auth),
            false,
            None,
            20,
            false,
        )
        .expect("update refresh state");

        let loaded = load_store_from_path(&store_path).expect("load store");
        let account = &loaded.accounts[0];
        let auth_path = account
            .profile_auth_path
            .as_ref()
            .expect("profile auth path");
        let config_path = account
            .profile_config_path
            .as_ref()
            .expect("profile config path");
        let profile_auth: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(auth_path).expect("read profile auth"))
                .expect("parse profile auth");
        let profile_config = fs::read_to_string(config_path).expect("read profile config");

        assert!(changed);
        assert_eq!(account.auth_json, refreshed_auth);
        assert!(account.profile_auth_ready);
        assert!(account.profile_config_ready);
        assert_eq!(account.profile_integrity_error, None);
        assert_eq!(profile_auth, refreshed_auth);
        assert!(profile_config.contains("cli_auth_credentials_store = \"file\""));
    }

    #[test]
    fn update_refresh_state_preserves_active_hybrid_profile() {
        let _guard = env_lock().lock().expect("lock env");
        let dir = temp_dir();
        let codex_dir = dir.join("codex-home");
        fs::create_dir_all(&codex_dir).expect("create codex dir");
        let _codex_dir_guard = EnvVarGuard::set_path("CODEX_SWITCH_DEV_CODEX_DIR", &codex_dir);
        let store_path = dir.join("accounts.json");
        let mut initial_auth = chatgpt_auth("old-access", "old-refresh", 4_100_000_000);
        initial_auth["OPENAI_API_KEY"] = json!("placeholder-stale-chatgpt-key");
        let relay = StoredAccount {
            id: "relay-1".to_string(),
            label: "Relay".to_string(),
            source_kind: AccountSourceKind::Relay,
            principal_id: Some("relay:relay-1".to_string()),
            email: None,
            account_id: "relay:relay-1".to_string(),
            plan_type: Some("api".to_string()),
            auth_json: json!({}),
            api_base_url: Some("https://relay.example.com/v1".to_string()),
            api_key: Some("sk-hybrid-secret".to_string()),
            api_keys: Vec::new(),
            proxy_priority: None,
            proxy_weight: None,
            proxy_key_selection_mode: None,
            proxy_endpoints: Vec::new(),
            model_name: Some("gpt-5.5".to_string()),
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
        };
        let mut store = sample_store("chatgpt", "workspace-1", 10);
        store.accounts[0].id = "chatgpt-1".to_string();
        store.accounts[0].source_kind = AccountSourceKind::Chatgpt;
        store.accounts[0].principal_id = Some("hybrid@example.com".to_string());
        store.accounts[0].email = Some("hybrid@example.com".to_string());
        store.accounts[0].auth_json = initial_auth.clone();
        store.accounts[0].auth_refresh_next_at = Some(4_100_000_000 - 600);
        store.accounts.push(relay);
        store.settings.active_account_id = Some("relay-1".to_string());
        store.settings.active_hybrid_profile = Some(crate::models::ActiveHybridProfile {
            chatgpt_account_id: "chatgpt-1".to_string(),
            relay_account_id: "relay-1".to_string(),
        });
        save_store_to_path(&store_path, &store).expect("save store");
        super::profile_files::apply_hybrid_account_profile(&store.accounts[0], &store.accounts[1])
            .expect("apply initial hybrid");

        let account_key = store.accounts[0].account_key();
        let mut refreshed_auth = chatgpt_auth("new-access", "new-refresh", 4_200_000_000);
        refreshed_auth["OPENAI_API_KEY"] = json!("placeholder-stale-refreshed-key");
        let changed = update_account_group_refresh_state_in_path(
            &store_path,
            &account_key,
            Some(&refreshed_auth),
            false,
            None,
            20,
            true,
        )
        .expect("update refresh state");

        let active_auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(codex_dir.join("auth.json")).expect("read active auth"),
        )
        .expect("parse active auth");
        let active_config =
            fs::read_to_string(codex_dir.join("config.toml")).expect("read active config");
        let loaded = load_store_from_path(&store_path).expect("load store");

        assert!(changed);
        assert!(active_auth.get("OPENAI_API_KEY").is_none());
        assert_eq!(
            active_auth.get("auth_mode"),
            refreshed_auth.get("auth_mode")
        );
        assert_eq!(active_auth.get("tokens"), refreshed_auth.get("tokens"));
        assert_eq!(active_auth.get("marker"), refreshed_auth.get("marker"));
        assert_eq!(
            loaded.accounts[0].auth_refresh_next_at,
            Some(4_200_000_000 - 600)
        );
        assert!(active_config.contains(r#"model_provider = "codexdeck_api""#));
        assert!(active_config.contains(r#"requires_openai_auth = true"#));
        assert!(active_config.contains(r#"experimental_bearer_token = "sk-hybrid-secret""#));
        assert!(!active_config.contains(r#"openai_base_url = "https://relay.example.com/v1""#));
    }

    #[test]
    fn load_store_backfills_proxy_upstream_snapshot_without_secrets() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");
        fs::write(
            &store_path,
            serde_json::to_string_pretty(&json!({
                "version": 1,
                "accounts": [{
                    "id": "relay-1",
                    "label": "Relay",
                    "sourceKind": "relay",
                    "principalId": "relay:relay-1",
                    "email": null,
                    "accountId": "relay-account",
                    "planType": "api",
                    "authJson": {},
                    "apiBaseUrl": "https://api.example.com/v1/",
                    "apiKey": "test-api-key-secret",
                    "modelName": "gpt-5.4",
                    "balanceText": null,
                    "providerId": null,
                    "providerName": null,
                    "tags": ["API"],
                    "profileAuthPath": null,
                    "profileConfigPath": null,
                    "profileAuthReady": false,
                    "profileConfigReady": false,
                    "profileIntegrityError": null,
                    "profileLastValidatedAt": null,
                    "profileLastValidationError": null,
                    "addedAt": 1,
                    "updatedAt": 2,
                    "usage": null,
                    "usageError": null,
                    "authRefreshBlocked": false,
                    "authRefreshError": null
                }],
                "settings": {}
            }))
            .expect("serialize legacy store"),
        )
        .expect("write legacy store");

        let loaded = load_store_from_path(&store_path).expect("load store");

        assert_eq!(loaded.proxy_upstreams.len(), 1);
        assert_eq!(
            loaded.proxy_upstreams[0].channels[0].base_url.as_deref(),
            Some("https://api.example.com/v1")
        );
        assert_eq!(loaded.proxy_upstreams[0].channels[0].keys.len(), 1);
        assert_eq!(loaded.proxy_upstreams[0].channels[0].keys[0].secret, None);

        let persisted: AccountsStore =
            serde_json::from_str(&fs::read_to_string(&store_path).expect("read store"))
                .expect("parse store");
        assert_eq!(persisted.proxy_upstreams.len(), 1);
        assert_eq!(
            persisted.proxy_upstreams[0].channels[0].keys[0].secret,
            None
        );
    }

    #[test]
    fn startup_sync_rebuilds_relay_profiles_with_disabled_websockets() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");
        let store = AccountsStore {
            version: 1,
            accounts: vec![StoredAccount {
                id: "relay-1".to_string(),
                label: "Relay".to_string(),
                source_kind: AccountSourceKind::Relay,
                principal_id: Some("relay:relay-1".to_string()),
                email: None,
                account_id: "relay:relay-1".to_string(),
                plan_type: Some("api".to_string()),
                auth_json: json!({}),
                api_base_url: Some("https://api.example.com/v1".to_string()),
                api_key: Some("test-api-key-secret".to_string()),
                api_keys: Vec::new(),
                proxy_priority: None,
                proxy_weight: None,
                proxy_key_selection_mode: None,
                proxy_endpoints: Vec::new(),
                model_name: Some("gpt-5.5".to_string()),
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
            }],
            proxy_upstreams: Vec::new(),
            proxy_route_bindings: Vec::new(),
            settings: Default::default(),
        };
        fs::write(
            &store_path,
            serde_json::to_string_pretty(&store).expect("serialize store"),
        )
        .expect("write store");

        let synced = sync_relay_account_profiles_on_startup_in_path(&store_path, false)
            .expect("sync relay profiles");

        let loaded = load_store_from_path(&store_path).expect("load store");
        let account = &loaded.accounts[0];
        let config_path = account
            .profile_config_path
            .as_ref()
            .expect("profile config path");
        let config = fs::read_to_string(config_path).expect("read profile config");

        assert_eq!(synced, 1);
        assert!(account.profile_auth_ready);
        assert!(account.profile_config_ready);
        assert_eq!(account.profile_integrity_error, None);
        assert!(config.contains("[features]"));
        assert!(config.contains("responses_websockets = false"));
        assert!(config.contains("responses_websockets_v2 = false"));
    }

    #[test]
    fn update_relay_key_health_persists_key_cooldown_state() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");
        fs::write(
            &store_path,
            serde_json::to_string_pretty(&json!({
                "version": 2,
                "accounts": [{
                    "id": "relay-1",
                    "label": "Relay",
                    "sourceKind": "relay",
                    "principalId": "relay:relay-1",
                    "email": null,
                    "accountId": "relay-account",
                    "planType": "api",
                    "authJson": {},
                    "apiBaseUrl": "https://api.example.com/v1/",
                    "apiKey": "test-api-key-secret",
                    "apiKeys": [{
                        "id": "key-a",
                        "label": "A",
                        "secret": "test-api-key-secret",
                        "enabled": true,
                        "priority": 100,
                        "weight": 100,
                        "healthStatus": "healthy"
                    }],
                    "modelName": "gpt-5.4",
                    "balanceText": null,
                    "providerId": null,
                    "providerName": null,
                    "tags": [],
                    "profileAuthPath": null,
                    "profileConfigPath": null,
                    "profileAuthReady": false,
                    "profileConfigReady": false,
                    "profileIntegrityError": null,
                    "profileLastValidatedAt": null,
                    "profileLastValidationError": null,
                    "addedAt": 1,
                    "updatedAt": 2,
                    "usage": null,
                    "usageError": null,
                    "authRefreshBlocked": false,
                    "authRefreshError": null
                }],
                "settings": {}
            }))
            .expect("serialize store"),
        )
        .expect("write store");

        let changed = update_relay_key_health_in_path(
            &store_path,
            "relay-1",
            "key-a",
            ProxyHealthStatus::CoolingDown,
            Some("429"),
            Some(123),
            99,
        )
        .expect("update key health");
        let loaded = load_store_from_path(&store_path).expect("load store");

        assert!(changed);
        assert_eq!(
            loaded.accounts[0].api_keys[0].health_status,
            ProxyHealthStatus::CoolingDown
        );
        assert_eq!(
            loaded.accounts[0].api_keys[0].last_error.as_deref(),
            Some("429")
        );
        assert_eq!(loaded.accounts[0].api_keys[0].cooldown_until, Some(123));
    }

    #[test]
    fn load_store_restores_expired_quota_key_to_healthy() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");
        fs::write(
            &store_path,
            serde_json::to_string_pretty(&json!({
                "version": 1,
                "accounts": [{
                    "id": "relay-1",
                    "label": "Relay",
                    "sourceKind": "relay",
                    "principalId": "relay:relay-1",
                    "email": null,
                    "accountId": "relay-account",
                    "planType": "api",
                    "authJson": {},
                    "apiBaseUrl": "https://api.example.com/v1/",
                    "apiKey": "test-api-key-secret",
                    "apiKeys": [{
                        "id": "key-a",
                        "label": "A",
                        "secret": "sk-a",
                        "enabled": true,
                        "priority": 100,
                        "weight": 100,
                        "healthStatus": "quota_exhausted",
                        "lastError": "quota",
                        "cooldownUntil": 1,
                        "failureCount": 1,
                        "lastUsedAt": null,
                        "updatedAt": 2
                    }],
                    "modelName": "gpt-5.4",
                    "balanceText": null,
                    "providerId": null,
                    "providerName": null,
                    "tags": [],
                    "profileAuthPath": null,
                    "profileConfigPath": null,
                    "profileAuthReady": false,
                    "profileConfigReady": false,
                    "profileIntegrityError": null,
                    "profileLastValidatedAt": null,
                    "profileLastValidationError": null,
                    "addedAt": 1,
                    "updatedAt": 2,
                    "usage": null,
                    "usageError": null,
                    "authRefreshBlocked": false,
                    "authRefreshError": null
                }],
                "settings": {}
            }))
            .expect("serialize store"),
        )
        .expect("write store");

        let loaded = load_store_from_path(&store_path).expect("load store");

        assert_eq!(
            loaded.accounts[0].resolved_relay_proxy_keys()[0].health_status,
            ProxyHealthStatus::Healthy
        );
    }

    #[test]
    fn load_store_removes_orphan_profile_directories() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");
        let store = sample_store("keep", "workspace-1", 10);
        fs::write(
            &store_path,
            serde_json::to_string_pretty(&store).expect("serialize store"),
        )
        .expect("write store");

        let kept_profile = dir.join("profiles").join("id-keep");
        let orphan_profile = dir.join("profiles").join("orphan");
        fs::create_dir_all(&kept_profile).expect("create kept profile");
        fs::create_dir_all(&orphan_profile).expect("create orphan profile");

        let loaded = load_store_from_path(&store_path).expect("load store");

        assert_eq!(loaded.accounts.len(), 1);
        assert!(kept_profile.exists());
        assert!(!orphan_profile.exists());
    }

    #[test]
    fn record_relay_key_success_clears_error_and_cooldown() {
        let dir = temp_dir();
        let store_path = dir.join("accounts.json");
        fs::write(
            &store_path,
            serde_json::to_string_pretty(&json!({
                "version": 2,
                "accounts": [{
                    "id": "relay-1",
                    "label": "Relay",
                    "sourceKind": "relay",
                    "principalId": "relay:relay-1",
                    "email": null,
                    "accountId": "relay-account",
                    "planType": "api",
                    "authJson": {},
                    "apiBaseUrl": "https://api.example.com/v1/",
                    "apiKey": "test-api-key-secret",
                    "apiKeys": [{
                        "id": "key-a",
                        "label": "A",
                        "secret": "sk-a",
                        "enabled": true,
                        "priority": 100,
                        "weight": 100,
                        "healthStatus": "degraded",
                        "lastError": "temporary",
                        "cooldownUntil": 123,
                        "failureCount": 2
                    }],
                    "modelName": "gpt-5.4",
                    "tags": [],
                    "addedAt": 1,
                    "updatedAt": 2
                }]
            }))
            .expect("serialize store"),
        )
        .expect("write store");

        let changed = record_relay_key_success_in_path(&store_path, "relay-1", "key-a", 456)
            .expect("record key success");
        let loaded = load_store_from_path(&store_path).expect("load store");
        let key = &loaded.accounts[0].api_keys[0];

        assert!(changed);
        assert_eq!(key.health_status, ProxyHealthStatus::Healthy);
        assert_eq!(key.last_error, None);
        assert_eq!(key.cooldown_until, None);
        assert_eq!(key.failure_count, 0);
        assert_eq!(key.last_used_at, Some(456));
    }
}
