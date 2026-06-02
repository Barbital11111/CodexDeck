# CodexDeck

CodexDeck 是一个基于 **React + Tauri** 的桌面工具，用来管理 Codex 账号、OpenAI 兼容 API、混合登录配置、账号切换、用量查看和通知数据源。

仓库地址：<https://github.com/Barbital11111/CodexDeck>

## 项目边界

- 管理 Codex OAuth 账号、API Key 条目和账号分组。
- 为每个账号/API 维护独立 `auth.json` 与 `config.toml` profile。
- 支持普通账号、API 登录、混合模式三种主要使用方式。
- 支持 API 余额显示、Sub2API/New API 额度查询和通知数据源配置。
- 支持增强启动相关开关，用于补齐 API 登录时的部分 Codex 页面能力。
- 不内置 Sub2API、远程控制 runtime、Android 端、cloudflared 或外部反代源码。

如需 Sub2API、New API、NAS 网关或其他中转服务，请作为外部服务部署，然后在 CodexDeck 中按普通 API 配置填写：

```text
Base URL: http://<your-gateway>/v1
API Key:  <gateway-api-key>
Model:    <model-name>
```

## 本地开发

### 环境准备

- Node.js 20+
- Rust stable
- Windows 或 macOS

### 安装依赖

```bash
npm install
```

### 启动桌面开发预览

```bash
npm run dev:desktop
```

该命令会把开发预览使用的数据隔离到仓库内 `.dev-runtime/`，避免本地调试时覆盖正式安装版保存的账号、profile 与 `~/.codex` 配置。

如果只需要浏览器页面预览：

```bash
npm run dev
```

## 主要功能

### 账号管理

- 支持 OAuth 登录导入。
- 支持上传单个或多个 `.json` 文件批量导入。
- 支持导入/导出账号备份。
- 支持账号分组、标签、智能切换和隐藏敏感信息。

### API 与混合模式

- 支持 OpenAI 兼容 `Base URL + API Key + Model` 配置。
- 保存前会检测 `/responses` 兼容性。
- API profile 默认写入 `codexdeck_api` provider。
- 混合模式会保留官方账号态，并通过 `experimental_bearer_token` 走指定 API 中转。
- 切换账号时会同步 Codex 线程 provider，降低历史会话不可见风险。

### 额度与通知

- 展示账号用量窗口和 API 余额信息。
- 支持 Sub2API/New API 额度来源配置。
- 支持平台账号无订阅、有订阅和管理账号模式。
- 通知中心可配置数据源、投递通道、模板与规则。

### 启动与增强

- 一键切换账号并启动 Codex。
- 找不到桌面应用时自动回退到 `codex app`。
- 可选同步 Opencode OpenAI 授权。
- 可选在切换后重启已选编辑器。
- API 登录可启用增强启动，补齐部分官方账号态页面能力。

## 安装兼容策略

当前 Windows 构建仍沿用历史安装身份：

```text
应用身份: com.carry.codex-tools
数据目录: %APPDATA%\com.carry.codex-tools
```

这是为了让新版 CodexDeck 能覆盖早期 Codex Tools / Codex Switch 安装，并继续读取已有账号、profile 和设置数据。后续如果要迁移到新的应用身份，需要单独做迁移版本，避免用户数据丢失。

## 验证

常用检查命令：

```bash
npm run lint -- --max-warnings=0
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

发布前必须执行脱敏检查，确认没有本地路径、邮箱、token、API key、auth 文件、`.env`、运行时缓存、构建临时目录或私有服务配置泄露。

## License

MIT，详见 [LICENSE](LICENSE)。
