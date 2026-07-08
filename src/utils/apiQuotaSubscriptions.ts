import type { ApiQuotaMode } from "../types/app";

export type ApiQuotaSubscriptionLabelMode = "none" | "manual" | "autoAndManual";
export type ApiQuotaBalanceDisplayControl = "manual" | "preset";

export type ApiQuotaProviderCapability = {
  balanceDisplayControl: ApiQuotaBalanceDisplayControl;
  balanceDisplayEnabled: boolean;
  defaultQuotaMode: ApiQuotaMode;
  subscriptionLabelMode: ApiQuotaSubscriptionLabelMode;
};

const DEFAULT_API_QUOTA_SUBSCRIPTION_LABELS = ["Lite", "Pro", "Max"] as const;
const MINIMAX_TOKEN_PLAN_SUBSCRIPTION_LABELS = ["Plus", "Max", "Ultra"] as const;
const MIMO_TOKEN_PLAN_SUBSCRIPTION_LABELS = ["Lite", "Standard", "Pro", "Max"] as const;
const KIMI_SUBSCRIPTION_LABELS = [
  "Adagio",
  "Moderato",
  "Allegretto",
  "Allegro",
  "Vivace",
] as const;

const CUSTOM_API_QUOTA_CAPABILITY: ApiQuotaProviderCapability = {
  balanceDisplayControl: "manual",
  balanceDisplayEnabled: false,
  defaultQuotaMode: "apiOnly",
  subscriptionLabelMode: "none",
};

export function normalizeApiQuotaSubscriptionName(value: string | null | undefined) {
  const normalized = value?.trim();
  return normalized ? normalized : null;
}

function normalizeApiBaseUrlForSubscription(value: string | null | undefined) {
  const normalized = (value ?? "").trim().replace(/\/+$/, "").toLowerCase();
  return normalized.replace(/\/api\/v1$/i, "").replace(/\/v1$/i, "");
}

function isDeepSeekBaseUrl(normalized: string) {
  return normalized.includes("api.deepseek.com");
}

function isZaiGlmBaseUrl(normalized: string) {
  return normalized.includes("api.z.ai") || normalized.includes("bigmodel.cn");
}

function isMiniMaxBaseUrl(normalized: string) {
  return (
    normalized.includes("api.minimaxi.com") ||
    normalized.includes("minimaxi.com") ||
    normalized.includes("api.minimax.io") ||
    normalized.includes("minimax.io")
  );
}

function isKimiBaseUrl(normalized: string) {
  return (
    normalized.includes("api.moonshot.cn") ||
    normalized.includes("api.moonshot.ai") ||
    normalized.includes("api.moonshot.com") ||
    normalized.includes("api.kimi.com")
  );
}

function isXiaomiMiMoBaseUrl(normalized: string) {
  return normalized.includes("xiaomimimo.com");
}

export function isMiMoTokenPlanBaseUrl(value: string | null | undefined) {
  const normalized = normalizeApiBaseUrlForSubscription(value);
  return normalized.includes("token-plan") && normalized.includes("xiaomimimo.com");
}

function apiQuotaSubscriptionLabelsForBaseUrl(baseUrl: string | null | undefined) {
  const normalized = normalizeApiBaseUrlForSubscription(baseUrl);
  if (isMiMoTokenPlanBaseUrl(normalized)) {
    return MIMO_TOKEN_PLAN_SUBSCRIPTION_LABELS;
  }
  if (isMiniMaxBaseUrl(normalized)) {
    return MINIMAX_TOKEN_PLAN_SUBSCRIPTION_LABELS;
  }
  if (isKimiBaseUrl(normalized)) {
    return KIMI_SUBSCRIPTION_LABELS;
  }
  return DEFAULT_API_QUOTA_SUBSCRIPTION_LABELS;
}

export function detectApiQuotaSubscriptionLabelMode(
  baseUrl: string | null | undefined,
): ApiQuotaSubscriptionLabelMode {
  return resolveApiQuotaProviderCapability(baseUrl).subscriptionLabelMode;
}

export function resolveApiQuotaProviderCapability(
  baseUrl: string | null | undefined,
): ApiQuotaProviderCapability {
  const normalized = normalizeApiBaseUrlForSubscription(baseUrl);
  if (!normalized) {
    return CUSTOM_API_QUOTA_CAPABILITY;
  }

  if (isZaiGlmBaseUrl(normalized)) {
    return {
      balanceDisplayControl: "preset",
      balanceDisplayEnabled: true,
      defaultQuotaMode: "apiOnly",
      subscriptionLabelMode: "autoAndManual",
    };
  }

  if (isMiMoTokenPlanBaseUrl(normalized)) {
    return {
      balanceDisplayControl: "preset",
      balanceDisplayEnabled: false,
      defaultQuotaMode: "apiOnly",
      subscriptionLabelMode: "manual",
    };
  }

  if (isMiniMaxBaseUrl(normalized)) {
    return {
      balanceDisplayControl: "preset",
      balanceDisplayEnabled: true,
      defaultQuotaMode: "apiOnly",
      subscriptionLabelMode: "manual",
    };
  }

  if (isKimiBaseUrl(normalized)) {
    return {
      balanceDisplayControl: "preset",
      balanceDisplayEnabled: true,
      defaultQuotaMode: "apiOnly",
      subscriptionLabelMode: "manual",
    };
  }

  if (isDeepSeekBaseUrl(normalized)) {
    return {
      balanceDisplayControl: "preset",
      balanceDisplayEnabled: true,
      defaultQuotaMode: "apiOnly",
      subscriptionLabelMode: "none",
    };
  }

  if (isXiaomiMiMoBaseUrl(normalized)) {
    return {
      balanceDisplayControl: "preset",
      balanceDisplayEnabled: false,
      defaultQuotaMode: "apiOnly",
      subscriptionLabelMode: "none",
    };
  }

  return CUSTOM_API_QUOTA_CAPABILITY;
}

export function apiQuotaSubscriptionSelectOptions(
  mode: ApiQuotaSubscriptionLabelMode,
  baseUrl?: string | null,
) {
  if (mode === "none") {
    return [];
  }

  return [
    {
      value: "",
      label: mode === "autoAndManual" ? "自动获取" : "不显示",
    },
    ...apiQuotaSubscriptionLabelsForBaseUrl(baseUrl).map((label) => ({
      value: label,
      label,
    })),
  ];
}
