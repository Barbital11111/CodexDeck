import type { ReactNode } from "react";
import {
  CalendarOutlined,
  CaretRightOutlined,
  ClockCircleOutlined,
  CodeOutlined,
  DeleteOutlined,
  FileTextOutlined,
  SyncOutlined,
  TagOutlined,
  UploadOutlined,
} from "@ant-design/icons";

type QuotaTone = "high" | "medium";

type ReferenceQuotaItemProps = {
  icon: ReactNode;
  label: string;
  value: string;
  tone: QuotaTone;
  progress: number;
  resetText: string;
};

function ReferenceQuotaItem({
  icon,
  label,
  value,
  tone,
  progress,
  resetText,
}: ReferenceQuotaItemProps) {
  return (
    <div className="quota-item">
      <div className="quota-header">
        {icon}
        <span className="quota-label">{label}</span>
        <span className={`quota-pct ${tone}`}>{value}</span>
      </div>
      <div className="quota-bar-track" aria-hidden="true">
        <div className={`quota-bar ${tone}`} style={{ width: `${progress}%` }} />
      </div>
      <span className="quota-reset">{resetText}</span>
    </div>
  );
}

const referenceActions = [
  { label: "CLI 快速启动", icon: <CodeOutlined /> },
  { label: "编辑标签", icon: <TagOutlined /> },
  { label: "账号备注", icon: <FileTextOutlined /> },
  { label: "切换", icon: <CaretRightOutlined /> },
  { label: "刷新配额", icon: <SyncOutlined /> },
  { label: "导出", icon: <UploadOutlined /> },
  { label: "删除", icon: <DeleteOutlined /> },
];

export function ReferenceAccountCard() {
  return (
    <article
      className="cockpitReferenceCard codex-account-card"
      aria-label="Cockpit Tools 参考样式账号卡片"
    >
      <div className="card-top">
        <div className="card-select">
          <input type="checkbox" aria-label="选择 Cockpit 参考账号" />
        </div>
        <span className="account-email" title="preview-account@example.test">
          preview-account...
        </span>
        <span className="codex-status-pill quota-refresh">OAuth</span>
        <span className="tier-badge team">TEAM</span>
      </div>

      <div className="account-sub-line">
        <span className="codex-login-subline" title="团队：演示团队">
          团队：演示团队
        </span>
        <button type="button" className="codex-account-note-chip">
          <FileTextOutlined />
          <span>加备注</span>
        </button>
      </div>

      <div className="account-sub-line">
        <span
          className="codex-login-subline"
          title="使用 Password 登录 | 用户 ID: user-preview-0001"
        >
          使用 Password 登录 | 用户 ID: user-preview-0001
        </span>
      </div>

      <div className="cockpitAuthPlaceholder" aria-hidden="true" />

      <div className="codex-quota-section">
        <ReferenceQuotaItem
          icon={<ClockCircleOutlined />}
          label="5h"
          value="100%"
          tone="high"
          progress={100}
          resetText="已重置"
        />
        <ReferenceQuotaItem
          icon={<CalendarOutlined />}
          label="Weekly"
          value="72%"
          tone="medium"
          progress={72}
          resetText="已重置"
        />
      </div>

      <div className="codex-subscription-footer missing" title="未获得订阅信息">
        <div className="codex-subscription-footer-main">
          <CalendarOutlined />
          <strong>未获得订阅信息</strong>
        </div>
      </div>

      <div className="codex-card-bottom">
        <span className="card-date">2026/04/23 14:01</span>
        <div className="card-footer">
          <div className="card-actions">
            {referenceActions.map((action) => (
              <button type="button" className="card-action-btn" key={action.label} title={action.label}>
                {action.icon}
              </button>
            ))}
          </div>
        </div>
      </div>
    </article>
  );
}
