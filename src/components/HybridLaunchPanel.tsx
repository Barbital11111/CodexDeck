import { Button, Select, Space, Typography } from "antd";
import { useMemo, useState } from "react";
import { useI18n } from "../i18n/I18nProvider";
import type { AccountSummary } from "../types/app";
import { displayAccountLabel, displayModelName, displayRelayEndpoint } from "../utils/privacy";

type HybridLaunchPanelProps = {
  accounts: AccountSummary[];
  switchingId: string | null;
  hideAccountDetails: boolean;
  variant?: "classic" | "modern";
  onSwitchHybrid: (
    chatgptAccount: AccountSummary,
    relayAccount: AccountSummary,
    options?: { useModelRouter?: boolean },
  ) => void;
};

function buildChatgptOptionLabel(account: AccountSummary, hideAccountDetails: boolean) {
  const label = displayAccountLabel(account, hideAccountDetails);
  const plan = account.planType || account.usage?.planType;
  return plan ? `${label} · ${plan.toUpperCase()}` : label;
}

function buildRelayOptionLabel(account: AccountSummary, hideAccountDetails: boolean) {
  const label = displayAccountLabel(account, hideAccountDetails);
  const endpoint = displayRelayEndpoint(account.apiBaseUrl, hideAccountDetails);
  const model = displayModelName(account.modelName, hideAccountDetails);
  return `${label} · ${endpoint} · ${model}`;
}

export function HybridLaunchPanel({
  accounts,
  switchingId,
  hideAccountDetails,
  variant = "modern",
  onSwitchHybrid,
}: HybridLaunchPanelProps) {
  const { copy } = useI18n();
  const [chatgptAccountId, setChatgptAccountId] = useState<string | null>(null);
  const [relayAccountId, setRelayAccountId] = useState<string | null>(null);

  const chatgptAccounts = useMemo(
    () => accounts.filter((account) => account.sourceKind === "chatgpt"),
    [accounts],
  );
  const relayAccounts = useMemo(
    () => accounts.filter((account) => account.sourceKind === "relay"),
    [accounts],
  );

  const effectiveChatgptAccountId = chatgptAccounts.some((account) => account.id === chatgptAccountId)
    ? chatgptAccountId
    : chatgptAccounts[0]?.id ?? null;
  const effectiveRelayAccountId = relayAccounts.some((account) => account.id === relayAccountId)
    ? relayAccountId
    : relayAccounts.find((account) => account.isCurrent)?.id ?? relayAccounts[0]?.id ?? null;
  const selectedChatgpt =
    chatgptAccounts.find((account) => account.id === effectiveChatgptAccountId) ?? null;
  const selectedRelay =
    relayAccounts.find((account) => account.id === effectiveRelayAccountId) ?? null;
  const canLaunch = Boolean(selectedChatgpt && selectedRelay && !switchingId);

  const launchPanelClassName = `launchModePanel launchModePanel-${variant}`;

  return (
    <section className={launchPanelClassName} aria-label="启动模式">
      <section className="hybridLaunchPanel launchModeCard" aria-label={copy.addAccount.hybridTitle}>
        <div className="hybridLaunchCopy">
          <Typography.Text strong>{copy.addAccount.hybridTitle}</Typography.Text>
        </div>
        <Space.Compact className="hybridLaunchControls">
          <Select
            className="hybridLaunchSelect hybridLaunchSelect-account"
            value={effectiveChatgptAccountId ?? undefined}
            placeholder={copy.addAccount.hybridChatgptPlaceholder}
            options={chatgptAccounts.map((account) => ({
              value: account.id,
              label: buildChatgptOptionLabel(account, hideAccountDetails),
            }))}
            onChange={(value) => setChatgptAccountId(value)}
            disabled={Boolean(switchingId) || chatgptAccounts.length === 0}
            aria-label={copy.addAccount.hybridChatgptLabel}
          />
          <Select
            className="hybridLaunchSelect hybridLaunchSelect-relay"
            value={effectiveRelayAccountId ?? undefined}
            placeholder={copy.addAccount.hybridRelayPlaceholder}
            options={relayAccounts.map((account) => ({
              value: account.id,
              label: buildRelayOptionLabel(account, hideAccountDetails),
            }))}
            onChange={(value) => setRelayAccountId(value)}
            disabled={Boolean(switchingId) || relayAccounts.length === 0}
            aria-label={copy.addAccount.hybridRelayLabel}
          />
          <Button
            type="primary"
            className="hybridLaunchButton"
            disabled={!canLaunch}
            title={canLaunch ? copy.addAccount.hybridStart : copy.addAccount.hybridMissing}
            aria-label={copy.addAccount.hybridStart}
            loading={Boolean(switchingId?.startsWith("hybrid:"))}
            onClick={() => {
              if (selectedChatgpt && selectedRelay) {
                onSwitchHybrid(selectedChatgpt, selectedRelay);
              }
            }}
          >
            {copy.addAccount.hybridStart}
          </Button>
        </Space.Compact>
      </section>
    </section>
  );

}
