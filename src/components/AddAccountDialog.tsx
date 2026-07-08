import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type InputHTMLAttributes,
  type ReactNode,
} from "react";
import {
  Alert,
  Button,
  Card,
  Empty,
  Input,
  Modal,
  Select,
  Space,
  Switch,
  Tabs,
  Tag,
  Typography,
} from "antd";
import {
  ApiOutlined,
  CloudUploadOutlined,
  DeleteOutlined,
  DownloadOutlined,
  FolderOpenOutlined,
  GlobalOutlined,
  KeyOutlined,
  PlusOutlined,
  ReloadOutlined,
  SaveOutlined,
  SortAscendingOutlined,
  SyncOutlined,
} from "@ant-design/icons";
import { useI18n } from "../i18n/I18nProvider";
import type {
  AccountSummary,
  ApiQuotaMode,
  AuthJsonImportInput,
  CreateApiAccountInput,
  PreparedOauthLogin,
  RelayModelCatalogEntry,
  UiSkinMode,
} from "../types/app";
import {
  apiQuotaSubscriptionSelectOptions,
  normalizeApiQuotaSubscriptionName,
  resolveApiQuotaProviderCapability,
} from "../utils/apiQuotaSubscriptions";
import {
  formatContextWindowInput,
  parseContextWindowInput,
} from "../utils/modelContextWindow";
import {
  createModelCatalogRowId,
  createModelCatalogRowIds,
  moveArrayItem,
  moveRelayModelCatalogEntry,
  sortRelayModelCatalog,
} from "../utils/modelCatalog";
import {
  SortableModelCatalogRow,
  SortableModelCatalogScope,
} from "./SortableModelCatalogRow";

type AddAccountRoute = "oauth" | "current" | "upload" | "api";
type ApiProviderPresetId =
  | "custom"
  | "minimax"
  | "xiaomiMimo"
  | "xiaomiMimoTokenPlanChina"
  | "deepseek"
  | "zhipuGlm"
  | "kimi";

type ApiProviderPreset = {
  id: ApiProviderPresetId;
  label: string;
  title?: string;
  subtitle?: string;
  baseUrl: string;
};

type AddAccountDialogProps = {
  open: boolean;
  reauthorizeAccount: AccountSummary | null;
  importingAccounts: boolean;
  oauthWaitingForCallback: boolean;
  onPrepareOauth: () => Promise<PreparedOauthLogin>;
  onOpenOauthPage: (url: string) => Promise<void>;
  onCompleteOauth: (callbackUrl: string) => Promise<void>;
  onCancelOauth: () => Promise<void>;
  onImportCurrentAuth: () => Promise<void>;
  onCreateApiAccount: (input: CreateApiAccountInput) => Promise<void>;
  onProbeApiModels: (baseUrl: string, apiKey: string | null) => Promise<RelayModelCatalogEntry[]>;
  onImportFiles: (items: AuthJsonImportInput[]) => Promise<void>;
  onClose: () => void;
  uiSkinMode: UiSkinMode;
};

const folderPickerAttributes = {
  webkitdirectory: "",
  directory: "",
} as unknown as InputHTMLAttributes<HTMLInputElement>;

function parseTagInput(input: string): string[] {
  return input
    .split(/[\n,，]/)
    .map((item) => item.trim())
    .filter(Boolean)
    .reduce<string[]>((acc, item) => {
      if (acc.some((existing) => existing === item)) {
        return acc;
      }
      acc.push(item);
      return acc;
    }, []);
}

const API_PROVIDER_PRESETS: ApiProviderPreset[] = [
  {
    id: "custom",
    label: "自定义供应商",
    baseUrl: "",
  },
  {
    id: "minimax",
    label: "MiniMax",
    baseUrl: "https://api.minimaxi.com/v1",
  },
  {
    id: "xiaomiMimo",
    label: "Xiaomi MiMo",
    baseUrl: "https://api.xiaomimimo.com/v1",
  },
  {
    id: "xiaomiMimoTokenPlanChina",
    label: "Xiaomi MiMo Token Plan (China)",
    title: "Xiaomi MiMo",
    subtitle: "Token Plan (China)",
    baseUrl: "https://token-plan-cn.xiaomimimo.com/v1",
  },
  {
    id: "deepseek",
    label: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1",
  },
  {
    id: "zhipuGlm",
    label: "Z.AI GLM",
    baseUrl: "https://api.z.ai/api/coding/paas/v4",
  },
  {
    id: "kimi",
    label: "Kimi",
    baseUrl: "https://api.moonshot.cn/v1",
  },
];

function normalizeProviderPresetText(value: string) {
  return value.trim().replace(/\/+$/, "").toLowerCase();
}

function isKnownProviderPresetLabel(value: string) {
  const normalized = normalizeProviderPresetText(value);
  return API_PROVIDER_PRESETS.some(
    (preset) => preset.id !== "custom" && normalizeProviderPresetText(preset.label) === normalized,
  );
}

function isKnownProviderPresetBaseUrl(value: string) {
  const normalized = normalizeProviderPresetText(value);
  return API_PROVIDER_PRESETS.some(
    (preset) =>
      preset.baseUrl && normalizeProviderPresetText(preset.baseUrl) === normalized,
  );
}

function firstEnabledModelName(entries: RelayModelCatalogEntry[]) {
  return (
    entries.find((entry) => entry.enabled !== false && entry.model.trim())?.model.trim() ??
    entries.find((entry) => entry.model.trim())?.model.trim() ??
    ""
  );
}

function resolvePreferredModelName(
  preferredModelName: string,
  entries: RelayModelCatalogEntry[],
) {
  const preferred = preferredModelName.trim();
  if (preferred && entries.some((entry) => entry.model.trim() === preferred)) {
    return preferred;
  }
  return firstEnabledModelName(entries);
}

function uniqueModelName(entries: RelayModelCatalogEntry[], fallbackModel: string) {
  const normalizedFallback = fallbackModel.trim() || "custom-model";
  const existing = new Set(
    entries.map((entry) => entry.model.trim()).filter((model) => model.length > 0),
  );
  if (!existing.has(normalizedFallback)) {
    return normalizedFallback;
  }

  let index = 2;
  let candidate = `${normalizedFallback}-${index}`;
  while (existing.has(candidate)) {
    index += 1;
    candidate = `${normalizedFallback}-${index}`;
  }
  return candidate;
}

function addAccountRouteIcon(route: AddAccountRoute) {
  if (route === "oauth") {
    return <GlobalOutlined />;
  }

  if (route === "current") {
    return <SyncOutlined />;
  }

  if (route === "api") {
    return <ApiOutlined />;
  }

  return <DownloadOutlined />;
}

export function AddAccountDialog({
  open,
  reauthorizeAccount,
  importingAccounts,
  oauthWaitingForCallback,
  onPrepareOauth,
  onOpenOauthPage,
  onCompleteOauth,
  onCancelOauth,
  onImportCurrentAuth,
  onCreateApiAccount,
  onProbeApiModels,
  onImportFiles,
  onClose,
  uiSkinMode,
}: AddAccountDialogProps) {
  const { copy } = useI18n();
  const [activeRoute, setActiveRoute] = useState<AddAccountRoute>(
    reauthorizeAccount ? "oauth" : "api",
  );
  const [selectedFiles, setSelectedFiles] = useState<File[]>([]);
  const [readingFiles, setReadingFiles] = useState(false);
  const [pendingRoute, setPendingRoute] = useState<AddAccountRoute | null>(null);
  const [preparingOauth, setPreparingOauth] = useState(false);
  const [oauthLogin, setOauthLogin] = useState<PreparedOauthLogin | null>(null);
  const [oauthCallbackUrl, setOauthCallbackUrl] = useState("");
  const [apiForm, setApiForm] = useState<CreateApiAccountInput>({
    label: "",
    baseUrl: "",
    apiKey: "",
    modelName: "",
    tags: [],
    forceSave: false,
    modelCatalog: [],
    balanceDisplayEnabled: false,
    apiQuotaMode: "apiOnly",
    apiQuotaSubscriptionName: null,
    platformLoginEmail: "",
    platformLoginPassword: "",
  });
  const [apiTagsInput, setApiTagsInput] = useState("");
  const [apiInlineError, setApiInlineError] = useState<string | null>(null);
  const [apiCanForceSave, setApiCanForceSave] = useState(false);
  const [apiModelProbePending, setApiModelProbePending] = useState(false);
  const [apiModelSortMode, setApiModelSortMode] = useState(false);
  const [apiModelCatalogRowIds, setApiModelCatalogRowIds] = useState<string[]>([]);
  const [apiProviderPresetId, setApiProviderPresetId] =
    useState<ApiProviderPresetId>("custom");
  const oauthAutoPrepareAttemptedRef = useRef(false);
  const oauthPrepareRequestRef = useRef(0);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const folderInputRef = useRef<HTMLInputElement>(null);

  const busy = importingAccounts || readingFiles;
  const actionLocked = busy || preparingOauth;
  const closeBlocked = busy;
  const isClassicSkin = uiSkinMode === "classic";

  const resetOauthState = useCallback(
    (cancelRemote: boolean) => {
      oauthAutoPrepareAttemptedRef.current = false;
      oauthPrepareRequestRef.current += 1;
      setPreparingOauth(false);
      setOauthLogin(null);
      setOauthCallbackUrl("");
      if (cancelRemote) {
        void onCancelOauth();
      }
    },
    [onCancelOauth],
  );

  useEffect(() => {
    if (open && reauthorizeAccount && activeRoute !== "oauth") {
      setActiveRoute("oauth");
      return;
    }

    if (!open) {
      setActiveRoute(reauthorizeAccount ? "oauth" : "api");
      setSelectedFiles([]);
      setReadingFiles(false);
      setPendingRoute(null);
      setApiForm({
        label: "",
        baseUrl: "",
        apiKey: "",
        modelName: "",
        modelCatalog: [],
        tags: [],
        forceSave: false,
        balanceDisplayEnabled: false,
        apiQuotaMode: "apiOnly",
        apiQuotaSubscriptionName: null,
        platformLoginEmail: "",
        platformLoginPassword: "",
      });
      setApiTagsInput("");
      setApiInlineError(null);
      setApiCanForceSave(false);
      setApiModelProbePending(false);
      setApiModelSortMode(false);
      setApiModelCatalogRowIds([]);
      setApiProviderPresetId("custom");
      resetOauthState(true);
      return;
    }

    document.documentElement.classList.add("addAccountDialogRootLocked");
    document.body.classList.add("addAccountDialogBodyLocked");
    return () => {
      document.documentElement.classList.remove("addAccountDialogRootLocked");
      document.body.classList.remove("addAccountDialogBodyLocked");
    };
  }, [open, reauthorizeAccount, activeRoute, resetOauthState]);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !closeBlocked) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [
    closeBlocked,
    onClose,
    open,
  ]);

  const routeOptions = useMemo(
    () => {
      const oauthRoute = {
        id: "oauth" as const,
        label: copy.addAccount.oauthTab,
        description: reauthorizeAccount
          ? copy.addAccount.reauthorizeOauthDescription
          : copy.addAccount.oauthDescription,
      };
      if (reauthorizeAccount) {
        return [oauthRoute];
      }
      return [
        oauthRoute,
        {
          id: "current" as const,
          label: copy.addAccount.currentTab,
          description: copy.addAccount.currentDescription,
        },
        {
          id: "upload" as const,
          label: copy.addAccount.uploadTab,
          description: copy.addAccount.uploadDescription,
        },
        {
          id: "api" as const,
          label: copy.addAccount.apiTab,
          description: copy.addAccount.apiDescription,
        },
      ];
    },
    [copy.addAccount, reauthorizeAccount],
  );

  const dialogTitle = reauthorizeAccount
    ? copy.addAccount.reauthorizeDialogTitle
    : copy.addAccount.dialogTitle;
  const dialogSubtitle = reauthorizeAccount
    ? copy.addAccount.reauthorizeDialogSubtitle(reauthorizeAccount.label)
    : copy.addAccount.dialogSubtitle;
  const apiQuotaModeOptions = useMemo(
    () => [
      {
        value: "apiOnly" as const,
        label: copy.accountCard.apiQuotaModeApiOnly,
      },
      {
        value: "platformBasic" as const,
        label: copy.accountCard.apiQuotaModePlatformBasic,
      },
      {
        value: "platformSubscription" as const,
        label: copy.accountCard.apiQuotaModePlatformSubscription,
      },
      {
        value: "admin" as const,
        label: copy.accountCard.apiQuotaModeAdmin,
      },
    ],
    [copy.accountCard],
  );
  const selectedApiProviderPreset =
    API_PROVIDER_PRESETS.find((preset) => preset.id === apiProviderPresetId) ??
    API_PROVIDER_PRESETS[0];
  const apiQuotaCapability = resolveApiQuotaProviderCapability(
    apiForm.baseUrl.trim() || selectedApiProviderPreset.baseUrl,
  );
  const apiQuotaSubscriptionLabelMode = apiQuotaCapability.subscriptionLabelMode;
  const apiQuotaBalancePresetLocked = apiQuotaCapability.balanceDisplayControl === "preset";
  const apiQuotaBalanceEnabled = apiQuotaBalancePresetLocked
    ? apiQuotaCapability.balanceDisplayEnabled
    : Boolean(apiForm.balanceDisplayEnabled);
  const apiQuotaSubscriptionOptions = useMemo(
    () => apiQuotaSubscriptionSelectOptions(apiQuotaSubscriptionLabelMode, apiForm.baseUrl),
    [apiForm.baseUrl, apiQuotaSubscriptionLabelMode],
  );
  const preferredModelOptions = useMemo(() => {
    const seen = new Set<string>();
    return (apiForm.modelCatalog ?? [])
      .map((entry) => ({
        value: entry.model.trim(),
        label: entry.displayName?.trim()
          ? `${entry.displayName.trim()} (${entry.model.trim()})`
          : entry.model.trim(),
      }))
      .filter((option) => {
        if (!option.value || seen.has(option.value)) {
          return false;
        }
        seen.add(option.value);
        return true;
      });
  }, [apiForm.modelCatalog]);
  const selectedSummary = useMemo(() => {
    if (selectedFiles.length === 0) {
      return copy.addAccount.uploadNoJsonFiles;
    }

    const firstPath = selectedFiles[0]?.webkitRelativePath || selectedFiles[0]?.name || "";
    if (selectedFiles.length === 1) {
      return firstPath;
    }

    return copy.addAccount.uploadFileSummary(firstPath, selectedFiles.length);
  }, [copy.addAccount, selectedFiles]);

  const selectedPreview = useMemo(
    () =>
      selectedFiles.slice(0, 4).map((file) => ({
        key: file.webkitRelativePath || file.name,
        label: file.webkitRelativePath || file.name,
      })),
    [selectedFiles],
  );
  const apiSubmitDisabled =
    actionLocked ||
    apiForm.label.trim() === "" ||
    apiForm.baseUrl.trim() === "" ||
    apiForm.apiKey.trim() === "" ||
    apiForm.modelName.trim() === "";

  const handlePrepareOauth = useCallback(async () => {
    if (busy || preparingOauth) {
      return;
    }

    const requestId = oauthPrepareRequestRef.current + 1;
    oauthPrepareRequestRef.current = requestId;
    setPreparingOauth(true);
    try {
      const prepared = await onPrepareOauth();
      if (oauthPrepareRequestRef.current !== requestId) {
        return;
      }
      setOauthLogin(prepared);
      setOauthCallbackUrl("");
    } finally {
      if (oauthPrepareRequestRef.current === requestId) {
        setPreparingOauth(false);
      }
    }
  }, [busy, onPrepareOauth, preparingOauth]);

  useEffect(() => {
    if (!open || activeRoute === "oauth") {
      return;
    }

    if (!oauthLogin && !oauthWaitingForCallback && oauthCallbackUrl.trim() === "" && !preparingOauth) {
      return;
    }

    resetOauthState(true);
  }, [
    activeRoute,
    oauthCallbackUrl,
    oauthLogin,
    oauthWaitingForCallback,
    open,
    preparingOauth,
    resetOauthState,
  ]);

  useEffect(() => {
    if (!open) {
      oauthAutoPrepareAttemptedRef.current = false;
      return;
    }

    if (activeRoute !== "oauth") {
      oauthAutoPrepareAttemptedRef.current = false;
      return;
    }

    if (busy || preparingOauth || oauthLogin || oauthAutoPrepareAttemptedRef.current) {
      return;
    }

    oauthAutoPrepareAttemptedRef.current = true;
    void handlePrepareOauth().catch(() => {});
  }, [activeRoute, busy, handlePrepareOauth, oauthLogin, open, preparingOauth]);

  if (!open) {
    return null;
  }

  const mergeSelectedFiles = (incomingFiles: File[]) => {
    setSelectedFiles((current) => {
      const nextMap = new Map<string, File>();
      for (const file of current) {
        const key = file.webkitRelativePath || file.name;
        nextMap.set(key, file);
      }
      for (const file of incomingFiles) {
        const key = file.webkitRelativePath || file.name;
        nextMap.set(key, file);
      }
      return Array.from(nextMap.entries())
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([, file]) => file);
    });
  };

  const handleFilesPicked = (event: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.currentTarget.files ?? []).filter((file) =>
      file.name.toLowerCase().endsWith(".json"),
    );
    if (files.length > 0) {
      mergeSelectedFiles(files);
    }
    event.currentTarget.value = "";
  };

  const handleCompleteOauth = async () => {
    if (actionLocked || oauthCallbackUrl.trim() === "") {
      return;
    }

    setPendingRoute("oauth");
    try {
      await onCompleteOauth(oauthCallbackUrl.trim());
    } finally {
      setPendingRoute(null);
    }
  };

  const handleOpenOauthPage = async () => {
    if (!oauthLogin || actionLocked) {
      return;
    }
    await onOpenOauthPage(oauthLogin.authUrl);
  };

  const handleImportCurrentAuth = async () => {
    if (actionLocked) {
      return;
    }

    setPendingRoute("current");
    try {
      await onImportCurrentAuth();
    } finally {
      setPendingRoute(null);
    }
  };

  const handleImportFiles = async () => {
    if (actionLocked || selectedFiles.length === 0) {
      return;
    }

    setPendingRoute("upload");
    setReadingFiles(true);
    try {
      const items = await Promise.all(
        selectedFiles.map(async (file) => ({
          source: file.webkitRelativePath || file.name,
          content: await file.text(),
          label: null,
        })),
      );
      await onImportFiles(items);
    } finally {
      setReadingFiles(false);
      setPendingRoute(null);
    }
  };

  const handleApiFieldChange =
    (field: keyof CreateApiAccountInput) =>
    (event: ChangeEvent<HTMLInputElement>) => {
      const value = event.target.value;
      setApiForm((current) => {
        const next = {
          ...current,
          [field]: value,
          forceSave: false,
        };
        const nextCapability = resolveApiQuotaProviderCapability(value);
        if (field === "baseUrl" && nextCapability.subscriptionLabelMode === "none") {
          return {
            ...next,
            balanceDisplayEnabled:
              nextCapability.balanceDisplayControl === "preset"
                ? nextCapability.balanceDisplayEnabled
                : next.balanceDisplayEnabled,
            apiQuotaMode:
              nextCapability.balanceDisplayControl === "preset"
                ? nextCapability.defaultQuotaMode
                : next.apiQuotaMode,
            apiQuotaSubscriptionName: null,
          };
        }
        if (field === "baseUrl" && nextCapability.balanceDisplayControl === "preset") {
          return {
            ...next,
            balanceDisplayEnabled: nextCapability.balanceDisplayEnabled,
            apiQuotaMode: nextCapability.defaultQuotaMode,
          };
        }
        return next;
      });
      setApiInlineError(null);
      setApiCanForceSave(false);
    };

  const updateApiModelCatalogEntry = (
    index: number,
    updater: (entry: RelayModelCatalogEntry) => RelayModelCatalogEntry,
  ) => {
    setApiForm((current) => {
      const currentCatalog = current.modelCatalog ?? [];
      const previousEntry = currentCatalog[index];
      const nextCatalog = currentCatalog.map((entry, entryIndex) =>
        entryIndex === index ? updater(entry) : entry,
      );
      const isUpdatingPreferred = previousEntry?.model.trim() === current.modelName.trim();
      const preferredCandidate =
        isUpdatingPreferred && nextCatalog[index]?.enabled === false
          ? ""
          : isUpdatingPreferred
            ? nextCatalog[index]?.model ?? ""
            : current.modelName;
      return {
        ...current,
        modelName: resolvePreferredModelName(preferredCandidate, nextCatalog),
        modelCatalog: nextCatalog,
        forceSave: false,
      };
    });
    setApiInlineError(null);
    setApiCanForceSave(false);
  };

  const handleApiProviderPresetChange = (presetId: ApiProviderPresetId) => {
    const preset =
      API_PROVIDER_PRESETS.find((candidate) => candidate.id === presetId) ??
      API_PROVIDER_PRESETS[0];
    setApiProviderPresetId(preset.id);
    setApiModelSortMode(false);
    setApiModelCatalogRowIds([]);
    if (preset.id === "custom") {
      setApiForm((current) => ({
        ...current,
        label: "",
        baseUrl: "",
        apiKey: "",
        modelName: "",
        modelCatalog: [],
        balanceDisplayEnabled: false,
        apiQuotaMode: "apiOnly",
        apiQuotaSubscriptionName: null,
        platformLoginEmail: "",
        platformLoginPassword: "",
        forceSave: false,
      }));
      setApiInlineError(null);
      setApiCanForceSave(false);
      return;
    }

    setApiForm((current) => {
      const shouldReplaceLabel =
        current.label.trim() === "" || isKnownProviderPresetLabel(current.label);
      const shouldReplaceBaseUrl =
        current.baseUrl.trim() === "" || isKnownProviderPresetBaseUrl(current.baseUrl);
      const nextBaseUrl = shouldReplaceBaseUrl && preset.baseUrl ? preset.baseUrl : current.baseUrl;
      const presetCapability = resolveApiQuotaProviderCapability(nextBaseUrl);
      return {
        ...current,
        label: shouldReplaceLabel ? preset.label : current.label,
        baseUrl: nextBaseUrl,
        modelName: "",
        modelCatalog: [],
        balanceDisplayEnabled: presetCapability.balanceDisplayEnabled,
        apiQuotaMode: presetCapability.defaultQuotaMode,
        apiQuotaSubscriptionName:
          presetCapability.subscriptionLabelMode === "none"
            ? null
            : current.apiQuotaSubscriptionName,
        forceSave: false,
      };
    });
    setApiInlineError(null);
    setApiCanForceSave(false);
  };

  const handleProbeApiModels = async () => {
    if (apiModelProbePending || !apiForm.baseUrl.trim() || !apiForm.apiKey.trim()) {
      return;
    }

    setApiModelProbePending(true);
    setApiInlineError(null);
    try {
      const probed = await onProbeApiModels(apiForm.baseUrl, apiForm.apiKey);
      const nextCatalog = sortRelayModelCatalog(
        probed.map((entry) => ({
          ...entry,
          enabled: entry.enabled ?? true,
        })),
      );
      const firstEnabled = nextCatalog.find((entry) => entry.enabled) ?? nextCatalog[0];
      setApiForm((current) => ({
        ...current,
        modelName: firstEnabled?.model || "",
        modelCatalog: nextCatalog,
        forceSave: false,
      }));
      setApiModelSortMode(false);
      setApiModelCatalogRowIds(createModelCatalogRowIds(nextCatalog.length, "api-model"));
      setApiCanForceSave(false);
    } catch (error) {
      setApiInlineError(error instanceof Error ? error.message : String(error));
    } finally {
      setApiModelProbePending(false);
    }
  };

  const handleSortApiModelCatalog = () => {
    setApiModelSortMode((current) => !current);
  };

  const handleMoveApiModelCatalog = (fromIndex: number, toIndex: number) => {
    setApiForm((current) => {
      const nextCatalog = moveRelayModelCatalogEntry(
        current.modelCatalog ?? [],
        fromIndex,
        toIndex,
      );
      return {
        ...current,
        modelName: resolvePreferredModelName(current.modelName, nextCatalog),
        modelCatalog: nextCatalog,
        forceSave: false,
      };
    });
    setApiModelCatalogRowIds((current) => moveArrayItem(current, fromIndex, toIndex));
    setApiInlineError(null);
    setApiCanForceSave(false);
  };

  const handleAddApiModelRow = () => {
    setApiForm((current) => {
      const currentCatalog = current.modelCatalog ?? [];
      const model = uniqueModelName(currentCatalog, current.modelName || "custom-model");
      return {
        ...current,
        modelName: model,
        modelCatalog: [
          ...currentCatalog,
          {
            model,
            displayName: null,
            requestModel: null,
            contextWindow: null,
            enabled: true,
          },
        ],
        forceSave: false,
      };
    });
    setApiModelCatalogRowIds((current) => [
      ...current,
      createModelCatalogRowId("api-model"),
    ]);
    setApiInlineError(null);
    setApiCanForceSave(false);
  };

  const handleRemoveApiModelRow = (index: number) => {
    setApiForm((current) => {
      const nextCatalog = (current.modelCatalog ?? []).filter(
        (_, entryIndex) => entryIndex !== index,
      );
      if (nextCatalog.length < 2) {
        setApiModelSortMode(false);
      }
      setApiModelCatalogRowIds((currentRowIds) =>
        currentRowIds.filter((_, entryIndex) => entryIndex !== index),
      );
      return {
        ...current,
        modelName: resolvePreferredModelName(current.modelName, nextCatalog),
        modelCatalog: nextCatalog,
        forceSave: false,
      };
    });
  };

  const handleCreateApiAccount = async (forceSave: boolean) => {
    if (actionLocked) {
      return;
    }

    const apiQuotaMode = apiQuotaBalancePresetLocked
      ? apiQuotaCapability.defaultQuotaMode
      : apiForm.apiQuotaMode ?? "apiOnly";
    const hasPlatformLogin = Boolean(
      apiForm.platformLoginEmail?.trim() && apiForm.platformLoginPassword?.trim(),
    );
    const resolvedApiQuotaMode: ApiQuotaMode =
      apiQuotaBalanceEnabled && hasPlatformLogin && apiQuotaMode === "apiOnly"
        ? "platformBasic"
        : apiQuotaMode;
    const resolvedSubscriptionName =
      apiQuotaBalanceEnabled && apiQuotaSubscriptionLabelMode !== "none"
        ? normalizeApiQuotaSubscriptionName(apiForm.apiQuotaSubscriptionName)
        : null;
    setPendingRoute("api");
    setApiInlineError(null);
    try {
      await onCreateApiAccount({
        ...apiForm,
        tags: parseTagInput(apiTagsInput),
        forceSave,
        balanceDisplayEnabled: apiQuotaBalanceEnabled,
        apiQuotaMode: resolvedApiQuotaMode,
        apiQuotaSubscriptionName: resolvedSubscriptionName,
        platformLoginEmail:
          resolvedApiQuotaMode === "apiOnly" ? "" : apiForm.platformLoginEmail,
        platformLoginPassword:
          resolvedApiQuotaMode === "apiOnly" ? "" : apiForm.platformLoginPassword,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setApiInlineError(message);
      setApiCanForceSave(!forceSave);
    } finally {
      setPendingRoute(null);
    }
  };

  const renderApiProviderPresets = () => (
    <div className="addAccountProviderGrid">
      {API_PROVIDER_PRESETS.map((preset) => (
        <Button
          key={preset.id}
          className="addAccountProviderPreset"
          type={preset.id === apiProviderPresetId ? "primary" : "default"}
          onClick={() => handleApiProviderPresetChange(preset.id)}
          disabled={actionLocked}
        >
          <span className="addAccountProviderPresetLabel">
            <span>{preset.title ?? preset.label}</span>
            {preset.subtitle ? <small>{preset.subtitle}</small> : null}
          </span>
        </Button>
      ))}
    </div>
  );

  const renderApiConnectionFields = () => (
    <div className="addAccountModalFormGrid">
      <div className="addAccountModalField">
        <Typography.Text strong>{copy.addAccount.apiNameLabel}</Typography.Text>
        <Input
          value={apiForm.label}
          onChange={handleApiFieldChange("label")}
          placeholder={copy.addAccount.apiNamePlaceholder}
          spellCheck={false}
        />
      </div>
      <div className="addAccountModalField">
        <Typography.Text strong>{copy.addAccount.apiBaseUrlLabel}</Typography.Text>
        <Input
          value={apiForm.baseUrl}
          onChange={handleApiFieldChange("baseUrl")}
          placeholder={copy.addAccount.apiBaseUrlPlaceholder}
          spellCheck={false}
        />
      </div>
      <div className="addAccountModalField">
        <Typography.Text strong>{copy.addAccount.apiKeyLabel}</Typography.Text>
        <Input.Password
          className="addAccountApiKeyInput"
          value={apiForm.apiKey}
          onChange={handleApiFieldChange("apiKey")}
          placeholder={copy.addAccount.apiKeyPlaceholder}
          spellCheck={false}
        />
      </div>
      <div className="addAccountModalField">
        <Typography.Text strong>{copy.addAccount.apiModelLabel}</Typography.Text>
        <Select
          value={apiForm.modelName || undefined}
          options={preferredModelOptions}
          placeholder={copy.addAccount.apiModelPlaceholder}
          onChange={(modelName) => {
            setApiForm((current) => ({
              ...current,
              modelName,
              forceSave: false,
            }));
            setApiInlineError(null);
            setApiCanForceSave(false);
          }}
          disabled={preferredModelOptions.length === 0}
        />
      </div>
    </div>
  );

  const renderApiTagsField = () => (
    <div className="addAccountModalField addAccountModalFieldFull">
      <Typography.Text strong>{copy.addAccount.apiTagsLabel}</Typography.Text>
      <Input
        value={apiTagsInput}
        onChange={(event) => {
          setApiTagsInput(event.target.value);
          setApiInlineError(null);
          setApiCanForceSave(false);
        }}
        placeholder={copy.addAccount.apiTagsPlaceholder}
        spellCheck={false}
      />
      <Typography.Text type="secondary">支持逗号分隔，保存后会同步到这个 API 账号组。</Typography.Text>
    </div>
  );

  const renderApiModelControls = () => (
    <div className="addAccountModalToolbar">
      <Space wrap>
        <Button
          icon={<ReloadOutlined spin={apiModelProbePending} />}
          onClick={() => void handleProbeApiModels()}
          disabled={
            actionLocked ||
            apiModelProbePending ||
            !apiForm.baseUrl.trim() ||
            !apiForm.apiKey.trim()
          }
        >
          {apiModelProbePending ? "探测中" : "探测模型"}
        </Button>
        <Button
          icon={<SortAscendingOutlined />}
          type={apiModelSortMode ? "primary" : "default"}
          onClick={handleSortApiModelCatalog}
          disabled={actionLocked || (apiForm.modelCatalog ?? []).length < 2}
          aria-pressed={apiModelSortMode}
        >
          排序
        </Button>
        <Button icon={<PlusOutlined />} onClick={handleAddApiModelRow} disabled={actionLocked}>
          添加模型
        </Button>
      </Space>
    </div>
  );

  const renderApiModelCatalog = () =>
    (apiForm.modelCatalog ?? []).length > 0 ? (
      <div className="addAccountModelTable">
        <div className={`addAccountModelHeader ${apiModelSortMode ? "isSorting" : ""}`}>
          {apiModelSortMode ? <span /> : null}
          <span>显示</span>
          <span>菜单模型 ID</span>
          <span>显示名称</span>
          <span>实际请求模型</span>
          <span>上下文</span>
          <span />
        </div>
        <SortableModelCatalogScope
          enabled={apiModelSortMode}
          items={apiModelCatalogRowIds}
          onMove={handleMoveApiModelCatalog}
        >
          {(apiForm.modelCatalog ?? []).map((entry, index) => (
            <SortableModelCatalogRow
              id={apiModelCatalogRowIds[index] ?? `api-model-${index}`}
              key={apiModelCatalogRowIds[index] ?? `api-model-${index}`}
              sortingEnabled={apiModelSortMode}
            >
              {(sortHandle) => (
                <div className={`addAccountModelRow ${apiModelSortMode ? "isSorting" : ""}`}>
                  {apiModelSortMode ? sortHandle : null}
                  <Switch
                    className="addAccountModelDisplaySwitch"
                    size="small"
                    checked={entry.enabled}
                    onChange={(checked) =>
                      updateApiModelCatalogEntry(index, (current) => ({
                        ...current,
                        enabled: checked,
                      }))
                    }
                  />
                  <Input
                    value={entry.model}
                    placeholder="菜单模型 ID"
                    onChange={(event) =>
                      updateApiModelCatalogEntry(index, (current) => ({
                        ...current,
                        model: event.target.value,
                      }))
                    }
                    spellCheck={false}
                  />
                  <Input
                    value={entry.displayName ?? ""}
                    placeholder="显示名称"
                    onChange={(event) =>
                      updateApiModelCatalogEntry(index, (current) => ({
                        ...current,
                        displayName: event.target.value,
                      }))
                    }
                    spellCheck={false}
                  />
                  <Input
                    value={entry.requestModel ?? ""}
                    placeholder="路由模式可填"
                    onChange={(event) =>
                      updateApiModelCatalogEntry(index, (current) => ({
                        ...current,
                        requestModel: event.target.value,
                      }))
                    }
                    spellCheck={false}
                  />
                  <Input
                    value={formatContextWindowInput(entry.contextWindow)}
                    placeholder="256K"
                    onChange={(event) =>
                      updateApiModelCatalogEntry(index, (current) => ({
                        ...current,
                        contextWindow: parseContextWindowInput(event.target.value),
                      }))
                    }
                    spellCheck={false}
                  />
                  <Button
                    icon={<DeleteOutlined />}
                    onClick={() => handleRemoveApiModelRow(index)}
                    disabled={actionLocked}
                  />
                </div>
              )}
            </SortableModelCatalogRow>
          ))}
        </SortableModelCatalogScope>
      </div>
    ) : isClassicSkin ? (
      <div className="addAccountModelEmpty">未设置时会只显示默认模型。</div>
    ) : (
      <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无模型" />
    );

  const renderApiQuotaControls = () => (
    <Space orientation="vertical" size={14} className="addAccountModalStack">
      {apiQuotaBalancePresetLocked ? (
        <div className="addAccountModalPresetQuotaState">
          {apiQuotaBalanceEnabled ? "余额显示已启用" : "暂不支持余额显示"}
        </div>
      ) : (
        <div className="addAccountModalSwitchRow">
          <div>
            <Typography.Text strong>{copy.addAccount.apiQuotaToggleLabel}</Typography.Text>
          </div>
          <Switch
            checked={Boolean(apiForm.balanceDisplayEnabled)}
            onChange={(checked) => {
              setApiForm((current) => ({
                ...current,
                balanceDisplayEnabled: checked,
                apiQuotaMode: checked ? current.apiQuotaMode ?? "apiOnly" : "apiOnly",
                apiQuotaSubscriptionName: checked ? current.apiQuotaSubscriptionName : null,
                platformLoginEmail: checked ? current.platformLoginEmail : "",
                platformLoginPassword: checked ? current.platformLoginPassword : "",
                forceSave: false,
              }));
              setApiInlineError(null);
              setApiCanForceSave(false);
            }}
          />
        </div>
      )}

      {apiQuotaBalanceEnabled ? (
        <div className="addAccountModalFormGrid">
          {!apiQuotaBalancePresetLocked ? (
            <div className="addAccountModalField addAccountModalFieldFull">
              <Typography.Text strong>{copy.accountCard.apiQuotaModeLabel}</Typography.Text>
              <Select
                value={apiForm.apiQuotaMode ?? "apiOnly"}
                options={apiQuotaModeOptions}
                onChange={(value: ApiQuotaMode) => {
                  setApiForm((current) => ({
                    ...current,
                    apiQuotaMode: value,
                    platformLoginEmail:
                      value === "apiOnly" ? "" : current.platformLoginEmail,
                    platformLoginPassword:
                      value === "apiOnly" ? "" : current.platformLoginPassword,
                    forceSave: false,
                  }));
                  setApiInlineError(null);
                  setApiCanForceSave(false);
                }}
              />
            </div>
          ) : null}
          {apiQuotaSubscriptionLabelMode !== "none" ? (
            <div className="addAccountModalField addAccountModalFieldFull">
              <Typography.Text strong>套餐标签</Typography.Text>
              <Select
                value={apiForm.apiQuotaSubscriptionName ?? ""}
                options={apiQuotaSubscriptionOptions}
                onChange={(value) => {
                  setApiForm((current) => ({
                    ...current,
                    apiQuotaSubscriptionName: normalizeApiQuotaSubscriptionName(value),
                    forceSave: false,
                  }));
                  setApiInlineError(null);
                  setApiCanForceSave(false);
                }}
              />
            </div>
          ) : null}
          <div className="addAccountModalField">
            <Typography.Text strong>{copy.addAccount.apiPlatformEmailLabel}</Typography.Text>
            <Input
              value={apiForm.platformLoginEmail ?? ""}
              onChange={handleApiFieldChange("platformLoginEmail")}
              placeholder={copy.addAccount.apiPlatformEmailPlaceholder}
              spellCheck={false}
            />
          </div>
          <div className="addAccountModalField">
            <Typography.Text strong>{copy.addAccount.apiPlatformPasswordLabel}</Typography.Text>
            <Input.Password
              value={apiForm.platformLoginPassword ?? ""}
              onChange={handleApiFieldChange("platformLoginPassword")}
              placeholder={copy.addAccount.apiPlatformPasswordPlaceholder}
              spellCheck={false}
            />
          </div>
        </div>
      ) : null}
    </Space>
  );

  const renderApiError = () =>
    apiInlineError ? (
      <Alert
        type="error"
        showIcon
        title={copy.addAccount.apiValidationFailed}
        description={apiInlineError}
      />
    ) : null;

  const renderApiActions = () => (
    <div className="addAccountModalActions">
      <Button
        type="primary"
        icon={<KeyOutlined />}
        onClick={() => void handleCreateApiAccount(false)}
        disabled={apiSubmitDisabled}
        loading={pendingRoute === "api"}
      >
        {copy.addAccount.apiValidateAndSave}
      </Button>
      {apiCanForceSave ? (
        <Button onClick={() => void handleCreateApiAccount(true)} disabled={actionLocked}>
          {copy.addAccount.apiForceSave}
        </Button>
      ) : null}
    </div>
  );

  const renderSectionFrame = (
    className: string,
    title: string,
    children: ReactNode,
    description?: string,
  ) => (
    <section className={`addAccountClassicSection ${className}`}>
      <div className="addAccountClassicSectionHead">
        <Typography.Text strong>{title}</Typography.Text>
        {description ? <Typography.Text type="secondary">{description}</Typography.Text> : null}
      </div>
      {children}
    </section>
  );

  return (
    <Modal
      open={open}
      title={
        <div className="addAccountModalTitle">
          <Typography.Title level={4}>{dialogTitle}</Typography.Title>
          <Typography.Text type="secondary">{dialogSubtitle}</Typography.Text>
        </div>
      }
      width={isClassicSkin ? 980 : 1040}
      centered
      destroyOnHidden
      mask={{ closable: false }}
      keyboard={!closeBlocked}
      onCancel={closeBlocked ? undefined : onClose}
      footer={null}
      className="addAccountModal"
      data-skin={uiSkinMode}
      styles={{
        body: {
          maxHeight: isClassicSkin ? "calc(100dvh - 92px)" : "calc(100dvh - 160px)",
          overflow: "auto",
        },
      }}
    >
      {isClassicSkin ? (
        <div className="addAccountRouteGrid" role="tablist" aria-label={dialogTitle}>
          {routeOptions.map((route) => (
            <button
              key={route.id}
              type="button"
              className={`addAccountRouteCard ${activeRoute === route.id ? "isActive" : ""}`}
              onClick={() => setActiveRoute(route.id)}
              disabled={busy}
              role="tab"
              aria-selected={activeRoute === route.id}
            >
              <span className="addAccountRouteIcon">{addAccountRouteIcon(route.id)}</span>
              <span className="addAccountRouteCopy">
                <strong>{route.label}</strong>
                <small>{route.description}</small>
              </span>
            </button>
          ))}
        </div>
      ) : (
        <Tabs
          className="addAccountModalTabs"
          activeKey={activeRoute}
          onChange={(key) => setActiveRoute(key as AddAccountRoute)}
          items={routeOptions.map((route) => ({
            key: route.id,
            icon: addAccountRouteIcon(route.id),
            label: route.label,
            disabled: busy,
            children: (
              <div className="addAccountModalTabBody">
                <Typography.Text type="secondary">{route.description}</Typography.Text>
              </div>
            ),
          }))}
        />
      )}

      {activeRoute === "oauth" ? (
        <div className="addAccountModalFlow">
          <Card
            className="addAccountModalCard"
            variant="outlined"
            title={copy.addAccount.oauthTab}
            extra={
              oauthWaitingForCallback ? (
                <Tag color="processing">{copy.addAccount.oauthListening}</Tag>
              ) : null
            }
          >
            <Space orientation="vertical" size={14} className="addAccountModalStack">
              <Button
                type="primary"
                icon={<GlobalOutlined />}
                onClick={() => void handleOpenOauthPage()}
                disabled={actionLocked || !oauthLogin}
              >
                {copy.addAccount.oauthOpenBrowser}
              </Button>
              <div className="addAccountModalField">
                <Typography.Text strong>{copy.addAccount.oauthLinkLabel}</Typography.Text>
                <Input value={oauthLogin?.authUrl ?? ""} readOnly />
              </div>
              <div className="addAccountModalField">
                <Typography.Text strong>{copy.addAccount.oauthCallbackLabel}</Typography.Text>
                <Input.TextArea
                  value={oauthCallbackUrl}
                  onChange={(event) => setOauthCallbackUrl(event.target.value)}
                  placeholder={copy.addAccount.oauthCallbackPlaceholder}
                  rows={4}
                  spellCheck={false}
                />
              </div>
              <Space wrap>
                <Button
                  type="primary"
                  icon={<SaveOutlined />}
                  onClick={() => void handleCompleteOauth()}
                  disabled={actionLocked || oauthCallbackUrl.trim() === ""}
                  loading={pendingRoute === "oauth" || importingAccounts}
                >
                  {reauthorizeAccount
                    ? copy.addAccount.reauthorizeParseCallback
                    : copy.addAccount.oauthParseCallback}
                </Button>
                {!oauthLogin ? (
                  <Tag icon={<ReloadOutlined spin={preparingOauth} />}>
                    {copy.addAccount.oauthPreparing}
                  </Tag>
                ) : null}
              </Space>
            </Space>
          </Card>
        </div>
      ) : null}

      {activeRoute === "current" ? (
        <div className="addAccountModalFlow">
          <Card className="addAccountModalCard" variant="outlined" title={copy.addAccount.currentTab}>
            <Space orientation="vertical" size={16} className="addAccountModalStack">
              <Alert
                type="info"
                showIcon
                title="AUTH.JSON"
                description={copy.addAccount.currentDescription}
              />
              <Button
                type="primary"
                icon={<SyncOutlined />}
                onClick={() => void handleImportCurrentAuth()}
                disabled={actionLocked}
                loading={pendingRoute === "current"}
              >
                {copy.addAccount.currentStart}
              </Button>
            </Space>
          </Card>
        </div>
      ) : null}

      {activeRoute === "upload" ? (
        <div className="addAccountModalFlow">
          <Card className="addAccountModalCard" variant="outlined" title={copy.addAccount.uploadTab}>
            <Space orientation="vertical" size={16} className="addAccountModalStack">
              <Space wrap>
                <Button
                  icon={<CloudUploadOutlined />}
                  onClick={() => fileInputRef.current?.click()}
                  disabled={actionLocked}
                >
                  {copy.addAccount.uploadChooseFiles}
                </Button>
                <Button
                  icon={<FolderOpenOutlined />}
                  onClick={() => folderInputRef.current?.click()}
                  disabled={actionLocked}
                >
                  {copy.addAccount.uploadChooseFolder}
                </Button>
              </Space>
              <Card className="addAccountModalSubCard" size="small" variant="borderless">
                <Space orientation="vertical" size={10} className="addAccountModalStack">
                  <div>
                    <Typography.Text strong>
                      {selectedFiles.length > 0
                        ? copy.addAccount.uploadSelectedCount(selectedFiles.length)
                        : copy.addAccount.uploadQueueTitle}
                    </Typography.Text>
                    <Typography.Paragraph type="secondary">
                      {selectedFiles.length > 0 ? selectedSummary : copy.addAccount.uploadQueueEmpty}
                    </Typography.Paragraph>
                  </div>
                  {selectedPreview.length > 0 ? (
                    <div className="addAccountModalFileList">
                      {selectedPreview.map((file, index) => (
                        <Tag key={file.key}>{`${index + 1}. ${file.label}`}</Tag>
                      ))}
                    </div>
                  ) : (
                    <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={copy.addAccount.uploadQueueEmpty} />
                  )}
                </Space>
              </Card>
              <Button
                type="primary"
                icon={<SaveOutlined />}
                onClick={() => void handleImportFiles()}
                disabled={actionLocked || selectedFiles.length === 0}
                loading={pendingRoute === "upload" || importingAccounts || readingFiles}
              >
                {copy.addAccount.uploadStartImport}
              </Button>
            </Space>
          </Card>
        </div>
      ) : null}

      {activeRoute === "api" ? (
        isClassicSkin ? (
          <div className="addAccountModalFlow addAccountClassicApiFlow">
            {renderSectionFrame(
              "addAccountClassicProviderSection",
              copy.addAccount.apiProviderPresetTitle,
              <>
                {renderApiProviderPresets()}
                <Typography.Text type="secondary">
                  {apiProviderPresetId === "custom"
                    ? "自定义配置需要手动填写所有必要字段。"
                    : "选择预设后会填入供应商名称和 Base URL；模型需要填写 API Key 后再探测。"}
                </Typography.Text>
              </>,
              "点击预设只会填入供应商名称和 Base URL；模型需要填写 API Key 后再探测。",
            )}

            {renderApiConnectionFields()}

            {renderSectionFrame(
              "addAccountClassicModelSection",
              "模型菜单",
              <>
                <Typography.Text type="secondary">
                  模型菜单轻量模式会把启用模型直接写入 Codex 菜单；路由启动会把多个供应商聚合，并按“实际请求模型”转发。
                </Typography.Text>
                {renderApiModelControls()}
                {renderApiModelCatalog()}
              </>,
            )}

            {renderApiTagsField()}

            {apiQuotaBalanceEnabled || !apiQuotaBalancePresetLocked ? (
              renderSectionFrame(
                "addAccountClassicQuotaSection",
                "额度显示",
                renderApiQuotaControls(),
              )
            ) : null}

            {renderApiError()}
            {renderApiActions()}
          </div>
        ) : (
        <div className="addAccountModalFlow">
          <Card
            className="addAccountModalCard addAccountProviderPresetCard"
            variant="outlined"
            title={copy.addAccount.apiProviderPresetTitle}
          >
            <Space orientation="vertical" size={14} className="addAccountModalStack">
              {renderApiProviderPresets()}
            </Space>
          </Card>

          <Card className="addAccountModalCard addAccountConnectionCard" variant="outlined" title="连接配置">
            {renderApiConnectionFields()}
          </Card>

          <Card className="addAccountModalCard addAccountModelMenuCard" variant="outlined" title="模型菜单">
            <Space orientation="vertical" size={14} className="addAccountModalStack">
              {renderApiModelControls()}
              {renderApiModelCatalog()}
            </Space>
          </Card>

          <Card className="addAccountModalCard addAccountTagsCard" variant="outlined" title="标签">
            {renderApiTagsField()}
          </Card>

          <Card className="addAccountModalCard addAccountQuotaCard" variant="outlined" title="额度显示">
            {renderApiQuotaControls()}
          </Card>

          {renderApiError()}
          {renderApiActions()}
        </div>
        )
      ) : null}

      <input
        ref={fileInputRef}
        className="visuallyHidden"
        type="file"
        multiple
        accept=".json,application/json"
        onChange={handleFilesPicked}
      />
      <input
        ref={folderInputRef}
        className="visuallyHidden"
        type="file"
        multiple
        accept=".json,application/json"
        onChange={handleFilesPicked}
        {...folderPickerAttributes}
      />
    </Modal>
  );
}
