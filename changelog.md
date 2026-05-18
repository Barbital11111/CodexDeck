# 更新日志

## v2.0.4

- 将项目主线整理为独立 CodexDeck。
- 接入外部远程控制安装版 runtime，CodexDeck 只负责外壳控制和状态展示。
- 修复 API profile 写入规则，固定使用 CodexDeck 管理的 provider，并关闭 responses websocket。
- 切换 API/普通账号时同步修复 Codex 线程 provider 与可见性。
- provider 备份移动到安装目录下的 `codex-state-provider-backups`，避免占用系统盘。
- 打包流程增加远控 runtime staging、运行态排除、manifest 校验和脱敏扫描。
