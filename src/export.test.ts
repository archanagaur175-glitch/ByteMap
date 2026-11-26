// Tests for the CSV export bridge (P8.T4): the invoke contract Rust expects
// and the notice string the UI shows. No webview: invoke is mocked.

import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { exportCsv, formatExportNotice, type ExportResult } from "./export";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mocked = vi.mocked(invoke);

function result(over: Partial<ExportResult> = {}): ExportResult {
  return {
    path: "C:\\Users\\me\\AppData\\Local\\ByteMap\\exports\\bytemap-view-1.csv",
    name: "bytemap-view-1.csv",
    scope: "view",
    rows: 42,
    totalRows: 42,
    bytes: 4096,
    truncated: false,
    ...over,
  };
}

describe("exportCsv", () => {
  beforeEach(() => {
    mocked.mockReset();
    mocked.mockResolvedValue(result());
  });

  it("sends the scope and the open node id", async () => {
    await exportCsv("view", 7);
    expect(mocked).toHaveBeenCalledWith("export_csv", { scope: "view", nodeId: 7 });
  });

  it("omits the node id as null for the basket", async () => {
    mocked.mockResolvedValue(result({ scope: "basket", rows: 3 }));
    await exportCsv("basket");
    expect(mocked).toHaveBeenCalledWith("export_csv", { scope: "basket", nodeId: null });
  });

  it("propagates a Rust refusal", async () => {
    mocked.mockRejectedValue("a scan is running; finish or cancel it before exporting");
    await expect(exportCsv("view", 0)).rejects.toBe(
      "a scan is running; finish or cancel it before exporting",
    );
  });
});

describe("formatExportNotice", () => {
  it("names the row count and the full path", () => {
    const notice = formatExportNotice(result());
    expect(notice).toContain("42 rows");
    expect(notice).toContain(
      "C:\\Users\\me\\AppData\\Local\\ByteMap\\exports\\bytemap-view-1.csv",
    );
  });

  it("says one row, not one rows", () => {
    expect(formatExportNotice(result({ rows: 1, totalRows: 1 }))).toContain("1 row ");
  });

  it("is explicit when the cap dropped rows", () => {
    const notice = formatExportNotice(result({ rows: 100_000, totalRows: 250_000, truncated: true }));
    expect(notice).toContain("first 100,000 rows of 250,000");
  });
});
