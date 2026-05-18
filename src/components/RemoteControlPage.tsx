import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AndroidOutlined,
  ApiOutlined,
  CloudServerOutlined,
  CodeOutlined,
  CopyOutlined,
  FolderOpenOutlined,
  LinkOutlined,
  PauseCircleOutlined,
  PlayCircleOutlined,
  RedoOutlined,
  ReloadOutlined,
} from "@ant-design/icons";
import { Alert, Button, QRCode, Space, Tag, Tooltip, message } from "antd";
import { invoke } from "@tauri-apps/api/core";

type RemoteRuntimeSource = "env" | "userDataCurrent" | "resource" | "missing";

type JsonRecord = Record<string, unknown>;

type RemoteRuntimeManifest = {
  builtAt?: string | null;
  repoRoot?: string | null;
  installRoot?: string | null;
  runtimeName?: string | null;
  runtimeVersion?: string | null;
  protocolVersion?: string | null;
  bridgeVersion?: string | null;
  panel?: JsonRecord | null;
  scripts?: JsonRecord | null;
  ports?: JsonRecord | null;
  capabilities?: JsonRecord | null;
  mobileApk?: string | null;
};

type RemoteRuntimeDetection = {
  available: boolean;
  runtimeRoot: string | null;
  source: RemoteRuntimeSource;
  missing: string[];
  statusUrl: string;
  runtimeUrl: string;
  capabilitiesUrl: string;
  logsUrl: string;
  panelUrl: string;
  manifestPath: string | null;
  mobileApkPath: string | null;
  manifest: RemoteRuntimeManifest | null;
  checkedRoots: string[];
};

type RemoteRuntimeState = {
  updatedAt?: string | null;
  panelStatus?: string | null;
  bridgeStatus?: string | null;
  relayStatus?: string | null;
  phoneStatus?: string | null;
  desktopStatus?: string | null;
  relayUrl?: string | null;
  bindingCode?: string | null;
  manualCode?: string | null;
  sessionId?: string | null;
  deviceId?: string | null;
  expiresAt?: string | null;
  panelUrl?: string | null;
  desktopPort?: string | null;
  desktopTargetId?: string | null;
  lastError?: string | null;
};

type RemoteStatusSnapshot = {
  reachable: boolean;
  state: RemoteRuntimeState | null;
  connectionAddress: string | null;
  connectionCode: string | null;
  error: string | null;
};

type RemoteJsonSnapshot = {
  reachable: boolean;
  data: JsonRecord | null;
  error: string | null;
};

type RemoteLogEntry = {
  name?: string | null;
  path?: string | null;
  kind?: string | null;
  size?: number | null;
  modifiedAt?: string | null;
  tail?: string | null;
};

type RemoteLogsSnapshot = {
  reachable: boolean;
  logsDir: string | null;
  entries: RemoteLogEntry[];
  latest: JsonRecord | null;
  error: string | null;
};

type RemoteAction =
  | "detect"
  | "refresh"
  | "start"
  | "stop"
  | "restart"
  | "openPanel"
  | "installApk"
  | "openLogs"
  | "listen";

const sourceLabels: Record<RemoteRuntimeSource, string> = {
  env: "开发配置",
  userDataCurrent: "用户数据目录",
  resource: "内置运行时",
  missing: "未检测到",
};

const statusLabels: Record<string, string> = {
  running: "运行中",
  connected: "已连接",
  ready: "就绪",
  stopped: "未启动",
  disconnected: "未连接",
  pending: "等待中",
  error: "异常",
};

const commandLabels: Record<Exclude<RemoteAction, "detect" | "refresh">, string> = {
  start: "启动远程控制台",
  stop: "停止远程控制台",
  restart: "重启远程控制台",
  openPanel: "打开网页控制台",
  installApk: "安装手机 App",
  openLogs: "打开日志目录",
  listen: "开始监听",
};

function delay(ms: number) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function isTauriDesktopRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function statusText(value: string | null | undefined) {
  const normalized = value?.trim();
  if (!normalized) {
    return "未知";
  }
  return statusLabels[normalized] ?? normalized;
}

function statusTone(value: string | null | undefined): "success" | "warning" | "error" | "default" {
  const normalized = value?.trim().toLowerCase();
  if (normalized === "running" || normalized === "connected" || normalized === "ready") {
    return "success";
  }
  if (normalized === "pending" || normalized === "starting") {
    return "warning";
  }
  if (normalized === "error" || normalized === "failed") {
    return "error";
  }
  return "default";
}

function formatTime(value: string | null | undefined) {
  if (!value) {
    return "暂无";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function buildQrPayload(address: string | null, code: string | null) {
  if (!address || !code) {
    return "CodexDeck Remote Control";
  }
  return JSON.stringify({ address, code });
}

function asRecord(value: unknown): JsonRecord | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  return value as JsonRecord;
}

function stringField(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function numberField(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? String(value) : null;
}

function boolField(value: unknown) {
  return typeof value === "boolean" ? value : null;
}

function runtimePayload(snapshot: RemoteJsonSnapshot | null) {
  return asRecord(snapshot?.data?.runtime) ?? asRecord(snapshot?.data);
}

function runtimeField(
  snapshot: RemoteJsonSnapshot | null,
  manifest: RemoteRuntimeManifest | null | undefined,
  key: keyof RemoteRuntimeManifest,
) {
  const payload = runtimePayload(snapshot);
  return stringField(payload?.[key]) ?? stringField(manifest?.[key]);
}

function runtimeNestedField(
  snapshot: RemoteJsonSnapshot | null,
  manifest: RemoteRuntimeManifest | null | undefined,
  group: keyof RemoteRuntimeManifest,
  key: string,
) {
  const payload = runtimePayload(snapshot);
  const liveGroup = asRecord(payload?.[group]);
  const manifestGroup = asRecord(manifest?.[group]);
  return (
    stringField(liveGroup?.[key]) ??
    numberField(liveGroup?.[key]) ??
    stringField(manifestGroup?.[key]) ??
    numberField(manifestGroup?.[key])
  );
}

function capabilitiesPayload(
  capabilities: RemoteJsonSnapshot | null,
  manifest: RemoteRuntimeManifest | null | undefined,
) {
  return (
    asRecord(capabilities?.data?.capabilities) ??
    asRecord(manifest?.capabilities) ??
    {}
  );
}

function capabilityEnabled(
  capabilities: RemoteJsonSnapshot | null,
  manifest: RemoteRuntimeManifest | null | undefined,
  key: string,
) {
  return boolField(capabilitiesPayload(capabilities, manifest)[key]) === true;
}

function logEntries(logs: RemoteLogsSnapshot | null) {
  if (logs?.entries.length) {
    return logs.entries.slice(0, 4);
  }

  const latest = asRecord(logs?.latest);
  if (!latest) {
    return [];
  }

  return Object.entries(latest)
    .map(([name, entry]) => ({ name, ...(asRecord(entry) ?? {}) }) as RemoteLogEntry)
    .filter((entry) => Boolean(entry.tail || entry.name));
}

async function copyText(value: string, label: string) {
  await navigator.clipboard.writeText(value);
  void message.success(`${label}已复制`);
}

function RuntimeDetectionNotice({ detection }: { detection: RemoteRuntimeDetection | null }) {
  if (!detection) {
    return null;
  }

  if (detection?.available) {
    return null;
  }

  const missing = detection?.missing.length ? detection.missing.join("、") : "runtimeRoot";
  const checkedRoots = detection?.checkedRoots.filter(Boolean) ?? [];

  return (
    <Alert
      className="remoteRuntimeAlert"
      type="warning"
      showIcon
      message="未检测到 Codex Command runtime"
      description={
        <div className="remoteRuntimeAlertBody">
          <p>
            开发环境请设置 <code>CODEXDECK_REMOTE_RUNTIME_DIR</code> 指向安装版目录；正式版会优先读取用户数据目录，
            再读取安装包内置 runtime。
          </p>
          <p>缺少：{missing}</p>
          {checkedRoots.length > 0 ? (
            <div className="remoteCheckedRoots">
              {checkedRoots.map((root) => (
                <code key={root}>{root}</code>
              ))}
            </div>
          ) : null}
        </div>
      }
    />
  );
}

function StatusMetric({
  label,
  value,
  description,
}: {
  label: string;
  value: string | null | undefined;
  description: string;
}) {
  return (
    <article className={`remoteStatusMetric is-${statusTone(value)}`}>
      <span>{label}</span>
      <strong>{statusText(value)}</strong>
      <p>{description}</p>
    </article>
  );
}

export function RemoteControlPage() {
  const [detection, setDetection] = useState<RemoteRuntimeDetection | null>(null);
  const [status, setStatus] = useState<RemoteStatusSnapshot | null>(null);
  const [runtimeInfo, setRuntimeInfo] = useState<RemoteJsonSnapshot | null>(null);
  const [capabilities, setCapabilities] = useState<RemoteJsonSnapshot | null>(null);
  const [logs, setLogs] = useState<RemoteLogsSnapshot | null>(null);
  const [action, setAction] = useState<RemoteAction | null>(null);
  const [listening, setListening] = useState(false);
  const desktopRuntime = isTauriDesktopRuntime();

  const state = status?.state ?? null;
  const manifest = detection?.manifest ?? null;
  const connectionAddress = status?.connectionAddress ?? null;
  const connectionCode = status?.connectionCode ?? null;
  const hasLastError = Boolean(state?.lastError?.trim());
  const runtimeReady = detection?.available ?? false;
  const statusReachable = status?.reachable ?? false;
  const runtimeName = runtimeField(runtimeInfo, manifest, "runtimeName") ?? "codex-command-runtime";
  const runtimeVersion = runtimeField(runtimeInfo, manifest, "runtimeVersion") ?? "未知版本";
  const protocolVersion = runtimeField(runtimeInfo, manifest, "protocolVersion") ?? "未知协议";
  const panelUrl = state?.panelUrl ?? runtimeNestedField(runtimeInfo, manifest, "panel", "url") ?? detection?.panelUrl ?? "http://127.0.0.1:47992/";
  const relayPort = runtimeNestedField(runtimeInfo, manifest, "ports", "relay") ?? "9000";
  const panelPort = runtimeNestedField(runtimeInfo, manifest, "ports", "panel") ?? "47992";
  const cdpPort = runtimeNestedField(runtimeInfo, manifest, "ports", "cdp") ?? state?.desktopPort ?? "9333";
  const recentLogs = useMemo(() => logEntries(logs), [logs]);
  const consoleRunning = statusReachable && !hasLastError;
  const qrPayload = useMemo(
    () => buildQrPayload(connectionAddress, connectionCode),
    [connectionAddress, connectionCode],
  );

  const refreshAll = useCallback(async (quiet = false, preserveReachableStatus = true) => {
    if (!quiet) {
      setAction("detect");
    }

    try {
      if (!isTauriDesktopRuntime()) {
        const previewStatus = {
          reachable: false,
          state: null,
          connectionAddress: null,
          connectionCode: null,
          error: "当前是浏览器预览，未连接桌面运行时。",
        };
        setDetection({
          available: false,
          runtimeRoot: null,
          source: "missing",
          missing: ["Tauri desktop runtime"],
          statusUrl: "http://127.0.0.1:47992/api/state",
          runtimeUrl: "http://127.0.0.1:47992/api/runtime",
          capabilitiesUrl: "http://127.0.0.1:47992/api/capabilities",
          logsUrl: "http://127.0.0.1:47992/api/logs",
          panelUrl: "http://127.0.0.1:47992/",
          manifestPath: null,
          mobileApkPath: null,
          manifest: null,
          checkedRoots: ["当前是浏览器预览，远程控制命令只在 CodexDeck 桌面应用中可用。"],
        });
        setStatus(previewStatus);
        setRuntimeInfo({ reachable: false, data: null, error: "当前是浏览器预览。" });
        setCapabilities({ reachable: false, data: null, error: "当前是浏览器预览。" });
        setLogs({ reachable: false, logsDir: null, entries: [], latest: null, error: "当前是浏览器预览。" });
        return previewStatus;
      }

      const [nextDetection, nextStatus, nextRuntime, nextCapabilities, nextLogs] = await Promise.all([
        invoke<RemoteRuntimeDetection>("remote_detect_runtime"),
        invoke<RemoteStatusSnapshot>("remote_get_status"),
        invoke<RemoteJsonSnapshot>("remote_get_runtime"),
        invoke<RemoteJsonSnapshot>("remote_get_capabilities"),
        invoke<RemoteLogsSnapshot>("remote_get_logs"),
      ]);
      setDetection(nextDetection);
      setStatus((current) => {
        if (quiet && preserveReachableStatus && !nextStatus.reachable && current?.reachable) {
          return {
            ...current,
            error: nextStatus.error ?? current.error,
          };
        }
        return nextStatus;
      });
      setRuntimeInfo((current) => (quiet && !nextRuntime.reachable && current?.reachable ? current : nextRuntime));
      setCapabilities((current) =>
        quiet && !nextCapabilities.reachable && current?.reachable ? current : nextCapabilities,
      );
      setLogs((current) => (quiet && !nextLogs.reachable && current?.reachable ? current : nextLogs));
      return nextStatus;
    } catch (error) {
      void message.error(`读取远程控制状态失败：${String(error)}`);
      return null;
    } finally {
      if (!quiet) {
        setAction(null);
      }
    }
  }, []);

  const refreshUntilReachable = useCallback(async () => {
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const nextStatus = await refreshAll(true);
      if (nextStatus?.reachable) {
        return;
      }
      await delay(800);
    }
  }, [refreshAll]);

  useEffect(() => {
    if (!listening) {
      return undefined;
    }

    void refreshAll();
    const timer = window.setInterval(() => {
      void refreshAll(true);
    }, 4000);
    return () => window.clearInterval(timer);
  }, [listening, refreshAll]);

  const startListening = async () => {
    setAction("listen");
    try {
      await refreshAll();
      setListening(true);
    } finally {
      setAction(null);
    }
  };

  const runAction = async (nextAction: Exclude<RemoteAction, "detect" | "refresh" | "listen">) => {
    if (!isTauriDesktopRuntime()) {
      void message.info("浏览器预览无法调用桌面运行时，请在 CodexDeck 桌面应用中操作。");
      return;
    }

    setAction(nextAction);
    const label = commandLabels[nextAction];
    try {
      if (nextAction === "start") {
        await invoke("remote_start_console");
      } else if (nextAction === "stop") {
        await invoke("remote_stop_console");
      } else if (nextAction === "restart") {
        await invoke("remote_restart_console");
      } else if (nextAction === "openPanel") {
        await invoke("remote_open_panel");
      } else if (nextAction === "installApk") {
        await invoke("remote_install_mobile_apk");
      } else {
        await invoke("remote_open_logs");
      }
      if (!["start", "stop", "restart"].includes(nextAction)) {
        void message.success(`${label}完成`);
      }
      if (nextAction === "start" || nextAction === "restart") {
        setListening(true);
        await refreshUntilReachable();
      } else if (nextAction === "stop") {
        setListening(false);
        await refreshAll(false, false);
      } else if (listening) {
        await refreshAll(true);
      }
    } catch (error) {
      void message.error(`${label}失败：${String(error)}`);
    } finally {
      setAction(null);
    }
  };

  const lifecycleLabel =
    action === "start"
      ? "正在启动"
      : action === "restart"
        ? "正在重启"
        : action === "stop"
          ? "正在停止"
          : null;
  const overallLabel = lifecycleLabel
    ? lifecycleLabel
    : hasLastError
    ? "运行异常"
    : statusReachable
      ? "控制台可用"
      : !listening && !detection
        ? "未开始监听"
        : runtimeReady
        ? "等待启动"
        : "未配置运行时";
  const overallTone = hasLastError ? "error" : statusReachable ? "success" : "warning";

  return (
    <section className="remoteControlPage" aria-label="远程控制">
      <div className="remoteControlShell">
        <div className="remoteControlHeader">
          <div>
            <p className="remoteControlKicker">REMOTE CONTROL</p>
            <h2>远程控制</h2>
            <p>CodexDeck 管理安装版受控运行时；普通 Codex 不会被手机端接管。</p>
          </div>
          <Space wrap>
            <Button
              icon={<ReloadOutlined />}
              loading={action === "detect" || action === "refresh" || action === "listen"}
              onClick={() => void refreshAll()}
            >
              刷新状态
            </Button>
            <Button
              icon={listening ? <PauseCircleOutlined /> : <PlayCircleOutlined />}
              loading={action === "listen"}
              onClick={() => {
                if (listening) {
                  setListening(false);
                  return;
                }
                void startListening();
              }}
            >
              {listening ? "停止监听" : "开始监听"}
            </Button>
            <Button
              type="primary"
              icon={<PlayCircleOutlined />}
              disabled={!runtimeReady || consoleRunning || action !== null}
              loading={action === "start"}
              onClick={() => void runAction("start")}
            >
              {consoleRunning ? "已启动" : "启动"}
            </Button>
          </Space>
        </div>

        <RuntimeDetectionNotice detection={detection} />

        {!listening ? (
          <Alert
            className="remoteRuntimeAlert"
            type="info"
            showIcon
            message="远程状态尚未开始监听"
            description="为避免页面打开后持续探测运行时，CodexDeck 只会在你点击“开始监听”或“刷新状态”后读取远程控制状态。"
          />
        ) : null}

        {!desktopRuntime ? (
          <Alert
            className="remoteRuntimeAlert"
            type="info"
            showIcon
            message="当前是浏览器预览"
            description="远程控制的启动、停止、状态读取和日志入口依赖 Tauri 桌面命令；浏览器里只用于查看界面布局。"
          />
        ) : null}

        <section className="remoteOverviewPanel">
          <div className="remoteOverviewMain">
            <div className="remoteOverviewTitle">
              <span className={`remotePulse is-${overallTone}`} />
              <div>
                <span>当前状态</span>
                <strong>{overallLabel}</strong>
              </div>
            </div>
            <Tag className={`remoteSourceTag is-${detection?.source ?? "missing"}`}>
              {sourceLabels[detection?.source ?? "missing"]}
            </Tag>
          </div>
          <div className="remoteRuntimeMetaGrid">
            <div className="remoteRuntimeMeta">
              <span>运行时目录</span>
              <code>{detection?.runtimeRoot ?? "未解析"}</code>
            </div>
            <div className="remoteRuntimeMeta">
              <span>手机 APK</span>
              <code>{detection?.mobileApkPath ?? "未解析"}</code>
            </div>
            <div className="remoteRuntimeMeta">
              <span>版本</span>
              <code>{runtimeName} / {runtimeVersion} / protocol {protocolVersion}</code>
            </div>
          </div>
          <div className="remoteOverviewActions">
            <Tooltip title="停止只调用 runtime 脚本，不会全局关闭 Codex.exe">
              <Button
                icon={<PauseCircleOutlined />}
                disabled={!runtimeReady || action !== null}
                loading={action === "stop"}
                onClick={() => void runAction("stop")}
              >
                停止
              </Button>
            </Tooltip>
            <Button
              icon={<RedoOutlined />}
              disabled={!runtimeReady || action !== null}
              loading={action === "restart"}
              onClick={() => void runAction("restart")}
            >
              重启
            </Button>
            <Button
              icon={<LinkOutlined />}
              loading={action === "openPanel"}
              onClick={() => void runAction("openPanel")}
            >
              打开控制台
            </Button>
            <Button
              icon={<FolderOpenOutlined />}
              disabled={!runtimeReady}
              loading={action === "openLogs"}
              onClick={() => void runAction("openLogs")}
            >
              日志
            </Button>
            <Button
              icon={<AndroidOutlined />}
              disabled={!runtimeReady}
              loading={action === "installApk"}
              onClick={() => void runAction("installApk")}
            >
              安装手机 App
            </Button>
          </div>
        </section>

        {hasLastError ? (
          <Alert
            className="remoteLastError"
            type="error"
            showIcon
            message="运行时报告异常"
            description={state?.lastError}
          />
        ) : null}

        <div className="remoteStatusGrid">
          <StatusMetric label="Bridge" value={state?.bridgeStatus} description="本机桥接进程" />
          <StatusMetric label="Relay" value={state?.relayStatus} description="手机与桌面中继" />
          <StatusMetric label="Phone" value={state?.phoneStatus} description="移动端连接" />
          <StatusMetric label="Desktop" value={state?.desktopStatus} description="受控 Codex 实例" />
        </div>

        <section className="remoteConnectionPanel">
          <div className="remoteConnectionCopy">
            <p className="remoteControlKicker">CONNECT</p>
            <h3>连接信息</h3>
            <p>手机端可使用连接地址和连接码接入；二维码内容跟随这两项生成。</p>

            <div className="remoteConnectionRows">
              <div className="remoteConnectionRow">
                <span>连接地址</span>
                <code>{connectionAddress ?? "等待运行时返回"}</code>
                <Button
                  icon={<CopyOutlined />}
                  disabled={!connectionAddress}
                  onClick={() =>
                    connectionAddress ? void copyText(connectionAddress, "连接地址") : undefined
                  }
                >
                  复制
                </Button>
              </div>
              <div className="remoteConnectionRow">
                <span>连接码</span>
                <code>{connectionCode ?? "等待运行时返回"}</code>
                <Button
                  icon={<CopyOutlined />}
                  disabled={!connectionCode}
                  onClick={() =>
                    connectionCode ? void copyText(connectionCode, "连接码") : undefined
                  }
                >
                  复制
                </Button>
              </div>
            </div>
          </div>
          <div className="remoteQrBox">
            <QRCode value={qrPayload} size={144} bordered={false} status={connectionAddress && connectionCode ? "active" : "loading"} />
          </div>
        </section>

        <section className="remoteDetailsGrid">
          <article>
            <ApiOutlined />
            <span>状态接口</span>
            <code>{detection?.statusUrl ?? "http://127.0.0.1:47992/api/state"}</code>
          </article>
          <article>
            <CloudServerOutlined />
            <span>网页控制台</span>
            <code>{panelUrl}</code>
          </article>
          <article>
            <CodeOutlined />
            <span>最近更新时间</span>
            <code>{formatTime(state?.updatedAt)}</code>
          </article>
        </section>

        <section className="remoteRuntimeGrid">
          <article className="remoteRuntimeCard">
            <p className="remoteControlKicker">RUNTIME</p>
            <h3>运行时契约</h3>
            <div className="remoteRuntimeRows">
              <div><span>runtime</span><code>{runtimeName}</code></div>
              <div><span>relay</span><code>{relayPort}</code></div>
              <div><span>panel</span><code>{panelPort}</code></div>
              <div><span>受控 Codex</span><code>CDP {cdpPort}</code></div>
            </div>
          </article>
          <article className="remoteRuntimeCard">
            <p className="remoteControlKicker">CAPABILITIES</p>
            <h3>能力矩阵</h3>
            <div className="remoteCapabilityTags">
              {[
                ["manualPairing", "手动连接码"],
                ["mobileApkInstall", "安装 APK"],
                ["controlledDesktop", "受控 Codex"],
                ["controlledDesktopOnly", "独立受控实例"],
                ["windowHosting", "窗口托管"],
                ["runtimeSelfUpdate", "运行时自更新"],
              ].map(([key, label]) => (
                <Tag key={key} color={capabilityEnabled(capabilities, manifest, key) ? "success" : "default"}>
                  {label}
                </Tag>
              ))}
            </div>
          </article>
        </section>

        <section className="remoteLogsPanel">
          <div className="remoteLogsHeader">
            <div>
              <p className="remoteControlKicker">LOGS</p>
              <h3>最近日志</h3>
            </div>
            <code>{logs?.logsDir ?? "等待运行时返回日志目录"}</code>
          </div>
          {logs?.error && listening ? (
            <Alert type="warning" showIcon message="暂时无法读取运行时日志" description={logs.error} />
          ) : null}
          <div className="remoteLogList">
            {recentLogs.length > 0 ? (
              recentLogs.map((entry, index) => (
                <article className="remoteLogEntry" key={`${entry.name ?? "log"}-${index}`}>
                  <div>
                    <strong>{entry.name ?? "runtime.log"}</strong>
                    <span>{formatTime(entry.modifiedAt)}</span>
                  </div>
                  <pre>{entry.tail?.trim() || "暂无日志内容"}</pre>
                </article>
              ))
            ) : (
              <div className="remoteLogsEmpty">暂未读取到运行时日志</div>
            )}
          </div>
        </section>
      </div>
    </section>
  );
}
