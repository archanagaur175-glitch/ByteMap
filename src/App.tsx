import { useRef } from "react";
import { TreeList } from "./TreeList";
import { OrbitView } from "./OrbitView";
import { StrataView } from "./StrataView";
import { OrbitBench } from "./OrbitBench";
import { Legend } from "./Legend";
import { ActionHub } from "./ActionHub";
import { Snapshots } from "./Snapshots";
import { useStore } from "./store";

export default function App() {
  // Perf bench mode: open with #orbit-bench in the URL.
  if (window.location.hash === "#orbit-bench") {
    return <OrbitBench />;
  }
  const {
    folder, phase, progress, error, rootId, view, viewMode, theme, selectedId, tailName,
    snapshotsOpen, loadedSnapshot, exportNotice,
    setFolder, startScan, cancelScan, openNode, setViewMode, setTheme,
    openSnapshots, closeSnapshots, exportViewCsv,
  } = useStore();

  const scanning = phase === "scanning";
  const otherTheme = theme === "dark" ? "light" : "dark";
  const orbitTabRef = useRef<HTMLButtonElement>(null);
  const strataTabRef = useRef<HTMLButtonElement>(null);

  // ARIA tabs pattern: Left/Right switch the view and move focus.
  const onTabsKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const next = viewMode === "orbit" ? "strata" : "orbit";
    setViewMode(next);
    (next === "orbit" ? orbitTabRef : strataTabRef).current?.focus();
  };

  return (
    <div id="app-shell">
      <header className="topbar">
        <input
          className="folder-input"
          placeholder="Folder to scan (e.g. %TEMP%\bytemap-bench-100k)"
          value={folder}
          onChange={(e) => setFolder(e.target.value)}
          disabled={scanning}
          aria-label="Folder to scan"
        />
        {scanning ? (
          <button className="btn cancel" onClick={() => void cancelScan()}>
            Cancel
          </button>
        ) : (
          <button className="btn primary" onClick={() => void startScan()}>
            Scan
          </button>
        )}
        <button
          className="toolbar-btn"
          onClick={() => (snapshotsOpen ? closeSnapshots() : openSnapshots())}
          aria-expanded={snapshotsOpen}
          aria-controls="snapshot-panel"
          title="save this scan / reopen a saved one"
        >
          Snapshots
        </button>
        <button
          className="theme-toggle"
          onClick={() => setTheme(otherTheme)}
          aria-label={`Switch to ${otherTheme} theme`}
          title={`Switch to ${otherTheme} theme (now: ${theme})`}
        >
          {theme === "dark" ? "☀ Light" : "☾ Dark"}
        </button>
        <button
          className="toolbar-btn"
          onClick={() => void exportViewCsv()}
          disabled={phase !== "done"}
          title="write this folder's items to a CSV in %LOCALAPPDATA%\ByteMap\exports"
        >
          Export CSV
        </button>
      </header>

      <Snapshots />

      {loadedSnapshot && (
        <div className="snapshot-banner" role="status" aria-live="polite">
          Snapshot mode · {loadedSnapshot.name} · {loadedSnapshot.rootName} · saved{" "}
          {new Date(loadedSnapshot.createdUnix * 1000).toLocaleString()} — rescan the folder to
          return to live data.
        </div>
      )}

      {progress && (
        <div className="progress-line" role="status" aria-live="polite">
          scanning… {progress.files.toLocaleString()} files, {progress.dirs.toLocaleString()} folders
          {progress.denied > 0 ? `, ${progress.denied.toLocaleString()} denied` : ""}
          {" · "}
          {(progress.elapsedMs / 1000).toFixed(1)}s
        </div>
      )}
      {error && (
        <div className="error-line" role="alert">
          error: {error}
        </div>
      )}
      {exportNotice && (
        <div className="hub-notice export-notice" role="status" aria-live="polite">
          {exportNotice}
        </div>
      )}

      {view && (
        <>
          <nav className="breadcrumb" aria-label="Folder path">
            {view.ancestors.map((a) => (
              <button
                key={a.id}
                type="button"
                className="crumb"
                onClick={() => void openNode(a.id)}
              >
                {a.name}
              </button>
            ))}
            <span className="crumb current" aria-current="page">
              {tailName ?? "root"}
            </span>
          </nav>
          <Legend />
          <div
            className="view-switch"
            role="tablist"
            aria-label="Visual model"
            onKeyDown={onTabsKeyDown}
          >
            <button
              ref={orbitTabRef}
              id="tab-orbit"
              role="tab"
              aria-selected={viewMode === "orbit"}
              aria-controls="visual-view"
              tabIndex={viewMode === "orbit" ? 0 : -1}
              className={viewMode === "orbit" ? "view-tab active" : "view-tab"}
              onClick={() => setViewMode("orbit")}
            >
              Orbit
            </button>
            <button
              ref={strataTabRef}
              id="tab-strata"
              role="tab"
              aria-selected={viewMode === "strata"}
              aria-controls="visual-view"
              tabIndex={viewMode === "strata" ? 0 : -1}
              className={viewMode === "strata" ? "view-tab active" : "view-tab"}
              onClick={() => setViewMode("strata")}
            >
              Strata
            </button>
          </div>
          {viewMode === "orbit" ? <OrbitView /> : <StrataView />}
          <div className="stats-line" role="status" aria-live="polite">
            {view.totalChildren.toLocaleString()} items in this folder
            {rootId === null ? " (no scan yet)" : ""}
          </div>
          <TreeList
            items={view.items}
            childDepth={view.ancestors.length}
            selectedId={selectedId}
            onOpen={(id) => void openNode(id)}
          />
          <ActionHub />
        </>
      )}
      {!view && !progress && !error && (
        <div className="empty-hint">
          ByteMap · enter a folder and press Scan. Rust owns the tree; this window only shows
          views (top-N children, the rest folded into "N small items").
        </div>
      )}
    </div>
  );
}
