import type { ThemeConfig } from "antd";
import type { UiSkinMode } from "../types/app";

type Palette = {
  ink: string;
  inkSoft: string;
  subtle: string;
  parchment: string;
  parchmentStrong: string;
  paper: string;
  paperElevated: string;
  sand: string;
  border: string;
  borderSoft: string;
  brand: string;
  brandHover: string;
  brandSoft: string;
  green: string;
  greenSoft: string;
  warning: string;
  danger: string;
  dangerSoft: string;
  focus: string;
  focusShadow: string;
};

export const codexSwitchPalettes: Record<UiSkinMode, Palette> = {
  classic: {
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
    focusShadow: "0 0 0 3px rgba(22, 119, 255, 0.12)",
  },
  modern: {
    ink: "#1f1f1f",
    inkSoft: "#595959",
    subtle: "#8c8c8c",
    parchment: "#f5f7fa",
    parchmentStrong: "#ffffff",
    paper: "#ffffff",
    paperElevated: "#fafafa",
    sand: "rgba(221, 116, 31, 0.12)",
    border: "#d9d9d9",
    borderSoft: "rgba(5, 5, 5, 0.06)",
    brand: "#dd741f",
    brandHover: "#c4631a",
    brandSoft: "rgba(221, 116, 31, 0.12)",
    green: "#52c41a",
    greenSoft: "rgba(82, 196, 26, 0.12)",
    warning: "#faad14",
    danger: "#ff4d4f",
    dangerSoft: "rgba(255, 77, 79, 0.1)",
    focus: "#dd741f",
    focusShadow: "0 0 0 3px rgba(221, 116, 31, 0.26)",
  },
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
} as const;

function createCodexSwitchAntdTheme(palette: Palette): ThemeConfig {
  return {
    token: {
      colorPrimary: palette.brand,
      colorInfo: palette.brand,
      colorSuccess: palette.green,
      colorWarning: palette.warning,
      colorError: palette.danger,
      colorTextBase: palette.ink,
      colorBgBase: palette.parchment,
      colorBgLayout: palette.parchment,
      colorBgContainer: palette.paper,
      colorBgElevated: palette.parchmentStrong,
      colorBorder: palette.border,
      colorBorderSecondary: palette.borderSoft,
      colorLink: palette.brand,
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
        defaultBg: palette.paper,
        defaultBorderColor: palette.border,
        defaultColor: palette.ink,
        primaryShadow: "none",
      },
      Card: {
        borderRadiusLG: codexSwitchRadii.xl,
        colorBgContainer: palette.paper,
        colorBorderSecondary: palette.borderSoft,
        paddingLG: 18,
      },
      Input: {
        borderRadius: codexSwitchRadii.sm,
        activeBorderColor: palette.focus,
        hoverBorderColor: palette.focus,
        activeShadow: palette.focusShadow,
      },
      Menu: {
        itemBg: "transparent",
        itemSelectedBg: palette.brandSoft,
        itemSelectedColor: palette.ink,
        itemHoverBg: palette.sand,
        itemHoverColor: palette.ink,
        itemBorderRadius: codexSwitchRadii.sm,
      },
      Modal: {
        borderRadiusLG: codexSwitchRadii.xl,
        contentBg: palette.parchmentStrong,
        headerBg: palette.parchmentStrong,
      },
      Select: {
        borderRadius: codexSwitchRadii.sm,
        optionSelectedBg: palette.brandSoft,
      },
      Switch: {
        trackHeight: 22,
        trackMinWidth: 44,
        trackPadding: 2,
        handleSize: 18,
        trackHeightSM: 16,
        trackMinWidthSM: 28,
        handleSizeSM: 12,
      },
      Table: {
        borderColor: palette.borderSoft,
        headerBg: palette.paperElevated,
        headerColor: palette.subtle,
        rowHoverBg: palette.sand,
        cellPaddingBlock: 12,
        cellPaddingInline: 14,
      },
      Tabs: {
        itemActiveColor: palette.ink,
        itemHoverColor: palette.brand,
        itemSelectedColor: palette.ink,
        inkBarColor: palette.brand,
      },
      Tag: {
        borderRadiusSM: 999,
        defaultBg: palette.paperElevated,
        defaultColor: palette.subtle,
      },
    },
  };
}

export const codexSwitchAntdThemes: Record<UiSkinMode, ThemeConfig> = {
  classic: createCodexSwitchAntdTheme(codexSwitchPalettes.classic),
  modern: createCodexSwitchAntdTheme(codexSwitchPalettes.modern),
};

export const codexSwitchAntdTheme = codexSwitchAntdThemes.classic;
