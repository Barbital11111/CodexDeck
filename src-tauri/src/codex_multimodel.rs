use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;
use uuid::Uuid;

use crate::app_paths;
use crate::codex_model_picker_patch::{self, PatchRequest};
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
const CONTROLLED_GENERATED_CANDIDATE_DIR_PREFIX: &str = "candidate-";
const CONTROLLED_PREVIOUS_DIR_NAME: &str = "previous";
const CONTROLLED_APP_DIR_NAME: &str = "app";
const CONTROLLED_LEGACY_BACKUPS_DIR_NAME: &str = "backups";
const STAGED_CODEX_DIR_NAME: &str = "staged-codex";
const PATCH_BACKUPS_DIR_NAME: &str = "patch-backups";
const PATCH_VALIDATION_DIR_PREFIX: &str = "patch-validation-";
const PREVIOUS_STASH_DIR_PREFIX: &str = ".previous-stash-";
const PATCH_BACKUP_KEEP_LATEST: usize = 1;
const PATCH_STATE_FILE_NAME: &str = "model-picker-patch-state.json";
const MODEL_PICKER_PATCH_VERSION: &str = "model-picker-v21";
const CANDIDATE_PROMOTION_PENDING_PREFIX: &str = "candidate-promotion-pending:";
const PROMOTION_FILE_OPERATION_ATTEMPTS: usize = 51;
const PROMOTION_FILE_OPERATION_RETRY_DELAY: Duration = Duration::from_millis(200);
const UNSUPPORTED_MESSAGE: &str = "当前 Codex 桌面端版本暂不适配多模型增强模式，已停止启动。";
const PATCH_BOOTSTRAP_MESSAGE: &str = "多模型增强脚本启动失败，已停止启动。";
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
    patch_version: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FileFingerprint {
    size: u64,
    modified_seconds: u64,
    modified_nanos: u32,
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

fn lock_controlled_copy_lifecycle() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| {
            log::warn!("多模型受控副本互斥锁曾发生 panic，已恢复锁并继续。");
            poisoned.into_inner()
        })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControlledCopyMarker {
    #[serde(default)]
    schema_version: Option<u8>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    patch_version: Option<String>,
    #[serde(default)]
    source_app_root: Option<String>,
    #[serde(default)]
    source_asar_hash: Option<String>,
    #[serde(default)]
    source_codex_version: Option<String>,
    #[serde(default)]
    controlled_asar_hash: Option<String>,
    #[serde(default)]
    source_asar_fingerprint: Option<FileFingerprint>,
    #[serde(default)]
    controlled_asar_fingerprint: Option<FileFingerprint>,
    #[serde(default)]
    controlled_app_root: Option<String>,
    #[serde(default)]
    controlled_exe_path: Option<String>,
    #[serde(default)]
    controlled_app_asar_path: Option<String>,
}

pub(crate) fn workspace_dir() -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var(WORKSPACE_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let exe =
        std::env::current_exe().map_err(|error| format!("无法获取 CodexDeck 安装路径: {error}"))?;
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
    let _lifecycle_guard = lock_controlled_copy_lifecycle();
    let workspace = workspace_dir()?;
    fs::create_dir_all(&workspace)
        .map_err(|error| format!("创建多模型工作区失败 {}: {error}", workspace.display()))?;
    schedule_stale_multimodel_artifact_cleanup(workspace.clone());
    let restore_point = create_restore_point(app, &workspace)?;
    let controlled_copy = rebuild_controlled_codex_copy(app, &workspace).map_err(|error| {
        let _ = restore_point_files_only(&restore_point);
        error
    })?;

    register_controlled_copy(app, &workspace, &controlled_copy, "controlled-copy-ready")?;
    let mut store = store::load_store(app)?;
    store.settings.codex_multi_model_restore_point =
        Some(restore_point.to_string_lossy().to_string());
    store::save_store(app, &store)?;
    schedule_stale_multimodel_artifact_cleanup(workspace.clone());

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
    let _lifecycle_guard = lock_controlled_copy_lifecycle();
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
    let _lifecycle_guard = lock_controlled_copy_lifecycle();
    let started_at = Instant::now();
    let workspace = workspace_dir()?;
    fs::create_dir_all(&workspace)
        .map_err(|error| format!("创建多模型工作区失败 {}: {error}", workspace.display()))?;
    log_multimodel_launch_phase(&started_at, "workspace");

    let restore_point = current_restore_point(app, &workspace).ok_or_else(|| {
        "多模型模式缺少稳定恢复点，已停止启动。请先在设置中重新开启多模型模式。".to_string()
    })?;
    reconcile_controlled_copy_registration(app, &workspace)?;
    log_multimodel_launch_phase(&started_at, "registration");
    let (mut controlled_copy, recovered) =
        recover_controlled_copy(current_controlled_copy(app, &workspace), || {
            rebuild_controlled_codex_copy(app, &workspace)
        })
        .map_err(|error| {
            let (status, restore_files) = prepare_failure_policy(&error);
            let _ = mark_status(app, status, Some(&workspace), Some(&restore_point));
            if restore_files {
                let _ = restore_point_files_only(&restore_point);
            }
            error
        })?;
    if recovered {
        log::warn!("受控 Codex 注册或副本不可用，已从官方来源自动重建。");
        register_controlled_copy(app, &workspace, &controlled_copy, "recovered")?;
    }
    log_multimodel_launch_phase(&started_at, "current-copy");
    let explicit_source_app_root =
        resolve_explicit_source_app_root_for_controlled_copy(app, &workspace)?
            .map(|(root, _exe)| root);
    let mut fast_patch_state = fast_launch_patch_state_for_expected_source(
        &controlled_copy,
        explicit_source_app_root.as_deref(),
    );
    if fast_patch_state.is_some() {
        log::info!("受控 Codex 来源与 patch 指纹未变化，跳过来源扫描、完整 hash、复制和 patch。");
    } else {
        controlled_copy =
            maybe_rebuild_controlled_copy_for_source_update(app, &workspace, controlled_copy)
                .map_err(|error| {
                    let (status, restore_files) = prepare_failure_policy(&error);
                    let _ = mark_status(app, status, Some(&workspace), Some(&restore_point));
                    if restore_files {
                        let _ = restore_point_files_only(&restore_point);
                    }
                    error
                })?;
    }
    log_multimodel_launch_phase(&started_at, "source-check");
    if let Err(error) = sync_codex_multi_model_assets() {
        let _ = mark_status(app, "failed", Some(&workspace), Some(&restore_point));
        let _ = restore_point_files_only(&restore_point);
        return Err(error);
    }
    log_multimodel_launch_phase(&started_at, "asset-sync");

    let state = match fast_patch_state.take() {
        Some(state) => state,
        None => run_patch_script(app, &workspace, &controlled_copy).map_err(|error| {
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
        })?,
    };
    log_multimodel_launch_phase(&started_at, "patch");
    let launch_path = launch_path_from_patch_state(&state).unwrap_or_else(|| {
        controlled_copy
            .controlled_exe_path
            .to_string_lossy()
            .to_string()
    });
    if !path_is_within_windows_like(&launch_path, &workspace.join(CONTROLLED_COPY_DIR_NAME)) {
        let _ = mark_status(app, "unsupported", Some(&workspace), Some(&restore_point));
        return Err(format!(
            "{UNSUPPORTED_MESSAGE}\npatch 状态返回了非受控启动路径。"
        ));
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
    fs::create_dir_all(&agents_dir).map_err(|error| {
        format!(
            "创建 Codex agents 目录失败 {}: {error}",
            agents_dir.display()
        )
    })?;
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
    for entry in fs::read_dir(agents_dir).map_err(|error| {
        format!(
            "读取 Codex agents 目录失败 {}: {error}",
            agents_dir.display()
        )
    })? {
        let entry = entry.map_err(|error| format!("读取 Codex agent 文件失败: {error}"))?;
        let path = entry.path();
        if path.is_file() && is_codexdeck_managed_agent(&path) {
            fs::remove_file(&path).map_err(|error| {
                format!("移除旧 CodexDeck agent 失败 {}: {error}", path.display())
            })?;
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
    rebuild_controlled_codex_copy_from_source(app, workspace, &source)
}

fn maybe_rebuild_controlled_copy_for_source_update(
    app: &AppHandle,
    workspace: &Path,
    current: ControlledCodexCopy,
) -> Result<ControlledCodexCopy, String> {
    let current_patch_is_valid = cached_patch_state_for_controlled_copy(&current).is_some();
    let source = match resolve_source_codex_snapshot(app, workspace) {
        Ok(source) => source,
        Err(error) => {
            if current_patch_is_valid {
                log::warn!("检测官方 Codex 版本失败，继续使用旧稳定受控副本: {error}");
                mark_status(app, "source-check-unavailable", Some(workspace), None)?;
                return Ok(current);
            }
            return Err(format!(
                "当前受控副本需要升级到 {MODEL_PICKER_PATCH_VERSION}，但无法读取官方 Codex 来源: {error}"
            ));
        }
    };

    let source_changed = source_snapshot_requires_rebuild(&current, &source);
    if !source_changed && current_patch_is_valid {
        return Ok(current);
    }

    if source_changed {
        log::info!(
            "检测到 Codex 来源已更新，尝试重建多模型受控副本: old={}, new={}",
            current.source_asar_hash,
            source.asar_hash
        );
    } else {
        log::info!(
            "检测到受控副本 patch 版本过旧，尝试通过 candidate 升级到 {MODEL_PICKER_PATCH_VERSION}。"
        );
    }
    match rebuild_controlled_codex_copy_from_source(app, workspace, &source) {
        Ok(rebuilt) => {
            register_controlled_copy(app, workspace, &rebuilt, "updated")?;
            Ok(rebuilt)
        }
        Err(error) => {
            if current_patch_is_valid && cached_patch_state_for_controlled_copy(&current).is_some()
            {
                log::warn!("重建新版多模型受控副本失败，继续使用旧稳定副本: {error}");
                mark_status(app, "fallback-previous", Some(workspace), None)?;
                Ok(current)
            } else {
                Err(format!(
                    "升级多模型受控副本到 {MODEL_PICKER_PATCH_VERSION} 失败，且旧副本未通过回滚后校验: {error}"
                ))
            }
        }
    }
}

fn source_snapshot_requires_rebuild(
    current: &ControlledCodexCopy,
    source: &SourceCodexSnapshot,
) -> bool {
    current.source_asar_hash != source.asar_hash
        || !canonical_patch_paths_match(&current.source_app_root, &source.app_root)
}

fn rebuild_controlled_codex_copy_from_source(
    app: &AppHandle,
    workspace: &Path,
    source: &SourceCodexSnapshot,
) -> Result<ControlledCodexCopy, String> {
    if let Some(candidate) = ready_candidate_controlled_copy_for_source(workspace, source)? {
        log::info!(
            "复用已完成 patch 的候选 Codex 副本并重新尝试晋级: {}",
            candidate.controlled_app_root.display()
        );
        return promote_candidate_controlled_copy(workspace, &candidate)
            .map_err(candidate_promotion_pending_error);
    }

    let candidate = prepare_candidate_controlled_copy(workspace, source)?;
    let state = run_patch_script(app, workspace, &candidate)?;
    validate_patch_state_for_controlled_copy(&state, &candidate)?;
    promote_candidate_controlled_copy(workspace, &candidate)
        .map_err(candidate_promotion_pending_error)
}

fn candidate_promotion_pending_error(error: String) -> String {
    format!("{CANDIDATE_PROMOTION_PENDING_PREFIX} {error}")
}

fn is_candidate_promotion_pending_error(error: &str) -> bool {
    error.contains(CANDIDATE_PROMOTION_PENDING_PREFIX)
}

fn prepare_failure_policy(error: &str) -> (&'static str, bool) {
    if is_candidate_promotion_pending_error(error) {
        ("promotion-pending", false)
    } else {
        ("failed", true)
    }
}

fn ready_candidate_controlled_copy_for_source(
    workspace: &Path,
    source: &SourceCodexSnapshot,
) -> Result<Option<ControlledCodexCopy>, String> {
    let controlled_root = workspace.join(CONTROLLED_COPY_DIR_NAME);
    if !controlled_root.is_dir() {
        return Ok(None);
    }

    let mut candidate_roots = Vec::new();
    let fixed_candidate = controlled_root.join(CONTROLLED_CANDIDATE_DIR_NAME);
    if fixed_candidate.is_dir() {
        candidate_roots.push(fixed_candidate);
    }
    for entry in fs::read_dir(&controlled_root).map_err(|error| {
        format!(
            "读取受控 Codex 候选目录失败 {}: {error}",
            controlled_root.display()
        )
    })? {
        let entry = entry.map_err(|error| format!("读取受控 Codex 候选条目失败: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if entry
            .file_type()
            .map_err(|error| format!("检查受控 Codex 候选条目失败: {error}"))?
            .is_dir()
            && is_generated_candidate_dir_name(&name)
        {
            candidate_roots.push(entry.path());
        }
    }

    for candidate_root in candidate_roots {
        let controlled_app_root = candidate_root.join(CONTROLLED_APP_DIR_NAME);
        if ensure_safe_controlled_copy_target(workspace, &controlled_app_root).is_err() {
            continue;
        }
        let controlled_exe_path = find_codex_launch_exe_in_app_root(&controlled_app_root);
        let controlled_app_asar_path = controlled_app_root.join("resources").join("app.asar");
        let patch_state_path = candidate_root.join(PATCH_STATE_FILE_NAME);
        if !controlled_exe_path.is_file()
            || !controlled_app_asar_path.is_file()
            || !patch_state_path.is_file()
        {
            continue;
        }

        let marker = match read_controlled_copy_marker(&controlled_app_root) {
            Ok(marker) => marker,
            Err(_) => continue,
        };
        if marker.source_asar_hash.as_deref() != Some(source.asar_hash.as_str())
            || marker.source_codex_version.as_deref() != source.codex_version.as_deref()
            || marker.source_app_root.as_deref().is_none_or(|path| {
                normalize_windows_like_path(path)
                    != normalize_windows_like_path(&source.app_root.to_string_lossy())
            })
        {
            continue;
        }

        let controlled_copy = ControlledCodexCopy {
            source_app_root: source.app_root.clone(),
            controlled_app_root,
            controlled_exe_path,
            controlled_app_asar_path,
            source_asar_hash: source.asar_hash.clone(),
            source_codex_version: source.codex_version.clone(),
            patch_state_path,
        };
        let raw = match fs::read_to_string(&controlled_copy.patch_state_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let state = match serde_json::from_str::<PatchState>(&raw) {
            Ok(state) => state,
            Err(_) => continue,
        };
        if !matches!(state.status.as_str(), "patched" | "already-patched")
            || validate_patch_state_for_controlled_copy(&state, &controlled_copy).is_err()
            || state.app_asar_path.as_deref().is_none_or(|path| {
                normalize_windows_like_path(path)
                    != normalize_windows_like_path(
                        &controlled_copy.controlled_app_asar_path.to_string_lossy(),
                    )
            })
        {
            continue;
        }
        let actual_hash = sha256_file(&controlled_copy.controlled_app_asar_path)?;
        if state.patched_asar_hash.as_deref() != Some(actual_hash.as_str()) {
            continue;
        }
        return Ok(Some(controlled_copy));
    }

    Ok(None)
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
    let preferred_candidate_root = controlled_root.join(CONTROLLED_CANDIDATE_DIR_NAME);
    let candidate_root = if preferred_candidate_root.exists() {
        let generated = controlled_root.join(format!(
            "{CONTROLLED_GENERATED_CANDIDATE_DIR_PREFIX}{}",
            Uuid::new_v4()
        ));
        log::info!(
            "固定 candidate 已存在，改用隔离候选代次，保留可能仍在运行的旧副本: {}",
            preferred_candidate_root.display()
        );
        generated
    } else {
        preferred_candidate_root
    };
    let controlled_app_root = candidate_root.join(CONTROLLED_APP_DIR_NAME);
    let patch_state_path = candidate_root.join(PATCH_STATE_FILE_NAME);
    ensure_safe_controlled_copy_target(workspace, &controlled_app_root)?;
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
        None,
    )?;

    Ok(ControlledCodexCopy {
        source_app_root: source.app_root.clone(),
        controlled_app_root,
        controlled_exe_path,
        controlled_app_asar_path,
        source_asar_hash: source.asar_hash.clone(),
        source_codex_version: source.codex_version.clone(),
        patch_state_path,
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

    let stashed_previous_app_root = if previous_app_root.exists() {
        let stashed = controlled_root
            .join(format!(".previous-stash-{}", Uuid::new_v4()))
            .join(CONTROLLED_APP_DIR_NAME);
        ensure_safe_controlled_copy_target(workspace, &stashed)?;
        if let Some(parent) = stashed.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 previous 暂存目录失败 {}: {error}", parent.display())
            })?;
        }
        rename_for_candidate_promotion(&previous_app_root, &stashed).map_err(|error| {
            format!(
                "暂存旧 previous Codex 副本失败 {} -> {}: {error}",
                previous_app_root.display(),
                stashed.display()
            )
        })?;
        Some(stashed)
    } else {
        None
    };
    let mut moved_current_to_previous = false;
    if current_app_root.exists() {
        if let Some(parent) = previous_app_root.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 previous Codex 目录失败 {}: {error}", parent.display())
            })?;
        }
        if let Err(error) = rename_for_candidate_promotion(&current_app_root, &previous_app_root) {
            let rollback = rollback_promotion_layout(
                workspace,
                &previous_app_root,
                &current_app_root,
                false,
                stashed_previous_app_root.as_deref(),
            )
            .err()
            .unwrap_or_else(|| "已恢复原 previous".to_string());
            return Err(format!(
                "保留旧稳定 Codex 副本失败 {} -> {}: {error}; 回滚结果: {rollback}",
                current_app_root.display(),
                previous_app_root.display()
            ));
        }
        moved_current_to_previous = true;
        let previous_exe_path = find_codex_launch_exe_in_app_root(&previous_app_root);
        let previous_asar_path = previous_app_root.join("resources").join("app.asar");
        if let Err(error) = rewrite_controlled_copy_marker_paths(
            workspace,
            &previous_app_root,
            &previous_exe_path,
            &previous_asar_path,
            None,
            false,
        ) {
            let rollback = rollback_promotion_layout(
                workspace,
                &previous_app_root,
                &current_app_root,
                moved_current_to_previous,
                stashed_previous_app_root.as_deref(),
            );
            return Err(format!(
                "更新 previous Codex marker 失败: {error}; 回滚结果: {}",
                rollback
                    .err()
                    .unwrap_or_else(|| "已恢复 current".to_string())
            ));
        }
    }
    if let Some(parent) = current_app_root.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            let rollback = rollback_promotion_layout(
                workspace,
                &previous_app_root,
                &current_app_root,
                moved_current_to_previous,
                stashed_previous_app_root.as_deref(),
            )
            .err()
            .unwrap_or_else(|| "已恢复晋级前目录布局".to_string());
            return Err(format!(
                "创建 current Codex 目录失败 {}: {error}; 回滚结果: {rollback}",
                parent.display()
            ));
        }
    }
    if let Err(error) =
        rename_for_candidate_promotion(&candidate.controlled_app_root, &current_app_root)
    {
        let rollback = rollback_promotion_layout(
            workspace,
            &previous_app_root,
            &current_app_root,
            moved_current_to_previous,
            stashed_previous_app_root.as_deref(),
        )
        .err()
        .unwrap_or_else(|| "已恢复晋级前目录布局".to_string());
        return Err(format!(
            "晋级候选 Codex 副本失败 {} -> {}: {error}; 回滚结果: {rollback}",
            candidate.controlled_app_root.display(),
            current_app_root.display()
        ));
    }

    let controlled_exe_path = relative_path_between(
        &candidate.controlled_app_root,
        &candidate.controlled_exe_path,
    )
    .map(|relative| current_app_root.join(relative))
    .unwrap_or_else(|| find_codex_launch_exe_in_app_root(&current_app_root));
    let controlled_app_asar_path = current_app_root.join("resources").join("app.asar");
    let metadata_result = (|| {
        rewrite_controlled_copy_marker_paths(
            workspace,
            &current_app_root,
            &controlled_exe_path,
            &controlled_app_asar_path,
            Some(MODEL_PICKER_PATCH_VERSION),
            true,
        )?;
        rewrite_promoted_patch_state_paths(
            workspace,
            candidate,
            &current_app_root,
            &controlled_exe_path,
            &controlled_app_asar_path,
        )
    })();
    if let Err(error) = metadata_result {
        let rollback = rollback_promoted_candidate(
            workspace,
            candidate,
            &previous_app_root,
            &current_app_root,
            moved_current_to_previous,
            stashed_previous_app_root.as_deref(),
        )
        .err()
        .unwrap_or_else(|| "已恢复旧 current，并保留 candidate".to_string());
        return Err(format!(
            "更新晋级后的 Codex 元数据失败: {error}; 回滚结果: {rollback}"
        ));
    }
    finalize_stashed_previous_copy(
        &previous_app_root,
        stashed_previous_app_root.as_deref(),
        moved_current_to_previous,
    );
    if let Err(error) = fs::remove_file(&candidate.patch_state_path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!(
                "清理已晋级 candidate patch 状态失败 {}: {error}",
                candidate.patch_state_path.display()
            );
        }
    }
    if let Some(candidate_root) = candidate.patch_state_path.parent() {
        remove_empty_directory_best_effort(candidate_root);
    }

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

fn rename_for_candidate_promotion(source: &Path, destination: &Path) -> std::io::Result<()> {
    retry_transient_promotion_file_operation(
        PROMOTION_FILE_OPERATION_ATTEMPTS,
        || std::thread::sleep(PROMOTION_FILE_OPERATION_RETRY_DELAY),
        || fs::rename(source, destination),
    )
}

fn retry_transient_promotion_file_operation<T, Wait, Operation>(
    max_attempts: usize,
    mut wait: Wait,
    mut operation: Operation,
) -> std::io::Result<T>
where
    Wait: FnMut(),
    Operation: FnMut() -> std::io::Result<T>,
{
    let max_attempts = max_attempts.max(1);
    for attempt in 1..=max_attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempt < max_attempts && is_transient_promotion_file_error(&error) => {
                wait();
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("promotion retry loop always returns")
}

fn is_transient_promotion_file_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

fn rewrite_promoted_patch_state_paths(
    workspace: &Path,
    candidate: &ControlledCodexCopy,
    controlled_app_root: &Path,
    controlled_exe_path: &Path,
    controlled_app_asar_path: &Path,
) -> Result<(), String> {
    let raw = fs::read_to_string(&candidate.patch_state_path).map_err(|error| {
        format!(
            "读取 candidate patch 状态失败 {}: {error}",
            candidate.patch_state_path.display()
        )
    })?;
    let mut state = serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| {
        format!(
            "解析 candidate patch 状态失败 {}: {error}",
            candidate.patch_state_path.display()
        )
    })?;
    let state_object = state
        .as_object_mut()
        .ok_or_else(|| "candidate patch 状态必须是 JSON object。".to_string())?;
    if state_object
        .get("patchVersion")
        .and_then(serde_json::Value::as_str)
        != Some(MODEL_PICKER_PATCH_VERSION)
    {
        return Err(format!(
            "candidate patch 状态版本不匹配，期望 {MODEL_PICKER_PATCH_VERSION}。"
        ));
    }
    let state_asar_path = state_object
        .get("appAsarPath")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "candidate patch 状态缺少 appAsarPath。".to_string())?;
    if normalize_windows_like_path(state_asar_path)
        != normalize_windows_like_path(&candidate.controlled_app_asar_path.to_string_lossy())
    {
        return Err("candidate patch 状态中的 app.asar 路径不匹配。".to_string());
    }

    state_object.insert(
        "appAsarPath".to_string(),
        serde_json::Value::String(controlled_app_asar_path.to_string_lossy().to_string()),
    );
    state_object.insert(
        "controlledAppRoot".to_string(),
        serde_json::Value::String(controlled_app_root.to_string_lossy().to_string()),
    );
    state_object.insert(
        "launchPath".to_string(),
        serde_json::Value::String(controlled_exe_path.to_string_lossy().to_string()),
    );

    let mut serialized = serde_json::to_vec_pretty(&state)
        .map_err(|error| format!("序列化晋级后的 patch 状态失败: {error}"))?;
    serialized.push(b'\n');
    write_patch_state_atomically(&workspace.join(PATCH_STATE_FILE_NAME), &serialized)
}

fn write_patch_state_atomically(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析 patch 状态目录 {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 patch 状态目录失败 {}: {error}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(PATCH_STATE_FILE_NAME);
    let temp_path = parent.join(format!(".{file_name}.tmp-{}", Uuid::new_v4()));
    let backup_path = parent.join(format!(".{file_name}.previous-{}", Uuid::new_v4()));

    let result = (|| -> Result<(), String> {
        let mut temp_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|error| format!("创建临时 patch 状态失败 {}: {error}", temp_path.display()))?;
        temp_file
            .write_all(contents)
            .map_err(|error| format!("写入临时 patch 状态失败 {}: {error}", temp_path.display()))?;
        temp_file
            .sync_all()
            .map_err(|error| format!("刷新临时 patch 状态失败 {}: {error}", temp_path.display()))?;
        drop(temp_file);

        let had_previous = path.exists();
        replace_patch_state_file(&temp_path, path, &backup_path)?;
        if had_previous {
            if let Err(error) = fs::remove_file(&backup_path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!(
                        "清理旧 patch 状态备份失败 {}: {error}",
                        backup_path.display()
                    );
                }
            }
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[cfg(target_os = "windows")]
fn replace_patch_state_file(
    source: &Path,
    destination: &Path,
    backup: &Path,
) -> Result<(), String> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACEFILE_WRITE_THROUGH};

    if !destination.exists() {
        return fs::rename(source, destination).map_err(|error| {
            format!(
                "写入 patch 状态失败 {} -> {}: {error}",
                source.display(),
                destination.display()
            )
        });
    }

    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let backup_wide = backup
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        ReplaceFileW(
            PCWSTR(destination_wide.as_ptr()),
            PCWSTR(source_wide.as_ptr()),
            PCWSTR(backup_wide.as_ptr()),
            REPLACEFILE_WRITE_THROUGH,
            None,
            None,
        )
    }
    .map_err(|error| {
        format!(
            "原子替换 patch 状态失败 {} -> {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(not(target_os = "windows"))]
fn replace_patch_state_file(
    source: &Path,
    destination: &Path,
    _backup: &Path,
) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| {
        format!(
            "原子替换 patch 状态失败 {} -> {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

fn rollback_promoted_candidate(
    workspace: &Path,
    candidate: &ControlledCodexCopy,
    previous_app_root: &Path,
    current_app_root: &Path,
    moved_current_to_previous: bool,
    stashed_previous_app_root: Option<&Path>,
) -> Result<(), String> {
    if current_app_root.exists() {
        if candidate.controlled_app_root.exists() {
            return Err(format!(
                "candidate 回滚目标已存在，未覆盖: {}",
                candidate.controlled_app_root.display()
            ));
        }
        if let Some(parent) = candidate.controlled_app_root.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 candidate 回滚目录失败 {}: {error}", parent.display())
            })?;
        }
        rename_for_candidate_promotion(current_app_root, &candidate.controlled_app_root).map_err(
            |error| {
                format!(
                    "保留失败 candidate 副本失败 {} -> {}: {error}",
                    current_app_root.display(),
                    candidate.controlled_app_root.display()
                )
            },
        )?;
    }
    rollback_promotion_layout(
        workspace,
        previous_app_root,
        current_app_root,
        moved_current_to_previous,
        stashed_previous_app_root,
    )
}

fn rollback_promotion_layout(
    workspace: &Path,
    previous_app_root: &Path,
    current_app_root: &Path,
    moved_current_to_previous: bool,
    stashed_previous_app_root: Option<&Path>,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if moved_current_to_previous {
        if let Err(error) =
            restore_previous_controlled_copy(workspace, previous_app_root, current_app_root)
        {
            errors.push(error);
        }
    }
    if let Some(stashed) = stashed_previous_app_root {
        if let Err(error) = restore_stashed_previous_copy(stashed, previous_app_root) {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn restore_stashed_previous_copy(
    stashed_previous_app_root: &Path,
    previous_app_root: &Path,
) -> Result<(), String> {
    if previous_app_root.exists() {
        return Err(format!(
            "previous 恢复目标已存在，未覆盖: {}",
            previous_app_root.display()
        ));
    }
    if let Some(parent) = previous_app_root.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 previous 恢复目录失败 {}: {error}", parent.display()))?;
    }
    rename_for_candidate_promotion(stashed_previous_app_root, previous_app_root).map_err(
        |error| {
            format!(
                "恢复暂存 previous Codex 副本失败 {} -> {}: {error}",
                stashed_previous_app_root.display(),
                previous_app_root.display()
            )
        },
    )?;
    if let Some(stash_root) = stashed_previous_app_root.parent() {
        let _ = fs::remove_dir(stash_root);
    }
    Ok(())
}

fn finalize_stashed_previous_copy(
    previous_app_root: &Path,
    stashed_previous_app_root: Option<&Path>,
    moved_current_to_previous: bool,
) {
    let Some(stashed) = stashed_previous_app_root else {
        return;
    };
    if moved_current_to_previous {
        let cleanup_root = stashed.parent().unwrap_or(stashed);
        log::info!(
            "已替换的 previous 暂存副本将由运行进程感知清理器处理: {}",
            cleanup_root.display()
        );
    } else if let Err(error) = restore_stashed_previous_copy(stashed, previous_app_root) {
        log::warn!(
            "恢复未替换的 previous 暂存副本失败 {}: {error}",
            stashed.display()
        );
    }
}

fn restore_previous_controlled_copy(
    workspace: &Path,
    previous_app_root: &Path,
    current_app_root: &Path,
) -> Result<(), String> {
    if current_app_root.exists() {
        return Err(format!(
            "回滚目标已存在，未覆盖: {}",
            current_app_root.display()
        ));
    }
    rename_for_candidate_promotion(previous_app_root, current_app_root).map_err(|error| {
        format!(
            "恢复 previous Codex 副本失败 {} -> {}: {error}",
            previous_app_root.display(),
            current_app_root.display()
        )
    })?;
    let controlled_exe_path = find_codex_launch_exe_in_app_root(current_app_root);
    let controlled_app_asar_path = current_app_root.join("resources").join("app.asar");
    rewrite_controlled_copy_marker_paths(
        workspace,
        current_app_root,
        &controlled_exe_path,
        &controlled_app_asar_path,
        None,
        false,
    )
}

fn resolve_source_app_root_for_controlled_copy(
    app: &AppHandle,
    workspace: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    if let Some(source) = resolve_explicit_source_app_root_for_controlled_copy(app, workspace)? {
        return Ok(source);
    }

    let mut candidates = Vec::new();
    if let Ok(running_app_roots) = resolve_running_windows_codex_app_dirs() {
        candidates.extend(running_app_roots);
    }
    candidates.extend(resolve_windows_apps_codex_app_dirs());

    for candidate in candidates {
        if let Some((root, exe)) = validated_source_app_root(&candidate, workspace) {
            return Ok((root, exe));
        }
    }

    Err(
        "无法找到可复制的 Codex 桌面端。请先确认当前 Codex 可正常启动，或在设置中指定 Codex.exe。"
            .to_string(),
    )
}

fn resolve_explicit_source_app_root_for_controlled_copy(
    app: &AppHandle,
    workspace: &Path,
) -> Result<Option<(PathBuf, PathBuf)>, String> {
    let store = store::load_store(app)?;
    let mut candidates = Vec::new();
    if let Some(path) = store.settings.codex_launch_path.as_deref() {
        candidates.push(PathBuf::from(path));
    }
    if let Some(path) = crate::settings_service::dev_controlled_codex_launch_path() {
        candidates.push(PathBuf::from(path));
    }

    for candidate in candidates {
        if let Some(source) = validated_source_app_root(&candidate, workspace) {
            return Ok(Some(source));
        }
    }

    Ok(None)
}

fn validated_source_app_root(candidate: &Path, workspace: &Path) -> Option<(PathBuf, PathBuf)> {
    if is_managed_codex_launch_path(&candidate.to_string_lossy(), Some(workspace)) {
        return None;
    }
    let (root, exe) = app_root_from_codex_launch_path(candidate)?;
    if path_is_within_windows_like(
        &root.to_string_lossy(),
        &workspace.join(CONTROLLED_COPY_DIR_NAME),
    ) {
        return None;
    }
    (root.join("resources").join("app.asar").is_file() && exe.is_file()).then_some((root, exe))
}

#[cfg(target_os = "windows")]
fn resolve_running_windows_codex_app_dirs() -> Result<Vec<PathBuf>, String> {
    let script = r#"$ErrorActionPreference = 'Stop'
Get-CimInstance Win32_Process -Filter "Name='Codex.exe' OR Name='ChatGPT.exe' OR Name='Codex Desktop.exe'" |
Select-Object ProcessId,Name,CommandLine,ExecutablePath |
ConvertTo-Json -Compress"#;
    let output = new_resolved_command("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(|error| format!("查询运行中的 Codex 进程失败: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("查询运行中的 Codex 进程失败，退出状态: {}", output.status)
        } else {
            format!("查询运行中的 Codex 进程失败: {stderr}")
        });
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Err(format!(
            "查询运行中的 Codex 进程产生错误输出，无法确认枚举完整性: {stderr}"
        ));
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    parse_running_windows_codex_app_dirs(&raw)
}

#[cfg(target_os = "windows")]
fn parse_running_windows_codex_app_dirs(raw: &str) -> Result<Vec<PathBuf>, String> {
    let value = serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|error| format!("解析运行中的 Codex 进程查询结果失败: {error}"))?;
    if value.is_null() {
        return Err("运行中的 Codex 进程查询返回 null，无法确认枚举完整性。".to_string());
    }
    let processes = value.as_array().cloned().unwrap_or_else(|| vec![value]);
    let mut out = Vec::new();
    for process in processes {
        let object = process
            .as_object()
            .ok_or_else(|| "运行中的 Codex 进程记录不是 JSON 对象。".to_string())?;
        let process_id = object
            .get("ProcessId")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "运行中的 Codex 进程记录缺少有效 ProcessId。".to_string())?;
        let name = object
            .get("Name")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("Codex 进程 {process_id} 缺少有效 Name。"))?;
        if !["ChatGPT.exe", "Codex.exe", "Codex Desktop.exe"]
            .iter()
            .any(|candidate| name.eq_ignore_ascii_case(candidate))
        {
            return Err(format!(
                "Codex 进程 {process_id} 返回了意外的映像名称 {name}。"
            ));
        }
        let command_line = object
            .get("CommandLine")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if command_line.is_some_and(|value| {
            value.contains("--type=") || value.to_ascii_lowercase().contains("app-server")
        }) {
            continue;
        }
        let path = object
            .get("ExecutablePath")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| command_line.and_then(executable_path_from_command_line))
            .ok_or_else(|| {
                format!(
                    "Codex 进程 {process_id} 的 ExecutablePath 与 CommandLine 均不可读，已拒绝清理。"
                )
            })?;
        let (root, _exe) = app_root_from_codex_launch_path(&path).ok_or_else(|| {
            format!(
                "无法从 Codex 进程 {process_id} 的路径解析桌面端根目录: {}",
                path.display()
            )
        })?;
        if name.eq_ignore_ascii_case("ChatGPT.exe") && !is_trusted_running_chatgpt_app_root(&root) {
            continue;
        }
        out.push(root);
    }
    Ok(dedupe_paths(out))
}

#[cfg(target_os = "windows")]
fn is_trusted_running_chatgpt_app_root(app_root: &Path) -> bool {
    crate::cli::is_windows_apps_codex_package_path(app_root)
        || controlled_copy_marker_path(app_root).is_file()
}

#[cfg(not(target_os = "windows"))]
fn resolve_running_windows_codex_app_dirs() -> Result<Vec<PathBuf>, String> {
    Ok(Vec::new())
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
            let mtime = path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok();
            (path, version_key, mtime)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.2.cmp(&left.2)));
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
                let preferred = find_codex_launch_exe_in_app_root(dir);
                let executable = if preferred.is_file() {
                    preferred
                } else {
                    path.to_path_buf()
                };
                return Some((dir.to_path_buf(), executable));
            }
            current = dir.parent();
        }
        return None;
    }

    for root in [
        path.to_path_buf(),
        path.join("app"),
        path.join("Application"),
    ] {
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
        PathBuf::from("ChatGPT.exe"),
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
    if !path_is_within_windows_like(&controlled_exe_path.to_string_lossy(), &controlled_app_root)
        || !path_is_within_windows_like(
            &controlled_app_asar_path.to_string_lossy(),
            &controlled_app_root,
        )
    {
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

fn recover_controlled_copy<F>(
    current: Result<ControlledCodexCopy, String>,
    rebuild: F,
) -> Result<(ControlledCodexCopy, bool), String>
where
    F: FnOnce() -> Result<ControlledCodexCopy, String>,
{
    match current {
        Ok(controlled_copy) => Ok((controlled_copy, false)),
        Err(current_error) => rebuild()
            .map(|controlled_copy| (controlled_copy, true))
            .map_err(|rebuild_error| {
                format!("{current_error}\n自动重建受控 Codex 副本失败: {rebuild_error}")
            }),
    }
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
    store.settings.codex_multi_model_source_app_root = Some(
        controlled_copy
            .source_app_root
            .to_string_lossy()
            .to_string(),
    );
    store.settings.codex_multi_model_controlled_app_root = Some(
        controlled_copy
            .controlled_app_root
            .to_string_lossy()
            .to_string(),
    );
    store.settings.codex_multi_model_controlled_exe_path = Some(
        controlled_copy
            .controlled_exe_path
            .to_string_lossy()
            .to_string(),
    );
    store.settings.codex_multi_model_controlled_app_asar_path = Some(
        controlled_copy
            .controlled_app_asar_path
            .to_string_lossy()
            .to_string(),
    );
    store.settings.codex_multi_model_patch_state_path = Some(
        controlled_copy
            .patch_state_path
            .to_string_lossy()
            .to_string(),
    );
    store.settings.codex_launch_path = Some(
        controlled_copy
            .controlled_exe_path
            .to_string_lossy()
            .to_string(),
    );
    store::save_store(app, &store)
}

fn reconcile_controlled_copy_registration(app: &AppHandle, workspace: &Path) -> Result<(), String> {
    let mut store = store::load_store(app)?;
    if restore_registered_controlled_copy_from_disk(&mut store.settings, workspace) {
        store::save_store(app, &store)?;
    }
    Ok(())
}

fn ensure_safe_controlled_copy_target(workspace: &Path, target: &Path) -> Result<(), String> {
    let root = workspace.join(CONTROLLED_COPY_DIR_NAME);
    if target
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "受控 Codex 副本路径包含 ..，已拒绝操作: {}",
            target.display()
        ));
    }
    if !path_is_within_windows_like(&target.to_string_lossy(), &root) {
        return Err(format!(
            "受控 Codex 副本路径不在多模型工作区内，已拒绝操作: {}",
            target.display()
        ));
    }
    ensure_path_has_no_link_like_ancestors(workspace, "多模型工作区")?;
    let workspace_metadata = fs::symlink_metadata(workspace)
        .map_err(|error| format!("检查多模型工作区路径失败 {}: {error}", workspace.display()))?;
    if metadata_is_link_like(&workspace_metadata) {
        return Err(format!(
            "多模型工作区是符号链接或 junction，已拒绝操作: {}",
            workspace.display()
        ));
    }
    let relative = target.strip_prefix(workspace).map_err(|_| {
        format!(
            "受控 Codex 副本路径无法相对到多模型工作区，已拒绝操作: {}",
            target.display()
        )
    })?;
    let mut current = workspace.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata_is_link_like(&metadata) => {
                return Err(format!(
                    "受控 Codex 副本路径包含符号链接或 junction，已拒绝操作: {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!(
                    "检查受控 Codex 副本路径失败 {}: {error}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

fn schedule_stale_multimodel_artifact_cleanup(workspace: PathBuf) {
    std::thread::spawn(move || {
        let _lifecycle_guard = lock_controlled_copy_lifecycle();
        match cleanup_stale_multimodel_artifacts(&workspace) {
            Ok(removed) if removed > 0 => {
                log::info!("已清理 {removed} 个旧多模型副本或备份产物。");
            }
            Ok(_) => {}
            Err(error) => log::warn!("清理旧多模型副本或备份产物失败: {error}"),
        }
    });
}

fn cleanup_stale_multimodel_artifacts(workspace: &Path) -> Result<usize, String> {
    let running_app_roots = resolve_running_windows_codex_app_dirs()
        .map_err(|error| format!("无法可靠枚举运行中的受控 Codex 副本，已跳过清理: {error}"))?;
    cleanup_stale_multimodel_artifacts_in_workspace(workspace, &running_app_roots, true)
}

#[cfg(test)]
fn cleanup_stale_multimodel_artifacts_after_running_root_lookup(
    workspace: &Path,
    running_app_roots: Result<Vec<PathBuf>, String>,
) -> Result<usize, String> {
    let running_app_roots = running_app_roots
        .map_err(|error| format!("无法可靠枚举运行中的受控 Codex 副本，已跳过清理: {error}"))?;
    cleanup_stale_multimodel_artifacts_in_workspace(workspace, &running_app_roots, false)
}

fn cleanup_stale_multimodel_artifacts_in_workspace(
    workspace: &Path,
    running_app_roots: &[PathBuf],
    refresh_running_roots_before_delete: bool,
) -> Result<usize, String> {
    if !workspace.exists() {
        return Ok(0);
    }
    ensure_safe_workspace_cleanup_target(workspace, &workspace.join(CONTROLLED_COPY_DIR_NAME))?;

    let controlled_root = workspace.join(CONTROLLED_COPY_DIR_NAME);
    let mut removed = 0usize;
    for target in [
        controlled_root.join(CONTROLLED_APP_DIR_NAME),
        controlled_root.join(CONTROLLED_LEGACY_BACKUPS_DIR_NAME),
        controlled_root.join(CONTROLLED_CANDIDATE_DIR_NAME),
    ] {
        if target.file_name().and_then(|name| name.to_str()) == Some(CONTROLLED_CANDIDATE_DIR_NAME)
            && completed_candidate_is_pending_promotion(&target)
        {
            continue;
        }
        removed += usize::from(remove_stale_workspace_directory(
            workspace,
            &target,
            running_app_roots,
            refresh_running_roots_before_delete,
        )?);
    }

    if controlled_root.is_dir() {
        for entry in fs::read_dir(&controlled_root).map_err(|error| {
            format!(
                "读取受控 Codex 目录失败 {}: {error}",
                controlled_root.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "读取受控 Codex 目录条目失败 {}: {error}",
                    controlled_root.display()
                )
            })?;
            let name = entry.file_name().to_string_lossy().to_string();
            let is_directory = entry
                .file_type()
                .map_err(|error| {
                    format!(
                        "检查受控 Codex 目录条目失败 {}: {error}",
                        entry.path().display()
                    )
                })?
                .is_dir();
            if is_directory && name.starts_with(PREVIOUS_STASH_DIR_PREFIX) {
                removed += usize::from(remove_stale_workspace_directory(
                    workspace,
                    &entry.path(),
                    running_app_roots,
                    refresh_running_roots_before_delete,
                )?);
            } else if is_directory && is_generated_candidate_dir_name(&name) {
                if completed_candidate_is_pending_promotion(&entry.path()) {
                    continue;
                }
                removed += usize::from(remove_stale_workspace_directory(
                    workspace,
                    &entry.path(),
                    running_app_roots,
                    refresh_running_roots_before_delete,
                )?);
            }
        }
    }

    let staged_root = workspace.join(STAGED_CODEX_DIR_NAME);
    if staged_root.is_dir() {
        ensure_safe_workspace_cleanup_target(workspace, &staged_root)?;
        for entry in fs::read_dir(&staged_root).map_err(|error| {
            format!(
                "读取旧 staged Codex 目录失败 {}: {error}",
                staged_root.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "读取旧 staged Codex 目录条目失败 {}: {error}",
                    staged_root.display()
                )
            })?;
            if !entry
                .file_type()
                .map_err(|error| {
                    format!(
                        "检查旧 staged Codex 目录条目失败 {}: {error}",
                        entry.path().display()
                    )
                })?
                .is_dir()
            {
                continue;
            }
            removed += usize::from(remove_stale_workspace_directory(
                workspace,
                &entry.path(),
                running_app_roots,
                refresh_running_roots_before_delete,
            )?);
        }
        remove_empty_directory_best_effort(&staged_root);
    }

    for entry in fs::read_dir(workspace)
        .map_err(|error| format!("读取多模型工作区失败 {}: {error}", workspace.display()))?
    {
        let entry = entry.map_err(|error| {
            format!("读取多模型工作区条目失败 {}: {error}", workspace.display())
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        let is_directory = entry
            .file_type()
            .map_err(|error| {
                format!(
                    "检查多模型工作区条目失败 {}: {error}",
                    entry.path().display()
                )
            })?
            .is_dir();
        if is_directory && name.starts_with(PATCH_VALIDATION_DIR_PREFIX) {
            removed += usize::from(remove_stale_workspace_directory(
                workspace,
                &entry.path(),
                running_app_roots,
                refresh_running_roots_before_delete,
            )?);
        }
    }

    removed += prune_managed_patch_backups(workspace, PATCH_BACKUP_KEEP_LATEST)?;
    Ok(removed)
}

fn completed_candidate_is_pending_promotion(candidate_root: &Path) -> bool {
    let state_path = candidate_root.join(PATCH_STATE_FILE_NAME);
    let Ok(raw) = fs::read_to_string(state_path) else {
        return false;
    };
    let Ok(state) = serde_json::from_str::<PatchState>(&raw) else {
        return false;
    };
    if !matches!(state.status.as_str(), "patched" | "already-patched")
        || state.patch_version.as_deref() != Some(MODEL_PICKER_PATCH_VERSION)
        || !state.patch_names.iter().any(|name| name == "model-picker")
        || !state
            .patch_names
            .iter()
            .any(|name| name == "custom-model-picker-ui")
    {
        return false;
    }

    let app_root = candidate_root.join(CONTROLLED_APP_DIR_NAME);
    let asar_path = app_root.join("resources").join("app.asar");
    if !find_codex_launch_exe_in_app_root(&app_root).is_file() || !asar_path.is_file() {
        return false;
    }
    if state.app_asar_path.as_deref().is_none_or(|path| {
        normalize_windows_like_path(path)
            != normalize_windows_like_path(&asar_path.to_string_lossy())
    }) {
        return false;
    }
    let Ok(actual_hash) = sha256_file(&asar_path) else {
        return false;
    };
    state.patched_asar_hash.as_deref() == Some(actual_hash.as_str())
}

fn is_generated_candidate_dir_name(name: &str) -> bool {
    name.strip_prefix(CONTROLLED_GENERATED_CANDIDATE_DIR_PREFIX)
        .and_then(|suffix| Uuid::parse_str(suffix).ok())
        .is_some()
}

fn remove_stale_workspace_directory(
    workspace: &Path,
    target: &Path,
    running_app_roots: &[PathBuf],
    refresh_running_roots_before_delete: bool,
) -> Result<bool, String> {
    if !target.exists() {
        return Ok(false);
    }
    ensure_safe_workspace_cleanup_target(workspace, target)?;
    if running_app_roots
        .iter()
        .any(|running_root| path_is_within_windows_like(&running_root.to_string_lossy(), target))
    {
        log::info!("旧多模型副本仍有进程运行，跳过清理: {}", target.display());
        return Ok(false);
    }
    if refresh_running_roots_before_delete {
        let latest_roots = resolve_running_windows_codex_app_dirs().map_err(|error| {
            format!("删除前无法可靠刷新运行中的 Codex 副本，已跳过清理: {error}")
        })?;
        if latest_roots.iter().any(|running_root| {
            path_is_within_windows_like(&running_root.to_string_lossy(), target)
        }) {
            log::info!("旧多模型副本刚启动了进程，跳过清理: {}", target.display());
            return Ok(false);
        }
    }
    ensure_safe_workspace_cleanup_target(workspace, target)?;
    let target_metadata = fs::symlink_metadata(target)
        .map_err(|error| format!("删除前检查旧多模型目录失败 {}: {error}", target.display()))?;
    if !target_metadata.is_dir() || metadata_is_link_like(&target_metadata) {
        return Err(format!("旧多模型清理目标不是目录: {}", target.display()));
    }
    fs::remove_dir_all(target)
        .map_err(|error| format!("删除旧多模型目录失败 {}: {error}", target.display()))?;
    Ok(true)
}

fn prune_managed_patch_backups(workspace: &Path, keep_latest: usize) -> Result<usize, String> {
    let backup_dir = workspace.join(PATCH_BACKUPS_DIR_NAME);
    if !backup_dir.exists() {
        return Ok(0);
    }
    ensure_safe_workspace_cleanup_target(workspace, &backup_dir)?;
    if !backup_dir.is_dir() {
        return Err(format!(
            "多模型 patch 备份路径不是目录: {}",
            backup_dir.display()
        ));
    }

    let mut asar_backups = Vec::new();
    let mut marker_backups = Vec::new();
    for entry in fs::read_dir(&backup_dir).map_err(|error| {
        format!(
            "读取多模型 patch 备份目录失败 {}: {error}",
            backup_dir.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "读取多模型 patch 备份条目失败 {}: {error}",
                backup_dir.display()
            )
        })?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            format!(
                "检查多模型 patch 备份失败 {}: {error}",
                entry.path().display()
            )
        })?;
        if !metadata.is_file() || metadata_is_link_like(&metadata) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(timestamp) =
            managed_patch_backup_timestamp(&name, "app.asar.controlled.", ".bak")
        {
            asar_backups.push((timestamp, entry.path()));
        } else if let Some(timestamp) =
            managed_patch_backup_timestamp(&name, ".codexdeck-controlled.", ".json.bak")
        {
            marker_backups.push((timestamp, entry.path()));
        }
    }

    let mut removed = 0usize;
    for backups in [&mut asar_backups, &mut marker_backups] {
        backups.sort_by(|left, right| left.0.cmp(&right.0));
        let remove_count = backups.len().saturating_sub(keep_latest);
        for (_, path) in backups.drain(..remove_count) {
            ensure_safe_workspace_cleanup_target(workspace, &path)?;
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                format!(
                    "删除前检查多模型 patch 备份失败 {}: {error}",
                    path.display()
                )
            })?;
            if !metadata.is_file() || metadata_is_link_like(&metadata) {
                return Err(format!(
                    "删除前多模型 patch 备份类型发生变化，已拒绝清理: {}",
                    path.display()
                ));
            }
            fs::remove_file(&path).map_err(|error| {
                format!("删除旧多模型 patch 备份失败 {}: {error}", path.display())
            })?;
            removed += 1;
        }
    }
    remove_empty_directory_best_effort(&backup_dir);
    Ok(removed)
}

fn managed_patch_backup_timestamp(
    file_name: &str,
    prefix: &str,
    suffix: &str,
) -> Option<(String, u32)> {
    let value = file_name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    let timestamp = value.get(..24)?;
    let collision = match value.get(24..)? {
        "" => 0,
        suffix => suffix
            .strip_prefix('-')?
            .parse::<u32>()
            .ok()?
            .checked_add(1)?,
    };
    if timestamp.len() != 24 {
        return None;
    }
    let bytes = timestamp.as_bytes();
    for index in [4usize, 7, 10, 13, 16, 19, 23] {
        let expected = if index == 10 {
            b'T'
        } else if index == 23 {
            b'Z'
        } else {
            b'-'
        };
        if bytes.get(index).copied() != Some(expected) {
            return None;
        }
    }
    for (index, byte) in bytes.iter().enumerate() {
        if ![4usize, 7, 10, 13, 16, 19, 23].contains(&index) && !byte.is_ascii_digit() {
            return None;
        }
    }
    let month = timestamp[5..7].parse::<u8>().ok()?;
    let day = timestamp[8..10].parse::<u8>().ok()?;
    let hour = timestamp[11..13].parse::<u8>().ok()?;
    let minute = timestamp[14..16].parse::<u8>().ok()?;
    let second = timestamp[17..19].parse::<u8>().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    Some((timestamp.to_string(), collision))
}

fn ensure_safe_workspace_cleanup_target(workspace: &Path, target: &Path) -> Result<(), String> {
    if target
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "多模型清理路径包含 ..，已拒绝操作: {}",
            target.display()
        ));
    }
    if !path_is_within_windows_like(&target.to_string_lossy(), workspace) || target == workspace {
        return Err(format!(
            "多模型清理路径越界，已拒绝操作: {}",
            target.display()
        ));
    }
    ensure_path_has_no_link_like_ancestors(workspace, "多模型工作区")?;
    let workspace_metadata = fs::symlink_metadata(workspace)
        .map_err(|error| format!("检查多模型工作区路径失败 {}: {error}", workspace.display()))?;
    if metadata_is_link_like(&workspace_metadata) {
        return Err(format!(
            "多模型工作区是符号链接或 junction，已拒绝清理: {}",
            workspace.display()
        ));
    }
    let relative = target.strip_prefix(workspace).map_err(|_| {
        format!(
            "多模型清理路径无法相对到工作区，已拒绝操作: {}",
            target.display()
        )
    })?;
    if relative.as_os_str().is_empty() {
        return Err("拒绝清理多模型工作区根目录。".to_string());
    }
    let mut current = workspace.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata_is_link_like(&metadata) => {
                return Err(format!(
                    "多模型清理路径包含符号链接或 junction，已拒绝操作: {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!(
                    "检查多模型清理路径失败 {}: {error}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

fn ensure_path_has_no_link_like_ancestors(path: &Path, label: &str) -> Result<(), String> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("读取当前目录失败，无法检查{label}: {error}"))?
            .join(path)
    };
    let ancestors: Vec<_> = absolute_path.ancestors().collect();
    for ancestor in ancestors.into_iter().rev() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata_is_link_like(&metadata) => {
                return Err(format!(
                    "{label}的祖先路径是符号链接或 junction，已拒绝操作: {}",
                    ancestor.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "检查{label}祖先路径失败 {}: {error}",
                    ancestor.display()
                ));
            }
        }
    }
    Ok(())
}

fn remove_empty_directory_best_effort(path: &Path) {
    let Ok(mut entries) = fs::read_dir(path) else {
        return;
    };
    if entries.next().is_none() {
        let _ = fs::remove_dir(path);
    }
}

fn metadata_is_link_like(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(target_os = "windows"))]
    false
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
    patch_version: Option<&str>,
) -> Result<(), String> {
    let source_asar_fingerprint =
        file_fingerprint(&source_app_root.join("resources").join("app.asar")).ok();
    let controlled_asar_fingerprint = file_fingerprint(controlled_app_asar_path).ok();
    let marker = serde_json::json!({
        "schemaVersion": 2,
        "createdAt": now_unix_seconds(),
        "patchVersion": patch_version,
        "workspace": workspace.to_string_lossy(),
        "sourceCodexVersion": source_codex_version,
        "sourceAppRoot": source_app_root.to_string_lossy(),
        "controlledAppRoot": controlled_app_root.to_string_lossy(),
        "controlledExePath": controlled_exe_path.to_string_lossy(),
        "controlledAppAsarPath": controlled_app_asar_path.to_string_lossy(),
        "sourceAsarHash": source_asar_hash,
        "controlledAsarHash": controlled_asar_hash,
        "sourceAsarFingerprint": source_asar_fingerprint,
        "controlledAsarFingerprint": controlled_asar_fingerprint,
    });
    let marker_path = controlled_copy_marker_path(controlled_app_root);
    let serialized = serde_json::to_vec_pretty(&marker)
        .map_err(|error| format!("序列化受控 Codex marker 失败: {error}"))?;
    fs::write(&marker_path, serialized).map_err(|error| {
        format!(
            "写入受控 Codex marker 失败 {}: {error}",
            marker_path.display()
        )
    })
}

fn controlled_copy_marker_path(controlled_app_root: &Path) -> PathBuf {
    controlled_app_root.join(".codexdeck-controlled.json")
}

fn read_controlled_copy_marker(controlled_app_root: &Path) -> Result<ControlledCopyMarker, String> {
    let marker_path = controlled_copy_marker_path(controlled_app_root);
    let raw = fs::read_to_string(&marker_path).map_err(|error| {
        format!(
            "读取受控 Codex marker 失败 {}: {error}",
            marker_path.display()
        )
    })?;
    serde_json::from_str::<ControlledCopyMarker>(&raw).map_err(|error| {
        format!(
            "解析受控 Codex marker 失败 {}: {error}",
            marker_path.display()
        )
    })
}

fn rewrite_controlled_copy_marker_paths(
    workspace: &Path,
    controlled_app_root: &Path,
    controlled_exe_path: &Path,
    controlled_app_asar_path: &Path,
    patch_version: Option<&str>,
    recompute_asar_hash: bool,
) -> Result<(), String> {
    let marker = read_controlled_copy_marker(controlled_app_root)?;
    let controlled_asar_hash = if recompute_asar_hash {
        sha256_file(controlled_app_asar_path)?
    } else {
        marker
            .controlled_asar_hash
            .clone()
            .ok_or_else(|| "受控 Codex marker 缺少 controlledAsarHash。".to_string())?
    };
    let patch_version = patch_version.or(marker.patch_version.as_deref());
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
        patch_version,
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
    let mut file = fs::File::open(path)
        .map_err(|error| format!("读取文件失败 {}: {error}", path.display()))?;
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
    fs::write(&launch_backup, launch_path_payload).map_err(|error| {
        format!(
            "写入启动路径恢复信息失败 {}: {error}",
            launch_backup.display()
        )
    })?;
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
    _app: &AppHandle,
    workspace: &Path,
    controlled_copy: &ControlledCodexCopy,
) -> Result<PatchState, String> {
    if let Some(state) = cached_patch_state_for_controlled_copy(controlled_copy) {
        log::info!("复用已验证的多模型 patch 状态，跳过重复 app.asar 扫描。");
        return Ok(state);
    }

    let request = validated_native_patch_request(workspace, controlled_copy)?;
    let state_value = codex_model_picker_patch::patch_controlled_app(&request)?;
    let state = serde_json::from_value::<PatchState>(state_value)
        .map_err(|error| format!("解析原生 patch 状态失败: {error}"))?;
    if !matches!(state.status.as_str(), "patched" | "already-patched") {
        return Err(format!("patch 状态不可用: {}", state.status));
    }
    if let Some(asar_path) = state.app_asar_path.as_deref() {
        if normalize_windows_like_path(asar_path)
            != normalize_windows_like_path(
                &controlled_copy.controlled_app_asar_path.to_string_lossy(),
            )
        {
            return Err("patch 状态中的 app.asar 不是当前受控副本。".to_string());
        }
    }
    validate_patch_state_for_controlled_copy(&state, controlled_copy)?;
    Ok(state)
}

fn validated_native_patch_request(
    workspace: &Path,
    controlled_copy: &ControlledCodexCopy,
) -> Result<PatchRequest, String> {
    ensure_safe_controlled_copy_target(workspace, &controlled_copy.controlled_app_root)?;
    ensure_safe_workspace_cleanup_target(workspace, &controlled_copy.patch_state_path)?;
    ensure_safe_workspace_cleanup_target(workspace, &workspace.join(PATCH_BACKUPS_DIR_NAME))?;

    let real_workspace = canonicalize_patch_target(workspace, "多模型工作区")?;
    let real_root = canonicalize_patch_target(
        &controlled_copy.controlled_app_root,
        "受控 Codex app 根目录",
    )?;
    let real_asar = canonicalize_patch_target(
        &controlled_copy.controlled_app_asar_path,
        "受控 Codex app.asar",
    )?;
    let real_launch =
        canonicalize_patch_target(&controlled_copy.controlled_exe_path, "受控 Codex 启动文件")?;

    if !real_root.is_dir() {
        return Err(format!(
            "受控 Codex app 根目录不是目录: {}",
            real_root.display()
        ));
    }
    if !real_asar.is_file() {
        return Err(format!(
            "受控 Codex app.asar 不是文件: {}",
            real_asar.display()
        ));
    }
    if !real_launch.is_file() {
        return Err(format!(
            "受控 Codex 启动文件不存在: {}",
            real_launch.display()
        ));
    }
    if is_windows_apps_install_path(&real_root) || is_windows_apps_install_path(&real_asar) {
        return Err("拒绝修改 WindowsApps 中的官方 Codex，只允许 patch 受控副本。".to_string());
    }
    if !path_is_within_windows_like(&real_root.to_string_lossy(), &real_workspace) {
        return Err("受控 Codex app 根目录不在多模型工作区内。".to_string());
    }
    if !path_is_within_windows_like(&real_asar.to_string_lossy(), &real_root) {
        return Err("受控 Codex app.asar 不在受控 app 根目录内。".to_string());
    }
    if !path_is_within_windows_like(&real_launch.to_string_lossy(), &real_root) {
        return Err("受控 Codex 启动文件不在受控 app 根目录内。".to_string());
    }

    let expected_asar = canonicalize_patch_target(
        &real_root.join("resources").join("app.asar"),
        "受控 Codex 标准 app.asar",
    )?;
    if !canonical_patch_paths_match(&real_asar, &expected_asar) {
        return Err(format!(
            "受控 app.asar 必须位于 {}。",
            expected_asar.display()
        ));
    }

    let state_parent = controlled_copy
        .patch_state_path
        .parent()
        .ok_or_else(|| "无法解析 patch 状态目录。".to_string())?;
    let real_state_parent = canonicalize_patch_target(state_parent, "patch 状态目录")?;
    if !path_is_within_windows_like(&real_state_parent.to_string_lossy(), &real_workspace) {
        return Err("patch 状态目录不在多模型工作区内。".to_string());
    }
    if controlled_copy.patch_state_path.exists() {
        let metadata =
            fs::symlink_metadata(&controlled_copy.patch_state_path).map_err(|error| {
                format!(
                    "检查 patch 状态路径失败 {}: {error}",
                    controlled_copy.patch_state_path.display()
                )
            })?;
        if metadata_is_link_like(&metadata) || !metadata.is_file() {
            return Err(format!(
                "patch 状态路径必须是普通文件且不能是符号链接或 junction: {}",
                controlled_copy.patch_state_path.display()
            ));
        }
    }

    let marker_path = controlled_copy_marker_path(&controlled_copy.controlled_app_root);
    let marker_metadata = fs::symlink_metadata(&marker_path).map_err(|error| {
        format!(
            "读取受控 Codex marker 失败 {}: {error}",
            marker_path.display()
        )
    })?;
    if metadata_is_link_like(&marker_metadata) || !marker_metadata.is_file() {
        return Err(format!(
            "受控 Codex marker 必须是普通文件且不能是符号链接或 junction: {}",
            marker_path.display()
        ));
    }
    let marker = read_controlled_copy_marker(&controlled_copy.controlled_app_root)?;
    let marker_workspace = canonicalize_marker_path(marker.workspace.as_deref(), "workspace")?;
    let marker_root =
        canonicalize_marker_path(marker.controlled_app_root.as_deref(), "controlledAppRoot")?;
    let marker_asar = canonicalize_marker_path(
        marker.controlled_app_asar_path.as_deref(),
        "controlledAppAsarPath",
    )?;
    let marker_launch =
        canonicalize_marker_path(marker.controlled_exe_path.as_deref(), "controlledExePath")?;
    if !canonical_patch_paths_match(&marker_workspace, &real_workspace)
        || !canonical_patch_paths_match(&marker_root, &real_root)
        || !canonical_patch_paths_match(&marker_asar, &real_asar)
        || !canonical_patch_paths_match(&marker_launch, &real_launch)
    {
        return Err("受控 Codex marker 与本次 patch 目标不匹配。".to_string());
    }

    Ok(PatchRequest {
        app_asar_path: controlled_copy.controlled_app_asar_path.clone(),
        backup_dir: workspace.join(PATCH_BACKUPS_DIR_NAME),
        patch_state_path: controlled_copy.patch_state_path.clone(),
        patch_version: MODEL_PICKER_PATCH_VERSION.to_string(),
        source_codex_version: controlled_copy.source_codex_version.clone(),
        source_asar_hash: Some(controlled_copy.source_asar_hash.clone()),
        controlled_app_root: Some(controlled_copy.controlled_app_root.clone()),
        launch_path: Some(controlled_copy.controlled_exe_path.clone()),
    })
}

fn canonicalize_patch_target(path: &Path, label: &str) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|error| format!("无法解析{label} {}: {error}", path.display()))
}

fn canonicalize_marker_path(value: Option<&str>, field: &str) -> Result<PathBuf, String> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("受控 Codex marker 缺少 {field}。"))?;
    canonicalize_patch_target(Path::new(value), &format!("marker.{field}"))
}

fn canonical_patch_paths_match(left: &Path, right: &Path) -> bool {
    normalize_windows_like_path(&left.to_string_lossy())
        == normalize_windows_like_path(&right.to_string_lossy())
}

fn is_windows_apps_install_path(path: &Path) -> bool {
    let normalized = normalize_windows_like_path(&path.to_string_lossy());
    normalized.contains("\\windowsapps\\") || normalized.ends_with("\\windowsapps")
}

fn cached_patch_state_for_controlled_copy(
    controlled_copy: &ControlledCodexCopy,
) -> Option<PatchState> {
    if let Some(state) = fast_launch_patch_state_for_controlled_copy(controlled_copy) {
        return Some(state);
    }

    let (state, marker) = cached_patch_metadata_for_controlled_copy(controlled_copy)?;
    let actual_hash = sha256_file(&controlled_copy.controlled_app_asar_path).ok()?;
    if marker.controlled_asar_hash.as_deref() != Some(actual_hash.as_str()) {
        return None;
    }
    if let Err(error) = rewrite_controlled_copy_marker_paths(
        controlled_copy
            .controlled_app_root
            .parent()?
            .parent()?
            .parent()?,
        &controlled_copy.controlled_app_root,
        &controlled_copy.controlled_exe_path,
        &controlled_copy.controlled_app_asar_path,
        Some(MODEL_PICKER_PATCH_VERSION),
        false,
    ) {
        log::warn!("回填受控 Codex 快速校验指纹失败，将继续使用完整 hash: {error}");
    }
    Some(state)
}

fn fast_launch_patch_state_for_controlled_copy(
    controlled_copy: &ControlledCodexCopy,
) -> Option<PatchState> {
    let (state, marker) = cached_patch_metadata_for_controlled_copy(controlled_copy)?;
    if marker.schema_version.unwrap_or_default() < 2 {
        return None;
    }
    if !source_app_root_is_current_for_fast_launch(&controlled_copy.source_app_root) {
        return None;
    }
    let source_fingerprint = marker.source_asar_fingerprint.as_ref()?;
    let controlled_fingerprint = marker.controlled_asar_fingerprint.as_ref()?;
    let source_asar_path = controlled_copy
        .source_app_root
        .join("resources")
        .join("app.asar");
    if &file_fingerprint(&source_asar_path).ok()? != source_fingerprint {
        return None;
    }
    if &file_fingerprint(&controlled_copy.controlled_app_asar_path).ok()? != controlled_fingerprint
    {
        return None;
    }
    Some(state)
}

fn fast_launch_patch_state_for_expected_source(
    controlled_copy: &ControlledCodexCopy,
    expected_source_app_root: Option<&Path>,
) -> Option<PatchState> {
    if expected_source_app_root.is_some_and(|expected| {
        !canonical_patch_paths_match(expected, &controlled_copy.source_app_root)
    }) {
        return None;
    }

    fast_launch_patch_state_for_controlled_copy(controlled_copy)
}

fn cached_patch_metadata_for_controlled_copy(
    controlled_copy: &ControlledCodexCopy,
) -> Option<(PatchState, ControlledCopyMarker)> {
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
    if marker.patch_version.as_deref() != Some(MODEL_PICKER_PATCH_VERSION) {
        return None;
    }
    for (marker_path, expected_path) in [
        (
            marker.source_app_root.as_deref(),
            controlled_copy.source_app_root.as_path(),
        ),
        (
            marker.controlled_app_root.as_deref(),
            controlled_copy.controlled_app_root.as_path(),
        ),
        (
            marker.controlled_exe_path.as_deref(),
            controlled_copy.controlled_exe_path.as_path(),
        ),
        (
            marker.controlled_app_asar_path.as_deref(),
            controlled_copy.controlled_app_asar_path.as_path(),
        ),
    ] {
        let marker_path = marker_path?;
        if normalize_windows_like_path(marker_path)
            != normalize_windows_like_path(&expected_path.to_string_lossy())
        {
            return None;
        }
    }
    match (
        marker.controlled_asar_hash.as_deref(),
        state.patched_asar_hash.as_deref(),
    ) {
        (Some(marker_hash), Some(state_hash)) if marker_hash == state_hash => {}
        _ => return None,
    }
    Some((state, marker))
}

fn file_fingerprint(path: &Path) -> Result<FileFingerprint, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("读取文件指纹失败 {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("文件指纹目标不是普通文件: {}", path.display()));
    }
    let modified = metadata
        .modified()
        .map_err(|error| format!("读取文件修改时间失败 {}: {error}", path.display()))?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("文件修改时间无效 {}: {error}", path.display()))?;
    Ok(FileFingerprint {
        size: metadata.len(),
        modified_seconds: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

fn source_app_root_is_current_for_fast_launch(source_app_root: &Path) -> bool {
    if !is_windows_apps_install_path(source_app_root) {
        return true;
    }

    #[cfg(target_os = "windows")]
    {
        return resolve_windows_apps_codex_app_dirs()
            .first()
            .is_some_and(|latest| canonical_patch_paths_match(latest, source_app_root));
    }
    #[cfg(not(target_os = "windows"))]
    false
}

fn validate_patch_state_for_controlled_copy(
    state: &PatchState,
    controlled_copy: &ControlledCodexCopy,
) -> Result<(), String> {
    if state.patch_version.as_deref() != Some(MODEL_PICKER_PATCH_VERSION) {
        return Err(format!(
            "patch 版本不匹配，期望 {MODEL_PICKER_PATCH_VERSION}。"
        ));
    }
    if !state.patch_names.iter().any(|name| name == "model-picker") {
        return Err("patch 未命中新版模型选择器过滤点 model-picker。".to_string());
    }
    if !state
        .patch_names
        .iter()
        .any(|name| name == "custom-model-picker-ui")
    {
        return Err("patch 未命中自定义模型选择器 UI。".to_string());
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
        if let Some(restore_point) = restore_point.map(Path::to_path_buf).or_else(|| {
            store
                .settings
                .codex_multi_model_restore_point
                .as_ref()
                .map(PathBuf::from)
        }) {
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
        settings.codex_multi_model_patch_state_path = Some(
            workspace
                .join(PATCH_STATE_FILE_NAME)
                .to_string_lossy()
                .to_string(),
        );
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
    if normalized_root.is_empty() {
        return false;
    }
    normalized_path == normalized_root
        || normalized_path.starts_with(&format!("{normalized_root}\\"))
}

fn normalize_windows_like_path(value: &str) -> String {
    let replaced = value.trim().replace('/', "\\");
    let rooted = replaced.starts_with('\\');
    let unc = replaced.starts_with("\\\\");
    let mut parts = Vec::new();
    for part in replaced.split('\\') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            let can_pop = parts
                .last()
                .is_some_and(|last: &String| last != ".." && !last.ends_with(':'));
            if can_pop {
                parts.pop();
            } else {
                parts.push(part.to_string());
            }
            continue;
        }
        parts.push(part.to_ascii_lowercase());
    }

    let normalized = parts.join("\\");
    if unc {
        format!("\\\\{normalized}")
    } else if rooted {
        format!("\\{normalized}")
    } else {
        normalized
    }
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

    let entries = fs::read_dir(agents_dir).map_err(|error| {
        format!(
            "读取 Codex agents 目录失败 {}: {error}",
            agents_dir.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("读取 Codex agent 文件失败: {error}"))?;
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
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建恢复目标目录失败 {}: {error}", parent.display()))?;
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
        .and_then(|store| {
            store
                .settings
                .codex_multi_model_restore_point
                .map(PathBuf::from)
        })
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

    fn create_patch_cache_fixture(
        workspace: &Path,
        state_patch_version: Option<&str>,
        marker_patch_version: Option<&str>,
    ) -> ControlledCodexCopy {
        create_patch_cache_fixture_at(
            workspace,
            controlled_current_app_root(workspace),
            state_patch_version,
            marker_patch_version,
        )
    }

    fn create_patch_cache_fixture_at(
        workspace: &Path,
        app_root: PathBuf,
        state_patch_version: Option<&str>,
        marker_patch_version: Option<&str>,
    ) -> ControlledCodexCopy {
        let resources_dir = app_root.join("resources");
        let exe_path = app_root.join("ChatGPT.exe");
        let asar_path = resources_dir.join("app.asar");
        fs::create_dir_all(&resources_dir).expect("create controlled resources dir");
        fs::write(&exe_path, b"test exe").expect("write controlled exe");
        fs::write(&asar_path, b"test asar").expect("write controlled asar");
        let controlled_hash = sha256_file(&asar_path).expect("hash controlled asar");
        let marker = serde_json::json!({
            "workspace": workspace,
            "patchVersion": marker_patch_version,
            "sourceAppRoot": "fixtures/OpenAI.Codex_test/app",
            "sourceAsarHash": "source-hash",
            "sourceCodexVersion": "test-version",
            "controlledAppRoot": app_root,
            "controlledExePath": exe_path,
            "controlledAppAsarPath": asar_path,
            "controlledAsarHash": controlled_hash,
        });
        fs::write(
            controlled_copy_marker_path(&app_root),
            serde_json::to_vec_pretty(&marker).expect("serialize controlled marker"),
        )
        .expect("write controlled marker");

        let candidate_app_root = workspace
            .join(CONTROLLED_COPY_DIR_NAME)
            .join(CONTROLLED_CANDIDATE_DIR_NAME)
            .join(CONTROLLED_APP_DIR_NAME);
        let patch_state_path = if app_root == candidate_app_root {
            app_root
                .parent()
                .expect("candidate app parent")
                .join(PATCH_STATE_FILE_NAME)
        } else {
            workspace.join(PATCH_STATE_FILE_NAME)
        };
        let state = serde_json::json!({
            "status": "patched",
            "patchVersion": state_patch_version,
            "appAsarPath": asar_path,
            "sourceAsarHash": "source-hash",
            "sourceCodexVersion": "test-version",
            "patchedAsarHash": controlled_hash,
            "patchNames": ["model-picker", "custom-model-picker-ui"],
        });
        fs::write(
            &patch_state_path,
            serde_json::to_vec_pretty(&state).expect("serialize patch state"),
        )
        .expect("write patch state");

        ControlledCodexCopy {
            source_app_root: PathBuf::from("fixtures/OpenAI.Codex_test/app"),
            controlled_app_root: app_root,
            controlled_exe_path: exe_path,
            controlled_app_asar_path: asar_path,
            source_asar_hash: "source-hash".to_string(),
            source_codex_version: Some("test-version".to_string()),
            patch_state_path,
        }
    }

    fn test_fingerprint_json(path: &Path) -> serde_json::Value {
        let metadata = fs::metadata(path).expect("fingerprint metadata");
        let modified = metadata
            .modified()
            .expect("fingerprint modified time")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("fingerprint modified time after epoch");
        serde_json::json!({
            "size": metadata.len(),
            "modifiedSeconds": modified.as_secs(),
            "modifiedNanos": modified.subsec_nanos(),
        })
    }

    fn create_fast_patch_cache_fixture(workspace: &Path) -> ControlledCodexCopy {
        let mut controlled = create_patch_cache_fixture(
            workspace,
            Some(MODEL_PICKER_PATCH_VERSION),
            Some(MODEL_PICKER_PATCH_VERSION),
        );
        let source_app_root = workspace.join("source").join("app");
        let source_asar_path = source_app_root.join("resources").join("app.asar");
        fs::create_dir_all(source_asar_path.parent().expect("source resources"))
            .expect("create source resources");
        fs::write(&source_asar_path, b"source asar").expect("write source asar");
        controlled.source_app_root = source_app_root.clone();

        let marker_path = controlled_copy_marker_path(&controlled.controlled_app_root);
        let mut marker = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(&marker_path).expect("read marker"),
        )
        .expect("parse marker");
        let object = marker.as_object_mut().expect("marker object");
        object.insert("schemaVersion".to_string(), serde_json::json!(2));
        object.insert(
            "sourceAppRoot".to_string(),
            serde_json::json!(source_app_root),
        );
        object.insert(
            "sourceAsarFingerprint".to_string(),
            test_fingerprint_json(&source_asar_path),
        );
        object.insert(
            "controlledAsarFingerprint".to_string(),
            test_fingerprint_json(&controlled.controlled_app_asar_path),
        );
        fs::write(
            marker_path,
            serde_json::to_vec_pretty(&marker).expect("serialize marker"),
        )
        .expect("write marker");
        controlled
    }

    #[test]
    fn patch_cache_accepts_matching_patch_version() {
        let workspace = temp_workspace("matching-patch-version");
        let controlled = create_patch_cache_fixture(
            &workspace,
            Some(MODEL_PICKER_PATCH_VERSION),
            Some(MODEL_PICKER_PATCH_VERSION),
        );

        assert!(cached_patch_state_for_controlled_copy(&controlled).is_some());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn fast_launch_cache_accepts_matching_source_and_controlled_fingerprints() {
        let workspace = temp_workspace("fast-launch-cache-match");
        let controlled = create_fast_patch_cache_fixture(&workspace);

        assert!(fast_launch_patch_state_for_controlled_copy(&controlled).is_some());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn fast_launch_cache_rejects_explicit_source_switch_from_a_to_b() {
        let workspace = temp_workspace("fast-launch-cache-source-switch");
        let controlled = create_fast_patch_cache_fixture(&workspace);
        let source_b = workspace.join("portable-source-b").join("app");

        assert!(fast_launch_patch_state_for_expected_source(
            &controlled,
            Some(&controlled.source_app_root),
        )
        .is_some());
        assert!(
            fast_launch_patch_state_for_expected_source(&controlled, Some(&source_b)).is_none()
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn source_switch_requires_rebuild_even_when_asar_hash_matches() {
        let workspace = temp_workspace("source-switch-same-hash");
        let controlled = create_fast_patch_cache_fixture(&workspace);
        let source = SourceCodexSnapshot {
            app_root: workspace.join("portable-source-b").join("app"),
            exe_path: workspace
                .join("portable-source-b")
                .join("app")
                .join("ChatGPT.exe"),
            asar_hash: controlled.source_asar_hash.clone(),
            codex_version: controlled.source_codex_version.clone(),
        };

        assert!(source_snapshot_requires_rebuild(&controlled, &source));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn fast_launch_cache_rejects_changed_source_asar_fingerprint() {
        let workspace = temp_workspace("fast-launch-cache-source-change");
        let controlled = create_fast_patch_cache_fixture(&workspace);
        fs::write(
            controlled
                .source_app_root
                .join("resources")
                .join("app.asar"),
            b"changed source asar",
        )
        .expect("change source asar");

        assert!(fast_launch_patch_state_for_controlled_copy(&controlled).is_none());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn fast_launch_cache_rejects_changed_controlled_asar_fingerprint() {
        let workspace = temp_workspace("fast-launch-cache-controlled-change");
        let controlled = create_fast_patch_cache_fixture(&workspace);
        fs::write(
            &controlled.controlled_app_asar_path,
            b"changed controlled asar",
        )
        .expect("change controlled asar");

        assert!(fast_launch_patch_state_for_controlled_copy(&controlled).is_none());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn fast_launch_cache_rejects_legacy_marker_without_fingerprints() {
        let workspace = temp_workspace("fast-launch-cache-legacy-marker");
        let controlled = create_patch_cache_fixture(
            &workspace,
            Some(MODEL_PICKER_PATCH_VERSION),
            Some(MODEL_PICKER_PATCH_VERSION),
        );

        assert!(fast_launch_patch_state_for_controlled_copy(&controlled).is_none());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn patch_cache_rejects_legacy_patch_version() {
        let workspace = temp_workspace("legacy-patch-version");
        let controlled = create_patch_cache_fixture(
            &workspace,
            Some("model-picker-v5"),
            Some("model-picker-v5"),
        );

        assert!(cached_patch_state_for_controlled_copy(&controlled).is_none());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn patch_cache_rejects_marker_without_patch_version() {
        let workspace = temp_workspace("missing-marker-patch-version");
        let controlled =
            create_patch_cache_fixture(&workspace, Some(MODEL_PICKER_PATCH_VERSION), None);

        assert!(cached_patch_state_for_controlled_copy(&controlled).is_none());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn patch_cache_rejects_modified_controlled_asar() {
        let workspace = temp_workspace("modified-controlled-asar");
        let controlled = create_patch_cache_fixture(
            &workspace,
            Some(MODEL_PICKER_PATCH_VERSION),
            Some(MODEL_PICKER_PATCH_VERSION),
        );
        fs::write(&controlled.controlled_app_asar_path, b"modified asar")
            .expect("modify controlled asar");

        assert!(cached_patch_state_for_controlled_copy(&controlled).is_none());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn candidate_patch_state_is_isolated_from_current_state() {
        let workspace = temp_workspace("candidate-state-isolation");
        let source_app_root = workspace.join("source").join("app");
        let source_resources = source_app_root.join("resources");
        fs::create_dir_all(&source_resources).expect("create source resources");
        let source_exe_path = source_app_root.join("ChatGPT.exe");
        let source_asar_path = source_resources.join("app.asar");
        fs::write(&source_exe_path, b"source exe").expect("write source exe");
        fs::write(&source_asar_path, b"source asar").expect("write source asar");
        let source = SourceCodexSnapshot {
            app_root: source_app_root,
            exe_path: source_exe_path,
            asar_hash: sha256_file(&source_asar_path).expect("hash source asar"),
            codex_version: Some("test-version".to_string()),
        };

        let candidate =
            prepare_candidate_controlled_copy(&workspace, &source).expect("prepare candidate");

        assert_ne!(
            candidate.patch_state_path,
            workspace.join(PATCH_STATE_FILE_NAME)
        );
        assert!(path_is_within_windows_like(
            &candidate.patch_state_path.to_string_lossy(),
            &workspace
                .join(CONTROLLED_COPY_DIR_NAME)
                .join(CONTROLLED_CANDIDATE_DIR_NAME),
        ));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn completed_candidate_is_reused_after_promotion_failure() {
        let workspace = temp_workspace("completed-candidate-reuse");
        let source_app_root = workspace.join("source").join("app");
        let source_resources = source_app_root.join("resources");
        fs::create_dir_all(&source_resources).expect("create source resources");
        let source_exe_path = source_app_root.join("ChatGPT.exe");
        let source_asar_path = source_resources.join("app.asar");
        fs::write(&source_exe_path, b"source exe").expect("write source exe");
        fs::write(&source_asar_path, b"source asar").expect("write source asar");
        let source = SourceCodexSnapshot {
            app_root: source_app_root,
            exe_path: source_exe_path,
            asar_hash: sha256_file(&source_asar_path).expect("hash source asar"),
            codex_version: Some("test-version".to_string()),
        };
        let candidate =
            prepare_candidate_controlled_copy(&workspace, &source).expect("prepare candidate");
        let patched_hash = sha256_file(&candidate.controlled_app_asar_path)
            .expect("hash completed candidate asar");
        fs::write(
            &candidate.patch_state_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "status": "patched",
                "patchVersion": MODEL_PICKER_PATCH_VERSION,
                "sourceCodexVersion": source.codex_version,
                "sourceAsarHash": source.asar_hash,
                "patchedAsarHash": patched_hash,
                "patchNames": ["model-picker", "custom-model-picker-ui"],
                "appAsarPath": candidate.controlled_app_asar_path,
                "controlledAppRoot": candidate.controlled_app_root,
                "launchPath": candidate.controlled_exe_path,
            }))
            .expect("serialize completed candidate state"),
        )
        .expect("write completed candidate state");

        let recovered = ready_candidate_controlled_copy_for_source(&workspace, &source)
            .expect("recover completed candidate")
            .expect("completed candidate must be reusable");

        assert_eq!(recovered.controlled_app_root, candidate.controlled_app_root);
        assert_eq!(recovered.patch_state_path, candidate.patch_state_path);
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn promotion_file_operation_retries_windows_sharing_violations() {
        let mut attempts = 0usize;
        let mut waits = 0usize;
        let result = retry_transient_promotion_file_operation(
            4,
            || waits += 1,
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(std::io::Error::from_raw_os_error(32))
                } else {
                    Ok("promoted")
                }
            },
        )
        .expect("sharing violation should be retried");

        assert_eq!(result, "promoted");
        assert_eq!(attempts, 3);
        assert_eq!(waits, 2);
    }

    #[test]
    fn promotion_pending_error_keeps_model_assets_for_retry() {
        let error = candidate_promotion_pending_error("sharing violation".to_string());

        let (status, restore_files) = prepare_failure_policy(&error);

        assert_eq!(status, "promotion-pending");
        assert!(!restore_files);
        assert_eq!(prepare_failure_policy("patch failed"), ("failed", true));
    }

    #[test]
    fn existing_candidate_uses_isolated_generation_without_deleting_existing() {
        let workspace = temp_workspace("candidate-generation-isolation");
        let source_app_root = workspace.join("source").join("app");
        let source_resources = source_app_root.join("resources");
        fs::create_dir_all(&source_resources).expect("create source resources");
        let source_exe_path = source_app_root.join("ChatGPT.exe");
        let source_asar_path = source_resources.join("app.asar");
        fs::write(&source_exe_path, b"source exe").expect("write source exe");
        fs::write(&source_asar_path, b"source asar").expect("write source asar");
        let source = SourceCodexSnapshot {
            app_root: source_app_root,
            exe_path: source_exe_path,
            asar_hash: sha256_file(&source_asar_path).expect("hash source asar"),
            codex_version: Some("test-version".to_string()),
        };
        let existing_root = workspace
            .join(CONTROLLED_COPY_DIR_NAME)
            .join(CONTROLLED_CANDIDATE_DIR_NAME);
        let existing_sentinel = existing_root.join("app").join("running.txt");
        fs::create_dir_all(existing_sentinel.parent().expect("sentinel parent"))
            .expect("create existing candidate");
        fs::write(&existing_sentinel, b"preserve").expect("write existing candidate sentinel");

        let candidate =
            prepare_candidate_controlled_copy(&workspace, &source).expect("prepare candidate");
        let generated_root = candidate
            .controlled_app_root
            .parent()
            .expect("generated candidate root");
        let generated_name = generated_root
            .file_name()
            .and_then(|name| name.to_str())
            .expect("generated candidate name");

        assert!(existing_sentinel.is_file());
        assert_ne!(generated_root, existing_root);
        assert!(is_generated_candidate_dir_name(generated_name));
        assert_eq!(candidate.patch_state_path.parent(), Some(generated_root));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn explicit_legacy_exe_prefers_chatgpt_in_same_app_root() {
        let workspace = temp_workspace("prefer-chatgpt-exe");
        let app_root = workspace.join("app");
        fs::create_dir_all(app_root.join("resources")).expect("create app resources");
        let legacy_exe = app_root.join("Codex.exe");
        let chatgpt_exe = app_root.join("ChatGPT.exe");
        fs::write(&legacy_exe, b"legacy exe").expect("write legacy exe");
        fs::write(&chatgpt_exe, b"current exe").expect("write current exe");
        fs::write(app_root.join("resources").join("app.asar"), b"asar").expect("write app asar");

        let (_, resolved_exe) =
            app_root_from_codex_launch_path(&legacy_exe).expect("resolve app root");

        assert_eq!(resolved_exe, chatgpt_exe);

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn windows_like_containment_collapses_parent_segments() {
        let root = Path::new(r"C:\codexdeck\controlled-codex");

        assert!(path_is_within_windows_like(
            r"C:\codexdeck\controlled-codex\current\app",
            root,
        ));
        assert!(!path_is_within_windows_like(
            r"C:\codexdeck\controlled-codex\..\..\outside",
            root,
        ));
    }

    #[test]
    fn safe_controlled_target_rejects_parent_segments() {
        let workspace = temp_workspace("parent-segment-target");
        fs::create_dir_all(&workspace).expect("create workspace");
        let target = workspace
            .join(CONTROLLED_COPY_DIR_NAME)
            .join("..")
            .join("outside");

        assert!(ensure_safe_controlled_copy_target(&workspace, &target).is_err());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn atomic_patch_state_write_replaces_existing_file() {
        let workspace = temp_workspace("atomic-patch-state");
        fs::create_dir_all(&workspace).expect("create workspace");
        let state_path = workspace.join(PATCH_STATE_FILE_NAME);
        fs::write(&state_path, b"old state").expect("write old state");

        write_patch_state_atomically(&state_path, b"new state\n").expect("replace patch state");

        assert_eq!(
            fs::read(&state_path).expect("read replaced state"),
            b"new state\n"
        );
        let leftovers = fs::read_dir(&workspace)
            .expect("read workspace")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
            .count();
        assert_eq!(leftovers, 0);

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn native_patch_target_accepts_matching_controlled_marker() {
        let workspace = temp_workspace("native-patch-target-valid");
        fs::create_dir_all(&workspace).expect("create workspace");
        let controlled_copy = create_patch_cache_fixture(&workspace, None, None);

        let request = validated_native_patch_request(&workspace, &controlled_copy)
            .expect("validate native patch target");

        assert_eq!(
            request.app_asar_path,
            controlled_copy.controlled_app_asar_path
        );
        assert_eq!(
            request.launch_path,
            Some(controlled_copy.controlled_exe_path)
        );
        assert_eq!(request.backup_dir, workspace.join(PATCH_BACKUPS_DIR_NAME));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn native_patch_target_rejects_mismatched_controlled_marker() {
        let workspace = temp_workspace("native-patch-target-mismatch");
        fs::create_dir_all(&workspace).expect("create workspace");
        let controlled_copy = create_patch_cache_fixture(&workspace, None, None);
        let marker_path = controlled_copy_marker_path(&controlled_copy.controlled_app_root);
        let mut marker: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker_path).expect("read controlled marker"))
                .expect("parse controlled marker");
        marker["controlledAppAsarPath"] = serde_json::Value::String(
            controlled_copy
                .controlled_exe_path
                .to_string_lossy()
                .to_string(),
        );
        fs::write(
            &marker_path,
            serde_json::to_vec_pretty(&marker).expect("serialize mismatched marker"),
        )
        .expect("write mismatched marker");

        let error = validated_native_patch_request(&workspace, &controlled_copy)
            .expect_err("reject mismatched native patch marker");

        assert!(error.contains("marker 与本次 patch 目标不匹配"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn safe_controlled_target_rejects_symlinked_ancestor() {
        use std::os::windows::fs::symlink_dir;

        let workspace = temp_workspace("symlinked-controlled-target");
        let controlled_root = workspace.join(CONTROLLED_COPY_DIR_NAME);
        let outside = temp_workspace("outside-controlled-target");
        fs::create_dir_all(&controlled_root).expect("create controlled root");
        fs::create_dir_all(&outside).expect("create outside target");
        let linked_candidate = controlled_root.join(CONTROLLED_CANDIDATE_DIR_NAME);
        symlink_dir(&outside, &linked_candidate).expect("create candidate directory symlink");

        let result = ensure_safe_controlled_copy_target(
            &workspace,
            &linked_candidate.join(CONTROLLED_APP_DIR_NAME),
        );

        assert!(result.is_err());

        fs::remove_dir(&linked_candidate).expect("remove candidate directory symlink");
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(outside);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn safe_controlled_target_rejects_symlinked_workspace() {
        use std::os::windows::fs::symlink_dir;

        let workspace = temp_workspace("symlinked-workspace");
        let real_workspace = temp_workspace("real-workspace");
        fs::create_dir_all(&real_workspace).expect("create real workspace");
        symlink_dir(&real_workspace, &workspace).expect("create workspace symlink");
        let target = workspace
            .join(CONTROLLED_COPY_DIR_NAME)
            .join(CONTROLLED_CANDIDATE_DIR_NAME)
            .join(CONTROLLED_APP_DIR_NAME);

        assert!(ensure_safe_controlled_copy_target(&workspace, &target).is_err());

        fs::remove_dir(&workspace).expect("remove workspace symlink");
        let _ = fs::remove_dir_all(real_workspace);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn safe_controlled_target_rejects_symlinked_workspace_ancestor() {
        use std::os::windows::fs::symlink_dir;

        let root = temp_workspace("symlinked-workspace-ancestor");
        let outside = root.join("outside");
        let real_workspace = outside.join("workspace");
        fs::create_dir_all(&real_workspace).expect("create real workspace");
        let linked_parent = root.join("linked-parent");
        symlink_dir(&outside, &linked_parent).expect("create workspace ancestor symlink");
        let workspace = linked_parent.join("workspace");
        let target = workspace
            .join(CONTROLLED_COPY_DIR_NAME)
            .join(CONTROLLED_CANDIDATE_DIR_NAME)
            .join(CONTROLLED_APP_DIR_NAME);

        assert!(ensure_safe_controlled_copy_target(&workspace, &target).is_err());

        fs::remove_dir(&linked_parent).expect("remove workspace ancestor symlink");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn promotion_rewrites_candidate_state_and_preserves_previous_copy() {
        let workspace = temp_workspace("promote-candidate-state");
        let previous_current = create_patch_cache_fixture(
            &workspace,
            Some("model-picker-v8"),
            Some("model-picker-v8"),
        );
        fs::write(
            previous_current.controlled_app_root.join("old-stable"),
            b"old",
        )
        .expect("write old stable sentinel");

        let candidate_root = workspace
            .join(CONTROLLED_COPY_DIR_NAME)
            .join(CONTROLLED_CANDIDATE_DIR_NAME)
            .join(CONTROLLED_APP_DIR_NAME);
        let candidate = create_patch_cache_fixture_at(
            &workspace,
            candidate_root,
            Some(MODEL_PICKER_PATCH_VERSION),
            None,
        );
        fs::write(candidate.controlled_app_root.join("new-candidate"), b"new")
            .expect("write candidate sentinel");

        let promoted =
            promote_candidate_controlled_copy(&workspace, &candidate).expect("promote candidate");

        assert!(promoted.controlled_app_root.join("new-candidate").is_file());
        assert!(controlled_previous_app_root(&workspace)
            .join("old-stable")
            .is_file());
        assert!(cached_patch_state_for_controlled_copy(&promoted).is_some());
        assert_eq!(
            promoted
                .controlled_exe_path
                .file_name()
                .and_then(|name| name.to_str()),
            Some("ChatGPT.exe")
        );

        let previous_marker: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(controlled_copy_marker_path(&controlled_previous_app_root(
                &workspace,
            )))
            .expect("read previous marker"),
        )
        .expect("parse previous marker");
        let previous_exe = previous_marker
            .get("controlledExePath")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .expect("previous marker launch path");
        assert_eq!(
            previous_exe.file_name().and_then(|name| name.to_str()),
            Some("ChatGPT.exe")
        );
        assert!(previous_exe.is_file());

        let state: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&promoted.patch_state_path).expect("read promoted patch state"),
        )
        .expect("parse promoted patch state");
        assert_eq!(
            state.get("appAsarPath").and_then(serde_json::Value::as_str),
            Some(promoted.controlled_app_asar_path.to_string_lossy().as_ref())
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn promotion_defers_stashed_previous_cleanup_until_process_check() {
        let workspace = temp_workspace("defer-stashed-previous-cleanup");
        let controlled_root = workspace.join(CONTROLLED_COPY_DIR_NAME);
        let previous_app_root = controlled_previous_app_root(&workspace);
        let stash_root =
            controlled_root.join(format!("{PREVIOUS_STASH_DIR_PREFIX}{}", Uuid::new_v4()));
        let stashed_app_root = stash_root.join(CONTROLLED_APP_DIR_NAME);
        fs::create_dir_all(&stashed_app_root).expect("create stashed previous");
        fs::write(stashed_app_root.join("still-running"), b"running")
            .expect("write stashed previous sentinel");

        finalize_stashed_previous_copy(&previous_app_root, Some(&stashed_app_root), true);

        assert!(stash_root.exists());
        cleanup_stale_multimodel_artifacts_after_running_root_lookup(
            &workspace,
            Ok(vec![stashed_app_root.clone()]),
        )
        .expect("cleanup while stashed copy runs");
        assert!(stash_root.exists());
        cleanup_stale_multimodel_artifacts_after_running_root_lookup(&workspace, Ok(Vec::new()))
            .expect("cleanup after stashed copy exits");
        assert!(!stash_root.exists());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn promotion_restores_previous_copy_when_candidate_marker_is_invalid() {
        let workspace = temp_workspace("promotion-marker-rollback");
        let previous_current = create_patch_cache_fixture(
            &workspace,
            Some(MODEL_PICKER_PATCH_VERSION),
            Some(MODEL_PICKER_PATCH_VERSION),
        );
        fs::write(
            previous_current.controlled_app_root.join("old-stable"),
            b"old",
        )
        .expect("write old stable sentinel");

        let existing_previous = controlled_previous_app_root(&workspace);
        fs::create_dir_all(&existing_previous).expect("create existing previous copy");
        fs::write(existing_previous.join("older-stable"), b"older")
            .expect("write older stable sentinel");

        let candidate_root = workspace
            .join(CONTROLLED_COPY_DIR_NAME)
            .join(CONTROLLED_CANDIDATE_DIR_NAME)
            .join(CONTROLLED_APP_DIR_NAME);
        let candidate = create_patch_cache_fixture_at(
            &workspace,
            candidate_root,
            Some(MODEL_PICKER_PATCH_VERSION),
            None,
        );
        fs::write(candidate.controlled_app_root.join("new-candidate"), b"new")
            .expect("write candidate sentinel");
        fs::write(
            controlled_copy_marker_path(&candidate.controlled_app_root),
            b"not-json",
        )
        .expect("corrupt candidate marker");

        let error = promote_candidate_controlled_copy(&workspace, &candidate)
            .expect_err("invalid candidate marker must fail promotion");

        assert!(error.contains("marker"));
        assert!(controlled_current_app_root(&workspace)
            .join("old-stable")
            .is_file());
        let restored_marker: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(controlled_copy_marker_path(&controlled_current_app_root(
                &workspace,
            )))
            .expect("read restored marker"),
        )
        .expect("parse restored marker");
        let restored_exe = restored_marker
            .get("controlledExePath")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .expect("restored marker launch path");
        assert_eq!(
            restored_exe.file_name().and_then(|name| name.to_str()),
            Some("ChatGPT.exe")
        );
        assert!(restored_exe.is_file());
        assert!(candidate
            .controlled_app_root
            .join("new-candidate")
            .is_file());
        assert!(cached_patch_state_for_controlled_copy(&previous_current).is_some());
        assert!(controlled_previous_app_root(&workspace)
            .join("older-stable")
            .is_file());

        let _ = fs::remove_dir_all(workspace);
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
    fn missing_controlled_copy_uses_rebuild_result() {
        let workspace = temp_workspace("recover-missing-copy");
        let expected = create_patch_cache_fixture(
            &workspace,
            Some(MODEL_PICKER_PATCH_VERSION),
            Some(MODEL_PICKER_PATCH_VERSION),
        );
        let mut rebuilt = false;

        let (actual, recovered) =
            recover_controlled_copy(Err(CONTROLLED_COPY_MISSING_MESSAGE.to_string()), || {
                rebuilt = true;
                Ok(expected.clone())
            })
            .expect("missing controlled copy should rebuild");

        assert!(rebuilt);
        assert!(recovered);
        assert_eq!(actual.controlled_app_root, expected.controlled_app_root);

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn stale_artifact_cleanup_preserves_stable_and_running_copies() {
        let workspace = temp_workspace("stale-artifact-cleanup");
        let controlled_root = workspace.join(CONTROLLED_COPY_DIR_NAME);
        let current = controlled_root.join(CONTROLLED_CURRENT_DIR_NAME);
        let previous = controlled_root.join(CONTROLLED_PREVIOUS_DIR_NAME);
        let legacy = controlled_root.join(CONTROLLED_APP_DIR_NAME);
        let backups = controlled_root.join("backups");
        let candidate = controlled_root.join(CONTROLLED_CANDIDATE_DIR_NAME);
        let generated_stale = controlled_root.join(format!(
            "{CONTROLLED_GENERATED_CANDIDATE_DIR_PREFIX}{}",
            Uuid::new_v4()
        ));
        let generated_running = controlled_root.join(format!(
            "{CONTROLLED_GENERATED_CANDIDATE_DIR_PREFIX}{}",
            Uuid::new_v4()
        ));
        let unmanaged_candidate_name = controlled_root.join("candidate-not-a-managed-uuid");
        let unknown = controlled_root.join("keep-me");
        let staged_root = workspace.join("staged-codex");
        let running_staged_app = staged_root.join("running").join("app");
        let stale_staged = staged_root.join("stale");
        let validation = workspace.join("patch-validation-old");
        let backup_root = workspace.join("patch-backups");

        for path in [
            &current,
            &previous,
            &legacy,
            &backups,
            &candidate,
            &generated_stale,
            &generated_running.join("app"),
            &unmanaged_candidate_name,
            &unknown,
            &running_staged_app,
            &stale_staged,
            &validation,
            &backup_root,
        ] {
            fs::create_dir_all(path).expect("create cleanup fixture directory");
        }
        for name in [
            "app.asar.controlled.2026-01-01T00-00-00-000Z.bak",
            "app.asar.controlled.2026-01-02T00-00-00-000Z.bak",
            "app.asar.controlled.2026-01-03T00-00-00-000Z.bak",
            "app.asar.controlled.2026-01-04T00-00-00-000Z.bak",
            "app.asar.controlled.2026-01-04T00-00-00-000Z-1.bak",
            ".codexdeck-controlled.2026-01-01T00-00-00-000Z.json.bak",
            ".codexdeck-controlled.2026-01-02T00-00-00-000Z.json.bak",
            "notes.txt",
            "app.asar.controlled.manual.bak",
        ] {
            fs::write(backup_root.join(name), b"backup").expect("write cleanup fixture");
        }
        let staged_note = staged_root.join("README.txt");
        let validation_note = workspace.join("patch-validation-notes.txt");
        fs::write(&staged_note, b"keep staged note").expect("write staged note");
        fs::write(&validation_note, b"keep validation note").expect("write validation note");

        cleanup_stale_multimodel_artifacts_after_running_root_lookup(
            &workspace,
            Ok(vec![
                running_staged_app.clone(),
                generated_running.join("app"),
            ]),
        )
        .expect("cleanup stale multimodel artifacts");

        assert!(current.exists());
        assert!(previous.exists());
        assert!(unknown.exists());
        assert!(generated_running.exists());
        assert!(unmanaged_candidate_name.exists());
        assert!(running_staged_app.exists());
        assert!(staged_note.exists());
        assert!(validation_note.exists());
        assert!(!legacy.exists());
        assert!(!backups.exists());
        assert!(!candidate.exists());
        assert!(!generated_stale.exists());
        assert!(!stale_staged.exists());
        assert!(!validation.exists());
        assert!(!backup_root
            .join("app.asar.controlled.2026-01-01T00-00-00-000Z.bak")
            .exists());
        assert!(!backup_root
            .join("app.asar.controlled.2026-01-02T00-00-00-000Z.bak")
            .exists());
        assert!(!backup_root
            .join("app.asar.controlled.2026-01-03T00-00-00-000Z.bak")
            .exists());
        assert!(!backup_root
            .join("app.asar.controlled.2026-01-04T00-00-00-000Z.bak")
            .exists());
        assert!(backup_root
            .join("app.asar.controlled.2026-01-04T00-00-00-000Z-1.bak")
            .exists());
        assert!(!backup_root
            .join(".codexdeck-controlled.2026-01-01T00-00-00-000Z.json.bak")
            .exists());
        assert!(backup_root
            .join(".codexdeck-controlled.2026-01-02T00-00-00-000Z.json.bak")
            .exists());
        assert!(backup_root.join("notes.txt").exists());
        assert!(backup_root.join("app.asar.controlled.manual.bak").exists());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn stale_artifact_cleanup_preserves_directories_when_running_root_lookup_fails() {
        let workspace = temp_workspace("stale-artifact-cleanup-unknown-roots");
        let controlled_root = workspace.join(CONTROLLED_COPY_DIR_NAME);
        let current = controlled_root.join(CONTROLLED_CURRENT_DIR_NAME);
        let candidate = controlled_root.join(CONTROLLED_CANDIDATE_DIR_NAME);
        let staged = workspace.join(STAGED_CODEX_DIR_NAME).join("pending");

        for path in [&current, &candidate, &staged] {
            fs::create_dir_all(path).expect("create cleanup fixture directory");
        }

        let error = cleanup_stale_multimodel_artifacts_after_running_root_lookup(
            &workspace,
            Err("PowerShell query failed".to_string()),
        )
        .expect_err("cleanup must fail closed when running roots are unknown");

        assert!(error.contains("已跳过清理"));

        assert!(current.exists());
        assert!(candidate.exists());
        assert!(staged.exists());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn stale_artifact_cleanup_preserves_completed_candidate_for_retry() {
        let workspace = temp_workspace("preserve-completed-candidate");
        let candidate_app_root = workspace
            .join(CONTROLLED_COPY_DIR_NAME)
            .join(CONTROLLED_CANDIDATE_DIR_NAME)
            .join(CONTROLLED_APP_DIR_NAME);
        let candidate = create_patch_cache_fixture_at(
            &workspace,
            candidate_app_root,
            Some(MODEL_PICKER_PATCH_VERSION),
            None,
        );

        cleanup_stale_multimodel_artifacts_after_running_root_lookup(&workspace, Ok(Vec::new()))
            .expect("cleanup stale multimodel artifacts");

        assert!(candidate.controlled_app_root.exists());
        assert!(candidate.patch_state_path.exists());
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
    fn running_process_parser_accepts_codex_desktop_and_skips_child_processes() {
        let workspace = temp_workspace("running-process-parser");
        let app_root = workspace.join("app");
        let executable = app_root.join("Codex Desktop.exe");
        fs::create_dir_all(app_root.join("resources")).expect("create app resources");
        fs::write(&executable, b"test executable").expect("write executable");
        fs::write(app_root.join("resources").join("app.asar"), b"test asar")
            .expect("write app.asar");
        let raw = serde_json::json!([
            {
                "ProcessId": 101,
                "Name": "Codex Desktop.exe",
                "CommandLine": null,
                "ExecutablePath": executable,
            },
            {
                "ProcessId": 102,
                "Name": "ChatGPT.exe",
                "CommandLine": "ChatGPT.exe --type=renderer",
                "ExecutablePath": null,
            }
        ])
        .to_string();

        let roots = parse_running_windows_codex_app_dirs(&raw).expect("parse running processes");

        assert_eq!(roots, vec![app_root]);
        let _ = fs::remove_dir_all(workspace);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn running_process_parser_fails_closed_for_unreadable_process_record() {
        let raw = serde_json::json!({
            "ProcessId": 103,
            "Name": "ChatGPT.exe",
            "CommandLine": null,
            "ExecutablePath": null,
        })
        .to_string();

        let error = parse_running_windows_codex_app_dirs(&raw)
            .expect_err("unreadable process record must fail closed");

        assert!(error.contains("已拒绝清理"));
        assert!(parse_running_windows_codex_app_dirs("null").is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn running_process_parser_skips_unidentified_chatgpt_and_accepts_controlled_copy() {
        let workspace = temp_workspace("running-chatgpt-identity");
        let ordinary_root = workspace.join("ordinary-chatgpt").join("app");
        let controlled_root = workspace
            .join("controlled-codex")
            .join("current")
            .join("app");
        for root in [&ordinary_root, &controlled_root] {
            fs::create_dir_all(root.join("resources")).expect("create app resources");
            fs::write(root.join("ChatGPT.exe"), b"test executable").expect("write executable");
            fs::write(root.join("resources").join("app.asar"), b"test asar")
                .expect("write app.asar");
        }
        fs::write(controlled_copy_marker_path(&controlled_root), b"{}")
            .expect("write controlled marker");
        let raw = serde_json::json!([
            {
                "ProcessId": 201,
                "Name": "ChatGPT.exe",
                "CommandLine": null,
                "ExecutablePath": ordinary_root.join("ChatGPT.exe"),
            },
            {
                "ProcessId": 202,
                "Name": "ChatGPT.exe",
                "CommandLine": null,
                "ExecutablePath": controlled_root.join("ChatGPT.exe"),
            }
        ])
        .to_string();

        let roots = parse_running_windows_codex_app_dirs(&raw).expect("parse running processes");

        assert_eq!(roots, vec![controlled_root]);
        let _ = fs::remove_dir_all(workspace);
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
