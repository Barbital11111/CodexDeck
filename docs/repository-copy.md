# Repository Copy Draft

这些文案用于公开仓库、Release 页面和截图说明。当前只做准备，发布前可以继续微调。

## GitHub Short Description

Codex account, API relay, quota, and notification workflow desktop workspace.

## 中文简介

CodexDeck 是一个面向 Codex 使用者的桌面工作台，用来集中管理 Codex 账号、API 中转配置、额度状态和通知链路。它适合同时维护多个 OAuth 账号、多个 API 平台，或者需要把额度日报、异常提醒推送到 Telegram/Webhook 的使用场景。

当前版本重点覆盖账号/API 管理、额度可视化、通知规则基础能力，并接入外部远程控制安装版 runtime。CodexDeck 只作为外壳控制台，不复制远程管理工具源码。

## 英文简介备选

CodexDeck is a desktop workspace for Codex account management, API relay profiles, quota visibility, and notification routes. It helps users manage multiple OAuth accounts and API providers from one local control surface, while keeping notification rules and delivery channels reusable.

The current release focuses on account/API management, quota visibility, notification workflow foundations, and an external remote-control runtime adapter. CodexDeck acts as the shell and does not vendor the remote-control source code.

## 截图说明文案

- `accounts-overview.png`：账号和 API 的统一工作台。
- `first-use-add-account.png`：首次使用时从“添加账号”开始。
- `account-import-oauth.png`：通过 OAuth、本机登录态、文件或 API profile 导入账号。
- `api-import-basic.png`：添加最小 OpenAI 兼容 API 中转配置。
- `api-import-quota.png`：绑定平台账号后开启 API 余额显示。
- `notification-home.png`：查看启用规则、可用数据源、发送渠道和最近推送记录。
- `notification-data-sources.png`：管理额度和用量数据源。
- `notification-delivery-channels.png`：管理 Telegram/Webhook 发送渠道。
- `notification-rule-drawer.png`：创建通知规则，配置数据源、发送渠道、模板、计划和测试推送。

## Release Note 草稿

CodexDeck 正在准备作为新的独立桌面工作台发布，用于 Codex 账号和 API 管理。

本次快照包含：

- 新品牌和图标整理为 CodexDeck。
- 账号与 API profile 统一工作台。
- API 平台账号绑定后的可选余额显示。
- 通知中心基础能力：数据源、发送渠道、模板、规则、计划和测试推送。
- 外部远程控制安装版 runtime 接入：启动/停止控制台、查看状态和日志、显示连接信息、安装手机 APK。
- API profile provider 写入修复：固定 `codexdeck_api`，禁用 responses websocket，并同步历史线程 provider 可见性。
- 脱敏后的公开仓库截图和文档。

当前范围：

- 远程控制 runtime 作为安装版资源打包进 release，不进入源码仓库。

## 建议 Topics

`codex`, `desktop-app`, `tauri`, `account-manager`, `api-relay`, `notification`, `quota`, `typescript`, `rust`
