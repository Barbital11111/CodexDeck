import {
  BarChartOutlined,
  EyeInvisibleOutlined,
  EyeOutlined,
  FolderAddOutlined,
  MoreOutlined,
  PlusOutlined,
  SkinOutlined,
  SwapOutlined,
  DownloadOutlined,
} from "@ant-design/icons";
import { Button, Dropdown, Space, Typography, type MenuProps } from "antd";
import { useI18n } from "../i18n/I18nProvider";
import type { UiSkinMode } from "../types/app";

type AppHeaderProps = {
  activeTab: "accounts" | "providers" | "notifications" | "settings";
  onOpenAddDialog: () => void;
  onCreatePool: () => void;
  onSmartSwitch: () => void;
  onExportAccounts: () => void;
  onToggleHideAccountDetails: () => void;
  onSetUiSkin: (mode: UiSkinMode) => void;
  saving: boolean;
  smartSwitching: boolean;
  exportingAccounts: boolean;
  accountCount: number;
  hideAccountDetails: boolean;
  uiSkinMode: UiSkinMode;
};

function pageTitle(activeTab: AppHeaderProps["activeTab"]) {
  if (activeTab === "providers") {
    return "供应商与模型";
  }
  if (activeTab === "notifications") {
    return "通知中心";
  }
  if (activeTab === "settings") {
    return "设置";
  }
  return "账户";
}

export function AppHeader({
  activeTab,
  onOpenAddDialog,
  onCreatePool,
  onSmartSwitch,
  onExportAccounts,
  onToggleHideAccountDetails,
  onSetUiSkin,
  saving,
  smartSwitching,
  exportingAccounts,
  accountCount,
  hideAccountDetails,
  uiSkinMode,
}: AppHeaderProps) {
  const { copy } = useI18n();
  const menu: MenuProps = {
    items: [
      {
        key: "smart",
        icon: <SwapOutlined />,
        label: copy.addAccount.smartSwitch,
        disabled: smartSwitching || accountCount === 0,
      },
      {
        key: "group",
        icon: <FolderAddOutlined />,
        label: copy.accountPools.create,
        disabled: saving,
      },
      {
        key: "privacy",
        icon: hideAccountDetails ? <EyeOutlined /> : <EyeInvisibleOutlined />,
        label: hideAccountDetails ? "显示信息" : "隐藏信息",
      },
      {
        key: "uiSkin",
        icon: <SkinOutlined />,
        label: uiSkinMode === "classic" ? "切到新版界面" : "切到经典界面",
      },
      {
        key: "export",
        icon: <DownloadOutlined />,
        label: copy.metaStrip.exportAll,
        disabled: exportingAccounts || accountCount === 0,
      },
    ],
    onClick: ({ key }) => {
      if (key === "smart") {
        onSmartSwitch();
      }
      if (key === "group") {
        onCreatePool();
      }
      if (key === "privacy") {
        onToggleHideAccountDetails();
      }
      if (key === "uiSkin") {
        onSetUiSkin(uiSkinMode === "classic" ? "modern" : "classic");
      }
      if (key === "export") {
        onExportAccounts();
      }
    },
  };

  return (
    <header className="appHeader">
      <Space orientation="vertical" size={2} className="appHeaderTitle">
        <Typography.Title level={1}>{pageTitle(activeTab)}</Typography.Title>
        {activeTab === "accounts" ? (
          <Typography.Text type="secondary">
            <BarChartOutlined /> {accountCount} 个账号
          </Typography.Text>
        ) : null}
      </Space>
      <Space size={10} className="appHeaderActions">
        {activeTab === "accounts" ? (
          <Button type="primary" icon={<PlusOutlined />} onClick={onOpenAddDialog}>
            {copy.addAccount.startButton}
          </Button>
        ) : null}
        <Dropdown menu={menu} trigger={["click"]}>
          <Button icon={<MoreOutlined />} aria-label="更多操作" />
        </Dropdown>
      </Space>
    </header>
  );
}
