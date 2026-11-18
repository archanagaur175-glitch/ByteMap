import { TreeList } from "./TreeList";
import { OrbitView } from "./OrbitView";
import { StrataView } from "./StrataView";
import { OrbitBench } from "./OrbitBench";
import { Legend } from "./Legend";
import { ActionHub } from "./ActionHub";
import { useStore } from "./store";

export default function App() {
  // Perf bench mode: open with #orbit-bench in the URL.
  if (window.location.hash === "#orbit-bench") {
    return <OrbitBench />;
  }
  const {
    folder, phase, progress, error, rootId, view, viewMode, selectedId, tailName,
    setFolder, startScan, cancelScan, openNode, setViewMode,
  } = useStore();

  const scanning = phase === "scanning";

  return (
    <div id="app-shell">
      <header className="topbar">
        <input
          className="folder-input"
          placeholder="Folder to scan (e.g. %TEMP%\bytemap-bench-100k)"
          value={folder}
          onChange={(e) => setFolder(e.target.value)}
          disabled={scanning}
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
      </header>

      {progress && (
        <div className="progress-line">
          scanning… {progress.files.toLocaleString()} files, {progress.dirs.toLocaleString()} folders
          {progress.denied > 0 ? `, ${progress.denied.toLocaleString()} denied` : ""}
          {" · "}
          {(progress.elapsedMs / 1000).toFixed(1)}s
        </div>
      )}
      {error && <div className="error-line">error: {error}</div>}

      {view && (
        <>
          <nav className="breadcrumb">
            {view.ancestors.map((a) => (
              <span key={a.id} className="crumb" onClick={() => void openNode(a.id)}>
                {a.name}
              </span>
            ))}
            <span className="crumb current">{tailName ?? "root"}</span>
          </nav>
          <Legend />
          <div className="view-switch" role="tablist" aria-label="Visual model">
            <button
              role="tab"
              aria-selected={viewMode === "orbit"}
              className={viewMode === "orbit" ? "view-tab active" : "view-tab"}
              onClick={() => setViewMode("orbit")}
            >
              Orbit
            </button>
            <button
              role="tab"
              aria-selected={viewMode === "strata"}
              className={viewMode === "strata" ? "view-tab active" : "view-tab"}
              onClick={() => setViewMode("strata")}
            >
              Strata
            </button>
          </div>
          {viewMode === "orbit" ? <OrbitView /> : <StrataView />}
          <div className="stats-line">
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
