import { TreeList } from "./TreeList";
import { useStore } from "./store";

export default function App() {
  const {
    folder, phase, progress, error, rootId, view, selectedId, tailName,
    setFolder, startScan, cancelScan, openNode,
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
