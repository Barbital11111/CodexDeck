import { useI18n } from "../i18n/I18nProvider";

type AppTab = "accounts" | "notifications" | "settings";
type NotificationViewTab = "settings" | "pipelines" | "templates" | "tests" | "activity";

type BottomDockProps = {
  activeTab: AppTab;
  onSelectTab: (tab: AppTab) => void;
  notificationView: NotificationViewTab;
  onSelectNotificationView: (view: NotificationViewTab) => void;
};

function AccountsIcon() {
  return (
    <svg className="bottomDockIcon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <rect x="4" y="4" width="7" height="7" rx="1.5" />
      <rect x="13" y="4" width="7" height="7" rx="1.5" />
      <rect x="4" y="13" width="7" height="7" rx="1.5" />
      <rect x="13" y="13" width="7" height="7" rx="1.5" />
    </svg>
  );
}

function NotificationsIcon() {
  return (
    <svg className="bottomDockIcon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path d="M12 4.5a5 5 0 0 0-5 5v3.2l-1.4 2.4c-.32.54.07 1.22.7 1.22h11.4c.63 0 1.02-.68.7-1.22L17 12.7V9.5a5 5 0 0 0-5-5Z" />
      <path d="M10 18.2a2.2 2.2 0 0 0 4 0" />
    </svg>
  );
}

function PlainIcon({ children }: { children: string }) {
  return (
    <span className="bottomDockIcon" aria-hidden="true">
      {children}
    </span>
  );
}

function SettingsIcon() {
  return (
    <svg className="bottomDockIcon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path d="M10.33 4.32c.43-1.76 2.91-1.76 3.34 0a1.72 1.72 0 0 0 2.57 1.06c1.54-.93 3.3.83 2.37 2.37a1.72 1.72 0 0 0 1.06 2.57c1.76.43 1.76 2.91 0 3.34a1.72 1.72 0 0 0-1.06 2.57c.93 1.54-.83 3.3-2.37 2.37a1.72 1.72 0 0 0-2.57 1.06c-.43 1.76-2.91 1.76-3.34 0a1.72 1.72 0 0 0-2.57-1.06c-1.54.93-3.3-.83-2.37-2.37a1.72 1.72 0 0 0-1.06-2.57c-1.76-.43-1.76-2.91 0-3.34a1.72 1.72 0 0 0 1.06-2.57c-.93-1.54.83-3.3 2.37-2.37.99.6 2.29.07 2.57-1.06Z" />
      <circle cx="12" cy="12" r="3.1" />
    </svg>
  );
}

export function BottomDock({
  activeTab,
  onSelectTab,
  notificationView,
  onSelectNotificationView,
}: BottomDockProps) {
  const { copy } = useI18n();
  const accountActive = activeTab === "accounts";
  const notificationsActive = activeTab === "notifications";
  const settingsActive = activeTab === "settings";
  const notificationItems: Array<{ id: NotificationViewTab; label: string }> = [
    { id: "settings", label: "通知概览" },
    { id: "pipelines", label: "通知规则" },
    { id: "tests", label: "发送渠道" },
    { id: "templates", label: "消息模板" },
    { id: "activity", label: "发送记录" },
  ];

  return (
    <nav className="bottomDock" aria-label={copy.bottomDock.ariaLabel}>
      <div className="dockWindowLights" aria-hidden="true">
        <span />
        <span />
        <span />
      </div>
      <div className="dockBrand">
        <img className="dockLogoMark" src="/codexdeck.png" alt="" aria-hidden="true" />
        <strong>CodexDeck</strong>
      </div>
      <div className="dockSection">
        <span className="dockSectionLabel">工作台</span>
        <button type="button" className="bottomDockButton isDisabled" aria-disabled="true">
          <PlainIcon>⌂</PlainIcon>
          <span className="bottomDockLabel">概览</span>
        </button>
        <button
          className={`bottomDockButton${accountActive ? " isActive" : ""}`}
          onClick={() => onSelectTab("accounts")}
          aria-label={copy.bottomDock.accounts}
          title={copy.bottomDock.accounts}
        >
          <AccountsIcon />
          <span className="bottomDockLabel">账户列表</span>
        </button>
        <button type="button" className="bottomDockButton isDisabled" aria-disabled="true">
          <PlainIcon>◎</PlainIcon>
          <span className="bottomDockLabel">额度总览</span>
        </button>
        <button type="button" className="bottomDockButton isDisabled" aria-disabled="true">
          <PlainIcon>⌁</PlainIcon>
          <span className="bottomDockLabel">使用统计</span>
        </button>
        <button
          className={`bottomDockButton${notificationsActive ? " isActive" : ""}`}
          onClick={() => {
            onSelectTab("notifications");
            onSelectNotificationView("settings");
          }}
          aria-label="通知中心"
          title="通知中心"
        >
          <NotificationsIcon />
          <span className="bottomDockLabel">通知中心</span>
        </button>
        {notificationsActive ? (
          <div className="dockSubmenu" aria-label="通知中心二级菜单">
            {notificationItems.map((item) => (
              <button
                type="button"
                className={`dockSubmenuButton${notificationView === item.id ? " isActive" : ""}`}
                key={item.id}
                onClick={() => onSelectNotificationView(item.id)}
              >
                <span>{item.label}</span>
              </button>
            ))}
          </div>
        ) : null}
      </div>
      <div className="dockSection">
        <span className="dockSectionLabel">工具箱</span>
        <button
          className={`bottomDockButton${settingsActive ? " isActive" : ""}`}
          onClick={() => onSelectTab("settings")}
          aria-label={copy.bottomDock.settings}
          title={copy.bottomDock.settings}
        >
          <SettingsIcon />
          <span className="bottomDockLabel">{copy.bottomDock.settings}</span>
        </button>
      </div>
    </nav>
  );
}
