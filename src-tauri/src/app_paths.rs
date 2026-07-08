use std::path::PathBuf;
use std::process::Command;

#[cfg(feature = "desktop")]
use std::path::Path;
#[cfg(feature = "desktop")]
use tauri::AppHandle;
#[cfg(feature = "desktop")]
use tauri::Manager;

const DEV_APP_DATA_DIR_ENV: &str = "CODEX_SWITCH_DEV_DATA_DIR";
const LEGACY_DEV_APP_DATA_DIR_ENV: &str = "CODEX_TOOLS_DEV_DATA_DIR";
const DEV_CODEX_DIR_ENV: &str = "CODEX_SWITCH_DEV_CODEX_DIR";
const LEGACY_DEV_CODEX_DIR_ENV: &str = "CODEX_TOOLS_DEV_CODEX_DIR";
#[cfg(feature = "desktop")]
const LEGACY_APP_DATA_DIR_NAME: &str = "com.carry.codex-tools";
#[cfg(feature = "desktop")]
const LEGACY_CODEXDECK_DATA_DIR_NAME: &str = "io.github.barbital11111.codexdeck";

fn env_path(name: &str) -> Option<PathBuf> {
    let value = std::env::var(name).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

pub(crate) fn is_dev_runtime() -> bool {
    cfg!(debug_assertions)
        && (env_path(DEV_APP_DATA_DIR_ENV).is_some()
            || env_path(LEGACY_DEV_APP_DATA_DIR_ENV).is_some())
}

#[cfg(feature = "desktop")]
pub(crate) fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        if let Some(path) = env_path(DEV_APP_DATA_DIR_ENV) {
            return Ok(path);
        }
        if let Some(path) = env_path(LEGACY_DEV_APP_DATA_DIR_ENV) {
            return Ok(path);
        }
    }

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法获取应用数据目录: {error}"))?;
    Ok(data_dir)
}

pub(crate) fn codex_dir() -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        if let Some(path) = env_path(DEV_CODEX_DIR_ENV) {
            return Ok(path);
        }
        if let Some(path) = env_path(LEGACY_DEV_CODEX_DIR_ENV) {
            return Ok(path);
        }
    }

    let home = dirs::home_dir().ok_or_else(|| "无法读取 HOME 目录".to_string())?;
    Ok(home.join(".codex"))
}

pub(crate) fn codex_auth_path() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join("auth.json"))
}

pub(crate) fn codex_config_path() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join("config.toml"))
}

pub(crate) fn apply_codex_home_env(command: &mut Command) -> Result<(), String> {
    command.env("CODEX_HOME", codex_dir()?);
    Ok(())
}

pub(crate) fn apply_codex_home_process_env() -> Result<(), String> {
    std::env::set_var("CODEX_HOME", codex_dir()?);
    Ok(())
}

pub(crate) fn codex_state_provider_backup_dir() -> Result<PathBuf, String> {
    let executable_path =
        std::env::current_exe().map_err(|error| format!("无法获取 CodexDeck 安装路径: {error}"))?;
    let install_dir = executable_path
        .parent()
        .ok_or_else(|| format!("无法解析 CodexDeck 安装目录 {}", executable_path.display()))?;
    Ok(install_dir.join("codex-state-provider-backups"))
}

#[cfg(feature = "desktop")]
pub(crate) fn account_data_dir_migration_candidates(data_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(parent) = data_dir.parent() {
        push_unique_candidate(&mut candidates, parent.join(LEGACY_APP_DATA_DIR_NAME));
        push_unique_candidate(&mut candidates, parent.join(LEGACY_CODEXDECK_DATA_DIR_NAME));
    }

    #[cfg(target_os = "windows")]
    {
        push_named_data_dir_candidates_from_env(&mut candidates, "APPDATA");
    }

    candidates
}

#[cfg(all(feature = "desktop", target_os = "windows"))]
fn push_named_data_dir_candidates_from_env(candidates: &mut Vec<PathBuf>, env_name: &str) {
    let Some(root) = std::env::var_os(env_name).map(PathBuf::from) else {
        return;
    };
    push_unique_candidate(candidates, root.join(LEGACY_APP_DATA_DIR_NAME));
    push_unique_candidate(candidates, root.join(LEGACY_CODEXDECK_DATA_DIR_NAME));
}

#[cfg(feature = "desktop")]
fn push_unique_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}
