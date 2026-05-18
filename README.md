# CodexDeck

CodexDeck 是一个基于 **React + Tauri** 的桌面控制台，用来管理 Codex 账号、OpenAI 兼容 API profile、账号切换、用量查看、通知规则和外接远程控制运行时。

仓库地址：<https://github.com/Barbital11111/CodexDeck>

## 边界

- 管理 OAuth 账号与外部 API 配置。
- 为每个账号/API 维护独立 `auth.json` 与 `config.toml` profile。
- 一键切换当前 Codex 配置并启动 Codex。
- API profile 固定写入 CodexDeck 管理的 provider，并同步修复线程可见性。
- 远程控制只接入外部安装版 runtime，不复制远控源码。
- 不内置本地网关、Sub2API Docker、cloudflared 或远程反代部署。

## 本地开发

### 环境

- Node.js 20+
- Rust stable
- Windows 或 macOS
- 远程控制开发/打包需要一份外部安装版 runtime

### 安装依赖

```bash
npm install
```

### 启动桌面开发版

```bash
npm run dev:desktop
```

开发预览会使用仓库内 `.dev-runtime/` 隔离目录，避免覆盖正式安装版账号、profile 与 `~/.codex` 配置。

只查看浏览器页面可以运行：

```bash
npm run dev
```

## 远程控制 Runtime

CodexDeck 的远程控制页面只做外壳：启动/停止控制台、状态卡、日志、打开控制台网页、安装手机 APK、显示连接地址和二维码。

打包时需要把外部安装版 runtime staged 到：

```text
src-tauri/resources/codex-command-runtime/
```

该目录被 `.gitignore` 忽略，不进入源码仓库。发布脚本会从外部安装版目录复制 runtime，并排除运行态数据。

打包时必须显式传入 runtime 来源，或设置：

```powershell
$env:CODEXDECK_REMOTE_RUNTIME_SOURCE = "<runtime-install-root>"
```

开发时可通过环境变量指定 runtime：

```powershell
$env:CODEXDECK_REMOTE_RUNTIME_DIR = "<runtime-install-root>"
```

## 验证

常用命令：

```bash
npm run lint -- --max-warnings=0
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

发布前必须执行脱敏检查，确认没有本机路径、用户名、token、API key、auth 文件、运行态数据库、日志或私有配置泄露。

## License

MIT，详见 [LICENSE](LICENSE)。
