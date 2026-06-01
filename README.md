# Codex Switch

Codex Switch 是一个基于 **React + Tauri** 的桌面工具，用来管理 Codex 账号、OpenAI 兼容 API 配置、账号 profile 切换、用量查看和后续额度通知。

项目边界：

- 管理 OAuth 账号与外部 API 配置
- 为每个账号/API 维护独立 `auth.json` 与 `config.toml` profile
- 一键切换当前 Codex 配置并启动 Codex
- 查看账号用量，支持智能切换与分组
- 预留通知中心，用于后续接入额度预警、恢复提醒和状态汇总
- 不再内置本地网关、Sub2API Docker、cloudflared 或远程反代部署

仓库地址：<https://github.com/Barbital11111/codex-switch>

## 当前状态

Codex Switch 正在从早期 Codex Tools 分支重构独立化。旧本地网关运行面已经移除；如果你需要 Sub2API、mihomo 或其他网关，请把它们作为外部服务部署，然后在 Codex Switch 中按普通 API 配置填写：

```text
Base URL: http://<你的网关地址>/v1
API Key:  <外部网关生成的 Key>
```

## 快速启动（本地开发）

### 环境准备

- Node.js 20+
- Rust stable
- Windows 或 macOS

### 安装依赖

```bash
npm install
```

### 启动桌面应用（推荐隔离预览）

```bash
npm run dev:desktop
```

该命令会把开发预览使用的数据隔离到仓库内 `.dev-runtime/`，避免本地调试时覆盖正式安装版保存的账号、profile 与 `~/.codex` 配置。

如果你只想看浏览器页面，也可以使用：

```bash
npm run dev
```

## 主要功能

### 账号管理

- 支持 OAuth 登录导入
- 支持上传单个或多个 `.json` 文件批量导入
- 支持导入导出的 `accounts.json` 备份
- 导入结束后会恢复当前本机登录态，不覆盖你正在使用的账号

### API 配置

- 支持 OpenAI 兼容 `Base URL + API Key + Model` 配置
- 保存前会检测 `/responses` 等接口能力
- 切换时自动写入独立 profile
- 可用于连接外部 NAS/Sub2API/自建网关

### 用量查看与智能切换

- 展示账号 **5h**、**1week** 用量窗口和计划类型
- 支持手动刷新和后台自动刷新
- 支持按余量排序和智能切换到更合适的账号
- 为后续通知中心提供数据基础

### 切换账号并联动本机环境

- 一键切换账号并启动 Codex
- 找不到桌面应用时自动回退到 `codex app`
- 可选同步 Opencode OpenAI 授权
- 可选在切换后重启已选编辑器

## 过渡安装策略

当前构建采用“覆盖旧安装优先”的过渡策略：

```text
安装器产品名: Codex Tools
应用窗口/界面: Codex Switch
应用身份:     com.carry.codex-tools
数据目录:     %APPDATA%\com.carry.codex-tools
```

这样做是为了让 Windows/NSIS 能识别本机已安装的 Codex Tools 1.9.3，并用 1.9.4 过渡包直接覆盖安装，而不是并排安装一个新程序。后续如果要切换到真正独立的 `io.github.barbital11111.codex-switch` 身份，需要单独做迁移包，避免丢失用户账号和 profile。

过渡包不会迁移旧网关/Docker runtime 数据，账号相关数据继续使用原目录：

- `accounts.json`
- `accounts.json.last-good.json`
- `accounts.json.prev-good.json`
- `profiles/`

开发预览仍使用仓库内 `.dev-runtime/` 隔离目录，并会优先从正式安装版的旧数据目录复制账号与 profile 副本。

## 打包与发布

常用验证命令：

```bash
npx tsc --noEmit
npm run lint -- --max-warnings=0
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

发布前必须执行脱敏检查，确认没有本地路径、邮箱、token、API key、auth 文件、`.env`、Docker runtime 数据或私有订阅泄露。

## License

MIT，详见 [LICENSE](LICENSE)。
