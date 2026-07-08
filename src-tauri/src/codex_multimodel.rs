use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;
use tauri::Manager;
use uuid::Uuid;

use crate::app_paths;
use crate::models::AppSettings;
use crate::profile_files;
use crate::store;
use crate::utils::{new_resolved_command, now_unix_seconds};

const WORKSPACE_ENV: &str = "CODEXDECK_MULTIMODEL_WORKSPACE_DIR";
const MANIFEST_FILE_NAME: &str = "manifest.json";
const CODEXDECK_CATALOG_FILE_NAME: &str = "codexdeck-model-catalog.json";
const MODELS_CACHE_FILE_NAME: &str = "models_cache.json";
const MANAGED_AGENT_PREFIX: &str = "codexdeck-";
const MANAGED_AGENT_MARKER: &str = "# codexdeck-managed = true";
const MANAGED_LAUNCHER_FILE_NAME: &str = "CodexDeck-Codex.cmd";
const CONTROLLED_COPY_DIR_NAME: &str = "controlled-codex";
const CONTROLLED_CURRENT_DIR_NAME: &str = "current";
const CONTROLLED_CANDIDATE_DIR_NAME: &str = "candidate";
const CONTROLLED_PREVIOUS_DIR_NAME: &str = "previous";
const CONTROLLED_APP_DIR_NAME: &str = "app";
const PATCH_STATE_FILE_NAME: &str = "model-picker-patch-state.json";
const UNSUPPORTED_MESSAGE: &str =
    "当前 Codex 桌面端版本暂不适配多模型增强模式，已停止启动。";
const PATCH_BOOTSTRAP_MESSAGE: &str =
    "多模型增强脚本启动失败，已停止启动。";
const CONTROLLED_COPY_MISSING_MESSAGE: &str =
    "多模型模式缺少受控 Codex 副本，已停止启动。请先在设置中重新开启多模型模式。";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MultiModelModeResult {
    pub(crate) enabled: bool,
    pub(crate) status: String,
    pub(crate) workspace: String,
    pub(crate) restore_point: Option<String>,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreManifest {
    schema_version: u8,
    created_at: i64,
    codex_dir: String,
    original_codex_launch_path: Option<String>,
    files: Vec<RestoreFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreFile {
    kind: String,
    source: String,
    backup: String,
    #[serde(default = "restore_file_present_by_default")]
    present: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchState {
    status: String,
    #[serde(default)]
    source_codex_version: Option<String>,
    #[serde(default)]
    source_asar_hash: Option<String>,
    #[serde(default)]
    patched_asar_hash: Option<String>,
    #[serde(default)]
    patch_names: Vec<String>,
    #[serde(default)]
    managed_app_root: Option<String>,
    #[serde(default)]
    launcher_path: Option<String>,
    #[serde(default)]
    app_asar_path: Option<String>,
    #[serde(default)]
    controlled_app_root: Option<String>,
    #[serde(default)]
    launch_path: Option<String>,
}

#[derive(Debug, Clone)]
struct ControlledCodexCopy {
    source_app_root: PathBuf,
    controlled_app_root: PathBuf,
    controlled_exe_path: PathBuf,
    controlled_app_asar_path: PathBuf,
    source_asar_hash: String,
    source_codex_version: Option<String>,
    patch_state_path: PathBuf,
}

#[derive(Debug, Clone)]
struct SourceCodexSnapshot {
    app_root: PathBuf,
    exe_path: PathBuf,
    asar_hash: String,
    codex_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControlledCopyMarker {
    #[serde(default)]
    source_app_root: Option<String>,
    #[serde(default)]
    source_asar_hash: Option<String>,
    #[serde(default)]
    source_codex_version: Option<String>,
    #[serde(default)]
    controlled_asar_hash: Option<String>,
}

pub(crate) fn workspace_dir() -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var(WORKSPACE_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let exe = std::env::current_exe()
        .map_err(|error| format!("无法获取 CodexDeck 安装路径: {error}"))?;
    let install_dir = exe
        .parent()
        .ok_or_else(|| format!("无法解析 CodexDeck 安装目录 {}", exe.display()))?;

    if cfg!(debug_assertions) {
        if let Some(dev_workspace) = dev_multimodel_workspace_outside_repo(install_dir) {
            return Ok(dev_workspace);
        }
    }

    Ok(install_dir.join("codexdeck-multimodel"))
}

fn dev_multimodel_workspace_outside_repo(install_dir: &Path) -> Option<PathBuf> {
    let mut current = Some(install_dir);
    while let Some(dir) = current {
        if dir.file_name().and_then(|name| name.to_str()) == Some("CodexDeck") {
            let parent = dir.parent()?;
            return Some(
                parent
                    .join("CodexDeck-dev-runtime")
                    .join("codexdeck-multimodel"),
            );
        }
        current = dir.parent();
    }
    None
}

pub(crate) fn enable_multi_model_mode(app: &AppHandle) -> Result<MultiModelModeResult, String> {
    let workspace = workspace_dir()?;
    fs::create_dir_all(&workspace)
        .map_err(|error| format!("创建多模型工作区失败 {}: {error}", workspace.display()))?;
    let restore_point = create_restore_point(app, &workspace)?;
    let controlled_copy = rebuild_controlled_codex_copy(app, &workspace)
        .map_err(|error| {
            let _ = restore_point_files_only(&restore_point);
            error
        })?;

    register_controlled_copy(app, &workspace, &controlled_copy, "controlled-copy-ready")?;
    let mut store = store::load_store(app)?;
    store.settings.codex_multi_model_restore_point =
        Some(restore_point.to_string_lossy().to_string());
    store::save_store(app, &store)?;

    Ok(MultiModelModeResult {
        enabled: true,
        status: "controlled-copy-ready".to_string(),
        workspace: workspace.to_string_lossy().to_string(),
        restore_point: Some(restore_point.to_string_lossy().to_string()),
        message: format!(
            "已创建稳定恢复点，并准备好受控 Codex 副本。来源 app.asar: {}",
            controlled_copy.source_asar_hash
        ),
    })
}

pub(crate) fn reset_multi_model_mode(app: &AppHandle) -> Result<MultiModelModeResult, String> {
    let workspace = workspace_dir()?;
    let mut store = store::load_store(app)?;
    let restore_point = store
        .settings
        .codex_multi_model_restore_point
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| latest_restore_point(&workspace))
        .ok_or_else(|| "未找到可用的多模型模式恢复点。".to_string())?;

    let manifest = read_manifest(&restore_point)?;
    restore_manifest_files(&manifest, &restore_point)?;

    store.settings.codex_launch_path = manifest.original_codex_launch_path.clone();
    store.settings.codex_multi_model_mode_enabled = false;
    store.settings.codex_multi_model_status = Some("reset".to_string());
    store.settings.codex_multi_model_workspace = Some(workspace.to_string_lossy().to_string());
    store.settings.codex_multi_model_restore_point =
        Some(restore_point.to_string_lossy().to_string());
    clear_controlled_copy_settings(&mut store.settings);
    reconcile_settings_state(app, &mut store.settings);
    store::save_store(app, &store)?;

    Ok(MultiModelModeResult {
        enabled: false,
        status: "reset".to_string(),
        workspace: workspace.to_string_lossy().to_string(),
        restore_point: Some(restore_point.to_string_lossy().to_string()),
        message: "已恢复到确认过的可用状态。".to_string(),
    })
}

pub(crate) fn prepare_managed_codex_launch_path(app: &AppHandle) -> Result<Option<String>, String> {
    let started_at = Instant::now();
    let workspace = workspace_dir()?;
    fs::create_dir_all(&workspace)
        .map_err(|error| format!("创建多模型工作区失败 {}: {error}", workspace.display()))?;
    log_multimodel_launch_phase(&started_at, "workspace");

    let restore_point = current_restore_point(app, &workspace)
        .ok_or_else(|| "多模型模式缺少稳定恢复点，已停止启动。请先在设置中重新开启多模型模式。".to_string())?;
    reconcile_controlled_copy_registration(app, &workspace)?;
    log_multimodel_launch_phase(&started_at, "registration");
    let mut controlled_copy = current_controlled_copy(app, &workspace).map_err(|error| {
        let _ = mark_status(app, "failed", Some(&workspace), Some(&restore_point));
        let _ = restore_point_files_only(&restore_point);
        error
    })?;
    log_multimodel_launch_phase(&started_at, "current-copy");
    controlled_copy = maybe_rebuild_controlled_copy_for_source_update(app, &workspace, controlled_copy)
        .map_err(|error| {
            let _ = mark_status(app, "failed", Some(&workspace), Some(&restore_point));
            let _ = restore_point_files_only(&restore_point);
            error
        })?;
    log_multimodel_launch_phase(&started_at, "source-check");
    if let Err(error) = sync_codex_multi_model_assets() {
        let _ = mark_status(app, "failed", Some(&workspace), Some(&restore_point));
        let _ = restore_point_files_only(&restore_point);
        return Err(error);
    }
    log_multimodel_launch_phase(&started_at, "asset-sync");

    let state = run_patch_script(app, &workspace, &controlled_copy).map_err(|error| {
        let status = if is_patch_bootstrap_error(&error) {
            "failed"
        } else {
            "unsupported"
        };
        let headline = if status == "failed" {
            PATCH_BOOTSTRAP_MESSAGE
        } else {
            UNSUPPORTED_MESSAGE
        };
        let _ = mark_status(app, status, Some(&workspace), Some(&restore_point));
        let _ = restore_point_files_only(&restore_point);
        format!("{headline}\n{error}")
    })?;
    log_multimodel_launch_phase(&started_at, "patch");
    let launch_path = launch_path_from_patch_state(&state)
        .unwrap_or_else(|| controlled_copy.controlled_exe_path.to_string_lossy().to_string());
    if !path_is_within_windows_like(
        &launch_path,
        &workspace.join(CONTROLLED_COPY_DIR_NAME),
    ) {
        let _ = mark_status(app, "unsupported", Some(&workspace), Some(&restore_point));
        return Err(format!("{UNSUPPORTED_MESSAGE}\npatch 状态返回了非受控启动路径。"));
    }

    if !Path::new(&launch_path).exists() {
        let _ = mark_status(app, "unsupported", Some(&workspace), Some(&restore_point));
        return Err(format!(
            "{UNSUPPORTED_MESSAGE}\n受控 Codex 启动文件不存在: {launch_path}"
        ));
    }

    let preserve_status = store::load_store(app)
        .ok()
        .and_then(|store| store.settings.codex_multi_model_status)
        .map(|status| should_preserve_launch_status(&status))
        .unwrap_or(false);
    if !preserve_status {
        mark_status(app, "enabled", Some(&workspace), None)?;
    }
    log::info!(
        "多模型启动准备完成: total_ms={}",
        started_at.elapsed().as_millis()
    );
    Ok(Some(launch_path))
}

fn log_multimodel_launch_phase(started_at: &Instant, phase: &str) {
    log::info!(
        "多模型启动准备阶段完成: phase={phase}, elapsed_ms={}",
        started_at.elapsed().as_millis()
    );
}

fn sync_codex_multi_model_assets() -> Result<(), String> {
    let codex_dir = app_paths::codex_dir()?;
    let catalog_path = codex_dir.join(CODEXDECK_CATALOG_FILE_NAME);
    if catalog_path.exists() {
        let catalog = fs::read_to_string(&catalog_path).map_err(|error| {
            format!(
                "读取 CodexDeck 模型 catalog 失败 {}: {error}",
                catalog_path.display()
            )
        })?;
        profile_files::sync_models_cache_from_model_catalog_json(&catalog)?;
        sync_agents_from_model_catalog_json(&catalog)?;
    }
    Ok(())
}

fn sync_agents_from_model_catalog_json(catalog_json: &str) -> Result<(), String> {
    let catalog: serde_json::Value = serde_json::from_str(catalog_json)
        .map_err(|error| format!("Codex 模型 catalog 不是合法 JSON: {error}"))?;
    let Some(models) = catalog.get("models").and_then(serde_json::Value::as_array) else {
        return Ok(());
    };

    let agents_dir = app_paths::codex_dir()?.join("agents");
    fs::create_dir_all(&agents_dir)
        .map_err(|error| format!("创建 Codex agents 目录失败 {}: {error}", agents_dir.display()))?;
    cleanup_codexdeck_agents(&agents_dir)?;

    for model in models {
        let Some(slug) = model.get("slug").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let slug = slug.trim();
        if slug.is_empty() {
            continue;
        }
        let display_name = model
            .get("display_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(slug);
        write_agent_toml(&agents_dir, slug, display_name)?;
    }
    Ok(())
}

fn cleanup_codexdeck_agents(agents_dir: &Path) -> Result<(), String> {
    if !agents_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(agents_dir)
        .map_err(|error| format!("读取 Codex agents 目录失败 {}: {error}", agents_dir.display()))?
    {
        let entry = entry.map_err(|error| format!("读取 Codex agent 文件失败: {error}"))?;
        let path = entry.path();
        if path.is_file() && is_codexdeck_managed_agent(&path) {
            fs::remove_file(&path)
                .map_err(|error| format!("移除旧 CodexDeck agent 失败 {}: {error}", path.display()))?;
        }
    }
    Ok(())
}

fn write_agent_toml(agents_dir: &Path, model: &str, display_name: &str) -> Result<(), String> {
    let file_name = format!("codexdeck-{}.toml", safe_file_stem(model));
    let path = agents_dir.join(file_name);
    let agent_name = format!("codexdeck_{}", safe_agent_name(model));
    let agent_name = toml_string(&agent_name);
    let description = toml_string(&format!("CodexDeck managed agent for {display_name}."));
    let model = toml_string(model);
    let contents = format!(
        r#"{MANAGED_AGENT_MARKER}
name = {agent_name}
description = {description}
model = {model}
sandbox_mode = "read-only"
developer_instructions = """
You are a CodexDeck managed subagent. Follow the user's task and keep the response concise.
"""
"#
    );
    fs::write(&path, contents)
        .map_err(|error| format!("写入 CodexDeck agent 失败 {}: {error}", path.display()))
}

fn safe_file_stem(value: &str) -> String {
    let safe = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if safe.is_empty() {
        "model".to_string()
    } else {
        safe
    }
}

fn safe_agent_name(value: &str) -> String {
    let safe = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if safe.is_empty() {
        "model".to_string()
    } else {
        safe
    }
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn rebuild_controlled_codex_copy(
    app: &AppHandle,
    workspace: &Path,
) -> Result<ControlledCodexCopy, String> {
    let source = resolve_source_codex_snapshot(app, workspace)?;
    let candidate = prepare_candidate_controlled_copy(workspace, &source)?;
    let state = run_patch_script(app, workspace, &candidate)?;
    validate_patch_state_for_controlled_copy(&state, &candidate)?;
    promote_candidate_controlled_copy(workspace, &candidate)
}

fn maybe_rebuild_controlled_copy_for_source_update(
    app: &AppHandle,
    workspace: &Path,
    current: ControlledCodexCopy,
) -> Result<ControlledCodexCopy, String> {
    let source = match resolve_source_codex_snapshot(app, workspace) {
        Ok(source) => source,
        Err(error) => {
            log::warn!("检测官方 Codex 版本失败，继续使用旧稳定受控副本: {error}");
            mark_status(app, "source-check-unavailable", Some(workspace), None)?;
            return Ok(current);
        }
    };

    if current.source_asar_hash == source.asar_hash {
        return Ok(current);
    }

    log::info!(
        "检测到官方 Codex 已更新，尝试重建多模型受控副本: old={}, new={}",
        current.source_asar_hash,
        source.asar_hash
    );
    match rebuild_controlled_codex_copy_from_source(app, workspace, &source) {
        Ok(rebuilt) => {
            register_controlled_copy(app, workspace, &rebuilt, "updated")?;
            Ok(rebuilt)
        }
        Err(error) => {
            log::warn!("重建新版多模型受控副本失败，继续使用旧稳定副本: {error}");
            mark_status(app, "fallback-previous", Some(workspace), None)?;
            Ok(current)
        }
    }
}

fn rebuild_controlled_codex_copy_from_source(
    app: &AppHandle,
    workspace: &Path,
    source: &SourceCodexSnapshot,
) -> Result<ControlledCodexCopy, String> {
    let candidate = prepare_candidate_controlled_copy(workspace, source)?;
    let state = run_patch_script(app, workspace, &candidate)?;
    validate_patch_state_for_controlled_copy(&state, &candidate)?;
    promote_candidate_controlled_copy(workspace, &candidate)
}

fn resolve_source_codex_snapshot(
    app: &AppHandle,
    workspace: &Path,
) -> Result<SourceCodexSnapshot, String> {
    let (source_app_root, source_exe_path) =
        resolve_source_app_root_for_controlled_copy(app, workspace)?;
    let source_app_asar_path = source_app_root.join("resources").join("app.asar");
    if !source_exe_path.is_file() {
        return Err(format!(
            "无法复制受控 Codex：来源缺少可启动文件: {}",
            source_exe_path.display()
        ));
    }
    if !source_app_asar_path.is_file() {
        return Err(format!(
            "无法复制受控 Codex：来源缺少 app.asar: {}",
            source_app_asar_path.display()
        ));
    }
    let asar_hash = sha256_file(&source_app_asar_path)?;
    Ok(SourceCodexSnapshot {
        app_root: source_app_root.clone(),
        exe_path: source_exe_path,
        asar_hash,
        codex_version: codex_version_from_app_root(&source_app_root),
    })
}

fn prepare_candidate_controlled_copy(
    workspace: &Path,
    source: &SourceCodexSnapshot,
) -> Result<ControlledCodexCopy, String> {
    let controlled_root = workspace.join(CONTROLLED_COPY_DIR_NAME);
    let controlled_app_root = controlled_root
        .join(CONTROLLED_CANDIDATE_DIR_NAME)
        .join(CONTROLLED_APP_DIR_NAME);
    ensure_safe_controlled_copy_target(workspace, &controlled_app_root)?;
    if controlled_app_root.exists() {
        fs::remove_dir_all(&controlled_app_root).map_err(|error| {
            format!(
                "清理旧候选 Codex 副本失败 {}: {error}",
                controlled_app_root.display()
            )
        })?;
    }
    copy_dir_recursive(&source.app_root, &controlled_app_root).map_err(|error| {
        format!(
            "复制受控 Codex 副本失败 {} -> {}: {error}",
            source.app_root.display(),
            controlled_app_root.display()
        )
    })?;

    let controlled_exe_path = relative_path_between(&source.app_root, &source.exe_path)
        .map(|relative| controlled_app_root.join(relative))
        .unwrap_or_else(|| find_codex_launch_exe_in_app_root(&controlled_app_root));
    let controlled_app_asar_path = controlled_app_root.join("resources").join("app.asar");
    if !controlled_exe_path.is_file() {
        return Err(format!(
            "受控 Codex 副本缺少可启动文件: {}",
            controlled_exe_path.display()
        ));
    }
    if !controlled_app_asar_path.is_file() {
        return Err(format!(
            "受控 Codex 副本缺少 app.asar: {}",
            controlled_app_asar_path.display()
        ));
    }

    let controlled_asar_hash = sha256_file(&controlled_app_asar_path)?;
    write_controlled_copy_marker(
        workspace,
        &source.app_root,
        &controlled_app_root,
        &controlled_exe_path,
        &controlled_app_asar_path,
        &source.asar_hash,
        &controlled_asar_hash,
        source.codex_version.as_deref(),
    )?;

    Ok(ControlledCodexCopy {
        source_app_root: source.app_root.clone(),
        controlled_app_root,
        controlled_exe_path,
        controlled_app_asar_path,
        source_asar_hash: source.asar_hash.clone(),
        source_codex_version: source.codex_version.clone(),
        patch_state_path: workspace.join(PATCH_STATE_FILE_NAME),
    })
}

fn promote_candidate_controlled_copy(
    workspace: &Path,
    candidate: &ControlledCodexCopy,
) -> Result<ControlledCodexCopy, String> {
    let controlled_root = workspace.join(CONTROLLED_COPY_DIR_NAME);
    let current_app_root = controlled_root
        .join(CONTROLLED_CURRENT_DIR_NAME)
        .join(CONTROLLED_APP_DIR_NAME);
    let previous_app_root = controlled_root
        .join(CONTROLLED_PREVIOUS_DIR_NAME)
        .join(CONTROLLED_APP_DIR_NAME);
    ensure_safe_controlled_copy_target(workspace, &current_app_root)?;
    ensure_safe_controlled_copy_target(workspace, &previous_app_root)?;

    if previous_app_root.exists() {
        fs::remove_dir_all(&previous_app_root).map_err(|error| {
            format!(
                "清理旧 previous Codex 副本失败 {}: {error}",
                previous_app_root.display()
            )
        })?;
    }
    if current_app_root.exists() {
        if let Some(parent) = previous_app_root.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 previous Codex 目录失败 {}: {error}", parent.display())
            })?;
        }
        fs::rename(&current_app_root, &previous_app_root).map_err(|error| {
            format!(
                "保留旧稳定 Codex 副本失败 {} -> {}: {error}",
                current_app_root.display(),
                previous_app_root.display()
            )
        })?;
    }
    if let Some(parent) = current_app_root.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 current Codex 目录失败 {}: {error}", parent.display()))?;
    }
    fs::rename(&candidate.controlled_app_root, &current_app_root).map_err(|error| {
        format!(
            "晋级候选 Codex 副本失败 {} -> {}: {error}",
            candidate.controlled_app_root.display(),
            current_app_root.display()
        )
    })?;

    let controlled_exe_path = relative_path_between(&candidate.controlled_app_root, &candidate.controlled_exe_path)
        .map(|relative| current_app_root.join(relative))
        .unwrap_or_else(|| find_codex_launch_exe_in_app_root(&current_app_root));
    let controlled_app_asar_path = current_app_root.join("resources").join("app.asar");
    rewrite_controlled_copy_marker_paths(
        workspace,
        &current_app_root,
        &controlled_exe_path,
        &controlled_app_asar_path,
    )?;

    Ok(ControlledCodexCopy {
        source_app_root: candidate.source_app_root.clone(),
        controlled_app_root: current_app_root,
        controlled_exe_path,
        controlled_app_asar_path,
        source_asar_hash: candidate.source_asar_hash.clone(),
        source_codex_version: candidate.source_codex_version.clone(),
        patch_state_path: workspace.join(PATCH_STATE_FILE_NAME),
    })
}

fn resolve_source_app_root_for_controlled_copy(
    app: &AppHandle,
    workspace: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    let store = store::load_store(app)?;
    let mut candidates = Vec::new();
    if let Some(path) = store.settings.codex_launch_path.as_deref() {
        candidates.push(PathBuf::from(path));
    }
    if let Some(path) = crate::settings_service::dev_controlled_codex_launch_path() {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend(resolve_running_windows_codex_app_dirs());
    candidates.extend(resolve_windows_apps_codex_app_dirs());

    for candidate in candidates {
        if is_managed_codex_launch_path(&candidate.to_string_lossy(), Some(workspace)) {
            continue;
        }
        if let Some((root, exe)) = app_root_from_codex_launch_path(&candidate) {
            if path_is_within_windows_like(
                &root.to_string_lossy(),
                &workspace.join(CONTROLLED_COPY_DIR_NAME),
            ) {
                continue;
            }
            if root.join("resources").join("app.asar").is_file() && exe.is_file() {
                return Ok((root, exe));
            }
        }
    }

    Err("无法找到可复制的 Codex 桌面端。请先确认当前 Codex 可正常启动，或在设置中指定 Codex.exe。".to_string())
}

#[cfg(target_os = "windows")]
fn resolve_running_windows_codex_app_dirs() -> Vec<PathBuf> {
    let script = r#"Get-CimInstance Win32_Process -Filter "Name='Codex.exe' OR Name='codex.exe'" |
Select-Object ProcessId,CommandLine,ExecutablePath |
ConvertTo-Json -Compress"#;
    let Ok(output) = new_resolved_command("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let processes = value
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![value]);
    let mut out = Vec::new();
    for process in processes {
        let command_line = process
            .get("CommandLine")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if command_line.contains("--type=") || command_line.to_ascii_lowercase().contains("app-server") {
            continue;
        }
        if let Some(path) = process
            .get("ExecutablePath")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| executable_path_from_command_line(command_line))
        {
            if let Some((root, _exe)) = app_root_from_codex_launch_path(&path) {
                out.push(root);
            }
        }
    }
    dedupe_paths(out)
}

#[cfg(not(target_os = "windows"))]
fn resolve_running_windows_codex_app_dirs() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(target_os = "windows")]
fn resolve_windows_apps_codex_app_dirs() -> Vec<PathBuf> {
    let mut candidates = resolve_windows_apps_codex_app_dirs_from_registry();
    candidates.extend(resolve_windows_apps_codex_app_dirs_from_directory_listing());
    sort_codex_app_dirs_by_version(candidates)
}

#[cfg(target_os = "windows")]
fn windows_apps_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for env_name in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = std::env::var_os(env_name).map(PathBuf::from) {
            roots.push(base.join("WindowsApps"));
        }
    }
    dedupe_paths(roots)
}

#[cfg(target_os = "windows")]
fn resolve_windows_apps_codex_app_dirs_from_registry() -> Vec<PathBuf> {
    const PACKAGE_REPOSITORY_KEY: &str = r"HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    let Ok(output) = new_resolved_command("reg")
        .args(["query", PACKAGE_REPOSITORY_KEY, "/k", "/f", "OpenAI.Codex_"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let package_names = parse_codex_package_names_from_registry_query(&raw);
    let roots = windows_apps_roots();
    let mut candidates = Vec::new();
    for package_name in package_names {
        for root in &roots {
            let app_dir = root.join(&package_name).join("app");
            if is_copyable_codex_app_root(&app_dir) {
                candidates.push(app_dir);
            }
        }
    }
    candidates
}

#[cfg(target_os = "windows")]
fn parse_codex_package_names_from_registry_query(raw: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in raw.lines().map(str::trim) {
        let Some(name) = line.rsplit('\\').next() else {
            continue;
        };
        if !name.starts_with("OpenAI.Codex_") || !name.contains("__") {
            continue;
        }
        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    }
    names
}

#[cfg(target_os = "windows")]
fn resolve_windows_apps_codex_app_dirs_from_directory_listing() -> Vec<PathBuf> {
    let roots = windows_apps_roots();

    let mut candidates = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.starts_with("OpenAI.Codex_") {
                continue;
            }
            let app_dir = path.join("app");
            if is_copyable_codex_app_root(&app_dir) {
                candidates.push(app_dir);
            }
        }
    }
    candidates
}

fn is_copyable_codex_app_root(app_dir: &Path) -> bool {
    app_dir.join("resources").join("app.asar").is_file()
        && find_codex_launch_exe_in_app_root(app_dir).is_file()
}

#[cfg(target_os = "windows")]
fn sort_codex_app_dirs_by_version(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut candidates = dedupe_paths(paths)
        .into_iter()
        .map(|path| {
            let version_key = codex_app_root_version_key(&path);
            let mtime = path.metadata().and_then(|metadata| metadata.modified()).ok();
            (path, version_key, mtime)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.2.cmp(&left.2))
    });
    candidates.into_iter().map(|(path, _, _)| path).collect()
}

#[cfg(not(target_os = "windows"))]
fn resolve_windows_apps_codex_app_dirs() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(target_os = "windows")]
fn executable_path_from_command_line(command_line: &str) -> Option<PathBuf> {
    let trimmed = command_line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(PathBuf::from(&rest[..end]));
    }
    let lower = trimmed.to_ascii_lowercase();
    let marker = ".exe";
    let end = lower.find(marker)? + marker.len();
    Some(PathBuf::from(&trimmed[..end]))
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if !out.iter().any(|existing: &PathBuf| existing == &path) {
            out.push(path);
        }
    }
    out
}

fn app_root_from_codex_launch_path(path: &Path) -> Option<(PathBuf, PathBuf)> {
    if path.is_file() {
        let mut current = path.parent();
        while let Some(dir) = current {
            if dir.join("resources").join("app.asar").is_file() {
                return Some((dir.to_path_buf(), path.to_path_buf()));
            }
            current = dir.parent();
        }
        return None;
    }

    for root in [path.to_path_buf(), path.join("app"), path.join("Application")] {
        if root.join("resources").join("app.asar").is_file() {
            let exe = find_codex_launch_exe_in_app_root(&root);
            if exe.is_file() {
                return Some((root, exe));
            }
        }
    }
    None
}

fn find_codex_launch_exe_in_app_root(app_root: &Path) -> PathBuf {
    for relative in [
        PathBuf::from("Codex.exe"),
        PathBuf::from("Codex Desktop.exe"),
        PathBuf::from("resources").join("codex.exe"),
    ] {
        let candidate = app_root.join(relative);
        if candidate.is_file() {
            return candidate;
        }
    }
    app_root.join("Codex.exe")
}

fn relative_path_between(root: &Path, child: &Path) -> Option<PathBuf> {
    child.strip_prefix(root).ok().map(Path::to_path_buf)
}

fn current_controlled_copy(
    app: &AppHandle,
    workspace: &Path,
) -> Result<ControlledCodexCopy, String> {
    let store = store::load_store(app)?;
    let controlled_app_root = store
        .settings
        .codex_multi_model_controlled_app_root
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| CONTROLLED_COPY_MISSING_MESSAGE.to_string())?;
    let controlled_exe_path = store
        .settings
        .codex_multi_model_controlled_exe_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| CONTROLLED_COPY_MISSING_MESSAGE.to_string())?;
    let controlled_app_asar_path = store
        .settings
        .codex_multi_model_controlled_app_asar_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| CONTROLLED_COPY_MISSING_MESSAGE.to_string())?;
    let source_app_root = store
        .settings
        .codex_multi_model_source_app_root
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_default();
    let patch_state_path = store
        .settings
        .codex_multi_model_patch_state_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join(PATCH_STATE_FILE_NAME));

    ensure_safe_controlled_copy_target(workspace, &controlled_app_root)?;
    if !controlled_exe_path.is_file() {
        return Err(format!(
            "{CONTROLLED_COPY_MISSING_MESSAGE}\n受控 Codex 启动文件不存在: {}",
            controlled_exe_path.display()
        ));
    }
    if !controlled_app_asar_path.is_file() {
        return Err(format!(
            "{CONTROLLED_COPY_MISSING_MESSAGE}\n受控 app.asar 不存在: {}",
            controlled_app_asar_path.display()
        ));
    }
    if !path_is_within_windows_like(
        &controlled_exe_path.to_string_lossy(),
        &controlled_app_root,
    ) || !path_is_within_windows_like(
        &controlled_app_asar_path.to_string_lossy(),
        &controlled_app_root,
    ) {
        return Err(format!(
            "{CONTROLLED_COPY_MISSING_MESSAGE}\n受控路径不在受控副本目录内。"
        ));
    }
    let marker_path = controlled_copy_marker_path(&controlled_app_root);
    if !marker_path.is_file() {
        return Err(format!(
            "{CONTROLLED_COPY_MISSING_MESSAGE}\n受控副本缺少 CodexDeck marker。"
        ));
    }
    let marker = read_controlled_copy_marker(&controlled_app_root).ok();
    let source_asar_hash = marker
        .as_ref()
        .and_then(|marker| marker.source_asar_hash.clone())
        .unwrap_or_default();
    let source_codex_version = marker
        .as_ref()
        .and_then(|marker| marker.source_codex_version.clone());

    Ok(ControlledCodexCopy {
        source_app_root,
        controlled_app_root,
        controlled_exe_path,
        controlled_app_asar_path,
        source_asar_hash,
        source_codex_version,
        patch_state_path,
    })
}

fn register_controlled_copy(
    app: &AppHandle,
    workspace: &Path,
    controlled_copy: &ControlledCodexCopy,
    status: &str,
) -> Result<(), String> {
    let mut store = store::load_store(app)?;
    store.settings.codex_multi_model_mode_enabled = true;
    store.settings.codex_multi_model_status = Some(status.to_string());
    store.settings.codex_multi_model_workspace = Some(workspace.to_string_lossy().to_string());
    store.settings.codex_multi_model_source_app_root =
        Some(controlled_copy.source_app_root.to_string_lossy().to_string());
    store.settings.codex_multi_model_controlled_app_root =
        Some(controlled_copy.controlled_app_root.to_string_lossy().to_string());
    store.settings.codex_multi_model_controlled_exe_path =
        Some(controlled_copy.controlled_exe_path.to_string_lossy().to_string());
    store.settings.codex_multi_model_controlled_app_asar_path =
        Some(controlled_copy.controlled_app_asar_path.to_string_lossy().to_string());
    store.settings.codex_multi_model_patch_state_path =
        Some(controlled_copy.patch_state_path.to_string_lossy().to_string());
    store.settings.codex_launch_path =
        Some(controlled_copy.controlled_exe_path.to_string_lossy().to_string());
    store::save_store(app, &store)
}

fn reconcile_controlled_copy_registration(
    app: &AppHandle,
    workspace: &Path,
) -> Result<(), String> {
    let mut store = store::load_store(app)?;
    if restore_registered_controlled_copy_from_disk(&mut store.settings, workspace) {
        store::save_store(app, &store)?;
    }
    Ok(())
}

fn ensure_safe_controlled_copy_target(workspace: &Path, target: &Path) -> Result<(), String> {
    let root = workspace.join(CONTROLLED_COPY_DIR_NAME);
    if !path_is_within_windows_like(&target.to_string_lossy(), &root) {
        return Err(format!(
            "受控 Codex 副本路径不在多模型工作区内，已拒绝操作: {}",
            target.display()
        ));
    }
    Ok(())
}

fn controlled_current_app_root(workspace: &Path) -> PathBuf {
    workspace
        .join(CONTROLLED_COPY_DIR_NAME)
        .join(CONTROLLED_CURRENT_DIR_NAME)
        .join(CONTROLLED_APP_DIR_NAME)
}

fn controlled_legacy_app_root(workspace: &Path) -> PathBuf {
    workspace
        .join(CONTROLLED_COPY_DIR_NAME)
        .join(CONTROLLED_APP_DIR_NAME)
}

fn controlled_previous_app_root(workspace: &Path) -> PathBuf {
    workspace
        .join(CONTROLLED_COPY_DIR_NAME)
        .join(CONTROLLED_PREVIOUS_DIR_NAME)
        .join(CONTROLLED_APP_DIR_NAME)
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if source_path.is_file() {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

fn write_controlled_copy_marker(
    workspace: &Path,
    source_app_root: &Path,
    controlled_app_root: &Path,
    controlled_exe_path: &Path,
    controlled_app_asar_path: &Path,
    source_asar_hash: &str,
    controlled_asar_hash: &str,
    source_codex_version: Option<&str>,
) -> Result<(), String> {
    let marker = serde_json::json!({
        "schemaVersion": 1,
        "createdAt": now_unix_seconds(),
        "workspace": workspace.to_string_lossy(),
        "sourceCodexVersion": source_codex_version,
        "sourceAppRoot": source_app_root.to_string_lossy(),
        "controlledAppRoot": controlled_app_root.to_string_lossy(),
        "controlledExePath": controlled_exe_path.to_string_lossy(),
        "controlledAppAsarPath": controlled_app_asar_path.to_string_lossy(),
        "sourceAsarHash": source_asar_hash,
        "controlledAsarHash": controlled_asar_hash,
    });
    let marker_path = controlled_copy_marker_path(controlled_app_root);
    let serialized = serde_json::to_vec_pretty(&marker)
        .map_err(|error| format!("序列化受控 Codex marker 失败: {error}"))?;
    fs::write(&marker_path, serialized)
        .map_err(|error| format!("写入受控 Codex marker 失败 {}: {error}", marker_path.display()))
}

fn controlled_copy_marker_path(controlled_app_root: &Path) -> PathBuf {
    controlled_app_root.join(".codexdeck-controlled.json")
}

fn read_controlled_copy_marker(
    controlled_app_root: &Path,
) -> Result<ControlledCopyMarker, String> {
    let marker_path = controlled_copy_marker_path(controlled_app_root);
    let raw = fs::read_to_string(&marker_path)
        .map_err(|error| format!("读取受控 Codex marker 失败 {}: {error}", marker_path.display()))?;
    serde_json::from_str::<ControlledCopyMarker>(&raw)
        .map_err(|error| format!("解析受控 Codex marker 失败 {}: {error}", marker_path.display()))
}

fn rewrite_controlled_copy_marker_paths(
    workspace: &Path,
    controlled_app_root: &Path,
    controlled_exe_path: &Path,
    controlled_app_asar_path: &Path,
) -> Result<(), String> {
    let marker = read_controlled_copy_marker(controlled_app_root)?;
    let controlled_asar_hash = sha256_file(controlled_app_asar_path)?;
    write_controlled_copy_marker(
        workspace,
        &marker
            .source_app_root
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_default(),
        controlled_app_root,
        controlled_exe_path,
        controlled_app_asar_path,
        marker.source_asar_hash.as_deref().unwrap_or_default(),
        &controlled_asar_hash,
        marker.source_codex_version.as_deref(),
    )
}

fn codex_version_from_app_root(app_root: &Path) -> Option<String> {
    let mut current = Some(app_root);
    while let Some(dir) = current {
        let name = dir.file_name()?.to_str()?;
        if let Some(rest) = name.strip_prefix("OpenAI.Codex_") {
            return rest.split('_').next().map(ToString::to_string);
        }
        current = dir.parent();
    }
    None
}

fn codex_app_root_version_key(app_root: &Path) -> Vec<u64> {
    codex_version_from_app_root(app_root)
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|error| format!("读取文件失败 {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("读取文件失败 {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn create_restore_point(app: &AppHandle, workspace: &Path) -> Result<PathBuf, String> {
    let stamp = format!("{}-{}", now_unix_seconds(), Uuid::new_v4());
    let root = workspace.join("restore-points").join(stamp);
    fs::create_dir_all(&root)
        .map_err(|error| format!("创建恢复点目录失败 {}: {error}", root.display()))?;

    let codex_dir = app_paths::codex_dir()?;
    let store = store::load_store(app)?;
    let mut manifest = RestoreManifest {
        schema_version: 1,
        created_at: now_unix_seconds(),
        codex_dir: codex_dir.to_string_lossy().to_string(),
        original_codex_launch_path: sanitize_restore_launch_path(
            store.settings.codex_launch_path.clone(),
            workspace,
        ),
        files: Vec::new(),
    };

    backup_optional_file(
        &mut manifest,
        &root,
        "config",
        &codex_dir.join("config.toml"),
        Path::new("config.toml"),
    )?;
    backup_optional_file(
        &mut manifest,
        &root,
        "models-cache",
        &codex_dir.join(MODELS_CACHE_FILE_NAME),
        Path::new(MODELS_CACHE_FILE_NAME),
    )?;
    backup_optional_file(
        &mut manifest,
        &root,
        "model-catalog",
        &codex_dir.join(CODEXDECK_CATALOG_FILE_NAME),
        Path::new(CODEXDECK_CATALOG_FILE_NAME),
    )?;
    backup_managed_agents(&mut manifest, &root, &codex_dir.join("agents"))?;

    let launch_path_payload = serde_json::to_vec_pretty(&serde_json::json!({
        "codexLaunchPath": manifest.original_codex_launch_path.clone(),
    }))
    .map_err(|error| format!("序列化启动路径恢复信息失败: {error}"))?;
    let launch_backup = root.join("accounts-launch-path.json");
    fs::write(&launch_backup, launch_path_payload)
        .map_err(|error| format!("写入启动路径恢复信息失败 {}: {error}", launch_backup.display()))?;
    manifest.files.push(RestoreFile {
        kind: "launch-path".to_string(),
        source: "accounts.json.settings.codexLaunchPath".to_string(),
        backup: "accounts-launch-path.json".to_string(),
        present: true,
    });

    write_manifest(&root, &manifest)?;
    Ok(root)
}

fn run_patch_script(
    app: &AppHandle,
    workspace: &Path,
    controlled_copy: &ControlledCodexCopy,
) -> Result<PatchState, String> {
    if let Some(state) = cached_patch_state_for_controlled_copy(controlled_copy) {
        log::info!("复用已验证的多模型 patch 状态，跳过重复 app.asar 扫描。");
        return Ok(state);
    }

    let (script_arg, command_cwd) = patch_script_invocation(app)?;
    let mut command = node_command();
    let output = command
        .current_dir(&command_cwd)
        .arg(&script_arg)
        .env("CODEXDECK_PATCH_TARGET", "controlled")
        .env("CODEXDECK_MULTIMODEL_WORKSPACE_DIR", workspace)
        .env(
            "CODEXDECK_CONTROLLED_APP_ROOT",
            &controlled_copy.controlled_app_root,
        )
        .env(
            "CODEXDECK_CONTROLLED_APP_ASAR",
            &controlled_copy.controlled_app_asar_path,
        )
        .env(
            "CODEXDECK_SOURCE_CODEX_VERSION",
            controlled_copy
                .source_codex_version
                .as_deref()
                .unwrap_or_default(),
        )
        .env("CODEXDECK_SOURCE_ASAR_HASH", &controlled_copy.source_asar_hash)
        .output()
        .map_err(|error| format!("运行 Codex 模型选择器 patch 脚本失败: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("退出码 {:?}", output.status.code())
        };
        return Err(detail);
    }

    let state_path = &controlled_copy.patch_state_path;
    let raw = fs::read_to_string(&state_path)
        .map_err(|error| format!("读取 patch 状态失败 {}: {error}", state_path.display()))?;
    let state = serde_json::from_str::<PatchState>(&raw)
        .map_err(|error| format!("解析 patch 状态失败 {}: {error}", state_path.display()))?;
    if !matches!(state.status.as_str(), "patched" | "already-patched") {
        return Err(format!("patch 状态不可用: {}", state.status));
    }
    if let Some(asar_path) = state.app_asar_path.as_deref() {
        if normalize_windows_like_path(asar_path)
            != normalize_windows_like_path(&controlled_copy.controlled_app_asar_path.to_string_lossy())
        {
            return Err("patch 状态中的 app.asar 不是当前受控副本。".to_string());
        }
    }
    validate_patch_state_for_controlled_copy(&state, controlled_copy)?;
    Ok(state)
}

fn cached_patch_state_for_controlled_copy(
    controlled_copy: &ControlledCodexCopy,
) -> Option<PatchState> {
    let raw = fs::read_to_string(&controlled_copy.patch_state_path).ok()?;
    let state = serde_json::from_str::<PatchState>(&raw).ok()?;
    if !matches!(state.status.as_str(), "patched" | "already-patched") {
        return None;
    }
    let asar_path = state.app_asar_path.as_deref()?;
    if normalize_windows_like_path(asar_path)
        != normalize_windows_like_path(&controlled_copy.controlled_app_asar_path.to_string_lossy())
    {
        return None;
    }
    validate_patch_state_for_controlled_copy(&state, controlled_copy).ok()?;
    let marker = read_controlled_copy_marker(&controlled_copy.controlled_app_root).ok()?;
    match (
        marker.controlled_asar_hash.as_deref(),
        state.patched_asar_hash.as_deref(),
    ) {
        (Some(marker_hash), Some(state_hash)) if marker_hash == state_hash => {}
        _ => return None,
    }
    Some(state)
}

fn validate_patch_state_for_controlled_copy(
    state: &PatchState,
    controlled_copy: &ControlledCodexCopy,
) -> Result<(), String> {
    if !state.patch_names.iter().any(|name| name == "model-picker") {
        return Err("patch 未命中新版模型选择器过滤点 model-picker。".to_string());
    }
    if let Some(source_hash) = state.source_asar_hash.as_deref() {
        if !controlled_copy.source_asar_hash.is_empty()
            && source_hash != controlled_copy.source_asar_hash
        {
            return Err("patch 状态中的来源 app.asar hash 与当前受控副本不一致。".to_string());
        }
    }
    if let (Some(state_version), Some(copy_version)) = (
        state.source_codex_version.as_deref(),
        controlled_copy.source_codex_version.as_deref(),
    ) {
        if state_version != copy_version {
            return Err("patch 状态中的 Codex 版本与当前受控副本不一致。".to_string());
        }
    }
    Ok(())
}

fn node_command() -> Command {
    new_resolved_command("node")
}

fn patch_script_invocation(app: &AppHandle) -> Result<(String, PathBuf), String> {
    if let Some(cwd) = repo_root_with_patch_script() {
        return Ok((
            PathBuf::from("scripts")
                .join("patch-codex-model-picker.mjs")
                .to_string_lossy()
                .to_string(),
            cwd,
        ));
    }

    let script = app
        .path()
        .resource_dir()
        .ok()
        .and_then(|dir| find_patch_script_near_install_dir(&dir))
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().and_then(|dir| find_patch_script_near_install_dir(dir)))
        })
        .ok_or_else(|| "未找到 Codex 模型选择器 patch 脚本。".to_string())?;

    let command_cwd = script
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("无法解析 patch 脚本目录: {}", script.display()))?;
    let script_arg = script
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| format!("无法解析 patch 脚本文件名: {}", script.display()))?;
    Ok((script_arg, command_cwd))
}

fn repo_root_with_patch_script() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let candidate = cwd.join("scripts").join("patch-codex-model-picker.mjs");
    candidate.exists().then_some(cwd)
}

fn find_patch_script_near_install_dir(install_dir: &Path) -> Option<PathBuf> {
    for base in [
        install_dir.to_path_buf(),
        install_dir.join("resources"),
        install_dir.join("scripts"),
    ] {
        let candidate = base.join("patch-codex-model-picker.mjs");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn launch_path_from_patch_state(state: &PatchState) -> Option<String> {
    if let Some(path) = state
        .launch_path
        .as_ref()
        .filter(|path| Path::new(path.as_str()).exists())
    {
        return Some(path.clone());
    }
    if let Some(root) = state.controlled_app_root.as_ref() {
        let path = Path::new(root).join("Codex.exe");
        if path.exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }
    if let Some(root) = state.managed_app_root.as_ref() {
        let path = Path::new(root).join("Codex.exe");
        if path.exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }
    if let Some(path) = state
        .launcher_path
        .as_ref()
        .filter(|path| Path::new(path.as_str()).exists())
    {
        return Some(path.clone());
    }
    state.app_asar_path.as_ref().and_then(|asar| {
        let app_root = Path::new(asar).parent()?.parent()?;
        let exe = app_root.join("Codex.exe");
        exe.exists().then(|| exe.to_string_lossy().to_string())
    })
}

fn mark_status(
    app: &AppHandle,
    status: &str,
    workspace: Option<&Path>,
    restore_point: Option<&Path>,
) -> Result<(), String> {
    let mut store = store::load_store(app)?;
    store.settings.codex_multi_model_status = Some(status.to_string());
    if let Some(workspace) = workspace {
        store.settings.codex_multi_model_workspace = Some(workspace.to_string_lossy().to_string());
    }
    if let Some(restore_point) = restore_point {
        store.settings.codex_multi_model_restore_point =
            Some(restore_point.to_string_lossy().to_string());
    }
    if is_inactive_status(status) {
        store.settings.codex_multi_model_mode_enabled = false;
        clear_controlled_copy_settings(&mut store.settings);
        if let Some(restore_point) = restore_point
            .map(Path::to_path_buf)
            .or_else(|| {
                store
                    .settings
                    .codex_multi_model_restore_point
                    .as_ref()
                    .map(PathBuf::from)
            })
        {
            if let Ok(manifest) = read_manifest(&restore_point) {
                store.settings.codex_launch_path = manifest.original_codex_launch_path.clone();
            }
        }
    }
    reconcile_settings_state(app, &mut store.settings);
    store::save_store(app, &store)
}

pub(crate) fn reconcile_settings_state(_app: &AppHandle, settings: &mut AppSettings) -> bool {
    let mut changed = false;
    let workspace = settings
        .codex_multi_model_workspace
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| workspace_dir().ok());
    let inactive_status = settings
        .codex_multi_model_status
        .as_deref()
        .is_some_and(is_inactive_status);

    if inactive_status {
        if settings.codex_multi_model_mode_enabled {
            settings.codex_multi_model_mode_enabled = false;
            changed = true;
        }
        if has_controlled_copy_settings(settings) {
            clear_controlled_copy_settings(settings);
            changed = true;
        }
    } else if let Some(workspace) = workspace.as_deref() {
        if restore_registered_controlled_copy_from_disk(settings, workspace) {
            changed = true;
        }
    }

    if !settings.codex_multi_model_mode_enabled
        && settings
            .codex_launch_path
            .as_deref()
            .zip(settings.codex_multi_model_workspace.as_deref())
            .is_some_and(|(path, workspace)| {
                is_managed_codex_launch_path(path, Some(Path::new(workspace)))
            })
    {
        settings.codex_launch_path = None;
        changed = true;
    }

    if !settings.codex_multi_model_mode_enabled
        && settings
            .codex_launch_path
            .as_deref()
            .is_some_and(|path| is_managed_codex_launch_path(path, None))
    {
        settings.codex_launch_path = None;
        changed = true;
    }

    changed
}

fn restore_registered_controlled_copy_from_disk(
    settings: &mut AppSettings,
    workspace: &Path,
) -> bool {
    if settings
        .codex_multi_model_status
        .as_deref()
        .is_some_and(is_inactive_status)
    {
        return false;
    }

    if settings
        .codex_multi_model_controlled_exe_path
        .as_deref()
        .is_some_and(|path| Path::new(path).is_file())
        && settings
            .codex_multi_model_controlled_app_asar_path
            .as_deref()
            .is_some_and(|path| Path::new(path).is_file())
    {
        return false;
    }

    let candidates = [
        controlled_current_app_root(workspace),
        controlled_legacy_app_root(workspace),
        controlled_previous_app_root(workspace),
    ];
    for app_root in candidates {
        let exe_path = find_codex_launch_exe_in_app_root(&app_root);
        let asar_path = app_root.join("resources").join("app.asar");
        let marker_path = controlled_copy_marker_path(&app_root);
        if !exe_path.is_file() || !asar_path.is_file() || !marker_path.is_file() {
            continue;
        }
        if ensure_safe_controlled_copy_target(workspace, &app_root).is_err() {
            continue;
        }
        let marker = read_controlled_copy_marker(&app_root).ok();
        settings.codex_multi_model_mode_enabled = true;
        settings.codex_multi_model_status = Some("enabled".to_string());
        settings.codex_multi_model_workspace = Some(workspace.to_string_lossy().to_string());
        settings.codex_multi_model_controlled_app_root =
            Some(app_root.to_string_lossy().to_string());
        settings.codex_multi_model_controlled_exe_path =
            Some(exe_path.to_string_lossy().to_string());
        settings.codex_multi_model_controlled_app_asar_path =
            Some(asar_path.to_string_lossy().to_string());
        settings.codex_multi_model_source_app_root = marker
            .and_then(|marker| marker.source_app_root)
            .filter(|value| !value.trim().is_empty());
        settings.codex_multi_model_patch_state_path =
            Some(workspace.join(PATCH_STATE_FILE_NAME).to_string_lossy().to_string());
        settings.codex_launch_path = Some(exe_path.to_string_lossy().to_string());
        return true;
    }
    false
}

fn has_controlled_copy_settings(settings: &AppSettings) -> bool {
    settings.codex_multi_model_controlled_app_root.is_some()
        || settings.codex_multi_model_controlled_exe_path.is_some()
        || settings
            .codex_multi_model_controlled_app_asar_path
            .is_some()
        || settings.codex_multi_model_source_app_root.is_some()
        || settings.codex_multi_model_patch_state_path.is_some()
}

fn clear_controlled_copy_settings(settings: &mut AppSettings) {
    settings.codex_multi_model_controlled_app_root = None;
    settings.codex_multi_model_controlled_exe_path = None;
    settings.codex_multi_model_controlled_app_asar_path = None;
    settings.codex_multi_model_source_app_root = None;
    settings.codex_multi_model_patch_state_path = None;
}

fn is_inactive_status(status: &str) -> bool {
    matches!(status, "unsupported" | "failed" | "reset")
}

fn should_preserve_launch_status(status: &str) -> bool {
    matches!(status, "fallback-previous" | "source-check-unavailable")
}

fn is_patch_bootstrap_error(error: &str) -> bool {
    error.contains("EISDIR")
        || error.contains("lstat 'D:'")
        || error.contains("Could not find official Codex app.asar")
        || error.contains("未找到 Codex 模型选择器 patch 脚本")
        || error.contains("运行 Codex 模型选择器 patch 脚本失败")
}

fn sanitize_restore_launch_path(path: Option<String>, workspace: &Path) -> Option<String> {
    path.filter(|value| !is_managed_codex_launch_path(value, Some(workspace)))
}

fn is_managed_codex_launch_path(path: &str, workspace: Option<&Path>) -> bool {
    let normalized = normalize_windows_like_path(path);
    if normalized.contains("\\codex-managed-copy\\")
        || normalized.contains("\\controlled-codex\\")
        || normalized.ends_with("\\launchers\\codexdeck-codex.cmd")
        || normalized.ends_with("\\codexdeck-codex.cmd")
    {
        return true;
    }

    let Some(workspace) = workspace else {
        return false;
    };
    path_is_within_windows_like(path, &workspace.join("codex-managed-copy"))
        || path_is_within_windows_like(path, &workspace.join(CONTROLLED_COPY_DIR_NAME))
        || path_is_within_windows_like(
            path,
            &workspace.join("launchers").join(MANAGED_LAUNCHER_FILE_NAME),
        )
}

fn path_is_within_windows_like(path: &str, root: &Path) -> bool {
    let normalized_path = normalize_windows_like_path(path);
    let normalized_root = normalize_windows_like_path(&root.to_string_lossy());
    normalized_path == normalized_root
        || normalized_path.starts_with(&format!("{normalized_root}\\"))
}

fn normalize_windows_like_path(value: &str) -> String {
    value
        .trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn backup_optional_file(
    manifest: &mut RestoreManifest,
    root: &Path,
    kind: &str,
    source: &Path,
    relative_backup: &Path,
) -> Result<(), String> {
    if !source.exists() {
        manifest.files.push(RestoreFile {
            kind: kind.to_string(),
            source: source.to_string_lossy().to_string(),
            backup: relative_backup.to_string_lossy().to_string(),
            present: false,
        });
        return Ok(());
    }
    let backup = root.join(relative_backup);
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建恢复点子目录失败 {}: {error}", parent.display()))?;
    }
    fs::copy(source, &backup).map_err(|error| {
        format!(
            "备份多模型恢复文件失败 {} -> {}: {error}",
            source.display(),
            backup.display()
        )
    })?;
    manifest.files.push(RestoreFile {
        kind: kind.to_string(),
        source: source.to_string_lossy().to_string(),
        backup: relative_backup.to_string_lossy().to_string(),
        present: true,
    });
    Ok(())
}

fn backup_if_exists(
    manifest: &mut RestoreManifest,
    root: &Path,
    kind: &str,
    source: &Path,
    relative_backup: &Path,
) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    backup_optional_file(manifest, root, kind, source, relative_backup)
}

fn backup_managed_agents(
    manifest: &mut RestoreManifest,
    root: &Path,
    agents_dir: &Path,
) -> Result<(), String> {
    if !agents_dir.is_dir() {
        return Ok(());
    }

    let entries = fs::read_dir(agents_dir)
        .map_err(|error| format!("读取 Codex agents 目录失败 {}: {error}", agents_dir.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("读取 Codex agent 文件失败: {error}"))?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|item| item.to_str()) != Some("toml") {
            continue;
        }
        if !is_codexdeck_managed_agent(&path) {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let relative = PathBuf::from("agents").join(file_name);
        backup_if_exists(manifest, root, "agent", &path, &relative)?;
    }
    Ok(())
}

fn is_codexdeck_managed_agent(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|item| item.to_str())
        .is_some_and(|name| name.starts_with(MANAGED_AGENT_PREFIX))
    {
        return true;
    }

    fs::read_to_string(path)
        .ok()
        .is_some_and(|contents| contents.contains(MANAGED_AGENT_MARKER))
}

fn restore_manifest_files(manifest: &RestoreManifest, root: &Path) -> Result<(), String> {
    cleanup_codexdeck_agents(&PathBuf::from(&manifest.codex_dir).join("agents"))?;
    for file in &manifest.files {
        if file.kind == "launch-path" {
            continue;
        }
        let destination = PathBuf::from(&file.source);
        if !file.present {
            if destination.exists() {
                fs::remove_file(&destination).map_err(|error| {
                    format!("移除多模型新增文件失败 {}: {error}", destination.display())
                })?;
            }
            continue;
        }
        let backup = root.join(&file.backup);
        if !backup.exists() {
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建恢复目标目录失败 {}: {error}", parent.display())
            })?;
        }
        fs::copy(&backup, &destination).map_err(|error| {
            format!(
                "恢复多模型文件失败 {} -> {}: {error}",
                backup.display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn latest_restore_point(workspace: &Path) -> Option<PathBuf> {
    let root = workspace.join("restore-points");
    let mut points = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(MANIFEST_FILE_NAME).is_file())
        .collect::<Vec<_>>();
    points.sort();
    points.pop()
}

fn read_manifest(root: &Path) -> Result<RestoreManifest, String> {
    let path = root.join(MANIFEST_FILE_NAME);
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("读取多模型恢复点失败 {}: {error}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("解析多模型恢复点失败 {}: {error}", path.display()))
}

fn current_restore_point(app: &AppHandle, workspace: &Path) -> Option<PathBuf> {
    store::load_store(app)
        .ok()
        .and_then(|store| store.settings.codex_multi_model_restore_point.map(PathBuf::from))
        .or_else(|| latest_restore_point(workspace))
}

fn restore_point_files_only(root: &Path) -> Result<(), String> {
    let manifest = read_manifest(root)?;
    restore_manifest_files(&manifest, root)
}

fn write_manifest(root: &Path, manifest: &RestoreManifest) -> Result<(), String> {
    let path = root.join(MANIFEST_FILE_NAME);
    let serialized = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("序列化多模型恢复点失败: {error}"))?;
    fs::write(&path, serialized)
        .map_err(|error| format!("写入多模型恢复点失败 {}: {error}", path.display()))
}

fn restore_file_present_by_default() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("codexdeck-{name}-{}", Uuid::new_v4()))
    }

    fn create_controlled_copy_fixture(workspace: &Path) -> PathBuf {
        let app_root = controlled_current_app_root(workspace);
        let resources_dir = app_root.join("resources");
        fs::create_dir_all(&resources_dir).expect("create controlled resources dir");
        fs::write(app_root.join("Codex.exe"), b"test exe").expect("write controlled exe");
        fs::write(resources_dir.join("app.asar"), b"test asar").expect("write controlled asar");
        fs::write(
            controlled_copy_marker_path(&app_root),
            r#"{"sourceAppRoot":"fixtures/OpenAI.Codex_test/app"}"#,
        )
        .expect("write controlled marker");
        app_root
    }

    #[test]
    fn inactive_status_does_not_restore_controlled_copy_from_disk() {
        let workspace = temp_workspace("inactive-restore");
        let app_root = create_controlled_copy_fixture(&workspace);
        let controlled_exe = app_root.join("Codex.exe");

        let mut settings = AppSettings {
            codex_multi_model_mode_enabled: false,
            codex_multi_model_status: Some("reset".to_string()),
            codex_multi_model_workspace: Some(workspace.to_string_lossy().to_string()),
            codex_launch_path: Some(controlled_exe.to_string_lossy().to_string()),
            ..AppSettings::default()
        };

        let changed = restore_registered_controlled_copy_from_disk(&mut settings, &workspace);

        assert!(!changed);
        assert!(!settings.codex_multi_model_mode_enabled);
        assert_eq!(settings.codex_multi_model_status.as_deref(), Some("reset"));
        assert!(settings.codex_multi_model_controlled_exe_path.is_none());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn active_status_restores_controlled_copy_from_disk() {
        let workspace = temp_workspace("active-restore");
        let app_root = create_controlled_copy_fixture(&workspace);
        let expected_exe = app_root.join("Codex.exe");

        let mut settings = AppSettings {
            codex_multi_model_mode_enabled: true,
            codex_multi_model_status: Some("enabled".to_string()),
            codex_multi_model_workspace: Some(workspace.to_string_lossy().to_string()),
            ..AppSettings::default()
        };

        let changed = restore_registered_controlled_copy_from_disk(&mut settings, &workspace);

        assert!(changed);
        assert!(settings.codex_multi_model_mode_enabled);
        assert_eq!(
            settings.codex_multi_model_status.as_deref(),
            Some("enabled")
        );
        assert_eq!(
            settings.codex_multi_model_controlled_exe_path.as_deref(),
            Some(expected_exe.to_string_lossy().as_ref())
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn codex_package_version_key_orders_numeric_segments() {
        let old = PathBuf::from("fixtures")
            .join("OpenAI.Codex_26.623.8305.0_x64__fixture")
            .join("app");
        let new = PathBuf::from("fixtures")
            .join("OpenAI.Codex_26.623.19656.0_x64__fixture")
            .join("app");

        assert!(codex_app_root_version_key(&new) > codex_app_root_version_key(&old));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn registry_query_parser_extracts_codex_package_names() {
        let raw = r#"
HKEY_CURRENT_USER\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages\OpenAI.Codex_26.623.19656.0_x64__2p2nqsd0c76g0
End of search: 1 match(es) found.
"#;

        assert_eq!(
            parse_codex_package_names_from_registry_query(raw),
            vec!["OpenAI.Codex_26.623.19656.0_x64__2p2nqsd0c76g0".to_string()]
        );
    }
}
