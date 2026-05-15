use std::path::PathBuf;

#[cfg(feature = "desktop")]
use std::fs;
#[cfg(feature = "desktop")]
use tauri::AppHandle;
#[cfg(feature = "desktop")]
use tauri::Manager;

const DEV_APP_DATA_DIR_ENV: &str = "CODEXDECK_DEV_DATA_DIR";
const DEV_APP_DATA_DIR_LEGACY_ENV: &str = "CODEX_SWITCH_DEV_DATA_DIR";
const DEV_APP_DATA_DIR_LEGACY_TOOLS_ENV: &str = "CODEX_TOOLS_DEV_DATA_DIR";
const DEV_CODEX_DIR_ENV: &str = "CODEXDECK_DEV_CODEX_DIR";
const DEV_CODEX_DIR_LEGACY_ENV: &str = "CODEX_SWITCH_DEV_CODEX_DIR";
const DEV_CODEX_DIR_LEGACY_TOOLS_ENV: &str = "CODEX_TOOLS_DEV_CODEX_DIR";
#[cfg(feature = "desktop")]
const LEGACY_APP_DATA_DIR_NAME: &str = "com.carry.codex-tools";

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
            || env_path(DEV_APP_DATA_DIR_LEGACY_ENV).is_some()
            || env_path(DEV_APP_DATA_DIR_LEGACY_TOOLS_ENV).is_some())
}

#[cfg(feature = "desktop")]
pub(crate) fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        if let Some(path) = env_path(DEV_APP_DATA_DIR_ENV) {
            return Ok(path);
        }
        if let Some(path) = env_path(DEV_APP_DATA_DIR_LEGACY_ENV) {
            return Ok(path);
        }
        if let Some(path) = env_path(DEV_APP_DATA_DIR_LEGACY_TOOLS_ENV) {
            return Ok(path);
        }
    }

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法获取应用数据目录: {error}"))?;
    migrate_legacy_app_data_if_needed(&data_dir)?;
    Ok(data_dir)
}

pub(crate) fn codex_dir() -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        if let Some(path) = env_path(DEV_CODEX_DIR_ENV) {
            return Ok(path);
        }
        if let Some(path) = env_path(DEV_CODEX_DIR_LEGACY_ENV) {
            return Ok(path);
        }
        if let Some(path) = env_path(DEV_CODEX_DIR_LEGACY_TOOLS_ENV) {
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

#[cfg(feature = "desktop")]
fn migrate_legacy_app_data_if_needed(data_dir: &PathBuf) -> Result<(), String> {
    if data_dir.join("accounts.json").exists() {
        return Ok(());
    }

    let Some(legacy_dir) = legacy_app_data_dir(data_dir) else {
        return Ok(());
    };
    if !legacy_dir.exists() || legacy_dir == *data_dir {
        return Ok(());
    }

    copy_legacy_file_if_missing(
        &legacy_dir.join("accounts.json"),
        &data_dir.join("accounts.json"),
    )?;
    copy_legacy_file_if_missing(
        &legacy_dir.join("accounts.json.last-good.json"),
        &data_dir.join("accounts.json.last-good.json"),
    )?;
    copy_legacy_file_if_missing(
        &legacy_dir.join("accounts.json.prev-good.json"),
        &data_dir.join("accounts.json.prev-good.json"),
    )?;
    copy_legacy_dir_if_missing(&legacy_dir.join("profiles"), &data_dir.join("profiles"))?;
    Ok(())
}

#[cfg(feature = "desktop")]
fn legacy_app_data_dir(data_dir: &PathBuf) -> Option<PathBuf> {
    data_dir
        .parent()
        .map(|parent| parent.join(LEGACY_APP_DATA_DIR_NAME))
}

#[cfg(feature = "desktop")]
fn copy_legacy_file_if_missing(source: &PathBuf, destination: &PathBuf) -> Result<(), String> {
    if !source.is_file() || destination.exists() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建迁移目录失败 {}: {error}", parent.display()))?;
    }
    fs::copy(source, destination).map_err(|error| {
        format!(
            "迁移历史版本数据失败 {} -> {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

#[cfg(feature = "desktop")]
fn copy_legacy_dir_if_missing(source: &PathBuf, destination: &PathBuf) -> Result<(), String> {
    if !source.is_dir() || destination.exists() {
        return Ok(());
    }
    copy_dir_recursive(source, destination).map_err(|error| {
        format!(
            "迁移历史版本目录失败 {} -> {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(feature = "desktop")]
fn copy_dir_recursive(source: &PathBuf, destination: &PathBuf) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if source_path.is_file() {
            fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

