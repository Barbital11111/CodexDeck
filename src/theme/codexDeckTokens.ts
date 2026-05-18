import type { ThemeConfig } from "antd";

export const codexDeckPalette = {
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

export const codexDeckRadii = {
  xs: 4,
  sm: 8,
  md: 8,
  lg: 12,
  xl: 12,
} as const;

export const codexDeckShadows = {
  card: "0 6px 16px rgba(5, 5, 5, 0.04), 0 1px 2px rgba(5, 5, 5, 0.03)",
  floating: "0 12px 32px rgba(5, 5, 5, 0.12), 0 0 0 1px rgba(5, 5, 5, 0.06)",
  focus: "0 0 0 3px rgba(22, 119, 255, 0.12)",
} as const;

export const codexDeckAntdTheme: ThemeConfig = {
  token: {
    colorPrimary: codexDeckPalette.brand,
    colorInfo: codexDeckPalette.brand,
    colorSuccess: codexDeckPalette.green,
    colorWarning: codexDeckPalette.warning,
    colorError: codexDeckPalette.danger,
    colorTextBase: codexDeckPalette.ink,
    colorBgBase: codexDeckPalette.parchment,
    colorBgLayout: codexDeckPalette.parchment,
    colorBgContainer: codexDeckPalette.paper,
    colorBgElevated: codexDeckPalette.parchmentStrong,
    colorBorder: codexDeckPalette.border,
    colorBorderSecondary: codexDeckPalette.borderSoft,
    colorLink: codexDeckPalette.brand,
    borderRadius: codexDeckRadii.md,
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
      borderRadius: codexDeckRadii.sm,
      controlHeight: 32,
      defaultBg: codexDeckPalette.paper,
      defaultBorderColor: codexDeckPalette.border,
      defaultColor: codexDeckPalette.ink,
      primaryShadow: "none",
    },
    Card: {
      borderRadiusLG: codexDeckRadii.xl,
      colorBgContainer: codexDeckPalette.paper,
      colorBorderSecondary: codexDeckPalette.borderSoft,
      paddingLG: 18,
    },
    Input: {
      borderRadius: codexDeckRadii.sm,
      activeBorderColor: codexDeckPalette.focus,
      hoverBorderColor: codexDeckPalette.focus,
      activeShadow: codexDeckShadows.focus,
    },
    Menu: {
      itemBg: "transparent",
      itemSelectedBg: "#e6f4ff",
      itemSelectedColor: codexDeckPalette.ink,
      itemHoverBg: "#f0f5ff",
      itemHoverColor: codexDeckPalette.ink,
      itemBorderRadius: codexDeckRadii.sm,
    },
    Modal: {
      borderRadiusLG: codexDeckRadii.xl,
      contentBg: codexDeckPalette.parchmentStrong,
      headerBg: codexDeckPalette.parchmentStrong,
    },
    Select: {
      borderRadius: codexDeckRadii.sm,
      optionSelectedBg: "#e6f4ff",
    },
    Table: {
      borderColor: codexDeckPalette.borderSoft,
      headerBg: codexDeckPalette.paperElevated,
      headerColor: codexDeckPalette.subtle,
      rowHoverBg: "#f0f5ff",
      cellPaddingBlock: 12,
      cellPaddingInline: 14,
    },
    Tabs: {
      itemActiveColor: codexDeckPalette.ink,
      itemHoverColor: codexDeckPalette.brand,
      itemSelectedColor: codexDeckPalette.ink,
      inkBarColor: codexDeckPalette.brand,
    },
    Tag: {
      borderRadiusSM: 999,
      defaultBg: codexDeckPalette.paperElevated,
      defaultColor: codexDeckPalette.subtle,
    },
  },
};
