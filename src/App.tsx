import { Suspense, lazy, useEffect, useMemo, useState } from "react";
import { Modal, Radio, Typography } from "antd";
import "./App.css";
import { AddAccountSection } from "./components/AddAccountSection";
import { AddAccountDialog } from "./components/AddAccountDialog";
import { AccountPoolManager } from "./components/AccountPoolManager";
import { BottomDock } from "./components/BottomDock";
import { HybridLaunchPanel } from "./components/HybridLaunchPanel";
import { MetaStrip } from "./components/MetaStrip";
import { NoticeBanner } from "./components/NoticeBanner";
import { UpdateBanner } from "./components/UpdateBanner";
import { WindowTitleBar } from "./components/WindowTitleBar";
import { useCodexController } from "./hooks/useCodexController";
import { useI18n } from "./i18n/I18nProvider";
import { useThemeMode } from "./hooks/useThemeMode";
import type { AccountPoolConfig, AccountSummary, AccountsExportFormat } from "./types/app";

type AppTab = "accounts" | "notifications" | "settings";
type NotificationViewTab = "settings" | "pipelines" | "templates" | "tests" | "activity";
const BROWSER_PREVIEW_WINDOW_PARAM = "codexdeckPreviewWindow";

const NotificationsPanel = lazy(() =>
    import("./components/NotificationsPanel").then((module) => ({
        default: module.NotificationsPanel,
    })),
);

const SettingsPanel = lazy(() =>
    import("./components/SettingsPanel").then((module) => ({
        default: module.SettingsPanel,
    })),
);

type ExportDialogState = {
    account?: AccountSummary;
    accountKeys?: string[];
};

function createLocalId(prefix: string) {
    if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
        return `${prefix}-${crypto.randomUUID()}`;
    }
    return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function sortAndNormalizeAccountKeys(
    keys: string[],
    activeAccountKeys: Set<string>,
) {
    return Array.from(new Set(keys)).filter((accountKey) => activeAccountKeys.has(accountKey));
}

function normalizeAccountPools(
    accountPools: AccountPoolConfig[],
    activeAccountKeys: Set<string>,
) {
    const assigned = new Set<string>();

    return accountPools.map((pool) => {
        const nextKeys: string[] = [];
        for (const accountKey of pool.accountKeys) {
            if (!activeAccountKeys.has(accountKey) || assigned.has(accountKey)) {
                continue;
            }
            assigned.add(accountKey);
            nextKeys.push(accountKey);
        }

        return {
            ...pool,
            accountKeys: nextKeys,
        };
    });
}

function normalizeApiQuotaProviderBaseUrl(value: string | null | undefined) {
    return (value ?? "")
        .trim()
        .replace(/\/+$/, "")
        .toLowerCase()
        .replace(/\/api\/v1$/i, "")
        .replace(/\/v1$/i, "");
}

function App() {
    if (shouldRenderBrowserPreviewWindow()) {
        return <BrowserPreviewWindow />;
    }

    return <CodexDeckApp />;
}

function hasTauriRuntime() {
    return (
        typeof window !== "undefined" &&
        Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__)
    );
}

function shouldRenderBrowserPreviewWindow() {
    if (typeof window === "undefined" || hasTauriRuntime() || !import.meta.env.DEV) {
        return false;
    }

    const params = new URLSearchParams(window.location.search);
    return !params.has(BROWSER_PREVIEW_WINDOW_PARAM);
}

function BrowserPreviewWindow() {
    const previewUrl = useMemo(() => {
        const url = new URL(window.location.href);
        url.searchParams.set(BROWSER_PREVIEW_WINDOW_PARAM, "1");
        return `${url.pathname}${url.search}${url.hash}`;
    }, []);

    return (
        <div className="browserPreviewHost">
            <div className="browserPreviewWindow">
                <iframe
                    title="CodexDeck browser preview"
                    src={previewUrl}
                    allow="clipboard-read; clipboard-write"
                />
            </div>
        </div>
    );
}

function CodexDeckApp() {
    const [activeTab, setActiveTab] = useState<AppTab>("accounts");
    const [notificationView, setNotificationView] = useState<NotificationViewTab>("settings");
    const [exportDialog, setExportDialog] = useState<ExportDialogState | null>(null);
    const [exportFormat, setExportFormat] = useState<AccountsExportFormat>("codexDeck");
    const tauriRuntime = hasTauriRuntime();
    const { copy } = useI18n();
    const { themeMode, toggleTheme } = useThemeMode();
    const {
        accounts,
        tokenUsage,
        tokenUsageError,
        loading,
        addDialogOpen,
        reauthorizeAccount,
        importingAccounts,
        oauthWaitingForCallback,
        exportingAccounts,
        switchingId,
        renamingAccountId,
        pendingDeleteId,
        checkingUpdate,
        installingUpdate,
        updateProgress,
        pendingUpdate,
        updateDialogOpen,
        skipPendingUpdateVersion,
        notice,
        hideAccountDetails,
        setHideAccountDetails,
        openExternalUrl,
        settings,
        installedEditorApps,
        hasOpencodeDesktopApp,
        savingSettings,
        refreshUsage,
        refreshUsageForAccountKeys,
        refreshApiQuotaForAccountKeys,
        refreshTokenUsage,
        checkForAppUpdate,
        installPendingUpdate,
        openManualDownloadPage,
        closeUpdateDialog,
        updateSettings,
        onOpenAddDialog,
        onReauthorizeAccount,
        onPrepareOauthLogin,
        onOpenOauthAuthorizationPage,
        onCloseAddDialog,
        onCancelOauthLogin,
        onCompleteOauthCallbackLogin,
        onImportCurrentAuth,
        onCreateApiAccount,
        onUpdateApiAccount,
        onUpdateAccountTags,
        onImportAuthFiles,
        onExportAccounts,
        onRenameAccountLabel,
        onDelete,
        onSwitch,
        onSwitchHybrid,
        onSmartSwitch,
        smartSwitching,
    } = useCodexController();

    const activeAccountKeys = useMemo(
        () => new Set(accounts.map((account) => account.accountKey)),
        [accounts],
    );

    const groupedAccountKeys = useMemo(
        () => new Set(settings.accountPools.flatMap((pool) => pool.accountKeys)),
        [settings.accountPools],
    );

    const ungroupedAccounts = useMemo(
        () => accounts.filter((account) => !groupedAccountKeys.has(account.accountKey)),
        [accounts, groupedAccountKeys],
    );

    const persistAccountPools = (accountPools: AccountPoolConfig[]) =>
        void updateSettings(
            { accountPools: normalizeAccountPools(accountPools, activeAccountKeys) },
            { silent: true, keepInteractive: true },
        );

    const openExportDialog = (account?: AccountSummary) => {
        setExportFormat("codexDeck");
        setExportDialog({ account });
    };

    const openBulkExportDialog = (accountKeys: string[]) => {
        const normalizedKeys = Array.from(
            new Set(accountKeys.map((accountKey) => accountKey.trim()).filter(Boolean)),
        );
        if (normalizedKeys.length === 0) {
            return;
        }
        setExportFormat("sub2api");
        setExportDialog({ accountKeys: normalizedKeys });
    };

    const closeExportDialog = () => {
        if (!exportingAccounts) {
            setExportDialog(null);
        }
    };

    const confirmExportDialog = async () => {
        const target = exportDialog;
        if (!target || exportingAccounts) {
            return;
        }
        await onExportAccounts(target.account, exportFormat, target.accountKeys);
        setExportDialog(null);
    };

    const reassignAccountKeysToPool = (poolId: string, accountKeys: string[]) => {
        const normalizedKeys = sortAndNormalizeAccountKeys(accountKeys, activeAccountKeys);
        if (normalizedKeys.length === 0) {
            return settings.accountPools;
        }

        return settings.accountPools.map((pool) => {
            const remainingKeys = pool.accountKeys.filter(
                (accountKey) => !normalizedKeys.includes(accountKey),
            );

            if (pool.id !== poolId) {
                return {
                    ...pool,
                    accountKeys: remainingKeys,
                };
            }

            return {
                ...pool,
                accountKeys: [...remainingKeys, ...normalizedKeys],
            };
        });
    };

    const createAccountPool = () => {
        const nextIndex = settings.accountPools.length + 1;
        persistAccountPools([
            ...settings.accountPools,
            {
                id: createLocalId("pool"),
                name: copy.accountPools.defaultGroupName(nextIndex),
                accountKeys: [],
                collapsed: false,
            },
        ]);
    };

    const updateAccountPool = (poolId: string, updater: (pool: AccountPoolConfig) => AccountPoolConfig) => {
        persistAccountPools(
            settings.accountPools.map((pool) => (pool.id === poolId ? updater(pool) : pool)),
        );
    };

    const assignAccountToPool = (accountKey: string, poolId: string) => {
        if (!poolId || !activeAccountKeys.has(accountKey)) {
            return;
        }
        persistAccountPools(reassignAccountKeysToPool(poolId, [accountKey]));
    };

    const removeAccountFromAllPools = (accountKey: string) => {
        if (!activeAccountKeys.has(accountKey)) {
            return;
        }
        persistAccountPools(
            settings.accountPools.map((pool) => ({
                ...pool,
                accountKeys: pool.accountKeys.filter((item) => item !== accountKey),
            })),
        );
    };

    useEffect(() => {
        const isMac =
            typeof navigator !== "undefined" &&
            /Mac|iPhone|iPad|iPod/i.test(navigator.platform);
        const onKeyDown = (event: KeyboardEvent) => {
            const key = event.key.toLowerCase();
            if (key !== "r") {
                return;
            }
            const isTrigger = isMac ? event.metaKey : event.ctrlKey;
            if (!isTrigger) {
                return;
            }
            event.preventDefault();
            void refreshUsage(false);
            void refreshApiQuotaForAccountKeys(
                accounts
                    .filter(
                        (account) =>
                            account.sourceKind === "relay" &&
                            settings.notificationProviders.some((provider) => {
                                return (
                                    normalizeApiQuotaProviderBaseUrl(provider.baseUrl) ===
                                        normalizeApiQuotaProviderBaseUrl(account.apiBaseUrl) &&
                                    Boolean(provider.email.trim()) &&
                                    Boolean(provider.password?.trim())
                                );
                            }),
                    )
                    .map((account) => account.accountKey),
                { quiet: true },
            );
            void refreshTokenUsage(false);
        };

        window.addEventListener("keydown", onKeyDown);
        return () => {
            window.removeEventListener("keydown", onKeyDown);
        };
    }, [accounts, refreshApiQuotaForAccountKeys, refreshTokenUsage, refreshUsage, settings.notificationProviders]);

    return (
        <div className={`shell${tauriRuntime ? " shellHasWindowTitleBar" : ""}`}>
            <div className="ambient" />
            <WindowTitleBar visible={tauriRuntime} />
            <main className="panel">
                <BottomDock
                    activeTab={activeTab}
                    onSelectTab={setActiveTab}
                    notificationView={notificationView}
                    onSelectNotificationView={setNotificationView}
                />
                <div className="appMainPane">
                    <AddAccountDialog
                        open={addDialogOpen}
                        reauthorizeAccount={reauthorizeAccount}
                        importingAccounts={importingAccounts}
                        oauthWaitingForCallback={oauthWaitingForCallback}
                        onPrepareOauth={onPrepareOauthLogin}
                        onOpenOauthPage={onOpenOauthAuthorizationPage}
                        onCompleteOauth={onCompleteOauthCallbackLogin}
                        onCancelOauth={onCancelOauthLogin}
                        onImportCurrentAuth={onImportCurrentAuth}
                        onCreateApiAccount={onCreateApiAccount}
                        onImportFiles={onImportAuthFiles}
                        onClose={onCloseAddDialog}
                    />

                    <NoticeBanner notice={notice} />
                    <UpdateBanner
                        open={updateDialogOpen}
                        pendingUpdate={pendingUpdate}
                        updateProgress={updateProgress}
                        installingUpdate={installingUpdate}
                        onClose={closeUpdateDialog}
                        onManualDownload={() => void openManualDownloadPage()}
                        onSkipVersion={() => void skipPendingUpdateVersion()}
                        onInstallNow={() => void installPendingUpdate()}
                    />
                    <Modal
                        title={copy.exportDialog.title}
                        open={exportDialog !== null}
                        onOk={() => void confirmExportDialog()}
                        onCancel={closeExportDialog}
                        okText={copy.exportDialog.ok}
                        cancelText={copy.exportDialog.cancel}
                        confirmLoading={exportingAccounts}
                        destroyOnHidden
                    >
                        <div className="exportFormatDialog">
                            <Typography.Text type="secondary">
                                {exportDialog?.account
                                    ? copy.exportDialog.singleDescription
                                    : exportDialog?.accountKeys?.length
                                      ? copy.exportDialog.selectedDescription(
                                            exportDialog.accountKeys.length,
                                        )
                                    : copy.exportDialog.allDescription}
                            </Typography.Text>
                            <Radio.Group
                                className="exportFormatOptions"
                                value={exportFormat}
                                onChange={(event) =>
                                    setExportFormat(event.target.value as AccountsExportFormat)
                                }
                                options={[
                                    {
                                        value: "codexDeck",
                                        label: (
                                            <span className="exportFormatOption">
                                                <strong>{copy.exportDialog.codexDeckTitle}</strong>
                                                <span>{copy.exportDialog.codexDeckDescription}</span>
                                            </span>
                                        ),
                                    },
                                    {
                                        value: "sub2api",
                                        label: (
                                            <span className="exportFormatOption">
                                                <strong>{copy.exportDialog.sub2apiTitle}</strong>
                                                <span>{copy.exportDialog.sub2apiDescription}</span>
                                            </span>
                                        ),
                                    },
                                ]}
                            />
                        </div>
                    </Modal>

                    <section className="viewStage">
                        {activeTab === "accounts" ? (
                            <div className="accountsPage">
                                <div className="accountsHero">
                                    <MetaStrip
                                        accountCount={accounts.length}
                                        tokenUsage={tokenUsage}
                                        tokenUsageError={tokenUsageError}
                                        exportingAccounts={exportingAccounts}
                                        onExportAccounts={() => openExportDialog()}
                                    />
                                    <AddAccountSection
                                        onOpenAddDialog={onOpenAddDialog}
                                        onCreatePool={createAccountPool}
                                        onSmartSwitch={() => void onSmartSwitch()}
                                        saving={savingSettings}
                                        smartSwitching={smartSwitching}
                                        hideAccountDetails={hideAccountDetails}
                                        onToggleHideAccountDetails={() =>
                                            setHideAccountDetails((current) => !current)
                                        }
                                    />
                                    <HybridLaunchPanel
                                        accounts={accounts}
                                        switchingId={switchingId}
                                        hideAccountDetails={hideAccountDetails}
                                        onSwitchHybrid={(chatgptAccount, relayAccount) =>
                                            void onSwitchHybrid(chatgptAccount, relayAccount)
                                        }
                                    />
                                </div>
                                <AccountPoolManager
                                    accounts={accounts}
                                    ungroupedAccounts={ungroupedAccounts}
                                    loading={loading}
                                    accountPools={settings.accountPools}
                                    saving={savingSettings}
                                    exportingAccounts={exportingAccounts}
                                    switchingId={switchingId}
                                    renamingAccountId={renamingAccountId}
                                    pendingDeleteId={pendingDeleteId}
                                    notificationProviders={settings.notificationProviders}
                                    usageDisplayMode={settings.trayUsageDisplayMode}
                                    hideAccountDetails={hideAccountDetails}
                                    apiEnhancedLaunchEnabled={settings.apiEnhancedLaunchEnabled}
                                    onRenamePool={(poolId, name) =>
                                        updateAccountPool(poolId, (pool) => ({ ...pool, name }))
                                    }
                                    onDeletePool={(poolId) =>
                                        persistAccountPools(
                                            settings.accountPools.filter((pool) => pool.id !== poolId),
                                        )
                                    }
                                    onTogglePoolCollapsed={(poolId, collapsed) =>
                                        updateAccountPool(poolId, (pool) => ({
                                            ...pool,
                                            collapsed,
                                        }))
                                    }
                                    onReorderPool={(poolId, accountKeys) =>
                                        updateAccountPool(poolId, (pool) => ({
                                            ...pool,
                                            accountKeys: sortAndNormalizeAccountKeys(accountKeys, activeAccountKeys),
                                        }))
                                    }
                                    onRefreshPoolUsage={(accountKeys, apiAccountKeys) => {
                                        if (accountKeys.length > 0) {
                                            void refreshUsageForAccountKeys(accountKeys, {
                                                notice: copy.notices.groupUsageRefreshed(accountKeys.length),
                                            });
                                        }
                                        if (apiAccountKeys.length > 0) {
                                            void refreshApiQuotaForAccountKeys(apiAccountKeys, {
                                                notice: copy.notices.apiQuotaRefreshed(apiAccountKeys.length),
                                            });
                                        }
                                    }}
                                    onAssignAccountToPool={assignAccountToPool}
                                    onRemoveAccountFromAllPools={removeAccountFromAllPools}
                                    onExportAccountKeys={openBulkExportDialog}
                                    onExport={(account) => openExportDialog(account)}
                                    onReauthorize={(account) => void onReauthorizeAccount(account)}
                                    onRename={(account, label) => onRenameAccountLabel(account, label)}
                                    onUpdateApiAccount={(account, input) =>
                                        onUpdateApiAccount(account, input)
                                    }
                                    onUpdateTags={(account, value) => onUpdateAccountTags(account, value)}
                                    onRefreshApiQuota={(account) =>
                                        void refreshApiQuotaForAccountKeys([account.accountKey], {
                                            notice: copy.notices.apiQuotaRefreshed(1),
                                        })
                                    }
                                    onSwitch={(account) => void onSwitch(account)}
                                    onDelete={(account) => void onDelete(account)}
                                />

                            </div>
                        ) : activeTab === "notifications" ? (
                            <Suspense fallback={<ViewLoadingFallback />}>
                                <NotificationsPanel
                                    settings={settings}
                                    saving={savingSettings}
                                    viewTab={notificationView}
                                    onViewTabChange={setNotificationView}
                                    onUpdateSettings={updateSettings}
                                />
                            </Suspense>
                        ) : (
                            <Suspense fallback={<ViewLoadingFallback />}>
                                <SettingsPanel
                                    themeMode={themeMode}
                                    onToggleTheme={toggleTheme}
                                    checkingUpdate={checkingUpdate}
                                    onCheckUpdate={() => void checkForAppUpdate(false)}
                                    onOpenExternalUrl={(url) => void openExternalUrl(url)}
                                    settings={settings}
                                    installedEditorApps={installedEditorApps}
                                    hasOpencodeDesktopApp={hasOpencodeDesktopApp}
                                    onUpdateSettings={updateSettings}
                                />
                            </Suspense>
                        )}
                    </section>
                </div>
            </main>
        </div>
    );
}

function ViewLoadingFallback() {
    return <div className="viewLoadingFallback">加载中...</div>;
}

export default App;
