// Action Hub (Phase 5): basket panel, dry-run report, execute buttons,
// undo. Safety mirrors the Rust gates — Protected tiers are never offered
// an add affordance, and permanent delete requires the user to type
// exactly "PERMANENT" (Rust re-checks the string).

import { useRef, useState } from "react";
import { formatBytes } from "./parseView";
import { CATEGORY_COLORS } from "./colors";
import { useStore, type BasketEntry, type DryRunEntry } from "./store";

const TIER_LABEL = ["safe", "review", "protected"];
const PERMANENT_CONFIRMATION = "PERMANENT";

function BasketRow(props: { item: BasketEntry; onRemove: (id: number) => void }) {
  const { item, onRemove } = props;
  return (
    <div className={`basket-row tier-${item.tier}`}>
      <span
        className="cat-dot"
        style={{ background: CATEGORY_COLORS[item.cat] ?? "#8892A6" }}
        aria-hidden="true"
      />
      <span className="basket-name" title={item.path}>
        {item.name}
      </span>
      <span className="tier-pill">{TIER_LABEL[item.tier] ?? "unknown"}</span>
      <span className="basket-size">{formatBytes(item.logical)}</span>
      <button className="btn tiny" onClick={() => onRemove(item.nodeId)} aria-label={`remove ${item.name}`}>
        ✕
      </button>
    </div>
  );
}

function DryRunRow(props: { e: DryRunEntry }) {
  const { e } = props;
  return (
    <div className={`dryrun-row status-${e.status}`}>
      <span className="status-pill">{e.status}</span>
      <span className="basket-name" title={e.path}>
        {e.path}
      </span>
      {e.inUse && <span className="status-pill in-use">in use</span>}
      {e.probeError && (
        <span className="status-pill probe-error" title={e.probeError}>
          probe failed
        </span>
      )}
    </div>
  );
}

export function ActionHub() {
  const {
    basket, basketNotice, dryRun, lastReport, lastUndo,
    removeFromBasket, clearBasket, addToBasket, runDryRun, executeAction, undoLast,
    exportBasketCsv,
  } = useStore();

  const [confirmText, setConfirmText] = useState("");
  const [showPermanent, setShowPermanent] = useState(false);
  const permanentBtnRef = useRef<HTMLButtonElement>(null);
  const modalRef = useRef<HTMLDivElement>(null);
  const confirmed = confirmText === PERMANENT_CONFIRMATION;

  const closePermanent = () => {
    setShowPermanent(false);
    setConfirmText("");
    // Return focus to the control that opened the dialog.
    requestAnimationFrame(() => permanentBtnRef.current?.focus());
  };

  // Dialog keyboard contract: Esc closes, Tab is trapped inside, and the
  // focusable set excludes the disabled "Delete forever" button.
  const onModalKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      closePermanent();
      return;
    }
    if (e.key !== "Tab") return;
    const focusables = modalRef.current?.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input, select, textarea, a[href], [tabindex]:not([tabindex="-1"])',
    );
    if (!focusables || focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  const addVisible = () => {
    // Bulk add of the current view's real items happens via the row buttons
    // in TreeList (single adds) — this button bulk-adds every addable item
    // in the current view. Folded entries are skipped (no single node id).
    const s = useStore.getState();
    const ids = (s.view?.items ?? [])
      .filter((it) => !it.isFolded && (it.tier === 0 || it.tier === 1) && !(it.flags & 1))
      .map((it) => it.id);
    void addToBasket(ids, false);
  };

  return (
    <section className="action-hub" aria-label="Action Hub">
      <div className="hub-head">
        <span className="hub-title">Action Hub</span>
        <span className="hub-count" role="status" aria-live="polite">
          {basket.length} staged
        </span>
        <button className="btn tiny" onClick={() => void addVisible()} disabled={basket.length > 0 && basket.length > 500}>
          Add addable items in view
        </button>
        <button className="btn tiny" onClick={() => void clearBasket()} disabled={basket.length === 0}>
          Clear
        </button>
      </div>

      {basketNotice && (
        <div className="hub-notice" role="status" aria-live="polite">
          {basketNotice}
        </div>
      )}

      {basket.length > 0 && (
        <div className="basket-list">
          {basket.map((item) => (
            <BasketRow key={item.nodeId} item={item} onRemove={(id) => void removeFromBasket(id)} />
          ))}
        </div>
      )}

      <div className="hub-actions">
        <button className="btn" onClick={() => void runDryRun()} disabled={basket.length === 0}>
          Dry run
        </button>
        <button
          className="btn"
          onClick={() => void exportBasketCsv()}
          disabled={basket.length === 0}
          title="write the staged list to a CSV in %LOCALAPPDATA%\ByteMap\exports"
        >
          CSV
        </button>
        <button className="btn primary" onClick={() => void executeAction("recycle")} disabled={basket.length === 0}>
          Recycle
        </button>
        <button className="btn" onClick={() => void executeAction("archive")} disabled={basket.length === 0}>
          Archive
        </button>
        <button
          ref={permanentBtnRef}
          className="btn danger"
          disabled={basket.length === 0}
          onClick={() => {
            setShowPermanent(true);
            setConfirmText("");
          }}
        >
          Permanent…
        </button>
        <button className="btn" onClick={() => void undoLast()}>
          Undo last
        </button>
      </div>

      {dryRun && (
        <div className="dryrun-report" role="region" aria-label="Dry run report">
          <div className="dryrun-head" role="status" aria-live="polite">
            Dry run — {dryRun.filter((e) => e.status === "ok").length} ok,{" "}
            {dryRun.filter((e) => e.status !== "ok").length} will be dropped
          </div>
          {dryRun.map((e) => (
            <DryRunRow key={e.nodeId} e={e} />
          ))}
        </div>
      )}

      {lastReport && (
        <div className="hub-report" role="status" aria-live="polite">
          {lastReport.executed.length > 0 && (
            <div className="hub-report-line ok">
              {lastReport.executed.length} item{lastReport.executed.length === 1 ? "" : "s"} processed
              {lastReport.journaled ? " (journaled — undo available)" : ""}
            </div>
          )}
          {lastReport.dropped.length > 0 && (
            <div className="hub-report-line warn">
              {lastReport.dropped.length} dropped (changed/missing since scan)
            </div>
          )}
          {lastReport.failed.length > 0 && (
            <div className="hub-report-line error">
              {lastReport.failed.length} failed: {lastReport.failed[0].error}
            </div>
          )}
        </div>
      )}

      {lastUndo && (
        <div className="hub-report" role="status" aria-live="polite">
          {lastUndo.undoable ? (
            <div className="hub-report-line ok">
              Undo ({lastUndo.action}): {lastUndo.restored?.length ?? 0} restored
              {(lastUndo.notFound?.length ?? 0) + (lastUndo.conflicts?.length ?? 0) > 0
                ? `, ${(lastUndo.notFound?.length ?? 0) + (lastUndo.conflicts?.length ?? 0)} not restored`
                : ""}
              {lastUndo.error ? ` — ${lastUndo.error}` : ""}
            </div>
          ) : (
            <div className="hub-report-line warn">
              Not undoable{lastUndo.action ? ` (${lastUndo.action})` : ""}: {lastUndo.reason}
            </div>
          )}
        </div>
      )}

      {showPermanent && (
        <div className="modal-backdrop" onClick={closePermanent}>
          <div
            className="modal"
            ref={modalRef}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={onModalKeyDown}
            role="dialog"
            aria-modal="true"
            aria-labelledby="permanent-title"
            aria-describedby="permanent-desc"
          >
            <h3 id="permanent-title">
              Permanently delete {basket.length} item{basket.length === 1 ? "" : "s"}?
            </h3>
            <p className="modal-warn" id="permanent-desc">
              This cannot be undone. Protected items are refused automatically.
            </p>
            <p>
              Type <strong>PERMANENT</strong> to confirm:
            </p>
            <input
              className="folder-input"
              value={confirmText}
              onChange={(e) => setConfirmText(e.target.value)}
              autoFocus
              aria-label="type PERMANENT to confirm"
            />
            <div className="modal-actions">
              <button className="btn" onClick={closePermanent}>
                Cancel
              </button>
              <button
                className="btn danger"
                disabled={!confirmed}
                onClick={() => {
                  const text = confirmText;
                  closePermanent();
                  void executeAction("permanent", text);
                }}
              >
                Delete forever
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
