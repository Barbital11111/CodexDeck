# CodexDeck

CodexDeck 是一个基于 **React + Tauri** 的桌面工具，用来管理 Codex 账号、OpenAI 兼容 API、多供应商模型目录、混合登录配置、路由模式、账号切换、用量查看和通知数据源。

仓库地址：<https://github.com/Barbital11111/CodexDeck>

## 项目边界

- 管理 Codex OAuth 账号、API Key 条目和账号分组。
- 为每个账号/API 维护独立 `auth.json` 与 `config.toml` profile。
- 支持普通账号、API 登录、混合模式三种主要使用方式。
- 支持 API 余额显示、Sub2API/New API 额度查询和通知数据源配置。
- 支持增强启动相关开关，用于补齐 API 登录时的部分 Codex 页面能力。
- 支持 API 模型探测、菜单模型与实际请求模型分离、模型上下文窗口配置和本地模型路由模式。
- 支持 classic / modern 两套 PC 端 UI 皮肤并行开发。
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
- 支持预设供应商入口，包括 MiniMax、Xiaomi MiMo、Xiaomi MiMo Token Plan、DeepSeek、Z.AI GLM 和 Kimi。
- 支持探测上游 `/models`，并在账号内维护模型目录；菜单模型 ID、显示名称、实际请求模型和上下文窗口可以分别编辑。
- 上下文窗口在 UI 中使用 `256K`、`512K`、`1M` 这类格式展示，内部仍保存为 token 数。未知非 GPT 模型默认推荐 `256K`，GPT 系列不自动补写。
- API profile 默认写入 `codexdeck_api` provider。
- 混合模式会保留官方账号态，并通过 `experimental_bearer_token` 走指定 API 中转。
- 切换账号时会同步 Codex 线程 provider，降低历史会话不可见风险。
- 路由模式会在本机临时启动只监听 `127.0.0.1` 的模型路由，将多个 API 账号的已选模型聚合为一个 OpenAI 兼容入口；关闭或切换模式时会停止旧路由。

### 额度与通知

- 展示账号用量窗口和 API 余额信息。
- 支持 Sub2API/New API 额度来源配置。
- 支持平台账号无订阅、有订阅和管理账号模式。
- 支持 DeepSeek、MiniMax、GLM、Kimi 等可查询平台的 API Key 额度刷新；MiMo Token Plan 订阅标签目前采用手动选择，不做余额查询。
- 通知中心可配置数据源、投递通道、模板与规则。

### 启动与增强

- 一键切换账号并启动 Codex。
- 找不到桌面应用时自动回退到 `codex app`。
- 可选同步 Opencode OpenAI 授权。
- 可选在切换后重启已选编辑器。
- API 登录可启用增强启动，补齐部分官方账号态页面能力。

### UI 皮肤

- `classic` 皮肤保留原版布局和蓝白观感，经典组件位于 `src/components/classic/`，样式集中在 `src/styles/classic-restore.css`。
- `modern` 皮肤承接新版暖橙 UI，使用新版 Header、供应商与模型入口和路由启动卡片。
- 预览可通过 URL 参数临时覆盖：`?codexdeckPreviewWindow=1&uiSkin=classic` 或 `?codexdeckPreviewWindow=1&uiSkin=modern`。

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
