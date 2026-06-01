import {
  DeleteOutlined,
  EditOutlined,
  ExportOutlined,
  PlusOutlined,
  SortAscendingOutlined,
  SyncOutlined,
} from "@ant-design/icons";
import { Button, Modal, Tooltip } from "antd";
import { useMemo, useState } from "react";
import { useI18n } from "../i18n/I18nProvider";
import type {
  AccountPoolConfig,
  AccountSummary,
  NotificationProviderConfig,
  TrayUsageDisplayMode,
  UpdateApiAccountInput,
  UsageWindow,
} from "../types/app";
import {
  compareAccountsByRemaining,
  compareAccountsForDisplay,
} from "../utils/accountRanking";
import { formatPlan, percent, remainingPercent, toProgressWidth } from "../utils/usage";
import {
  displayAccountLabel,
  displayModelName,
  displayRelayEndpoint,
} from "../utils/privacy";
import { AccountCard } from "./AccountCard";
import { AccountsGrid } from "./AccountsGrid";

type LogicalAccountEntry = {
  accountKey: string;
  variants: AccountSummary[];
  primary: AccountSummary;
  label: string;
  sourceKind: AccountSummary["sourceKind"];
  planLabel: string;
  isCurrent: boolean;
  hasIssue: boolean;
};

type AccountPoolManagerProps = {
  accounts: AccountSummary[];
  ungroupedAccounts: AccountSummary[];
  loading: boolean;
  accountPools: AccountPoolConfig[];
  saving: boolean;
  exportingAccounts: boolean;
  switchingId: string | null;
  renamingAccountId: string | null;
  pendingDeleteId: string | null;
  notificationProviders: NotificationProviderConfig[];
  usageDisplayMode: TrayUsageDisplayMode;
  hideAccountDetails: boolean;
  apiEnhancedLaunchEnabled: boolean;
  onRenamePool: (poolId: string, name: string) => void;
  onDeletePool: (poolId: string) => void;
  onTogglePoolCollapsed: (poolId: string, collapsed: boolean) => void;
  onReorderPool: (poolId: string, accountKeys: string[]) => void;
  onRefreshPoolUsage: (accountKeys: string[], apiAccountKeys: string[], label: string) => void;
  onAssignAccountToPool: (accountKey: string, poolId: string) => void;
  onRemoveAccountFromAllPools: (accountKey: string) => void;
  onExportAccountKeys: (accountKeys: string[]) => void;
  onExport: (account: AccountSummary) => void;
  onReauthorize: (account: AccountSummary) => void;
  onRename: (account: AccountSummary, label: string) => Promise<boolean>;
  onUpdateApiAccount: (account: AccountSummary, input: UpdateApiAccountInput) => Promise<boolean>;
  onUpdateTags: (account: AccountSummary, value: string) => Promise<boolean>;
  onRefreshApiQuota: (account: AccountSummary) => void;
  onSwitch: (account: AccountSummary) => void;
  onDelete: (account: AccountSummary) => void;
};

const PLAN_PRIORITY: Record<string, number> = {
  api: 0,
  team: 0,
  enterprise: 1,
  business: 2,
  pro: 3,
  plus: 4,
  free: 5,
  unknown: 6,
};

function planPriority(planType: string | null | undefined): number {
  const normalized = planType?.trim().toLowerCase() ?? "";
  return PLAN_PRIORITY[normalized] ?? PLAN_PRIORITY.unknown;
}

function sortVariantsForGroup(left: AccountSummary, right: AccountSummary): number {
  const priorityDiff =
    planPriority(left.planType ?? left.usage?.planType) -
    planPriority(right.planType ?? right.usage?.planType);
  if (priorityDiff !== 0) {
    return priorityDiff;
  }

  if (left.isCurrent !== right.isCurrent) {
    return left.isCurrent ? -1 : 1;
  }

  return compareAccountsByRemaining(left, right);
}

function normalizeProviderBaseUrl(value: string | null | undefined) {
  return (value ?? "")
    .trim()
    .replace(/\/+$/, "")
    .toLowerCase()
    .replace(/\/api\/v1$/i, "")
    .replace(/\/v1$/i, "");
}

function hasApiQuotaProvider(account: AccountSummary, providers: NotificationProviderConfig[]) {
  if (account.sourceKind !== "relay" || !account.balanceDisplayEnabled) {
    return false;
  }
  if (account.apiQuotaMode === "apiOnly" && Boolean(account.balanceText)) {
    return true;
  }
  const accountBaseUrl = normalizeProviderBaseUrl(account.apiBaseUrl);
  if (!accountBaseUrl) {
    return false;
  }

  return providers.some(
    (provider) =>
      normalizeProviderBaseUrl(provider.baseUrl) === accountBaseUrl &&
      Boolean(provider.email.trim()) &&
      Boolean(provider.password?.trim()),
  );
}

function formatResetValue(epochSec: number | null | undefined, locale?: string) {
  if (!epochSec) {
    return "--";
  }

  return new Date(epochSec * 1000).toLocaleString(locale, {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function buildLogicalAccountMap(
  copy: ReturnType<typeof useI18n>["copy"],
  accounts: AccountSummary[],
) {
  const groups = new Map<string, AccountSummary[]>();
  for (const account of accounts) {
    const existing = groups.get(account.accountKey);
    if (existing) {
      existing.push(account);
    } else {
      groups.set(account.accountKey, [account]);
    }
  }

  return new Map<string, LogicalAccountEntry>(
    Array.from(groups.entries()).map(([accountKey, variants]) => {
      const sortedVariants = [...variants].sort(sortVariantsForGroup);
      const primary = sortedVariants.find((item) => item.isCurrent) ?? sortedVariants[0];
      const resolvedPlan = formatPlan(
        primary.planType || primary.usage?.planType,
        copy.accountCard.planLabels,
      );

      return [
        accountKey,
        {
          accountKey,
          variants: sortedVariants,
          primary,
          label: primary.label,
          sourceKind: primary.sourceKind,
          planLabel: primary.sourceKind === "relay" ? copy.accountCard.apiBadge : resolvedPlan,
          isCurrent: sortedVariants.some((item) => item.isCurrent),
          hasIssue: sortedVariants.some(
            (item) =>
              Boolean(item.profileIntegrityError) ||
              Boolean(item.profileLastValidationError) ||
              Boolean(item.authRefreshBlocked),
          ),
        },
      ];
    }),
  );
}

function compareLogicalEntriesForDisplay(left: LogicalAccountEntry, right: LogicalAccountEntry) {
  const primaryDiff = compareAccountsForDisplay(left.primary, right.primary);
  if (primaryDiff !== 0) {
    return primaryDiff;
  }

  const labelDiff = left.label.localeCompare(right.label, "zh-Hans-CN");
  if (labelDiff !== 0) {
    return labelDiff;
  }

  return left.accountKey.localeCompare(right.accountKey, "zh-Hans-CN");
}

function CompactUsageBar({
  label,
  window,
  tone,
  locale,
}: {
  label: string;
  window: UsageWindow | null;
  tone: "hot" | "cool";
  locale: string;
}) {
  const value = remainingPercent(window);
  return (
    <div className={`accountGroupCompactQuota accountGroupCompactQuota-${tone}`}>
      <div className="accountGroupCompactQuotaTop">
        <span>{label}</span>
        <strong>{percent(value)}</strong>
      </div>
      <div className="accountGroupCompactQuotaTrack" aria-hidden="true">
        <span style={{ width: toProgressWidth(value) }} />
      </div>
      <small>{formatResetValue(window?.resetAt, locale)}</small>
    </div>
  );
}

function fiveHourResetAt(entry: LogicalAccountEntry): number | null {
  const resetAt = entry.primary.usage?.fiveHour?.resetAt;
  return typeof resetAt === "number" && Number.isFinite(resetAt) ? resetAt : null;
}

function compareLogicalEntriesByFiveHourReset(
  left: LogicalAccountEntry,
  right: LogicalAccountEntry,
) {
  const leftResetAt = fiveHourResetAt(left);
  const rightResetAt = fiveHourResetAt(right);

  if (leftResetAt !== null && rightResetAt !== null && leftResetAt !== rightResetAt) {
    return leftResetAt - rightResetAt;
  }

  if (leftResetAt === null && rightResetAt !== null) {
    return 1;
  }

  if (leftResetAt !== null && rightResetAt === null) {
    return -1;
  }

  return compareLogicalEntriesForDisplay(left, right);
}

export function AccountPoolManager({
  accounts,
  ungroupedAccounts,
  loading,
  accountPools,
  saving,
  exportingAccounts,
  switchingId,
  renamingAccountId,
  pendingDeleteId,
  notificationProviders,
  usageDisplayMode,
  hideAccountDetails,
  apiEnhancedLaunchEnabled,
  onRenamePool,
  onDeletePool,
  onTogglePoolCollapsed,
  onReorderPool,
  onRefreshPoolUsage,
  onAssignAccountToPool,
  onRemoveAccountFromAllPools,
  onExportAccountKeys,
  onExport,
  onReauthorize,
  onRename,
  onUpdateApiAccount,
  onUpdateTags,
  onRefreshApiQuota,
  onSwitch,
  onDelete,
}: AccountPoolManagerProps) {
  const { copy, locale } = useI18n();
  const groupCopy = copy.accountPools;
  const [editingPoolId, setEditingPoolId] = useState<string | null>(null);
  const [draftPoolName, setDraftPoolName] = useState("");
  const [collapsedOverrides, setCollapsedOverrides] = useState<Record<string, boolean>>({});
  const [addingToPoolId, setAddingToPoolId] = useState<string | null>(null);
  const [focusedExpandedPoolId, setFocusedExpandedPoolId] = useState<string | null>(null);
  const [deletePoolCandidate, setDeletePoolCandidate] = useState<
    (AccountPoolConfig & { entries: LogicalAccountEntry[] }) | null
  >(null);

  const logicalAccountMap = useMemo(
    () => buildLogicalAccountMap(copy, accounts),
    [accounts, copy],
  );

  const ungroupedLogicalEntries = useMemo(
    () =>
      Array.from(buildLogicalAccountMap(copy, ungroupedAccounts).values()).sort(
        compareLogicalEntriesForDisplay,
      ),
    [copy, ungroupedAccounts],
  );

  const ungroupedAccountsForDisplay = useMemo(
    () => [...ungroupedAccounts].sort(compareAccountsForDisplay),
    [ungroupedAccounts],
  );

  const groupSummaries = useMemo(
    () =>
      accountPools.map((pool) => ({
        ...pool,
        entries: pool.accountKeys
          .map((accountKey) => logicalAccountMap.get(accountKey))
          .filter((entry): entry is LogicalAccountEntry => Boolean(entry)),
      })),
    [accountPools, logicalAccountMap],
  );

  const resolveCollapsed = (pool: AccountPoolConfig): boolean =>
    collapsedOverrides[pool.id] ?? pool.collapsed;

  const orderedGroupSummaries = useMemo(() => {
    const indexedGroups = groupSummaries.map((pool, index) => ({
      pool,
      index,
      collapsed: collapsedOverrides[pool.id] ?? pool.collapsed,
    }));

    indexedGroups.sort((left, right) => {
      const leftIsFocused = focusedExpandedPoolId === left.pool.id && !left.collapsed;
      const rightIsFocused = focusedExpandedPoolId === right.pool.id && !right.collapsed;
      if (leftIsFocused !== rightIsFocused) {
        return leftIsFocused ? -1 : 1;
      }

      if (left.collapsed !== right.collapsed) {
        return left.collapsed ? 1 : -1;
      }

      return left.index - right.index;
    });

    return indexedGroups.map(({ pool }) => pool);
  }, [collapsedOverrides, focusedExpandedPoolId, groupSummaries]);

  const togglePoolCollapsed = (pool: AccountPoolConfig) => {
    const nextCollapsed = !resolveCollapsed(pool);
    setCollapsedOverrides((current) => ({ ...current, [pool.id]: nextCollapsed }));
    if (nextCollapsed && addingToPoolId === pool.id) {
      setAddingToPoolId(null);
    }
    if (nextCollapsed) {
      setFocusedExpandedPoolId((current) => (current === pool.id ? null : current));
    } else {
      setFocusedExpandedPoolId(pool.id);
    }
    onTogglePoolCollapsed(pool.id, nextCollapsed);
  };

  const toggleAddPanel = (pool: AccountPoolConfig) => {
    const collapsed = resolveCollapsed(pool);
    if (collapsed) {
      setCollapsedOverrides((current) => ({ ...current, [pool.id]: false }));
      setFocusedExpandedPoolId(pool.id);
      onTogglePoolCollapsed(pool.id, false);
    }

    if (!collapsed) {
      setFocusedExpandedPoolId(pool.id);
    }
    setAddingToPoolId((current) => (current === pool.id ? null : pool.id));
  };

  const startRename = (pool: AccountPoolConfig) => {
    setEditingPoolId(pool.id);
    setDraftPoolName(pool.name);
  };

  const finishRename = (pool: AccountPoolConfig) => {
    const normalized = draftPoolName.trim();
    if (normalized && normalized !== pool.name) {
      onRenamePool(pool.id, normalized);
    }
    setEditingPoolId((current) => (current === pool.id ? null : current));
    setDraftPoolName("");
  };

  const reorderPoolByFiveHourReset = (pool: AccountPoolConfig & { entries: LogicalAccountEntry[] }) => {
    const nextAccountKeys = [...pool.entries]
      .sort(compareLogicalEntriesByFiveHourReset)
      .map((entry) => entry.accountKey);
    onReorderPool(pool.id, nextAccountKeys);
  };

  const refreshPoolUsage = (pool: AccountPoolConfig & { entries: LogicalAccountEntry[] }) => {
    const nativeAccountKeys = pool.entries
      .filter((entry) => entry.sourceKind !== "relay")
      .map((entry) => entry.accountKey);
    const apiAccountKeys = pool.entries
      .filter((entry) => hasApiQuotaProvider(entry.primary, notificationProviders))
      .map((entry) => entry.accountKey);
    onRefreshPoolUsage(nativeAccountKeys, apiAccountKeys, pool.name || groupCopy.groupUntitled);
  };

  const exportPoolAccounts = (pool: AccountPoolConfig & { entries: LogicalAccountEntry[] }) => {
    if (pool.entries.length === 0) {
      return;
    }
    onExportAccountKeys(pool.entries.map((entry) => entry.accountKey));
  };

  const renderCardEntryActions = (entry: LogicalAccountEntry) => (
    <Tooltip title={groupCopy.removeSingle}>
      <Button
        type="default"
        size="small"
        className="accountGroupCardRemoveButton"
        onClick={() => onRemoveAccountFromAllPools(entry.accountKey)}
        disabled={saving}
      >
        {groupCopy.removeSingle}
      </Button>
    </Tooltip>
  );

  const renderCollapsedEntry = (entry: LogicalAccountEntry) => {
    const displayLabel = displayAccountLabel(entry.primary, hideAccountDetails);

    return (
      <div className="accountGroupCompactEntry" key={entry.accountKey}>
        <div className="accountGroupCompactIdentity">
          <div className="accountGroupCompactHeader">
            <strong title={displayLabel}>{displayLabel}</strong>
            {entry.isCurrent ? (
              <mark className="accountGroupCurrentGlass">{copy.accountCard.currentStamp}</mark>
            ) : null}
          </div>
          <div className="accountGroupCompactBadges">
            <span>{entry.planLabel}</span>
            {entry.hasIssue ? <em>{groupCopy.accountIncomplete}</em> : null}
          </div>
        </div>

        {entry.sourceKind === "relay" ? (
          <div className="accountGroupCompactRelay">
            <span>{displayRelayEndpoint(entry.primary.apiBaseUrl, hideAccountDetails)}</span>
            <strong>{displayModelName(entry.primary.modelName, hideAccountDetails)}</strong>
          </div>
        ) : (
          <div className="accountGroupCompactQuotaGrid">
            <CompactUsageBar
              label={copy.accountCard.fiveHourFallback}
              window={entry.primary.usage?.fiveHour ?? null}
              tone="hot"
              locale={locale}
            />
            <CompactUsageBar
              label={copy.accountCard.oneWeekLabel}
              window={entry.primary.usage?.oneWeek ?? null}
              tone="cool"
              locale={locale}
            />
          </div>
        )}
      </div>
    );
  };

  return (
    <section className="accountGroupsWorkspace">
      {groupSummaries.length > 0 ? (
        <div className="accountGroupsGrid">
          {orderedGroupSummaries.map((pool) => {
            const collapsed = resolveCollapsed(pool);
            const isAdding = addingToPoolId === pool.id;
            const showCards = !collapsed;

            return (
              <article className={`accountGroupCard${collapsed ? "" : " isExpanded"}`} key={pool.id}>
                <header className="accountGroupCardHeader">
                  <button
                    type="button"
                    className="ghost accountGroupToggle"
                    onClick={() => togglePoolCollapsed(pool)}
                    aria-label={collapsed ? groupCopy.expand : groupCopy.collapse}
                  >
                    <svg viewBox="0 0 16 16" aria-hidden="true">
                      {collapsed ? <path d="M6 4l4 4-4 4" /> : <path d="M4 6l4 4 4-4" />}
                    </svg>
                  </button>

                  <div className="accountGroupIdentity">
                    {editingPoolId === pool.id ? (
                      <input
                        className="accountGroupNameInput"
                        value={draftPoolName}
                        autoFocus
                        placeholder={groupCopy.renamePlaceholder}
                        disabled={saving}
                        onChange={(event) => setDraftPoolName(event.target.value)}
                        onBlur={() => finishRename(pool)}
                        onKeyDown={(event) => {
                          if (event.key === "Escape") {
                            setEditingPoolId(null);
                            setDraftPoolName("");
                          }
                          if (event.key === "Enter") {
                            event.preventDefault();
                            finishRename(pool);
                          }
                        }}
                      />
                    ) : (
                      <>
                        <strong>{pool.name || groupCopy.groupUntitled}</strong>
                        <span>
                          {pool.entries.length} {groupCopy.groupCountLabel}
                        </span>
                      </>
                    )}
                  </div>

                  <div className="accountGroupActions">
                    <Tooltip title={groupCopy.addAccount}>
                      <Button
                        type="text"
                        className="accountGroupIconButton"
                        icon={<PlusOutlined />}
                        onClick={() => toggleAddPanel(pool)}
                        disabled={saving}
                        aria-label={groupCopy.addAccount}
                      />
                    </Tooltip>
                    <Tooltip title={groupCopy.refreshQuota}>
                      <Button
                        type="text"
                        className="accountGroupIconButton"
                        icon={<SyncOutlined />}
                        onClick={() => refreshPoolUsage(pool)}
                        disabled={
                          saving ||
                          !pool.entries.some(
                            (entry) =>
                              entry.sourceKind !== "relay" ||
                              hasApiQuotaProvider(entry.primary, notificationProviders),
                          )
                        }
                        aria-label={groupCopy.refreshQuota}
                      />
                    </Tooltip>
                    <Tooltip title={groupCopy.reorder}>
                      <Button
                        type="text"
                        className="accountGroupIconButton"
                        icon={<SortAscendingOutlined />}
                        onClick={() => reorderPoolByFiveHourReset(pool)}
                        disabled={saving || pool.entries.length < 2}
                        aria-label={groupCopy.reorder}
                      />
                    </Tooltip>
                    <Tooltip title={groupCopy.rename}>
                      <Button
                        type="text"
                        className="accountGroupIconButton"
                        icon={<EditOutlined />}
                        onClick={() => startRename(pool)}
                        disabled={saving}
                        aria-label={groupCopy.rename}
                      />
                    </Tooltip>
                    <Tooltip title={groupCopy.delete}>
                      <Button
                        type="text"
                        danger
                        className="accountGroupIconButton accountGroupIconButton-danger"
                        icon={<DeleteOutlined />}
                        onClick={() => setDeletePoolCandidate(pool)}
                        disabled={saving}
                        aria-label={groupCopy.delete}
                      />
                    </Tooltip>
                    <Tooltip title={groupCopy.exportGroup}>
                      <Button
                        type="text"
                        className="accountGroupIconButton"
                        icon={<ExportOutlined />}
                        onClick={() => exportPoolAccounts(pool)}
                        disabled={saving || exportingAccounts || pool.entries.length === 0}
                        aria-label={groupCopy.exportGroup}
                      />
                    </Tooltip>
                  </div>
                </header>

                {collapsed ? (
                  <div className="accountGroupCompactPreview">
                    {pool.entries.length === 0 ? (
                      <p className="accountGroupEmptyText">{groupCopy.groupEmpty}</p>
                    ) : (
                      pool.entries.map(renderCollapsedEntry)
                    )}
                  </div>
                ) : null}

                {!collapsed && isAdding ? (
                  <div className="accountGroupAddPanel">
                    <div className="accountGroupAddPanelHeader">
                      <strong>{groupCopy.addAccount}</strong>
                      <button
                        type="button"
                        className="ghost"
                        onClick={() => setAddingToPoolId(null)}
                        disabled={saving}
                      >
                        {copy.common.close}
                      </button>
                    </div>
                    {ungroupedLogicalEntries.length === 0 ? (
                      <p className="accountGroupEmptyText">{groupCopy.addAccountEmpty}</p>
                    ) : (
                      <div className="accountGroupAddEntries">
                        {ungroupedLogicalEntries.map((entry) => {
                          const displayLabel = displayAccountLabel(
                            entry.primary,
                            hideAccountDetails,
                          );

                          return (
                            <button
                              type="button"
                              className="accountGroupAddEntry"
                              key={`${pool.id}-${entry.accountKey}`}
                              onClick={() => onAssignAccountToPool(entry.accountKey, pool.id)}
                              disabled={saving}
                            >
                              <div className="accountGroupAddIdentity">
                                <strong title={displayLabel}>{displayLabel}</strong>
                                <div className="accountGroupAddBadges">
                                  <span>{entry.planLabel}</span>
                                  {entry.isCurrent ? <em>{copy.accountCard.currentStamp}</em> : null}
                                </div>
                              </div>
                              <div className="accountGroupAddMeta">
                                {entry.sourceKind === "relay" ? (
                                  <>
                                    <span>
                                      {displayRelayEndpoint(entry.primary.apiBaseUrl, hideAccountDetails)}
                                    </span>
                                    <strong>
                                      {displayModelName(entry.primary.modelName, hideAccountDetails)}
                                    </strong>
                                  </>
                                ) : (
                                  <>
                                    <span>
                                      {copy.accountCard.fiveHourFallback}{" "}
                                      {percent(remainingPercent(entry.primary.usage?.fiveHour ?? null))}
                                    </span>
                                    <strong>
                                      {copy.accountCard.oneWeekLabel}{" "}
                                      {percent(remainingPercent(entry.primary.usage?.oneWeek ?? null))}
                                    </strong>
                                  </>
                                )}
                              </div>
                            </button>
                          );
                        })}
                      </div>
                    )}
                  </div>
                ) : null}

                {showCards ? (
                  <div className="accountGroupNestedCards">
                    {pool.entries.length === 0 ? (
                      <p className="accountGroupEmptyText">{groupCopy.groupEmpty}</p>
                    ) : (
                      pool.entries.map((entry) => (
                        <div className="accountGroupMemberCard" key={entry.accountKey}>
                          <div className="accountGroupNestedSlot">
                            {renderCardEntryActions(entry)}
                            <AccountCard
                              accounts={entry.variants}
                              exportingAccounts={exportingAccounts}
                              switchingId={switchingId}
                              renamingAccountId={renamingAccountId}
                              pendingDeleteId={pendingDeleteId}
                              notificationProviders={notificationProviders}
                              usageDisplayMode={usageDisplayMode}
                              hideAccountDetails={hideAccountDetails}
                              apiEnhancedLaunchEnabled={apiEnhancedLaunchEnabled}
                              onExport={onExport}
                              onReauthorize={onReauthorize}
                              onRename={onRename}
                              onUpdateApiAccount={onUpdateApiAccount}
                              onUpdateTags={onUpdateTags}
                              onRefreshApiQuota={onRefreshApiQuota}
                              onSwitch={onSwitch}
                              onDelete={onDelete}
                            />
                          </div>
                        </div>
                      ))
                    )}
                  </div>
                ) : null}
              </article>
            );
          })}
        </div>
      ) : null}

      <Modal
        open={Boolean(deletePoolCandidate)}
        title={groupCopy.deleteConfirmTitle}
        okText={groupCopy.deleteConfirmOk}
        cancelText={groupCopy.deleteConfirmCancel}
        okButtonProps={{ danger: true, disabled: saving, className: "accountGroupConfirmButton" }}
        cancelButtonProps={{ disabled: saving, className: "accountGroupConfirmButton" }}
        onOk={() => {
          if (!deletePoolCandidate) {
            return;
          }
          onDeletePool(deletePoolCandidate.id);
          setDeletePoolCandidate(null);
        }}
        onCancel={() => setDeletePoolCandidate(null)}
      >
        <p className="accountGroupDeleteConfirmText">
          {groupCopy.deleteConfirmContent}
        </p>
      </Modal>

      {ungroupedAccounts.length > 0 || accounts.length === 0 ? (
        <section className="accountUngroupedWorkspace">
          {accountPools.length > 0 ? (
            <div className="accountUngroupedHeader">
              <div className="accountUngroupedHeading">
                <h3>{groupCopy.ungroupedTitle}</h3>
                <p>{groupCopy.ungroupedDescription}</p>
              </div>
            </div>
          ) : null}
          <div className="accountUngroupedGrid">
            <AccountsGrid
              accounts={ungroupedAccountsForDisplay}
              loading={loading}
              exportingAccounts={exportingAccounts}
              switchingId={switchingId}
              renamingAccountId={renamingAccountId}
              pendingDeleteId={pendingDeleteId}
              notificationProviders={notificationProviders}
              usageDisplayMode={usageDisplayMode}
              hideAccountDetails={hideAccountDetails}
              apiEnhancedLaunchEnabled={apiEnhancedLaunchEnabled}
              onExport={onExport}
              onReauthorize={onReauthorize}
              onRename={onRename}
              onUpdateApiAccount={onUpdateApiAccount}
              onUpdateTags={onUpdateTags}
              onRefreshApiQuota={onRefreshApiQuota}
              onSwitch={onSwitch}
              onDelete={onDelete}
            />
          </div>
        </section>
      ) : null}
    </section>
  );
}
