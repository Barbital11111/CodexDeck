import {
  CaretRightOutlined,
  DeleteOutlined,
  DownloadOutlined,
  EditOutlined,
  SyncOutlined,
  TagsOutlined,
} from "@ant-design/icons";
import { Button, Drawer, Input, Select, Switch, Tooltip } from "antd";
import { useMemo, useState } from "react";
import { useI18n } from "../i18n/I18nProvider";
import type {
  AccountSummary,
  ApiQuotaMode,
  NotificationProviderConfig,
  TrayUsageDisplayMode,
  UpdateApiAccountInput,
  UsageWindow,
} from "../types/app";
import {
  formatPlan,
  formatWindowLabel,
  percent,
  planTone,
  remainingPercent,
} from "../utils/usage";
import {
  displayAccountLabel,
  displayBalanceText,
  displayModelName,
  displayProviderName,
  displayRelayEndpoint,
} from "../utils/privacy";

type AccountCardProps = {
  accounts: AccountSummary[];
  exportingAccounts: boolean;
  switchingId: string | null;
  renamingAccountId: string | null;
  pendingDeleteId: string | null;
  notificationProviders: NotificationProviderConfig[];
  usageDisplayMode: TrayUsageDisplayMode;
  hideAccountDetails: boolean;
  onExport: (account: AccountSummary) => void;
  onReauthorize: (account: AccountSummary) => void;
  onRename: (account: AccountSummary, label: string) => Promise<boolean>;
  onUpdateApiAccount: (account: AccountSummary, input: UpdateApiAccountInput) => Promise<boolean>;
  onUpdateTags: (account: AccountSummary, value: string) => Promise<boolean>;
  onRefreshApiQuota: (account: AccountSummary) => void;
  onSwitch: (account: AccountSummary) => void;
  onDelete: (account: AccountSummary) => void;
};

type UsageDialProps = {
  accent: "hot" | "cool";
  centerLabel: string;
  label: string;
  resetTitle: string;
  resetValue: string;
  displayPercent: number | null | undefined;
};

function UsageDial({
  accent,
  centerLabel,
  label,
  resetTitle,
  resetValue,
  displayPercent,
}: UsageDialProps) {
  const radius = 29;
  const circumference = 2 * Math.PI * radius;
  const normalized =
    displayPercent === undefined || displayPercent === null || Number.isNaN(displayPercent)
      ? 0
      : Math.max(0, Math.min(100, displayPercent));
  const dashOffset = circumference * (1 - normalized / 100);

  return (
    <section className={`usageDial usageDial-${accent}`}>
      <strong className="usageDialLabel">{label}</strong>
      <div className="usageDialChart" aria-hidden="true">
        <svg className="usageDialSvg" viewBox="0 0 84 84">
          <circle className="usageDialTrack" cx="42" cy="42" r={radius} />
          <circle
            className="usageDialProgress"
            cx="42"
            cy="42"
            r={radius}
            style={{
              strokeDasharray: circumference,
              strokeDashoffset: dashOffset,
            }}
          />
        </svg>
        <div className="usageDialCenter">
          <strong>{percent(displayPercent)}</strong>
          <span>{centerLabel}</span>
        </div>
      </div>
      <div className="usageDialReset">
        <span>{resetTitle}</span>
        <strong>{resetValue}</strong>
      </div>
    </section>
  );
}

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

function normalizeApiBaseUrlForMatch(value: string | null | undefined) {
  const normalized = (value ?? "").trim().replace(/\/+$/, "").toLowerCase();
  return normalized.replace(/\/api\/v1$/i, "").replace(/\/v1$/i, "");
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

function resolveQuotaMode(
  account: AccountSummary,
  provider: NotificationProviderConfig | null,
): ApiQuotaMode {
  if (account.apiQuotaMode && account.apiQuotaMode !== "apiOnly") {
    return account.apiQuotaMode;
  }

  return hasProviderLogin(provider) ? "platformBasic" : "apiOnly";
}

function tagsToInput(tags: string[]) {
  return tags.join(", ");
}

function pickDefaultAccount(accounts: AccountSummary[]): AccountSummary | null {
  const current = accounts.find((account) => account.isCurrent);
  if (current) {
    return current;
  }
  return accounts[0] ?? null;
}

export function AccountCard({
  accounts,
  exportingAccounts,
  switchingId,
  renamingAccountId,
  pendingDeleteId,
  notificationProviders,
  usageDisplayMode,
  hideAccountDetails,
  onExport,
  onReauthorize,
  onRename,
  onUpdateApiAccount,
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
  const [draftApiBalanceEnabled, setDraftApiBalanceEnabled] = useState(false);
  const [draftApiQuotaMode, setDraftApiQuotaMode] = useState<ApiQuotaMode>("apiOnly");
  const [draftApiQuotaTodayUsedText, setDraftApiQuotaTodayUsedText] = useState("");
  const [draftApiQuotaRemainingText, setDraftApiQuotaRemainingText] = useState("");
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
  const usageCenterLabel =
    usageDisplayMode === "remaining" ? copy.accountCard.remaining : copy.accountCard.used;
  const displayUsagePercent = (window: UsageWindow | null) =>
    usageDisplayMode === "remaining" ? remainingPercent(window) : window?.usedPercent ?? null;
  const launchLabel = isSwitching ? copy.accountCard.launching : copy.accountCard.launch;
  const fiveHourReset = formatResetValue(fiveHour?.resetAt, locale);
  const oneWeekReset = formatResetValue(oneWeek?.resetAt, locale);
  const apiDailyWindow = selectedAccount.apiQuotaDailyWindow ?? null;
  const apiTotalWindow = selectedAccount.apiQuotaTotalWindow ?? null;
  const apiDailyReset = formatResetValue(apiDailyWindow?.resetAt, locale);
  const apiSubscriptionExpiresAt = formatResetValue(
    selectedAccount.apiQuotaSubscriptionExpiresAt ?? apiTotalWindow?.resetAt,
    locale,
  );
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
  const matchingNotificationProvider = findMatchingNotificationProvider(
    selectedAccount,
    notificationProviders,
  );
  const hasApiQuotaRefresh = isRelay && hasProviderLogin(matchingNotificationProvider);
  const rawApiQuotaMode = resolveQuotaMode(selectedAccount, matchingNotificationProvider);
  const resolvedApiQuotaMode =
    rawApiQuotaMode === "platformSubscription" && !hasApiSubscriptionUsage
      ? "platformBasic"
      : rawApiQuotaMode;
  const normalizedDraftLabel = draftLabel.trim();
  const normalizedDraftApiLabel = draftApiLabel.trim();
  const normalizedDraftApiBaseUrl = draftApiBaseUrl.trim();
  const normalizedDraftApiKey = draftApiKey.trim();
  const normalizedDraftApiModelName = draftApiModelName.trim();
  const normalizedDraftApiQuotaTodayUsedText = draftApiQuotaTodayUsedText.trim();
  const normalizedDraftApiQuotaRemainingText = draftApiQuotaRemainingText.trim();
  const normalizedDraftApiPlatformEmail = draftApiPlatformEmail.trim();
  const normalizedDraftApiPlatformPassword = draftApiPlatformPassword.trim();
  const canEditApiQuotaDisplay =
    draftApiBalanceEnabled && draftApiQuotaMode !== "apiOnly";
  const shouldShowAuthRefreshError = Boolean(
    selectedAccount.authRefreshError &&
      (!selectedAccount.usage || selectedAccount.usageError),
  );
  const footerErrors = [
    selectedAccount.profileIntegrityError,
    selectedAccount.profileLastValidationError,
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
    setDraftApiBalanceEnabled(Boolean(apiBalanceSource) || resolvedApiQuotaMode !== "apiOnly");
    setDraftApiQuotaMode(resolvedApiQuotaMode);
    setDraftApiQuotaTodayUsedText(selectedAccount.apiQuotaTodayUsedText ?? "");
    setDraftApiQuotaRemainingText(selectedAccount.apiQuotaRemainingText ?? "");
    setDraftApiPlatformEmail(matchingNotificationProvider?.email ?? "");
    setDraftApiPlatformPassword("");
    setIsEditingApi(true);
  };

  const handleCancelApiEdit = () => {
    setDraftApiLabel(selectedAccount.label);
    setDraftApiBaseUrl(selectedAccount.apiBaseUrl ?? "");
    setDraftApiKey("");
    setDraftApiModelName(selectedAccount.modelName ?? "");
    setDraftApiBalanceEnabled(Boolean(apiBalanceSource) || resolvedApiQuotaMode !== "apiOnly");
    setDraftApiQuotaMode(resolvedApiQuotaMode);
    setDraftApiQuotaTodayUsedText(selectedAccount.apiQuotaTodayUsedText ?? "");
    setDraftApiQuotaRemainingText(selectedAccount.apiQuotaRemainingText ?? "");
    setDraftApiPlatformEmail(matchingNotificationProvider?.email ?? "");
    setDraftApiPlatformPassword("");
    setIsEditingApi(false);
  };

  const handleCancelTagsEdit = () => {
    setDraftTags(tagsToInput(selectedAccount.tags));
    setIsEditingTags(false);
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
      draftApiBalanceEnabled === Boolean(apiBalanceSource) &&
      draftApiQuotaMode === resolvedApiQuotaMode &&
      normalizedDraftApiQuotaTodayUsedText ===
        (selectedAccount.apiQuotaTodayUsedText ?? "").trim() &&
      normalizedDraftApiQuotaRemainingText ===
        (selectedAccount.apiQuotaRemainingText ?? "").trim() &&
      normalizedDraftApiPlatformEmail === (matchingNotificationProvider?.email ?? "").trim() &&
      !normalizedDraftApiPlatformPassword &&
      !normalizedDraftApiKey;
    if (unchanged) {
      setIsEditingApi(false);
      return;
    }

    const updated = await onUpdateApiAccount(selectedAccount, {
      label: normalizedDraftApiLabel,
      baseUrl: normalizedDraftApiBaseUrl,
      apiKey: normalizedDraftApiKey ? normalizedDraftApiKey : null,
      modelName: normalizedDraftApiModelName,
      balanceDisplayEnabled: draftApiBalanceEnabled,
      apiQuotaMode: draftApiQuotaMode,
      apiQuotaTodayUsedText: canEditApiQuotaDisplay
        ? normalizedDraftApiQuotaTodayUsedText || null
        : null,
      apiQuotaRemainingText: canEditApiQuotaDisplay
        ? normalizedDraftApiQuotaRemainingText || null
        : null,
      platformLoginEmail: normalizedDraftApiPlatformEmail,
      platformLoginPassword:
        normalizedDraftApiPlatformPassword ||
        (normalizedDraftApiPlatformEmail ? matchingNotificationProvider?.password?.trim() : "") ||
        "",
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
        <div className="cardIdentity">
          <div className="cardBadges">
            {isRelay ? (
              <>
                <span className="cardBadge planBadge apiBadge">{copy.accountCard.apiBadge}</span>
                {selectedAccount.profileIntegrityError ? (
                  <span className="cardBadge stateBadge">{copy.accountCard.profileIncomplete}</span>
                ) : null}
                {selectedAccount.profileLastValidationError ? (
                  <span className="cardBadge stateBadge">{copy.accountCard.validationFailed}</span>
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

      {showUsage && !isRelay ? (
        <div className={`usageGrid ${isFreePlan ? "isFreePlan" : ""}`}>
          {!isFreePlan && (
            <UsageDial
              accent="hot"
              centerLabel={usageCenterLabel}
              label={formatWindowLabel(fiveHour, {
                fallback: copy.accountCard.fiveHourFallback,
                oneWeek: copy.accountCard.oneWeekLabel,
                hourSuffix: copy.accountCard.hourSuffix,
                minuteSuffix: copy.accountCard.minuteSuffix,
              })}
              resetTitle={copy.accountCard.resetAt}
              resetValue={fiveHourReset}
              displayPercent={displayUsagePercent(fiveHour)}
            />
          )}
          <UsageDial
            accent="cool"
            centerLabel={usageCenterLabel}
            label={formatWindowLabel(oneWeek, {
              fallback: copy.accountCard.oneWeekFallback,
              oneWeek: copy.accountCard.oneWeekLabel,
              hourSuffix: copy.accountCard.hourSuffix,
              minuteSuffix: copy.accountCard.minuteSuffix,
            })}
            resetTitle={copy.accountCard.resetAt}
            resetValue={oneWeekReset}
            displayPercent={displayUsagePercent(oneWeek)}
          />
        </div>
      ) : null}

      {isRelay ? (
        <div className="relayInfoPanel">
          {resolvedApiQuotaMode === "apiOnly" ? (
            <section className="apiQuotaPanel apiQuotaPanel-single" aria-label={copy.accountCard.apiQuotaTitle}>
              <span>{copy.accountCard.balanceLabel}</span>
              <strong>{apiBalanceText}</strong>
            </section>
          ) : resolvedApiQuotaMode === "platformBasic" ? (
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
          ) : resolvedApiQuotaMode === "platformSubscription" ? (
            <section className="apiQuotaPanel apiQuotaUsagePanel" aria-label={copy.accountCard.apiQuotaTitle}>
              <UsageDial
                accent="hot"
                centerLabel={usageCenterLabel}
                label={copy.accountCard.apiQuotaDailyLabel}
                resetTitle={copy.accountCard.resetAt}
                resetValue={apiDailyReset}
                displayPercent={displayUsagePercent(apiDailyWindow)}
              />
              <UsageDial
                accent="cool"
                centerLabel={usageCenterLabel}
                label={copy.accountCard.apiQuotaTotalLabel}
                resetTitle={copy.accountCard.apiQuotaExpiresAt}
                resetValue={apiSubscriptionExpiresAt}
                displayPercent={displayUsagePercent(apiTotalWindow)}
              />
            </section>
          ) : (
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
          )}
          {selectedAccount.providerName ? (
            <div className="relayInfoRow">
              <span>{copy.accountCard.providerLabel}</span>
              <strong>{displayProviderName(selectedAccount.providerName, hideAccountDetails)}</strong>
            </div>
          ) : null}
          <div className="relayInfoRow">
            <span>{copy.accountCard.endpointLabel}</span>
            <strong>{displayRelayEndpoint(selectedAccount.apiBaseUrl, hideAccountDetails)}</strong>
          </div>
          <div className="relayInfoRow">
            <span>{copy.accountCard.modelLabel}</span>
            <strong>{displayModelName(selectedAccount.modelName, hideAccountDetails)}</strong>
          </div>
        </div>
      ) : null}

      <div className="accountMetaPanel">
          {!isRelay && selectedAccount.providerName ? (
            <div className="accountMetaRow">
              <span className="accountMetaLabel">{copy.accountCard.providerLabel}</span>
              <strong className="accountMetaValue">{selectedAccount.providerName}</strong>
            </div>
          ) : null}
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
          width={520}
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
                  onChange={(event) => setDraftApiBaseUrl(event.target.value)}
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
                <Input
                  value={draftApiModelName}
                  disabled={isRenaming}
                  placeholder={copy.addAccount.apiModelPlaceholder}
                  onChange={(event) => setDraftApiModelName(event.target.value)}
                />
              </label>
            </section>

            <section className="accountApiDrawerSection">
              <div className="accountApiDrawerSwitchRow">
                <div className="accountApiDrawerSectionTitle">
                  <strong>{copy.addAccount.apiQuotaToggleLabel}</strong>
                  <span>{copy.accountCard.apiDrawerQuotaDescription}</span>
                </div>
                <Switch
                  checked={draftApiBalanceEnabled}
                  onChange={(checked) => {
                    setDraftApiBalanceEnabled(checked);
                    if (!checked) {
                      setDraftApiQuotaMode("apiOnly");
                      setDraftApiQuotaTodayUsedText("");
                      setDraftApiQuotaRemainingText("");
                      setDraftApiPlatformEmail("");
                      setDraftApiPlatformPassword("");
                    }
                  }}
                />
              </div>

              {draftApiBalanceEnabled ? (
                <>
                  <label className="accountApiDrawerField">
                    <span>{copy.accountCard.apiQuotaModeLabel}</span>
                    <Select
                      className="accountApiDrawerSelect"
                      value={draftApiQuotaMode}
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
                  {draftApiQuotaMode !== "apiOnly" ? (
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
