import type { ThemeConfig } from "antd";

export const codexSwitchPalette = {
  ink: "#1f1f1f",
  inkSoft: "#595959",
  subtle: "#8c8c8c",
  parchment: "#f5f7fa",
  parchmentStrong: "#ffffff",
  paper: "#ffffff",
  paperElevated: "#fafafa",
  sand: "#f0f5ff",
  border: "#d9d9d9",
  borderSoft: "rgba(5, 5, 5, 0.06)",
  brand: "#1677ff",
  brandHover: "#4096ff",
  brandSoft: "rgba(22, 119, 255, 0.12)",
  green: "#52c41a",
  greenSoft: "rgba(82, 196, 26, 0.12)",
  warning: "#faad14",
  danger: "#ff4d4f",
  dangerSoft: "rgba(255, 77, 79, 0.1)",
  focus: "#1677ff",
} as const;

export const codexSwitchRadii = {
  xs: 4,
  sm: 8,
  md: 8,
  lg: 12,
  xl: 12,
} as const;

export const codexSwitchShadows = {
  card: "0 6px 16px rgba(5, 5, 5, 0.04), 0 1px 2px rgba(5, 5, 5, 0.03)",
  floating: "0 12px 32px rgba(5, 5, 5, 0.12), 0 0 0 1px rgba(5, 5, 5, 0.06)",
  focus: "0 0 0 3px rgba(22, 119, 255, 0.12)",
} as const;

export const codexSwitchAntdTheme: ThemeConfig = {
  token: {
    colorPrimary: codexSwitchPalette.brand,
    colorInfo: codexSwitchPalette.brand,
    colorSuccess: codexSwitchPalette.green,
    colorWarning: codexSwitchPalette.warning,
    colorError: codexSwitchPalette.danger,
    colorTextBase: codexSwitchPalette.ink,
    colorBgBase: codexSwitchPalette.parchment,
    colorBgLayout: codexSwitchPalette.parchment,
    colorBgContainer: codexSwitchPalette.paper,
    colorBgElevated: codexSwitchPalette.parchmentStrong,
    colorBorder: codexSwitchPalette.border,
    colorBorderSecondary: codexSwitchPalette.borderSoft,
    colorLink: codexSwitchPalette.brand,
    borderRadius: codexSwitchRadii.md,
    controlHeight: 32,
    controlHeightSM: 24,
    controlHeightLG: 40,
    fontFamily:
      '"MiSans", "HarmonyOS Sans SC", "Microsoft YaHei UI", "PingFang SC", "Segoe UI", sans-serif',
    fontFamilyCode: '"Maple Mono NF CN", "JetBrains Mono", "SFMono-Regular", Consolas, monospace',
    fontSize: 14,
    lineWidth: 1,
    wireframe: false,
  },
  components: {
    Button: {
      borderRadius: codexSwitchRadii.sm,
      controlHeight: 32,
      defaultBg: codexSwitchPalette.paper,
      defaultBorderColor: codexSwitchPalette.border,
      defaultColor: codexSwitchPalette.ink,
      primaryShadow: "none",
    },
    Card: {
      borderRadiusLG: codexSwitchRadii.xl,
      colorBgContainer: codexSwitchPalette.paper,
      colorBorderSecondary: codexSwitchPalette.borderSoft,
      paddingLG: 18,
    },
    Input: {
      borderRadius: codexSwitchRadii.sm,
      activeBorderColor: codexSwitchPalette.focus,
      hoverBorderColor: codexSwitchPalette.focus,
      activeShadow: codexSwitchShadows.focus,
    },
    Menu: {
      itemBg: "transparent",
      itemSelectedBg: "#e6f4ff",
      itemSelectedColor: codexSwitchPalette.ink,
      itemHoverBg: "#f0f5ff",
      itemHoverColor: codexSwitchPalette.ink,
      itemBorderRadius: codexSwitchRadii.sm,
    },
    Modal: {
      borderRadiusLG: codexSwitchRadii.xl,
      contentBg: codexSwitchPalette.parchmentStrong,
      headerBg: codexSwitchPalette.parchmentStrong,
    },
    Select: {
      borderRadius: codexSwitchRadii.sm,
      optionSelectedBg: "#e6f4ff",
    },
    Table: {
      borderColor: codexSwitchPalette.borderSoft,
      headerBg: codexSwitchPalette.paperElevated,
      headerColor: codexSwitchPalette.subtle,
      rowHoverBg: "#f0f5ff",
      cellPaddingBlock: 12,
      cellPaddingInline: 14,
    },
    Tabs: {
      itemActiveColor: codexSwitchPalette.ink,
      itemHoverColor: codexSwitchPalette.brand,
      itemSelectedColor: codexSwitchPalette.ink,
      inkBarColor: codexSwitchPalette.brand,
    },
    Tag: {
      borderRadiusSM: 999,
      defaultBg: codexSwitchPalette.paperElevated,
      defaultColor: codexSwitchPalette.subtle,
    },
  },
};
