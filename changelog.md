# 更新日志

- v2.0.4
  1. 接入外部远程控制安装版 runtime，CodexDeck 负责启动/停止控制台、展示状态、日志、连接地址、连接码和 APK 安装入口。
  2. 修复 API profile 写入规则，固定使用 `codexdeck_api` provider，并关闭 responses websocket。
  3. 新增混合模式：选择一个官方账号和一个 API 条目后，写入官方登录态 + `experimental_bearer_token`，让 Codex 保持账号态并走 API 中转。
  4. 切换 API、普通账号或混合模式时同步修复 Codex 线程 provider 与可见性，降低历史会话不可见风险。
  5. provider 同步备份移动到安装目录下的 `codex-state-provider-backups`，避免占用系统盘。
  6. 打包流程增加远控 runtime staging、运行态排除、manifest 校验和脱敏扫描。
  7. 修复 2.0.4 同版本覆盖安装逻辑，安装器会优先识别现有 CodexDeck、旧 Codex Tools 和 CodexSwitch 目录，避免误装到新的默认目录。
  8. release 构建增加 Rust 路径重映射，避免安装包内残留本机 Cargo / 用户目录路径。
  9. 修复远程控制启动兼容性：生产路径改用系统自带 Windows PowerShell，并修复带空格安装目录下 Node 参数截断和 runtime JS 包装中文乱码问题。

- v2.0.0
  1. 创建 CodexDeck 独立仓库，正式切换为新的产品身份与安装包命名。
  2. 移除首发版本中的远程控制页面和运行时入口，后续按独立运行时适配器重新接入。
  3. 保留账号管理、API 配置、用量统计、智能切换和通知中心作为 2.0.0 基线能力。
  4. 统一仓库地址、更新地址、开发预览脚本、发布脚本和应用元数据。
