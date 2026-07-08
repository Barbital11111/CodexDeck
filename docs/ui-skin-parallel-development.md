# CodexDeck UI 双皮肤并行开发说明

状态：2026-06-26 已落地基础并行机制。本文只记录皮肤边界，避免后续继续把 classic 和 modern 混在同一套组件或样式里改。

## 皮肤定义

- `classic`：从 `HEAD` 源码复制经典组件和经典样式作用域，保留原版入口布局与蓝白观感。
- `modern`：承接新 UI 重构方向，使用暖橙主色和新版 Header / 供应商模型入口。

## 入口与预览

普通启动仍读取本地设置：

- localStorage：`codexdeck-ui-skin`
- 兼容旧键：`codexdeck-card-skin`

并行预览使用 URL 覆盖，不写入 localStorage：

- classic：`http://127.0.0.1:5173/?codexdeckPreviewWindow=1&uiSkin=classic`
- modern：`http://127.0.0.1:5173/?codexdeckPreviewWindow=1&uiSkin=modern`

## 代码边界

- `src/hooks/useUiSkinMode.ts`：读取 URL/localStorage，写入 `data-ui-skin` 和 `data-card-skin`。
- `src/App.tsx`：按 `uiSkinMode` 分流页面结构；classic 使用 `src/components/classic/*`，modern 使用现有新版组件。
- `src/components/classic/*`：从 `HEAD` 提取的经典 UI 组件副本，后续 classic 修改只在这里推进。
- `src/theme/codexSwitchTokens.ts`：导出 `codexSwitchAntdThemes.classic` 和 `codexSwitchAntdThemes.modern`，供 `ConfigProvider` 按皮肤切换。
- `src/styles/themes/light.css` / `dark.css`：原版变量和新版变量按 `data-ui-skin` 分流。
- `src/styles/tokens.css` / `antd-theme.css`：补充 UI token 和 `--cs-*` token 的皮肤分流。
- `src/styles/ui-polish.css`：只允许写 `modern` 视觉校准；不得再写 classic 临时补丁。
- `src/styles/classic-restore.css`：由 `HEAD:src/styles/*` 生成并作用域到 `data-ui-skin="classic"`，用于压回经典样式。

## 开发规则

- 改 classic 时只恢复原版布局/配色，不把 modern 的暖橙 token 混进去。
- 改 modern 时只推进新版结构，不覆盖 classic 的蓝白变量。
- classic 的页面结构优先改 `src/components/classic/*`；modern 的页面结构优先改 `src/components/*` 当前新版组件。
- 不要把 modern 的 `AppHeader`、`ProvidersView`、`launchModePanel` 路由启动结构塞进 classic。
- classic 的添加账号弹窗、设置页、通知页也必须走 `src/components/classic/*`，不要复用 modern 面板。
- 新增全局 CSS 变量时，除非是尺寸/字体等无主题含义的 token，否则必须按 `data-ui-skin` 分流。
- 新增 AntD 组件级主题时，优先改 `codexSwitchAntdThemes`，不要只写 CSS 硬覆盖。
- PC 桌面优先；不再把手机宽度作为当前验收目标。

## 当前验证

已验证：

- `npx tsc --noEmit`
- `npm run lint -- --max-warnings=0`
- `npm run build`
- Playwright 打开 classic / modern 两个 URL 并行预览。

验证读数：

- classic：`--brand = #1677ff`，无 `.appHeader`，无“供应商与模型”，侧栏含“概览 / 账户列表 / 额度总览 / 使用统计 / 通知中心 / 设置”，使用旧 `hybridLaunchPanel`。
- classic 添加账号：使用 `.settingsDialog.addAuthDialog` 旧弹窗，无 `.addAccountModal`，无“预设供应商”和供应商小卡片。
- classic 设置页：无“界面皮肤”开关。
- classic 通知页：无 `.appHeader`，无“供应商与模型”入口。
- modern：`--brand = #dd741f`，有 `.appHeader`，侧栏含“账户 / 供应商与模型 / 通知中心 / 设置”，使用新版 `launchModePanel` 和路由启动。
- classic / modern 冷刷新后 console error 为 0。

未完成：

- `npm run build` 仍有一个约 1 MB `index` chunk 警告，原因是 classic 组件副本同步进入主入口；后续应把 classic 面板也 lazy-load。
