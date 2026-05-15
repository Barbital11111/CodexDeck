type AppTopBarProps = {
  onGoHome: () => void;
};

export function AppTopBar({
  onGoHome,
}: AppTopBarProps) {
  return (
    <header className="topbar">
      <button type="button" className="commandSearch" onClick={onGoHome}>
        <span aria-hidden="true">⌕</span>
        <strong>搜索或运行命令...</strong>
        <kbd>⌘K</kbd>
      </button>
      <div className="topDragRegion" data-tauri-drag-region aria-hidden="true" />
    </header>
  );
}
