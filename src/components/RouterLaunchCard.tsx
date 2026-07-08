import {
  CaretRightOutlined,
  ClusterOutlined,
  SyncOutlined,
} from "@ant-design/icons";
import { Button, Checkbox, Drawer, Select, Tooltip, Typography } from "antd";
import { useMemo, useState } from "react";
import type { AccountSummary, AppSettings, ModelRouterRouteSelection } from "../types/app";
import { displayAccountLabel, displayModelName } from "../utils/privacy";

type RouterLaunchCardProps = {
  accounts: AccountSummary[];
  switchingId: string | null;
  hideAccountDetails: boolean;
  settings: AppSettings;
  skin?: "classic" | "modern";
  onSetModelRouterMode: (enabled: boolean, relayAccountId: string | null) => void | Promise<void>;
  onLaunchCurrentCodexConfig: () => void | Promise<void>;
  onUpdateSettings: (
    patch: Partial<AppSettings>,
    options?: { silent?: boolean; keepInteractive?: boolean },
  ) => void | Promise<void>;
};

type RouterModelRouteOption = {
  key: string;
  accountId: string;
  model: string;
  displayName: string;
  requestModel: string;
  accountLabel: string;
};

function routerRouteKey(accountId: string, model: string) {
  return `${accountId}:${model}`;
}

function buildRouterRouteSelectionKeys(selections: ModelRouterRouteSelection[] | undefined) {
  return new Set(
    (selections ?? [])
      .map((selection) => routerRouteKey(selection.accountId, selection.model))
      .filter((key) => key.trim().length > 1),
  );
}

function buildRelayOptionLabel(account: AccountSummary, hideAccountDetails: boolean) {
  const label = displayAccountLabel(account, hideAccountDetails);
  const model = displayModelName(account.modelName, hideAccountDetails);
  return model ? `${label} · ${model}` : label;
}

function buildRelayOptionTitle(account: AccountSummary, hideAccountDetails: boolean) {
  return displayAccountLabel(account, hideAccountDetails);
}

function buildRelayOptionModel(account: AccountSummary, hideAccountDetails: boolean) {
  return displayModelName(account.modelName, hideAccountDetails);
}

function RouterRelayOptionLabel({
  title,
  model,
}: {
  title: string;
  model: string;
}) {
  return (
    <span className="routerLaunchOptionLabel" title={model ? `${title} · ${model}` : title}>
      <strong>{title}</strong>
      {model ? <span>{model}</span> : null}
    </span>
  );
}

export function RouterLaunchCard({
  accounts,
  switchingId,
  hideAccountDetails,
  settings,
  skin = "modern",
  onSetModelRouterMode,
  onLaunchCurrentCodexConfig,
  onUpdateSettings,
}: RouterLaunchCardProps) {
  const [routerRelayAccountId, setRouterRelayAccountId] = useState<string | null>(null);
  const [routerModelDrawerOpen, setRouterModelDrawerOpen] = useState(false);
  const [routerModelDraftKeys, setRouterModelDraftKeys] = useState<Set<string> | null>(null);

  const relayAccounts = useMemo(
    () => accounts.filter((account) => account.sourceKind === "relay"),
    [accounts],
  );
  const routerRouteOptions = useMemo<RouterModelRouteOption[]>(
    () =>
      relayAccounts.flatMap((account) => {
        const accountLabel = displayAccountLabel(account, hideAccountDetails);
        const catalog =
          account.modelCatalog && account.modelCatalog.length > 0
            ? account.modelCatalog
            : account.modelName?.trim()
              ? [
                  {
                    model: account.modelName.trim(),
                    displayName: null,
                    requestModel: null,
                    contextWindow: null,
                    enabled: true,
                  },
                ]
              : [];

        return catalog
          .filter((entry) => entry.enabled !== false && entry.model.trim())
          .map((entry) => {
            const model = entry.model.trim();
            const displayName = entry.displayName?.trim() || model;
            const requestModel = entry.requestModel?.trim() || model;
            return {
              key: routerRouteKey(account.id, model),
              accountId: account.id,
              model,
              displayName,
              requestModel,
              accountLabel,
            };
          });
      }),
    [hideAccountDetails, relayAccounts],
  );
  const routerRouteOptionKeys = useMemo(
    () => new Set(routerRouteOptions.map((option) => option.key)),
    [routerRouteOptions],
  );
  const savedRouterModelKeys = useMemo(
    () => buildRouterRouteSelectionKeys(settings.modelRouterRouteSelections),
    [settings.modelRouterRouteSelections],
  );
  const effectiveRouterModelKeys = useMemo(() => {
    const selected = savedRouterModelKeys.size > 0 ? savedRouterModelKeys : routerRouteOptionKeys;
    return new Set(Array.from(selected).filter((key) => routerRouteOptionKeys.has(key)));
  }, [routerRouteOptionKeys, savedRouterModelKeys]);
  const currentRouterModelDraftKeys = routerModelDraftKeys ?? effectiveRouterModelKeys;
  const selectedRouterModelCount = effectiveRouterModelKeys.size;
  const effectiveRouterRelayAccountId = relayAccounts.some(
    (account) => account.id === routerRelayAccountId,
  )
    ? routerRelayAccountId
    : relayAccounts.some((account) => account.id === settings.modelRouterAccountId)
      ? settings.modelRouterAccountId ?? null
      : relayAccounts.find((account) => account.isCurrent)?.id ?? relayAccounts[0]?.id ?? null;
  const selectedRouterRelay =
    relayAccounts.find((account) => account.id === effectiveRouterRelayAccountId) ?? null;
  const routerModeBusy = Boolean(switchingId?.startsWith("router-mode:"));
  const launchBusy = switchingId === "router-launch";
  const routeStartBusy = routerModeBusy || launchBusy;
  const isBusy = Boolean(switchingId);
  const canLaunchRouter = Boolean(
    selectedRouterRelay &&
      !isBusy &&
      selectedRouterModelCount > 0,
  );
  const canSaveRouterModels = currentRouterModelDraftKeys.size > 0 && !isBusy;
  const previewModelNames = routerRouteOptions
    .filter((option) => effectiveRouterModelKeys.has(option.key))
    .slice(0, 3)
    .map((option) => option.displayName);
  const hasRelayAccounts = relayAccounts.length > 0;

  const openRouterModelDrawer = () => {
    setRouterModelDraftKeys(new Set(effectiveRouterModelKeys));
    setRouterModelDrawerOpen(true);
  };

  const closeRouterModelDrawer = () => {
    setRouterModelDrawerOpen(false);
    setRouterModelDraftKeys(null);
  };

  const selectAllRouterModels = () => {
    setRouterModelDraftKeys(new Set(routerRouteOptionKeys));
  };

  const clearRouterModels = () => {
    setRouterModelDraftKeys(new Set());
  };

  const toggleRouterModel = (key: string, checked: boolean) => {
    setRouterModelDraftKeys((current) => {
      const next = new Set(current ?? effectiveRouterModelKeys);
      if (checked) {
        next.add(key);
      } else {
        next.delete(key);
      }
      return next;
    });
  };

  const saveRouterModels = async () => {
    const nextSelections = routerRouteOptions
      .filter((option) => currentRouterModelDraftKeys.has(option.key))
      .map((option) => ({ accountId: option.accountId, model: option.model }));
    await onUpdateSettings(
      { modelRouterRouteSelections: nextSelections },
      { silent: true, keepInteractive: true },
    );
    closeRouterModelDrawer();
    if (settings.modelRouterEnabled) {
      void onSetModelRouterMode(true, selectedRouterRelay?.id ?? null);
    }
  };

  const launchRouter = async () => {
    if (!selectedRouterRelay || selectedRouterModelCount === 0 || isBusy) {
      return;
    }
    await onSetModelRouterMode(true, selectedRouterRelay.id);
    await onLaunchCurrentCodexConfig();
  };

  return (
    <>
      <article
        className={`accountCard routerLaunchCard tone-api routerLaunchCard-${skin} ${
          settings.modelRouterEnabled ? "isCurrent" : ""
        } ${routeStartBusy ? "isSwitching" : ""} ${hasRelayAccounts ? "" : "isEmpty"}`}
      >
        <header className="cardHeader">
          <div className="cardIdentity">
            <div className="cardBadges">
              <span className="cardBadge planBadge">路由</span>
              <span className="cardBadge stateBadge">
                {settings.modelRouterEnabled ? "已开启" : "未开启"}
              </span>
            </div>
            <h3 className={settings.modelRouterEnabled ? "nameCurrent" : ""}>路由模式启动</h3>
          </div>
        </header>

        <div className="relayInfoPanel routerLaunchPanelBody">
          <label className="routerLaunchField">
            <span>默认配置</span>
            <Select
              className="routerLaunchSelect"
              value={effectiveRouterRelayAccountId ?? undefined}
              placeholder={hasRelayAccounts ? "选择默认 API" : "暂无 API 账号"}
              optionLabelProp="label"
              options={relayAccounts.map((account) => {
                const title = buildRelayOptionTitle(account, hideAccountDetails);
                const model = buildRelayOptionModel(account, hideAccountDetails);
                return {
                  value: account.id,
                  label: <RouterRelayOptionLabel title={title} model={model} />,
                  title: buildRelayOptionLabel(account, hideAccountDetails),
                };
              })}
              optionRender={(option) => option.data.label}
              onChange={(value) => {
                setRouterRelayAccountId(value);
                if (settings.modelRouterEnabled) {
                  void onSetModelRouterMode(true, value);
                }
              }}
              disabled={isBusy || !hasRelayAccounts}
              aria-label="默认 API"
            />
          </label>

          <div className="routerLaunchStats">
            <div className="apiQuotaMetric">
              <span>模型</span>
              <strong>
                {selectedRouterModelCount}/{routerRouteOptions.length}
              </strong>
            </div>
            <div className="apiQuotaMetric">
              <span>状态</span>
              <strong>{settings.modelRouterEnabled ? "已写入" : "待启动"}</strong>
            </div>
          </div>
        </div>

        <div className="accountMetaPanel routerLaunchMetaPanel">
          <div className="accountMetaRow">
            <span className="accountMetaLabel">编辑模型</span>
            <div className="accountMetaValue accountTagSummary">
              {previewModelNames.length > 0 ? (
                <div className="accountTagList">
                  {previewModelNames.map((model, index) => (
                    <span className="accountTagChip" key={`${model}-${index}`}>
                      {model}
                    </span>
                  ))}
                  {selectedRouterModelCount > previewModelNames.length ? (
                    <span className="accountMetaEmpty">
                      +{selectedRouterModelCount - previewModelNames.length}
                    </span>
                  ) : null}
                </div>
              ) : (
                <span className="accountMetaEmpty">
                  {hasRelayAccounts ? "未选择模型" : "添加 API 账号后可选择模型"}
                </span>
              )}
            </div>
          </div>
        </div>

        <footer className="cardFooter">
          <div className="cardFooterActions" aria-label="路由模式操作">
            <Tooltip title={hasRelayAccounts ? "路由启动" : "先添加 API 账号"}>
              <Button
                className="cardFooterAction"
                icon={routeStartBusy ? <SyncOutlined spin /> : <CaretRightOutlined />}
                onClick={() => void launchRouter()}
                disabled={!canLaunchRouter}
                aria-label="路由启动"
              />
            </Tooltip>
            <Tooltip title="编辑模型">
              <Button
                className="cardFooterAction"
                icon={<ClusterOutlined />}
                onClick={openRouterModelDrawer}
                disabled={routerRouteOptions.length === 0 || isBusy}
                aria-label="编辑模型"
              />
            </Tooltip>
          </div>
        </footer>
      </article>

      <Drawer
        className="routerModelDrawer"
        title="路由模型"
        placement="right"
        open={routerModelDrawerOpen}
        size={560}
        onClose={closeRouterModelDrawer}
        destroyOnHidden
        footer={
          <div className="routerModelDrawerFooter">
            <Button autoInsertSpace={false} onClick={closeRouterModelDrawer}>
              取消
            </Button>
            <Button
              type="primary"
              autoInsertSpace={false}
              disabled={!canSaveRouterModels}
              onClick={() => void saveRouterModels()}
            >
              保存
            </Button>
          </div>
        }
      >
        <div className="routerModelDrawerBody">
          <div className="routerModelDrawerActions">
            <Typography.Text type="secondary">
              已选 {currentRouterModelDraftKeys.size} / {routerRouteOptions.length}
            </Typography.Text>
            <div className="routerModelDrawerActionButtons">
              <Button
                size="small"
                autoInsertSpace={false}
                onClick={selectAllRouterModels}
                disabled={routerRouteOptions.length === 0}
              >
                全选
              </Button>
              <Button
                size="small"
                autoInsertSpace={false}
                onClick={clearRouterModels}
                disabled={routerRouteOptions.length === 0}
              >
                清空
              </Button>
            </div>
          </div>
          <div className="routerModelRouteList">
            {routerRouteOptions.map((option) => (
              <label className="routerModelRouteItem" key={option.key}>
                <Checkbox
                  checked={currentRouterModelDraftKeys.has(option.key)}
                  onChange={(event) => toggleRouterModel(option.key, event.target.checked)}
                />
                <span className="routerModelRouteMain">
                  <strong>{option.displayName}</strong>
                  <span>{option.model}</span>
                </span>
                <span className="routerModelRouteMeta">
                  <span>{option.accountLabel}</span>
                  {option.requestModel !== option.model ? <span>{option.requestModel}</span> : null}
                </span>
              </label>
            ))}
            {routerRouteOptions.length === 0 ? (
              <div className="routerModelRouteEmpty">暂无可用模型</div>
            ) : null}
          </div>
        </div>
      </Drawer>
    </>
  );
}
