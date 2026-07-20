CodexDeck v2.1.5

- 优化受控 Codex 启动和候选副本生命周期，避免重复生成副本。
- 修复在非 Git 工作目录中持续重试 Git metadata 的问题。
- 临时 patch 失败后复用已复制的候选副本，减少重复等待。
