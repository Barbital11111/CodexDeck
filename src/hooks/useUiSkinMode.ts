import { useCallback, useLayoutEffect, useState } from "react";
import type { UiSkinMode } from "../types/app";

const UI_SKIN_STORAGE_KEY = "codexdeck-ui-skin";
const LEGACY_CARD_SKIN_STORAGE_KEY = "codexdeck-card-skin";
const UI_SKIN_URL_PARAM = "uiSkin";
const MODERN_DEV_SKIN_PARAM = "modern-dev";

type UiSkinSetterInput = UiSkinMode | ((current: UiSkinMode) => UiSkinMode);

function readUrlUiSkinOverride(): UiSkinMode | null {
  if (typeof window === "undefined") {
    return null;
  }

  const requestedSkin = new URLSearchParams(window.location.search).get(UI_SKIN_URL_PARAM);
  if (requestedSkin === "modern" || requestedSkin === MODERN_DEV_SKIN_PARAM) {
    return "modern";
  }
  if (requestedSkin === "classic") {
    return "classic";
  }
  return null;
}

function readSavedUiSkin(): UiSkinMode {
  return "classic";
}

export function useUiSkinMode() {
  const [urlUiSkinOverride] = useState<UiSkinMode | null>(() => readUrlUiSkinOverride());
  const modernDevEnabled = urlUiSkinOverride === "modern";
  const [uiSkinMode, setUiSkinModeState] = useState<UiSkinMode>(
    () => urlUiSkinOverride ?? readSavedUiSkin(),
  );

  useLayoutEffect(() => {
    document.documentElement.setAttribute("data-ui-skin", uiSkinMode);
    document.documentElement.setAttribute("data-card-skin", uiSkinMode);
    if (!urlUiSkinOverride) {
      window.localStorage.setItem(UI_SKIN_STORAGE_KEY, uiSkinMode);
      window.localStorage.setItem(LEGACY_CARD_SKIN_STORAGE_KEY, uiSkinMode);
    }
  }, [uiSkinMode, urlUiSkinOverride]);

  const setUiSkinMode = useCallback(
    (nextMode: UiSkinSetterInput) => {
      setUiSkinModeState((current) => {
        const resolved = typeof nextMode === "function" ? nextMode(current) : nextMode;
        return modernDevEnabled ? resolved : "classic";
      });
    },
    [modernDevEnabled],
  );

  const toggleUiSkinMode = useCallback(() => {
    setUiSkinModeState((current) =>
      modernDevEnabled && current === "classic" ? "modern" : "classic",
    );
  }, [modernDevEnabled]);

  return {
    uiSkinMode,
    setUiSkinMode,
    toggleUiSkinMode,
  };
}
