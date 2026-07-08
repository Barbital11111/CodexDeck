import { ApiOutlined, PlusOutlined, ReloadOutlined } from "@ant-design/icons";
import { Button, Card, Empty, Space, Tag, Typography } from "antd";
import type { AccountSummary, NotificationProviderConfig } from "../types/app";
import { displayModelName, displayRelayEndpoint } from "../utils/privacy";

type ProvidersViewProps = {
  accounts: AccountSummary[];
  notificationProviders: NotificationProviderConfig[];
  hideAccountDetails: boolean;
  onOpenAddDialog: () => void;
  onRefreshApiQuota: (account: AccountSummary) => void;
};

function providerStatus(account: AccountSummary) {
  const capabilityNotice =
    account.profileLastValidationError &&
    (account.profileLastValidationError.includes("接口能力已重置为仅 /v1/chat/completions") ||
      account.profileLastValidationError.includes("已跳过接口探测，仅启用 /v1/chat/completions"));
  if (account.profileIntegrityError || (account.profileLastValidationError && !capabilityNotice)) {
    return <Tag color="error">需检查</Tag>;
  }
  if (capabilityNotice) {
    return <Tag color="default">仅 Chat</Tag>;
  }
  if (account.balanceDisplayEnabled) {
    return <Tag color="success">额度已启用</Tag>;
  }
  return <Tag>未显示额度</Tag>;
}

export function ProvidersView({
  accounts,
  notificationProviders,
  hideAccountDetails,
  onOpenAddDialog,
  onRefreshApiQuota,
}: ProvidersViewProps) {
  const relayAccounts = accounts.filter((account) => account.sourceKind === "relay");

  return (
    <section className="providersView">
      <Card
        className="providersIntroCard"
        title="供应商与模型"
        extra={
          <Button type="primary" icon={<PlusOutlined />} onClick={onOpenAddDialog}>
            导入供应商
          </Button>
        }
      >
        {relayAccounts.length === 0 ? (
          <Empty description="还没有 API 供应商。" />
        ) : (
          <div className="providersGrid">
            {relayAccounts.map((account) => {
              const modelCount =
                account.modelCatalog?.filter((entry) => entry.enabled !== false).length ?? 0;
              const provider = notificationProviders.find(
                (item) => item.accountKey === account.accountKey,
              );
              return (
                <Card className="providerCard" size="small" key={account.id}>
                  <div className="providerCardHead">
                    <span className="providerCardIcon" aria-hidden="true">
                      <ApiOutlined />
                    </span>
                    <Space orientation="vertical" size={1} className="providerCardTitle">
                      <Typography.Text strong>{account.label}</Typography.Text>
                      <Typography.Text type="secondary">
                        {displayRelayEndpoint(account.apiBaseUrl, hideAccountDetails)}
                      </Typography.Text>
                    </Space>
                    {providerStatus(account)}
                  </div>
                  <div className="providerCardMeta">
                    <span>首选模型</span>
                    <strong>{displayModelName(account.modelName, hideAccountDetails)}</strong>
                    <span>模型菜单</span>
                    <strong>{modelCount > 0 ? `${modelCount} 个` : "默认模型"}</strong>
                    <span>额度数据源</span>
                    <strong>{provider ? provider.name : account.balanceDisplayEnabled ? "Key 自查" : "未绑定"}</strong>
                  </div>
                  <div className="providerCardActions">
                    <Button
                      icon={<ReloadOutlined />}
                      onClick={() => onRefreshApiQuota(account)}
                      disabled={!account.balanceDisplayEnabled}
                    >
                      刷新配额
                    </Button>
                  </div>
                </Card>
              );
            })}
          </div>
        )}
      </Card>
    </section>
  );
}
