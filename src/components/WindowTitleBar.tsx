import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect } from "react";

type WindowTitleBarProps = {
  visible: boolean;
};

export function WindowTitleBar({ visible }: WindowTitleBarProps) {
  useEffect(() => {
    if (!visible) {
      return;
    }

    const appWindow = getCurrentWindow();
    void appWindow.setDecorations(false);
    void appWindow.setShadow(true);
  }, [visible]);

  if (!visible) {
    return null;
  }

  const appWindow = getCurrentWindow();

  return (
    <header className="windowTitleBar">
      <div
        className="windowTitleDragRegion"
        data-tauri-drag-region
        onDoubleClick={() => void appWindow.toggleMaximize()}
      >
        <img className="windowTitleIcon" src="/codexdeck.png" alt="" aria-hidden="true" />
        <span>CodexDeck</span>
      </div>
      <div className="windowTitleControls" aria-label="窗口控制">
        <button
          type="button"
          className="windowTitleControl"
          aria-label="最小化"
          title="最小化"
          onClick={() => void appWindow.minimize()}
        >
          <span aria-hidden="true">-</span>
        </button>
        <button
          type="button"
          className="windowTitleControl"
          aria-label="最大化"
          title="最大化"
          onClick={() => void appWindow.toggleMaximize()}
        >
          <span aria-hidden="true">□</span>
        </button>
        <button
          type="button"
          className="windowTitleControl windowTitleControlClose"
          aria-label="关闭"
          title="关闭"
          onClick={() => void appWindow.close()}
        >
          <span aria-hidden="true">×</span>
        </button>
      </div>
    </header>
  );
}
