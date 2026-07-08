import {
  CaretRightOutlined,
  DeleteOutlined,
  DownloadOutlined,
  EditOutlined,
  SortAscendingOutlined,
  SyncOutlined,
  TagsOutlined,
} from "@ant-design/icons";
import { Button, Drawer, Input, Select, Switch, Tooltip } from "antd";
import { type ReactNode, useMemo, useState } from "react";
import { useI18n } from "../../i18n/I18nProvider";
import type {
  AccountSummary,
  ApiQuotaMode,
  NotificationProviderConfig,
  RelayModelCatalogEntry,
  TrayUsageDisplayMode,
  UpdateApiAccountInput,
  UsageWindow,
} from "../../types/app";
import {
  formatPlan,
  formatWindowLabel,
  planTone,
  remainingPercent,
} from "../../utils/usage";
import {
  displayAccountLabel,
  displayBalanceText,
  displayModelName,
} from "../../utils/privacy";
import {
  apiQuotaSubscriptionSelectOptions,
  normalizeApiQuotaSubscriptionName,
  resolveApiQuotaProviderCapability,
} from "../../utils/apiQuotaSubscriptions";
import {
  formatContextWindowInput,
  parseContextWindowInput,
} from "../../utils/modelContextWindow";
import {
  createModelCatalogRowId,
  createModelCatalogRowIds,
  moveArrayItem,
  moveRelayModelCatalogEntry,
  sortRelayModelCatalog,
} from "../../utils/modelCatalog";
import {
  SortableModelCatalogRow,
  SortableModelCatalogScope,
} from "../SortableModelCatalogRow";
import { QuotaMeter } from "../QuotaMeter";

type AccountCardProps = {
  accounts: AccountSummary[];
  exportingAccounts: boolean;
  switchingId: string | null;
  renamingAccountId: string | null;
  pendingDeleteId: string | null;
  notificationProviders: NotificationProviderConfig[];
  usageDisplayMode: TrayUsageDisplayMode;
  hideAccountDetails: boolean;
  sortHandle?: ReactNode;
  sortHandlePlacement?: "header" | "body";
  onExport: (account: AccountSummary) => void;
  onReauthorize: (account: AccountSummary) => void;
  onRename: (account: AccountSummary, label: string) => Promise<boolean>;
  onUpdateApiAccount: (account: AccountSummary, input: UpdateApiAccountInput) => Promise<boolean>;
  onProbeApiModels: (
    baseUrl: string,
    apiKey: string | null,
    accountKey?: string,
  ) => Promise<RelayModelCatalogEntry[]>;
  onUpdateTags: (account: AccountSummary, value: string) => Promise<boolean>;
  onRefreshApiQuota: (account: AccountSummary) => void;
  onSwitch: (account: AccountSummary) => void;
  onDelete: (account: AccountSummary) => void;
};

function formatResetValue(epochSec: number | null | undefined, locale?: string) {
  if (!epochSec) {
    return "--";
  }

  const value = new Date(epochSec * 1000);
  return value.toLocaleString(locale, {
    year: "numeric",
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function normalizeCreditBalance(value: string | null | undefined) {
  const normalized = value?.trim();
  if (!normalized) {
    return null;
  }

  if (/^\d+(?:\.\d+)?$/.test(normalized)) {
    return `$${Number(normalized).toFixed(2)}`;
  }

  return normalized;
}

function apiSubscriptionName(account: AccountSummary) {
  return account.apiQuotaSubscriptionName?.trim() || null;
}

function apiSubscriptionTone(value: string | null | undefined) {
  const normalized = value?.trim().toLowerCase() ?? "";
  if (!normalized) {
    return "unknown";
  }
  if (["free", "lite", "adagio"].includes(normalized)) {
    return "free";
  }
  if (["plus", "standard", "moderato"].includes(normalized)) {
    return "plus";
  }
  if (["pro", "allegretto"].includes(normalized)) {
    return "pro";
  }
  if (["max", "allegro"].includes(normalized)) {
    return "max";
  }
  if (["ultra", "vivace"].includes(normalized)) {
    return "ultra";
  }
  if (["team"].includes(normalized)) {
    return "team";
  }
  return "unknown";
}

function normalizeApiBaseUrlForMatch(value: string | null | undefined) {
  const normalized = (value ?? "").trim().replace(/\/+$/, "").toLowerCase();
  return normalized.replace(/\/api\/v1$/i, "").replace(/\/v1$/i, "");
}

function supportsProviderApiKeyQuota(baseUrl: string | null | undefined) {
  const normalized = normalizeApiBaseUrlForMatch(baseUrl);
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

function isMiniMaxApiBaseUrl(baseUrl: string | null | undefined) {
  const normalized = normalizeApiBaseUrlForMatch(baseUrl);
  return (
    normalized.includes("api.minimaxi.com") ||
    normalized.includes("minimaxi.com") ||
    normalized.includes("api.minimax.io") ||
    normalized.includes("minimax.io")
  );
}

function isMiniMaxApiAccount(account: AccountSummary) {
  const metadata = [
    account.providerId,
    account.providerName,
    account.label,
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();

  return isMiniMaxApiBaseUrl(account.apiBaseUrl) || metadata.includes("minimax");
}

function findMatchingNotificationProvider(
  account: AccountSummary,
  providers: NotificationProviderConfig[],
) {
  const accountBaseUrl = normalizeApiBaseUrlForMatch(account.apiBaseUrl);
  if (!accountBaseUrl) {
    return null;
  }

  return (
    providers.find((provider) => normalizeApiBaseUrlForMatch(provider.baseUrl) === accountBaseUrl) ??
    null
  );
}

function hasProviderLogin(provider: NotificationProviderConfig | null | undefined) {
  return Boolean(provider?.email.trim() && provider.password?.trim());
}

function isApiQuotaErrorMessage(message: string | null | undefined) {
  return Boolean(
    message &&
      (message.includes("NewAPI 额度接口") ||
        message.includes("连接 NewAPI 额度接口失败") ||
        message.includes("API 平台连接失败") ||
        message.includes("API 平台接口失败") ||
        message.includes("API 平台接口返回格式异常") ||
        message.includes("API 平台用户接口失败") ||
        message.includes("API 平台用量统计接口失败") ||
        message.includes("API 平台 URL 无效")),
  );
}

function isEndpointCapabilityNotice(message: string | null | undefined) {
  return Boolean(
    message &&
      (message.includes("接口能力已重置为仅 /v1/chat/completions") ||
        message.includes("已跳过接口探测，仅启用 /v1/chat/completions")),
  );
}

function resolveQuotaMode(
  account: AccountSummary,
  provider: NotificationProviderConfig | null,
): ApiQuotaMode {
  if (account.apiQuotaMode) {
    return account.apiQuotaMode;
  }

  return hasProviderLogin(provider) ? "platformBasic" : "apiOnly";
}

function tagsToInput(tags: string[]) {
  return tags.join(", ");
}

function firstEnabledModelName(entries: RelayModelCatalogEntry[]) {
  return (
    entries.find((entry) => entry.enabled !== false && entry.model.trim())?.model.trim() ??
    entries.find((entry) => entry.model.trim())?.model.trim() ??
    ""
  );
}

function resolvePreferredModelName(
  preferredModelName: string,
  entries: RelayModelCatalogEntry[],
) {
  const preferred = preferredModelName.trim();
  if (preferred && entries.some((entry) => entry.model.trim() === preferred)) {
    return preferred;
  }
  return firstEnabledModelName(entries);
}

function uniqueModelName(entries: RelayModelCatalogEntry[], fallbackModel: string) {
  const normalizedFallback = fallbackModel.trim() || "custom-model";
  const existing = new Set(
    entries.map((entry) => entry.model.trim()).filter((model) => model.length > 0),
  );
  if (!existing.has(normalizedFallback)) {
    return normalizedFallback;
  }

  let index = 2;
  let candidate = `${normalizedFallback}-${index}`;
  while (existing.has(candidate)) {
    index += 1;
    candidate = `${normalizedFallback}-${index}`;
  }
  return candidate;
}

function pickDefaultAccount(accounts: AccountSummary[]): AccountSummary | null {
  const current = accounts.find((account) => account.isCurrent);
  if (current) {
    return current;
  }
  return accounts[0] ?? null;
}

export function ClassicAccountCard({
  accounts,
  exportingAccounts,
  switchingId,
  renamingAccountId,
  pendingDeleteId,
  notificationProviders,
  usageDisplayMode,
  hideAccountDetails,
  sortHandle,
  sortHandlePlacement = "header",
  onExport,
  onReauthorize,
  onRename,
  onUpdateApiAccount,
  onProbeApiModels,
  onUpdateTags,
  onRefreshApiQuota,
  onSwitch,
  onDelete,
}: AccountCardProps) {
  const { copy, locale } = useI18n();
  const [preferredSelectedId, setPreferredSelectedId] = useState<string | null>(
    () => pickDefaultAccount(accounts)?.id ?? null,
  );
  const [isEditingAlias, setIsEditingAlias] = useState(false);
  const [draftLabel, setDraftLabel] = useState("");
  const [isEditingApi, setIsEditingApi] = useState(false);
  const [draftApiLabel, setDraftApiLabel] = useState("");
  const [draftApiBaseUrl, setDraftApiBaseUrl] = useState("");
  const [draftApiKey, setDraftApiKey] = useState("");
  const [draftApiModelName, setDraftApiModelName] = useState("");
  const [draftApiModelCatalog, setDraftApiModelCatalog] = useState<RelayModelCatalogEntry[]>([]);
  const [draftApiModelProbePending, setDraftApiModelProbePending] = useState(false);
  const [draftApiModelSortMode, setDraftApiModelSortMode] = useState(false);
  const [draftApiModelCatalogRowIds, setDraftApiModelCatalogRowIds] = useState<string[]>([]);
  const [draftApiBalanceEnabled, setDraftApiBalanceEnabled] = useState(false);
  const [draftApiQuotaMode, setDraftApiQuotaMode] = useState<ApiQuotaMode>("apiOnly");
  const [draftApiQuotaTodayUsedText, setDraftApiQuotaTodayUsedText] = useState("");
  const [draftApiQuotaRemainingText, setDraftApiQuotaRemainingText] = useState("");
  const [draftApiSubscriptionName, setDraftApiSubscriptionName] = useState("");
  const [draftApiPlatformEmail, setDraftApiPlatformEmail] = useState("");
  const [draftApiPlatformPassword, setDraftApiPlatformPassword] = useState("");
  const [isEditingTags, setIsEditingTags] = useState(false);
  const [draftTags, setDraftTags] = useState("");
  const [savingTags, setSavingTags] = useState(false);

  const selectedAccount = useMemo(
    () =>
      (switchingId && accounts.find((account) => account.id === switchingId)) ||
      (pendingDeleteId && accounts.find((account) => account.id === pendingDeleteId)) ||
      accounts.find((account) => account.isCurrent) ||
      (preferredSelectedId && accounts.find((account) => account.id === preferredSelectedId)) ||
      pickDefaultAccount(accounts),
    [accounts, pendingDeleteId, preferredSelectedId, switchingId],
  );

  if (!selectedAccount) {
    return null;
  }

  const usage = selectedAccount.usage;
  const isRelay = selectedAccount.sourceKind === "relay";
  const fiveHour = usage?.fiveHour ?? null;
  const oneWeek = usage?.oneWeek ?? null;
  const normalizedPlan = isRelay ? "api" : selectedAccount.planType || usage?.planType;
  const tone = planTone(normalizedPlan);
  const isSwitching = switchingId === selectedAccount.id;
  const isRenaming = renamingAccountId === selectedAccount.accountKey;
  const isDeletePending = pendingDeleteId === selectedAccount.id;
  const isFreePlan = tone === "free";
  const showUsage = usageDisplayMode !== "hidden";
  const displayUsagePercent = (window: UsageWindow | null) =>
    usageDisplayMode === "remaining" ? remainingPercent(window) : window?.usedPercent ?? null;
  const launchLabel = isSwitching ? copy.accountCard.launching : copy.accountCard.launch;
  const fiveHourReset = formatResetValue(fiveHour?.resetAt, locale);
  const oneWeekReset = formatResetValue(oneWeek?.resetAt, locale);
  const apiDailyWindow = selectedAccount.apiQuotaDailyWindow ?? null;
  const apiTotalWindow = selectedAccount.apiQuotaTotalWindow ?? null;
  const isMiniMaxSubscriptionAccount = isMiniMaxApiAccount(selectedAccount);
  const apiDailyLabel = isMiniMaxSubscriptionAccount ? "5h 限额" : copy.accountCard.apiQuotaDailyLabel;
  const apiTotalLabel = isMiniMaxSubscriptionAccount ? "周限额" : copy.accountCard.apiQuotaTotalLabel;
  const apiDailyReset = formatResetValue(apiDailyWindow?.resetAt, locale);
  const apiTotalResetAt = isMiniMaxSubscriptionAccount
    ? apiTotalWindow?.resetAt
    : selectedAccount.apiQuotaSubscriptionExpiresAt ?? apiTotalWindow?.resetAt;
  const apiTotalReset = formatResetValue(apiTotalResetAt, locale);
  const selectedAccountLabel = displayAccountLabel(selectedAccount, hideAccountDetails);
  const hasApiSubscriptionUsage = Boolean(apiDailyWindow || apiTotalWindow);
  const apiBalanceSource = selectedAccount.balanceText || usage?.credits?.balance;
  const apiBalanceText = usage?.credits?.unlimited
    ? copy.accountCard.unlimited
    : displayBalanceText(normalizeCreditBalance(apiBalanceSource), hideAccountDetails);
  const apiTodayUsedText = displayBalanceText(
    selectedAccount.apiQuotaTodayUsedText,
    hideAccountDetails,
  );
  const apiRemainingText = displayBalanceText(
    selectedAccount.apiQuotaRemainingText || apiBalanceSource,
    hideAccountDetails,
  );
  const apiTotalTokensText = displayBalanceText(
    selectedAccount.apiQuotaTotalTokensText,
    hideAccountDetails,
  );
  const apiTodayTokensText = displayBalanceText(
    selectedAccount.apiQuotaTodayTokensText,
    hideAccountDetails,
  );
  const selectedApiSubscriptionName = apiSubscriptionName(selectedAccount);
  const selectedApiSubscriptionTone = apiSubscriptionTone(selectedApiSubscriptionName);
  const matchingNotificationProvider = findMatchingNotificationProvider(
    selectedAccount,
    notificationProviders,
  );
  const profileLastValidationError =
    !selectedAccount.balanceDisplayEnabled &&
    isApiQuotaErrorMessage(selectedAccount.profileLastValidationError)
      ? null
      : selectedAccount.profileLastValidationError;
  const profileLastValidationNotice = isEndpointCapabilityNotice(profileLastValidationError)
    ? profileLastValidationError
    : null;
  const profileLastValidationErrorForFooter =
    profileLastValidationNotice ? null : profileLastValidationError;
  const hasApiQuotaRefresh =
    isRelay &&
    selectedAccount.balanceDisplayEnabled &&
    (hasProviderLogin(matchingNotificationProvider) ||
      selectedAccount.apiQuotaMode === "apiOnly" ||
      supportsProviderApiKeyQuota(selectedAccount.apiBaseUrl));
  const rawApiQuotaMode = resolveQuotaMode(selectedAccount, matchingNotificationProvider);
  const resolvedApiQuotaMode =
    rawApiQuotaMode === "platformSubscription" && !hasApiSubscriptionUsage
      ? "platformBasic"
      : rawApiQuotaMode;
  const shouldShowApiQuotaPanel = isRelay && selectedAccount.balanceDisplayEnabled;
  const normalizedDraftLabel = draftLabel.trim();
  const normalizedDraftApiLabel = draftApiLabel.trim();
  const normalizedDraftApiBaseUrl = draftApiBaseUrl.trim();
  const normalizedDraftApiKey = draftApiKey.trim();
  const normalizedDraftApiModelName = draftApiModelName.trim();
  const normalizedDraftApiQuotaTodayUsedText = draftApiQuotaTodayUsedText.trim();
  const normalizedDraftApiQuotaRemainingText = draftApiQuotaRemainingText.trim();
  const normalizedDraftApiSubscriptionName =
    normalizeApiQuotaSubscriptionName(draftApiSubscriptionName);
  const normalizedDraftApiPlatformEmail = draftApiPlatformEmail.trim();
  const normalizedDraftApiPlatformPassword = draftApiPlatformPassword.trim();
  const draftApiQuotaCapability = resolveApiQuotaProviderCapability(normalizedDraftApiBaseUrl);
  const draftApiQuotaSubscriptionLabelMode = draftApiQuotaCapability.subscriptionLabelMode;
  const draftApiBalancePresetLocked =
    draftApiQuotaCapability.balanceDisplayControl === "preset";
  const effectiveDraftApiBalanceEnabled = draftApiBalancePresetLocked
    ? draftApiQuotaCapability.balanceDisplayEnabled
    : draftApiBalanceEnabled;
  const effectiveDraftApiQuotaMode = draftApiBalancePresetLocked
    ? draftApiQuotaCapability.defaultQuotaMode
    : draftApiQuotaMode;
  const draftApiQuotaSubscriptionOptions = apiQuotaSubscriptionSelectOptions(
    draftApiQuotaSubscriptionLabelMode,
    normalizedDraftApiBaseUrl,
  );
  const draftPreferredModelOptions = (() => {
    const seen = new Set<string>();
    return draftApiModelCatalog
      .map((entry) => ({
        value: entry.model.trim(),
        label: entry.displayName?.trim()
          ? `${entry.displayName.trim()} (${entry.model.trim()})`
          : entry.model.trim(),
      }))
      .filter((option) => {
        if (!option.value || seen.has(option.value)) {
          return false;
        }
        seen.add(option.value);
        return true;
      });
  })();
  const canEditApiQuotaDisplay =
    effectiveDraftApiBalanceEnabled && effectiveDraftApiQuotaMode !== "apiOnly";
  const shouldShowAuthRefreshError = Boolean(
    selectedAccount.authRefreshError &&
      (!selectedAccount.usage || selectedAccount.usageError),
  );
  const footerErrors = [
    selectedAccount.profileIntegrityError,
    profileLastValidationErrorForFooter,
    shouldShowAuthRefreshError ? selectedAccount.authRefreshError : null,
    selectedAccount.usageError,
  ].filter((value, index, values): value is string => Boolean(value) && values.indexOf(value) === index);

  const handleLaunch = () => {
    if (isSwitching) return;
    onSwitch(selectedAccount);
  };

  const handleSelectAccount = (account: AccountSummary) => {
    setPreferredSelectedId(account.id);
  };

  const handleStartAliasEdit = () => {
    setDraftLabel(selectedAccount.label);
    setIsEditingAlias(true);
  };

  const handleCancelAliasEdit = () => {
    setDraftLabel(selectedAccount.label);
    setIsEditingAlias(false);
  };

  const handleStartTagsEdit = () => {
    setIsEditingApi(false);
    setDraftTags(tagsToInput(selectedAccount.tags));
    setIsEditingTags(true);
  };

  const handleStartApiEdit = () => {
    setIsEditingTags(false);
    setDraftApiLabel(selectedAccount.label);
    setDraftApiBaseUrl(selectedAccount.apiBaseUrl ?? "");
    setDraftApiKey("");
    setDraftApiModelName(selectedAccount.modelName ?? "");
    setDraftApiModelCatalog(selectedAccount.modelCatalog ?? []);
    setDraftApiModelCatalogRowIds(
      createModelCatalogRowIds(
        selectedAccount.modelCatalog?.length ?? 0,
        "classic-draft-api-model",
      ),
    );
    setDraftApiModelProbePending(false);
    setDraftApiModelSortMode(false);
    setDraftApiBalanceEnabled(selectedAccount.balanceDisplayEnabled);
    setDraftApiQuotaMode(resolvedApiQuotaMode);
    setDraftApiQuotaTodayUsedText(selectedAccount.apiQuotaTodayUsedText ?? "");
    setDraftApiQuotaRemainingText(selectedAccount.apiQuotaRemainingText ?? "");
    setDraftApiSubscriptionName(selectedAccount.apiQuotaSubscriptionName ?? "");
    setDraftApiPlatformEmail(
      resolvedApiQuotaMode === "apiOnly" ? "" : matchingNotificationProvider?.email ?? "",
    );
    setDraftApiPlatformPassword("");
    setIsEditingApi(true);
  };

  const handleCancelApiEdit = () => {
    setDraftApiLabel(selectedAccount.label);
    setDraftApiBaseUrl(selectedAccount.apiBaseUrl ?? "");
    setDraftApiKey("");
    setDraftApiModelName(selectedAccount.modelName ?? "");
    setDraftApiModelCatalog(selectedAccount.modelCatalog ?? []);
    setDraftApiModelCatalogRowIds(
      createModelCatalogRowIds(
        selectedAccount.modelCatalog?.length ?? 0,
        "classic-draft-api-model",
      ),
    );
    setDraftApiModelProbePending(false);
    setDraftApiModelSortMode(false);
    setDraftApiBalanceEnabled(selectedAccount.balanceDisplayEnabled);
    setDraftApiQuotaMode(resolvedApiQuotaMode);
    setDraftApiQuotaTodayUsedText(selectedAccount.apiQuotaTodayUsedText ?? "");
    setDraftApiQuotaRemainingText(selectedAccount.apiQuotaRemainingText ?? "");
    setDraftApiSubscriptionName(selectedAccount.apiQuotaSubscriptionName ?? "");
    setDraftApiPlatformEmail(
      resolvedApiQuotaMode === "apiOnly" ? "" : matchingNotificationProvider?.email ?? "",
    );
    setDraftApiPlatformPassword("");
    setIsEditingApi(false);
  };

  const handleCancelTagsEdit = () => {
    setDraftTags(tagsToInput(selectedAccount.tags));
    setIsEditingTags(false);
  };

  const updateDraftApiModelCatalogEntry = (
    index: number,
    updater: (entry: RelayModelCatalogEntry) => RelayModelCatalogEntry,
  ) => {
    const previousEntry = draftApiModelCatalog[index];
    const nextCatalog = draftApiModelCatalog.map((entry, entryIndex) =>
      entryIndex === index ? updater(entry) : entry,
    );
    const isUpdatingPreferred = previousEntry?.model.trim() === draftApiModelName.trim();
    const preferredCandidate =
      isUpdatingPreferred && nextCatalog[index]?.enabled === false
        ? ""
        : isUpdatingPreferred
          ? nextCatalog[index]?.model ?? ""
          : draftApiModelName;
    setDraftApiModelCatalog(nextCatalog);
    setDraftApiModelName(resolvePreferredModelName(preferredCandidate, nextCatalog));
  };

  const handleSetDraftApiPreferredModel = (index: number) => {
    const model = draftApiModelCatalog[index]?.model.trim() ?? "";
    if (!model) {
      return;
    }
    setDraftApiModelName(model);
    setDraftApiModelCatalog((current) =>
      current.map((entry, entryIndex) =>
        entryIndex === index ? { ...entry, enabled: true } : entry,
      ),
    );
  };

  const handleProbeDraftApiModels = async () => {
    if (draftApiModelProbePending || !normalizedDraftApiBaseUrl) {
      return;
    }

    setDraftApiModelProbePending(true);
    try {
      const nextCatalog = sortRelayModelCatalog(
        (await onProbeApiModels(
          normalizedDraftApiBaseUrl,
          normalizedDraftApiKey || null,
          selectedAccount.accountKey,
        )).map((entry) => ({
          ...entry,
          enabled: entry.enabled ?? true,
        })),
      );
      setDraftApiModelCatalog(nextCatalog);
      setDraftApiModelCatalogRowIds(
        createModelCatalogRowIds(nextCatalog.length, "classic-draft-api-model"),
      );
      const firstEnabled = nextCatalog.find((entry) => entry.enabled) ?? nextCatalog[0];
      setDraftApiModelName(firstEnabled?.model || "");
      setDraftApiModelSortMode(false);
    } finally {
      setDraftApiModelProbePending(false);
    }
  };

  const handleSortDraftApiModels = () => {
    setDraftApiModelSortMode((current) => !current);
  };

  const handleMoveDraftApiModel = (fromIndex: number, toIndex: number) => {
    const nextCatalog = moveRelayModelCatalogEntry(
      draftApiModelCatalog,
      fromIndex,
      toIndex,
    );
    setDraftApiModelCatalog(nextCatalog);
    setDraftApiModelCatalogRowIds((current) => moveArrayItem(current, fromIndex, toIndex));
    setDraftApiModelName(resolvePreferredModelName(draftApiModelName, nextCatalog));
  };

  const handleAddDraftApiModelRow = () => {
    const model = uniqueModelName(draftApiModelCatalog, normalizedDraftApiModelName || "custom-model");
    setDraftApiModelName(model);
    setDraftApiModelCatalog((current) => [
      ...current,
      {
        model,
        displayName: null,
        requestModel: null,
        contextWindow: null,
        enabled: true,
      },
    ]);
    setDraftApiModelCatalogRowIds((current) => [
      ...current,
      createModelCatalogRowId("classic-draft-api-model"),
    ]);
  };

  const handleRemoveDraftApiModelRow = (index: number) => {
    const nextCatalog = draftApiModelCatalog.filter((_, entryIndex) => entryIndex !== index);
    if (nextCatalog.length < 2) {
      setDraftApiModelSortMode(false);
    }
    setDraftApiModelCatalog(nextCatalog);
    setDraftApiModelCatalogRowIds((current) =>
      current.filter((_, entryIndex) => entryIndex !== index),
    );
    setDraftApiModelName(resolvePreferredModelName(draftApiModelName, nextCatalog));
  };

  const commitAliasEdit = async () => {
    if (!normalizedDraftLabel) {
      handleCancelAliasEdit();
      return;
    }

    if (normalizedDraftLabel === selectedAccount.label.trim()) {
      setIsEditingAlias(false);
      return;
    }

    const updated = await onRename(selectedAccount, normalizedDraftLabel);
    if (updated) {
      setIsEditingAlias(false);
    }
  };

  const commitApiEdit = async () => {
    if (!normalizedDraftApiLabel || !normalizedDraftApiBaseUrl || !normalizedDraftApiModelName) {
      return;
    }

    const unchanged =
      normalizedDraftApiLabel === selectedAccount.label.trim() &&
      normalizedDraftApiBaseUrl === (selectedAccount.apiBaseUrl ?? "").trim() &&
      normalizedDraftApiModelName === (selectedAccount.modelName ?? "").trim() &&
      JSON.stringify(draftApiModelCatalog) ===
        JSON.stringify(selectedAccount.modelCatalog ?? []) &&
      effectiveDraftApiBalanceEnabled === selectedAccount.balanceDisplayEnabled &&
      effectiveDraftApiQuotaMode === resolvedApiQuotaMode &&
      normalizedDraftApiQuotaTodayUsedText ===
        (selectedAccount.apiQuotaTodayUsedText ?? "").trim() &&
      normalizedDraftApiQuotaRemainingText ===
        (selectedAccount.apiQuotaRemainingText ?? "").trim() &&
      (normalizedDraftApiSubscriptionName ?? "") ===
        (selectedAccount.apiQuotaSubscriptionName ?? "").trim() &&
      normalizedDraftApiPlatformEmail === (matchingNotificationProvider?.email ?? "").trim() &&
      !normalizedDraftApiPlatformPassword &&
      !normalizedDraftApiKey;
    if (unchanged) {
      setIsEditingApi(false);
      return;
    }

    const platformLoginEmail =
      effectiveDraftApiQuotaMode === "apiOnly" ? "" : normalizedDraftApiPlatformEmail;
    const platformLoginPassword =
      effectiveDraftApiQuotaMode === "apiOnly"
        ? ""
        : normalizedDraftApiPlatformPassword ||
          (normalizedDraftApiPlatformEmail ? matchingNotificationProvider?.password?.trim() : "") ||
          "";

    const updated = await onUpdateApiAccount(selectedAccount, {
      label: normalizedDraftApiLabel,
      baseUrl: normalizedDraftApiBaseUrl,
      apiKey: normalizedDraftApiKey ? normalizedDraftApiKey : null,
      modelName: normalizedDraftApiModelName,
      modelCatalog: draftApiModelCatalog,
      balanceDisplayEnabled: effectiveDraftApiBalanceEnabled,
      apiQuotaMode: effectiveDraftApiQuotaMode,
      apiQuotaTodayUsedText: canEditApiQuotaDisplay
        ? normalizedDraftApiQuotaTodayUsedText || null
        : null,
      apiQuotaRemainingText: canEditApiQuotaDisplay
        ? normalizedDraftApiQuotaRemainingText || null
        : null,
      apiQuotaSubscriptionName:
        effectiveDraftApiBalanceEnabled && draftApiQuotaSubscriptionLabelMode !== "none"
          ? normalizedDraftApiSubscriptionName
          : null,
      platformLoginEmail,
      platformLoginPassword,
    });
    if (updated) {
      setDraftApiKey("");
      setDraftApiPlatformPassword("");
      setIsEditingApi(false);
    }
  };

  const commitTagsEdit = async () => {
    if (savingTags) {
      return;
    }
    setSavingTags(true);
    try {
      const updated = await onUpdateTags(selectedAccount, draftTags);
      if (updated) {
        setIsEditingTags(false);
      }
    } finally {
      setSavingTags(false);
    }
  };

  return (
    <article
      className={`accountCard tone-${tone} ${selectedAccount.isCurrent ? "isCurrent" : ""} ${
        isSwitching ? "isSwitching" : ""
      }`}
    >
      <header className="cardHeader">
        {sortHandle && sortHandlePlacement === "header" ? (
          <div className="cardSortHandleSlot">{sortHandle}</div>
        ) : null}
        <div className="cardIdentity">
          <div className="cardBadges">
            {isRelay ? (
              <>
                <span className="cardBadge planBadge apiBadge">{copy.accountCard.apiBadge}</span>
                {selectedApiSubscriptionName ? (
                  <span
                    className={`cardBadge subscriptionBadge subscriptionBadge-${selectedApiSubscriptionTone}`}
                  >
                    {selectedApiSubscriptionName}
                  </span>
                ) : null}
                {selectedAccount.profileIntegrityError ? (
                  <span className="cardBadge stateBadge">{copy.accountCard.profileIncomplete}</span>
                ) : null}
                {profileLastValidationError ? (
                  profileLastValidationNotice ? null : (
                    <span className="cardBadge stateBadge">{copy.accountCard.validationFailed}</span>
                  )
                ) : null}
                {selectedAccount.isCurrent ? (
                  <span className="planCurrentGlass" aria-hidden="true">
                    {copy.accountCard.currentStamp}
                  </span>
                ) : null}
              </>
            ) : (
              accounts.map((account) => {
              const variantPlan = formatPlan(
                account.planType || account.usage?.planType,
                copy.accountCard.planLabels,
              );
              const isSelected = account.id === selectedAccount.id;
              return (
                <button
                  key={account.id}
                  type="button"
                  className={`cardBadge planBadge planBadgeButton ${
                    isSelected ? "isSelected" : ""
                  } ${account.isCurrent ? "isCurrent" : ""}`}
                  onClick={() => handleSelectAccount(account)}
                  aria-pressed={isSelected}
                  title={
                    account.isCurrent
                      ? `${variantPlan} · ${copy.accountCard.currentStamp}`
                      : variantPlan
                  }
                >
                  {variantPlan}
                  {account.isCurrent && (
                    <span className="planCurrentGlass" aria-hidden="true">
                      {copy.accountCard.currentStamp}
                    </span>
                  )}
                </button>
              );
              })
            )}
          </div>
          {!isRelay && isEditingAlias ? (
            <div className="cardAliasEditor">
              <label className="visuallyHidden" htmlFor={`account-alias-${selectedAccount.id}`}>
                {copy.accountCard.aliasInputLabel}
              </label>
              <input
                id={`account-alias-${selectedAccount.id}`}
                value={draftLabel}
                maxLength={60}
                autoFocus
                disabled={isRenaming}
                onChange={(event) => setDraftLabel(event.target.value)}
                onBlur={() => {
                  void commitAliasEdit();
                }}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    handleCancelAliasEdit();
                  }
                  if (event.key === "Enter") {
                    event.preventDefault();
                    event.currentTarget.blur();
                  }
                }}
              />
            </div>
          ) : (
            <h3 className={selectedAccount.isCurrent ? "nameCurrent" : ""}>
              {selectedAccountLabel}
            </h3>
          )}
        </div>
      </header>
      {sortHandle && sortHandlePlacement === "body" ? (
        <div className="cardSortBarSlot">{sortHandle}</div>
      ) : null}

      {showUsage && !isRelay ? (
        <div className={`quotaStack ${isFreePlan ? "isFreePlan" : ""}`}>
          {!isFreePlan && (
            <QuotaMeter
              variant="bar"
              label={formatWindowLabel(fiveHour, {
                fallback: copy.accountCard.fiveHourFallback,
                oneWeek: copy.accountCard.oneWeekLabel,
                hourSuffix: copy.accountCard.hourSuffix,
                minuteSuffix: copy.accountCard.minuteSuffix,
              })}
              percent={displayUsagePercent(fiveHour)}
              caption={fiveHourReset}
            />
          )}
          <QuotaMeter
            variant="bar"
            label={formatWindowLabel(oneWeek, {
              fallback: copy.accountCard.oneWeekFallback,
              oneWeek: copy.accountCard.oneWeekLabel,
              hourSuffix: copy.accountCard.hourSuffix,
              minuteSuffix: copy.accountCard.minuteSuffix,
            })}
            percent={displayUsagePercent(oneWeek)}
            caption={oneWeekReset}
          />
        </div>
      ) : null}

      {isRelay ? (
        <div className="relayInfoPanel">
          {shouldShowApiQuotaPanel && resolvedApiQuotaMode === "apiOnly" ? (
            <section className="apiQuotaPanel apiQuotaPanel-single" aria-label={copy.accountCard.apiQuotaTitle}>
              <span>{copy.accountCard.balanceLabel}</span>
              <strong>{apiBalanceText}</strong>
            </section>
          ) : shouldShowApiQuotaPanel && resolvedApiQuotaMode === "platformBasic" ? (
            <section className="apiQuotaPanel" aria-label={copy.accountCard.apiQuotaTitle}>
              <div className="apiQuotaGrid">
                <div className="apiQuotaMetric">
                  <span>{copy.accountCard.apiQuotaTodayUsed}</span>
                  <strong>{apiTodayUsedText}</strong>
                </div>
                <div className="apiQuotaMetric">
                  <span>{copy.accountCard.apiQuotaRemaining}</span>
                  <strong>{apiRemainingText}</strong>
                </div>
              </div>
            </section>
          ) : shouldShowApiQuotaPanel && resolvedApiQuotaMode === "platformSubscription" ? (
            <section className="apiQuotaPanel apiQuotaUsagePanel" aria-label={copy.accountCard.apiQuotaTitle}>
              <QuotaMeter
                variant="bar"
                label={apiDailyLabel}
                percent={displayUsagePercent(apiDailyWindow)}
                caption={apiDailyReset}
              />
              <QuotaMeter
                variant="bar"
                label={apiTotalLabel}
                percent={displayUsagePercent(apiTotalWindow)}
                caption={apiTotalReset}
              />
            </section>
          ) : shouldShowApiQuotaPanel ? (
            <section className="apiQuotaPanel" aria-label={copy.accountCard.apiQuotaTitle}>
              <div className="apiQuotaGrid">
                <div className="apiQuotaMetric">
                  <span>{copy.accountCard.apiQuotaTotalTokens}</span>
                  <strong>{apiTotalTokensText}</strong>
                </div>
                <div className="apiQuotaMetric">
                  <span>{copy.accountCard.apiQuotaTodayTokens}</span>
                  <strong>{apiTodayTokensText}</strong>
                </div>
              </div>
            </section>
          ) : null}
          <div className="relayInfoRow">
            <span>{copy.accountCard.modelLabel}</span>
            <strong>{displayModelName(selectedAccount.modelName, hideAccountDetails)}</strong>
          </div>
        </div>
      ) : null}

      <div className="accountMetaPanel">
          <div className="accountMetaRow">
            <span className="accountMetaLabel">{copy.accountCard.tagsLabel}</span>
            {isEditingTags ? (
              <div className="accountTagEditor">
                <input
                  className="accountTagInput"
                  value={draftTags}
                  disabled={savingTags}
                  placeholder={copy.accountCard.tagsPlaceholder}
                  onChange={(event) => setDraftTags(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") {
                      event.preventDefault();
                      handleCancelTagsEdit();
                    }
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void commitTagsEdit();
                    }
                  }}
                />
                <div className="accountMetaActions">
                  <button
                    type="button"
                    className="ghost accountMetaInlineAction"
                    onClick={() => void commitTagsEdit()}
                    disabled={savingTags}
                  >
                    {copy.accountCard.saveTags}
                  </button>
                  <button
                    type="button"
                    className="ghost accountMetaInlineAction"
                    onClick={handleCancelTagsEdit}
                    disabled={savingTags}
                  >
                    {copy.accountCard.cancelTags}
                  </button>
                </div>
              </div>
            ) : (
              <div className="accountMetaValue accountTagSummary">
                {selectedAccount.tags.length > 0 ? (
                  <div className="accountTagList">
                    {selectedAccount.tags.map((tag) => (
                      <span className="accountTagChip" key={tag}>
                        {tag}
                      </span>
                    ))}
                  </div>
                ) : (
                  <span className="accountMetaEmpty">{copy.accountCard.tagsEmpty}</span>
                )}
                <button
                  type="button"
                  className="ghost accountMetaInlineAction"
                  onClick={handleStartTagsEdit}
                >
                  {copy.accountCard.editTags}
                </button>
              </div>
            )}
          </div>
      </div>

      <footer className="cardFooter">
        {footerErrors.map((message) => (
          <p key={message} className="errorText">
            {message}
          </p>
        ))}
        <div className="cardFooterActions" aria-label={copy.accountCard.actionsGroupLabel}>
          <Tooltip title={launchLabel}>
            <Button
              className="cardFooterAction"
              icon={isSwitching ? <SyncOutlined spin /> : <CaretRightOutlined />}
              onClick={handleLaunch}
              disabled={isSwitching}
              aria-label={launchLabel}
            />
          </Tooltip>
          <Tooltip title={copy.accountCard.editTags}>
            <Button
              className="cardFooterAction"
              icon={<TagsOutlined />}
              onClick={handleStartTagsEdit}
              aria-label={copy.accountCard.editTags}
              disabled={isEditingTags}
            />
          </Tooltip>
          <Tooltip title={isRelay ? copy.accountCard.editApi : copy.accountCard.editAlias}>
            <Button
              className="cardFooterAction"
              icon={<EditOutlined />}
              onClick={isRelay ? handleStartApiEdit : handleStartAliasEdit}
              disabled={isEditingAlias || isRenaming}
              aria-label={isRelay ? copy.accountCard.editApi : copy.accountCard.editAlias}
            />
          </Tooltip>
          {!isRelay ? (
            <Tooltip title={copy.accountCard.reauthorize}>
              <Button
                className="cardFooterAction"
                icon={<SyncOutlined />}
                onClick={() => onReauthorize(selectedAccount)}
                aria-label={copy.accountCard.reauthorize}
              />
            </Tooltip>
          ) : hasApiQuotaRefresh ? (
            <Tooltip title={copy.accountCard.refreshApiQuota}>
              <Button
                className="cardFooterAction"
                icon={<SyncOutlined spin={isRenaming} />}
                onClick={() => onRefreshApiQuota(selectedAccount)}
                disabled={isRenaming}
                aria-label={copy.accountCard.refreshApiQuota}
              />
            </Tooltip>
          ) : null}
          <Tooltip title={copy.addAccount.exportButton}>
            <Button
              className="cardFooterAction"
              icon={<DownloadOutlined />}
              onClick={() => onExport(selectedAccount)}
              disabled={exportingAccounts}
              aria-label={copy.addAccount.exportButton}
            />
          </Tooltip>
          <Tooltip title={isDeletePending ? copy.accountCard.deleteConfirm : copy.accountCard.delete}>
            <Button
              className={`cardFooterAction cardFooterActionDanger ${isDeletePending ? "isPending" : ""}`}
              icon={<DeleteOutlined />}
              onClick={() => onDelete(selectedAccount)}
              aria-label={isDeletePending ? copy.accountCard.deleteConfirm : copy.accountCard.delete}
            />
          </Tooltip>
        </div>
      </footer>
      {isRelay ? (
        <Drawer
          className="accountApiDrawer"
          title={copy.accountCard.apiDrawerTitle}
          placement="right"
          open={isEditingApi}
          size={520}
          onClose={handleCancelApiEdit}
          destroyOnHidden
          footer={
            <div className="accountApiDrawerFooter">
              <Button onClick={handleCancelApiEdit}>{copy.accountCard.cancelTags}</Button>
              <Button
                type="primary"
                loading={isRenaming}
                disabled={
                  !normalizedDraftApiLabel ||
                  !normalizedDraftApiBaseUrl ||
                  !normalizedDraftApiModelName
                }
                onClick={() => void commitApiEdit()}
              >
                {copy.accountCard.apiDrawerSave}
              </Button>
            </div>
          }
        >
          <div className="accountApiDrawerBody">
            <section className="accountApiDrawerSection">
              <div className="accountApiDrawerSectionTitle">
                <strong>{copy.accountCard.apiDrawerBasicTitle}</strong>
                <span>{copy.accountCard.apiDrawerBasicDescription}</span>
              </div>
              <label className="accountApiDrawerField">
                <span>{copy.addAccount.apiNameLabel}</span>
                <Input
                  value={draftApiLabel}
                  disabled={isRenaming}
                  onChange={(event) => setDraftApiLabel(event.target.value)}
                />
              </label>
              <label className="accountApiDrawerField">
                <span>{copy.addAccount.apiBaseUrlLabel}</span>
                <Input
                  value={draftApiBaseUrl}
                  disabled={isRenaming}
                  placeholder={copy.addAccount.apiBaseUrlPlaceholder}
                  onChange={(event) => {
                    const value = event.target.value;
                    const nextCapability = resolveApiQuotaProviderCapability(value);
                    setDraftApiBaseUrl(value);
                    if (nextCapability.balanceDisplayControl === "preset") {
                      setDraftApiBalanceEnabled(nextCapability.balanceDisplayEnabled);
                      setDraftApiQuotaMode(nextCapability.defaultQuotaMode);
                    }
                    if (nextCapability.subscriptionLabelMode === "none") {
                      setDraftApiSubscriptionName("");
                    }
                  }}
                />
              </label>
              <label className="accountApiDrawerField">
                <span>{copy.addAccount.apiKeyLabel}</span>
                <Input.Password
                  value={draftApiKey}
                  disabled={isRenaming}
                  placeholder={copy.accountCard.apiKeyKeepPlaceholder}
                  onChange={(event) => setDraftApiKey(event.target.value)}
                />
              </label>
              <label className="accountApiDrawerField">
                <span>{copy.addAccount.apiModelLabel}</span>
                <Select
                  className="accountApiDrawerSelect"
                  value={draftApiModelName}
                  disabled={isRenaming}
                  options={draftPreferredModelOptions}
                  placeholder={copy.addAccount.apiModelPlaceholder}
                  onChange={setDraftApiModelName}
                />
                <small className="accountApiDrawerDisplayHint">
                  从下方模型菜单中选择首选模型。
                </small>
              </label>
              <section className="accountApiDrawerModelPanel">
                <div className="accountApiDrawerModelHeader">
                  <div className="accountApiDrawerSectionTitle">
                    <strong>模型菜单</strong>
                    <span>轻量模式直接显示启用模型；路由启动会聚合多个供应商并使用请求模型映射。</span>
                  </div>
                </div>
                <div className="apiModelCatalogActions">
                  <Button
                    size="small"
                    onClick={() => void handleProbeDraftApiModels()}
                    loading={draftApiModelProbePending}
                    disabled={isRenaming || !normalizedDraftApiBaseUrl}
                  >
                    探测模型
                  </Button>
                  <Button
                    size="small"
                    icon={<SortAscendingOutlined />}
                    type={draftApiModelSortMode ? "primary" : "default"}
                    onClick={handleSortDraftApiModels}
                    disabled={isRenaming || draftApiModelCatalog.length < 2}
                    aria-pressed={draftApiModelSortMode}
                  >
                    排序
                  </Button>
                  <Button size="small" onClick={handleAddDraftApiModelRow} disabled={isRenaming}>
                    添加模型
                  </Button>
                </div>
                {draftApiModelCatalog.length > 0 ? (
                  <div className="apiModelCatalogTable">
                    <div
                      className={`apiModelCatalogColumnHeader ${
                        draftApiModelSortMode ? "isSorting" : ""
                      }`}
                      aria-hidden="true"
                    >
                      {draftApiModelSortMode ? <span /> : null}
                      <span>显示</span>
                      <span>菜单模型 ID</span>
                      <span>显示名称</span>
                      <span>路由请求模型</span>
                      <span>上下文</span>
                      <span />
                    </div>
                    <SortableModelCatalogScope
                      enabled={draftApiModelSortMode}
                      items={draftApiModelCatalogRowIds}
                      onMove={handleMoveDraftApiModel}
                    >
                      {draftApiModelCatalog.map((entry, index) => (
                        <SortableModelCatalogRow
                          id={
                            draftApiModelCatalogRowIds[index] ??
                            `classic-draft-api-model-${index}`
                          }
                          key={
                            draftApiModelCatalogRowIds[index] ??
                            `classic-draft-api-model-${index}`
                          }
                          sortingEnabled={draftApiModelSortMode}
                        >
                          {(sortHandle) => (
                            <div
                              className={`apiModelCatalogRow ${
                                draftApiModelSortMode ? "isSorting" : ""
                              }`}
                            >
                              {draftApiModelSortMode ? sortHandle : null}
                              <label className="apiModelCatalogCheck" title="设为首选模型">
                                <input
                                  type="radio"
                                  name="classic-draft-api-preferred-model"
                                  checked={entry.model.trim() === draftApiModelName.trim()}
                                  disabled={isRenaming}
                                  onChange={() => handleSetDraftApiPreferredModel(index)}
                                />
                              </label>
                              <label className="apiModelCatalogCheck">
                                <input
                                  type="checkbox"
                                  checked={entry.enabled}
                                  disabled={isRenaming}
                                  onChange={(event) =>
                                    updateDraftApiModelCatalogEntry(index, (current) => ({
                                      ...current,
                                      enabled: event.target.checked,
                                    }))
                                  }
                                />
                              </label>
                              <Input
                                className="accountApiDrawerModelInput"
                                value={entry.model}
                                disabled={isRenaming}
                                placeholder="菜单模型 ID"
                                onChange={(event) =>
                                  updateDraftApiModelCatalogEntry(index, (current) => ({
                                    ...current,
                                    model: event.target.value,
                                  }))
                                }
                              />
                              <Input
                                className="accountApiDrawerModelDisplayInput"
                                value={entry.displayName ?? ""}
                                disabled={isRenaming}
                                placeholder="显示名称"
                                onChange={(event) =>
                                  updateDraftApiModelCatalogEntry(index, (current) => ({
                                    ...current,
                                    displayName: event.target.value,
                                  }))
                                }
                              />
                              <Input
                                className="accountApiDrawerModelRequestInput"
                                value={entry.requestModel ?? ""}
                                disabled={isRenaming}
                                placeholder="路由模式可填"
                                onChange={(event) =>
                                  updateDraftApiModelCatalogEntry(index, (current) => ({
                                    ...current,
                                    requestModel: event.target.value,
                                  }))
                                }
                              />
                              <Input
                                className="accountApiDrawerModelContextInput"
                                value={formatContextWindowInput(entry.contextWindow)}
                                disabled={isRenaming}
                                placeholder="256K"
                                onChange={(event) =>
                                  updateDraftApiModelCatalogEntry(index, (current) => ({
                                    ...current,
                                    contextWindow: parseContextWindowInput(event.target.value),
                                  }))
                                }
                              />
                              <Button
                                className="accountApiDrawerModelRemoveButton"
                                size="small"
                                onClick={() => handleRemoveDraftApiModelRow(index)}
                                disabled={isRenaming}
                              >
                                移除
                              </Button>
                            </div>
                          )}
                        </SortableModelCatalogRow>
                      ))}
                    </SortableModelCatalogScope>
                  </div>
                ) : (
                  <p className="accountApiDrawerDisplayHint">未设置时会只显示默认模型。</p>
                )}
              </section>
            </section>

            <section className="accountApiDrawerSection">
              <div className="accountApiDrawerSwitchRow">
                <div className="accountApiDrawerSectionTitle">
                  <strong>{copy.addAccount.apiQuotaToggleLabel}</strong>
                  <span>{copy.accountCard.apiDrawerQuotaDescription}</span>
                </div>
                <Switch
                  checked={effectiveDraftApiBalanceEnabled}
                  disabled={draftApiBalancePresetLocked}
                  onChange={(checked) => {
                    setDraftApiBalanceEnabled(checked);
                    if (!checked) {
                      setDraftApiQuotaMode("apiOnly");
                      setDraftApiQuotaTodayUsedText("");
                      setDraftApiQuotaRemainingText("");
                      setDraftApiSubscriptionName("");
                      setDraftApiPlatformEmail("");
                      setDraftApiPlatformPassword("");
                    }
                  }}
                />
              </div>

              {effectiveDraftApiBalanceEnabled ? (
                <>
                  <label className="accountApiDrawerField">
                    <span>{copy.accountCard.apiQuotaModeLabel}</span>
                    <Select
                      className="accountApiDrawerSelect"
                      value={effectiveDraftApiQuotaMode}
                      disabled={draftApiBalancePresetLocked}
                      options={[
                        {
                          value: "apiOnly",
                          label: copy.accountCard.apiQuotaModeApiOnly,
                        },
                        {
                          value: "platformBasic",
                          label: copy.accountCard.apiQuotaModePlatformBasic,
                        },
                        {
                          value: "platformSubscription",
                          label: copy.accountCard.apiQuotaModePlatformSubscription,
                        },
                        {
                          value: "admin",
                          label: copy.accountCard.apiQuotaModeAdmin,
                        },
                      ]}
                      onChange={(value) => {
                        setDraftApiQuotaMode(value);
                        if (value === "apiOnly") {
                          setDraftApiQuotaTodayUsedText("");
                          setDraftApiQuotaRemainingText("");
                        }
                      }}
                    />
                  </label>
                  {draftApiQuotaSubscriptionLabelMode !== "none" ? (
                    <label className="accountApiDrawerField">
                      <span>套餐标签</span>
                      <Select
                        className="accountApiDrawerSelect"
                        value={draftApiSubscriptionName}
                        options={draftApiQuotaSubscriptionOptions}
                        onChange={(value) =>
                          setDraftApiSubscriptionName(
                            normalizeApiQuotaSubscriptionName(value) ?? "",
                          )
                        }
                      />
                    </label>
                  ) : null}
                  {effectiveDraftApiQuotaMode !== "apiOnly" ? (
                    <div className="accountApiDrawerGrid">
                      <label className="accountApiDrawerField">
                        <span>{copy.addAccount.apiPlatformEmailLabel}</span>
                        <Input
                          value={draftApiPlatformEmail}
                          disabled={isRenaming}
                          placeholder={copy.addAccount.apiPlatformEmailPlaceholder}
                          onChange={(event) => setDraftApiPlatformEmail(event.target.value)}
                        />
                      </label>
                      <label className="accountApiDrawerField">
                        <span>{copy.addAccount.apiPlatformPasswordLabel}</span>
                        <Input.Password
                          value={draftApiPlatformPassword}
                          disabled={isRenaming}
                          placeholder={
                            matchingNotificationProvider
                              ? copy.accountCard.apiPasswordKeepPlaceholder
                              : copy.addAccount.apiPlatformPasswordPlaceholder
                          }
                          onChange={(event) => setDraftApiPlatformPassword(event.target.value)}
                        />
                      </label>
                    </div>
                  ) : null}
                  <div className="accountApiDrawerDisplayBox">
                    <div className="accountApiDrawerSectionTitle">
                      <strong>{copy.accountCard.apiQuotaDisplayTitle}</strong>
                      <span>{copy.accountCard.apiQuotaDisplayDescription}</span>
                    </div>
                    <div className="accountApiDrawerGrid">
                      <label className="accountApiDrawerField">
                        <span>{copy.accountCard.apiQuotaTodayUsed}</span>
                        <Input
                          value={draftApiQuotaTodayUsedText}
                          disabled={isRenaming || !canEditApiQuotaDisplay}
                          placeholder={copy.accountCard.apiQuotaDisplayAutoPlaceholder}
                          onChange={(event) =>
                            setDraftApiQuotaTodayUsedText(event.target.value)
                          }
                        />
                      </label>
                      <label className="accountApiDrawerField">
                        <span>{copy.accountCard.apiQuotaRemaining}</span>
                        <Input
                          value={draftApiQuotaRemainingText}
                          disabled={isRenaming || !canEditApiQuotaDisplay}
                          placeholder={copy.accountCard.apiQuotaDisplayAutoPlaceholder}
                          onChange={(event) =>
                            setDraftApiQuotaRemainingText(event.target.value)
                          }
                        />
                      </label>
                    </div>
                    {!canEditApiQuotaDisplay ? (
                      <p className="accountApiDrawerDisplayHint">
                        {copy.accountCard.apiQuotaDisplayLockedHint}
                      </p>
                    ) : null}
                  </div>
                </>
              ) : null}
            </section>
          </div>
        </Drawer>
      ) : null}
    </article>
  );
}
