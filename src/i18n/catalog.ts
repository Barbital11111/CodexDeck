import enUsRaw from "./locales/en-US.json";
import jaJpRaw from "./locales/ja-JP.json";
import koKrRaw from "./locales/ko-KR.json";
import ruRuRaw from "./locales/ru-RU.json";
import zhCnRaw from "./locales/zh-CN.json";

export const SUPPORTED_LOCALES = ["zh-CN", "en-US", "ja-JP", "ko-KR", "ru-RU"] as const;

export type AppLocale = (typeof SUPPORTED_LOCALES)[number];

export type LocaleOption = {
  code: AppLocale;
  shortLabel: string;
  nativeLabel: string;
};

export const LOCALE_OPTIONS: LocaleOption[] = [
  { code: "zh-CN", shortLabel: "中", nativeLabel: "中文" },
  { code: "en-US", shortLabel: "EN", nativeLabel: "English" },
  { code: "ja-JP", shortLabel: "日", nativeLabel: "日本語" },
  { code: "ko-KR", shortLabel: "한", nativeLabel: "한국어" },
  { code: "ru-RU", shortLabel: "RU", nativeLabel: "Русский" },
];

export const DEFAULT_LOCALE: AppLocale = "zh-CN";

export function isSupportedLocale(value: string | null | undefined): value is AppLocale {
  return (
    value === "zh-CN" ||
    value === "en-US" ||
    value === "ja-JP" ||
    value === "ko-KR" ||
    value === "ru-RU"
  );
}

export function getNextLocale(current: AppLocale): AppLocale {
  const index = LOCALE_OPTIONS.findIndex((item) => item.code === current);
  if (index < 0) {
    return DEFAULT_LOCALE;
  }
  return LOCALE_OPTIONS[(index + 1) % LOCALE_OPTIONS.length].code;
}

export type MessageCatalog = {
  common: {
    close: string;
    clear: string;
  };
  topBar: {
    appTitle: string;
    logoAlt: string;
    checkUpdate: string;
    checkingUpdate: string;
    manualRefresh: string;
    refreshing: string;
    openSettings: string;
    toggleLanguage: (nextLanguage: string) => string;
    languagePicker: string;
  };
  metaStrip: {
    ariaLabel: string;
    accountCount: string;
    currentActive: string;
    tokens7d: string;
    tokens30d: string;
    tokensPending: string;
    tokensUpdatedAt: string;
    tokensSources: string;
    tokensEvents: string;
    tokensFailedSources: string;
    exportAll: string;
  };
  exportDialog: {
    title: string;
    allDescription: string;
    selectedDescription: (count: number) => string;
    singleDescription: string;
    codexDeckTitle: string;
    codexDeckDescription: string;
    sub2apiTitle: string;
    sub2apiDescription: string;
    ok: string;
    cancel: string;
  };
  addAccount: {
    smartSwitch: string;
    hybridTitle: string;
    hybridChatgptLabel: string;
    hybridRelayLabel: string;
    hybridChatgptPlaceholder: string;
    hybridRelayPlaceholder: string;
    hybridStart: string;
    hybridStarting: string;
    hybridMissing: string;
    exportButton: string;
    startButton: string;
    dialogAriaLabel: string;
    dialogTitle: string;
    dialogSubtitle: string;
    reauthorizeDialogTitle: string;
    reauthorizeDialogSubtitle: (label: string) => string;
    tabsAriaLabel: string;
    oauthTab: string;
    oauthDescription: string;
    reauthorizeOauthDescription: string;
    oauthLinkLabel: string;
    oauthOpenBrowser: string;
    oauthListening: string;
    oauthCallbackLabel: string;
    oauthCallbackPlaceholder: string;
    oauthParseCallback: string;
    reauthorizeParseCallback: string;
    oauthPreparing: string;
    oauthCallbackSubmitting: string;
    currentTab: string;
    currentDescription: string;
    currentStart: string;
    currentImporting: string;
    uploadTab: string;
    uploadDescription: string;
    apiTab: string;
    apiDescription: string;
    apiProviderPresetTitle: string;
    apiProviderPresetHint: string;
    apiProviderCustomHint: string;
    apiProviderSelectedHint: (label: string) => string;
    apiNameLabel: string;
    apiNamePlaceholder: string;
    apiBaseUrlLabel: string;
    apiBaseUrlPlaceholder: string;
    apiBaseUrlHint: string;
    apiKeyLabel: string;
    apiKeyPlaceholder: string;
    apiModelLabel: string;
    apiModelPlaceholder: string;
    apiTagsLabel: string;
    apiTagsPlaceholder: string;
    apiTagsHint: string;
    apiValidationTitle: string;
    apiValidationDescription: string;
    apiValidationFailed: string;
    apiValidateAndSave: string;
    apiSaving: string;
    apiForceSave: string;
    apiQuotaToggleLabel: string;
    apiQuotaToggleHint: string;
    apiQuotaTokenTitle: string;
    apiQuotaTokenDescription: string;
    apiPlatformEmailLabel: string;
    apiPlatformEmailPlaceholder: string;
    apiPlatformPasswordLabel: string;
    apiPlatformPasswordPlaceholder: string;
    apiPlatformSyncHint: string;
    uploadChooseFiles: string;
    uploadChooseFolder: string;
    uploadNoJsonFiles: string;
    uploadFileSummary: (firstPath: string, count: number) => string;
    uploadSelectedCount: (count: number) => string;
    uploadNoFiles: string;
    uploadQueueTitle: string;
    uploadQueueEmpty: string;
    uploadImporting: string;
    uploadStartImport: string;
  };
  accountCard: {
    currentStamp: string;
    currentBadge: string;
    launch: string;
    launching: string;
    apiBadge: string;
    profileIncomplete: string;
    validationFailed: string;
    providerLabel: string;
    endpointLabel: string;
    modelLabel: string;
    balanceLabel: string;
    apiQuotaTitle: string;
    apiQuotaTodayUsed: string;
    apiQuotaRemaining: string;
    apiQuotaTotalRemaining: string;
    apiQuotaTotalTokens: string;
    apiQuotaTodayTokens: string;
    apiQuotaDailyLabel: string;
    apiQuotaTotalLabel: string;
    apiQuotaExpiresAt: string;
    apiQuotaModeLabel: string;
    apiQuotaModeApiOnly: string;
    apiQuotaModePlatformBasic: string;
    apiQuotaModePlatformSubscription: string;
    apiQuotaModeAdmin: string;
    apiQuotaDisplayTitle: string;
    apiQuotaDisplayDescription: string;
    apiQuotaDisplayAutoPlaceholder: string;
    apiQuotaDisplayLockedHint: string;
    apiDrawerTitle: string;
    apiDrawerSave: string;
    apiDrawerBasicTitle: string;
    apiDrawerBasicDescription: string;
    apiDrawerQuotaDescription: string;
    apiPasswordKeepPlaceholder: string;
    apiQuotaRecentHit: string;
    apiQuotaLastScan: string;
    apiQuotaSourceToken: string;
    apiQuotaSourcePending: string;
    apiQuotaPendingGateway: string;
    apiQuotaNeedAdminLog: string;
    apiQuotaHint: string;
    refreshApiQuota: string;
    reauthorize: string;
    actionsGroupLabel: string;
    editAlias: string;
    editApi: string;
    editTags: string;
    aliasInputLabel: string;
    apiKeyKeepPlaceholder: string;
    tagsLabel: string;
    tagsPlaceholder: string;
    tagsEmpty: string;
    saveTags: string;
    cancelTags: string;
    delete: string;
    deleteConfirm: string;
    used: string;
    remaining: string;
    resetAt: string;
    credits: string;
    unlimited: string;
    fiveHourFallback: string;
    oneWeekFallback: string;
    oneWeekLabel: string;
    hourSuffix: string;
    minuteSuffix: string;
    planLabels: Record<string, string>;
  };
  accountsGrid: {
    emptyTitle: string;
    emptyDescription: string;
  };
  accountPools: {
    title: string;
    description: string;
    create: string;
    defaultGroupName: (index: number) => string;
    addAccount: string;
    addAccountEmpty: string;
    emptyTitle: string;
    emptyDescription: string;
    groupUntitled: string;
    groupCountLabel: string;
    groupEmpty: string;
    expand: string;
    collapse: string;
    rename: string;
    reorder: string;
    refreshQuota: string;
    refreshQuotaEmpty: string;
    exportGroup: string;
    renamePlaceholder: string;
    delete: string;
    deleteConfirmTitle: string;
    deleteConfirmContent: string;
    deleteConfirmOk: string;
    deleteConfirmCancel: string;
    viewModeAriaLabel: string;
    viewModeList: string;
    viewModeCards: string;
    removeSingle: string;
    warmup: string;
    warmingUp: string;
    accountIncomplete: string;
    ungroupedTitle: string;
    ungroupedDescription: string;
  };
  bottomDock: {
    ariaLabel: string;
    accounts: string;
    proxy: string;
    settings: string;
  };
  apiProxy: {
    kicker: string;
    title: string;
    hint: string;
    statusLabel: string;
    statusRunning: string;
    statusStopped: string;
    portLabel: string;
    accountCountLabel: string;
    defaultStartLabel: string;
    defaultStartEnabled: string;
    defaultStartDisabled: string;
    portInputAriaLabel: string;
    refreshStatus: string;
    stop: string;
    stopping: string;
    start: string;
    starting: string;
    launchWithProxy: string;
    launchingWithProxy: string;
    baseUrlLabel: string;
    localBaseUrlLabel: string;
    lanBaseUrlLabel: string;
    copy: string;
    baseUrlPlaceholder: string;
    apiKeyLabel: string;
    refreshKey: string;
    refreshingKey: string;
    apiKeyPlaceholder: string;
    activeAccountLabel: string;
    activeAccountEmptyTitle: string;
    activeAccountEmptyDescription: string;
    lastErrorLabel: string;
    none: string;
    poolTitle: string;
    poolDescription: string;
    poolCountLabel: string;
    poolEmptyTitle: string;
    poolEmptyDescription: string;
    poolModeLabel: string;
    poolModeSticky: string;
    poolModeSequential: string;
    poolModeHybrid: string;
    poolModeStickyDescription: string;
    poolModeSequentialDescription: string;
    poolModeHybridDescription: string;
    poolPriorityLabel: string;
    poolPriorityAuto: string;
    poolMoveUp: string;
    poolMoveDown: string;
    sequenceCursorLabel: string;
    sequenceCursorEmpty: string;
    poolAccountTypeChatgpt: string;
    poolAccountTypeRelay: string;
    poolAccountCurrent: string;
    poolAccountBlocked: string;
    poolAccountIncomplete: string;
    poolRelayResponses: string;
    poolUsageFallback: string;
    poolSelected: string;
    poolNotSelected: string;
    poolImportLabel: string;
    poolImportPlaceholder: string;
    poolImportAppend: string;
    poolImportReplace: string;
    poolImportEmpty: string;
    poolImportHint: string;
    threadRoutesTitle: string;
    threadRoutesCountLabel: string;
    threadRoutesDescriptionSticky: string;
    threadRoutesDescriptionSequential: string;
    threadRoutesDescriptionHybrid: string;
    threadRoutesEmptyTitle: string;
    threadRoutesEmptyDescription: string;
    threadRoutesNoMatchTitle: string;
    threadRoutesNoMatchDescription: string;
    activeRequestsTitle: string;
    activeRequestsCountLabel: string;
    activeRequestsEmptyTitle: string;
    activeRequestsEmptyDescription: string;
    activeRequestThreadLabel: string;
    activeRequestModelLabel: string;
    activeRequestEndpointLabel: string;
    activeRequestStartedLabel: string;
    activeRequestRunningBadge: string;
    threadFilterLabel: string;
    threadFilterAll: string;
    threadFilterManual: string;
    threadFilterAuto: string;
    threadSearchLabel: string;
    threadSearchPlaceholder: string;
    threadSessionLabel: string;
    threadAccountLabel: string;
    threadLastSeenLabel: string;
    threadSourceAuto: string;
    threadSourceManual: string;
    threadMissingAccount: string;
    threadRestoreAuto: string;
    threadRestoreSequence: string;
    threadBindingUnavailable: string;
    threadApplying: string;
    threadAutoHint: string;
    upstreamsTitle: string;
    upstreamsCountLabel: string;
    upstreamsDescription: string;
    upstreamsEmptyTitle: string;
    upstreamsEmptyDescription: string;
    upstreamAvailableKeysLabel: string;
    upstreamProviderLabel: string;
    upstreamModelLabel: string;
    upstreamPriorityLabel: string;
    upstreamWeightLabel: string;
    keySelectionModeLabel: string;
    keySelectionRoundRobin: string;
    keySelectionRandom: string;
    keySelectionFixedPriority: string;
    endpointCapabilitiesLabel: string;
    endpointResponses: string;
    endpointResponsesCompact: string;
    endpointChatCompletions: string;
    endpointRealtime: string;
    endpointRealtimeUnavailable: string;
    upstreamStatusLabel: string;
    upstreamKeyLabel: string;
    upstreamLastErrorLabel: string;
    upstreamCooldownLabel: string;
    upstreamLastUsedLabel: string;
    routingEventsTitle: string;
    routingEventsCountLabel: string;
    routingEventsDescription: string;
    routingEventsEmptyTitle: string;
    routingEventsEmptyDescription: string;
    routingEventResultSuccess: string;
    routingEventResultFailed: string;
    routingEventResultCooldownBlocked: string;
    routingEventCandidatesLabel: string;
    routingEventReasonLabel: string;
    routingEventTimeLabel: string;
    editKeys: string;
    saveKeys: string;
    savingKeys: string;
    cancelEditKeys: string;
    addKey: string;
    bulkAddKeys: string;
    bulkKeyPlaceholder: string;
    probeKey: string;
    probingKey: string;
    removeKey: string;
    keyLabelPlaceholder: string;
    keySecretPlaceholder: string;
    keyEnabledLabel: string;
    healthHealthy: string;
    healthDegraded: string;
    healthCoolingDown: string;
    healthDisabled: string;
    healthAuthFailed: string;
    healthQuotaExhausted: string;
    remoteKicker: string;
    remoteTitle: string;
    remoteDescription: string;
    remoteHistoryTitle: string;
    remoteAddServer: string;
    remoteExpand: string;
    remoteCollapse: string;
    remoteEmptyTitle: string;
    remoteEmptyDescription: string;
    remoteNameLabel: string;
    remoteHostLabel: string;
    remoteSshPortLabel: string;
    remoteUserLabel: string;
    remoteAuthLabel: string;
    remoteIdentityFileLabel: string;
    remoteIdentityFilePlaceholder: string;
    remotePickIdentityFile: string;
    remoteDirLabel: string;
    remoteListenPortLabel: string;
    remoteAuthKeyContent: string;
    remoteAuthKeyFile: string;
    remoteAuthKeyPath: string;
    remoteAuthPassword: string;
    remotePrivateKeyLabel: string;
    remotePrivateKeyPlaceholder: string;
    remotePasswordLabel: string;
    remotePasswordPlaceholder: string;
    remoteConfigTitle: string;
    remoteSave: string;
    remoteRemove: string;
    remoteDeploy: string;
    remoteDeploying: string;
    remoteRefresh: string;
    remoteRefreshing: string;
    remoteStart: string;
    remoteStarting: string;
    remoteStop: string;
    remoteStopping: string;
    remoteInstalledLabel: string;
    remoteInstalledYes: string;
    remoteInstalledNo: string;
    remoteSystemdLabel: string;
    remoteEnabledLabel: string;
    remoteRunningLabel: string;
    remotePidLabel: string;
    remoteServiceLabel: string;
    remoteBaseUrlLabel: string;
    remoteApiKeyLabel: string;
    remoteLogsLabel: string;
    remoteLogsEmpty: string;
    remoteReadLogs: string;
    remoteReadingLogs: string;
    remoteLastErrorLabel: string;
    remoteStatusUnknown: string;
    remoteLastCheckedLabel: string;
    remoteNeverChecked: string;
    remoteGuideSetupTitle: string;
    remoteGuideSetupDescription: string;
    remoteGuideDeployTitle: string;
    remoteGuideDeployDescription: string;
    remoteGuideStartTitle: string;
    remoteGuideStartDescription: string;
    remoteGuideReadyTitle: string;
    remoteGuideReadyDescription: string;
    remoteDeployProgressTitle: (label: string) => string;
    remoteDeployStageValidating: string;
    remoteDeployStageDetectingPlatform: string;
    remoteDeployStagePreparingBuilder: string;
    remoteDeployStageBuildingBinary: string;
    remoteDeployStagePreparingFiles: string;
    remoteDeployStageUploadingBinary: string;
    remoteDeployStageUploadingAccounts: string;
    remoteDeployStageUploadingService: string;
    remoteDeployStageInstallingService: string;
    remoteDeployStageVerifying: string;
    cloudflaredKicker: string;
    cloudflaredTitle: string;
    cloudflaredDescription: string;
    cloudflaredToggle: string;
    startLocalProxyFirstTitle: string;
    startLocalProxyFirstDescription: string;
    notInstalledLabel: string;
    installTitle: string;
    installDescription: string;
    installing: string;
    installButton: string;
    quickModeLabel: string;
    quickModeTitle: string;
    quickModeDescription: string;
    namedModeLabel: string;
    namedModeTitle: string;
    namedModeDescription: string;
    quickNoteTitle: string;
    quickNoteBody: string;
    apiTokenLabel: string;
    apiTokenPlaceholder: string;
    accountIdLabel: string;
    accountIdPlaceholder: string;
    zoneIdLabel: string;
    zoneIdPlaceholder: string;
    hostnameLabel: string;
    hostnamePlaceholder: string;
    useHttp2: string;
    refreshPublicStatus: string;
    stopPublic: string;
    stoppingPublic: string;
    startPublic: string;
    startingPublic: string;
    publicStatusLabel: string;
    publicStatusRunning: string;
    publicStatusStopped: string;
    publicStatusRunningDescription: string;
    publicStatusStoppedDescription: string;
    publicUrlLabel: string;
    installPathLabel: string;
    notDetected: string;
  };
  settings: {
    dialogAriaLabel: string;
    title: string;
    subtitle: string;
    languageSubtitle: string;
    close: string;
    launchAtStartup: {
      label: string;
      description: string;
      checkedText: string;
      uncheckedText: string;
    };
    launchCodexAfterSwitch: {
      label: string;
      description: string;
      checkedText: string;
      uncheckedText: string;
    };
    smartSwitchIncludeApi: {
      label: string;
      checkedText: string;
      uncheckedText: string;
    };
    autoRefresh: {
      label: string;
      description: string;
      checkedText: string;
      uncheckedText: string;
    };
    autoRefreshInterval: {
      label: string;
      groupAriaLabel: string;
    };
    apiQuotaAutoRefresh: {
      label: string;
      description: string;
      checkedText: string;
      uncheckedText: string;
    };
    apiQuotaAutoRefreshInterval: {
      label: string;
      groupAriaLabel: string;
    };
    quotaAlert: {
      label: string;
      description: string;
      checkedText: string;
      uncheckedText: string;
    };
    quotaAlertFiveHourThreshold: {
      label: string;
      groupAriaLabel: string;
    };
    quotaAlertOneWeekThreshold: {
      label: string;
      groupAriaLabel: string;
    };
    codexLaunchPath: {
      label: string;
    };
    syncOpencode: {
      label: string;
      description: string;
      checkedText: string;
      uncheckedText: string;
    };
    restartOpencodeDesktop: {
      label: string;
      checkedText: string;
      uncheckedText: string;
    };
    restartEditorsOnSwitch: {
      label: string;
      description: string;
      checkedText: string;
      uncheckedText: string;
    };
    restartEditorTargets: {
      label: string;
      description: string;
    };
    noSupportedEditors: string;
    trayUsageDisplay: {
      label: string;
      description: string;
      groupAriaLabel: string;
      remaining: string;
      used: string;
      hidden: string;
    };
    theme: {
      label: string;
      description: string;
      switchAriaLabel: string;
      dark: string;
      light: string;
    };
    projectInfo: {
      versionLabel: string;
      repositoryLabel: string;
      releasesLabel: string;
      openRepository: string;
      openIssues: string;
      openReleases: string;
      openChangelog: string;
    };
  };
  editorPicker: {
    ariaLabel: string;
    placeholder: string;
  };
  editorAppLabels: Record<string, string>;
  updateDialog: {
    ariaLabel: string;
    title: (version: string) => string;
    subtitle: (currentVersion: string) => string;
    close: string;
    publishedAt: (date: string) => string;
    statusReady: string;
    statusInstalling: string;
    manualDownload: string;
    skipThisVersion: string;
    installNow: string;
    installingNow: string;
  };
  notices: {
    settingsUpdated: string;
    updateSettingsFailed: (error: string) => string;
    usageRefreshed: string;
    groupUsageRefreshed: (count: number) => string;
    groupUsageRefreshNoNativeAccounts: string;
    refreshFailed: (error: string) => string;
    preparingUpdateDownload: string;
    alreadyLatest: string;
    updateDownloadStarted: string;
    updateDownloadingPercent: (percent: number) => string;
    updateDownloading: string;
    updateDownloadFinished: string;
    updateInstalling: string;
    updateInstallFailed: (error: string) => string;
    foundNewVersion: (version: string, currentVersion: string) => string;
    updateCheckFailed: (error: string) => string;
    updateCheckFailedWithUpdaterHint: (error: string) => string;
    openExternalFailed: (error: string) => string;
    openManualDownloadFailed: (error: string) => string;
    oauthLinkPrepareFailed: (error: string) => string;
    oauthImportPrefix: string;
    currentAccountImportSuccess: string;
    currentAccountImportFailed: (error: string) => string;
    apiAccountCreated: (label: string) => string;
    apiAccountCreateFailed: (error: string) => string;
    apiAccountUpdated: (label: string) => string;
    apiAccountUpdateFailed: (error: string) => string;
    apiAccountKeysUpdated: (label: string) => string;
    apiAccountKeysUpdateFailed: (error: string) => string;
    apiAccountKeyProbeHealthy: (label: string) => string;
    apiAccountKeyProbeFailed: (error: string) => string;
    apiQuotaRefreshed: (count: number) => string;
    apiQuotaRefreshFailed: (error: string) => string;
    apiQuotaRefreshNoBoundAccounts: string;
    profileIntegrityWarning: (count: number) => string;
    accountAliasUpdated: (label: string) => string;
    accountAliasUpdateFailed: (error: string) => string;
    accountTagsUpdated: (label: string) => string;
    accountTagsUpdateFailed: (error: string) => string;
    quotaAlertCurrentLow: (label: string, summary: string, suggestion: string) => string;
    accountsExported: string;
    accountsExportFailed: (error: string) => string;
    deleteConfirm: (label: string) => string;
    accountDeleted: string;
    deleteFailed: (error: string) => string;
    switchedOnly: string;
    switchedAndLaunchByCli: string;
    switchedAndLaunching: string;
    hybridSwitchedOnly: string;
    hybridSwitchedAndLaunchByCli: string;
    hybridSwitchedAndLaunching: string;
    opencodeSyncFailed: (base: string, error: string) => string;
    opencodeSynced: (base: string) => string;
    opencodeDesktopRestartFailed: (base: string, error: string) => string;
    opencodeDesktopRestarted: (base: string) => string;
    editorRestartFailed: (base: string, error: string) => string;
    editorsRestarted: (base: string, labels: string) => string;
    noEditorRestarted: (base: string) => string;
    switchFailed: (error: string) => string;
    smartSwitchNoTarget: string;
    smartSwitchAlreadyBest: string;
    fileImportPrefix: string;
    importFilesRequired: string;
    importFailedPlain: (prefix: string, error: string) => string;
    importFailedWithSource: (prefix: string, source: string, error: string) => string;
    importFailedNoValidJson: (prefix: string) => string;
    importSummaryAdded: (count: number) => string;
    importSummaryUpdated: (count: number) => string;
    importSummaryFailed: (count: number) => string;
    importSummaryFirstFailure: (source: string, error: string) => string;
    importSummaryDone: (prefix: string, summary: string, suffix: string) => string;
    proxyLocalTargetFallback: string;
    proxyStarted: (target: string) => string;
    proxyStartFailed: (error: string) => string;
    proxyLaunched: string;
    proxyLaunchedByCli: string;
    proxyLaunchFailed: (error: string) => string;
    proxyStopped: string;
    proxyStopFailed: (error: string) => string;
    proxyKeyRefreshed: string;
    proxyKeyRefreshFailed: (error: string) => string;
    proxyThreadBindingFailed: (error: string) => string;
    installingDependency: (name: string) => string;
    dependencyInstalled: (name: string) => string;
    dependencyInstallFailed: (name: string, error: string) => string;
    remoteStatusFailed: (label: string, error: string) => string;
    remoteProxyDeployed: (label: string) => string;
    remoteProxyDeployFailed: (label: string, error: string) => string;
    remoteProxyStarted: (label: string) => string;
    remoteProxyStartFailed: (label: string, error: string) => string;
    remoteProxyStopped: (label: string) => string;
    remoteProxyStopFailed: (label: string, error: string) => string;
    remoteLogsFailed: (label: string, error: string) => string;
    pickIdentityFileFailed: (error: string) => string;
    cloudflaredInstalled: string;
    cloudflaredInstallFailed: (error: string) => string;
    cloudflaredPublicUrlFallback: string;
    cloudflaredStarted: (target: string) => string;
    cloudflaredStartFailed: (error: string) => string;
    cloudflaredStopped: string;
    cloudflaredStopFailed: (error: string) => string;
  };
};

type Rawify<T> = T extends (...args: infer _Args) => string
  ? string
  : T extends Record<string, unknown>
    ? { [K in keyof T]: Rawify<T[K]> }
    : T;

type RawMessageCatalog = Rawify<MessageCatalog>;

function fillTemplate(template: string, values: Record<string, string | number>): string {
  return template.replace(/\{\{\s*([a-zA-Z0-9_]+)\s*\}\}/g, (_, key: string) => {
    const value = values[key];
    return value === undefined ? "" : String(value);
  });
}

function compileLocale(raw: RawMessageCatalog): MessageCatalog {
  const zhFallback = zhCnRaw as RawMessageCatalog;
  const settingsRaw = {
    ...zhFallback.settings,
    ...raw.settings,
  };
  const noticesRaw = {
    ...zhFallback.notices,
    ...raw.notices,
  };
  const addAccountRaw = {
    ...zhFallback.addAccount,
    ...raw.addAccount,
  };
  const apiProxyRaw = {
    ...zhFallback.apiProxy,
    ...raw.apiProxy,
  };
  return {
    common: raw.common,
    topBar: {
      ...raw.topBar,
      toggleLanguage: (nextLanguage) => fillTemplate(raw.topBar.toggleLanguage, { nextLanguage }),
    },
    metaStrip: raw.metaStrip,
    exportDialog: {
      ...zhFallback.exportDialog,
      ...raw.exportDialog,
      selectedDescription: (count) =>
        fillTemplate(raw.exportDialog.selectedDescription, { count }),
    },
    addAccount: {
      ...addAccountRaw,
      reauthorizeDialogSubtitle: (label) =>
        fillTemplate(addAccountRaw.reauthorizeDialogSubtitle, { label }),
      apiProviderSelectedHint: (label) =>
        fillTemplate(addAccountRaw.apiProviderSelectedHint, { label }),
      uploadFileSummary: (firstPath, count) =>
        fillTemplate(addAccountRaw.uploadFileSummary, {
          firstPath,
          count,
          remainingCount: Math.max(count - 1, 0),
        }),
      uploadSelectedCount: (count) => fillTemplate(addAccountRaw.uploadSelectedCount, { count }),
    },
    accountCard: {
      ...zhFallback.accountCard,
      ...raw.accountCard,
    },
    accountsGrid: raw.accountsGrid,
    accountPools: {
      ...zhFallback.accountPools,
      ...raw.accountPools,
      defaultGroupName: (index) =>
        fillTemplate(raw.accountPools.defaultGroupName, { index }),
    },
    bottomDock: raw.bottomDock,
    apiProxy: {
      ...apiProxyRaw,
      remoteDeployProgressTitle: (label) =>
        fillTemplate(apiProxyRaw.remoteDeployProgressTitle, { label }),
    },
    settings: {
      ...settingsRaw,
    },
    editorPicker: raw.editorPicker,
    editorAppLabels: raw.editorAppLabels,
    updateDialog: {
      ...raw.updateDialog,
      title: (version) => fillTemplate(raw.updateDialog.title, { version }),
      subtitle: (currentVersion) =>
        fillTemplate(raw.updateDialog.subtitle, { currentVersion }),
      publishedAt: (date) => fillTemplate(raw.updateDialog.publishedAt, { date }),
    },
    notices: {
      ...noticesRaw,
      updateSettingsFailed: (error) => fillTemplate(noticesRaw.updateSettingsFailed, { error }),
      groupUsageRefreshed: (count) =>
        fillTemplate(noticesRaw.groupUsageRefreshed, { count }),
      refreshFailed: (error) => fillTemplate(noticesRaw.refreshFailed, { error }),
      updateDownloadingPercent: (percent) =>
        fillTemplate(noticesRaw.updateDownloadingPercent, { percent }),
      updateInstallFailed: (error) => fillTemplate(noticesRaw.updateInstallFailed, { error }),
      foundNewVersion: (version, currentVersion) =>
        fillTemplate(noticesRaw.foundNewVersion, { version, currentVersion }),
      updateCheckFailed: (error) => fillTemplate(noticesRaw.updateCheckFailed, { error }),
      updateCheckFailedWithUpdaterHint: (error) =>
        fillTemplate(noticesRaw.updateCheckFailedWithUpdaterHint, { error }),
      openExternalFailed: (error) => fillTemplate(noticesRaw.openExternalFailed, { error }),
      openManualDownloadFailed: (error) =>
        fillTemplate(noticesRaw.openManualDownloadFailed, { error }),
      oauthLinkPrepareFailed: (error) =>
        fillTemplate(noticesRaw.oauthLinkPrepareFailed, { error }),
      currentAccountImportFailed: (error) =>
        fillTemplate(noticesRaw.currentAccountImportFailed, { error }),
      apiAccountCreated: (label) => fillTemplate(noticesRaw.apiAccountCreated, { label }),
      apiAccountCreateFailed: (error) =>
        fillTemplate(noticesRaw.apiAccountCreateFailed, { error }),
      apiAccountUpdated: (label) => fillTemplate(noticesRaw.apiAccountUpdated, { label }),
      apiAccountUpdateFailed: (error) =>
        fillTemplate(noticesRaw.apiAccountUpdateFailed, { error }),
      apiAccountKeysUpdated: (label) =>
        fillTemplate(noticesRaw.apiAccountKeysUpdated, { label }),
      apiAccountKeysUpdateFailed: (error) =>
        fillTemplate(noticesRaw.apiAccountKeysUpdateFailed, { error }),
      apiAccountKeyProbeHealthy: (label) =>
        fillTemplate(noticesRaw.apiAccountKeyProbeHealthy, { label }),
      apiAccountKeyProbeFailed: (error) =>
        fillTemplate(noticesRaw.apiAccountKeyProbeFailed, { error }),
      apiQuotaRefreshed: (count) => fillTemplate(noticesRaw.apiQuotaRefreshed, { count }),
      apiQuotaRefreshFailed: (error) =>
        fillTemplate(noticesRaw.apiQuotaRefreshFailed, { error }),
      profileIntegrityWarning: (count) =>
        fillTemplate(noticesRaw.profileIntegrityWarning, { count }),
      accountAliasUpdated: (label) => fillTemplate(noticesRaw.accountAliasUpdated, { label }),
      accountAliasUpdateFailed: (error) =>
        fillTemplate(noticesRaw.accountAliasUpdateFailed, { error }),
      accountTagsUpdated: (label) => fillTemplate(noticesRaw.accountTagsUpdated, { label }),
      accountTagsUpdateFailed: (error) =>
        fillTemplate(noticesRaw.accountTagsUpdateFailed, { error }),
      quotaAlertCurrentLow: (label, summary, suggestion) =>
        fillTemplate(noticesRaw.quotaAlertCurrentLow, { label, summary, suggestion }).trim(),
      accountsExportFailed: (error) =>
        fillTemplate(noticesRaw.accountsExportFailed, { error }),
      deleteConfirm: (label) => fillTemplate(noticesRaw.deleteConfirm, { label }),
      deleteFailed: (error) => fillTemplate(noticesRaw.deleteFailed, { error }),
      opencodeSyncFailed: (base, error) =>
        fillTemplate(noticesRaw.opencodeSyncFailed, { base, error }),
      opencodeSynced: (base) => fillTemplate(noticesRaw.opencodeSynced, { base }),
      opencodeDesktopRestartFailed: (base, error) =>
        fillTemplate(noticesRaw.opencodeDesktopRestartFailed, { base, error }),
      opencodeDesktopRestarted: (base) =>
        fillTemplate(noticesRaw.opencodeDesktopRestarted, { base }),
      editorRestartFailed: (base, error) =>
        fillTemplate(noticesRaw.editorRestartFailed, { base, error }),
      editorsRestarted: (base, labels) =>
        fillTemplate(noticesRaw.editorsRestarted, { base, labels }),
      noEditorRestarted: (base) => fillTemplate(noticesRaw.noEditorRestarted, { base }),
      switchFailed: (error) => fillTemplate(noticesRaw.switchFailed, { error }),
      importFailedPlain: (prefix, error) =>
        fillTemplate(noticesRaw.importFailedPlain, { prefix, error }),
      importFailedWithSource: (prefix, source, error) =>
        fillTemplate(noticesRaw.importFailedWithSource, { prefix, source, error }),
      importFailedNoValidJson: (prefix) =>
        fillTemplate(noticesRaw.importFailedNoValidJson, { prefix }),
      importSummaryAdded: (count) => fillTemplate(noticesRaw.importSummaryAdded, { count }),
      importSummaryUpdated: (count) => fillTemplate(noticesRaw.importSummaryUpdated, { count }),
      importSummaryFailed: (count) => fillTemplate(noticesRaw.importSummaryFailed, { count }),
      importSummaryFirstFailure: (source, error) =>
        fillTemplate(noticesRaw.importSummaryFirstFailure, { source, error }),
      importSummaryDone: (prefix, summary, suffix) =>
        fillTemplate(noticesRaw.importSummaryDone, { prefix, summary, suffix }).trim(),
      proxyStarted: (target) => fillTemplate(noticesRaw.proxyStarted, { target }),
      proxyStartFailed: (error) => fillTemplate(noticesRaw.proxyStartFailed, { error }),
      proxyLaunchFailed: (error) => fillTemplate(noticesRaw.proxyLaunchFailed, { error }),
      proxyStopFailed: (error) => fillTemplate(noticesRaw.proxyStopFailed, { error }),
      proxyKeyRefreshFailed: (error) =>
        fillTemplate(noticesRaw.proxyKeyRefreshFailed, { error }),
      proxyThreadBindingFailed: (error) =>
        fillTemplate(noticesRaw.proxyThreadBindingFailed, { error }),
      installingDependency: (name) =>
        fillTemplate(noticesRaw.installingDependency, { name }),
      dependencyInstalled: (name) =>
        fillTemplate(noticesRaw.dependencyInstalled, { name }),
      dependencyInstallFailed: (name, error) =>
        fillTemplate(noticesRaw.dependencyInstallFailed, { name, error }),
      remoteStatusFailed: (label, error) =>
        fillTemplate(noticesRaw.remoteStatusFailed, { label, error }),
      remoteProxyDeployed: (label) => fillTemplate(noticesRaw.remoteProxyDeployed, { label }),
      remoteProxyDeployFailed: (label, error) =>
        fillTemplate(noticesRaw.remoteProxyDeployFailed, { label, error }),
      remoteProxyStarted: (label) => fillTemplate(noticesRaw.remoteProxyStarted, { label }),
      remoteProxyStartFailed: (label, error) =>
        fillTemplate(noticesRaw.remoteProxyStartFailed, { label, error }),
      remoteProxyStopped: (label) => fillTemplate(noticesRaw.remoteProxyStopped, { label }),
      remoteProxyStopFailed: (label, error) =>
        fillTemplate(noticesRaw.remoteProxyStopFailed, { label, error }),
      remoteLogsFailed: (label, error) =>
        fillTemplate(noticesRaw.remoteLogsFailed, { label, error }),
      pickIdentityFileFailed: (error) =>
        fillTemplate(noticesRaw.pickIdentityFileFailed, { error }),
      cloudflaredInstallFailed: (error) =>
        fillTemplate(noticesRaw.cloudflaredInstallFailed, { error }),
      cloudflaredStarted: (target) => fillTemplate(noticesRaw.cloudflaredStarted, { target }),
      cloudflaredStartFailed: (error) =>
        fillTemplate(noticesRaw.cloudflaredStartFailed, { error }),
      cloudflaredStopFailed: (error) =>
        fillTemplate(noticesRaw.cloudflaredStopFailed, { error }),
    },
  };
}

export const MESSAGES: Record<AppLocale, MessageCatalog> = {
  "zh-CN": compileLocale(zhCnRaw as unknown as RawMessageCatalog),
  "en-US": compileLocale(enUsRaw as unknown as RawMessageCatalog),
  "ja-JP": compileLocale(jaJpRaw as unknown as RawMessageCatalog),
  "ko-KR": compileLocale(koKrRaw as unknown as RawMessageCatalog),
  "ru-RU": compileLocale(ruRuRaw as unknown as RawMessageCatalog),
};
