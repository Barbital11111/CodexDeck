# 更新日志

- v2.0.4
  1. 接入外部远程控制安装版 runtime，CodexDeck 负责启动/停止控制台、展示状态、日志、连接地址、连接码和 APK 安装入口。
  2. 修复 API profile 写入规则，固定使用 `codexdeck_api` provider，并关闭 responses websocket。
  3. 切换 API/普通账号时同步修复 Codex 线程 provider 与可见性，降低历史会话不可见风险。
  4. provider 同步备份移动到安装目录下的 `codex-state-provider-backups`，避免占用系统盘。
  5. 打包流程增加远控 runtime staging、运行态排除、manifest 校验和脱敏扫描。

- v2.0.0
  1. 创建 CodexDeck 独立仓库，正式切换为新的产品身份与安装包命名。
  2. 移除首发版本中的远程控制页面和运行时入口，后续按独立运行时适配器重新接入。
  3. 保留账号管理、API 配置、用量统计、智能切换和通知中心作为 2.0.0 基线能力。
  4. 统一仓库地址、更新地址、开发预览脚本、发布脚本和应用元数据。
