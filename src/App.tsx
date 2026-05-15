import { Suspense, lazy, useEffect, useMemo, useState } from "react";
import "./App.css";
import { AddAccountSection } from "./components/AddAccountSection";
import { AddAccountDialog } from "./components/AddAccountDialog";
import { AccountPoolManager } from "./components/AccountPoolManager";
import { AppTopBar } from "./components/AppTopBar";
import { BottomDock } from "./components/BottomDock";
import { MetaStrip } from "./components/MetaStrip";
import { NoticeBanner } from "./components/NoticeBanner";
import { UpdateBanner } from "./components/UpdateBanner";
import { useCodexController } from "./hooks/useCodexController";
import { useI18n } from "./i18n/I18nProvider";
import { useThemeMode } from "./hooks/useThemeMode";
import type { AccountPoolConfig } from "./types/app";

type AppTab = "accounts" | "notifications" | "settings";
type NotificationViewTab = "settings" | "pipelines" | "templates" | "tests" | "activity";

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
    const [activeTab, setActiveTab] = useState<AppTab>("accounts");
    const [notificationView, setNotificationView] = useState<NotificationViewTab>("settings");
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
        <div className="shell">
            <div className="ambient" />
            <main className="panel">
                <BottomDock
                    activeTab={activeTab}
                    onSelectTab={setActiveTab}
                    notificationView={notificationView}
                    onSelectNotificationView={setNotificationView}
                />
                <div className="appMainPane">
                    <AppTopBar
                        onGoHome={() => setActiveTab("accounts")}
                    />

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

                    <section className="viewStage">
                        {activeTab === "accounts" ? (
                            <div className="accountsPage">
                                <div className="accountsHero">
                                    <MetaStrip
                                        accountCount={accounts.length}
                                        tokenUsage={tokenUsage}
                                        tokenUsageError={tokenUsageError}
                                        exportingAccounts={exportingAccounts}
                                        onExportAccounts={() => void onExportAccounts()}
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
                                    onExport={(account) => void onExportAccounts(account)}
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
                            <Suspense fallback={null}>
                                <NotificationsPanel
                                    settings={settings}
                                    saving={savingSettings}
                                    viewTab={notificationView}
                                    onViewTabChange={setNotificationView}
                                    onUpdateSettings={updateSettings}
                                />
                            </Suspense>
                        ) : (
                            <Suspense fallback={null}>
                                <SettingsPanel
                                    themeMode={themeMode}
                                    onToggleTheme={toggleTheme}
                                    checkingUpdate={checkingUpdate}
                                    onCheckUpdate={() => void checkForAppUpdate(false)}
                                    onOpenExternalUrl={(url) => void openExternalUrl(url)}
                                    settings={settings}
                                    installedEditorApps={installedEditorApps}
                                    hasOpencodeDesktopApp={hasOpencodeDesktopApp}
                                    savingSettings={savingSettings}
                                    onUpdateSettings={(patch, options) => void updateSettings(patch, options)}
                                />
                            </Suspense>
                        )}
                    </section>
                </div>
            </main>
        </div>
    );
}

export default App;
