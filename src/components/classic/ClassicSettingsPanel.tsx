import { useEffect, useState, type ComponentProps } from "react";
import { Button as AntButton, Modal } from "antd";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import {
  PROJECT_CHANGELOG_URL,
  PROJECT_ISSUES_URL,
  PROJECT_RELEASES_URL,
  PROJECT_REPOSITORY_DISPLAY,
  PROJECT_REPOSITORY_URL,
} from "../../constants/externalLinks";
import { useI18n } from "../../i18n/I18nProvider";
import { EditorMultiSelect } from "../EditorMultiSelect";
import { ThemeSwitch } from "../ThemeSwitch";
import { SwitchField } from "../SwitchField";
import type {
  AppSettings,
  InstalledEditorApp,
  ThemeMode,
  UpdateSettingsOptions,
} from "../../types/app";

function GitHubIcon() {
  return (
    <svg className="settingLinkIcon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path
        fill="currentColor"
        d="M12 1.5a10.5 10.5 0 0 0-3.32 20.46c.52.1.7-.22.7-.5v-1.86c-2.86.62-3.46-1.2-3.46-1.2-.48-1.18-1.16-1.5-1.16-1.5-.96-.66.08-.64.08-.64 1.04.08 1.6 1.08 1.6 1.08.94 1.58 2.44 1.12 3.04.86.1-.68.36-1.12.66-1.38-2.28-.26-4.68-1.12-4.68-5a3.9 3.9 0 0 1 1.04-2.72c-.1-.26-.46-1.32.1-2.74 0 0 .86-.28 2.82 1.04a9.8 9.8 0 0 1 5.14 0c1.96-1.32 2.82-1.04 2.82-1.04.56 1.42.2 2.48.1 2.74a3.9 3.9 0 0 1 1.04 2.72c0 3.88-2.4 4.74-4.7 4.98.38.32.7.94.7 1.92v2.84c0 .28.18.62.72.5A10.5 10.5 0 0 0 12 1.5Z"
      />
    </svg>
  );
}

type SettingsPanelProps = {
  themeMode: ThemeMode;
  onToggleTheme: () => void;
  checkingUpdate: boolean;
  onCheckUpdate: () => void;
  onOpenExternalUrl: (url: string) => void;
  settings: AppSettings;
  installedEditorApps: InstalledEditorApp[];
  hasOpencodeDesktopApp: boolean;
  onReloadSettings: () => void | Promise<void>;
  onUpdateSettings: (
    patch: Partial<AppSettings>,
    options?: UpdateSettingsOptions,
  ) => void | Promise<void>;
};

type SettingPendingKey = keyof AppSettings;

type MultiModelModeResult = {
  enabled: boolean;
  status: string;
  workspace: string;
  restorePoint?: string | null;
  message: string;
};

function isMultiModelModeActive(settings: AppSettings) {
  return (
    settings.codexMultiModelModeEnabled &&
    !["unsupported", "failed", "reset"].includes(settings.codexMultiModelStatus ?? "")
  );
}

function multiModelStatusLabel(status?: string | null) {
  switch (status) {
    case "restore-point-ready":
      return "已创建恢复点";
    case "controlled-copy-ready":
      return "已准备";
    case "enabled":
      return "已启用";
    case "updated":
      return "已更新";
    case "fallback-previous":
      return "使用旧副本";
    case "source-check-unavailable":
      return "版本检查失败";
    case "unsupported":
      return "版本不适配";
    case "reset":
      return "已重置";
    case "failed":
      return "启用失败";
    default:
      return "未启用";
  }
}

function Button(props: ComponentProps<typeof AntButton>) {
  return <AntButton autoInsertSpace={false} {...props} />;
}

export function ClassicSettingsPanel({
  themeMode,
  onToggleTheme,
  checkingUpdate,
  onCheckUpdate,
  onOpenExternalUrl,
  settings,
  installedEditorApps,
  hasOpencodeDesktopApp,
  onReloadSettings,
  onUpdateSettings,
}: SettingsPanelProps) {
  const { copy, locale, localeOptions, setLocale } = useI18n();
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [pickingCodexLaunchPathKind, setPickingCodexLaunchPathKind] = useState<"file" | "directory" | null>(null);
  const [pendingSettings, setPendingSettings] = useState<Partial<Record<SettingPendingKey, boolean>>>({});
  const [multiModelPending, setMultiModelPending] = useState(false);
  const languageLabel = copy.topBar.languagePicker;
  const languageOptions = localeOptions.map((item) => ({
    id: item.code,
    label: item.nativeLabel,
  }));
  const autoRefreshOptions = [30, 60, 120, 300];
  const apiQuotaRefreshOptions = [600, 900, 1800, 3600];
  const quotaThresholdOptions = [10, 15, 20, 30, 40];
  const isSettingPending = (...keys: SettingPendingKey[]) =>
    keys.some((key) => Boolean(pendingSettings[key]));
  const updateSetting = (
    keys: SettingPendingKey | SettingPendingKey[],
    patch: Partial<AppSettings>,
    options?: UpdateSettingsOptions,
  ) => {
    const pendingKeys = Array.isArray(keys) ? keys : [keys];
    setPendingSettings((current) => {
      const next = { ...current };
      for (const key of pendingKeys) {
        next[key] = true;
      }
      return next;
    });

    const updateOptions = {
      ...options,
      keepInteractive: options?.keepInteractive ?? true,
    };

    return Promise.resolve()
      .then(() => onUpdateSettings(patch, updateOptions))
      .finally(() => {
        setPendingSettings((current) => {
          const next = { ...current };
          for (const key of pendingKeys) {
            delete next[key];
          }
          return next;
        });
      });
  };
  const multiModelActive = isMultiModelModeActive(settings);
  const versionValue = appVersion ? `v${appVersion}` : "...";
  const optionButtonType = (active: boolean): "primary" | "default" => (active ? "primary" : "default");

  useEffect(() => {
    let cancelled = false;

    void getVersion()
      .then((version) => {
        if (!cancelled) {
          setAppVersion(version);
        }
      })
      .catch(() => {});

    return () => {
      cancelled = true;
    };
  }, []);

  const pickCodexLaunchPath = async (kind: "file" | "directory") => {
    if (isSettingPending("codexLaunchPath") || pickingCodexLaunchPathKind) {
      return;
    }

    setPickingCodexLaunchPathKind(kind);
    try {
      const selected = await invoke<string | null>("pick_codex_launch_path", {
        kind,
        currentPath: settings.codexLaunchPath,
      });
      if (!selected) {
        return;
      }
      await updateSetting("codexLaunchPath", { codexLaunchPath: selected });
    } finally {
      setPickingCodexLaunchPathKind(null);
    }
  };

  const refreshSettingsAfterMultiModelAction = async () => {
    await Promise.resolve(onReloadSettings());
  };

  const enableMultiModelMode = () => {
    if (multiModelPending || multiModelActive) {
      return;
    }

    Modal.confirm({
      title: "当前 Codex 是否可以正常启动并使用？",
      content: null,
      okText: "是",
      cancelText: "否",
      centered: true,
      onOk: async () => {
        setMultiModelPending(true);
        try {
          await invoke<MultiModelModeResult>("enable_codex_multi_model_mode");
          await refreshSettingsAfterMultiModelAction();
        } catch (error) {
          await refreshSettingsAfterMultiModelAction();
          Modal.error({
            title: "多模型模式开启失败",
            content: String(error),
            centered: true,
          });
        } finally {
          setMultiModelPending(false);
        }
      },
    });
  };

  const resetMultiModelMode = async () => {
    if (multiModelPending) {
      return;
    }
    setMultiModelPending(true);
    try {
      await invoke<MultiModelModeResult>("reset_codex_multi_model_mode");
      await refreshSettingsAfterMultiModelAction();
    } catch (error) {
      await refreshSettingsAfterMultiModelAction();
      Modal.error({
        title: "多模型模式重置失败",
        content: String(error),
        centered: true,
      });
    } finally {
      setMultiModelPending(false);
    }
  };

  return (
    <section className="settingsPage" aria-label={copy.settings.title}>
      <div className="settingsShell">
        <div className="settingsGroup">
          <div className="settingRow">
            <div className="settingMeta">
              <strong>{languageLabel}</strong>
            </div>
            <EditorMultiSelect
              options={languageOptions}
              value={locale}
              className="languagePicker"
              ariaLabel={languageLabel}
              placeholder={languageLabel}
              onChange={setLocale}
            />
          </div>

          <div className="settingRow">
            <div className="settingMeta">
              <strong>{copy.settings.theme.label}</strong>
            </div>
            <ThemeSwitch themeMode={themeMode} onToggle={onToggleTheme} />
          </div>

          <div className="settingRow">
            <div className="settingMeta">
              <strong>{copy.settings.trayUsageDisplay.label}</strong>
            </div>
            <div className="modeGroup" role="radiogroup" aria-label={copy.settings.trayUsageDisplay.groupAriaLabel}>
              <Button
                type={optionButtonType(settings.trayUsageDisplayMode === "remaining")}
                disabled={isSettingPending("trayUsageDisplayMode")}
                loading={isSettingPending("trayUsageDisplayMode")}
                onClick={() => void updateSetting("trayUsageDisplayMode", { trayUsageDisplayMode: "remaining" })}
                aria-pressed={settings.trayUsageDisplayMode === "remaining"}
              >
                {copy.settings.trayUsageDisplay.remaining}
              </Button>
              <Button
                type={optionButtonType(settings.trayUsageDisplayMode === "used")}
                disabled={isSettingPending("trayUsageDisplayMode")}
                loading={isSettingPending("trayUsageDisplayMode")}
                onClick={() => void updateSetting("trayUsageDisplayMode", { trayUsageDisplayMode: "used" })}
                aria-pressed={settings.trayUsageDisplayMode === "used"}
              >
                {copy.settings.trayUsageDisplay.used}
              </Button>
              <Button
                type={optionButtonType(settings.trayUsageDisplayMode === "hidden")}
                disabled={isSettingPending("trayUsageDisplayMode")}
                loading={isSettingPending("trayUsageDisplayMode")}
                onClick={() => void updateSetting("trayUsageDisplayMode", { trayUsageDisplayMode: "hidden" })}
                aria-pressed={settings.trayUsageDisplayMode === "hidden"}
              >
                {copy.settings.trayUsageDisplay.hidden}
              </Button>
            </div>
          </div>
        </div>

        <div className="settingsGroup">
          <SwitchField
            checked={settings.launchAtStartup}
            onChange={(checked) => void updateSetting("launchAtStartup", { launchAtStartup: checked })}
            label={copy.settings.launchAtStartup.label}
            checkedText={copy.settings.launchAtStartup.checkedText}
            uncheckedText={copy.settings.launchAtStartup.uncheckedText}
            disabled={isSettingPending("launchAtStartup")}
            loading={isSettingPending("launchAtStartup")}
          />

          <SwitchField
            checked={settings.smartSwitchIncludeApi}
            onChange={(checked) => void updateSetting("smartSwitchIncludeApi", { smartSwitchIncludeApi: checked })}
            label={copy.settings.smartSwitchIncludeApi.label}
            checkedText={copy.settings.smartSwitchIncludeApi.checkedText}
            uncheckedText={copy.settings.smartSwitchIncludeApi.uncheckedText}
            disabled={isSettingPending("smartSwitchIncludeApi")}
            loading={isSettingPending("smartSwitchIncludeApi")}
          />

          <SwitchField
            checked={settings.codexModelInstructionsFixEnabled}
            onChange={(checked) =>
              void updateSetting("codexModelInstructionsFixEnabled", {
                codexModelInstructionsFixEnabled: checked,
              })
            }
            label="降智修复"
            checkedText="已开启"
            uncheckedText="已关闭"
            disabled={isSettingPending("codexModelInstructionsFixEnabled")}
            loading={isSettingPending("codexModelInstructionsFixEnabled")}
          />

          <SwitchField
            checked={settings.codexDisableGpuAcceleration}
            onChange={(checked) =>
              void updateSetting("codexDisableGpuAcceleration", {
                codexDisableGpuAcceleration: checked,
              })
            }
            label="禁用 Codex GPU 加速"
            checkedText="已禁用"
            uncheckedText="使用默认"
            disabled={isSettingPending("codexDisableGpuAcceleration")}
            loading={isSettingPending("codexDisableGpuAcceleration")}
          />

          <div className="settingRow">
            <div className="settingMeta">
              <strong>多模型模式</strong>
              <span className="settingValueMuted">
                {multiModelStatusLabel(settings.codexMultiModelStatus)}
              </span>
            </div>
            <div className="settingActionGroup">
              {settings.codexMultiModelRestorePoint && !multiModelActive ? (
                <Button
                  danger
                  disabled={multiModelPending}
                  loading={multiModelPending}
                  onClick={() => void resetMultiModelMode()}
                >
                  重置到可用状态
                </Button>
              ) : null}
              <Button
                type={multiModelActive ? "primary" : "default"}
                disabled={multiModelPending}
                loading={multiModelPending}
                onClick={multiModelActive ? () => void resetMultiModelMode() : enableMultiModelMode}
              >
                {multiModelActive ? "关闭" : "开启"}
              </Button>
            </div>
          </div>

          <SwitchField
            checked={settings.usageAutoRefreshEnabled}
            onChange={(checked) => void updateSetting("usageAutoRefreshEnabled", { usageAutoRefreshEnabled: checked })}
            label={copy.settings.autoRefresh.label}
            checkedText={copy.settings.autoRefresh.checkedText}
            uncheckedText={copy.settings.autoRefresh.uncheckedText}
            disabled={isSettingPending("usageAutoRefreshEnabled")}
            loading={isSettingPending("usageAutoRefreshEnabled")}
          />

          {settings.usageAutoRefreshEnabled ? (
            <div className="settingRow settingRowCompact settingRowNested">
              <div className="settingMeta">
                <strong>{copy.settings.autoRefreshInterval.label}</strong>
              </div>
              <div
                className="modeGroup"
                role="radiogroup"
                aria-label={copy.settings.autoRefreshInterval.groupAriaLabel}
              >
                {autoRefreshOptions.map((seconds) => (
                  <Button
                    key={seconds}
                    type={optionButtonType(settings.usageAutoRefreshIntervalSecs === seconds)}
                    disabled={isSettingPending("usageAutoRefreshIntervalSecs")}
                    loading={isSettingPending("usageAutoRefreshIntervalSecs")}
                    onClick={() =>
                      void updateSetting(
                        "usageAutoRefreshIntervalSecs",
                        { usageAutoRefreshIntervalSecs: seconds },
                        { silent: true, keepInteractive: true },
                      )}
                    aria-pressed={settings.usageAutoRefreshIntervalSecs === seconds}
                  >
                    {seconds}s
                  </Button>
                ))}
              </div>
            </div>
          ) : null}

          <SwitchField
            checked={settings.apiQuotaAutoRefreshEnabled}
            onChange={(checked) =>
              void updateSetting("apiQuotaAutoRefreshEnabled", { apiQuotaAutoRefreshEnabled: checked })
            }
            label={copy.settings.apiQuotaAutoRefresh.label}
            checkedText={copy.settings.apiQuotaAutoRefresh.checkedText}
            uncheckedText={copy.settings.apiQuotaAutoRefresh.uncheckedText}
            disabled={isSettingPending("apiQuotaAutoRefreshEnabled")}
            loading={isSettingPending("apiQuotaAutoRefreshEnabled")}
          />

          {settings.apiQuotaAutoRefreshEnabled ? (
            <div className="settingRow settingRowCompact settingRowNested">
              <div className="settingMeta">
                <strong>{copy.settings.apiQuotaAutoRefreshInterval.label}</strong>
              </div>
              <div
                className="modeGroup"
                role="radiogroup"
                aria-label={copy.settings.apiQuotaAutoRefreshInterval.groupAriaLabel}
              >
                {apiQuotaRefreshOptions.map((seconds) => (
                  <Button
                    key={seconds}
                    type={optionButtonType(settings.apiQuotaAutoRefreshIntervalSecs === seconds)}
                    disabled={isSettingPending("apiQuotaAutoRefreshIntervalSecs")}
                    loading={isSettingPending("apiQuotaAutoRefreshIntervalSecs")}
                    onClick={() =>
                      void updateSetting(
                        "apiQuotaAutoRefreshIntervalSecs",
                        { apiQuotaAutoRefreshIntervalSecs: seconds },
                        { silent: true, keepInteractive: true },
                      )}
                    aria-pressed={settings.apiQuotaAutoRefreshIntervalSecs === seconds}
                  >
                    {seconds < 3600 ? `${seconds / 60}m` : "1h"}
                  </Button>
                ))}
              </div>
            </div>
          ) : null}

          <SwitchField
            checked={settings.quotaAlertEnabled}
            onChange={(checked) => void updateSetting("quotaAlertEnabled", { quotaAlertEnabled: checked })}
            label={copy.settings.quotaAlert.label}
            checkedText={copy.settings.quotaAlert.checkedText}
            uncheckedText={copy.settings.quotaAlert.uncheckedText}
            disabled={isSettingPending("quotaAlertEnabled")}
            loading={isSettingPending("quotaAlertEnabled")}
          />

          {settings.quotaAlertEnabled ? (
            <>
              <div className="settingRow settingRowCompact settingRowNested">
                <div className="settingMeta">
                  <strong>{copy.settings.quotaAlertFiveHourThreshold.label}</strong>
                </div>
                <div
                  className="modeGroup"
                  role="radiogroup"
                  aria-label={copy.settings.quotaAlertFiveHourThreshold.groupAriaLabel}
                >
                  {quotaThresholdOptions.map((value) => (
                    <Button
                      key={`five-${value}`}
                      type={optionButtonType(settings.quotaAlertFiveHourThreshold === value)}
                      disabled={isSettingPending("quotaAlertFiveHourThreshold")}
                      loading={isSettingPending("quotaAlertFiveHourThreshold")}
                      onClick={() =>
                        void updateSetting(
                          "quotaAlertFiveHourThreshold",
                          { quotaAlertFiveHourThreshold: value },
                          { silent: true, keepInteractive: true },
                        )}
                      aria-pressed={settings.quotaAlertFiveHourThreshold === value}
                    >
                      {value}%
                    </Button>
                  ))}
                </div>
              </div>

              <div className="settingRow settingRowCompact settingRowNested">
                <div className="settingMeta">
                  <strong>{copy.settings.quotaAlertOneWeekThreshold.label}</strong>
                </div>
                <div
                  className="modeGroup"
                  role="radiogroup"
                  aria-label={copy.settings.quotaAlertOneWeekThreshold.groupAriaLabel}
                >
                  {quotaThresholdOptions.map((value) => (
                    <Button
                      key={`week-${value}`}
                      type={optionButtonType(settings.quotaAlertOneWeekThreshold === value)}
                      disabled={isSettingPending("quotaAlertOneWeekThreshold")}
                      loading={isSettingPending("quotaAlertOneWeekThreshold")}
                      onClick={() =>
                        void updateSetting(
                          "quotaAlertOneWeekThreshold",
                          { quotaAlertOneWeekThreshold: value },
                          { silent: true, keepInteractive: true },
                        )}
                      aria-pressed={settings.quotaAlertOneWeekThreshold === value}
                    >
                      {value}%
                    </Button>
                  ))}
                </div>
              </div>
            </>
          ) : null}

          <div className="settingRow">
            <div className="settingMeta">
              <strong>{copy.settings.codexLaunchPath.label}</strong>
            </div>
            <div className="settingFieldGroup">
              {settings.codexLaunchPath ? (
                <span className="settingPathValue">{settings.codexLaunchPath}</span>
              ) : null}
              <div className="settingActionGroup">
                {settings.codexLaunchPath ? (
                  <Button
                    className="settingPathClearButton"
                    aria-label={copy.common.clear}
                    disabled={isSettingPending("codexLaunchPath") || pickingCodexLaunchPathKind !== null}
                    loading={isSettingPending("codexLaunchPath")}
                    onClick={() => void updateSetting("codexLaunchPath", { codexLaunchPath: null })}
                  >
                    ×
                  </Button>
                ) : null}
                <Button
                  disabled={isSettingPending("codexLaunchPath") || pickingCodexLaunchPathKind !== null}
                  loading={pickingCodexLaunchPathKind === "file"}
                  onClick={() => {
                    void pickCodexLaunchPath("file");
                  }}
                >
                  {copy.addAccount.uploadChooseFiles}
                </Button>
                <Button
                  disabled={isSettingPending("codexLaunchPath") || pickingCodexLaunchPathKind !== null}
                  loading={pickingCodexLaunchPathKind === "directory"}
                  onClick={() => {
                    void pickCodexLaunchPath("directory");
                  }}
                >
                  {copy.addAccount.uploadChooseFolder}
                </Button>
              </div>
            </div>
          </div>

          <SwitchField
            checked={settings.syncOpencodeOpenaiAuth}
            onChange={(checked) => void updateSetting("syncOpencodeOpenaiAuth", { syncOpencodeOpenaiAuth: checked })}
            label={copy.settings.syncOpencode.label}
            checkedText={copy.settings.syncOpencode.checkedText}
            uncheckedText={copy.settings.syncOpencode.uncheckedText}
            disabled={isSettingPending("syncOpencodeOpenaiAuth")}
            loading={isSettingPending("syncOpencodeOpenaiAuth")}
          />

          {settings.syncOpencodeOpenaiAuth && hasOpencodeDesktopApp ? (
            <SwitchField
              checked={settings.restartOpencodeDesktopOnSwitch}
              onChange={(checked) =>
                void updateSetting("restartOpencodeDesktopOnSwitch", { restartOpencodeDesktopOnSwitch: checked })
              }
              label={copy.settings.restartOpencodeDesktop.label}
              checkedText={copy.settings.restartOpencodeDesktop.checkedText}
              uncheckedText={copy.settings.restartOpencodeDesktop.uncheckedText}
              disabled={isSettingPending("restartOpencodeDesktopOnSwitch")}
              loading={isSettingPending("restartOpencodeDesktopOnSwitch")}
              rowClassName="settingRowCompact settingRowNested"
            />
          ) : null}

          <SwitchField
            checked={settings.restartEditorsOnSwitch}
            onChange={(checked) => {
              if (checked && settings.restartEditorTargets.length === 0 && installedEditorApps.length > 0) {
                void updateSetting(
                  ["restartEditorsOnSwitch", "restartEditorTargets"],
                  {
                    restartEditorsOnSwitch: true,
                    restartEditorTargets: [installedEditorApps[0].id],
                  },
                );
                return;
              }
              void updateSetting("restartEditorsOnSwitch", { restartEditorsOnSwitch: checked });
            }}
            label={copy.settings.restartEditorsOnSwitch.label}
            checkedText={copy.settings.restartEditorsOnSwitch.checkedText}
            uncheckedText={copy.settings.restartEditorsOnSwitch.uncheckedText}
            disabled={isSettingPending("restartEditorsOnSwitch", "restartEditorTargets")}
            loading={isSettingPending("restartEditorsOnSwitch", "restartEditorTargets")}
          />

          {settings.restartEditorsOnSwitch ? (
            <div className="settingRow settingRowCompact settingRowNested">
              <div className="settingMeta">
                <strong>{copy.settings.restartEditorTargets.label}</strong>
              </div>
              {installedEditorApps.length > 0 ? (
                <EditorMultiSelect
                  options={installedEditorApps}
                  value={settings.restartEditorTargets[0] ?? null}
                  onChange={(selected) =>
                    void updateSetting(
                      "restartEditorTargets",
                      { restartEditorTargets: [selected] },
                      { silent: true, keepInteractive: true },
                    )
                  }
                />
              ) : (
                <span className="settingValueMuted">{copy.settings.noSupportedEditors}</span>
              )}
            </div>
          ) : null}
        </div>

        <div className="settingsGroup">
          <div className="settingRow">
            <div className="settingMeta settingMetaInline">
              <strong>{copy.settings.projectInfo.versionLabel}</strong>
              <span className="settingInlineValue">{versionValue}</span>
            </div>
            <div className="settingActionGroup">
              <Button type="primary" onClick={onCheckUpdate} loading={checkingUpdate} disabled={checkingUpdate}>
                {checkingUpdate ? copy.topBar.checkingUpdate : copy.topBar.checkUpdate}
              </Button>
            </div>
          </div>

          <div className="settingRow">
            <a
              className="settingLink"
              href={PROJECT_REPOSITORY_URL}
              title={PROJECT_REPOSITORY_DISPLAY}
              onClick={(event) => {
                event.preventDefault();
                onOpenExternalUrl(PROJECT_REPOSITORY_URL);
              }}
            >
              <GitHubIcon />
              <span className="settingLinkLabel">{PROJECT_REPOSITORY_DISPLAY}</span>
            </a>
            <div className="settingActionGroup">
              <Button onClick={() => onOpenExternalUrl(PROJECT_ISSUES_URL)}>
                {copy.settings.projectInfo.openIssues}
              </Button>
            </div>
          </div>

          <div className="settingRow">
            <div className="settingMeta">
              <strong>{copy.settings.projectInfo.releasesLabel}</strong>
            </div>
            <div className="settingActionGroup">
              <Button onClick={() => onOpenExternalUrl(PROJECT_RELEASES_URL)}>
                {copy.settings.projectInfo.openReleases}
              </Button>
              <Button onClick={() => onOpenExternalUrl(PROJECT_CHANGELOG_URL)}>
                {copy.settings.projectInfo.openChangelog}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
