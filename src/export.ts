// CSV export (P8.T4). Rust writes the file (RFC 4180) and returns where it
// landed — there is no dialog plugin, so exports always go to
// %LOCALAPPDATA%\ByteMap\exports. Sizes are raw bytes so spreadsheets can sum
// and sort them without parsing units.

import { invoke } from "@tauri-apps/api/core";

export type ExportScope = "view" | "basket";

/** Result of export_csv (mirrors the Rust JSON). */
export interface ExportResult {
  /** Absolute path of the written file. */
  path: string;
  name: string;
  scope: ExportScope;
  /** Rows written (after the 100k cap). */
  rows: number;
  /** Rows the tree/basket had before the cap. */
  totalRows: number;
  bytes: number;
  /** True when rows were dropped by the cap. */
  truncated: boolean;
}

export async function exportCsv(scope: ExportScope, nodeId?: number): Promise<ExportResult> {
  return invoke<ExportResult>("export_csv", {
    scope,
    nodeId: nodeId ?? null,
  });
}

/** Fixed grouping so the notice reads the same on every machine. */
const NUM = new Intl.NumberFormat("en-US");

/** One-line confirmation for the UI: row count plus the exact file path. */
export function formatExportNotice(res: ExportResult): string {
  const rows = `${NUM.format(res.rows)} row${res.rows === 1 ? "" : "s"}`;
  const head = res.truncated
    ? `Exported first ${rows} of ${NUM.format(res.totalRows)}`
    : `Exported ${rows}`;
  return `${head} → ${res.path}`;
}
