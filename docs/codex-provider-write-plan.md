# CodexDeck Provider 写入方案

本文档记录 CodexDeck 在“普通 Codex 账号”和“API 中转站账号”之间切换时，最终应写入 Codex 配置与线程 provider 的规则。

## 核心修正

之前 2.0.3 的问题不是“不能写 provider”，而是 provider 没有统一：

- `model_provider` 写成了 `custom`。
- `[model_providers.*]` 里又出现了 `codexdeck_api` 或其他名字。
- provider 的 `name` 又写成了 `CodexDeck API`。
- `state_5.sqlite` 里的历史线程 provider 没有同步成同一个值。

正确逻辑是：provider 是一个稳定身份，URL 只是这个 provider 的 `base_url` 属性。

因此第一版固定使用 CodexDeck 管理的 provider id：

```text
codexdeck_api
```

后续如果要支持用户自定义 provider id，也必须保证下面三处完全一致：

```toml
model_provider = "<provider_id>"

[model_providers.<provider_id>]
name = "<provider_id>"
```

并且同步 rollout 元数据、`state_5.sqlite` 中的线程可见性字段和 workspace roots 为同一个 `<provider_id>`。

## API 中转站登录时写入

假设 API 配置为：

- Base URL：`https://gateway.example.invalid/v1`
- 模型：`gpt-5.4`

CodexDeck 写入 `~/.codex/config.toml` 的受控字段为：

```toml
model_provider = "codexdeck_api"
model = "gpt-5.4"
openai_base_url = "https://gateway.example.invalid/v1"
cli_auth_credentials_store = "file"

[features]
responses_websockets = false
responses_websockets_v2 = false

[model_providers]

[model_providers.codexdeck_api]
name = "codexdeck_api"
base_url = "https://gateway.example.invalid/v1"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = false
```

重点：

- `model_provider` 固定为 `codexdeck_api`。
- `[model_providers.codexdeck_api]` 和 `name = "codexdeck_api"` 保持一致。
- `base_url` 才随着用户填写的 API 地址变化。
- `supports_websockets = false` 禁止该 provider 使用 websocket。
- `[features] responses_websockets = false / responses_websockets_v2 = false` 从全局 feature 层面禁用 websocket。

## API Key 写入

API Key 不写入 `config.toml`，而是写入当前 profile 对应的 `auth.json`：

```json
{
  "OPENAI_API_KEY": "sk-...",
  "auth_mode": "apikey"
}
```

切换账号时，CodexDeck 会把该 profile 的 `auth.json` 应用到 `~/.codex/auth.json`。

## 普通 Codex 账号登录时写入

切回普通 Codex 账号时，CodexDeck 会移除 API 中转站受控字段：

```toml
openai_base_url
model_provider
[model_providers]
[features].responses_websockets
[features].responses_websockets_v2
```

如果 `[features]` 中还有其他用户配置，只删除上面两个 websocket 字段，保留其他字段。

普通账号继续使用：

```toml
cli_auth_credentials_store = "file"
```

并使用该账号自己的 OAuth `auth.json`。

## 历史线程可见性同步

Codex 的线程列表不只依赖 `state_5.sqlite` 中的 `threads.model_provider`。
Cockpit 能修复会话可见性的原因，是它同时对齐了三层索引：

因此：

- rollout 文件首行 `session_meta.payload.model_provider`。
- `state_5.sqlite.threads.model_provider`。
- `state_5.sqlite.threads.has_user_event`，从 rollout 中的用户消息事件反推。
- `state_5.sqlite.threads.cwd`，从 rollout 的 `session_meta.payload.cwd` 反推，并把 `\\?\D:\...` 这类 Windows extended path 转回普通桌面路径。
- `.codex-global-state.json` 中的 saved/project/active workspace roots，保证工作区路径和 SQLite 的 `cwd` 一致。

CodexDeck 切换账号时使用同一套修复方式：

- 切换到 API 中转站时，同步所有可见性索引到 `codexdeck_api`。
- 切回普通 Codex 账号时，同步所有可见性索引到 `openai`。
- 同步前会先扫描是否真的存在待修改项；没有待修复项时不创建备份。
- SQLite 更新在事务中执行；rollout 和 workspace roots 写入失败时，已写入的 rollout 会尽量回滚，SQLite 事务不会提交。

备份不放在 `~/.codex` 目录，避免随着 `state_5.sqlite` 一起占用系统盘空间。备份放在 CodexDeck 可执行文件所在目录下：

```text
<CodexDeck 安装目录>/codex-state-provider-backups/provider-sync-<timestamp>/
```

备份内容包含：

- `db/state_5.sqlite`，以及存在时的 `state_5.sqlite-shm` / `state_5.sqlite-wal`。
- `.codex-global-state.json` 和 `.codex-global-state.json.bak`。
- `session-meta-backup.json`，只记录被改 rollout 文件的首行元数据，不复制整个会话文件树。
- `metadata.json`，记录备份命名空间、目标 provider、备份时间和实际变更文件数。

这样如果 provider 同步出现问题，可以从备份恢复关键索引。

备份会自动清理：

- 应用成功启动后会清理一次旧 provider 同步备份。
- 每次线程 provider 同步成功后也会清理一次旧备份。
- 默认只保留最近 1 份 `provider-sync-*` 备份，避免大体积 sqlite 备份长期占用磁盘。
- 旧版本曾经写到 `~/.codex` 目录下的 `state_5.sqlite.provider-sync-*.bak`，以及 Cockpit 默认写到 `~/.codex/backups_state/provider-sync` 的旧备份，会在下一次线程 provider 同步成功后清理；不会删除真正的 `state_5.sqlite`。

## 为什么不再从 URL 推导 Provider

不使用下面这种规则：

```text
https://gateway.example.invalid/v1 -> aihubmix
https://backup-gateway.invalid/v1  -> deepkey
```

原因是 URL 不是 provider 身份。用户切换不同 API 地址时，如果 provider id 跟着 URL 变，历史线程 provider 也会跟着变化，容易再次出现：

- 一部分线程是 `custom`
- 一部分线程是 `codexdeck_api`
- 一部分线程是按域名推导出的 provider

这会导致线程显示、恢复和 websocket 禁用逻辑都不稳定。

## 代理配置

默认不写入：

```toml
[shell_environment_policy]
set = { HTTP_PROXY = "...", HTTPS_PROXY = "..." }
```

这个配置只适合用户没有 TUN、只开 HTTP 代理的情况。它属于可选代理策略，不应该默认写入，否则可能破坏用户本机已有代理、公司网络或无代理环境。

## 受保护内容

CodexDeck 合并配置时不覆盖以下用户已有内容：

- `[mcp_servers.*]`
- `[projects.*]`
- `[marketplaces.*]`
- `[memories]`
- `[plugins.*]`
- 用户自定义但不属于账号切换的其他配置

账号切换只管理必要字段：

- `cli_auth_credentials_store`
- `model`
- `openai_base_url`
- `model_provider`
- `[model_providers]`
- `[features].responses_websockets`
- `[features].responses_websockets_v2`
- 上下文窗口相关字段
- sandbox / approval 中 CodexDeck 明确接管的字段

## 后续可扩展方向

如果未来要让用户自定义 provider id，应在设置或 API 编辑抽屉中提供一个高级字段，例如：

```text
Provider ID: codexdeck_api
```

但无论用户填什么，都必须做校验，并同步写入：

1. `model_provider = "<provider_id>"`
2. `[model_providers.<provider_id>]`
3. `name = "<provider_id>"`
4. rollout 元数据、`state_5.sqlite` 线程可见性字段和 workspace roots 同步到 `<provider_id>`

## 参考

- Linux.do 讨论中提到的 provider + 禁用 websocket 思路：<https://linux.do/t/topic/2077916>
