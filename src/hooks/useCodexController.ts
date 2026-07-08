import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import { PROJECT_LATEST_RELEASE_URL } from "../constants/externalLinks";
import { useI18n } from "../i18n/I18nProvider";
import { localizeBackendError } from "../i18n/backendErrors";
import { DEFAULT_LOCALE } from "../i18n/catalog";
import type { MessageCatalog } from "../i18n/catalog";
import type {
  AccountSummary,
  AccountsExportFormat,
  ApiQuotaMode,
  AppSettings,
  AuthJsonImportInput,
  CodexTokenUsageSnapshot,
  CreateApiAccountInput,
  ImportAccountsResult,
  InstalledEditorApp,
  NotificationProviderConfig,
  Notice,
  OauthCallbackFinishedEvent,
  PendingUpdateInfo,
  PreparedOauthLogin,
  RelayModelCatalogEntry,
  SwitchAccountResult,
  UpdateApiAccountInput,
  UpdateApiAccountKeyInput,
  UpdateSettingsOptions,
} from "../types/app";
import {
  pickBestSmartSwitchAccount,
  sortAccountsByRemaining,
  sortAccountsForDisplay,
} from "../utils/accountRanking";
import { normalizeApiQuotaSubscriptionName } from "../utils/apiQuotaSubscriptions";
import {
  normalizeModelCatalogContextWindows,
  normalizeModelContextWindow,
} from "../utils/modelContextWindow";
import { displayAccountLabel } from "../utils/privacy";

const DEFAULT_USAGE_REFRESH_INTERVAL_SECS = 30;
const DEFAULT_API_QUOTA_REFRESH_INTERVAL_SECS = 600;
const TOKEN_USAGE_REFRESH_MS = 6 * 60 * 60 * 1000;
const EDITOR_SCAN_MS = 60_000;
const UPDATE_CHECK_MS = 60 * 60 * 1000;

function isMissingUpdaterJsonError(error: string) {
  return error.includes("Could not fetch a valid release JSON from the remote");
}

const DEFAULT_SETTINGS: AppSettings = {
  launchAtStartup: false,
  trayUsageDisplayMode: "remaining",
  launchCodexAfterSwitch: true,
  smartSwitchIncludeApi: false,
  apiEnhancedLaunchEnabled: false,
  codexMultiModelModeEnabled: false,
  codexModelInstructionsFixEnabled: false,
  codexDisableGpuAcceleration: false,
  codexMultiModelStatus: null,
  codexMultiModelWorkspace: null,
  codexMultiModelRestorePoint: null,
  codexMultiModelControlledAppRoot: null,
  codexMultiModelControlledExePath: null,
  codexMultiModelControlledAppAsarPath: null,
  codexMultiModelSourceAppRoot: null,
  codexMultiModelPatchStatePath: null,
  modelRouterEnabled: false,
  modelRouterAccountId: null,
  modelRouterRouteSelections: [],
  usageAutoRefreshEnabled: true,
  usageAutoRefreshIntervalSecs: DEFAULT_USAGE_REFRESH_INTERVAL_SECS,
  apiQuotaAutoRefreshEnabled: true,
  apiQuotaAutoRefreshIntervalSecs: DEFAULT_API_QUOTA_REFRESH_INTERVAL_SECS,
  quotaAlertEnabled: true,
  quotaAlertFiveHourThreshold: 15,
  quotaAlertOneWeekThreshold: 20,
  codexLaunchPath: null,
  syncOpencodeOpenaiAuth: false,
  restartOpencodeDesktopOnSwitch: false,
  restartEditorsOnSwitch: false,
  restartEditorTargets: [],
  accountCardOrder: [],
  accountPools: [],
  notificationProviders: [],
  notificationTargets: [],
  notificationBots: [],
  notificationTemplates: [],
  notificationPipelines: [],
  notificationSchemaVersion: 1,
  locale: DEFAULT_LOCALE,
  skippedUpdateVersion: null,
};

const PREVIEW_SETTINGS_STORAGE_KEY_V2 = "codexdeck:preview-settings-v2";
const PREVIEW_ACCOUNTS_STORAGE_KEY_V2 = "codexdeck:preview-accounts-v2";
const ACCOUNT_SUMMARY_CACHE_STORAGE_KEY_V1 = "codexdeck:account-summary-cache-v1";
const PREVIEW_CHATGPT_ACCOUNT_KEY = "preview-chatgpt-pro";
const PREVIEW_RELAY_API_ONLY_KEY = "preview-api-quota-api-only";
const PREVIEW_RELAY_PLATFORM_BASIC_KEY = "preview-api-quota-platform-basic";
const PREVIEW_RELAY_PLATFORM_SUBSCRIPTION_KEY = "preview-api-quota-platform-subscription";
const PREVIEW_RELAY_ADMIN_KEY = "preview-api-quota-admin";
const PREVIEW_RELAY_ACCOUNT_KEYS = [
  PREVIEW_RELAY_API_ONLY_KEY,
  PREVIEW_RELAY_PLATFORM_BASIC_KEY,
  PREVIEW_RELAY_PLATFORM_SUBSCRIPTION_KEY,
  PREVIEW_RELAY_ADMIN_KEY,
];

function hasTauriRuntime() {
  return (
    typeof window !== "undefined" &&
    Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__)
  );
}

function isPreviewRuntime() {
  return !hasTauriRuntime() && import.meta.env.DEV;
}

function nowUnixSeconds() {
  return Math.floor(Date.now() / 1000);
}

function normalizeNotificationProviderBaseUrl(value: string) {
  const trimmed = value.trim().replace(/\/+$/, "");
  return trimmed;
}

function createLocalId(prefix: string) {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `${prefix}-${crypto.randomUUID()}`;
  }
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function buildNotificationProviderFromApiInput(
  input: CreateApiAccountInput,
  accountKey?: string | null,
  existing?: NotificationProviderConfig,
): NotificationProviderConfig | null {
  if (!input.balanceDisplayEnabled) {
    return null;
  }
  if ((input.apiQuotaMode ?? "apiOnly") === "apiOnly") {
    return null;
  }
  const email = input.platformLoginEmail?.trim() ?? "";
  const password = input.platformLoginPassword?.trim() ?? "";
  if (!email || !password) {
    return null;
  }

  const timestamp = nowUnixSeconds();
  const baseUrl = normalizeNotificationProviderBaseUrl(input.baseUrl);
  const name = input.label.trim() || "API 平台";
  const isUnchanged =
    existing &&
    existing.name === name &&
    existing.baseUrl === baseUrl &&
    existing.email === email &&
    existing.password === password;

  return {
    id: existing?.id ?? createLocalId("provider"),
    name,
    accountKey: accountKey ?? existing?.accountKey ?? null,
    kind: "sub2api",
    enabled: existing?.enabled ?? true,
    costMultiplier: existing?.costMultiplier ?? 1,
    baseUrl,
    email,
    password,
    createdAt: existing?.createdAt ?? timestamp,
    updatedAt: timestamp,
    lastTestAt: isUnchanged ? existing.lastTestAt : null,
    lastTestError: isUnchanged ? existing.lastTestError : null,
  };
}

function buildNotificationProviderFromApiUpdate(
  input: UpdateApiAccountInput,
  accountKey?: string | null,
  existing?: NotificationProviderConfig,
): NotificationProviderConfig | null {
  if (input.balanceDisplayEnabled === false) {
    return null;
  }
  if ((input.apiQuotaMode ?? "apiOnly") === "apiOnly") {
    return null;
  }
  const email = input.platformLoginEmail?.trim() ?? "";
  const password = input.platformLoginPassword?.trim() || existing?.password?.trim() || "";
  if (!email || !password) {
    return null;
  }

  const timestamp = nowUnixSeconds();
  const baseUrl = normalizeNotificationProviderBaseUrl(input.baseUrl);
  const name = input.label.trim() || "API 平台";
  const isUnchanged =
    existing &&
    existing.name === name &&
    existing.baseUrl === baseUrl &&
    existing.email === email &&
    existing.password === password;

  return {
    id: existing?.id ?? createLocalId("provider"),
    name,
    accountKey: accountKey ?? existing?.accountKey ?? null,
    kind: "sub2api",
    enabled: existing?.enabled ?? true,
    costMultiplier: existing?.costMultiplier ?? 1,
    baseUrl,
    email,
    password,
    createdAt: existing?.createdAt ?? timestamp,
    updatedAt: timestamp,
    lastTestAt: isUnchanged ? existing.lastTestAt : null,
    lastTestError: isUnchanged ? existing.lastTestError : null,
  };
}

function normalizeApiQuotaProviderBaseUrl(value: string | null | undefined) {
  return (value ?? "")
    .trim()
    .replace(/\/+$/, "")
    .toLowerCase()
    .replace(/\/api\/v1$/i, "")
    .replace(/\/v1$/i, "");
}

function supportsProviderApiKeyQuota(baseUrl: string | null | undefined) {
  const normalized = normalizeApiQuotaProviderBaseUrl(baseUrl);
  return (
    normalized.includes("api.deepseek.com") ||
    normalized.includes("api.moonshot.cn") ||
    normalized.includes("api.moonshot.ai") ||
    normalized.includes("api.moonshot.com") ||
    normalized.includes("api.kimi.com") ||
    normalized.includes("api.z.ai") ||
    normalized.includes("bigmodel.cn") ||
    normalized.includes("api.minimaxi.com") ||
    normalized.includes("minimaxi.com") ||
    normalized.includes("api.minimax.io") ||
    normalized.includes("minimax.io")
  );
}

function normalizeRelayModelCatalog(
  defaultModelName: string,
  entries: RelayModelCatalogEntry[] | undefined,
): RelayModelCatalogEntry[] {
  const defaultModel = defaultModelName.trim();
  const seen = new Set<string>();
  const output: RelayModelCatalogEntry[] = [];

  for (const entry of entries ?? []) {
    const requestModel = entry.requestModel?.trim() || "";
    const model = entry.model.trim() || requestModel;
    if (!model || seen.has(model)) {
      continue;
    }
    seen.add(model);
    output.push({
      model,
      displayName: entry.displayName?.trim() || null,
      requestModel: requestModel && requestModel !== model ? requestModel : null,
      contextWindow: normalizeModelContextWindow(entry.contextWindow, model, requestModel),
      enabled: entry.enabled,
    });
  }

  if (defaultModel) {
    const existing = output.find((entry) => entry.model === defaultModel);
    if (existing) {
      existing.enabled = true;
    } else {
      output.unshift({
        model: defaultModel,
        displayName: null,
        requestModel: null,
        contextWindow: null,
        enabled: true,
      });
    }
  }

  if (output.length > 0 && output.every((entry) => !entry.enabled)) {
    output[0] = { ...output[0], enabled: true };
  }

  return output;
}

function previewModelEntry(
  model: string,
  displayName: string,
  contextWindow: number | null = null,
): RelayModelCatalogEntry {
  return {
    model,
    displayName,
    requestModel: null,
    contextWindow,
    enabled: true,
  };
}

function previewProbeApiModels(baseUrl: string): RelayModelCatalogEntry[] {
  const normalized = baseUrl.trim().replace(/\/+$/, "").toLowerCase();
  if (normalized.includes("token-plan-cn.xiaomimimo.com")) {
    return [previewModelEntry("mimo-v2.5-pro", "MiMo V2.5 Pro", 256_000)];
  }
  if (normalized.includes("xiaomimimo.com")) {
    return [previewModelEntry("mimo-v2.5-pro", "MiMo V2.5 Pro", 256_000)];
  }
  if (normalized.includes("deepseek.com")) {
    return [
      previewModelEntry("deepseek-v4-flash", "DeepSeek V4 Flash", 256_000),
      previewModelEntry("deepseek-v4-pro", "DeepSeek V4 Pro", 256_000),
    ];
  }
  if (normalized.includes("api.z.ai") || normalized.includes("bigmodel.cn")) {
    return [
      previewModelEntry("glm-5.2", "GLM-5.2", 1_000_000),
      previewModelEntry("glm-5.1", "GLM-5.1", 200_000),
    ];
  }
  if (normalized.includes("moonshot.cn")) {
    return [previewModelEntry("kimi-k2.6", "Kimi K2.6", 256_000)];
  }
  if (normalized.includes("minimax.io") || normalized.includes("minimaxi.com")) {
    return [
      previewModelEntry("MiniMax-M2.7", "MiniMax M2.7", 256_000),
      previewModelEntry("MiniMax-M2.7-HighSpeed", "MiniMax M2.7 HighSpeed", 256_000),
      previewModelEntry("MiniMax-M3", "MiniMax M3", 512_000),
    ];
  }

  return [previewModelEntry("preview-model", "Preview Model")];
}

function accountHasApiQuotaProvider(
  account: AccountSummary,
  providers: NotificationProviderConfig[],
) {
  if (account.sourceKind !== "relay" || !account.balanceDisplayEnabled) {
    return false;
  }
  if (account.apiQuotaMode === "apiOnly") {
    return true;
  }
  if (supportsProviderApiKeyQuota(account.apiBaseUrl)) {
    return true;
  }
  const accountKey = account.accountKey.trim();
  const boundProvider = providers.find(
    (provider) =>
      provider.accountKey === accountKey &&
      Boolean(provider.email.trim()) &&
      Boolean(provider.password?.trim()),
  );
  if (boundProvider) {
    return true;
  }
  const accountBaseUrl = normalizeApiQuotaProviderBaseUrl(account.apiBaseUrl);
  if (!accountBaseUrl) {
    return false;
  }

  const unboundMatches = providers.filter(
    (provider) =>
      !provider.accountKey?.trim() &&
      normalizeApiQuotaProviderBaseUrl(provider.baseUrl) === accountBaseUrl &&
      Boolean(provider.email.trim()) &&
      Boolean(provider.password?.trim()),
  );
  return unboundMatches.length === 1;
}

function firstApiQuotaRefreshError(items: AccountSummary[], accountKeys: string[]) {
  const requested = new Set(accountKeys);
  return items.find(
    (account) =>
      requested.has(account.accountKey) &&
      account.balanceDisplayEnabled &&
      Boolean(account.profileLastValidationError),
  )?.profileLastValidationError ?? null;
}

async function upsertNotificationProviderForApiUpdate(
  input: UpdateApiAccountInput,
  accountKey: string,
  providers: NotificationProviderConfig[],
) {
  const normalizedBaseUrl = normalizeNotificationProviderBaseUrl(input.baseUrl);
  const normalizedEmail = input.platformLoginEmail?.trim().toLowerCase() ?? "";
  const boundProvider = providers.find((item) => item.accountKey === accountKey);
  const matchingUnboundProviders = providers.filter((item) => {
    const sameBaseUrl = item.baseUrl === normalizedBaseUrl;
    const sameEmail = item.email.trim().toLowerCase() === normalizedEmail;
    return !item.accountKey?.trim() && sameBaseUrl && (!normalizedEmail || sameEmail);
  });
  const fallbackUnboundProviders =
    matchingUnboundProviders.length > 0
      ? matchingUnboundProviders
      : providers.filter((item) => !item.accountKey?.trim() && item.baseUrl === normalizedBaseUrl);
  const existing =
    boundProvider ??
    (fallbackUnboundProviders.length === 1 ? fallbackUnboundProviders[0] : undefined);

  const provider = buildNotificationProviderFromApiUpdate(input, accountKey, existing);
  if (!provider) {
    if ((input.platformLoginEmail !== undefined || input.platformLoginPassword !== undefined) && existing) {
      return providers.filter((item) => item.id !== existing.id);
    }
    return providers;
  }

  const testedProvider = await probeNotificationProviderForImport(provider);
  const existingIndex = providers.findIndex((item) => item.id === testedProvider.id);
  if (existingIndex >= 0) {
    return providers.map((item, index) =>
      index === existingIndex
        ? {
            ...testedProvider,
            id: item.id,
            createdAt: item.createdAt,
          }
        : item,
    );
  }

  return [...providers, testedProvider];
}

async function probeNotificationProviderForImport(
  provider: NotificationProviderConfig,
): Promise<NotificationProviderConfig> {
  if (provider.lastTestAt && !provider.lastTestError) {
    return provider;
  }

  if (isPreviewRuntime()) {
    return {
      ...provider,
      lastTestAt: nowUnixSeconds(),
      lastTestError: null,
    };
  }

  try {
    await invoke<string>("test_notification_provider", { provider });
    return {
      ...provider,
      lastTestAt: nowUnixSeconds(),
      lastTestError: null,
    };
  } catch (error) {
    return {
      ...provider,
      lastTestAt: null,
      lastTestError: String(error),
    };
  }
}

function buildPreviewRelayAccount(
  overrides: {
    key: string;
    label: string;
    baseUrl: string;
    balanceText: string;
    balanceDisplayEnabled?: boolean;
    apiQuotaMode: ApiQuotaMode;
    providerId: string | null;
    providerName: string;
    tags: string[];
    addedAgo: number;
    isCurrent?: boolean;
    apiQuotaTodayUsedText?: string | null;
    apiQuotaRemainingText?: string | null;
    apiQuotaTotalRemainingText?: string | null;
    apiQuotaSubscriptionName?: string | null;
    apiQuotaTotalTokensText?: string | null;
    apiQuotaTodayTokensText?: string | null;
    apiQuotaDailyWindow?: {
      usedPercent: number;
      totalPercent?: number | null;
      resetInSeconds: number;
    } | null;
    apiQuotaTotalWindow?: {
      usedPercent: number;
      totalPercent?: number | null;
      resetInSeconds: number;
      windowSeconds?: number;
    } | null;
    apiQuotaSubscriptionExpiresInSeconds?: number | null;
  },
): AccountSummary {
  const now = nowUnixSeconds();
  return {
    id: `preview-account-${overrides.key}`,
    label: overrides.label,
    sourceKind: "relay",
    email: null,
    accountKey: overrides.key,
    accountId: overrides.key,
    planType: "api",
    apiBaseUrl: overrides.baseUrl,
    modelName: "gpt-5.5",
    modelCatalog: [
      {
        model: "gpt-5.5",
        displayName: "GPT-5.5",
        requestModel: null,
        contextWindow: null,
        enabled: true,
      },
    ],
    modelRoutingEnabled: false,
    balanceText: overrides.balanceText,
    balanceDisplayEnabled:
      overrides.balanceDisplayEnabled ??
      Boolean(overrides.balanceText || overrides.apiQuotaMode !== "apiOnly"),
    apiQuotaMode: overrides.apiQuotaMode,
    apiQuotaTodayUsedText: overrides.apiQuotaTodayUsedText ?? null,
    apiQuotaRemainingText: overrides.apiQuotaRemainingText ?? null,
    apiQuotaTotalRemainingText: overrides.apiQuotaTotalRemainingText ?? null,
    apiQuotaSubscriptionName: overrides.apiQuotaSubscriptionName ?? null,
    apiQuotaTotalTokensText: overrides.apiQuotaTotalTokensText ?? null,
    apiQuotaTodayTokensText: overrides.apiQuotaTodayTokensText ?? null,
    apiQuotaDailyWindow: overrides.apiQuotaDailyWindow
      ? {
          usedPercent: overrides.apiQuotaDailyWindow.usedPercent,
          totalPercent: overrides.apiQuotaDailyWindow.totalPercent ?? null,
          windowSeconds: 86_400,
          resetAt: now + overrides.apiQuotaDailyWindow.resetInSeconds,
        }
      : null,
    apiQuotaTotalWindow: overrides.apiQuotaTotalWindow
      ? {
          usedPercent: overrides.apiQuotaTotalWindow.usedPercent,
          totalPercent: overrides.apiQuotaTotalWindow.totalPercent ?? null,
          windowSeconds:
            overrides.apiQuotaTotalWindow.windowSeconds ??
            overrides.apiQuotaSubscriptionExpiresInSeconds ??
            overrides.apiQuotaTotalWindow.resetInSeconds,
          resetAt: now + overrides.apiQuotaTotalWindow.resetInSeconds,
        }
      : null,
    apiQuotaSubscriptionExpiresAt: overrides.apiQuotaSubscriptionExpiresInSeconds
      ? now + overrides.apiQuotaSubscriptionExpiresInSeconds
      : null,
    providerId: overrides.providerId,
    providerName: overrides.providerName,
    tags: overrides.tags,
    profileAuthReady: true,
    profileConfigReady: true,
    profileIntegrityError: null,
    profileLastValidatedAt: now,
    profileLastValidationError: null,
    addedAt: now - overrides.addedAgo,
    updatedAt: now,
    usage: {
      fetchedAt: now,
      planType: "api",
      fiveHour: {
        usedPercent: 38,
        windowSeconds: 18_000,
        resetAt: now + 7_200,
      },
      oneWeek: {
        usedPercent: 24,
        windowSeconds: 604_800,
        resetAt: now + 302_400,
      },
      credits: {
        hasCredits: true,
        unlimited: false,
        balance: overrides.balanceText.replace(/^\$/, ""),
      },
    },
    usageError: null,
    authRefreshBlocked: false,
    authRefreshError: null,
    authRefreshNextAt: null,
    isCurrent: Boolean(overrides.isCurrent),
  };
}

function buildPreviewChatGptAccount(): AccountSummary {
  const now = nowUnixSeconds();
  return {
    id: "preview-account-chatgpt-pro",
    label: "Codex Pro（演示）",
    sourceKind: "chatgpt",
    email: "daily-codex@example.com",
    accountKey: PREVIEW_CHATGPT_ACCOUNT_KEY,
    accountId: "acc_preview_chatgpt_pro",
    planType: "pro",
    apiBaseUrl: null,
    modelName: null,
    modelCatalog: [],
    modelRoutingEnabled: false,
    balanceText: null,
    balanceDisplayEnabled: false,
    apiQuotaMode: "apiOnly",
    providerId: null,
    providerName: "ChatGPT",
    tags: ["主力", "Pro"],
    profileAuthReady: true,
    profileConfigReady: true,
    profileIntegrityError: null,
    profileLastValidatedAt: now - 900,
    profileLastValidationError: null,
    addedAt: now - 172_800,
    updatedAt: now - 900,
    usage: {
      fetchedAt: now - 120,
      planType: "pro",
      fiveHour: {
        usedPercent: 44,
        windowSeconds: 18_000,
        resetAt: now + 5_400,
      },
      oneWeek: {
        usedPercent: 31,
        windowSeconds: 604_800,
        resetAt: now + 388_800,
      },
      credits: null,
    },
    usageError: null,
    authRefreshBlocked: false,
    authRefreshError: null,
    authRefreshNextAt: now + 3_000,
    isCurrent: false,
  };
}

function buildPreviewAccounts(): AccountSummary[] {
  return [
    buildPreviewChatGptAccount(),
    buildPreviewRelayAccount({
      key: PREVIEW_RELAY_API_ONLY_KEY,
      label: "API Key 余额（演示）",
      baseUrl: "https://relay-demo.example.com/v1",
      balanceText: "$37.02",
      apiQuotaMode: "apiOnly",
      providerId: null,
      providerName: "示例中转平台（演示）",
      tags: ["只有 API"],
      addedAgo: 86_400,
      isCurrent: true,
    }),
    buildPreviewRelayAccount({
      key: PREVIEW_RELAY_PLATFORM_BASIC_KEY,
      label: "平台账号无订阅（演示）",
      baseUrl: "https://relay-basic.example.com/v1",
      balanceText: "$18.64",
      apiQuotaMode: "platformBasic",
      providerId: "preview-notification-provider-basic",
      providerName: "基础额度平台（演示）",
      tags: ["账号密码", "无订阅"],
      addedAgo: 82_000,
      apiQuotaTodayUsedText: "$4.28",
      apiQuotaRemainingText: "$18.64",
    }),
    buildPreviewRelayAccount({
      key: PREVIEW_RELAY_PLATFORM_SUBSCRIPTION_KEY,
      label: "MiniMax Token Plan（演示）",
      baseUrl: "https://api.minimaxi.com/v1",
      balanceText: "$126.40",
      apiQuotaMode: "platformSubscription",
      providerId: "preview-notification-provider-subscription",
      providerName: "MiniMax（演示）",
      tags: ["账号密码", "订阅"],
      addedAgo: 78_000,
      apiQuotaTodayUsedText: "$12.80",
      apiQuotaTotalRemainingText: "$126.40",
      apiQuotaSubscriptionName: "Max",
      apiQuotaDailyWindow: {
        usedPercent: 8,
        totalPercent: 100,
        resetInSeconds: 8_400,
      },
      apiQuotaTotalWindow: {
        usedPercent: 8,
        totalPercent: 150,
        resetInSeconds: 1_728_000,
      },
      apiQuotaSubscriptionExpiresInSeconds: 1_728_000,
    }),
    buildPreviewRelayAccount({
      key: PREVIEW_RELAY_ADMIN_KEY,
      label: "管理账号统计（演示）",
      baseUrl: "https://relay-admin.example.com/v1",
      balanceText: "$248.90",
      apiQuotaMode: "admin",
      providerId: "preview-notification-provider-admin",
      providerName: "管理统计平台（演示）",
      tags: ["管理账号"],
      addedAgo: 74_000,
      apiQuotaTotalTokensText: "12.8M",
      apiQuotaTodayTokensText: "486K",
    }),
  ];
}

function buildPreviewSettings(): AppSettings {
  const now = nowUnixSeconds();
  const providerId = "preview-notification-provider-sub2api";
  const providerBasicId = "preview-notification-provider-basic";
  const providerSubscriptionId = "preview-notification-provider-subscription";
  const providerAdminId = "preview-notification-provider-admin";
  const botId = "preview-notification-bot-telegram";
  return {
    ...DEFAULT_SETTINGS,
    accountPools: [
      {
        id: "preview-pool-default",
        name: "演示账号",
        accountKeys: [PREVIEW_CHATGPT_ACCOUNT_KEY, ...PREVIEW_RELAY_ACCOUNT_KEYS],
        collapsed: false,
      },
    ],
    notificationProviders: [
      {
        id: providerId,
        name: "示例中转平台（演示）",
        kind: "sub2api",
        enabled: true,
        costMultiplier: 1,
        baseUrl: "https://relay-demo.example.com",
        email: "demo@example.com",
        password: "demo-password",
        createdAt: now - 3_600,
        updatedAt: now,
        lastTestAt: now - 300,
        lastTestError: null,
      },
      {
        id: providerBasicId,
        name: "基础额度平台（演示）",
        kind: "sub2api",
        enabled: true,
        costMultiplier: 1,
        baseUrl: "https://relay-basic.example.com",
        email: "basic@example.com",
        password: "demo-password",
        createdAt: now - 3_400,
        updatedAt: now,
        lastTestAt: now - 260,
        lastTestError: null,
      },
      {
        id: providerSubscriptionId,
        name: "订阅额度平台（演示）",
        kind: "sub2api",
        enabled: true,
        costMultiplier: 1,
        baseUrl: "https://relay-pro.example.com",
        email: "pro@example.com",
        password: "demo-password",
        createdAt: now - 3_200,
        updatedAt: now,
        lastTestAt: now - 220,
        lastTestError: null,
      },
      {
        id: providerAdminId,
        name: "管理统计平台（演示）",
        kind: "sub2api",
        enabled: true,
        costMultiplier: 1,
        baseUrl: "https://relay-admin.example.com",
        email: "admin@example.com",
        password: "demo-password",
        createdAt: now - 3_000,
        updatedAt: now,
        lastTestAt: now - 180,
        lastTestError: null,
      },
    ],
    notificationBots: [
      {
        id: botId,
        name: "Telegram Bot（演示）",
        kind: "telegram",
        enabled: true,
        telegramBotToken: "123456:demo-preview-token",
        telegramChatId: "123456789",
        webhookUrl: null,
        createdAt: now - 3_600,
        updatedAt: now,
        lastTestAt: now - 240,
        lastTestError: null,
      },
      {
        id: "preview-notification-bot-ops",
        name: "Ops Alerts Bot（演示）",
        kind: "telegram",
        enabled: true,
        telegramBotToken: "123456:demo-preview-ops",
        telegramChatId: "@ops_alerts",
        webhookUrl: null,
        createdAt: now - 2_900,
        updatedAt: now,
        lastTestAt: now - 180,
        lastTestError: null,
      },
      {
        id: "preview-notification-bot-dev",
        name: "Dev Team Bot（演示）",
        kind: "telegram",
        enabled: true,
        telegramBotToken: "123456:demo-preview-dev",
        telegramChatId: "@dev_team",
        webhookUrl: null,
        createdAt: now - 2_700,
        updatedAt: now,
        lastTestAt: now - 150,
        lastTestError: null,
      },
    ],
    notificationPipelines: [
      {
        id: "preview-notification-pipeline-daily",
        name: "每日构建通知",
        enabled: true,
        aggregateEnabled: true,
        providerIds: [providerId],
        botIds: [botId],
        templateId: "builtin-usage-report",
        templateOverride: null,
        scheduleMode: "daily",
        scheduleDate: null,
        scheduleTime: "09:00",
        scheduleIntervalMinutes: null,
        createdAt: now - 3_600,
        updatedAt: now,
        lastRunAt: null,
        lastTestAt: now - 120,
        lastTestError: null,
      },
      {
        id: "preview-notification-pipeline-quota",
        name: "额度预警：OpenAI",
        enabled: true,
        aggregateEnabled: true,
        providerIds: [providerId],
        botIds: ["preview-notification-bot-ops"],
        templateId: "builtin-quota-low",
        templateOverride: null,
        scheduleMode: "daily",
        scheduleDate: null,
        scheduleTime: "10:30",
        scheduleIntervalMinutes: null,
        createdAt: now - 3_300,
        updatedAt: now - 600,
        lastRunAt: now - 480,
        lastTestAt: now - 240,
        lastTestError: null,
      },
      {
        id: "preview-notification-pipeline-failed",
        name: "流水线失败告警",
        enabled: true,
        aggregateEnabled: true,
        providerIds: [providerId],
        botIds: ["preview-notification-bot-dev"],
        templateId: "builtin-account-error",
        templateOverride: null,
        scheduleMode: "daily",
        scheduleDate: null,
        scheduleTime: "11:05",
        scheduleIntervalMinutes: null,
        createdAt: now - 3_000,
        updatedAt: now - 240,
        lastRunAt: now - 420,
        lastTestAt: now - 360,
        lastTestError: "Telegram Bot API 返回 500",
      },
      {
        id: "preview-notification-pipeline-chatid",
        name: "Chat ID 探测结果",
        enabled: false,
        aggregateEnabled: false,
        providerIds: [providerId],
        botIds: [botId],
        templateId: "builtin-test",
        templateOverride: null,
        scheduleMode: "manual",
        scheduleDate: null,
        scheduleTime: null,
        scheduleIntervalMinutes: null,
        createdAt: now - 2_500,
        updatedAt: now - 120,
        lastRunAt: now - 300,
        lastTestAt: now - 300,
        lastTestError: null,
      },
      {
        id: "preview-notification-pipeline-manual",
        name: "手动测试：模板消息",
        enabled: true,
        aggregateEnabled: false,
        providerIds: [providerId],
        botIds: [botId, "preview-notification-bot-dev"],
        templateId: "builtin-test",
        templateOverride: null,
        scheduleMode: "manual",
        scheduleDate: null,
        scheduleTime: null,
        scheduleIntervalMinutes: null,
        createdAt: now - 2_200,
        updatedAt: now - 90,
        lastRunAt: now - 90,
        lastTestAt: now - 90,
        lastTestError: null,
      },
      {
        id: "preview-notification-pipeline-anthropic",
        name: "额度同步：Anthropic",
        enabled: true,
        aggregateEnabled: true,
        providerIds: [providerId],
        botIds: ["preview-notification-bot-ops"],
        templateId: "builtin-usage-report",
        templateOverride: null,
        scheduleMode: "interval",
        scheduleDate: null,
        scheduleTime: null,
        scheduleIntervalMinutes: 30,
        createdAt: now - 2_100,
        updatedAt: now - 60,
        lastRunAt: null,
        lastTestAt: now - 60,
        lastTestError: null,
      },
    ],
    notificationSchemaVersion: 1,
  };
}

function readPreviewJson<T>(key: string, fallback: T): T {
  if (typeof window === "undefined") {
    return fallback;
  }

  try {
    const raw = window.localStorage.getItem(key);
    return raw ? { ...fallback, ...JSON.parse(raw) } : fallback;
  } catch {
    return fallback;
  }
}

function readPreviewSettings() {
  const seed = buildPreviewSettings();
  const saved = readPreviewJson<AppSettings>(PREVIEW_SETTINGS_STORAGE_KEY_V2, seed);
  const savedAccountPools = (saved.accountPools ?? []).map((pool) =>
    pool.id === "preview-pool-default" && !pool.accountKeys.includes(PREVIEW_CHATGPT_ACCOUNT_KEY)
      ? { ...pool, accountKeys: [PREVIEW_CHATGPT_ACCOUNT_KEY, ...pool.accountKeys] }
      : pool,
  ).map((pool) =>
    pool.id === "preview-pool-default"
      ? {
          ...pool,
          accountKeys: [
            ...pool.accountKeys.filter((accountKey) => accountKey !== "preview-sub2api-nas"),
            ...PREVIEW_RELAY_ACCOUNT_KEYS.filter((accountKey) => !pool.accountKeys.includes(accountKey)),
          ],
        }
      : pool,
  );
  const savedNotificationProviders = (saved.notificationProviders ?? []).map((provider) => {
    const seedProvider = seed.notificationProviders.find((item) => item.id === provider.id);
    return seedProvider ? { ...seedProvider, enabled: provider.enabled } : provider;
  });
  const savedNotificationBots = saved.notificationBots ?? [];
  const savedNotificationPipelines = saved.notificationPipelines ?? [];
  return {
    ...saved,
    accountPools: [
      ...seed.accountPools.filter(
        (seedPool) => !savedAccountPools.some((pool) => pool.id === seedPool.id),
      ),
      ...savedAccountPools,
    ],
    notificationProviders: [
      ...seed.notificationProviders.filter(
        (seedProvider) =>
          !savedNotificationProviders.some((provider) => provider.id === seedProvider.id),
      ),
      ...savedNotificationProviders,
    ],
    notificationBots: [
      ...seed.notificationBots.filter(
        (seedBot) => !savedNotificationBots.some((bot) => bot.id === seedBot.id),
      ),
      ...savedNotificationBots,
    ],
    notificationPipelines: [
      ...seed.notificationPipelines.filter(
        (seedPipeline) =>
          !savedNotificationPipelines.some((pipeline) => pipeline.id === seedPipeline.id),
      ),
      ...savedNotificationPipelines,
    ],
    notificationSchemaVersion: 1,
  };
}

function readPreviewAccounts() {
  const seed = buildPreviewAccounts();
  if (typeof window === "undefined") {
    return seed;
  }

  try {
    const raw = window.localStorage.getItem(PREVIEW_ACCOUNTS_STORAGE_KEY_V2);
    if (!raw) {
      return seed;
    }
    const saved = JSON.parse(raw) as AccountSummary[];
    if (!Array.isArray(saved)) {
      return seed;
    }
    const normalizedSaved = saved
      .filter((account) => account.accountKey !== "preview-sub2api-nas")
      .map((account) => {
        const apiQuotaMode: ApiQuotaMode = account.apiQuotaMode ?? "apiOnly";
        const seedAccount = seed.find((item) => item.accountKey === account.accountKey);
        const balanceDisplayEnabled =
          account.balanceDisplayEnabled ??
          seedAccount?.balanceDisplayEnabled ??
          Boolean(account.balanceText || apiQuotaMode !== "apiOnly");
        if (!seedAccount) {
          return { ...account, apiQuotaMode, balanceDisplayEnabled };
        }

        return {
          ...seedAccount,
          ...account,
          apiQuotaMode,
          balanceDisplayEnabled,
          label: seedAccount.label,
          email: seedAccount.email,
          planType: seedAccount.planType,
          providerName: seedAccount.providerName,
          tags: seedAccount.tags,
          usage: seedAccount.usage,
          updatedAt: seedAccount.updatedAt,
        };
      });
    return [
      ...seed.filter(
        (seedAccount) =>
          !normalizedSaved.some((account) => account.accountKey === seedAccount.accountKey),
      ),
      ...normalizedSaved,
    ];
  } catch {
    return seed;
  }
}

function writePreviewSettings(settings: AppSettings) {
  if (typeof window === "undefined") {
    return;
  }
  window.localStorage.setItem(PREVIEW_SETTINGS_STORAGE_KEY_V2, JSON.stringify(settings));
}

function writePreviewAccounts(accounts: AccountSummary[]) {
  if (typeof window === "undefined") {
    return;
  }
  window.localStorage.setItem(PREVIEW_ACCOUNTS_STORAGE_KEY_V2, JSON.stringify(accounts));
}

type AccountSummaryCachePayload = {
  version: 1;
  updatedAt: number;
  accounts: AccountSummary[];
};

function readCachedAccountSummaries(): AccountSummary[] {
  if (typeof window === "undefined" || isPreviewRuntime()) {
    return [];
  }

  try {
    const raw = window.localStorage.getItem(ACCOUNT_SUMMARY_CACHE_STORAGE_KEY_V1);
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw) as Partial<AccountSummaryCachePayload>;
    if (parsed.version !== 1 || !Array.isArray(parsed.accounts)) {
      return [];
    }
    return parsed.accounts.filter(
      (account) =>
        account &&
        typeof account.id === "string" &&
        typeof account.accountKey === "string" &&
        typeof account.label === "string",
    );
  } catch {
    return [];
  }
}

function writeCachedAccountSummaries(accounts: AccountSummary[]) {
  if (typeof window === "undefined" || isPreviewRuntime()) {
    return;
  }

  try {
    const payload: AccountSummaryCachePayload = {
      version: 1,
      updatedAt: nowUnixSeconds(),
      accounts,
    };
    window.localStorage.setItem(ACCOUNT_SUMMARY_CACHE_STORAGE_KEY_V1, JSON.stringify(payload));
  } catch {
    // 缓存只用于首屏加速；WebView 存储不可用时直接退回后端读取。
  }
}

function buildPreviewTokenUsage(): CodexTokenUsageSnapshot {
  const now = nowUnixSeconds();
  return {
    updatedAt: now,
    sourcePathCount: 1,
    failedPathCount: 0,
    eventCount: 28,
    last7d: {
      inputTokens: 1_820_000,
      cachedInputTokens: 1_310_000,
      outputTokens: 148_000,
      reasoningOutputTokens: 39_000,
      totalTokens: 1_968_000,
    },
    last30d: {
      inputTokens: 5_960_000,
      cachedInputTokens: 4_120_000,
      outputTokens: 482_000,
      reasoningOutputTokens: 126_000,
      totalTokens: 6_442_000,
    },
  };
}

const HIDE_ACCOUNT_DETAILS_STORAGE_KEY = "codex-switch:hide-account-details";
const LEGACY_HIDE_ACCOUNT_DETAILS_STORAGE_KEY = "codex-tools:hide-account-details";
const PROFILE_INTEGRITY_NOTICE_ACK_STORAGE_KEY =
  "codexdeck:profile-integrity-notice-ack-v1";

function readHideAccountDetailsPreference() {
  if (typeof window === "undefined") {
    return false;
  }

  try {
    return (
      window.localStorage.getItem(HIDE_ACCOUNT_DETAILS_STORAGE_KEY) ??
      window.localStorage.getItem(LEGACY_HIDE_ACCOUNT_DETAILS_STORAGE_KEY)
    ) === "true";
  } catch {
    return false;
  }
}

function writeHideAccountDetailsPreference(value: boolean) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(HIDE_ACCOUNT_DETAILS_STORAGE_KEY, value ? "true" : "false");
  } catch {
    // localStorage can be unavailable in restricted webviews; hiding still works in memory.
  }
}

function hashProfileIntegrityNoticeInput(value: string) {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

function buildProfileIntegrityNoticeAck(items: AccountSummary[]) {
  const incompleteAccounts = items
    .filter((account) => account.profileIntegrityError)
    .map((account) =>
      [
        account.sourceKind,
        account.accountKey,
        account.profileAuthReady ? "auth-ready" : "auth-missing",
        account.profileConfigReady ? "config-ready" : "config-missing",
      ].join(":"),
    )
    .sort();

  if (incompleteAccounts.length === 0) {
    return null;
  }

  return {
    count: incompleteAccounts.length,
    key: `v1:${incompleteAccounts.length}:${hashProfileIntegrityNoticeInput(
      incompleteAccounts.join("|"),
    )}`,
  };
}

function readProfileIntegrityNoticeAck() {
  if (typeof window === "undefined") {
    return null;
  }

  try {
    return window.localStorage.getItem(PROFILE_INTEGRITY_NOTICE_ACK_STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeProfileIntegrityNoticeAck(value: string) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(PROFILE_INTEGRITY_NOTICE_ACK_STORAGE_KEY, value);
  } catch {
    // localStorage can be unavailable in restricted webviews; the in-memory guard still applies.
  }
}

function buildImportNotice(
  result: ImportAccountsResult,
  prefix: string,
  notices: MessageCatalog["notices"],
  locale: string,
): Notice {
  const successCount = result.importedCount + result.updatedCount;
  const failureCount = result.failures.length;
  const firstFailure = result.failures[0];

  if (successCount === 0) {
    if (firstFailure) {
      return {
        type: "error",
        message: notices.importFailedWithSource(prefix, firstFailure.source, firstFailure.error),
      };
    }
    return {
      type: "error",
      message: notices.importFailedNoValidJson(prefix),
    };
  }

  const segments: string[] = [];
  if (result.importedCount > 0) {
    segments.push(notices.importSummaryAdded(result.importedCount));
  }
  if (result.updatedCount > 0) {
    segments.push(notices.importSummaryUpdated(result.updatedCount));
  }
  if (failureCount > 0) {
    segments.push(notices.importSummaryFailed(failureCount));
  }

  const suffix =
    failureCount > 0 && firstFailure
      ? notices.importSummaryFirstFailure(firstFailure.source, firstFailure.error)
      : "";
  const listFormatter = new Intl.ListFormat(locale, {
    style: "short",
    type: "conjunction",
  });

  return {
    type: failureCount > 0 ? "info" : "ok",
    message: notices.importSummaryDone(prefix, listFormatter.format(segments), suffix),
  };
}

function windowRemainingPercent(usedPercent: number | null | undefined): number | null {
  if (usedPercent === null || usedPercent === undefined || Number.isNaN(usedPercent)) {
    return null;
  }
  return Math.max(0, Math.min(100, 100 - usedPercent));
}

function parseTagInput(input: string): string[] {
  return input
    .split(/[\n,，]/)
    .map((item) => item.trim())
    .filter(Boolean)
    .reduce<string[]>((acc, item) => {
      if (acc.some((existing) => existing === item)) {
        return acc;
      }
      acc.push(item);
      return acc;
    }, []);
}

export function useCodexController() {
  const { copy, locale } = useI18n();
  const initialCachedAccountsRef = useRef<AccountSummary[] | null>(null);
  if (initialCachedAccountsRef.current === null) {
    initialCachedAccountsRef.current = readCachedAccountSummaries();
  }
  const initialCachedAccounts = initialCachedAccountsRef.current;
  const [accounts, setAccounts] = useState<AccountSummary[]>(initialCachedAccounts);
  const [tokenUsage, setTokenUsage] = useState<CodexTokenUsageSnapshot | null>(null);
  const [tokenUsageError, setTokenUsageError] = useState<string | null>(null);
  const [loading, setLoading] = useState(initialCachedAccounts.length === 0);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshingTokenUsage, setRefreshingTokenUsage] = useState(false);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [reauthorizeAccount, setReauthorizeAccount] = useState<AccountSummary | null>(null);
  const [importingAccounts, setImportingAccounts] = useState(false);
  const [oauthWaitingForCallback, setOauthWaitingForCallback] = useState(false);
  const [exportingAccounts, setExportingAccounts] = useState(false);
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [renamingAccountId, setRenamingAccountId] = useState<string | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<string | null>(null);
  const [pendingUpdate, setPendingUpdate] = useState<PendingUpdateInfo | null>(null);
  const [updateDialogOpen, setUpdateDialogOpen] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [savingSettings, setSavingSettings] = useState(false);
  const [installedEditorApps, setInstalledEditorApps] = useState<InstalledEditorApp[]>([]);
  const [hasOpencodeDesktopApp, setHasOpencodeDesktopApp] = useState(false);
  const [hideAccountDetails, setHideAccountDetails] = useState(readHideAccountDetailsPreference);
  const hideAccountDetailsRef = useRef(hideAccountDetails);
  const installingUpdateRef = useRef(false);
  const deleteConfirmTimerRef = useRef<number | null>(null);
  const settingsUpdateQueueRef = useRef<Promise<void>>(Promise.resolve());
  const settingsRef = useRef<AppSettings>(DEFAULT_SETTINGS);
  const accountsRef = useRef<AccountSummary[]>(initialCachedAccounts);
  const profileIntegrityPromptedRef = useRef<string | null>(null);
  const lastQuotaAlertKeyRef = useRef<string | null>(null);
  const localeRelocalizeReadyRef = useRef(false);

  hideAccountDetailsRef.current = hideAccountDetails;

  const noticeAccountLabel = useCallback(
    (account: AccountSummary) => displayAccountLabel(account, hideAccountDetailsRef.current),
    [],
  );

  const displayAccounts = useMemo(() => sortAccountsForDisplay(accounts), [accounts]);
  const rankedAccounts = useMemo(() => sortAccountsByRemaining(accounts), [accounts]);

  const localizeError = useCallback(
    (error: string) => localizeBackendError(error, locale),
    [locale],
  );

  const localizeAccounts = useCallback(
    (items: AccountSummary[]) =>
      items.map((account) => ({
        ...account,
        usageError: account.usageError ? localizeError(account.usageError) : null,
        authRefreshError: account.authRefreshError ? localizeError(account.authRefreshError) : null,
        profileIntegrityError: account.profileIntegrityError
          ? localizeError(account.profileIntegrityError)
          : null,
        profileLastValidationError: account.profileLastValidationError
          ? localizeError(account.profileLastValidationError)
          : null,
      })),
    [localizeError],
  );

  const applyAccounts = useCallback(
    (items: AccountSummary[]) => {
      const localized = localizeAccounts(items);
      writeCachedAccountSummaries(localized);
      accountsRef.current = localized;
      setAccounts(localized);

      const currentSettings = settingsRef.current;
      const currentAccount = localized.find((account) => account.isCurrent) ?? null;
      if (!currentSettings.quotaAlertEnabled || !currentAccount) {
        lastQuotaAlertKeyRef.current = null;
        return false;
      }

      const fiveHourRemaining = windowRemainingPercent(currentAccount.usage?.fiveHour?.usedPercent);
      const oneWeekRemaining = windowRemainingPercent(currentAccount.usage?.oneWeek?.usedPercent);
      const quotaReasons: string[] = [];
      if (
        fiveHourRemaining !== null &&
        fiveHourRemaining <= currentSettings.quotaAlertFiveHourThreshold
      ) {
        quotaReasons.push(`5h ${fiveHourRemaining}%`);
      }
      if (
        oneWeekRemaining !== null &&
        oneWeekRemaining <= currentSettings.quotaAlertOneWeekThreshold
      ) {
        quotaReasons.push(`1week ${oneWeekRemaining}%`);
      }

      if (quotaReasons.length === 0) {
        lastQuotaAlertKeyRef.current = null;
        return false;
      }

      const suggestedAccount = pickBestSmartSwitchAccount(
        localized.filter((account) => account.accountKey !== currentAccount.accountKey),
        currentSettings.smartSwitchIncludeApi,
      );
      const alertKey = [
        currentAccount.accountKey,
        quotaReasons.join("|"),
        suggestedAccount?.accountKey ?? "none",
      ].join("::");
      if (lastQuotaAlertKeyRef.current === alertKey) {
        return false;
      }

      lastQuotaAlertKeyRef.current = alertKey;
      setNotice({
        type: "info",
        message: copy.notices.quotaAlertCurrentLow(
          noticeAccountLabel(currentAccount),
          quotaReasons.join(" / "),
          suggestedAccount ? noticeAccountLabel(suggestedAccount) : "",
        ),
      });
      return true;
    },
    [copy.notices, localizeAccounts, noticeAccountLabel],
  );

  const updateAccountsState = useCallback((updater: (items: AccountSummary[]) => AccountSummary[]) => {
    setAccounts((prev) => {
      const next = updater(prev);
      accountsRef.current = next;
      writeCachedAccountSummaries(next);
      return next;
    });
  }, []);

  const applyRefreshAccounts = useCallback(
    (items: AccountSummary[], quiet: boolean, source: string) => {
      if (items.length === 0 && accountsRef.current.length > 0) {
        if (!quiet) {
          setNotice({
            type: "error",
            message: `${source} 返回了空账号列表，已保留当前账号数据。`,
          });
        }
        return false;
      }
      return applyAccounts(items);
    },
    [applyAccounts],
  );

  const applyLoadedAccounts = useCallback(
    (items: AccountSummary[], source: string) => {
      if (items.length === 0 && accountsRef.current.length > 0) {
        setNotice({
          type: "error",
          message: `${source} 返回了空账号列表，已保留当前账号数据。`,
        });
        return accountsRef.current;
      }
      applyAccounts(items);
      return items;
    },
    [applyAccounts],
  );

  useEffect(() => {
    writeHideAccountDetailsPreference(hideAccountDetails);
  }, [hideAccountDetails]);

  const localizeImportResult = useCallback(
    (result: ImportAccountsResult): ImportAccountsResult => ({
      ...result,
      failures: result.failures.map((failure) => ({
        ...failure,
        error: localizeError(failure.error),
      })),
    }),
    [localizeError],
  );

  const loadAccounts = useCallback(async () => {
    if (isPreviewRuntime()) {
      const data = readPreviewAccounts();
      applyLoadedAccounts(data, "预览账号加载");
      return data;
    }

    const data = await invoke<AccountSummary[]>("list_accounts");
    return applyLoadedAccounts(data, "账号加载");
  }, [applyLoadedAccounts]);

  const runStartupMaintenance = useCallback(async () => {
    if (isPreviewRuntime()) {
      return accountsRef.current;
    }
    const data = await invoke<AccountSummary[]>("run_startup_maintenance");
    return applyLoadedAccounts(data, "启动维护");
  }, [applyLoadedAccounts]);

  const maybeShowProfileIntegrityNotice = useCallback(
    (items: AccountSummary[]) => {
      const ack = buildProfileIntegrityNoticeAck(items);
      if (!ack) {
        profileIntegrityPromptedRef.current = null;
        return;
      }
      if (
        profileIntegrityPromptedRef.current === ack.key ||
        readProfileIntegrityNoticeAck() === ack.key
      ) {
        profileIntegrityPromptedRef.current = ack.key;
        return;
      }
      profileIntegrityPromptedRef.current = ack.key;
      writeProfileIntegrityNoticeAck(ack.key);
      setNotice({
        type: "info",
        message: copy.notices.profileIntegrityWarning(ack.count),
      });
    },
    [copy.notices],
  );

  const loadSettings = useCallback(async () => {
    if (isPreviewRuntime()) {
      const data = readPreviewSettings();
      settingsRef.current = data;
      setSettings(data);
      return;
    }

    const data = await invoke<AppSettings>("get_app_settings");
    settingsRef.current = data;
    setSettings(data);
  }, []);

  const loadInstalledEditorApps = useCallback(async () => {
    if (isPreviewRuntime()) {
      setInstalledEditorApps([]);
      return;
    }

    try {
      const data = await invoke<InstalledEditorApp[]>("list_installed_editor_apps");
      setInstalledEditorApps(data);
    } catch {
      setInstalledEditorApps([]);
    }
  }, []);

  const loadOpencodeDesktopAppInstalled = useCallback(async () => {
    if (isPreviewRuntime()) {
      setHasOpencodeDesktopApp(false);
      return;
    }

    try {
      const installed = await invoke<boolean>("is_opencode_desktop_app_installed");
      setHasOpencodeDesktopApp(installed);
    } catch {
      setHasOpencodeDesktopApp(false);
    }
  }, []);

  const updateSettings = useCallback(
    async (patch: Partial<AppSettings>, options?: UpdateSettingsOptions) => {
      const shouldLockUi = !options?.keepInteractive;
      const task = async () => {
        if (shouldLockUi) {
          setSavingSettings(true);
        }

        try {
          if (isPreviewRuntime()) {
            const data = {
              ...settingsRef.current,
              ...patch,
            };
            settingsRef.current = data;
            setSettings(data);
            writePreviewSettings(data);
            if (!options?.silent) {
              setNotice({ type: "ok", message: copy.notices.settingsUpdated });
            }
            return;
          }

          const data = await invoke<AppSettings>("update_app_settings", { patch });
          settingsRef.current = data;
          setSettings(data);
          if (!options?.silent) {
            setNotice({ type: "ok", message: copy.notices.settingsUpdated });
          }
        } catch (error) {
          setNotice({
            type: "error",
            message: copy.notices.updateSettingsFailed(localizeError(String(error))),
          });
        } finally {
          if (shouldLockUi) {
            setSavingSettings(false);
          }
        }
      };

      const run = settingsUpdateQueueRef.current.then(task, task);
      settingsUpdateQueueRef.current = run.then(
        () => undefined,
        () => undefined,
      );
      return run;
    },
    [copy.notices, localizeError],
  );

  const refreshUsage = useCallback(async (quiet = false) => {
    try {
      if (!quiet) {
        setRefreshing(true);
      }
      if (isPreviewRuntime()) {
        const data = readPreviewAccounts();
        const accountNoticeShown = applyAccounts(data);
        if (!quiet && !accountNoticeShown) {
          setNotice({ type: "ok", message: "预览账号额度已刷新。" });
        }
        return;
      }

      const data = await invoke<AccountSummary[]>("refresh_all_usage", {
        forceAuthRefresh: !quiet,
      });
      const accountNoticeShown = applyRefreshAccounts(data, quiet, copy.notices.usageRefreshed);
      if (!quiet && !accountNoticeShown) {
        setNotice({ type: "ok", message: copy.notices.usageRefreshed });
      }
    } catch (error) {
      if (!quiet) {
        setNotice({
          type: "error",
          message: copy.notices.refreshFailed(localizeError(String(error))),
        });
      }
    } finally {
      if (!quiet) {
        setRefreshing(false);
      }
    }
  }, [applyAccounts, applyRefreshAccounts, copy.notices, localizeError]);

  const refreshUsageForAccountKeys = useCallback(
    async (accountKeys: string[], options?: { quiet?: boolean; notice?: string }) => {
      const normalizedKeys = Array.from(
        new Set(accountKeys.map((key) => key.trim()).filter(Boolean)),
      );
      if (normalizedKeys.length === 0) {
        setNotice({ type: "error", message: copy.notices.groupUsageRefreshNoNativeAccounts });
        return;
      }

      const quiet = options?.quiet ?? false;
      try {
        if (!quiet) {
          setRefreshing(true);
        }
        if (isPreviewRuntime()) {
          const data = readPreviewAccounts();
          const accountNoticeShown = applyAccounts(data);
          if (!quiet && !accountNoticeShown) {
            setNotice({
              type: "ok",
              message: options?.notice ?? copy.notices.groupUsageRefreshed(normalizedKeys.length),
            });
          }
          return;
        }

        const data = await invoke<AccountSummary[]>("refresh_usage_for_account_keys", {
          accountKeys: normalizedKeys,
          forceAuthRefresh: !quiet,
        });
        const accountNoticeShown = applyRefreshAccounts(
          data,
          quiet,
          options?.notice ?? copy.notices.groupUsageRefreshed(normalizedKeys.length),
        );
        if (!quiet && !accountNoticeShown) {
          setNotice({
            type: "ok",
            message: options?.notice ?? copy.notices.groupUsageRefreshed(normalizedKeys.length),
          });
        }
      } catch (error) {
        if (!quiet) {
          setNotice({
            type: "error",
            message: copy.notices.refreshFailed(localizeError(String(error))),
          });
        }
      } finally {
        if (!quiet) {
          setRefreshing(false);
        }
      }
    },
    [applyAccounts, applyRefreshAccounts, copy.notices, localizeError],
  );

  const refreshApiQuotaForAccountKeys = useCallback(
    async (accountKeys: string[], options?: { quiet?: boolean; notice?: string }) => {
      const normalizedKeys = Array.from(
        new Set(accountKeys.map((key) => key.trim()).filter(Boolean)),
      );
      if (normalizedKeys.length === 0) {
        if (!options?.quiet) {
          setNotice({ type: "info", message: copy.notices.apiQuotaRefreshNoBoundAccounts });
        }
        return;
      }

      const quiet = options?.quiet ?? false;
      try {
        if (!quiet) {
          setRefreshing(true);
        }
        if (isPreviewRuntime()) {
          const now = nowUnixSeconds();
          const requested = new Set(normalizedKeys);
          const data = readPreviewAccounts().map((account) =>
            requested.has(account.accountKey)
              ? {
                  ...account,
                  updatedAt: now,
                  profileLastValidationError: null,
                }
              : account,
          );
          writePreviewAccounts(data);
          const accountNoticeShown = applyAccounts(data);
          if (!quiet && !accountNoticeShown) {
            setNotice({
              type: "ok",
              message: options?.notice ?? copy.notices.apiQuotaRefreshed(normalizedKeys.length),
            });
          }
          return;
        }

        const data = await invoke<AccountSummary[]>("refresh_api_quota_for_account_keys", {
          accountKeys: normalizedKeys,
        });
        const accountNoticeShown = applyRefreshAccounts(
          data,
          quiet,
          options?.notice ?? copy.notices.apiQuotaRefreshed(normalizedKeys.length),
        );
        if (!quiet && !accountNoticeShown) {
          const refreshError = firstApiQuotaRefreshError(data, normalizedKeys);
          setNotice(
            refreshError
              ? {
                  type: "error",
                  message: copy.notices.apiQuotaRefreshFailed(localizeError(refreshError)),
                }
              : {
                  type: "ok",
                  message:
                    options?.notice ?? copy.notices.apiQuotaRefreshed(normalizedKeys.length),
                },
          );
        }
      } catch (error) {
        if (!quiet) {
          setNotice({
            type: "error",
            message: copy.notices.apiQuotaRefreshFailed(localizeError(String(error))),
          });
        }
      } finally {
        if (!quiet) {
          setRefreshing(false);
        }
      }
    },
    [applyAccounts, applyRefreshAccounts, copy.notices, localizeError],
  );

  const refreshAllApiQuota = useCallback(
    async (quiet = false) => {
      const providers = settingsRef.current.notificationProviders ?? [];
      const targetAccountKeys = accountsRef.current
        .filter((account) => accountHasApiQuotaProvider(account, providers))
        .map((account) => account.accountKey);
      const refreshedCount = targetAccountKeys.length;

      try {
        if (!quiet) {
          setRefreshing(true);
        }
        if (isPreviewRuntime()) {
          const now = nowUnixSeconds();
          const data = readPreviewAccounts().map((account) =>
            accountHasApiQuotaProvider(account, providers)
              ? {
                  ...account,
                  updatedAt: now,
                  profileLastValidationError: null,
                }
              : account,
          );
          writePreviewAccounts(data);
          const accountNoticeShown = applyAccounts(data);
          if (!quiet && !accountNoticeShown) {
            setNotice({ type: "ok", message: copy.notices.apiQuotaRefreshed(refreshedCount) });
          }
          return;
        }

        const data = await invoke<AccountSummary[]>("refresh_all_api_quota");
        const refreshError = firstApiQuotaRefreshError(data, targetAccountKeys);
        const accountNoticeShown = applyRefreshAccounts(
          data,
          quiet,
          copy.notices.apiQuotaRefreshed(refreshedCount),
        );
        if (!quiet && !accountNoticeShown) {
          setNotice(
            refreshError
              ? {
                  type: "error",
                  message: copy.notices.apiQuotaRefreshFailed(localizeError(refreshError)),
                }
              : { type: "ok", message: copy.notices.apiQuotaRefreshed(refreshedCount) },
          );
        }
      } catch (error) {
        if (!quiet) {
          setNotice({
            type: "error",
            message: copy.notices.apiQuotaRefreshFailed(localizeError(String(error))),
          });
        }
      } finally {
        if (!quiet) {
          setRefreshing(false);
        }
      }
    },
    [applyAccounts, applyRefreshAccounts, copy.notices, localizeError],
  );

  const refreshTokenUsage = useCallback(async (quiet = false) => {
    try {
      if (!quiet) {
        setRefreshingTokenUsage(true);
      }
      if (isPreviewRuntime()) {
        setTokenUsage(buildPreviewTokenUsage());
        setTokenUsageError(null);
        return;
      }

      const data = await invoke<CodexTokenUsageSnapshot>("get_codex_token_usage");
      setTokenUsage(data);
      setTokenUsageError(null);
    } catch (error) {
      const localized = localizeError(String(error));
      setTokenUsageError(localized);
      if (!quiet) {
        setNotice({
          type: "error",
          message: copy.notices.refreshFailed(localized),
        });
      }
    } finally {
      if (!quiet) {
        setRefreshingTokenUsage(false);
      }
    }
  }, [copy.notices, localizeError]);

  const applyImportResult = useCallback(
    async (result: ImportAccountsResult, prefix: string) => {
      const successCount = result.importedCount + result.updatedCount;
      if (successCount > 0) {
        await loadAccounts();
      }

      if (successCount > 0 && result.failures.length === 0) {
        setAddDialogOpen(false);
      }

      setNotice(buildImportNotice(result, prefix, copy.notices, locale));
    },
    [copy.notices, loadAccounts, locale],
  );

  useEffect(() => {
    installingUpdateRef.current = installingUpdate;
  }, [installingUpdate]);

  useEffect(() => {
    if (!notice) {
      return;
    }
    const ttl = notice.type === "error" ? 6_000 : 3_500;
    const timer = window.setTimeout(() => {
      setNotice((current) => (current === notice ? null : current));
    }, ttl);
    return () => {
      window.clearTimeout(timer);
    };
  }, [notice]);

  useEffect(
    () => () => {
      if (deleteConfirmTimerRef.current !== null) {
        window.clearTimeout(deleteConfirmTimerRef.current);
        deleteConfirmTimerRef.current = null;
      }
    },
    [],
  );

  const installPendingUpdate = useCallback(
    async (knownUpdate?: NonNullable<Awaited<ReturnType<typeof check>>>) => {
      if (isPreviewRuntime()) {
        setNotice({ type: "info", message: "预览环境不会安装更新。" });
        return;
      }

      if (installingUpdateRef.current) {
        return;
      }

      setInstallingUpdate(true);
      setUpdateProgress(copy.notices.preparingUpdateDownload);
      try {
        const update = knownUpdate ?? (await check());
        if (!update) {
          setPendingUpdate(null);
          setUpdateDialogOpen(false);
          setNotice({ type: "ok", message: copy.notices.alreadyLatest });
          return;
        }

        let totalBytes = 0;
        let downloadedBytes = 0;
        await update.downloadAndInstall((event) => {
          if (event.event === "Started") {
            totalBytes = event.data.contentLength ?? 0;
            downloadedBytes = 0;
            setUpdateProgress(copy.notices.updateDownloadStarted);
          } else if (event.event === "Progress") {
            downloadedBytes += event.data.chunkLength;
            if (totalBytes > 0) {
              const percentValue = Math.min(
                100,
                Math.round((downloadedBytes / totalBytes) * 100),
              );
              setUpdateProgress(copy.notices.updateDownloadingPercent(percentValue));
            } else {
              setUpdateProgress(copy.notices.updateDownloading);
            }
          } else if (event.event === "Finished") {
            setUpdateProgress(copy.notices.updateDownloadFinished);
          }
        });

        setUpdateProgress(copy.notices.updateInstalling);
        await relaunch();
      } catch (error) {
        setNotice({
          type: "error",
          message: copy.notices.updateInstallFailed(localizeError(String(error))),
        });
        setUpdateProgress(null);
      } finally {
        setInstallingUpdate(false);
      }
    },
    [copy.notices, localizeError],
  );

  const checkForAppUpdate = useCallback(
    async (quiet = false) => {
      if (isPreviewRuntime()) {
        if (!quiet) {
          setNotice({ type: "ok", message: "预览环境已跳过更新检查。" });
        }
        return;
      }

      if (!quiet) {
        setCheckingUpdate(true);
      }
      try {
        const update = await check();
        if (update) {
          if (quiet && settingsRef.current.skippedUpdateVersion === update.version) {
            return;
          }

          setUpdateProgress(null);
          setPendingUpdate({
            currentVersion: update.currentVersion,
            version: update.version,
            body: update.body,
            date: update.date,
          });
          setUpdateDialogOpen(true);
          if (!quiet) {
            setNotice({
              type: "info",
              message: copy.notices.foundNewVersion(update.version, update.currentVersion),
            });
          }
        } else {
          setPendingUpdate(null);
          setUpdateDialogOpen(false);
          setUpdateProgress(null);
          if (!quiet) {
            setNotice({ type: "ok", message: copy.notices.alreadyLatest });
          }
        }
      } catch (error) {
        const errorMessage = localizeError(String(error));
        if (!quiet) {
          setNotice({
            type: "error",
            message: isMissingUpdaterJsonError(errorMessage)
              ? copy.notices.updateCheckFailedWithUpdaterHint(errorMessage)
              : copy.notices.updateCheckFailed(errorMessage),
          });
        }
      } finally {
        if (!quiet) {
          setCheckingUpdate(false);
        }
      }
    },
    [copy.notices, localizeError],
  );

  const openManualDownloadPage = useCallback(async () => {
    if (isPreviewRuntime()) {
      window.open(PROJECT_LATEST_RELEASE_URL, "_blank", "noopener,noreferrer");
      return;
    }

    try {
      await invoke("open_external_url", { url: PROJECT_LATEST_RELEASE_URL });
    } catch (error) {
      setNotice({
        type: "error",
        message: copy.notices.openManualDownloadFailed(localizeError(String(error))),
      });
    }
  }, [copy.notices, localizeError]);

  const openExternalUrl = useCallback(async (url: string) => {
    if (isPreviewRuntime()) {
      window.open(url, "_blank", "noopener,noreferrer");
      return;
    }

    try {
      await invoke("open_external_url", { url });
    } catch (error) {
      setNotice({
        type: "error",
        message: copy.notices.openExternalFailed(localizeError(String(error))),
      });
    }
  }, [copy.notices, localizeError]);

  const closeUpdateDialog = useCallback(() => {
    setUpdateDialogOpen(false);
  }, []);

  const skipPendingUpdateVersion = useCallback(async () => {
    if (!pendingUpdate) {
      return;
    }

    setPendingUpdate(null);
    setUpdateProgress(null);
    setUpdateDialogOpen(false);
    await updateSettings(
      { skippedUpdateVersion: pendingUpdate.version },
      { silent: true, keepInteractive: true },
    );
  }, [pendingUpdate, updateSettings]);

  const startupCallbacksRef = useRef({
    checkForAppUpdate,
    loadAccounts,
    loadInstalledEditorApps,
    loadOpencodeDesktopAppInstalled,
    loadSettings,
    localizeError,
    maybeShowProfileIntegrityNotice,
    refreshAllApiQuota,
    refreshTokenUsage,
    refreshUsage,
    runStartupMaintenance,
  });
  startupCallbacksRef.current = {
    checkForAppUpdate,
    loadAccounts,
    loadInstalledEditorApps,
    loadOpencodeDesktopAppInstalled,
    loadSettings,
    localizeError,
    maybeShowProfileIntegrityNotice,
    refreshAllApiQuota,
    refreshTokenUsage,
    refreshUsage,
    runStartupMaintenance,
  };

  useEffect(() => {
    let cancelled = false;
    let startupMaintenanceTimer: number | null = null;
    let startupFailureNoticeShown = false;

    const showStartupFailure = (message: string) => {
      if (cancelled || startupFailureNoticeShown) {
        return;
      }
      startupFailureNoticeShown = true;
      setNotice({
        type: "error",
        message,
      });
    };

    const scheduleDeferredStartupWork = () => {
      if (startupMaintenanceTimer !== null) {
        return;
      }
      startupMaintenanceTimer = window.setTimeout(() => {
        if (cancelled) {
          return;
        }
        const callbacks = startupCallbacksRef.current;
        void callbacks.loadInstalledEditorApps();
        void callbacks.loadOpencodeDesktopAppInstalled();
        void callbacks.runStartupMaintenance()
          .then((items) => {
            if (!cancelled) {
              callbacks.maybeShowProfileIntegrityNotice(items);
            }
          })
          .catch((error) => {
            console.warn("startup maintenance failed", error);
          });
        if (settingsRef.current.usageAutoRefreshEnabled) {
          void callbacks.refreshUsage(true);
        }
        if (settingsRef.current.apiQuotaAutoRefreshEnabled) {
          void callbacks.refreshAllApiQuota(true);
        }
        void callbacks.refreshTokenUsage(true);
        void callbacks.checkForAppUpdate(true);
      }, 3000);
    };

    const bootstrap = async () => {
      const callbacks = startupCallbacksRef.current;
      const settingsPromise = callbacks.loadSettings().catch((error) => {
        showStartupFailure(`启动设置加载失败：${callbacks.localizeError(String(error))}`);
      });

      try {
        const initialAccounts = await callbacks.loadAccounts();
        callbacks.maybeShowProfileIntegrityNotice(initialAccounts);
      } catch (error) {
        showStartupFailure(`启动账号加载失败：${callbacks.localizeError(String(error))}`);
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }

      void settingsPromise.finally(() => {
        if (!cancelled) {
          scheduleDeferredStartupWork();
        }
      });
    };

    void bootstrap();

    return () => {
      cancelled = true;
      if (startupMaintenanceTimer !== null) {
        clearTimeout(startupMaintenanceTimer);
      }
    };
  }, []);

  useEffect(() => {
    if (!settings.usageAutoRefreshEnabled) {
      return;
    }
    const usageTimer =
      setInterval(() => {
        void refreshUsage(true);
      }, Math.max(15, settings.usageAutoRefreshIntervalSecs) * 1000);

    return () => {
      clearInterval(usageTimer);
    };
  }, [refreshUsage, settings.usageAutoRefreshEnabled, settings.usageAutoRefreshIntervalSecs]);

  useEffect(() => {
    if (!settings.apiQuotaAutoRefreshEnabled) {
      return;
    }
    const apiQuotaTimer =
      setInterval(() => {
        void refreshAllApiQuota(true);
      }, Math.max(60, settings.apiQuotaAutoRefreshIntervalSecs) * 1000);

    return () => {
      clearInterval(apiQuotaTimer);
    };
  }, [
    refreshAllApiQuota,
    settings.apiQuotaAutoRefreshEnabled,
    settings.apiQuotaAutoRefreshIntervalSecs,
  ]);

  useEffect(() => {
    const tokenUsageTimer = setInterval(() => {
      void refreshTokenUsage(true);
    }, TOKEN_USAGE_REFRESH_MS);

    return () => {
      clearInterval(tokenUsageTimer);
    };
  }, [refreshTokenUsage]);

  useEffect(() => {
    const editorTimer = setInterval(() => {
      void loadInstalledEditorApps();
      void loadOpencodeDesktopAppInstalled();
    }, EDITOR_SCAN_MS);

    return () => {
      clearInterval(editorTimer);
    };
  }, [loadInstalledEditorApps, loadOpencodeDesktopAppInstalled]);

  useEffect(() => {
    const updateTimer = setInterval(() => {
      void checkForAppUpdate(true);
    }, UPDATE_CHECK_MS);

    return () => {
      clearInterval(updateTimer);
    };
  }, [checkForAppUpdate]);

  useEffect(() => {
    if (loading) {
      return;
    }
    if (!localeRelocalizeReadyRef.current) {
      localeRelocalizeReadyRef.current = true;
      return;
    }

    void loadAccounts();
  }, [loadAccounts, loading, locale]);

  useEffect(() => {
    if (isPreviewRuntime()) {
      return;
    }

    let disposed = false;
    let unlisten: UnlistenFn | null = null;

    void listen<OauthCallbackFinishedEvent>("oauth-callback-finished", (event) => {
      if (disposed) {
        return;
      }

      setOauthWaitingForCallback(false);
      if (event.payload.result) {
        void applyImportResult(
          localizeImportResult(event.payload.result),
          copy.notices.oauthImportPrefix,
        );
        setReauthorizeAccount(null);
        return;
      }

      if (event.payload.error) {
        setNotice({
          type: "error",
          message: copy.notices.importFailedPlain(
            copy.notices.oauthImportPrefix,
            localizeError(event.payload.error),
          ),
        });
      }
    })
      .then((fn) => {
        if (disposed) {
          void fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      if (unlisten) {
        void unlisten();
      }
    };
  }, [applyImportResult, copy.notices, localizeError, localizeImportResult]);

  const onOpenAddDialog = useCallback(() => {
    setOauthWaitingForCallback(false);
    setReauthorizeAccount(null);
    setAddDialogOpen(true);
  }, []);

  const onPrepareOauthLogin = useCallback(async () => {
    setOauthWaitingForCallback(false);
    try {
      return await invoke<PreparedOauthLogin>("prepare_oauth_login", {
        accountId: reauthorizeAccount?.id ?? null,
      });
    } catch (error) {
      setNotice({
        type: "error",
        message: copy.notices.oauthLinkPrepareFailed(localizeError(String(error))),
      });
      throw error;
    }
  }, [copy.notices, localizeError, reauthorizeAccount]);

  const onOpenOauthAuthorizationPage = useCallback(
    async (url: string) => {
      setOauthWaitingForCallback(true);
      try {
        await invoke<void>("open_external_url", { url });
      } catch (error) {
        setOauthWaitingForCallback(false);
        setNotice({
          type: "error",
          message: copy.notices.openExternalFailed(localizeError(String(error))),
        });
      }
    },
    [copy.notices, localizeError],
  );

  const onCancelOauthLogin = useCallback(async () => {
    setOauthWaitingForCallback(false);
    try {
      await invoke<void>("cancel_oauth_login");
    } catch {
      // Ignore cancel failures so closing the dialog stays responsive.
    }
  }, []);

  const onCloseAddDialog = useCallback(() => {
    if (importingAccounts) {
      return;
    }

    void onCancelOauthLogin();
    setAddDialogOpen(false);
    setReauthorizeAccount(null);
  }, [importingAccounts, onCancelOauthLogin]);

  const onReauthorizeAccount = useCallback((account: AccountSummary) => {
    setOauthWaitingForCallback(false);
    setReauthorizeAccount(account);
    setAddDialogOpen(true);
  }, []);

  const onImportCurrentAuth = useCallback(async () => {
    if (importingAccounts) {
      return;
    }

    setImportingAccounts(true);
    try {
      await invoke<AccountSummary>("import_current_auth_account", { label: null });
      await refreshUsage(true);
      await loadAccounts();
      setAddDialogOpen(false);
      setNotice({ type: "ok", message: copy.notices.currentAccountImportSuccess });
    } catch (error) {
      setNotice({
        type: "error",
        message: copy.notices.currentAccountImportFailed(localizeError(String(error))),
      });
    } finally {
      setImportingAccounts(false);
    }
  }, [copy.notices, importingAccounts, loadAccounts, localizeError, refreshUsage]);

  const onImportAuthFiles = useCallback(
    async (items: AuthJsonImportInput[]) => {
      if (items.length === 0) {
        setNotice({ type: "error", message: copy.notices.importFilesRequired });
        return;
      }

      setImportingAccounts(true);
      try {
        const result = await invoke<ImportAccountsResult>("import_auth_json_accounts", {
          items,
        });
        await applyImportResult(localizeImportResult(result), copy.notices.fileImportPrefix);
      } catch (error) {
        setNotice({
          type: "error",
          message: copy.notices.importFailedPlain(
            copy.notices.fileImportPrefix,
            localizeError(String(error)),
          ),
        });
      } finally {
        setImportingAccounts(false);
      }
    },
    [applyImportResult, copy.notices, localizeError, localizeImportResult],
  );

  const onCreateApiAccount = useCallback(
    async (input: CreateApiAccountInput) => {
      setImportingAccounts(true);
      const normalizedModelCatalog = normalizeRelayModelCatalog(
        input.modelName,
        input.modelCatalog,
      );
      const normalizedInput: CreateApiAccountInput = {
        ...input,
        modelCatalog: normalizedModelCatalog,
        apiQuotaSubscriptionName:
          input.balanceDisplayEnabled === false
            ? null
            : normalizeApiQuotaSubscriptionName(input.apiQuotaSubscriptionName),
      };
      try {
        if (isPreviewRuntime()) {
          const now = nowUnixSeconds();
          const nextAccount: AccountSummary = {
            id: `preview-api-${now}`,
            label: input.label.trim(),
            sourceKind: "relay",
            email: null,
            accountKey: `preview-api-${now}`,
            accountId: `preview-api-${now}`,
            planType: "api",
            apiBaseUrl: normalizedInput.baseUrl.trim(),
            modelName: normalizedInput.modelName.trim(),
            modelCatalog: normalizedModelCatalog,
            modelRoutingEnabled: false,
            balanceText: normalizedInput.balanceDisplayEnabled ? "$50.00" : null,
            balanceDisplayEnabled: Boolean(normalizedInput.balanceDisplayEnabled),
            apiQuotaMode: normalizedInput.apiQuotaMode ?? "apiOnly",
            apiQuotaSubscriptionName: normalizedInput.apiQuotaSubscriptionName ?? null,
            providerId: null,
            providerName: null,
            tags: normalizedInput.tags,
            profileAuthReady: true,
            profileConfigReady: true,
            profileIntegrityError: null,
            profileLastValidatedAt: now,
            profileLastValidationError: null,
            addedAt: now,
            updatedAt: now,
            usage: {
              fetchedAt: now,
              planType: "api",
              fiveHour: {
                usedPercent: 12,
                windowSeconds: 18_000,
                resetAt: now + 14_400,
              },
              oneWeek: {
                usedPercent: 18,
                windowSeconds: 604_800,
                resetAt: now + 500_000,
              },
              credits: {
                hasCredits: true,
                unlimited: false,
                balance: "50.00",
              },
            },
            usageError: null,
            authRefreshBlocked: false,
            authRefreshError: null,
            authRefreshNextAt: null,
            isCurrent: false,
          };
          const nextAccounts = [...readPreviewAccounts(), nextAccount];
          writePreviewAccounts(nextAccounts);
          applyAccounts(nextAccounts);
          const provider = buildNotificationProviderFromApiInput(
            normalizedInput,
            nextAccount.accountKey,
          );
          if (provider) {
            const testedProvider = await probeNotificationProviderForImport(provider);
            const providers = settingsRef.current.notificationProviders ?? [];
            const existingIndex = providers.findIndex(
              (item) =>
                item.accountKey === nextAccount.accountKey ||
                (!item.accountKey?.trim() &&
                  item.baseUrl === testedProvider.baseUrl &&
                  item.email.trim().toLowerCase() === testedProvider.email.trim().toLowerCase()),
            );
            const nextProviders =
              existingIndex >= 0
                ? providers.map((item, index) =>
                    index === existingIndex
                      ? {
                          ...testedProvider,
                          id: item.id,
                          createdAt: item.createdAt,
                        }
                      : item,
                  )
                : [...providers, testedProvider];
            const nextSettings = {
              ...settingsRef.current,
              notificationProviders: nextProviders,
              notificationSchemaVersion: 1,
            };
            settingsRef.current = nextSettings;
            setSettings(nextSettings);
            writePreviewSettings(nextSettings);
          }
          setAddDialogOpen(false);
          setNotice({
            type: "ok",
            message: copy.notices.apiAccountCreated(normalizedInput.label),
          });
          return;
        }

        const created = await invoke<AccountSummary>("create_api_account", {
          input: normalizedInput,
        });
        const provider = buildNotificationProviderFromApiInput(normalizedInput, created.accountKey);
        if (provider) {
          const testedProvider = await probeNotificationProviderForImport(provider);
          const providers = settingsRef.current.notificationProviders ?? [];
          const existingIndex = providers.findIndex(
            (item) =>
              item.accountKey === created.accountKey ||
              (!item.accountKey?.trim() &&
                item.baseUrl === testedProvider.baseUrl &&
                item.email.trim().toLowerCase() === testedProvider.email.trim().toLowerCase()),
          );
          const nextProviders =
            existingIndex >= 0
              ? providers.map((item, index) =>
                  index === existingIndex
                    ? {
                        ...testedProvider,
                        id: item.id,
                        createdAt: item.createdAt,
                      }
                    : item,
                )
              : [...providers, testedProvider];
          await updateSettings(
            { notificationProviders: nextProviders, notificationSchemaVersion: 1 },
            { silent: true },
          );
        }
        await loadAccounts();
        setAddDialogOpen(false);
        setNotice({
          type: "ok",
          message: copy.notices.apiAccountCreated(input.label),
        });
      } catch (error) {
        const message = localizeError(String(error));
        setNotice({
          type: "error",
          message: copy.notices.apiAccountCreateFailed(message),
        });
        throw new Error(message);
      } finally {
        setImportingAccounts(false);
      }
    },
    [applyAccounts, copy.notices, loadAccounts, localizeError, updateSettings],
  );

  const onUpdateAccountTags = useCallback(
    async (account: AccountSummary, rawInput: string): Promise<boolean> => {
      const tags = parseTagInput(rawInput);
      const unchanged =
        tags.length === account.tags.length &&
        tags.every((tag, index) => tag === account.tags[index]);
      if (unchanged) {
        return true;
      }

      try {
        const resolvedTags = await invoke<string[]>("update_account_tags", {
          accountKey: account.accountKey,
          tags,
        });
        updateAccountsState((prev) =>
          prev.map((item) =>
            item.accountKey === account.accountKey
              ? {
                  ...item,
                  tags: resolvedTags,
                }
              : item,
          ),
        );
        setNotice({
          type: "ok",
          message: copy.notices.accountTagsUpdated(noticeAccountLabel(account)),
        });
        return true;
      } catch (error) {
        setNotice({
          type: "error",
          message: copy.notices.accountTagsUpdateFailed(localizeError(String(error))),
        });
        return false;
      }
    },
    [copy.notices, localizeError, noticeAccountLabel, updateAccountsState],
  );

  const onCompleteOauthCallbackLogin = useCallback(
    async (callbackUrl: string) => {
      setOauthWaitingForCallback(false);
      setImportingAccounts(true);
      try {
        const result = await invoke<ImportAccountsResult>("complete_oauth_callback_login", {
          callbackUrl,
        });
        await applyImportResult(localizeImportResult(result), copy.notices.oauthImportPrefix);
        setReauthorizeAccount(null);
      } catch (error) {
        setNotice({
          type: "error",
          message: copy.notices.importFailedPlain(
            copy.notices.oauthImportPrefix,
            localizeError(String(error)),
          ),
        });
        throw error;
      } finally {
        setImportingAccounts(false);
      }
    },
    [
      applyImportResult,
      copy.notices,
      localizeError,
      localizeImportResult,
      setOauthWaitingForCallback,
    ],
  );

  const onExportAccounts = useCallback(async (
    account?: AccountSummary,
    format: AccountsExportFormat = "codexDeck",
    accountKeys?: string[],
  ) => {
    if (exportingAccounts) {
      return;
    }

    setExportingAccounts(true);
    try {
      if (isPreviewRuntime()) {
        setNotice({
          type: "ok",
          message: account ? "预览环境已模拟导出这个账号。" : copy.notices.accountsExported,
        });
        return;
      }

      const exportedPath = await invoke<string | null>("export_accounts_zip", {
        accountKey: account?.accountKey ?? null,
        accountKeys: accountKeys ?? null,
        format,
      });
      if (exportedPath) {
        setNotice({ type: "ok", message: copy.notices.accountsExported });
      }
    } catch (error) {
      setNotice({
        type: "error",
        message: copy.notices.accountsExportFailed(localizeError(String(error))),
      });
    } finally {
      setExportingAccounts(false);
    }
  }, [copy.notices, exportingAccounts, localizeError]);

  const onRenameAccountLabel = useCallback(
    async (account: AccountSummary, label: string): Promise<boolean> => {
      const normalizedLabel = label.trim();
      if (!normalizedLabel) {
        return false;
      }
      if (normalizedLabel === account.label.trim()) {
        return true;
      }
      if (renamingAccountId === account.accountKey) {
        return false;
      }

      setRenamingAccountId(account.accountKey);
      try {
        if (isPreviewRuntime()) {
          const nextAccounts = readPreviewAccounts().map((item) =>
            item.accountKey === account.accountKey
              ? {
                  ...item,
                  label: normalizedLabel,
                  updatedAt: nowUnixSeconds(),
                }
              : item,
          );
          writePreviewAccounts(nextAccounts);
          applyAccounts(nextAccounts);
          setNotice({
            type: "ok",
            message: copy.notices.accountAliasUpdated(
              noticeAccountLabel({ ...account, label: normalizedLabel }),
            ),
          });
          return true;
        }

        const resolvedLabel = await invoke<string>("update_account_label", {
          accountKey: account.accountKey,
          label: normalizedLabel,
        });
        updateAccountsState((prev) =>
          prev.map((item) =>
            item.accountKey === account.accountKey
              ? {
                  ...item,
                  label: resolvedLabel,
                }
              : item,
          ),
        );
        setNotice({
          type: "ok",
          message: copy.notices.accountAliasUpdated(
            noticeAccountLabel({ ...account, label: resolvedLabel }),
          ),
        });
        return true;
      } catch (error) {
        setNotice({
          type: "error",
          message: copy.notices.accountAliasUpdateFailed(localizeError(String(error))),
        });
        return false;
      } finally {
        setRenamingAccountId((current) =>
          current === account.accountKey ? null : current,
        );
      }
    },
    [
      applyAccounts,
      copy.notices,
      localizeError,
      noticeAccountLabel,
      renamingAccountId,
      updateAccountsState,
    ],
  );

  const onUpdateApiAccount = useCallback(
    async (account: AccountSummary, input: UpdateApiAccountInput): Promise<boolean> => {
      const normalizedLabel = input.label.trim();
      const normalizedBaseUrl = input.baseUrl.trim();
      const normalizedApiKey = input.apiKey?.trim() ?? "";
      const normalizedModelName = input.modelName.trim();
      const normalizedModelCatalog = normalizeRelayModelCatalog(
        normalizedModelName,
        input.modelCatalog,
      );
      const normalizedQuotaTodayUsedText = input.apiQuotaTodayUsedText?.trim() || null;
      const normalizedQuotaRemainingText = input.apiQuotaRemainingText?.trim() || null;
      const normalizedQuotaSubscriptionName =
        input.balanceDisplayEnabled === false
          ? null
          : normalizeApiQuotaSubscriptionName(input.apiQuotaSubscriptionName);

      if (!normalizedLabel || !normalizedBaseUrl || !normalizedModelName) {
        return false;
      }
      if (renamingAccountId === account.accountKey) {
        return false;
      }

      setRenamingAccountId(account.accountKey);
      try {
        if (isPreviewRuntime()) {
          const nextAccounts = readPreviewAccounts().map((item) =>
            item.accountKey === account.accountKey
              ? {
                  ...item,
                  label: normalizedLabel,
                  apiBaseUrl: normalizedBaseUrl,
                  modelName: normalizedModelName,
                  modelCatalog: normalizedModelCatalog,
                  modelRoutingEnabled: false,
                  balanceDisplayEnabled: input.balanceDisplayEnabled ?? item.balanceDisplayEnabled,
                  balanceText: input.balanceDisplayEnabled === false ? null : item.balanceText,
                  apiQuotaMode:
                    input.balanceDisplayEnabled === false
                      ? "apiOnly"
                      : input.apiQuotaMode ?? item.apiQuotaMode ?? "apiOnly",
                  apiQuotaTodayUsedText:
                    input.balanceDisplayEnabled === false ? null : normalizedQuotaTodayUsedText,
                  apiQuotaRemainingText:
                    input.balanceDisplayEnabled === false ? null : normalizedQuotaRemainingText,
                  apiQuotaTotalRemainingText:
                    input.balanceDisplayEnabled === false ? null : item.apiQuotaTotalRemainingText,
                  apiQuotaTotalTokensText:
                    input.balanceDisplayEnabled === false ? null : item.apiQuotaTotalTokensText,
                  apiQuotaTodayTokensText:
                    input.balanceDisplayEnabled === false ? null : item.apiQuotaTodayTokensText,
                  apiQuotaDailyWindow:
                    input.balanceDisplayEnabled === false ? null : item.apiQuotaDailyWindow,
                  apiQuotaTotalWindow:
                    input.balanceDisplayEnabled === false ? null : item.apiQuotaTotalWindow,
                  apiQuotaSubscriptionExpiresAt:
                    input.balanceDisplayEnabled === false
                      ? null
                      : item.apiQuotaSubscriptionExpiresAt,
                  apiQuotaSubscriptionName:
                    input.balanceDisplayEnabled === false
                      ? null
                      : normalizedQuotaSubscriptionName,
                  profileLastValidationError:
                    input.balanceDisplayEnabled === false ? null : item.profileLastValidationError,
                  updatedAt: nowUnixSeconds(),
                }
              : item,
          );
          writePreviewAccounts(nextAccounts);
          applyAccounts(nextAccounts);
          if (input.platformLoginEmail !== undefined || input.platformLoginPassword !== undefined) {
            const nextProviders = await upsertNotificationProviderForApiUpdate(
              input,
              account.accountKey,
              settingsRef.current.notificationProviders ?? [],
            );
            if (nextProviders !== settingsRef.current.notificationProviders) {
              const nextSettings = {
                ...settingsRef.current,
                notificationProviders: nextProviders,
                notificationSchemaVersion: 1,
              };
              settingsRef.current = nextSettings;
              setSettings(nextSettings);
              writePreviewSettings(nextSettings);
            }
          }
          setNotice({
            type: "ok",
            message: copy.notices.apiAccountUpdated(
              noticeAccountLabel({
                ...account,
                label: normalizedLabel,
                apiBaseUrl: normalizedBaseUrl,
                modelName: normalizedModelName,
              }),
            ),
          });
          return true;
        }

        const updated = await invoke<AccountSummary>("update_api_account", {
          accountKey: account.accountKey,
          input: {
            label: normalizedLabel,
            baseUrl: normalizedBaseUrl,
            apiKey: normalizedApiKey ? normalizedApiKey : null,
            modelName: normalizedModelName,
            modelCatalog: normalizedModelCatalog,
            balanceDisplayEnabled: input.balanceDisplayEnabled,
            apiQuotaMode: input.apiQuotaMode ?? "apiOnly",
            apiQuotaTodayUsedText: normalizedQuotaTodayUsedText,
            apiQuotaRemainingText: normalizedQuotaRemainingText,
            apiQuotaSubscriptionName: normalizedQuotaSubscriptionName,
            platformLoginEmail: input.platformLoginEmail,
            platformLoginPassword: input.platformLoginPassword,
          },
        });
        if (input.platformLoginEmail !== undefined || input.platformLoginPassword !== undefined) {
          const nextProviders = await upsertNotificationProviderForApiUpdate(
            input,
            account.accountKey,
            settingsRef.current.notificationProviders ?? [],
          );
          await updateSettings(
            { notificationProviders: nextProviders, notificationSchemaVersion: 1 },
            { silent: true },
          );
        }

        updateAccountsState((prev) =>
          prev.map((item) =>
            item.accountKey === account.accountKey
              ? {
                  ...item,
                  ...updated,
                  isCurrent: item.isCurrent,
                }
              : item,
          ),
        );
        setNotice({
          type: "ok",
          message: copy.notices.apiAccountUpdated(noticeAccountLabel(updated)),
        });
        return true;
      } catch (error) {
        setNotice({
          type: "error",
          message: copy.notices.apiAccountUpdateFailed(localizeError(String(error))),
        });
        return false;
      } finally {
        setRenamingAccountId((current) =>
          current === account.accountKey ? null : current,
        );
      }
    },
    [
      applyAccounts,
      copy.notices,
      localizeError,
      noticeAccountLabel,
      renamingAccountId,
      updateAccountsState,
      updateSettings,
    ],
  );

  const onProbeApiModels = useCallback(
    async (
      baseUrl: string,
      apiKey: string | null,
      accountKey?: string,
    ): Promise<RelayModelCatalogEntry[]> => {
      const normalizedBaseUrl = baseUrl.trim();
      const normalizedApiKey = apiKey?.trim() ?? "";
      const normalizedAccountKey = accountKey?.trim() ?? "";
      if (!normalizedBaseUrl || (!normalizedApiKey && !normalizedAccountKey)) {
        throw new Error("请先填写 Base URL 和 API Key。");
      }
      if (isPreviewRuntime()) {
        return normalizeModelCatalogContextWindows(previewProbeApiModels(normalizedBaseUrl));
      }

      const entries = normalizedAccountKey
        ? await invoke<RelayModelCatalogEntry[]>("probe_api_account_models", {
            accountKey: normalizedAccountKey,
            baseUrl: normalizedBaseUrl,
            apiKey: normalizedApiKey || null,
          })
        : await invoke<RelayModelCatalogEntry[]>("probe_api_models", {
            baseUrl: normalizedBaseUrl,
            apiKey: normalizedApiKey,
          });
      return normalizeModelCatalogContextWindows(entries);
    },
    [],
  );

  const onUpdateApiAccountKeys = useCallback(
    async (
      account: AccountSummary,
      keys: UpdateApiAccountKeyInput[],
    ): Promise<boolean> => {
      if (renamingAccountId === account.accountKey) {
        return false;
      }

      setRenamingAccountId(account.accountKey);
      try {
        if (isPreviewRuntime()) {
          setNotice({
            type: "ok",
            message: copy.notices.apiAccountKeysUpdated(noticeAccountLabel(account)),
          });
          return true;
        }

        const updated = await invoke<AccountSummary>("update_api_account_keys", {
          accountKey: account.accountKey,
          keys,
        });

        updateAccountsState((prev) =>
          prev.map((item) =>
            item.accountKey === account.accountKey
              ? {
                  ...item,
                  ...updated,
                  isCurrent: item.isCurrent,
                }
              : item,
          ),
        );
        setNotice({
          type: "ok",
          message: copy.notices.apiAccountKeysUpdated(noticeAccountLabel(updated)),
        });
        return true;
      } catch (error) {
        setNotice({
          type: "error",
          message: copy.notices.apiAccountKeysUpdateFailed(localizeError(String(error))),
        });
        return false;
      } finally {
        setRenamingAccountId((current) =>
          current === account.accountKey ? null : current,
        );
      }
    },
    [copy.notices, localizeError, noticeAccountLabel, renamingAccountId, updateAccountsState],
  );

  const onProbeApiAccountKey = useCallback(
    async (account: AccountSummary, keyId: string): Promise<boolean> => {
      if (renamingAccountId === account.accountKey) {
        return false;
      }

      setRenamingAccountId(account.accountKey);
      try {
        if (isPreviewRuntime()) {
          setNotice({
            type: "ok",
            message: copy.notices.apiAccountKeyProbeHealthy(noticeAccountLabel(account)),
          });
          return true;
        }

        const updated = await invoke<AccountSummary>("probe_api_account_key", {
          accountKey: account.accountKey,
          keyId,
        });
        updateAccountsState((prev) =>
          prev.map((item) =>
            item.accountKey === account.accountKey
              ? {
                  ...item,
                  ...updated,
                  isCurrent: item.isCurrent,
                }
              : item,
          ),
        );
        setNotice({
          type: "ok",
          message: copy.notices.apiAccountKeyProbeHealthy(noticeAccountLabel(updated)),
        });
        return true;
      } catch (error) {
        setNotice({
          type: "error",
          message: copy.notices.apiAccountKeyProbeFailed(localizeError(String(error))),
        });
        return false;
      } finally {
        setRenamingAccountId((current) =>
          current === account.accountKey ? null : current,
        );
      }
    },
    [copy.notices, localizeError, noticeAccountLabel, renamingAccountId, updateAccountsState],
  );

  const onDelete = useCallback(async (account: AccountSummary) => {
    if (pendingDeleteId !== account.id) {
      setPendingDeleteId(account.id);
      if (deleteConfirmTimerRef.current !== null) {
        window.clearTimeout(deleteConfirmTimerRef.current);
      }
      deleteConfirmTimerRef.current = window.setTimeout(() => {
        setPendingDeleteId((current) => (current === account.id ? null : current));
        deleteConfirmTimerRef.current = null;
      }, 5_000);
      setNotice({
        type: "info",
        message: copy.notices.deleteConfirm(noticeAccountLabel(account)),
      });
      return;
    }

    if (deleteConfirmTimerRef.current !== null) {
      window.clearTimeout(deleteConfirmTimerRef.current);
      deleteConfirmTimerRef.current = null;
    }
    setPendingDeleteId(null);

    try {
      if (isPreviewRuntime()) {
        const nextAccounts = readPreviewAccounts().filter((item) => item.id !== account.id);
        writePreviewAccounts(nextAccounts);
        applyAccounts(nextAccounts);
        setNotice({ type: "ok", message: copy.notices.accountDeleted });
        return;
      }

      await invoke<void>("delete_account", { id: account.id });
      updateAccountsState((prev) => prev.filter((item) => item.id !== account.id));
      setNotice({ type: "ok", message: copy.notices.accountDeleted });
    } catch (error) {
      setNotice({
        type: "error",
        message: copy.notices.deleteFailed(localizeError(String(error))),
      });
    }
  }, [
    applyAccounts,
    copy.notices,
    localizeError,
    noticeAccountLabel,
    pendingDeleteId,
    updateAccountsState,
  ]);

  const onSwitch = useCallback(
    async (account: AccountSummary, options?: { useModelRouter?: boolean }) => {
      const useModelRouter = Boolean(options?.useModelRouter);
      const switchingKey = useModelRouter ? `router:${account.id}` : account.id;
      setSwitchingId(switchingKey);
      try {
        if (isPreviewRuntime()) {
          const nextAccounts = readPreviewAccounts().map((item) => ({
            ...item,
            isCurrent: item.id === account.id,
          }));
          writePreviewAccounts(nextAccounts);
          applyAccounts(nextAccounts);
          setNotice({ type: "ok", message: copy.notices.switchedOnly });
          return;
        }

        const result = await invoke<SwitchAccountResult>("switch_account_and_launch", {
          id: account.id,
          workspacePath: null,
          launchCodex: settings.launchCodexAfterSwitch,
          useModelRouter,
          restartEditorsOnSwitch: settings.restartEditorsOnSwitch,
          restartEditorTargets: settings.restartEditorTargets,
        });
        await loadAccounts();

        let baseNotice: Notice;
        if (!settings.launchCodexAfterSwitch) {
          baseNotice = { type: "ok", message: copy.notices.switchedOnly };
        } else if (result.usedFallbackCli) {
          baseNotice = {
            type: "info",
            message: copy.notices.switchedAndLaunchByCli,
          };
        } else {
          baseNotice = { type: "ok", message: copy.notices.switchedAndLaunching };
        }

        if (settings.syncOpencodeOpenaiAuth) {
          if (result.opencodeSyncError) {
            baseNotice = {
              type: "error",
              message: copy.notices.opencodeSyncFailed(
                baseNotice.message,
                localizeError(result.opencodeSyncError),
              ),
            };
          } else if (result.opencodeSynced) {
            baseNotice = {
              ...baseNotice,
              message: copy.notices.opencodeSynced(baseNotice.message),
            };
          }

          if (settings.restartOpencodeDesktopOnSwitch) {
            if (result.opencodeDesktopRestartError) {
              baseNotice = {
                type: "error",
                message: copy.notices.opencodeDesktopRestartFailed(
                  baseNotice.message,
                  localizeError(result.opencodeDesktopRestartError),
                ),
              };
            } else if (result.opencodeDesktopRestarted) {
              baseNotice = {
                ...baseNotice,
                message: copy.notices.opencodeDesktopRestarted(baseNotice.message),
              };
            }
          }
        }

        if (settings.restartEditorsOnSwitch) {
          if (result.editorRestartError) {
            baseNotice = {
              type: "error",
              message: copy.notices.editorRestartFailed(
                baseNotice.message,
                localizeError(result.editorRestartError),
              ),
            };
          } else if (result.restartedEditorApps.length > 0) {
            const restartedLabels = result.restartedEditorApps
              .map((id) => copy.editorAppLabels[id] ?? id)
              .join(" / ");
            baseNotice = {
              ...baseNotice,
              message: copy.notices.editorsRestarted(baseNotice.message, restartedLabels),
            };
          } else {
            baseNotice = {
              ...baseNotice,
              message: copy.notices.noEditorRestarted(baseNotice.message),
            };
          }
        }

        setNotice(baseNotice);
      } catch (error) {
        setNotice({
          type: "error",
          message: copy.notices.switchFailed(localizeError(String(error))),
        });
      } finally {
        setSwitchingId(null);
      }
    },
    [
      copy.editorAppLabels,
      copy.notices,
      applyAccounts,
      loadAccounts,
      localizeError,
      settings.launchCodexAfterSwitch,
      settings.syncOpencodeOpenaiAuth,
      settings.restartOpencodeDesktopOnSwitch,
      settings.restartEditorsOnSwitch,
      settings.restartEditorTargets,
    ],
  );

  const onSwitchHybrid = useCallback(
    async (
      chatgptAccount: AccountSummary,
      relayAccount: AccountSummary,
      options?: { useModelRouter?: boolean },
    ) => {
      const useModelRouter = Boolean(options?.useModelRouter);
      const switchingKey = `${useModelRouter ? "router-hybrid" : "hybrid"}:${chatgptAccount.id}:${relayAccount.id}`;
      setSwitchingId(switchingKey);
      try {
        if (isPreviewRuntime()) {
          const nextAccounts = readPreviewAccounts().map((item) => ({
            ...item,
            isCurrent: item.id === relayAccount.id,
          }));
          writePreviewAccounts(nextAccounts);
          applyAccounts(nextAccounts);
          setNotice({ type: "ok", message: copy.notices.hybridSwitchedOnly });
          return;
        }

        const result = await invoke<SwitchAccountResult>("switch_hybrid_account_and_launch", {
          chatgptAccountId: chatgptAccount.id,
          relayAccountId: relayAccount.id,
          workspacePath: null,
          launchCodex: settings.launchCodexAfterSwitch,
          useModelRouter,
          restartEditorsOnSwitch: settings.restartEditorsOnSwitch,
          restartEditorTargets: settings.restartEditorTargets,
        });
        await loadAccounts();

        let baseNotice: Notice;
        if (!settings.launchCodexAfterSwitch) {
          baseNotice = { type: "ok", message: copy.notices.hybridSwitchedOnly };
        } else if (result.usedFallbackCli) {
          baseNotice = {
            type: "info",
            message: copy.notices.hybridSwitchedAndLaunchByCli,
          };
        } else {
          baseNotice = { type: "ok", message: copy.notices.hybridSwitchedAndLaunching };
        }

        if (settings.restartEditorsOnSwitch) {
          if (result.editorRestartError) {
            baseNotice = {
              type: "error",
              message: copy.notices.editorRestartFailed(
                baseNotice.message,
                localizeError(result.editorRestartError),
              ),
            };
          } else if (result.restartedEditorApps.length > 0) {
            const restartedLabels = result.restartedEditorApps
              .map((id) => copy.editorAppLabels[id] ?? id)
              .join(" / ");
            baseNotice = {
              ...baseNotice,
              message: copy.notices.editorsRestarted(baseNotice.message, restartedLabels),
            };
          } else {
            baseNotice = {
              ...baseNotice,
              message: copy.notices.noEditorRestarted(baseNotice.message),
            };
          }
        }

        setNotice(baseNotice);
      } catch (error) {
        setNotice({
          type: "error",
          message: copy.notices.switchFailed(localizeError(String(error))),
        });
      } finally {
        setSwitchingId(null);
      }
    },
    [
      copy.editorAppLabels,
      copy.notices,
      applyAccounts,
      loadAccounts,
      localizeError,
      settings.launchCodexAfterSwitch,
      settings.restartEditorsOnSwitch,
      settings.restartEditorTargets,
    ],
  );

  const onSetModelRouterMode = useCallback(
    async (enabled: boolean, relayAccountId: string | null) => {
      const switchingKey = enabled ? `router-mode:${relayAccountId ?? "auto"}` : "router-mode:off";
      setSwitchingId(switchingKey);
      try {
        if (isPreviewRuntime()) {
          const data = {
            ...settingsRef.current,
            modelRouterEnabled: enabled,
            modelRouterAccountId: relayAccountId,
          };
          settingsRef.current = data;
          setSettings(data);
          writePreviewSettings(data);
          setNotice({
            type: "ok",
            message: enabled ? "路由模式已开启。" : "路由模式已关闭，已恢复之前的 Codex 配置。",
          });
          return;
        }

        const data = await invoke<AppSettings>("set_model_router_mode", {
          enabled,
          relayAccountId,
        });
        settingsRef.current = data;
        setSettings(data);
        await loadAccounts();
        setNotice({
          type: "ok",
          message: enabled ? "路由模式已开启并写入 Codex 配置。" : "路由模式已关闭，已恢复之前的 Codex 配置。",
        });
      } catch (error) {
        setNotice({
          type: "error",
          message: enabled
            ? `开启路由模式失败：${localizeError(String(error))}`
            : `关闭路由模式失败：${localizeError(String(error))}`,
        });
      } finally {
        setSwitchingId(null);
      }
    },
    [loadAccounts, localizeError],
  );

  const onLaunchCurrentCodexConfig = useCallback(async () => {
    setSwitchingId("router-launch");
    try {
      if (isPreviewRuntime()) {
        setNotice({ type: "ok", message: copy.notices.switchedAndLaunching });
        return;
      }

      const result = await invoke<SwitchAccountResult>("launch_current_codex_config", {
        workspacePath: null,
        restartEditorsOnSwitch: settings.restartEditorsOnSwitch,
        restartEditorTargets: settings.restartEditorTargets,
      });
      await loadAccounts();

      let baseNotice: Notice = result.usedFallbackCli
        ? { type: "info", message: copy.notices.switchedAndLaunchByCli }
        : { type: "ok", message: copy.notices.switchedAndLaunching };

      if (settings.restartEditorsOnSwitch) {
        if (result.editorRestartError) {
          baseNotice = {
            type: "error",
            message: copy.notices.editorRestartFailed(
              baseNotice.message,
              localizeError(result.editorRestartError),
            ),
          };
        } else if (result.restartedEditorApps.length > 0) {
          const restartedLabels = result.restartedEditorApps
            .map((id) => copy.editorAppLabels[id] ?? id)
            .join(" / ");
          baseNotice = {
            ...baseNotice,
            message: copy.notices.editorsRestarted(baseNotice.message, restartedLabels),
          };
        } else {
          baseNotice = {
            ...baseNotice,
            message: copy.notices.noEditorRestarted(baseNotice.message),
          };
        }
      }

      setNotice(baseNotice);
    } catch (error) {
      setNotice({
        type: "error",
        message: copy.notices.switchFailed(localizeError(String(error))),
      });
    } finally {
      setSwitchingId(null);
    }
  }, [
    copy.editorAppLabels,
    copy.notices,
    loadAccounts,
    localizeError,
    settings.restartEditorsOnSwitch,
    settings.restartEditorTargets,
  ]);

  const onSmartSwitch = useCallback(async () => {
    if (switchingId) {
      return;
    }

    const target = pickBestSmartSwitchAccount(
      rankedAccounts,
      settings.smartSwitchIncludeApi,
    );
    if (!target) {
      setNotice({ type: "info", message: copy.notices.smartSwitchNoTarget });
      return;
    }
    if (target.isCurrent) {
      setNotice({
        type: "info",
        message: copy.notices.smartSwitchAlreadyBest,
      });
      return;
    }

    await onSwitch(target);
  }, [copy.notices, onSwitch, rankedAccounts, settings.smartSwitchIncludeApi, switchingId]);

  return {
    accounts: displayAccounts,
    tokenUsage,
    tokenUsageError,
    loading,
    refreshing,
    refreshingTokenUsage,
    addDialogOpen,
    importingAccounts,
    reauthorizeAccount,
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
    savingSettings,
    installedEditorApps,
    hasOpencodeDesktopApp,
    refreshUsage,
    refreshUsageForAccountKeys,
    refreshApiQuotaForAccountKeys,
    refreshAllApiQuota,
    refreshTokenUsage,
    checkForAppUpdate,
    installPendingUpdate,
    openManualDownloadPage,
    closeUpdateDialog,
    reloadSettings: loadSettings,
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
    onUpdateAccountTags,
    onImportAuthFiles,
    onExportAccounts,
    onRenameAccountLabel,
    onUpdateApiAccount,
    onProbeApiModels,
    onUpdateApiAccountKeys,
    onProbeApiAccountKey,
    onDelete,
    onSwitch,
    onSwitchHybrid,
    onSetModelRouterMode,
    onLaunchCurrentCodexConfig,
    onSmartSwitch,
    smartSwitching: switchingId !== null,
  };
}
