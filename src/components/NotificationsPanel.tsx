import { forwardRef, useEffect, useMemo, useRef, useState } from "react";
import type { ComponentProps, ComponentRef } from "react";
import { Button, Collapse, Input, Modal, Select, Steps, Switch, Tag, message } from "antd";
import type { ButtonProps, InputProps, SelectProps } from "antd";
import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  NotificationScheduleMode,
  NotificationBotConfig,
  NotificationPipelineConfig,
  NotificationProviderConfig,
  NotificationTargetConfig,
  NotificationTemplateConfig,
  NotificationTemplatePreset,
  UpdateSettingsOptions,
} from "../types/app";

type NotificationsPanelProps = {
  settings: AppSettings;
  saving: boolean;
  viewTab: NotificationViewTab;
  onViewTabChange: (tab: NotificationViewTab) => void;
  onUpdateSettings: (
    patch: Partial<AppSettings>,
    options?: UpdateSettingsOptions,
  ) => void | Promise<void>;
};

type NotificationTab = "pipelines" | "providers" | "bots" | "templates";
type NotificationViewTab = "settings" | "pipelines" | "templates" | "tests" | "activity";
type NotificationResourceTab = Exclude<NotificationTab, "pipelines">;
type NotificationEntityKind = "provider" | "bot" | "pipeline" | "template";

type ProviderDraft = {
  name: string;
  baseUrl: string;
  email: string;
  password: string;
  costMultiplier: string;
};

type BotDraft = {
  name: string;
  kind: NotificationBotConfig["kind"];
  telegramBotToken: string;
  telegramChatId: string;
  webhookUrl: string;
};

type PipelineDraft = {
  id: string;
  name: string;
  providerIds: string[];
  botIds: string[];
  templateId: string;
  templateOverrideEnabled: boolean;
  templateOverride: string;
  aggregateEnabled: boolean;
  scheduleMode: NotificationScheduleMode;
  scheduleDate: string;
  scheduleTime: string;
  scheduleIntervalMinutes: string;
};

type TemplateDraft = {
  id: string;
  name: string;
  preset: NotificationTemplatePreset;
  messageTemplate: string;
};

type DraftTestState = {
  lastTestAt: number | null;
  lastTestError: string | null;
  dirty: boolean;
};

type ProviderDrawer =
  | { kind: "provider"; mode: "create"; id?: undefined }
  | { kind: "provider"; mode: "edit"; id: string };

type BotDrawer =
  | { kind: "bot"; mode: "create"; id?: undefined }
  | { kind: "bot"; mode: "edit"; id: string };

type PipelineDrawer =
  | { kind: "pipeline"; mode: "create"; id?: undefined }
  | { kind: "pipeline"; mode: "edit"; id: string };

type TemplateDrawer =
  | { kind: "template"; mode: "create"; id?: undefined }
  | { kind: "template"; mode: "edit"; id: string };

type ResourceDrawer = ProviderDrawer | BotDrawer | PipelineDrawer | TemplateDrawer;
type ProviderOrBotDrawer = ProviderDrawer | BotDrawer;

type TelegramChatCandidate = {
  id: string;
  title: string;
  chatType: string;
};

type TelegramChatDiscoveryResult = {
  botUsername: string | null;
  chats: TelegramChatCandidate[];
};

type RowMenuState = {
  kind: NotificationEntityKind;
  id: string;
};

type CsButtonProps = Omit<ButtonProps, "type"> & {
  tone?: "ghost" | "primary" | "danger";
};

function classNames(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}
function CsButton({
  tone = "ghost",
  className,
  htmlType = "button",
  children,
  ...props
}: CsButtonProps) {
  return (
    <Button
      {...props}
      autoInsertSpace={props.autoInsertSpace ?? false}
      htmlType={htmlType}
      type={tone === "primary" ? "primary" : "default"}
      danger={tone === "danger" || props.danger}
      className={classNames(
        tone === "primary" ? "primary-button" : "ghost-button",
        tone === "danger" && "notificationDangerButton",
        className,
      )}
    >
      {children}
    </Button>
  );
}

function CsInput({ className, ...props }: InputProps) {
  return <Input {...props} className={classNames("notificationInput", className)} />;
}

const CsTextArea = forwardRef<
  ComponentRef<typeof Input.TextArea>,
  ComponentProps<typeof Input.TextArea>
>(function CsTextArea({ className, ...props }, ref) {
  return <Input.TextArea {...props} ref={ref} className={classNames("notificationInput", className)} />;
});

function CsSelect<ValueType = string>({ className, ...props }: SelectProps<ValueType>) {
  return (
    <Select<ValueType>
      {...props}
      variant="outlined"
      className={classNames("notificationInput notificationSelect", className)}
      classNames={{ popup: { root: "notificationSelectPopup" } }}
    />
  );
}

type RouteSelectOption = {
  value: string;
  label: string;
  description: string;
  disabled?: boolean;
};

const defaultProviderDraft: ProviderDraft = {
  name: "",
  baseUrl: "",
  email: "",
  password: "",
  costMultiplier: "1",
};

const defaultBotDraft: BotDraft = {
  name: "",
  kind: "telegram",
  telegramBotToken: "",
  telegramChatId: "",
  webhookUrl: "",
};

const defaultPipelineDraft: PipelineDraft = {
  id: "",
  name: "",
  providerIds: [],
  botIds: [],
  templateId: "builtin-usage-report",
  templateOverrideEnabled: false,
  templateOverride: "",
  aggregateEnabled: true,
  scheduleMode: "daily",
  scheduleDate: "",
  scheduleTime: "09:00",
  scheduleIntervalMinutes: "30",
};

const defaultTemplateDraft: TemplateDraft = {
  id: "",
  name: "",
  preset: "usageReport",
  messageTemplate: "",
};

const defaultDraftTestState: DraftTestState = {
  lastTestAt: null,
  lastTestError: null,
  dirty: false,
};

const builtinTemplates: Record<
  NotificationTemplatePreset,
  { id: string; label: string; template: string; description: string }
> = {
  test: {
    id: "builtin-test",
    label: "手动测试消息",
    description: "用于验证机器人或 Webhook 的消息输出。",
    template: "CodexDeck 测试消息\n通道：{target}\n时间：{time}",
  },
  usageReport: {
    id: "builtin-usage-report",
    label: "额度日报",
    description: "迁移自旧额度通知插件的 Sub2API 消耗日报格式。",
    template: [
      "📊 {reportTitle} · {reportDate}",
      "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
      "{providerName}",
      "{progressBar} 开销进度 {usageProgress}",
      "累计开销: {totalCost} / 可用总额 {availableTotal}",
      "今日开销: {todayCost}",
      "当前余额: {balance} USD",
      "今日请求/Token: {todayRequests} / {todayTokens}",
      "累计请求/Token: {totalRequests} / {totalTokens}",
      "今日开销占当前余额比例: {todayBalanceRatio}",
      "较上次日报新增累计开销: {previousDelta}",
      "主要模型开销:",
      "{modelCostLines}",
      "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
      "生成时间: {generatedTime}",
    ].join("\n"),
  },
  quotaLow: {
    id: "builtin-quota-low",
    label: "额度预警",
    description: "账号额度接近阈值时使用。",
    template:
      "CodexDeck 额度预警\n账号：{account}\n窗口：{window}\n剩余额度：{remaining}\n预计恢复：{resetTime}",
  },
  quotaRecovered: {
    id: "builtin-quota-recovered",
    label: "恢复提醒",
    description: "额度窗口恢复后使用。",
    template:
      "CodexDeck 额度恢复\n账号：{account}\n窗口：{window}\n恢复时间：{resetTime}\n可以重新使用该账号。",
  },
  accountError: {
    id: "builtin-account-error",
    label: "异常提醒",
    description: "授权失败、刷新失败或账号不可用时使用。",
    template: "CodexDeck 异常提醒\n账号：{account}\n错误：{error}\n时间：{time}",
  },
};

const templateVariableOptions = [
  { token: "{reportTitle}", label: "报告标题", description: "当前规则或日报标题" },
  { token: "{reportDate}", label: "报告日期", description: "生成日报的日期" },
  { token: "{providerName}", label: "数据源名称", description: "绑定的 API 平台名称" },
  { token: "{todayCost}", label: "今日已用", description: "当天已使用额度" },
  { token: "{balance}", label: "剩余额度", description: "平台余额或可用额度" },
  { token: "{totalCost}", label: "累计已用", description: "账号周期累计用量" },
  { token: "{availableTotal}", label: "可用总额", description: "账号周期总额度" },
  { token: "{modelCostLines}", label: "模型明细", description: "按模型汇总的用量行" },
  { token: "{generatedTime}", label: "生成时间", description: "消息渲染完成时间" },
];

function nowUnixSeconds() {
  return Math.floor(Date.now() / 1000);
}

function createLocalId(prefix: string) {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `${prefix}-${crypto.randomUUID()}`;
  }
  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function optionalValue(value: string) {
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}

function hasTauriRuntime() {
  return (
    typeof window !== "undefined" &&
    Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__)
  );
}

function isPreviewRuntime() {
  return !hasTauriRuntime() && import.meta.env.DEV;
}

function formatDateTime(value: number | null | undefined) {
  if (!value) {
    return "尚未测试";
  }
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value * 1000));
}

function normalizeScheduleMode(
  mode: NotificationScheduleMode | null | undefined,
  date?: string | null,
  time?: string | null,
  intervalMinutes?: number | null,
): NotificationScheduleMode {
  if (mode === "manual" || mode === "daily" || mode === "interval" || mode === "date") {
    return mode;
  }
  if (date?.trim()) {
    return "date";
  }
  if (Number.isFinite(intervalMinutes) && (intervalMinutes ?? 0) > 0) {
    return "interval";
  }
  if (time?.trim()) {
    return "daily";
  }
  return "manual";
}

function normalizeScheduleIntervalMinutes(value: string | number | null | undefined) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    return null;
  }
  return Math.min(Math.max(Math.round(parsed), 1), 1440);
}

function formatIntervalMinutes(value: number | null | undefined) {
  if (!value) {
    return "间隔未设置";
  }
  if (value % 60 === 0) {
    const hours = value / 60;
    return `每 ${hours} 小时`;
  }
  if (value > 60) {
    const hours = Math.floor(value / 60);
    const minutes = value % 60;
    return `每 ${hours} 小时 ${minutes} 分钟`;
  }
  return `每 ${value} 分钟`;
}

function formatSchedule(
  mode: NotificationScheduleMode | null | undefined,
  date: string | null,
  time: string | null,
  intervalMinutes?: number | null,
) {
  const scheduleMode = normalizeScheduleMode(mode, date, time, intervalMinutes);
  const normalizedDate = date?.trim();
  const normalizedTime = time?.trim();
  if (scheduleMode === "interval") {
    return formatIntervalMinutes(intervalMinutes ?? null);
  }
  if (scheduleMode === "date") {
    return normalizedDate && normalizedTime
      ? `${normalizedDate} ${normalizedTime}`
      : "指定日期";
  }
  if (scheduleMode === "daily") {
    return normalizedTime ? `每日 ${normalizedTime}` : "每日定时";
  }
  return "手动";
}

function maskSecret(value: string | null | undefined) {
  if (!value) {
    return "未填写";
  }
  if (value.length <= 8) {
    return "••••";
  }
  return `${value.slice(0, 4)}••••${value.slice(-4)}`;
}

function maskEmail(value: string) {
  const trimmed = value.trim();
  if (!trimmed.includes("@")) {
    return maskSecret(trimmed);
  }
  const [name, domain] = trimmed.split("@");
  if (name.length <= 2) {
    return `${name.slice(0, 1)}•••@${domain}`;
  }
  return `${name.slice(0, 2)}•••${name.slice(-1)}@${domain}`;
}

function formatCostMultiplier(value: number) {
  return value.toFixed(4).replace(/0+$/, "").replace(/\.$/, "");
}

function normalizeBaseUrl(value: string) {
  const trimmed = value.trim().replace(/\/+$/, "");
  return trimmed;
}

function normalizeCostMultiplier(value: string | number | null | undefined) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return 1;
  }
  return Math.min(Math.max(parsed, 0.0001), 1000);
}

function resourceStatus(
  item: { enabled: boolean; lastTestAt: number | null; lastTestError: string | null },
) {
  if (!item.enabled) {
    return { label: "已停用", tone: "muted" };
  }
  if (item.lastTestError) {
    return { label: "测试失败", tone: "danger" };
  }
  if (item.lastTestAt) {
    return { label: "可用", tone: "ok" };
  }
  return { label: "未测试", tone: "draft" };
}

function isResourceReady(
  item: { enabled: boolean; lastTestAt: number | null; lastTestError: string | null },
) {
  return item.enabled && Boolean(item.lastTestAt) && !item.lastTestError;
}

function providerById(providers: NotificationProviderConfig[]) {
  return new Map(providers.map((provider) => [provider.id, provider]));
}

function botById(bots: NotificationBotConfig[]) {
  return new Map(bots.map((bot) => [bot.id, bot]));
}

function templateById(templates: NotificationTemplateConfig[]) {
  return new Map(templates.map((template) => [template.id, template]));
}

function targetKindLabel(kind: NotificationBotConfig["kind"]) {
  return kind === "telegram" ? "Telegram" : "Webhook";
}

function defaultTemplates(): NotificationTemplateConfig[] {
  const timestamp = nowUnixSeconds();
  return Object.values(builtinTemplates).map((template) => ({
    id: template.id,
    name: template.label,
    preset: template.id.replace("builtin-", "") as NotificationTemplatePreset,
    messageTemplate: template.template,
    createdAt: timestamp,
    updatedAt: timestamp,
  })).map((template) => ({
    ...template,
    preset:
      template.id === "builtin-usage-report"
        ? "usageReport"
        : template.id === "builtin-quota-low"
          ? "quotaLow"
          : template.id === "builtin-quota-recovered"
            ? "quotaRecovered"
            : template.id === "builtin-account-error"
              ? "accountError"
              : "test",
  }));
}

function deriveBotsFromLegacyTargets(targets: NotificationTargetConfig[]) {
  return targets.map<NotificationBotConfig>((target) => ({
    id: `bot-${target.id}`,
    name: target.name,
    kind: target.kind,
    enabled: target.enabled,
    telegramBotToken: target.telegramBotToken,
    telegramChatId: target.telegramChatId,
    webhookUrl: target.webhookUrl,
    createdAt: target.createdAt,
    updatedAt: target.updatedAt,
    lastTestAt: target.lastTestAt,
    lastTestError: target.lastTestError,
  }));
}

function deriveTemplatesFromLegacyTargets(targets: NotificationTargetConfig[]) {
  return targets.map<NotificationTemplateConfig>((target) => ({
    id: `template-${target.id}`,
    name: `${target.name} 模板`,
    preset: target.templatePreset,
    messageTemplate: target.messageTemplate,
    createdAt: target.createdAt,
    updatedAt: target.updatedAt,
  }));
}

function derivePipelinesFromLegacyTargets(targets: NotificationTargetConfig[]) {
  return targets.map<NotificationPipelineConfig>((target) => ({
    id: `pipeline-${target.id}`,
    name: target.name,
    enabled: target.enabled,
    aggregateEnabled: target.aggregateEnabled,
    providerIds: target.providerIds,
    botIds: [`bot-${target.id}`],
    templateId: `template-${target.id}`,
    templateOverride: null,
    scheduleMode: normalizeScheduleMode(null, target.scheduleDate, target.scheduleTime, null),
    scheduleDate: target.scheduleDate,
    scheduleTime: target.scheduleTime,
    scheduleIntervalMinutes: null,
    createdAt: target.createdAt,
    updatedAt: target.updatedAt,
    lastRunAt: null,
    lastTestAt: target.lastTestAt,
    lastTestError: target.lastTestError,
  }));
}

function providerDraftFromConfig(provider: NotificationProviderConfig): ProviderDraft {
  return {
    name: provider.name,
    baseUrl: provider.baseUrl,
    email: provider.email,
    password: provider.password ?? "",
    costMultiplier: formatCostMultiplier(provider.costMultiplier),
  };
}

function botDraftFromConfig(bot: NotificationBotConfig): BotDraft {
  return {
    name: bot.name,
    kind: bot.kind,
    telegramBotToken: bot.telegramBotToken ?? "",
    telegramChatId: bot.telegramChatId ?? "",
    webhookUrl: bot.webhookUrl ?? "",
  };
}

function pipelineDraftFromConfig(
  pipeline: NotificationPipelineConfig,
  templates: NotificationTemplateConfig[],
): PipelineDraft {
  const templateId = pipeline.templateId && templates.some((template) => template.id === pipeline.templateId)
    ? pipeline.templateId
    : templates[0]?.id ?? "";
  const scheduleMode = normalizeScheduleMode(
    pipeline.scheduleMode,
    pipeline.scheduleDate,
    pipeline.scheduleTime,
    pipeline.scheduleIntervalMinutes,
  );

  return {
    id: pipeline.id,
    name: pipeline.name,
    providerIds: pipeline.providerIds,
    botIds: pipeline.botIds,
    templateId,
    templateOverrideEnabled: Boolean(pipeline.templateOverride?.trim()),
    templateOverride: pipeline.templateOverride ?? "",
    aggregateEnabled: pipeline.aggregateEnabled,
    scheduleMode,
    scheduleDate: pipeline.scheduleDate ?? "",
    scheduleTime: pipeline.scheduleTime ?? "09:00",
    scheduleIntervalMinutes: String(pipeline.scheduleIntervalMinutes ?? 30),
  };
}

function templateDraftFromConfig(template: NotificationTemplateConfig): TemplateDraft {
  return {
    id: template.id,
    name: template.name,
    preset: template.preset,
    messageTemplate: template.messageTemplate,
  };
}

function buildProviderFromDraft(
  draft: ProviderDraft,
  existing?: NotificationProviderConfig,
  testState?: DraftTestState,
): NotificationProviderConfig {
  const timestamp = nowUnixSeconds();
  const normalizedBaseUrl = normalizeBaseUrl(draft.baseUrl);
  const password = optionalValue(draft.password);
  const testStateStillValid = existing
    ? existing.baseUrl === normalizedBaseUrl &&
      existing.email === draft.email.trim() &&
      existing.password === password &&
      existing.costMultiplier === normalizeCostMultiplier(draft.costMultiplier)
    : false;
  const hasFreshDraftTest = Boolean(testState?.lastTestAt) && !testState?.dirty;
  return {
    id: existing?.id ?? createLocalId("provider"),
    name: draft.name.trim() || "数据源",
    kind: "sub2api",
    enabled: existing?.enabled ?? true,
    costMultiplier: normalizeCostMultiplier(draft.costMultiplier),
    baseUrl: normalizedBaseUrl,
    email: draft.email.trim(),
    password,
    createdAt: existing?.createdAt ?? timestamp,
    updatedAt: timestamp,
    lastTestAt: hasFreshDraftTest
      ? testState?.lastTestAt ?? null
      : testStateStillValid ? existing?.lastTestAt ?? null : null,
    lastTestError: hasFreshDraftTest
      ? testState?.lastTestError ?? null
      : testStateStillValid ? existing?.lastTestError ?? null : null,
  };
}

function buildBotFromDraft(
  draft: BotDraft,
  existing?: NotificationBotConfig,
  testState?: DraftTestState,
): NotificationBotConfig {
  const timestamp = nowUnixSeconds();
  const telegramBotToken = optionalValue(draft.telegramBotToken);
  const telegramChatId = optionalValue(draft.telegramChatId);
  const webhookUrl = optionalValue(draft.webhookUrl);
  const testStateStillValid = existing
    ? existing.kind === draft.kind &&
      existing.telegramBotToken === telegramBotToken &&
      existing.telegramChatId === telegramChatId &&
      existing.webhookUrl === webhookUrl
    : false;
  const hasFreshDraftTest = Boolean(testState?.lastTestAt) && !testState?.dirty;
  return {
    id: existing?.id ?? createLocalId("bot"),
    name: draft.name.trim() || (draft.kind === "telegram" ? "Telegram 机器人" : "Webhook"),
    kind: draft.kind,
    enabled: existing?.enabled ?? true,
    telegramBotToken,
    telegramChatId,
    webhookUrl,
    createdAt: existing?.createdAt ?? timestamp,
    updatedAt: timestamp,
    lastTestAt: hasFreshDraftTest
      ? testState?.lastTestAt ?? null
      : testStateStillValid ? existing?.lastTestAt ?? null : null,
    lastTestError: hasFreshDraftTest
      ? testState?.lastTestError ?? null
      : testStateStillValid ? existing?.lastTestError ?? null : null,
  };
}

function buildPipelineFromDraft(
  draft: PipelineDraft,
  existing?: NotificationPipelineConfig,
  testState?: DraftTestState,
): NotificationPipelineConfig {
  const timestamp = nowUnixSeconds();
  const scheduleMode = normalizeScheduleMode(draft.scheduleMode);
  const scheduleIntervalMinutes = scheduleMode === "interval"
    ? normalizeScheduleIntervalMinutes(draft.scheduleIntervalMinutes) ?? 30
    : null;
  const scheduleDate = scheduleMode === "date" ? optionalValue(draft.scheduleDate) : null;
  const scheduleTime = scheduleMode === "daily" || scheduleMode === "date"
    ? optionalValue(draft.scheduleTime)
    : null;
  const templateOverride = draft.templateOverrideEnabled
    ? optionalValue(draft.templateOverride)
    : null;

  return {
    id: existing?.id ?? (draft.id || createLocalId("pipeline")),
    name: draft.name.trim() || "通知规则",
    enabled: existing?.enabled ?? false,
    aggregateEnabled: draft.aggregateEnabled,
    providerIds: draft.providerIds,
    botIds: draft.botIds,
    templateId: draft.templateOverrideEnabled ? null : optionalValue(draft.templateId),
    templateOverride,
    scheduleMode,
    scheduleDate,
    scheduleTime,
    scheduleIntervalMinutes,
    createdAt: existing?.createdAt ?? timestamp,
    updatedAt: timestamp,
    lastRunAt: existing?.lastRunAt ?? null,
    lastTestAt: testState?.dirty
      ? testState.lastTestAt
      : testState?.lastTestAt ?? existing?.lastTestAt ?? null,
    lastTestError: testState?.dirty
      ? testState.lastTestError
      : testState?.lastTestError ?? existing?.lastTestError ?? null,
  };
}

function buildTemplateFromDraft(
  draft: TemplateDraft,
  existing?: NotificationTemplateConfig,
): NotificationTemplateConfig {
  const timestamp = nowUnixSeconds();
  return {
    id: existing?.id ?? (draft.id || createLocalId("template")),
    name: draft.name.trim() || "自定义模板",
    preset: draft.preset,
    messageTemplate: draft.messageTemplate.trim() || builtinTemplates[draft.preset].template,
    createdAt: existing?.createdAt ?? timestamp,
    updatedAt: timestamp,
  };
}

function botToLegacyTarget(bot: NotificationBotConfig): NotificationTargetConfig {
  return {
    id: bot.id,
    name: bot.name,
    kind: bot.kind,
    enabled: bot.enabled,
    aggregateEnabled: false,
    providerIds: [],
    templatePreset: "test",
    messageTemplate: builtinTemplates.test.template,
    scheduleDate: null,
    scheduleTime: null,
    telegramBotToken: bot.telegramBotToken,
    telegramChatId: bot.telegramChatId,
    webhookUrl: bot.webhookUrl,
    createdAt: bot.createdAt,
    updatedAt: bot.updatedAt,
    lastTestAt: bot.lastTestAt,
    lastTestError: bot.lastTestError,
  };
}

function pipelineToLegacyTarget(
  pipeline: NotificationPipelineConfig,
  bot: NotificationBotConfig,
  template: NotificationTemplateConfig | null,
): NotificationTargetConfig {
  return {
    ...botToLegacyTarget(bot),
    id: pipeline.id,
    name: pipeline.name,
    enabled: pipeline.enabled,
    aggregateEnabled: pipeline.aggregateEnabled,
    providerIds: pipeline.providerIds,
    templatePreset: template?.preset ?? "usageReport",
    messageTemplate: pipeline.templateOverride ?? template?.messageTemplate ?? builtinTemplates.usageReport.template,
    scheduleDate: pipeline.scheduleDate,
    scheduleTime: pipeline.scheduleTime,
    lastTestAt: pipeline.lastTestAt,
    lastTestError: pipeline.lastTestError,
  };
}

async function testProviderConnection(provider: NotificationProviderConfig) {
  if (isPreviewRuntime()) {
    return "数据源连接检查已完成。";
  }
  return invoke<string>("test_notification_provider", { provider });
}

async function testBotConnection(bot: NotificationBotConfig) {
  if (isPreviewRuntime()) {
    return "测试消息流程已完成。";
  }
  await invoke<void>("test_notification_target", { target: botToLegacyTarget(bot) });
  return "测试消息流程已完成。";
}

async function discoverTelegramChats(botToken: string) {
  if (isPreviewRuntime()) {
    return {
      botUsername: "codexdeck_demo_bot",
      chats: [
        {
          id: "123456789",
          title: "演示私聊",
          chatType: "private",
        },
      ],
    } satisfies TelegramChatDiscoveryResult;
  }

  return invoke<TelegramChatDiscoveryResult>("discover_telegram_chats", { botToken });
}

async function testPipelineConnection(
  pipeline: NotificationPipelineConfig,
  providers: NotificationProviderConfig[],
  bots: NotificationBotConfig[],
  templates: NotificationTemplateConfig[],
) {
  const selectedProviders = providers.filter((provider) => pipeline.providerIds.includes(provider.id));
  const selectedBots = bots.filter((bot) => pipeline.botIds.includes(bot.id));
  const template = pipeline.templateId
    ? templates.find((item) => item.id === pipeline.templateId) ?? null
    : null;

  if (selectedProviders.length === 0) {
    throw new Error("请选择至少一个数据源。");
  }
  if (selectedBots.length === 0) {
    throw new Error("请选择至少一个发送渠道。");
  }
  const unavailableProvider = selectedProviders.find((provider) => !isResourceReady(provider));
  if (unavailableProvider) {
    throw new Error(`数据源未测试通过或已停用：${unavailableProvider.name}`);
  }
  const unavailableBot = selectedBots.find((bot) => !isResourceReady(bot));
  if (unavailableBot) {
    throw new Error(`发送渠道未测试通过或已停用：${unavailableBot.name}`);
  }
  if (!pipeline.templateOverride && !template) {
    throw new Error("请选择模板，或为这条规则填写覆盖文案。");
  }

  if (isPreviewRuntime()) {
    return "规则测试流程已完成。";
  }

  for (const bot of selectedBots) {
    const target = pipelineToLegacyTarget(pipeline, bot, template);
    await invoke<void>("test_aggregate_notification", {
      target,
      providers: selectedProviders,
    });
  }

  return `规则测试流程已完成：已查询 ${selectedProviders.length} 个数据源，并发送到 ${selectedBots.length} 个发送渠道。`;
}

function mergeTemplates(
  savedTemplates: NotificationTemplateConfig[],
  legacyTargets: NotificationTargetConfig[],
) {
  const builtins = defaultTemplates();
  const legacyTemplates = savedTemplates.length > 0
    ? []
    : deriveTemplatesFromLegacyTargets(legacyTargets);
  const seen = new Set<string>();
  return [...builtins, ...legacyTemplates, ...savedTemplates].filter((template) => {
    if (seen.has(template.id)) {
      return false;
    }
    seen.add(template.id);
    return true;
  });
}

function ledgerPipelineStatus(
  pipeline: NotificationPipelineConfig,
  providers: NotificationProviderConfig[],
  bots: NotificationBotConfig[],
  templates: NotificationTemplateConfig[],
) {
  const readiness = pipelineReadiness(pipeline, providers, bots, templates);
  if (!readiness.ok) {
    if (pipeline.lastTestError) {
      return { label: "异常", tone: "danger" };
    }
    return { label: readiness.label, tone: readiness.tone };
  }
  if (!pipeline.enabled) {
    return { label: "已停用", tone: "muted" };
  }
  if (pipeline.lastRunAt || pipeline.lastTestAt) {
    return { label: "成功", tone: "ok" };
  }
  return { label: "待发送", tone: "draft" };
}

function pipelineReadiness(
  pipeline: NotificationPipelineConfig,
  providers: NotificationProviderConfig[],
  bots: NotificationBotConfig[],
  templates: NotificationTemplateConfig[],
) {
  const providersMap = providerById(providers);
  const botsMap = botById(bots);
  const templatesMap = templateById(templates);
  const selectedProviders = pipeline.providerIds.map((id) => providersMap.get(id));
  const selectedBots = pipeline.botIds.map((id) => botsMap.get(id));

  if (pipeline.providerIds.length === 0) {
    return { ok: false, label: "缺少数据源", tone: "draft", message: "请选择至少一个数据源。" };
  }
  if (selectedProviders.some((provider) => !provider)) {
    return { ok: false, label: "数据源缺失", tone: "danger", message: "规则引用的数据源已被删除。" };
  }
  const unavailableProvider = selectedProviders.find((provider) => provider && !isResourceReady(provider));
  if (unavailableProvider) {
    return {
      ok: false,
      label: "数据源不可用",
      tone: "danger",
      message: `数据源未测试通过或已停用：${unavailableProvider.name}`,
    };
  }

  if (pipeline.botIds.length === 0) {
    return { ok: false, label: "缺少渠道", tone: "draft", message: "请选择至少一个发送渠道。" };
  }
  if (selectedBots.some((bot) => !bot)) {
    return { ok: false, label: "目标缺失", tone: "danger", message: "规则引用的发送渠道已被删除。" };
  }
  const unavailableBot = selectedBots.find((bot) => bot && !isResourceReady(bot));
  if (unavailableBot) {
    return {
      ok: false,
      label: "目标不可用",
      tone: "danger",
      message: `发送渠道未测试通过或已停用：${unavailableBot.name}`,
    };
  }

  if (pipeline.templateId && !templatesMap.has(pipeline.templateId)) {
    return { ok: false, label: "模板缺失", tone: "danger", message: "规则引用的模板已被删除。" };
  }
  if (!pipeline.templateId && !pipeline.templateOverride) {
    return { ok: false, label: "缺少模板", tone: "draft", message: "请选择模板或填写规则覆盖文案。" };
  }
  if (!pipeline.lastTestAt) {
    return { ok: false, label: "待测试", tone: "draft", message: "请先测试规则成功。" };
  }
  if (pipeline.lastTestError) {
    return { ok: false, label: "测试失败", tone: "danger", message: `最近测试失败：${pipeline.lastTestError}` };
  }

  return { ok: true, label: "就绪", tone: "ok", message: "通知规则可启用。" };
}

function idListSummary<T extends { id: string; name: string }>(
  ids: string[],
  items: T[],
  emptyLabel: string,
) {
  if (ids.length === 0) {
    return emptyLabel;
  }
  const byId = new Map(items.map((item) => [item.id, item]));
  const names = ids.map((id) => byId.get(id)?.name ?? "已删除资源");
  if (names.length <= 2) {
    return names.join("、");
  }
  return `${names.slice(0, 2).join("、")} 等 ${names.length} 个`;
}

function targetRecipientSummary(
  bots: NotificationBotConfig[],
  ids: string[],
  emptyLabel: string,
) {
  if (ids.length === 0) {
    return emptyLabel;
  }
  const byId = new Map(bots.map((bot) => [bot.id, bot]));
  const names = ids.map((id) => {
    const bot = byId.get(id);
    if (!bot) {
      return "已删除目标";
    }
    return bot.telegramChatId ? `${bot.name} · ${bot.telegramChatId}` : bot.name;
  });
  if (names.length <= 2) {
    return names.join("、");
  }
  return `${names.slice(0, 2).join("、")} 等 ${names.length} 个`;
}

function selectedTemplateName(
  pipeline: NotificationPipelineConfig,
  templates: NotificationTemplateConfig[],
) {
  if (pipeline.templateId) {
    return templateById(templates).get(pipeline.templateId)?.name ?? "模板已删除";
  }
  return pipeline.templateOverride ? "规则覆盖模板" : "未选择";
}

function affectedPipelineNames(
  resourceId: string,
  kind: "provider" | "bot" | "template",
  pipelines: NotificationPipelineConfig[],
) {
  const affected = pipelines.filter((pipeline) => {
    if (kind === "provider") {
      return pipeline.providerIds.includes(resourceId);
    }
    if (kind === "bot") {
      return pipeline.botIds.includes(resourceId);
    }
    return pipeline.templateId === resourceId;
  });
  if (affected.length === 0) {
    return "当前没有通知规则引用它。";
  }
  const names = affected.slice(0, 3).map((pipeline) => pipeline.name).join("、");
  const suffix = affected.length > 3 ? ` 等 ${affected.length} 条规则` : "";
  return `将影响 ${affected.length} 条规则：${names}${suffix}。`;
}

function useNotificationResources(settings: AppSettings) {
  return useMemo(() => {
    const legacyTargets = settings.notificationTargets ?? [];
    const isNewSchema = (settings.notificationSchemaVersion ?? 0) > 0;
    const providers = settings.notificationProviders ?? [];
    const bots =
      isNewSchema || settings.notificationBots?.length > 0
        ? settings.notificationBots
        : deriveBotsFromLegacyTargets(legacyTargets);
    const templates = mergeTemplates(settings.notificationTemplates ?? [], legacyTargets);
    const pipelines =
      isNewSchema || settings.notificationPipelines?.length > 0
        ? settings.notificationPipelines
        : derivePipelinesFromLegacyTargets(legacyTargets);

    return { providers, bots, templates, pipelines };
  }, [settings]);
}

function StatusPill({ label, tone }: { label: string; tone: string }) {
  return <Tag className={`notificationStatusPill is-${tone}`}>{label}</Tag>;
}

function deliveryStatus(pipeline: NotificationPipelineConfig) {
  if (pipeline.lastTestError) {
    return { label: "推送失败", tone: "danger" };
  }
  return { label: "推送成功", tone: "ok" };
}

function ActionPlaceholder({
  label,
  onClick,
}: {
  label: string;
  onClick: () => void;
}) {
  return (
    <CsButton onClick={onClick}>
      {label}
    </CsButton>
  );
}

export function NotificationsPanel({
  settings,
  saving,
  viewTab,
  onViewTabChange,
  onUpdateSettings,
}: NotificationsPanelProps) {
  const [resourceTab, setResourceTab] = useState<NotificationResourceTab>("providers");
  const [panelNotice, setPanelNotice] = useState<string | null>(null);
  const [drawer, setDrawer] = useState<ResourceDrawer | null>(null);
  const [providerDraft, setProviderDraft] = useState<ProviderDraft>(defaultProviderDraft);
  const [botDraft, setBotDraft] = useState<BotDraft>(defaultBotDraft);
  const [pipelineDraft, setPipelineDraft] = useState<PipelineDraft>(defaultPipelineDraft);
  const [templateDraft, setTemplateDraft] = useState<TemplateDraft>(defaultTemplateDraft);
  const [draftTestState, setDraftTestState] = useState<DraftTestState>(defaultDraftTestState);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [pendingSwitchId, setPendingSwitchId] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<RowMenuState | null>(null);
  const [openRowMenu, setOpenRowMenu] = useState<RowMenuState | null>(null);
  const [telegramChats, setTelegramChats] = useState<TelegramChatCandidate[]>([]);
  const [discoveringChats, setDiscoveringChats] = useState(false);
  const [messageApi, messageContextHolder] = message.useMessage();
  const [modalApi, modalContextHolder] = Modal.useModal();
  const { providers, bots, templates, pipelines } = useNotificationResources(settings);

  const showTestingMessage = (key: string, content: string) => {
    void messageApi.open({ type: "loading", key, content, duration: 0 });
  };

  const showResultMessage = (
    type: "success" | "error" | "warning" | "info",
    key: string,
    content: string,
  ) => {
    void messageApi.open({
      type,
      key,
      content,
      duration: type === "success" ? 2.4 : 4,
    });
  };

  const openResourceManager = (tab: NotificationResourceTab) => {
    setResourceTab(tab);
    onViewTabChange(tab === "templates" ? "templates" : "tests");
    setPanelNotice(null);
    setOpenRowMenu(null);
    setPendingDelete(null);
  };

  const openProviderDrawer = (provider?: NotificationProviderConfig) => {
    setPanelNotice(null);
    setPendingDelete(null);
    setOpenRowMenu(null);
    setDraftTestState(
      provider
        ? { lastTestAt: provider.lastTestAt, lastTestError: provider.lastTestError, dirty: false }
        : defaultDraftTestState,
    );
    setProviderDraft(provider ? providerDraftFromConfig(provider) : defaultProviderDraft);
    setDrawer(provider ? { kind: "provider", mode: "edit", id: provider.id } : { kind: "provider", mode: "create" });
  };

  const openBotDrawer = (bot?: NotificationBotConfig) => {
    setPanelNotice(null);
    setPendingDelete(null);
    setOpenRowMenu(null);
    setTelegramChats([]);
    setDraftTestState(
      bot
        ? { lastTestAt: bot.lastTestAt, lastTestError: bot.lastTestError, dirty: false }
        : defaultDraftTestState,
    );
    setBotDraft(bot ? botDraftFromConfig(bot) : defaultBotDraft);
    setDrawer(bot ? { kind: "bot", mode: "edit", id: bot.id } : { kind: "bot", mode: "create" });
  };

  const closeDrawer = () => {
    setDrawer(null);
    setTelegramChats([]);
    setDiscoveringChats(false);
    setDraftTestState(defaultDraftTestState);
    setOpenRowMenu(null);
  };

  useEffect(() => {
    if (!drawer) {
      return;
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setDrawer(null);
        setTelegramChats([]);
        setDiscoveringChats(false);
        setDraftTestState(defaultDraftTestState);
        setOpenRowMenu(null);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [drawer]);

  const defaultPipelineDraftWithResources = () => ({
    ...defaultPipelineDraft,
    id: createLocalId("pipeline-draft"),
    providerIds: providers.filter(isResourceReady).map((provider) => provider.id),
    botIds: bots.filter(isResourceReady).slice(0, 1).map((bot) => bot.id),
    templateId: templates.find((template) => template.id === "builtin-usage-report")?.id ?? templates[0]?.id ?? "",
  });

  const openPipelineDrawer = (pipeline?: NotificationPipelineConfig) => {
    setPanelNotice(null);
    setPendingDelete(null);
    setOpenRowMenu(null);
    setDraftTestState(
      pipeline
        ? { lastTestAt: pipeline.lastTestAt, lastTestError: pipeline.lastTestError, dirty: false }
        : defaultDraftTestState,
    );
    setPipelineDraft(pipeline ? pipelineDraftFromConfig(pipeline, templates) : defaultPipelineDraftWithResources());
    setDrawer(
      pipeline
        ? { kind: "pipeline", mode: "edit", id: pipeline.id }
        : { kind: "pipeline", mode: "create" },
    );
  };

  const openTemplateDrawer = (template?: NotificationTemplateConfig, copy = false) => {
    setPanelNotice(null);
    setPendingDelete(null);
    setOpenRowMenu(null);
    if (template) {
      const draft = templateDraftFromConfig(template);
      setTemplateDraft(
        copy
          ? {
              ...draft,
              id: createLocalId("template"),
              name: `${template.name} 副本`,
            }
          : draft,
      );
    } else {
      setTemplateDraft({
        ...defaultTemplateDraft,
        id: createLocalId("template"),
        messageTemplate: builtinTemplates.usageReport.template,
      });
    }
    setDrawer(template && !copy ? { kind: "template", mode: "edit", id: template.id } : { kind: "template", mode: "create" });
  };

  const updatePipelineDraft = (draft: PipelineDraft) => {
    setPipelineDraft(draft);
    setDraftTestState({ lastTestAt: null, lastTestError: null, dirty: true });
  };

  const updateProviderDraft = (draft: ProviderDraft) => {
    setProviderDraft(draft);
    setDraftTestState({ lastTestAt: null, lastTestError: null, dirty: true });
  };

  const updateBotDraft = (draft: BotDraft) => {
    setBotDraft(draft);
    setDraftTestState({ lastTestAt: null, lastTestError: null, dirty: true });
  };

  const persistProviders = async (
    notificationProviders: NotificationProviderConfig[],
    options?: UpdateSettingsOptions,
  ) => {
    await onUpdateSettings(
      { notificationProviders, notificationSchemaVersion: 1 },
      options ?? { silent: true, keepInteractive: true },
    );
  };

  const persistBots = async (
    notificationBots: NotificationBotConfig[],
    options?: UpdateSettingsOptions,
  ) => {
    await onUpdateSettings(
      { notificationBots, notificationSchemaVersion: 1 },
      options ?? { silent: true, keepInteractive: true },
    );
  };

  const persistPipelines = async (
    notificationPipelines: NotificationPipelineConfig[],
    options?: UpdateSettingsOptions,
  ) => {
    await onUpdateSettings(
      { notificationPipelines, notificationSchemaVersion: 1 },
      options ?? { silent: true, keepInteractive: true },
    );
  };

  const persistTemplates = async (
    notificationTemplates: NotificationTemplateConfig[],
    options?: UpdateSettingsOptions,
  ) => {
    const customTemplates = notificationTemplates.filter((template) => !template.id.startsWith("builtin-"));
    await onUpdateSettings(
      { notificationTemplates: customTemplates, notificationSchemaVersion: 1 },
      options ?? { silent: true, keepInteractive: true },
    );
  };

  const saveProviderDraft = async () => {
    const existing =
      drawer?.kind === "provider" && drawer.mode === "edit"
        ? providers.find((provider) => provider.id === drawer.id)
        : undefined;
    const provider = buildProviderFromDraft(providerDraft, existing, draftTestState);
    if (!provider.baseUrl || !provider.email) {
      setPanelNotice("请至少填写数据源 URL 和登录账号。");
      return;
    }

    const nextProviders = existing
      ? providers.map((item) => (item.id === provider.id ? provider : item))
      : [...providers, provider];
    await persistProviders(nextProviders);
    setResourceTab("providers");
    onViewTabChange("tests");
    setPanelNotice(null);
    closeDrawer();
  };

  const testProviderDraft = async () => {
    const existing =
      drawer?.kind === "provider" && drawer.mode === "edit"
        ? providers.find((provider) => provider.id === drawer.id)
        : undefined;
    const provider = buildProviderFromDraft(providerDraft, existing, draftTestState);
    if (!provider.baseUrl || !provider.email) {
      setPanelNotice("请至少填写数据源 URL 和登录账号。");
      return;
    }

    const messageKey = `provider-draft-test-${provider.id}`;
    setTestingId(provider.id);
    setPanelNotice(null);
    showTestingMessage(messageKey, "正在测试数据源登录和用量接口...");
    try {
      await testProviderConnection(provider);
      const timestamp = nowUnixSeconds();
      setDraftTestState({ lastTestAt: timestamp, lastTestError: null, dirty: false });
      showResultMessage("success", messageKey, "数据源测试完成，可以用于通知规则。");
    } catch (error) {
      const timestamp = nowUnixSeconds();
      const errorMessage = String(error);
      setDraftTestState({ lastTestAt: timestamp, lastTestError: errorMessage, dirty: true });
      showResultMessage("error", messageKey, `数据源测试失败：${errorMessage}`);
    } finally {
      setTestingId(null);
    }
  };

  const saveBotDraft = async () => {
    const existing =
      drawer?.kind === "bot" && drawer.mode === "edit"
        ? bots.find((bot) => bot.id === drawer.id)
        : undefined;
    const bot = buildBotFromDraft(botDraft, existing, draftTestState);
    const missingTelegram = bot.kind === "telegram" && (!bot.telegramBotToken || !bot.telegramChatId);
    const missingWebhook = bot.kind === "webhook" && !bot.webhookUrl;
    if (missingTelegram || missingWebhook) {
      setPanelNotice(bot.kind === "telegram" ? "请填写 Bot Token 和 Chat ID。" : "请填写 Webhook URL。");
      return;
    }

    const nextBots = existing
      ? bots.map((item) => (item.id === bot.id ? bot : item))
      : [...bots, bot];
    await persistBots(nextBots);
    setResourceTab("bots");
    onViewTabChange("tests");
    setPanelNotice(null);
    closeDrawer();
  };

  const testBotDraft = async () => {
    const existing =
      drawer?.kind === "bot" && drawer.mode === "edit"
        ? bots.find((bot) => bot.id === drawer.id)
        : undefined;
    const bot = buildBotFromDraft(botDraft, existing, draftTestState);
    const missingTelegram = bot.kind === "telegram" && (!bot.telegramBotToken || !bot.telegramChatId);
    const missingWebhook = bot.kind === "webhook" && !bot.webhookUrl;
    if (missingTelegram || missingWebhook) {
      setPanelNotice(bot.kind === "telegram" ? "请填写 Bot Token 和 Chat ID。" : "请填写 Webhook URL。");
      return;
    }

    const messageKey = `bot-draft-test-${bot.id}`;
    setTestingId(bot.id);
    setPanelNotice(null);
    showTestingMessage(messageKey, "正在发送测试消息...");
    try {
      await testBotConnection(bot);
      const timestamp = nowUnixSeconds();
      setDraftTestState({ lastTestAt: timestamp, lastTestError: null, dirty: false });
      showResultMessage("success", messageKey, "测试消息已发送，发送渠道可用。");
    } catch (error) {
      const timestamp = nowUnixSeconds();
      const errorMessage = String(error);
      setDraftTestState({ lastTestAt: timestamp, lastTestError: errorMessage, dirty: true });
      showResultMessage("error", messageKey, `发送渠道测试失败：${errorMessage}`);
    } finally {
      setTestingId(null);
    }
  };

  const confirmSaveFailedPipelineDraft = (errorMessage: string) =>
    new Promise<boolean>((resolve) => {
      modalApi.confirm({
        title: "规则检测未通过",
        content: (
          <div className="notificationConfirmBody">
            <p>保存并启用前的自动检测失败，这条规则暂时不适合直接启用。</p>
            <p className="notificationConfirmError">{errorMessage}</p>
            <p>你可以返回修改，也可以先保存为草稿，稍后再处理。</p>
          </div>
        ),
        okText: "保存草稿",
        cancelText: "返回修改",
        onOk: () => resolve(true),
        onCancel: () => resolve(false),
      });
    });

  const validatePipelineBasics = (pipeline: NotificationPipelineConfig) => {
    if (pipeline.providerIds.length === 0 || pipeline.botIds.length === 0) {
      return "规则至少需要选择一个数据源和一个发送渠道。";
    }
    if (!pipeline.templateId && !pipeline.templateOverride) {
      return "规则至少需要选择一个模板，或填写规则覆盖文案。";
    }
    if (pipeline.scheduleMode === "date" && !pipeline.scheduleDate) {
      return "指定日期模式需要填写推送日期。";
    }
    if (pipeline.scheduleMode === "interval" && !pipeline.scheduleIntervalMinutes) {
      return "间隔推送需要填写推送间隔。";
    }
    return null;
  };

  const validatePipelineForSend = (pipeline: NotificationPipelineConfig) => {
    return validatePipelineBasics(pipeline);
  };

  const testPipelineDraft = async () => {
    const existing =
      drawer?.kind === "pipeline" && drawer.mode === "edit"
        ? pipelines.find((pipeline) => pipeline.id === drawer.id)
        : undefined;
    const pipeline = buildPipelineFromDraft(pipelineDraft, existing, draftTestState);
    const validationMessage = validatePipelineForSend(pipeline);
    if (validationMessage) {
      setPanelNotice(validationMessage);
      return;
    }

    const messageKey = `pipeline-draft-test-${pipeline.id}`;
    setTestingId(pipeline.id);
    setPanelNotice(null);
    showTestingMessage(messageKey, "正在发送测试消息...");
    try {
      await testPipelineConnection(pipeline, providers, bots, templates);
      const timestamp = nowUnixSeconds();
      setDraftTestState({ lastTestAt: timestamp, lastTestError: null, dirty: false });
      showResultMessage("success", messageKey, "测试消息已发送，当前规则配置可用。");
    } catch (error) {
      const timestamp = nowUnixSeconds();
      const errorMessage = String(error);
      setDraftTestState({ lastTestAt: timestamp, lastTestError: errorMessage, dirty: true });
      showResultMessage("error", messageKey, `规则测试失败：${errorMessage}`);
    } finally {
      setTestingId(null);
    }
  };

  const savePipelineDraft = async (enableAfterSave = false) => {
    const existing =
      drawer?.kind === "pipeline" && drawer.mode === "edit"
        ? pipelines.find((pipeline) => pipeline.id === drawer.id)
        : undefined;
    const pipeline = buildPipelineFromDraft(pipelineDraft, existing, draftTestState);
    const validationMessage = enableAfterSave
      ? validatePipelineForSend(pipeline)
      : validatePipelineBasics(pipeline);
    if (validationMessage) {
      setPanelNotice(validationMessage);
      return;
    }

    const persistPipelineDraft = async (
      nextPipeline: NotificationPipelineConfig,
      successMessage: string,
      messageKey = `pipeline-save-${nextPipeline.id}`,
    ) => {
      const nextPipelines = existing
        ? pipelines.map((item) => (item.id === nextPipeline.id ? nextPipeline : item))
        : [...pipelines, nextPipeline];
      await persistPipelines(nextPipelines);
      onViewTabChange("pipelines");
      setPanelNotice(null);
      showResultMessage("success", messageKey, successMessage);
      closeDrawer();
    };

    if (!enableAfterSave) {
      await persistPipelineDraft(
        {
          ...pipeline,
          enabled: draftTestState.dirty ? false : pipeline.enabled,
        },
        "通知规则草稿已保存。",
      );
      return;
    }

    const messageKey = `pipeline-auto-test-${pipeline.id}`;
    setTestingId(pipeline.id);
    setPanelNotice(null);
    showTestingMessage(messageKey, "正在自动检测通知规则...");
    try {
      await testPipelineConnection(pipeline, providers, bots, templates);
      const timestamp = nowUnixSeconds();
      const testedPipeline = {
        ...pipeline,
        enabled: true,
        updatedAt: timestamp,
        lastTestAt: timestamp,
        lastTestError: null,
      };
      setDraftTestState({ lastTestAt: timestamp, lastTestError: null, dirty: true });
      await persistPipelineDraft(testedPipeline, "规则检测通过，已保存并启用。", messageKey);
    } catch (error) {
      const timestamp = nowUnixSeconds();
      const errorMessage = String(error);
      const failedPipeline = {
        ...pipeline,
        enabled: false,
        updatedAt: timestamp,
        lastTestAt: timestamp,
        lastTestError: errorMessage,
      };
      setDraftTestState({ lastTestAt: timestamp, lastTestError: errorMessage, dirty: true });
      showResultMessage("error", messageKey, `规则检测失败：${errorMessage}`);
      setTestingId(null);
      const shouldSaveDraft = await confirmSaveFailedPipelineDraft(errorMessage);
      if (shouldSaveDraft) {
        await persistPipelineDraft(failedPipeline, "检测未通过，已保存为草稿。");
      }
      return;
    } finally {
      setTestingId(null);
    }
  };

  const saveTemplateDraft = async () => {
    const existing =
      drawer?.kind === "template" && drawer.mode === "edit"
        ? templates.find((template) => template.id === drawer.id && !template.id.startsWith("builtin-"))
        : undefined;
    const template = buildTemplateFromDraft(templateDraft, existing);
    if (!template.name.trim() || !template.messageTemplate.trim()) {
      setPanelNotice("请填写模板名称和模板内容。");
      return;
    }

    const customTemplates = templates.filter((item) => !item.id.startsWith("builtin-"));
    const nextTemplates = existing
      ? customTemplates.map((item) => (item.id === template.id ? template : item))
      : [...customTemplates, template];
    await persistTemplates(nextTemplates);
    setResourceTab("templates");
    onViewTabChange("templates");
    setPanelNotice(null);
    closeDrawer();
  };

  const togglePipelineEnabled = async (pipeline: NotificationPipelineConfig) => {
    const readiness = pipelineReadiness(pipeline, providers, bots, templates);
    if (!pipeline.enabled && !readiness.ok) {
      setPanelNotice(readiness.message);
      return;
    }
    const messageKey = `pipeline-toggle-${pipeline.id}`;
    setPendingSwitchId(pipeline.id);
    setPanelNotice(null);
    try {
      const nextEnabled = !pipeline.enabled;
      await persistPipelines(
        pipelines.map((item) =>
          item.id === pipeline.id ? { ...item, enabled: nextEnabled, updatedAt: nowUnixSeconds() } : item,
        ),
      );
      showResultMessage("success", messageKey, nextEnabled ? "通知规则已启用。" : "通知规则已停用。");
    } catch (error) {
      const errorMessage = String(error);
      setPanelNotice(`通知规则状态更新失败：${errorMessage}`);
      showResultMessage("error", messageKey, `状态更新失败：${errorMessage}`);
    } finally {
      setPendingSwitchId(null);
    }
  };

  const testPipeline = async (pipeline: NotificationPipelineConfig) => {
    const validationMessage = validatePipelineForSend(pipeline);
    if (validationMessage) {
      setPanelNotice(validationMessage);
      return;
    }

    const messageKey = `pipeline-test-${pipeline.id}`;
    setTestingId(pipeline.id);
    setPanelNotice(null);
    showTestingMessage(messageKey, "正在发送通知规则测试...");
    try {
      await testPipelineConnection(pipeline, providers, bots, templates);
      const timestamp = nowUnixSeconds();
      await persistPipelines(
        pipelines.map((item) =>
          item.id === pipeline.id
            ? { ...item, updatedAt: timestamp, lastTestAt: timestamp, lastTestError: null }
            : item,
        ),
      );
      showResultMessage("success", messageKey, "测试消息已发送，通知规则可用。");
    } catch (error) {
      const timestamp = nowUnixSeconds();
      const errorMessage = String(error);
      await persistPipelines(
        pipelines.map((item) =>
          item.id === pipeline.id
            ? { ...item, updatedAt: timestamp, lastTestAt: timestamp, lastTestError: errorMessage }
            : item,
        ),
      );
      showResultMessage("error", messageKey, `通知规则测试失败：${errorMessage}`);
    } finally {
      setTestingId(null);
    }
  };

  const deletePipeline = (pipeline: NotificationPipelineConfig) => {
    modalApi.confirm({
      title: "删除通知规则？",
      content: (
        <div className="notificationConfirmBody">
          <p>删除后这条规则不会再触发，也不会继续出现在发送记录里。</p>
          <p className="notificationConfirmError">{pipeline.name}</p>
        </div>
      ),
      okText: "删除",
      okButtonProps: { danger: true },
      cancelText: "取消",
      async onOk() {
        await persistPipelines(pipelines.filter((item) => item.id !== pipeline.id));
        setPanelNotice(null);
        showResultMessage("success", `pipeline-delete-${pipeline.id}`, "通知规则已删除。");
      },
    });
  };

  const deleteProvider = async (provider: NotificationProviderConfig) => {
    const affected = affectedPipelineNames(provider.id, "provider", pipelines);
    if (pendingDelete?.kind !== "provider" || pendingDelete.id !== provider.id) {
      setPendingDelete({ kind: "provider", id: provider.id });
      setPanelNotice(
        `再次点击“删除”以确认删除数据源：${provider.name}。${affected}`,
      );
      return;
    }
    await persistProviders(providers.filter((item) => item.id !== provider.id));
    setPendingDelete(null);
    setOpenRowMenu(null);
    setPanelNotice(null);
  };

  const deleteBot = async (bot: NotificationBotConfig) => {
    const affected = affectedPipelineNames(bot.id, "bot", pipelines);
    if (pendingDelete?.kind !== "bot" || pendingDelete.id !== bot.id) {
      setPendingDelete({ kind: "bot", id: bot.id });
      setPanelNotice(`再次点击“删除”以确认删除机器人：${bot.name}。${affected}`);
      return;
    }
    await persistBots(bots.filter((item) => item.id !== bot.id));
    setPendingDelete(null);
    setOpenRowMenu(null);
    setPanelNotice(null);
  };

  const deleteTemplate = async (template: NotificationTemplateConfig) => {
    if (template.id.startsWith("builtin-")) {
      setPanelNotice("内置模板不能删除，可以复制后编辑自己的版本。");
      return;
    }
    const affected = affectedPipelineNames(template.id, "template", pipelines);
    if (pendingDelete?.kind !== "template" || pendingDelete.id !== template.id) {
      setPendingDelete({ kind: "template", id: template.id });
      setPanelNotice(`再次点击“删除”以确认删除模板：${template.name}。${affected}`);
      return;
    }
    await persistTemplates(templates.filter((item) => item.id !== template.id));
    setPendingDelete(null);
    setOpenRowMenu(null);
    setPanelNotice(null);
  };

  const discoverChats = async () => {
    const token = botDraft.telegramBotToken.trim();
    const messageKey = "telegram-chat-discovery";
    if (!token) {
      showResultMessage("warning", messageKey, "请先填写 Telegram Bot Token。");
      return;
    }
    setDiscoveringChats(true);
    setTelegramChats([]);
    setPanelNotice(null);
    showTestingMessage(messageKey, "正在验证 Bot Token 并读取最近会话...");
    try {
      const result = await discoverTelegramChats(token);
      if (result.chats.length === 1) {
        setBotDraft((current) => ({ ...current, telegramChatId: result.chats[0].id }));
        showResultMessage("success", messageKey, "Chat ID 已发现并填入。");
      } else if (result.chats.length > 1) {
        setTelegramChats(result.chats);
        showResultMessage("info", messageKey, `已验证 @${result.botUsername ?? "bot"}，找到 ${result.chats.length} 个会话，请选择一个。`);
      } else {
        showResultMessage("warning", messageKey, "Bot Token 有效，但没有读取到最近会话。请先给机器人发一条消息，或把机器人加入群组/频道后再试。");
      }
    } catch (error) {
      showResultMessage("error", messageKey, `自动获取 Chat ID 失败：${String(error)}`);
    } finally {
      setDiscoveringChats(false);
    }
  };

  const commonDrawer = drawer ? (
    drawer.kind === "pipeline" ? (
      <PipelineDrawerPanel
        drawer={drawer}
        draft={pipelineDraft}
        draftTestState={draftTestState}
        providers={providers}
        bots={bots}
        templates={templates}
        saving={saving}
        testing={testingId === (pipelineDraft.id || drawer.id)}
        onDraftChange={updatePipelineDraft}
        onTest={() => void testPipelineDraft()}
        onSaveDraft={() => void savePipelineDraft(false)}
        onSaveEnabled={() => void savePipelineDraft(true)}
        onClose={closeDrawer}
      />
    ) : drawer.kind === "template" ? (
      <TemplateDrawerPanel
        drawer={drawer}
        draft={templateDraft}
        providers={providers}
        saving={saving}
        onDraftChange={setTemplateDraft}
        onSave={() => void saveTemplateDraft()}
        onClose={closeDrawer}
      />
    ) : (
      <ResourceDrawerPanel
        drawer={drawer}
        providerDraft={providerDraft}
        botDraft={botDraft}
        draftTestState={draftTestState}
        saving={saving}
        testing={Boolean(testingId)}
        discoveringChats={discoveringChats}
        telegramChats={telegramChats}
        onProviderDraftChange={updateProviderDraft}
        onBotDraftChange={updateBotDraft}
        onDiscoverChats={() => void discoverChats()}
        onSelectChat={(chatId) => {
          updateBotDraft({ ...botDraft, telegramChatId: chatId });
          setTelegramChats([]);
          setPanelNotice(null);
        }}
        onTest={() => void (drawer.kind === "provider" ? testProviderDraft() : testBotDraft())}
        onSave={() => void (drawer.kind === "provider" ? saveProviderDraft() : saveBotDraft())}
        onClose={closeDrawer}
      />
    )
  ) : null;

  return (
    <section className="notificationCenter" aria-label="通知中心">
      {messageContextHolder}
      {modalContextHolder}
      <div className="notificationPageHeader">
        <div>
          <h2>通知中心</h2>
          <p>配置数据源、发送渠道和通知规则，并查看每一次发送结果。</p>
        </div>
        <div className="notificationHeaderActions">
          <span className="notificationSaveState">{saving ? "保存中" : "自动保存"}</span>
        </div>
      </div>

      {panelNotice ? (
        <div className="notificationInlineNotice">
          <span>{panelNotice}</span>
          <CsButton className="notificationCompactButton" onClick={() => setPanelNotice(null)}>
            知道了
          </CsButton>
        </div>
      ) : null}

      {viewTab === "settings" ? (
        <NotificationSettingsHome
          providers={providers}
          bots={bots}
          pipelines={pipelines}
          onOpenRules={() => onViewTabChange("pipelines")}
          onOpenChannels={() => openResourceManager("bots")}
          onOpenActivity={() => onViewTabChange("activity")}
        />
      ) : viewTab === "pipelines" ? (
        <NotificationLedgerPage
          pipelines={pipelines}
          providers={providers}
          bots={bots}
          templates={templates}
          saving={saving}
          testingId={testingId}
          pendingSwitchId={pendingSwitchId}
          onCreatePipeline={() => openPipelineDrawer()}
          onTogglePipeline={(pipeline) => void togglePipelineEnabled(pipeline)}
          onEditPipeline={openPipelineDrawer}
          onTestPipeline={(pipeline) => void testPipeline(pipeline)}
          onDeletePipeline={deletePipeline}
          onCreateProvider={() => openProviderDrawer()}
          onCreateBot={() => openBotDrawer()}
        />
      ) : viewTab === "templates" ? (
        <TemplateList
          templates={templates}
          saving={saving}
          pendingDelete={pendingDelete}
          openRowMenu={openRowMenu}
          onCreate={() => openTemplateDrawer()}
          onEdit={(template) => openTemplateDrawer(template)}
          onCopy={(template) => openTemplateDrawer(template, true)}
          onDelete={(template) => void deleteTemplate(template)}
          onToggleMenu={(state) => setOpenRowMenu(state)}
        />
      ) : viewTab === "tests" ? (
        <section className="notificationResourcePanel notificationTestBenchPanel" aria-label="发送渠道">
          <div className="notificationResourcePanelHeader notificationTestBenchHeader">
            <div className="notificationTestBenchCopy">
              <p className="proxy-kicker">DELIVERY SETUP</p>
              <h3>发送渠道</h3>
              <p>先接入数据源，再配置真正负责发送消息的机器人或 Webhook。</p>
            </div>
            <div className="notificationTabs notificationTestTabs" role="tablist" aria-label="资源分类">
              {[
                ["providers", "数据源", providers.length],
                ["bots", "发送渠道", bots.length],
              ].map(([tab, label, count]) => (
                <button
                  type="button"
                  key={tab}
                  role="tab"
                  aria-selected={resourceTab === tab}
                  className={`notificationTab${resourceTab === tab ? " is-active" : ""}`}
                  onClick={() => setResourceTab(tab as NotificationResourceTab)}
                >
                  {label}
                  <span>{count}</span>
                </button>
              ))}
            </div>
            <div className="notificationToolbarActions notificationTestActions">
              <ActionPlaceholder label="添加数据源" onClick={() => openProviderDrawer()} />
              <ActionPlaceholder label="添加发送渠道" onClick={() => openBotDrawer()} />
            </div>
          </div>

          {resourceTab === "providers" ? (
            <ProviderList
              providers={providers}
              saving={saving}
              pendingDelete={pendingDelete}
              openRowMenu={openRowMenu}
          onCreate={() => openProviderDrawer()}
          onEdit={openProviderDrawer}
          onDelete={(provider) => void deleteProvider(provider)}
          onToggleMenu={(state) => setOpenRowMenu(state)}
        />
          ) : (
            <BotList
              bots={bots}
              saving={saving}
              pendingDelete={pendingDelete}
              openRowMenu={openRowMenu}
          onCreate={() => openBotDrawer()}
          onEdit={openBotDrawer}
          onDelete={(bot) => void deleteBot(bot)}
          onToggleMenu={(state) => setOpenRowMenu(state)}
        />
          )}
          <div className="notificationChannelNextStep">
            <div>
              <h4>资源准备完成后</h4>
              <p>数据源和发送渠道都可用后，再把它们组合成一条通知规则。</p>
            </div>
            <CsButton tone="primary" onClick={() => openPipelineDrawer()}>
              新建通知规则
            </CsButton>
          </div>
        </section>
      ) : (
        <NotificationActivityPage
          pipelines={pipelines}
          bots={bots}
          onOpenPipelines={() => onViewTabChange("pipelines")}
        />
      )}

      {commonDrawer}
    </section>
  );
}

function NotificationSettingsHome({
  providers,
  bots,
  pipelines,
  onOpenRules,
  onOpenChannels,
  onOpenActivity,
}: {
  providers: NotificationProviderConfig[];
  bots: NotificationBotConfig[];
  pipelines: NotificationPipelineConfig[];
  onOpenRules: () => void;
  onOpenChannels: () => void;
  onOpenActivity: () => void;
}) {
  const readyProviderCount = providers.filter(isResourceReady).length;
  const readyBotCount = bots.filter(isResourceReady).length;
  const enabledPipelineCount = pipelines.filter((pipeline) => pipeline.enabled).length;
  const primaryBot = bots[0] ?? null;
  const activityRows = pipelines
    .flatMap((pipeline) => {
      const at = pipeline.lastRunAt ?? pipeline.lastTestAt;
      const rows = at
        ? [{
          id: `${pipeline.id}-run`,
          at,
          label: pipeline.lastRunAt ? "发送记录" : "测试记录",
          title: pipeline.name,
          detail: pipeline.lastTestError
            ? pipeline.lastTestError
            : targetRecipientSummary(bots, pipeline.botIds, "未选择接收人"),
          status: deliveryStatus(pipeline),
        }]
        : [];

      if (pipeline.lastTestError) {
        rows.push({
          id: `${pipeline.id}-error`,
          at: pipeline.lastTestAt ?? pipeline.lastRunAt ?? 0,
          label: "错误",
          title: pipeline.name,
          detail: pipeline.lastTestError,
          status: { label: "推送失败", tone: "danger" },
        });
      }

      return rows;
    })
    .sort((left, right) => {
      if (left.status.tone === "danger" && right.status.tone !== "danger") {
        return -1;
      }
      if (right.status.tone === "danger" && left.status.tone !== "danger") {
        return 1;
      }
      return (right.at ?? 0) - (left.at ?? 0);
    })
    .slice(0, 10);

  return (
    <div className="notificationSettingsPage">
      <div className="notificationStatusGrid">
        <section className="notificationStatusCard">
          <div>
            <span className="notificationCardIcon">✓</span>
            <h3>已启用规则</h3>
          </div>
          <strong>{enabledPipelineCount} 条</strong>
          <p>{enabledPipelineCount > 0 ? "这些规则会按计划或手动触发发送。" : "还没有可用规则，先创建一条通知规则。"}</p>
          <CsButton onClick={onOpenRules}>
            管理规则
          </CsButton>
        </section>
        <section className="notificationStatusCard">
          <div>
            <span className="notificationCardIcon">◎</span>
            <h3>可用数据源</h3>
          </div>
          <strong>{readyProviderCount} 个</strong>
          <p>{readyProviderCount > 0 ? "数据源负责读取额度、开销和模型用量。" : "先添加并测试 Sub2API 数据源。"}</p>
          <CsButton onClick={onOpenChannels}>
            管理数据源
          </CsButton>
        </section>
        <section className="notificationStatusCard">
          <div>
            <span className="notificationCardIcon">✈</span>
            <h3>发送渠道</h3>
          </div>
          <strong>{readyBotCount} 个</strong>
          <p>{primaryBot ? `${primaryBot.name} · ${resourceStatus(primaryBot).label}` : "添加 Telegram 或 Webhook 后才能触达消息。"}</p>
          <CsButton onClick={onOpenChannels}>
            管理渠道
          </CsButton>
        </section>
      </div>

      <div className="notificationHomeGrid">
        <section className="notificationActivityCard">
          <div className="notificationSectionTitle">
            <div>
              <h3>最近发送记录</h3>
              <p>展示最近发送、测试和异常；完整历史在发送记录中查看。</p>
            </div>
            <CsButton className="notificationCompactButton" onClick={onOpenActivity}>
              查看全部
            </CsButton>
          </div>
          {activityRows.length > 0 ? (
            <div className="notificationActivityList">
              {activityRows.map((row) => (
                <div className="notificationActivityItem" key={row.id}>
                  <span className="notificationLogTime">{formatDateTime(row.at)}</span>
                  <strong>{row.title}</strong>
                  <p>{row.label} · {row.detail}</p>
                  <StatusPill {...row.status} />
                </div>
              ))}
            </div>
          ) : (
            <div className="notificationSoftEmpty">还没有发送记录。创建通知规则并发送测试后会显示在这里。</div>
          )}
        </section>
      </div>

    </div>
  );
}

function NotificationLedgerPage({
  pipelines,
  providers,
  bots,
  templates,
  saving,
  testingId,
  pendingSwitchId,
  onCreatePipeline,
  onTogglePipeline,
  onEditPipeline,
  onTestPipeline,
  onDeletePipeline,
  onCreateProvider,
  onCreateBot,
}: {
  pipelines: NotificationPipelineConfig[];
  providers: NotificationProviderConfig[];
  bots: NotificationBotConfig[];
  templates: NotificationTemplateConfig[];
  saving: boolean;
  testingId: string | null;
  pendingSwitchId: string | null;
  onCreatePipeline: () => void;
  onTogglePipeline: (pipeline: NotificationPipelineConfig) => void;
  onEditPipeline: (pipeline: NotificationPipelineConfig) => void;
  onTestPipeline: (pipeline: NotificationPipelineConfig) => void;
  onDeletePipeline: (pipeline: NotificationPipelineConfig) => void;
  onCreateProvider: () => void;
  onCreateBot: () => void;
}) {
  const shouldShowGuide =
    providers.filter(isResourceReady).length === 0 ||
    bots.filter(isResourceReady).length === 0 ||
    templates.length === 0 ||
    pipelines.filter((pipeline) => pipeline.enabled).length === 0;

  return (
    <div className={`notificationLedgerPage${shouldShowGuide ? "" : " is-compact"}`}>
      {shouldShowGuide ? (
        <NotificationRouteGuide
          providers={providers}
          bots={bots}
          templates={templates}
          pipelines={pipelines}
        />
      ) : null}
      <div className="notificationLedgerWorkspace">
        {pipelines.length === 0 ? (
          <EmptyPanel
            title="还没有通知规则"
            description="先准备一个数据源和一个发送渠道，再创建第一条发送规则。"
            primary="新建通知规则"
            onPrimary={onCreatePipeline}
            secondaryActions={[
              { label: "添加数据源", onClick: onCreateProvider },
              { label: "添加发送渠道", onClick: onCreateBot },
            ]}
          />
        ) : (
          <PipelineList
            pipelines={pipelines}
            providers={providers}
            bots={bots}
            templates={templates}
            saving={saving}
            testingId={testingId}
            pendingSwitchId={pendingSwitchId}
            onCreatePipeline={onCreatePipeline}
            onToggle={onTogglePipeline}
            onEdit={onEditPipeline}
            onTest={onTestPipeline}
            onDelete={onDeletePipeline}
            onManageResources={onCreateProvider}
          />
        )}
      </div>
    </div>
  );
}

function NotificationRouteGuide({
  providers,
  bots,
  templates,
  pipelines,
}: {
  providers: NotificationProviderConfig[];
  bots: NotificationBotConfig[];
  templates: NotificationTemplateConfig[];
  pipelines: NotificationPipelineConfig[];
}) {
  const readyProviderCount = providers.filter(isResourceReady).length;
  const readyBotCount = bots.filter(isResourceReady).length;
  const enabledPipelineCount = pipelines.filter((pipeline) => pipeline.enabled).length;
  const checks = [
    readyProviderCount > 0,
    readyBotCount > 0,
    templates.length > 0,
    enabledPipelineCount > 0,
  ];
  const firstPending = checks.findIndex((done) => !done);
  const current = firstPending === -1 ? checks.length - 1 : firstPending;
  const baseReady = checks.slice(0, 3).every(Boolean) && pipelines.length > 0;

  const stepStatus = (index: number) => {
    if (checks[index]) {
      return "finish" as const;
    }
    return index === current ? "process" as const : "wait" as const;
  };

  return (
    <section className="notificationRouteGuideCard">
      <div className="notificationSectionTitle">
        <div>
          <h3>快速创建通知规则</h3>
          <p>首次使用按四步完成数据源、发送渠道、消息模板和规则启用。</p>
        </div>
        <StatusPill
          label={enabledPipelineCount > 0 ? "服务正常" : baseReady ? "待启用" : "需要配置"}
          tone={enabledPipelineCount > 0 ? "ok" : "draft"}
        />
      </div>
      <Steps
        className="notificationRouteSteps"
        current={current}
        labelPlacement="vertical"
        responsive
        items={[
          {
            title: "数据源",
            description: readyProviderCount > 0 ? `${readyProviderCount} 个可用` : "添加并测试数据源",
            status: stepStatus(0),
          },
          {
            title: "发送渠道",
            description: readyBotCount > 0 ? `${readyBotCount} 个可用` : "配置消息接收端",
            status: stepStatus(1),
          },
          {
            title: "消息模板",
            description: templates.length > 0 ? "模板可用" : "选择或新建模板",
            status: stepStatus(2),
          },
          {
            title: "通知规则",
            description: enabledPipelineCount > 0
              ? `${enabledPipelineCount} 条已启用`
              : pipelines.length > 0
                ? `${pipelines.length} 条待启用`
                : "组合并启用规则",
            status: stepStatus(3),
          },
        ]}
      />
    </section>
  );
}

function NotificationActivityPage({
  pipelines,
  bots,
  onOpenPipelines,
}: {
  pipelines: NotificationPipelineConfig[];
  bots: NotificationBotConfig[];
  onOpenPipelines: () => void;
}) {
  const rows = pipelines
    .map((pipeline) => ({
      pipeline,
      at: pipeline.lastRunAt ?? pipeline.lastTestAt,
      status: deliveryStatus(pipeline),
    }))
    .sort((left, right) => (right.at ?? 0) - (left.at ?? 0));

  return (
    <section className="notificationResourcePanel notificationActivityPanel" aria-label="发送记录">
      <div className="notificationResourcePanelHeader">
        <div>
          <p className="proxy-kicker">DELIVERY LOG</p>
          <h3>发送记录</h3>
          <p>记录通知规则的测试、发送和失败结果；后台定时运行日志会在后续阶段接入。</p>
        </div>
        <CsButton tone="primary" onClick={onOpenPipelines}>
          管理通知规则
        </CsButton>
      </div>
      {rows.length > 0 ? (
        <div className="notificationActivityTable">
          {rows.map(({ pipeline, at, status }) => (
            <div className="notificationActivityRow" key={pipeline.id}>
              <span>{formatDateTime(at)}</span>
              <strong>{pipeline.name}</strong>
              <p>{targetRecipientSummary(bots, pipeline.botIds, "未选择接收人")}</p>
              <StatusPill {...status} />
            </div>
          ))}
        </div>
      ) : (
        <div className="notificationSoftEmpty">还没有发送记录。测试一条通知规则后会显示记录。</div>
      )}
    </section>
  );
}

function PipelineList({
  pipelines,
  providers,
  bots,
  templates,
  saving,
  testingId,
  pendingSwitchId,
  onCreatePipeline,
  onToggle,
  onEdit,
  onTest,
  onDelete,
  onManageResources,
}: {
  pipelines: NotificationPipelineConfig[];
  providers: NotificationProviderConfig[];
  bots: NotificationBotConfig[];
  templates: NotificationTemplateConfig[];
  saving: boolean;
  testingId: string | null;
  pendingSwitchId: string | null;
  onCreatePipeline: () => void;
  onToggle: (pipeline: NotificationPipelineConfig) => void;
  onEdit: (pipeline: NotificationPipelineConfig) => void;
  onTest: (pipeline: NotificationPipelineConfig) => void;
  onDelete: (pipeline: NotificationPipelineConfig) => void;
  onManageResources: () => void;
}) {
  if (pipelines.length === 0) {
    return (
          <EmptyPanel
            title="还没有通知规则"
            description="通知规则决定何时读取数据源、用什么模板、发到哪个发送渠道。"
            primary="管理数据源和渠道"
            onPrimary={onManageResources}
          />
    );
  }

  return (
    <div className="notificationTableCard notificationLedgerTableCard">
      <div className="notificationSectionHeader">
        <div>
          <h3>
            通知规则
            <span>{pipelines.length} 条</span>
          </h3>
          <p>每条规则定义数据来源、发送内容、接收渠道和启用状态。</p>
        </div>
        <div className="notificationLedgerFilters" aria-label="规则筛选">
          <Button htmlType="button" type="primary" size="small" onClick={onCreatePipeline}>
            新建通知规则
          </Button>
        </div>
      </div>
      <div className="notificationLedgerHeader" aria-hidden="true">
        <span>时间</span>
        <span>规则 / 模板</span>
        <span>状态</span>
        <span>数据源</span>
        <span>发送渠道</span>
        <span>计划</span>
        <span>启用状态</span>
        <span>操作</span>
      </div>
      <div className="notificationLedgerRows">
        {pipelines.map((pipeline) => {
          const status = ledgerPipelineStatus(pipeline, providers, bots, templates);
          const templateName = selectedTemplateName(pipeline, templates);
          const isSwitching = pendingSwitchId === pipeline.id;
          const isTesting = testingId === pipeline.id;
          return (
            <article className="notificationLedgerRow" key={pipeline.id}>
              <span className="notificationLedgerTime">{formatDateTime(pipeline.lastRunAt ?? pipeline.lastTestAt)}</span>
              <div className="notificationLedgerEvent">
                <strong>{pipeline.name}</strong>
                <small>{templateName}</small>
              </div>
              <StatusPill {...status} />
              <span>{idListSummary(pipeline.providerIds, providers, "未选择")}</span>
              <span>{targetRecipientSummary(bots, pipeline.botIds, "未选择")}</span>
              <span>
                {formatSchedule(
                  pipeline.scheduleMode,
                  pipeline.scheduleDate,
                  pipeline.scheduleTime,
                  pipeline.scheduleIntervalMinutes,
                )}
              </span>
              <label className="notificationLedgerSwitch">
                <Switch
                  checked={pipeline.enabled}
                  disabled={saving || Boolean(pendingSwitchId) || Boolean(testingId)}
                  loading={isSwitching}
                  size="small"
                  onChange={() => onToggle(pipeline)}
                />
                <span>{pipeline.enabled ? "已启用" : "已停用"}</span>
              </label>
              <div className="notificationLedgerActionsCell">
                <Button htmlType="button" size="small" onClick={() => onEdit(pipeline)}>
                  编辑
                </Button>
                <Button
                  htmlType="button"
                  size="small"
                  loading={isTesting}
                  disabled={saving || Boolean(testingId)}
                  onClick={() => onTest(pipeline)}
                >
                  测试
                </Button>
                <Button
                  danger
                  htmlType="button"
                  size="small"
                  disabled={saving || Boolean(testingId) || Boolean(pendingSwitchId)}
                  onClick={() => onDelete(pipeline)}
                >
                  删除
                </Button>
              </div>
            </article>
          );
        })}
      </div>
    </div>
  );
}

function ProviderList({
  providers,
  saving,
  pendingDelete,
  openRowMenu,
  onCreate,
  onEdit,
  onDelete,
  onToggleMenu,
}: {
  providers: NotificationProviderConfig[];
  saving: boolean;
  pendingDelete: RowMenuState | null;
  openRowMenu: RowMenuState | null;
  onCreate: () => void;
  onEdit: (provider: NotificationProviderConfig) => void;
  onDelete: (provider: NotificationProviderConfig) => void;
  onToggleMenu: (state: RowMenuState | null) => void;
}) {
  if (providers.length === 0) {
    return (
      <EmptyPanel
        title="还没有数据源"
        description="数据源用于登录 Sub2API，并读取额度、开销和模型用量。"
        primary="添加数据源"
        onPrimary={onCreate}
      />
    );
  }

  return (
    <div className="notificationTableCard">
      <div className="notificationSectionHeader">
        <div>
          <h3>数据源</h3>
          <p>数据源只负责读取额度，不负责决定发给谁。</p>
        </div>
        <CsButton tone="primary" onClick={onCreate}>
          添加数据源
        </CsButton>
      </div>
      <div className="notificationList">
        {providers.map((provider) => {
          const status = resourceStatus(provider);
          return (
            <article className="notificationRow" key={provider.id}>
              <div className="notificationRowMain">
                <div>
                  <h4>{provider.name}</h4>
                  <p>{provider.baseUrl}</p>
                </div>
                <StatusPill {...status} />
              </div>
              <div className="notificationRowMeta">
                <span>类型：Sub2API</span>
                <span>账号：{maskEmail(provider.email)}</span>
                <span>倍率：x{formatCostMultiplier(provider.costMultiplier)}</span>
                <span>最近测试：{formatDateTime(provider.lastTestAt)}</span>
              </div>
              {provider.lastTestError ? (
                <p className="notificationTestState is-error">上次测试失败：{provider.lastTestError}</p>
              ) : null}
              <div className="notificationRowActions">
                <CsButton disabled={saving} onClick={() => onEdit(provider)}>
                  编辑
                </CsButton>
                <RowMoreMenu
                  kind="provider"
                  id={provider.id}
                  saving={saving}
                  openRowMenu={openRowMenu}
                  pendingDelete={pendingDelete}
                  deleteLabel="删除数据源"
                  onToggleMenu={onToggleMenu}
                  onDelete={() => onDelete(provider)}
                />
              </div>
            </article>
          );
        })}
      </div>
    </div>
  );
}

function BotList({
  bots,
  saving,
  pendingDelete,
  openRowMenu,
  onCreate,
  onEdit,
  onDelete,
  onToggleMenu,
}: {
  bots: NotificationBotConfig[];
  saving: boolean;
  pendingDelete: RowMenuState | null;
  openRowMenu: RowMenuState | null;
  onCreate: () => void;
  onEdit: (bot: NotificationBotConfig) => void;
  onDelete: (bot: NotificationBotConfig) => void;
  onToggleMenu: (state: RowMenuState | null) => void;
}) {
  if (bots.length === 0) {
    return (
      <EmptyPanel
        title="还没有发送渠道"
        description="发送渠道用于发送 Telegram 或 Webhook 通知，可以被多条规则复用。"
        primary="添加发送渠道"
        onPrimary={onCreate}
      />
    );
  }

  return (
    <div className="notificationTableCard">
      <div className="notificationSectionHeader">
        <div>
          <h3>发送渠道</h3>
          <p>发送渠道只负责触达消息，不绑定数据源和模板。</p>
        </div>
        <CsButton tone="primary" onClick={onCreate}>
          添加发送渠道
        </CsButton>
      </div>
      <div className="notificationList">
        {bots.map((bot) => {
          const status = resourceStatus(bot);
          return (
            <article className="notificationRow" key={bot.id}>
              <div className="notificationRowMain">
                <div>
                  <h4>{bot.name}</h4>
                  <p>
                    {targetKindLabel(bot.kind)} ·{" "}
                    {bot.kind === "telegram"
                      ? `Chat ID ${bot.telegramChatId ?? "未填写"}`
                      : bot.webhookUrl ?? "未填写 Webhook"}
                  </p>
                </div>
                <StatusPill {...status} />
              </div>
              <div className="notificationRowMeta">
                <span>类型：{targetKindLabel(bot.kind)}</span>
                <span>Token：{maskSecret(bot.telegramBotToken)}</span>
                <span>最近测试：{formatDateTime(bot.lastTestAt)}</span>
              </div>
              {bot.lastTestError ? (
                <p className="notificationTestState is-error">上次测试失败：{bot.lastTestError}</p>
              ) : null}
              <div className="notificationRowActions">
                <CsButton disabled={saving} onClick={() => onEdit(bot)}>
                  编辑
                </CsButton>
                <RowMoreMenu
                  kind="bot"
                  id={bot.id}
                  saving={saving}
                  openRowMenu={openRowMenu}
                  pendingDelete={pendingDelete}
                  deleteLabel="删除机器人"
                  onToggleMenu={onToggleMenu}
                  onDelete={() => onDelete(bot)}
                />
              </div>
            </article>
          );
        })}
      </div>
    </div>
  );
}

function TemplateList({
  templates,
  saving,
  pendingDelete,
  openRowMenu,
  onCreate,
  onEdit,
  onCopy,
  onDelete,
  onToggleMenu,
}: {
  templates: NotificationTemplateConfig[];
  saving: boolean;
  pendingDelete: RowMenuState | null;
  openRowMenu: RowMenuState | null;
  onCreate: () => void;
  onEdit: (template: NotificationTemplateConfig) => void;
  onCopy: (template: NotificationTemplateConfig) => void;
  onDelete: (template: NotificationTemplateConfig) => void;
  onToggleMenu: (state: RowMenuState | null) => void;
}) {
  const builtinTemplateRows = templates.filter((template) => template.id.startsWith("builtin-"));
  const customTemplateRows = templates.filter((template) => !template.id.startsWith("builtin-"));

  const renderTemplateItem = (template: NotificationTemplateConfig) => {
    const builtin = template.id.startsWith("builtin-");
    const variableCount = (template.messageTemplate.match(/\{[^}]+\}/g) ?? []).length;

    return {
      key: template.id,
      label: (
        <div className="notificationTemplateItemHeader">
          <div className="notificationTemplateItemTitle">
            <h5>{template.name}</h5>
            <p>{builtinTemplates[template.preset]?.description ?? "自定义通知模板"}</p>
          </div>
          <div className="notificationTemplateItemMeta">
            <StatusPill label={builtin ? "内置" : "自定义"} tone={builtin ? "draft" : "ok"} />
            <span>{variableCount} 个变量</span>
          </div>
        </div>
      ),
      children: (
        <div className="notificationTemplateDetails">
        <div className="notificationRowMeta">
          <span>类型：{builtinTemplates[template.preset]?.label ?? template.preset}</span>
          <span>变量：{variableCount} 个</span>
          <span>最近修改：{formatDateTime(template.updatedAt)}</span>
        </div>
        <div className="notificationPreviewBox notificationSavedPreview">
          <span>内容预览</span>
          <pre>{template.messageTemplate}</pre>
        </div>
        <div className="notificationTemplateDetailActions">
          <CsButton disabled={saving} onClick={() => (builtin ? onCopy(template) : onEdit(template))}>
            {builtin ? "复制" : "编辑"}
          </CsButton>
          {!builtin ? (
            <RowMoreMenu
              kind="template"
              id={template.id}
              saving={saving}
              openRowMenu={openRowMenu}
              pendingDelete={pendingDelete}
              deleteLabel="删除模板"
              onToggleMenu={onToggleMenu}
              onDelete={() => onDelete(template)}
            />
          ) : null}
        </div>
      </div>
      ),
    };
  };

  const renderTemplateTree = (rows: NotificationTemplateConfig[]) => {
    if (rows.length === 0) {
      return <div className="notificationSoftEmpty">还没有自定义模板。可以复制内置模板后再修改。</div>;
    }
    return (
      <Collapse
        accordion
        className="notificationTemplateItemCollapse"
        expandIconPosition="end"
        items={rows.map(renderTemplateItem)}
      />
    );
  };

  return (
    <div className="notificationTableCard notificationTemplateCard">
      <div className="notificationSectionHeader">
        <div>
          <h3>模板库</h3>
          <p>消息模板决定发送什么内容，通知规则决定何时发、发给谁。</p>
        </div>
        <CsButton tone="primary" onClick={onCreate}>
          新建模板
        </CsButton>
      </div>
      <Collapse
        className="notificationTemplateGroups notificationTemplateCollapse"
        accordion
        expandIconPosition="end"
        items={[
          {
            key: "builtin",
            label: (
              <div className="notificationTemplateGroupHeader">
                <div>
                  <h4>内置模板</h4>
                  <p>系统预置模板不可直接修改，复制后可作为自定义模板继续编辑。</p>
                </div>
                <StatusPill label={`${builtinTemplateRows.length} 个`} tone="draft" />
              </div>
            ),
            children: renderTemplateTree(builtinTemplateRows),
          },
          {
            key: "custom",
            label: (
              <div className="notificationTemplateGroupHeader">
                <div>
                  <h4>自定义模板</h4>
                  <p>保存从内置模板复制或手动创建的发送内容，可被通知规则引用。</p>
                </div>
                <StatusPill
                  label={`${customTemplateRows.length} 个`}
                  tone={customTemplateRows.length > 0 ? "ok" : "draft"}
                />
              </div>
            ),
            children: renderTemplateTree(customTemplateRows),
          },
        ]}
      />
    </div>
  );
}

function RowMoreMenu({
  kind,
  id,
  saving,
  openRowMenu,
  pendingDelete,
  deleteLabel,
  secondaryLabel,
  onToggleMenu,
  onDelete,
  onSecondary,
}: {
  kind: NotificationEntityKind;
  id: string;
  saving: boolean;
  openRowMenu: RowMenuState | null;
  pendingDelete: RowMenuState | null;
  deleteLabel: string;
  secondaryLabel?: string;
  onToggleMenu: (state: RowMenuState | null) => void;
  onDelete: () => void;
  onSecondary?: () => void;
}) {
  const isOpen = openRowMenu?.kind === kind && openRowMenu.id === id;
  const isPendingDelete = pendingDelete?.kind === kind && pendingDelete.id === id;

  return (
    <div className="notificationMoreMenuWrap">
      <CsButton
        disabled={saving}
        onClick={() => onToggleMenu(isOpen ? null : { kind, id })}
      >
        更多
      </CsButton>
      {isOpen ? (
        <div className="notificationMoreMenu" role="menu">
          {secondaryLabel && onSecondary ? (
            <CsButton role="menuitem" onClick={onSecondary}>
              {secondaryLabel}
            </CsButton>
          ) : null}
          <CsButton
            tone="danger"
            role="menuitem"
            disabled={saving}
            onClick={onDelete}
          >
            {isPendingDelete ? "确认删除" : deleteLabel}
          </CsButton>
        </div>
      ) : null}
    </div>
  );
}

function PipelineDrawerPanel({
  drawer,
  draft,
  draftTestState,
  providers,
  bots,
  templates,
  saving,
  testing,
  onDraftChange,
  onTest,
  onSaveDraft,
  onSaveEnabled,
  onClose,
}: {
  drawer: PipelineDrawer;
  draft: PipelineDraft;
  draftTestState: DraftTestState;
  providers: NotificationProviderConfig[];
  bots: NotificationBotConfig[];
  templates: NotificationTemplateConfig[];
  saving: boolean;
  testing: boolean;
  onDraftChange: (draft: PipelineDraft) => void;
  onTest: () => void;
  onSaveDraft: () => void;
  onSaveEnabled: () => void;
  onClose: () => void;
}) {
  const title = drawer.mode === "edit" ? "编辑通知规则" : "新建通知规则";
  const selectedTemplate = templates.find((template) => template.id === draft.templateId);
  const canSaveEnabled =
    draft.providerIds.length > 0 &&
    draft.botIds.length > 0 &&
    (draft.templateOverrideEnabled ? draft.templateOverride.trim().length > 0 : Boolean(draft.templateId)) &&
    (draft.scheduleMode !== "date" || Boolean(draft.scheduleDate.trim())) &&
    (draft.scheduleMode !== "interval" || Boolean(normalizeScheduleIntervalMinutes(draft.scheduleIntervalMinutes)));
  const selectedProviderNames = providers
    .filter((provider) => draft.providerIds.includes(provider.id))
    .map((provider) => provider.name);
  const selectedBotNames = bots
    .filter((bot) => draft.botIds.includes(bot.id))
    .map((bot) => bot.name);
  const providerSelectOptions = providers.map((provider) => {
    const status = resourceStatus(provider);
    const ready = isResourceReady(provider);
    return {
      value: provider.id,
      label: provider.name,
      description: `数据源 · x${formatCostMultiplier(provider.costMultiplier)} · ${status.label}`,
      disabled: !ready && !draft.providerIds.includes(provider.id),
    };
  });
  const botSelectOptions = bots.map((bot) => {
    const status = resourceStatus(bot);
    const ready = isResourceReady(bot);
    return {
      value: bot.id,
      label: bot.name,
      description: `${targetKindLabel(bot.kind)} · ${status.label}`,
      disabled: !ready && !draft.botIds.includes(bot.id),
    };
  });

  return (
    <div className="notificationDrawerBackdrop" role="presentation">
      <aside className="notificationDrawer notificationPipelineDrawer" aria-label={title}>
        <div className="notificationDrawerHeader">
          <div>
            <p className="proxy-kicker">NOTIFICATION RULE</p>
            <h3>{title}</h3>
          </div>
          <CsButton className="notificationCompactButton" onClick={onClose}>
            关闭
          </CsButton>
        </div>

        <div className="notificationDrawerBody">
          <div className="notificationWizardSteps" aria-label="通知规则配置步骤">
            <span>1 选择</span>
            <span>2 内容</span>
            <span>3 检测</span>
          </div>

          <label className="notificationField">
            <span>规则名称</span>
            <CsInput
              value={draft.name}
              placeholder="例如：每日额度日报"
              onChange={(event) => onDraftChange({ ...draft, name: event.target.value })}
            />
          </label>

          <div className="notificationAssignmentBox notificationRouteAssignmentBox">
            <div className="notificationAssignmentHeader">
              <div>
                <span>1. 选择数据源和发送渠道</span>
                <p>选择要读取额度的数据源，以及真正接收消息的发送渠道。</p>
              </div>
              <CsButton
                className="notificationCompactButton"
                onClick={() =>
                  onDraftChange({
                    ...draft,
                    providerIds: providers.filter(isResourceReady).map((provider) => provider.id),
                    botIds: bots.filter(isResourceReady).map((bot) => bot.id),
                  })
                }
              >
                全选可用
              </CsButton>
            </div>
            {providers.length > 0 ? (
              <RouteResourceMultiSelect
                title="数据源"
                description="选择需要参与额度查询的数据源。"
                placeholder="选择数据源"
                options={providerSelectOptions}
                value={draft.providerIds}
                onChange={(providerIds) => onDraftChange({ ...draft, providerIds })}
                emptyText="还没有可选数据源。"
              />
            ) : (
              <p className="notificationAssignmentEmpty">还没有数据源，请先添加并测试。</p>
            )}
            {bots.length > 0 ? (
              <RouteResourceMultiSelect
                title="发送渠道"
                description="选择要接收消息的 Telegram 机器人或 Webhook。"
                placeholder="选择发送渠道"
                options={botSelectOptions}
                value={draft.botIds}
                onChange={(botIds) => onDraftChange({ ...draft, botIds })}
                emptyText="还没有可选发送渠道。"
              />
            ) : (
              <p className="notificationAssignmentEmpty">还没有发送渠道，请先添加并发送测试。</p>
            )}
            <div className="notificationRouteSummary">
              <span>当前规则</span>
              <strong>
                {selectedProviderNames.length > 0 ? selectedProviderNames.join("、") : "未选择数据源"}
                {" -> "}
                {selectedBotNames.length > 0 ? selectedBotNames.join("、") : "未选择发送渠道"}
              </strong>
            </div>
          </div>

          <div className="notificationAssignmentBox">
            <div className="notificationAssignmentHeader">
              <div>
                <span>2. 选择通知内容</span>
                <p>默认使用消息模板，也可以只为这条规则覆盖文案。</p>
              </div>
            </div>
            <label className="notificationField">
              <span>模板</span>
              <CsSelect
                value={draft.templateId}
                disabled={draft.templateOverrideEnabled}
                onChange={(templateId) => onDraftChange({ ...draft, templateId })}
                options={templates.map((template) => ({
                  value: template.id,
                  label: template.name,
                }))}
              />
            </label>
            <label className="notificationToggleField">
              <Switch
                checked={draft.templateOverrideEnabled}
                onChange={(checked) =>
                  onDraftChange({
                    ...draft,
                    templateOverrideEnabled: checked,
                  })
                }
              />
              <span>
                <strong>为这条规则单独覆盖模板内容</strong>
                不影响模板库，适合给某个发送渠道定制文案。
              </span>
            </label>
            {draft.templateOverrideEnabled ? (
              <label className="notificationField">
                <span>规则覆盖文案</span>
              <CsTextArea
                className="notificationTextarea"
                value={draft.templateOverride}
                placeholder={selectedTemplate?.messageTemplate ?? builtinTemplates.usageReport.template}
                onChange={(event) => onDraftChange({ ...draft, templateOverride: event.target.value })}
              />
              </label>
            ) : null}
            <div className="notificationPreviewBox notificationSavedPreview">
              <span>内容预览</span>
              <pre>{draft.templateOverrideEnabled ? draft.templateOverride || "尚未填写覆盖文案" : selectedTemplate?.messageTemplate ?? "未选择模板"}</pre>
            </div>
          </div>

          <div className="notificationAssignmentBox">
            <div className="notificationAssignmentHeader">
              <div>
                <span>3. 计划与启用检测</span>
                <p>点击保存并启用时会自动检测规则；失败时可以返回修改或保存草稿。</p>
              </div>
            </div>
            <label className="notificationField">
              <span>推送模式</span>
              <CsSelect
                value={draft.scheduleMode}
                onChange={(scheduleMode) =>
                  onDraftChange({
                    ...draft,
                    scheduleMode: scheduleMode as PipelineDraft["scheduleMode"],
                  })
                }
                options={[
                  { value: "manual", label: "手动触发" },
                  { value: "daily", label: "每日定时" },
                  { value: "interval", label: "间隔推送" },
                  { value: "date", label: "指定日期" },
                ]}
              />
            </label>
            {draft.scheduleMode === "date" ? (
              <label className="notificationField">
                <span>推送日期</span>
                <CsInput
                  className="notificationInput"
                  type="date"
                  value={draft.scheduleDate}
                  onChange={(event) => onDraftChange({ ...draft, scheduleDate: event.target.value })}
                />
              </label>
            ) : null}
            {draft.scheduleMode === "daily" || draft.scheduleMode === "date" ? (
              <label className="notificationField">
                <span>推送时间</span>
                <CsInput
                  type="time"
                  value={draft.scheduleTime}
                  onChange={(event) => onDraftChange({ ...draft, scheduleTime: event.target.value })}
                />
              </label>
            ) : null}
            {draft.scheduleMode === "interval" ? (
              <div className="notificationScheduleInterval">
                <label className="notificationField">
                  <span>间隔时间</span>
                  <CsInput
                    inputMode="numeric"
                    min={1}
                    max={1440}
                    type="number"
                    value={draft.scheduleIntervalMinutes}
                    onChange={(event) =>
                      onDraftChange({ ...draft, scheduleIntervalMinutes: event.target.value })
                    }
                  />
                </label>
                <div className="notificationSchedulePresets" aria-label="常用间隔">
                  {[10, 30, 60, 180].map((minutes) => (
                    <Button
                      htmlType="button"
                      key={minutes}
                      size="small"
                      type={draft.scheduleIntervalMinutes === String(minutes) ? "primary" : "default"}
                      onClick={() =>
                        onDraftChange({ ...draft, scheduleIntervalMinutes: String(minutes) })
                      }
                    >
                      {formatIntervalMinutes(minutes)}
                    </Button>
                  ))}
                </div>
                <p className="notificationScheduleHint">
                  保存后后台调度器会按这个周期推送；最小 1 分钟，最大 24 小时。
                </p>
              </div>
            ) : null}
            <label className="notificationToggleField">
              <Switch
                checked={draft.aggregateEnabled}
                onChange={(checked) => onDraftChange({ ...draft, aggregateEnabled: checked })}
              />
              <span>
                <strong>聚合推送</strong>
                多个数据源同步查询后合并为一条日报，并按等效倍率统一价格口径。
              </span>
            </label>
            {draftTestState.lastTestAt ? (
              <p className={`notificationTestState ${draftTestState.lastTestError ? "is-error" : "is-ok"}`}>
                {draftTestState.lastTestError
                  ? `最近测试失败：${draftTestState.lastTestError}`
                  : `最近测试时间：${formatDateTime(draftTestState.lastTestAt)}`}
              </p>
            ) : (
              <p className="notificationTestState is-info">保存并启用时会自动检测这条规则。</p>
            )}
          </div>
        </div>

        <div className="notificationDrawerActions">
          <CsButton onClick={onClose}>
            取消
          </CsButton>
          <CsButton disabled={saving} onClick={onSaveDraft}>
            保存草稿
          </CsButton>
          <CsButton
            disabled={saving || testing || !canSaveEnabled}
            loading={testing}
            onClick={onTest}
          >
            {testing ? "测试中..." : "发送测试"}
          </CsButton>
          <CsButton
            tone="primary"
            disabled={saving || testing || !canSaveEnabled}
            loading={saving || testing}
            onClick={onSaveEnabled}
          >
            {testing ? "检测中..." : "保存并启用"}
          </CsButton>
        </div>
      </aside>
    </div>
  );
}

function RouteResourceMultiSelect({
  title,
  description,
  placeholder,
  options,
  value,
  emptyText,
  onChange,
}: {
  title: string;
  description: string;
  placeholder: string;
  options: RouteSelectOption[];
  value: string[];
  emptyText: string;
  onChange: (keys: string[]) => void;
}) {
  const selectedOptions = options.filter((option) => value.includes(option.value));
  return (
    <div className="notificationRouteSelectGroup">
      <div className="notificationRouteSelectHeader">
        <strong>{title}</strong>
        <span>{description}</span>
      </div>
      <CsSelect<string[]>
        mode="multiple"
        className="notificationRouteMultiSelect"
        value={value}
        placeholder={placeholder}
        optionFilterProp="label"
        maxTagCount="responsive"
        options={options}
        notFoundContent={emptyText}
        onChange={(nextValue) => onChange(nextValue)}
      />
      <div className="notificationSelectedSummary">
        {selectedOptions.length > 0 ? (
          selectedOptions.map((option) => (
            <span className="notificationSelectedChip" key={option.value}>
              <strong>{option.label}</strong>
              <em>{option.description}</em>
            </span>
          ))
        ) : (
          <span className="notificationSelectedEmpty">{emptyText}</span>
        )}
      </div>
    </div>
  );
}

function TemplateDrawerPanel({
  drawer,
  draft,
  providers,
  saving,
  onDraftChange,
  onSave,
  onClose,
}: {
  drawer: TemplateDrawer;
  draft: TemplateDraft;
  providers: NotificationProviderConfig[];
  saving: boolean;
  onDraftChange: (draft: TemplateDraft) => void;
  onSave: () => void;
  onClose: () => void;
}) {
  const title = drawer.mode === "edit" ? "编辑模板" : "新建模板";
  const templateEditorRef = useRef<ComponentRef<typeof Input.TextArea>>(null);
  const readyProviders = providers.filter(isResourceReady);
  const readyProviderNames = readyProviders.map((provider) => provider.name).slice(0, 3);

  const insertTemplateVariable = (token: string) => {
    const template = draft.messageTemplate;
    const textarea = templateEditorRef.current?.resizableTextArea?.textArea ?? null;
    const selectionStart = textarea?.selectionStart ?? template.length;
    const selectionEnd = textarea?.selectionEnd ?? selectionStart;
    const nextTemplate = `${template.slice(0, selectionStart)}${token}${template.slice(selectionEnd)}`;
    const cursor = selectionStart + token.length;

    onDraftChange({ ...draft, messageTemplate: nextTemplate });
    window.requestAnimationFrame(() => {
      const nextTextarea = templateEditorRef.current?.resizableTextArea?.textArea ?? null;
      templateEditorRef.current?.focus();
      nextTextarea?.setSelectionRange(cursor, cursor);
    });
  };

  return (
    <div className="notificationDrawerBackdrop" role="presentation">
      <aside className="notificationDrawer" aria-label={title}>
        <div className="notificationDrawerHeader">
          <div>
            <p className="proxy-kicker">MESSAGE TEMPLATE</p>
            <h3>{title}</h3>
          </div>
          <CsButton className="notificationCompactButton" onClick={onClose}>
            关闭
          </CsButton>
        </div>

        <div className="notificationDrawerBody">
          <label className="notificationField">
            <span>模板名称</span>
            <CsInput
              value={draft.name}
              placeholder="例如：我的额度日报"
              onChange={(event) => onDraftChange({ ...draft, name: event.target.value })}
            />
          </label>
          <label className="notificationField">
            <span>模板类型</span>
            <CsSelect
              value={draft.preset}
              onChange={(nextPreset) => {
                const preset = nextPreset as NotificationTemplatePreset;
                onDraftChange({
                  ...draft,
                  preset,
                  messageTemplate: draft.messageTemplate || builtinTemplates[preset].template,
                });
              }}
              options={Object.entries(builtinTemplates).map(([preset, template]) => ({
                value: preset,
                label: template.label,
              }))}
            />
          </label>
          <label className="notificationField">
            <span>模板内容</span>
            <CsTextArea
              ref={templateEditorRef}
              rows={14}
              className="notificationTextarea notificationTemplateEditor"
              value={draft.messageTemplate}
              placeholder={builtinTemplates[draft.preset].template}
              onChange={(event) => onDraftChange({ ...draft, messageTemplate: event.target.value })}
            />
          </label>
          <section className="notificationTemplateVariablePanel" aria-label="模板变量">
            <div className="notificationTemplateVariableHeader">
              <span>插入变量</span>
              <p>
                {readyProviderNames.length > 0
                  ? `当前可读取：${readyProviderNames.join("、")}${readyProviders.length > readyProviderNames.length ? "等" : ""}`
                  : "保存规则后会按绑定的 API 数据源渲染。"}
              </p>
            </div>
            <div className="notificationTemplateVariableGrid">
              {templateVariableOptions.map((option) => (
                <Button
                  key={option.token}
                  size="small"
                  className="notificationTemplateVariableButton"
                  title={option.description}
                  onClick={() => insertTemplateVariable(option.token)}
                >
                  <span>{option.label}</span>
                  <code>{option.token}</code>
                </Button>
              ))}
            </div>
          </section>
          <div className="notificationTemplateHelper">
            <p>变量会在发送时按规则绑定的数据源替换；点击按钮会插入到当前光标位置。</p>
            <p>聚合日报由后端真实渲染；这里保存的是模板文本和规则覆盖文案。</p>
          </div>
        </div>

        <div className="notificationDrawerActions">
          <CsButton onClick={onClose}>
            取消
          </CsButton>
          <CsButton tone="primary" disabled={saving} onClick={onSave}>
            {saving ? "保存中..." : "保存模板"}
          </CsButton>
        </div>
      </aside>
    </div>
  );
}

function ResourceDrawerPanel({
  drawer,
  providerDraft,
  botDraft,
  draftTestState,
  saving,
  testing,
  discoveringChats,
  telegramChats,
  onProviderDraftChange,
  onBotDraftChange,
  onDiscoverChats,
  onSelectChat,
  onTest,
  onSave,
  onClose,
}: {
  drawer: ProviderOrBotDrawer;
  providerDraft: ProviderDraft;
  botDraft: BotDraft;
  draftTestState: DraftTestState;
  saving: boolean;
  testing: boolean;
  discoveringChats: boolean;
  telegramChats: TelegramChatCandidate[];
  onProviderDraftChange: (draft: ProviderDraft) => void;
  onBotDraftChange: (draft: BotDraft) => void;
  onDiscoverChats: () => void;
  onSelectChat: (chatId: string) => void;
  onTest: () => void;
  onSave: () => void;
  onClose: () => void;
}) {
  const isProvider = drawer.kind === "provider";
  const title = isProvider
    ? drawer.mode === "edit" ? "编辑数据源" : "添加数据源"
    : drawer.mode === "edit" ? "编辑发送渠道" : "添加发送渠道";

  return (
    <div className="notificationDrawerBackdrop" role="presentation">
      <aside className="notificationDrawer" aria-label={title}>
        <div className="notificationDrawerHeader">
          <div>
            <p className="proxy-kicker">{isProvider ? "DATA SOURCE" : "DELIVERY CHANNEL"}</p>
            <h3>{title}</h3>
          </div>
          <CsButton className="notificationCompactButton" onClick={onClose}>
            关闭
          </CsButton>
        </div>

        {isProvider ? (
          <ProviderDrawerForm
            draft={providerDraft}
            draftTestState={draftTestState}
            onChange={onProviderDraftChange}
          />
        ) : (
          <BotDrawerForm
            draft={botDraft}
            draftTestState={draftTestState}
            discoveringChats={discoveringChats}
            telegramChats={telegramChats}
            onChange={onBotDraftChange}
            onDiscoverChats={onDiscoverChats}
            onSelectChat={onSelectChat}
          />
        )}

        <div className="notificationDrawerActions">
          <CsButton onClick={onClose}>
            取消
          </CsButton>
          <CsButton disabled={saving || testing} loading={testing} onClick={onTest}>
            {testing ? "测试中..." : isProvider ? "测试数据源" : "发送测试"}
          </CsButton>
          <CsButton tone="primary" disabled={saving} onClick={onSave}>
            {saving ? "保存中..." : "保存资源"}
          </CsButton>
        </div>
      </aside>
    </div>
  );
}

function ProviderDrawerForm({
  draft,
  draftTestState,
  onChange,
}: {
  draft: ProviderDraft;
  draftTestState: DraftTestState;
  onChange: (draft: ProviderDraft) => void;
}) {
  return (
    <div className="notificationDrawerBody">
      <label className="notificationField">
        <span>数据源名称</span>
        <CsInput
          value={draft.name}
          placeholder="例如：示例中转 / 备用平台 / 团队网关"
          onChange={(event) => onChange({ ...draft, name: event.target.value })}
        />
      </label>
      <label className="notificationField">
        <span>数据源类型</span>
        <CsInput value="Sub2API" disabled />
      </label>
      <label className="notificationField">
        <span>访问 URL</span>
        <CsInput
          value={draft.baseUrl}
          placeholder="https://example.com/api/v1"
          onChange={(event) => onChange({ ...draft, baseUrl: event.target.value })}
        />
      </label>
      <label className="notificationField">
        <span>登录账号</span>
        <CsInput
          value={draft.email}
          placeholder="admin@example.com"
          onChange={(event) => onChange({ ...draft, email: event.target.value })}
        />
      </label>
      <label className="notificationField">
        <span>登录密码</span>
        <CsInput
          type="password"
          value={draft.password}
          placeholder="用于读取 Sub2API 用量"
          onChange={(event) => onChange({ ...draft, password: event.target.value })}
        />
      </label>
      <label className="notificationField">
        <span>等效倍率</span>
        <CsInput
          value={draft.costMultiplier}
          placeholder="1"
          onChange={(event) => onChange({ ...draft, costMultiplier: event.target.value })}
        />
      </label>
      <p className="notificationScheduleHint">
        等效倍率用于把不同中转站的价格换算到统一口径，例如 0.7、1、2。
      </p>
      <ResourceDraftTestState state={draftTestState} emptyText="保存前可以先测试数据源连通性。" />
    </div>
  );
}

function BotDrawerForm({
  draft,
  draftTestState,
  discoveringChats,
  telegramChats,
  onChange,
  onDiscoverChats,
  onSelectChat,
}: {
  draft: BotDraft;
  draftTestState: DraftTestState;
  discoveringChats: boolean;
  telegramChats: TelegramChatCandidate[];
  onChange: (draft: BotDraft) => void;
  onDiscoverChats: () => void;
  onSelectChat: (chatId: string) => void;
}) {
  return (
    <div className="notificationDrawerBody">
      <label className="notificationField">
        <span>渠道名称</span>
        <CsInput
          value={draft.name}
          placeholder="例如：额度推送 Bot"
          onChange={(event) => onChange({ ...draft, name: event.target.value })}
        />
      </label>
      <label className="notificationField">
        <span>类型</span>
        <CsSelect
          value={draft.kind}
          onChange={(kind) =>
            onChange({ ...draft, kind: kind as NotificationBotConfig["kind"] })
          }
          options={[
            { value: "telegram", label: "Telegram Bot" },
            { value: "webhook", label: "Webhook" },
          ]}
        />
      </label>

      {draft.kind === "telegram" ? (
        <>
          <label className="notificationField">
            <span>Bot Token</span>
            <CsInput
              type="password"
              value={draft.telegramBotToken}
              placeholder="123456:ABC-DEF..."
              onChange={(event) => onChange({ ...draft, telegramBotToken: event.target.value })}
            />
          </label>
          <label className="notificationField">
            <span>Chat ID</span>
            <CsInput
              value={draft.telegramChatId}
              placeholder="-1001234567890 或个人 chat_id"
              onChange={(event) => onChange({ ...draft, telegramChatId: event.target.value })}
            />
          </label>
          <div className="notificationTelegramDiscovery">
            <CsButton
              disabled={!draft.telegramBotToken.trim() || discoveringChats}
              onClick={onDiscoverChats}
            >
              {discoveringChats ? "获取中..." : "验证 Token 并获取 Chat ID"}
            </CsButton>
            <p>需要机器人先收到过消息，或已加入群组/频道；否则 Telegram 不会返回 Chat ID。</p>
            {telegramChats.length > 0 ? (
              <div className="notificationChatCandidateList">
                {telegramChats.map((chat) => (
                  <CsButton
                    key={chat.id}
                    onClick={() => onSelectChat(chat.id)}
                  >
                    {chat.title} · {chat.chatType} · {chat.id}
                  </CsButton>
                ))}
              </div>
            ) : null}
          </div>
        </>
      ) : (
        <label className="notificationField">
          <span>Webhook URL</span>
          <CsInput
            value={draft.webhookUrl}
            placeholder="https://example.com/webhook"
          onChange={(event) => onChange({ ...draft, webhookUrl: event.target.value })}
        />
      </label>
      )}
      <ResourceDraftTestState state={draftTestState} emptyText="保存前可以先发送一条测试消息。" />
    </div>
  );
}

function ResourceDraftTestState({
  state,
  emptyText,
}: {
  state: DraftTestState;
  emptyText: string;
}) {
  if (state.lastTestAt) {
    return (
      <p className={`notificationTestState ${state.lastTestError ? "is-error" : "is-ok"}`}>
        {state.lastTestError
          ? `最近测试失败：${state.lastTestError}`
          : `最近测试时间：${formatDateTime(state.lastTestAt)}`}
      </p>
    );
  }
  return <p className="notificationTestState is-info">{emptyText}</p>;
}

function EmptyPanel({
  title,
  description,
  primary,
  onPrimary,
  secondaryActions,
}: {
  title: string;
  description: string;
  primary: string;
  onPrimary: () => void;
  secondaryActions?: Array<{ label: string; onClick: () => void }>;
}) {
  return (
    <div className="notificationEmptyPanel">
      <h3>{title}</h3>
      <p>{description}</p>
      <div className="notificationEmptyActions">
        <CsButton tone="primary" onClick={onPrimary}>
          {primary}
        </CsButton>
        {secondaryActions?.map((action) => (
          <CsButton key={action.label} onClick={action.onClick}>
            {action.label}
          </CsButton>
        ))}
      </div>
    </div>
  );
}
