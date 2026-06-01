import type { AppLocale } from "../i18n/catalog";

export type UsageWindow = {
  usedPercent: number;
  windowSeconds: number;
  resetAt: number | null;
};

export type CreditSnapshot = {
  hasCredits: boolean;
  unlimited: boolean;
  balance: string | null;
};

export type UsageSnapshot = {
  fetchedAt: number;
  planType: string | null;
  fiveHour: UsageWindow | null;
  oneWeek: UsageWindow | null;
  credits: CreditSnapshot | null;
};

export type CodexTokenTotals = {
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  totalTokens: number;
};

export type CodexTokenUsageSnapshot = {
  updatedAt: number;
  sourcePathCount: number;
  failedPathCount: number;
  eventCount: number;
  last7d: CodexTokenTotals;
  last30d: CodexTokenTotals;
};

export type AccountSourceKind = "chatgpt" | "relay";

export type ApiQuotaMode = "apiOnly" | "platformBasic" | "platformSubscription" | "admin";

export type AccountsExportFormat = "codexDeck" | "sub2api";

export type AccountSummary = {
  id: string;
  label: string;
  sourceKind: AccountSourceKind;
  email: string | null;
  accountKey: string;
  accountId: string;
  planType: string | null;
  apiBaseUrl: string | null;
  modelName: string | null;
  balanceText: string | null;
  balanceDisplayEnabled: boolean;
  apiQuotaMode: ApiQuotaMode;
  apiQuotaTodayUsedText?: string | null;
  apiQuotaRemainingText?: string | null;
  apiQuotaTotalRemainingText?: string | null;
  apiQuotaTotalTokensText?: string | null;
  apiQuotaTodayTokensText?: string | null;
  apiQuotaDailyWindow?: UsageWindow | null;
  apiQuotaTotalWindow?: UsageWindow | null;
  apiQuotaSubscriptionExpiresAt?: number | null;
  providerId: string | null;
  providerName: string | null;
  tags: string[];
  profileAuthReady: boolean;
  profileConfigReady: boolean;
  profileIntegrityError: string | null;
  profileLastValidatedAt: number | null;
  profileLastValidationError: string | null;
  addedAt: number;
  updatedAt: number;
  usage: UsageSnapshot | null;
  usageError: string | null;
  authRefreshBlocked: boolean;
  authRefreshError: string | null;
  isCurrent: boolean;
};

export type AccountPoolConfig = {
  id: string;
  name: string;
  accountKeys: string[];
  collapsed: boolean;
};

export type NotificationTargetKind = "telegram" | "webhook";

export type NotificationProviderKind = "sub2api";

export type NotificationTemplatePreset =
  | "test"
  | "usageReport"
  | "quotaLow"
  | "quotaRecovered"
  | "accountError";
export type NotificationScheduleMode = "manual" | "daily" | "interval" | "date";

export type NotificationProviderConfig = {
  id: string;
  name: string;
  kind: NotificationProviderKind;
  enabled: boolean;
  costMultiplier: number;
  baseUrl: string;
  email: string;
  password: string | null;
  createdAt: number;
  updatedAt: number;
  lastTestAt: number | null;
  lastTestError: string | null;
};

export type NotificationTargetConfig = {
  id: string;
  name: string;
  kind: NotificationTargetKind;
  enabled: boolean;
  aggregateEnabled: boolean;
  providerIds: string[];
  templatePreset: NotificationTemplatePreset;
  messageTemplate: string;
  scheduleDate: string | null;
  scheduleTime: string | null;
  telegramBotToken: string | null;
  telegramChatId: string | null;
  webhookUrl: string | null;
  createdAt: number;
  updatedAt: number;
  lastTestAt: number | null;
  lastTestError: string | null;
};

export type NotificationBotConfig = {
  id: string;
  name: string;
  kind: NotificationTargetKind;
  enabled: boolean;
  telegramBotToken: string | null;
  telegramChatId: string | null;
  webhookUrl: string | null;
  createdAt: number;
  updatedAt: number;
  lastTestAt: number | null;
  lastTestError: string | null;
};

export type NotificationTemplateConfig = {
  id: string;
  name: string;
  preset: NotificationTemplatePreset;
  messageTemplate: string;
  createdAt: number;
  updatedAt: number;
};

export type NotificationPipelineConfig = {
  id: string;
  name: string;
  enabled: boolean;
  aggregateEnabled: boolean;
  providerIds: string[];
  botIds: string[];
  templateId: string | null;
  templateOverride: string | null;
  scheduleMode: NotificationScheduleMode;
  scheduleDate: string | null;
  scheduleTime: string | null;
  scheduleIntervalMinutes: number | null;
  createdAt: number;
  updatedAt: number;
  lastRunAt: number | null;
  lastTestAt: number | null;
  lastTestError: string | null;
};

export type SwitchAccountResult = {
  accountId: string;
  launchedAppPath: string | null;
  usedFallbackCli: boolean;
  opencodeSynced: boolean;
  opencodeSyncError: string | null;
  opencodeDesktopRestarted: boolean;
  opencodeDesktopRestartError: string | null;
  restartedEditorApps: EditorAppId[];
  editorRestartError: string | null;
};

export type PreparedOauthLogin = {
  authUrl: string;
  redirectUri: string;
};

export type OauthCallbackFinishedEvent = {
  result: ImportAccountsResult | null;
  error: string | null;
};

export type AuthJsonImportInput = {
  source: string;
  content: string;
  label: string | null;
};

export type CreateApiAccountInput = {
  label: string;
  baseUrl: string;
  apiKey: string;
  modelName: string;
  tags: string[];
  forceSave: boolean;
  balanceDisplayEnabled?: boolean;
  apiQuotaMode?: ApiQuotaMode;
  platformLoginEmail?: string;
  platformLoginPassword?: string;
};

export type UpdateApiAccountInput = {
  label: string;
  baseUrl: string;
  apiKey: string | null;
  modelName: string;
  balanceDisplayEnabled?: boolean;
  apiQuotaMode?: ApiQuotaMode;
  apiQuotaTodayUsedText?: string | null;
  apiQuotaRemainingText?: string | null;
  platformLoginEmail?: string;
  platformLoginPassword?: string;
};

export type UpdateApiAccountKeyInput = {
  id: string | null;
  label: string | null;
  apiKey: string | null;
  enabled: boolean;
  priority: number;
  weight: number;
};

export type ImportAccountFailure = {
  source: string;
  error: string;
};

export type ImportAccountsResult = {
  totalCount: number;
  importedCount: number;
  updatedCount: number;
  failures: ImportAccountFailure[];
};

export type Notice = {
  type: "ok" | "error" | "info";
  message: string;
};

export type PendingUpdateInfo = {
  currentVersion: string;
  version: string;
  body?: string;
  date?: string;
};

export type ThemeMode = "light" | "dark";

export type TrayUsageDisplayMode = "remaining" | "used" | "hidden";

export type EditorAppId =
  | "vscode"
  | "vscodeInsiders"
  | "cursor"
  | "antigravity"
  | "kiro"
  | "trae"
  | "qoder";

export type InstalledEditorApp = {
  id: EditorAppId;
  label: string;
};

export type AppSettings = {
  launchAtStartup: boolean;
  trayUsageDisplayMode: TrayUsageDisplayMode;
  launchCodexAfterSwitch: boolean;
  smartSwitchIncludeApi: boolean;
  apiEnhancedLaunchEnabled: boolean;
  usageAutoRefreshEnabled: boolean;
  usageAutoRefreshIntervalSecs: number;
  apiQuotaAutoRefreshEnabled: boolean;
  apiQuotaAutoRefreshIntervalSecs: number;
  quotaAlertEnabled: boolean;
  codexContextWindowK: number | null;
  codexContextWindowModel: string | null;
  codexContextWindowLimitK: number | null;
  codexContextWindowEffectiveLimitK: number | null;
  quotaAlertFiveHourThreshold: number;
  quotaAlertOneWeekThreshold: number;
  codexLaunchPath: string | null;
  activeHybridProfile?: {
    chatgptAccountId: string;
    relayAccountId: string;
  } | null;
  syncOpencodeOpenaiAuth: boolean;
  restartOpencodeDesktopOnSwitch: boolean;
  restartEditorsOnSwitch: boolean;
  restartEditorTargets: EditorAppId[];
  accountPools: AccountPoolConfig[];
  notificationProviders: NotificationProviderConfig[];
  notificationTargets: NotificationTargetConfig[];
  notificationBots: NotificationBotConfig[];
  notificationTemplates: NotificationTemplateConfig[];
  notificationPipelines: NotificationPipelineConfig[];
  notificationSchemaVersion: number;
  locale: AppLocale;
  skippedUpdateVersion: string | null;
};

export type UpdateSettingsOptions = {
  silent?: boolean;
  keepInteractive?: boolean;
};
