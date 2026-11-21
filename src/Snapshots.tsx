// Snapshots panel (Phase 8): save the current tree, then reopen it later
// without touching the disk. Rows come from the Rust header-only listing,
// so opening the panel never decompresses a snapshot body.

import { useState } from "react";
import { formatBytes } from "./parseView";
import { useStore, type SnapshotEntry } from "./store";
import { DiffView } from "./DiffView";

function when(unix: number): string {
  if (unix <= 0) return "unknown date";
  return new Date(unix * 1000).toLocaleString();
}

function SnapshotRow({ entry }: { entry: SnapshotEntry }) {
  const { loadSnapshot, deleteSnapshot } = useStore();

  if (entry.error) {
    return (
      <div className="snapshot-row broken">
        <span className="snapshot-name" title={entry.name}>
          {entry.name}
        </span>
        <span className="snapshot-meta" title={entry.error}>
          unreadable · {entry.error}
        </span>
        <button
          className="btn tiny danger"
          onClick={() => void deleteSnapshot(entry.name)}
          aria-label={`delete unreadable ${entry.name}`}
        >
          Delete
        </button>
      </div>
    );
  }

  return (
    <div className="snapshot-row">
      <span className="snapshot-name" title={`${entry.rootName} (${entry.name})`}>
        {entry.rootName}
      </span>
      <span className="snapshot-meta">
        {when(entry.createdUnix)} · {entry.nodes.toLocaleString()} entries ·{" "}
        {formatBytes(entry.totalLogical)}
        {entry.denied > 0 ? ` · ${entry.denied.toLocaleString()} denied` : ""}
      </span>
      <span className="tier-pill" title={`scanned by the ${entry.tier} tier`}>
        {entry.tier}
      </span>
      <button
        className="btn tiny"
        onClick={() => void loadSnapshot(entry.name)}
        aria-label={`load ${entry.name}`}
      >
        Load
      </button>
      <button
        className="btn tiny danger"
        onClick={() => void deleteSnapshot(entry.name)}
        aria-label={`delete ${entry.name}`}
      >
        Delete
      </button>
    </div>
  );
}

export function Snapshots() {
  const {
    snapshotsOpen, snapshots, snapshotNotice, phase, diff, diffLoading,
    closeSnapshots, saveSnapshot, diffSnapshots,
  } = useStore();
  // Local compare selection; defaults are the two newest snapshots.
  const [pickedOld, setPickedOld] = useState<string | null>(null);
  const [pickedNew, setPickedNew] = useState<string | null>(null);

  if (!snapshotsOpen) return null;

  // Saving needs a completed tree: a live scan completion or a loaded one.
  const canSave = phase === "done";

  if (diff || diffLoading) {
    return (
      <section id="snapshot-panel" className="action-hub snapshot-panel" aria-label="Snapshot changes">
        <DiffView />
      </section>
    );
  }

  const oldName = pickedOld ?? snapshots[1]?.name ?? "";
  const newName = pickedNew ?? snapshots[0]?.name ?? "";
  const canCompare =
    snapshots.length >= 2 && oldName !== "" && newName !== "" && oldName !== newName;

  const optionLabel = (entry: SnapshotEntry) =>
    `${when(entry.createdUnix)} · ${entry.rootName}`;

  return (
    <section id="snapshot-panel" className="action-hub snapshot-panel" aria-label="Snapshots">
      <div className="hub-head">
        <span className="hub-title">Snapshots</span>
        <span className="hub-count">
          {snapshots.length} saved · %LOCALAPPDATA%\ByteMap\snapshots
        </span>
        <button
          className="btn tiny primary"
          onClick={() => void saveSnapshot()}
          disabled={!canSave}
          title={canSave ? "save the current tree" : "scan a folder or load a snapshot first"}
        >
          Save current
        </button>
        <button className="btn tiny" onClick={closeSnapshots}>
          Close
        </button>
      </div>

      {snapshotNotice && <div className="hub-notice">{snapshotNotice}</div>}

      {snapshots.length >= 2 && (
        <div className="compare-bar">
          <span className="compare-label">Compare</span>
          <select
            className="compare-select"
            value={oldName}
            onChange={(e) => setPickedOld(e.target.value)}
            aria-label="older snapshot"
          >
            {snapshots.map((entry) => (
              <option key={entry.name} value={entry.name}>
                {optionLabel(entry)}
              </option>
            ))}
          </select>
          <span className="compare-arrow" aria-hidden="true">
            →
          </span>
          <select
            className="compare-select"
            value={newName}
            onChange={(e) => setPickedNew(e.target.value)}
            aria-label="newer snapshot"
          >
            {snapshots.map((entry) => (
              <option key={entry.name} value={entry.name}>
                {optionLabel(entry)}
              </option>
            ))}
          </select>
          <button
            className="btn tiny"
            disabled={!canCompare}
            onClick={() => void diffSnapshots(oldName, newName)}
            title={canCompare ? "compare these two snapshots" : "pick two different snapshots"}
          >
            Compare
          </button>
        </div>
      )}

      {snapshots.length === 0 ? (
        <div className="snapshot-empty">
          No snapshots yet. Save a completed scan to reopen it later without rescanning the disk.
        </div>
      ) : (
        <div className="snapshot-list">
          {snapshots.map((entry) => (
            <SnapshotRow key={entry.name} entry={entry} />
          ))}
        </div>
      )}
    </section>
  );
}
