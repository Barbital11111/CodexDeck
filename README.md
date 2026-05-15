# CodexDeck

CodexDeck 是一个面向 Codex 使用者的桌面工作台，用来集中管理 Codex 账号、API 中转配置、额度状态和通知链路。

它适合同时维护多个 Codex OAuth 账号、多个 API 平台，或者希望把账号状态、额度查询、API 配置和通知规则放到同一个本地控制台里管理的使用场景。

![CodexDeck 账号工作台](docs/images/accounts-overview.png)

## 主要能力

- 管理 Codex OAuth 账号、账号分组和 API 中转配置。
- 在账号和 API profile 之间快速切换，减少手动修改本地配置文件。
- 查看 Codex 账号额度；API 平台绑定账号后，也可以显示可查询到的用量和余额。
- 通过数据源、发送渠道、消息模板和计划规则创建通知链路。
- 预留远程控制入口，用于后续接入手机端或远程管理流程。

## 使用流程

### 账号与 API 工作台

账号列表会把 Codex 账号和 API 中转条目放在同一个工作区里。API 条目可以是仅 API Key 的配置，也可以绑定平台账号，用于显示余额、订阅额度或管理员统计信息。

![账号与 API 工作台](docs/images/accounts-overview.png)

### 导入账号

CodexDeck 支持 OAuth 网页登录、同步当前设备登录态、上传账号文件，以及直接导入 API。

![首次使用添加账号](docs/images/first-use-add-account.png)

![OAuth 账号导入](docs/images/account-import-oauth.png)

导入 API 时，只需要填写名称、Base URL、API Key 和模型名称即可保存为独立 profile。

![API 基础导入](docs/images/api-import-basic.png)

如果开启余额显示，可以额外填写平台账号信息，用于后续查询 API 平台的余额或用量。

![API 余额显示](docs/images/api-import-quota.png)

### 创建通知链路

通知链路由几个可复用模块组成：

1. 添加额度或用量数据源。
2. 添加 Telegram Bot、Webhook 等发送渠道。
3. 选择内置模板，或编辑自定义消息模板。
4. 创建通知规则，设置每日定时、间隔推送或手动触发，并在保存前发送测试消息。

![通知中心概览](docs/images/notification-home.png)

![通知数据源](docs/images/notification-data-sources.png)

![新建通知规则](docs/images/notification-rule-drawer.png)

## 当前状态

CodexDeck 正在准备作为新的独立发布线。当前版本重点覆盖账号管理、API profile 管理、额度可视化和通知规则的基础能力。

远程控制目前是预留入口：界面入口已经存在，但手机端和远程管理后端会在后续版本中接入。

## 本地开发

```bash
npm install
npm run dev
```

桌面端预览：

```bash
npm run dev:desktop
```

常用验证：

```bash
npm run lint
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

## License

MIT
