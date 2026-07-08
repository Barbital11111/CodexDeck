import { type ReactNode, useMemo } from "react";
import { DndContext, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { rectSortingStrategy, SortableContext } from "@dnd-kit/sortable";
import type {
  AccountSummary,
  NotificationProviderConfig,
  RelayModelCatalogEntry,
  TrayUsageDisplayMode,
  UpdateApiAccountInput,
} from "../types/app";
import { useI18n } from "../i18n/I18nProvider";
import { AccountCard } from "./AccountCard";
import {
  compareAccountsByRemaining,
  compareAccountsForDisplay,
} from "../utils/accountRanking";
import {
  moveAccountKeyToTarget,
  sortBySavedAccountOrder,
} from "../utils/accountCardOrder";
import { SortableAccountCardSlot } from "./SortableAccountCardSlot";

type AccountGroup = {
  id: string;
  variants: AccountSummary[];
  primary: AccountSummary;
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
  const priorityDiff = planPriority(left.planType ?? left.usage?.planType) - planPriority(right.planType ?? right.usage?.planType);
  if (priorityDiff !== 0) {
    return priorityDiff;
  }

  if (left.isCurrent !== right.isCurrent) {
    return left.isCurrent ? -1 : 1;
  }

  return compareAccountsByRemaining(left, right);
}

function compareAccountGroups(left: AccountGroup, right: AccountGroup): number {
  return compareAccountsForDisplay(left.primary, right.primary);
}

type AccountsGridProps = {
  accounts: AccountSummary[];
  loading: boolean;
  exportingAccounts: boolean;
  switchingId: string | null;
  renamingAccountId: string | null;
  pendingDeleteId: string | null;
  notificationProviders: NotificationProviderConfig[];
  usageDisplayMode: TrayUsageDisplayMode;
  hideAccountDetails: boolean;
  accountCardOrder: string[];
  sortingEnabled?: boolean;
  leadingSlot?: ReactNode;
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
  onReorderAccountCards: (accountKeys: string[]) => void;
  onSwitch: (account: AccountSummary) => void;
  onDelete: (account: AccountSummary) => void;
};

export function AccountsGrid({
  accounts,
  loading,
  exportingAccounts,
  switchingId,
  renamingAccountId,
  pendingDeleteId,
  notificationProviders,
  usageDisplayMode,
  hideAccountDetails,
  accountCardOrder,
  sortingEnabled = false,
  leadingSlot,
  onExport,
  onReauthorize,
  onRename,
  onUpdateApiAccount,
  onProbeApiModels,
  onUpdateTags,
  onRefreshApiQuota,
  onReorderAccountCards,
  onSwitch,
  onDelete,
}: AccountsGridProps) {
  const { copy } = useI18n();
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }));
  const groupedAccounts = useMemo<AccountGroup[]>(() => {
    const groups = new Map<string, AccountSummary[]>();

    for (const account of accounts) {
      const existing = groups.get(account.accountKey);
      if (existing) {
        existing.push(account);
      } else {
        groups.set(account.accountKey, [account]);
      }
    }

    const accountGroups = Array.from(groups.entries()).map(([id, variants]) => {
      const sortedVariants = [...variants].sort(sortVariantsForGroup);
      const primary = sortedVariants.find((item) => item.isCurrent) ?? sortedVariants[0];

      return {
        id,
        variants: sortedVariants,
        primary,
      };
    });
    return sortBySavedAccountOrder(
      accountGroups,
      accountCardOrder,
      (group) => group.id,
      compareAccountGroups,
    );
  }, [accountCardOrder, accounts]);

  const handleDragEnd = (event: DragEndEvent) => {
    const activeKey = String(event.active.id);
    const overKey = event.over ? String(event.over.id) : "";
    if (!overKey || activeKey === overKey) {
      return;
    }
    const currentKeys = groupedAccounts.map((group) => group.id);
    const nextKeys = moveAccountKeyToTarget(currentKeys, activeKey, overKey);
    if (nextKeys.join("\n") !== currentKeys.join("\n")) {
      onReorderAccountCards(nextKeys);
    }
  };

  return (
    <DndContext sensors={sensors} onDragEnd={handleDragEnd}>
      <section className="cards" aria-busy={loading}>
        {leadingSlot}

        {groupedAccounts.length === 0 && !loading && !leadingSlot && (
          <div className="emptyState">
            <h3>{copy.accountsGrid.emptyTitle}</h3>
            <p>{copy.accountsGrid.emptyDescription}</p>
          </div>
        )}

        <SortableContext items={groupedAccounts.map((group) => group.id)} strategy={rectSortingStrategy}>
          {groupedAccounts.map((group) => (
            <SortableAccountCardSlot
              key={group.id}
              id={group.id}
              enabled={sortingEnabled}
              handleVariant="bar"
            >
              {(sortHandle) => (
                <AccountCard
                  accounts={group.variants}
                  exportingAccounts={exportingAccounts}
                  switchingId={switchingId}
                  renamingAccountId={renamingAccountId}
                  pendingDeleteId={pendingDeleteId}
                  notificationProviders={notificationProviders}
                  usageDisplayMode={usageDisplayMode}
                  hideAccountDetails={hideAccountDetails}
                  sortHandle={sortHandle}
                  sortHandlePlacement="body"
                  onExport={onExport}
                  onReauthorize={onReauthorize}
                  onRename={onRename}
                  onUpdateApiAccount={onUpdateApiAccount}
                  onProbeApiModels={onProbeApiModels}
                  onUpdateTags={onUpdateTags}
                  onRefreshApiQuota={onRefreshApiQuota}
                  onSwitch={onSwitch}
                  onDelete={onDelete}
                />
              )}
            </SortableAccountCardSlot>
          ))}
        </SortableContext>
      </section>
    </DndContext>
  );
}
