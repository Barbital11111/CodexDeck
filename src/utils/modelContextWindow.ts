import type { RelayModelCatalogEntry } from "../types/app";

const TOKEN_UNITS = {
  k: 1_000,
  m: 1_000_000,
} as const;

const DEFAULT_RECOMMENDED_CONTEXT_WINDOW = 256_000;

type KnownContextWindowRule = {
  patterns: RegExp[];
  contextWindow: number;
};

const KNOWN_CONTEXT_WINDOW_RULES: KnownContextWindowRule[] = [
  {
    patterns: [/^glm-?5\.?2/i],
    contextWindow: 1_000_000,
  },
  {
    patterns: [/^glm-?5\.?1/i],
    contextWindow: 200_000,
  },
  {
    patterns: [/^minimax-?m3/i],
    contextWindow: 512_000,
  },
];

function normalizeModelId(value: string | null | undefined) {
  return (value ?? "").trim();
}

function isGptModel(model: string) {
  return /^gpt(?:[-_.]|$)/i.test(model);
}

export function resolveKnownModelContextWindow(
  model: string | null | undefined,
  requestModel?: string | null,
) {
  const candidates = [normalizeModelId(model), normalizeModelId(requestModel)].filter(Boolean);
  for (const candidate of candidates) {
    const rule = KNOWN_CONTEXT_WINDOW_RULES.find((item) =>
      item.patterns.some((pattern) => pattern.test(candidate)),
    );
    if (rule) {
      return rule.contextWindow;
    }
  }

  if (candidates.length > 0 && candidates.every((candidate) => !isGptModel(candidate))) {
    return DEFAULT_RECOMMENDED_CONTEXT_WINDOW;
  }
  return null;
}

export function normalizeModelContextWindow(
  contextWindow: number | null | undefined,
  model?: string | null,
  requestModel?: string | null,
) {
  if (typeof contextWindow === "number" && Number.isFinite(contextWindow) && contextWindow > 0) {
    return Math.floor(contextWindow);
  }
  return resolveKnownModelContextWindow(model, requestModel);
}

export function normalizeModelCatalogContextWindows(
  entries: RelayModelCatalogEntry[],
): RelayModelCatalogEntry[] {
  return entries.map((entry) => ({
    ...entry,
    contextWindow: normalizeModelContextWindow(
      entry.contextWindow,
      entry.model,
      entry.requestModel,
    ),
  }));
}

export function parseContextWindowInput(value: string) {
  const normalized = value.trim();
  if (!normalized) {
    return null;
  }

  const match = normalized.match(/^(\d+(?:\.\d+)?)\s*([kKmM])?$/);
  if (!match) {
    return null;
  }

  const parsed = Number(match[1]);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return null;
  }

  const unit = match[2]?.toLowerCase() as keyof typeof TOKEN_UNITS | undefined;
  const multiplier = unit ? TOKEN_UNITS[unit] : 1;
  return Math.floor(parsed * multiplier);
}

export function formatContextWindowInput(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    return "";
  }

  if (value >= TOKEN_UNITS.m && value % TOKEN_UNITS.m === 0) {
    return `${value / TOKEN_UNITS.m}M`;
  }
  if (value >= TOKEN_UNITS.k && value % TOKEN_UNITS.k === 0) {
    return `${value / TOKEN_UNITS.k}K`;
  }
  if (value >= TOKEN_UNITS.k) {
    return `${Number((value / TOKEN_UNITS.k).toFixed(1))}K`;
  }
  return String(Math.floor(value));
}
