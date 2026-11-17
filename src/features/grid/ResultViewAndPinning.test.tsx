import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { client } from "../../ipc/client";
import type { ColumnInfo, ResultInfo } from "../../ipc/generated";
import { useStudio, MAX_PINNED_RESULTS_PER_TAB } from "../../state/studio";
import {
  cycleResultSort,
  ResultQuickFilterBar,
  resultSortIndicator,
  useResultViewState,
} from "./result-view";

function makeCol(id: string, name: string, kind: ColumnInfo["kind"] = "text"): ColumnInfo {
  return {
    id,
    name,
    databaseType: kind,
    kind,
    nullable: true,
    primaryKey: id === "c_id",
    foreignKey: false,
    editable: true,
    insertable: true,
    hasDefault: false,
  };
}

function makeResult(id: string, statementIndex = 0, retainedRows = 5): ResultInfo {
  return {
    id,
    runId: `run-${id}`,
    statementIndex,
    columns: [makeCol("c_id", "id", "integer"), makeCol("c_name", "name", "text")],
    retainedRows,
    status: "complete",
    table: { schema: "public", name: "orders" },
    editability: {
      status: "editable",
      table: { schema: "public", name: "orders" },
      primaryKey: ["id"],
    },
  };
}

function ResultFilterHarness({
  result,
  lockedReason,
  onNotify,
}: {
  result: ResultInfo;
  lockedReason?: string | null;
  onNotify?: (msg: string) => void;
}) {
  const view = useResultViewState(result, { lockedReason, onNotify });
  return (
    <div>
      <ResultQuickFilterBar columns={result.columns} view={view} />
      <button type="button" onClick={() => view.toggleSort(result.columns[0], false)}>
        Sort ID
      </button>
      <div data-testid="mapped-indices">
        {view.displayRowIndices ? view.displayRowIndices.join(",") : "unfiltered"}
      </div>
    </div>
  );
}

describe("Local query result filter/sort and pinned console result subtabs", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useStudio.setState({
      tabs: [],
      activeTabId: null,
      sessions: {},
      sessionPool: {},
      consoleSessions: {},
      snowflakeConsoleReceipts: {},
      snowflakeConsoleResultOwners: {},
      toast: null,
    });
  });

  it("cycles single-column and Shift+click multi-column sort specs with indicators", () => {
    const s1 = cycleResultSort([], "c_id", false);
    expect(s1).toEqual([{ columnId: "c_id", descending: false }]);
    expect(resultSortIndicator(s1, "c_id")).toBe("▲");

    const s2 = cycleResultSort(s1, "c_id", false);
    expect(s2).toEqual([{ columnId: "c_id", descending: true }]);
    expect(resultSortIndicator(s2, "c_id")).toBe("▼");

    const s3 = cycleResultSort(s2, "c_id", false);
    expect(s3).toEqual([]);
    expect(resultSortIndicator(s3, "c_id")).toBe("");

    // Multi-column additive sort
    const m1 = cycleResultSort([{ columnId: "c_name", descending: false }], "c_id", true);
    expect(m1).toEqual([
      { columnId: "c_name", descending: false },
      { columnId: "c_id", descending: false },
    ]);
    expect(resultSortIndicator(m1, "c_name")).toBe("▲1");
    expect(resultSortIndicator(m1, "c_id")).toBe("▲2");

    const m2 = cycleResultSort(m1, "c_id", true);
    expect(resultSortIndicator(m2, "c_id")).toBe("▼2");

    const m3 = cycleResultSort(m2, "c_id", true);
    expect(m3).toEqual([{ columnId: "c_name", descending: false }]);
  });

  it("invokes query_result_view for substring filters, column scope, case toggle, and resets cleanly", async () => {
    const sample = makeResult("res-filter-1", 0, 4);
    const callSpy = vi.spyOn(client, "call").mockImplementation(async (cmd, args) => {
      if (cmd === "query_result_view") {
        const req = (args as { request: { filterText: string | null; caseSensitive: boolean } })
          .request;
        return {
          resultId: sample.id,
          rowIndices: req.caseSensitive ? [2] : [2, 0],
          matchedRows: req.caseSensitive ? 1 : 2,
          scannedRows: 4,
          truncated: false,
          elapsedMs: 1,
        };
      }
      throw new Error(`Unexpected IPC: ${cmd}`);
    });

    render(<ResultFilterHarness result={sample} />);
    expect(screen.getByTestId("mapped-indices").textContent).toBe("unfiltered");

    fireEvent.change(screen.getByLabelText("Filter result rows"), {
      target: { value: "Acme" },
    });
    await waitFor(() => {
      expect(screen.getByTestId("mapped-indices").textContent).toBe("2,0");
    });

    fireEvent.change(screen.getByLabelText("Result filter column"), {
      target: { value: "c_name" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Match case in result filter" }));
    await waitFor(() => {
      expect(screen.getByTestId("mapped-indices").textContent).toBe("2");
    });

    fireEvent.click(screen.getByRole("button", { name: "Reset result filter and sort" }));
    expect(screen.getByTestId("mapped-indices").textContent).toBe("unfiltered");
    expect(callSpy).toHaveBeenCalled();

    // Respects lockedReason when pending edits exist
    const notify = vi.fn();
    render(
      <ResultFilterHarness
        result={makeResult("res-locked")}
        lockedReason="Review pending edits first"
        onNotify={notify}
      />,
    );
    const sortButtons = screen.getAllByRole("button", { name: "Sort ID" });
    fireEvent.click(sortButtons[1]);
    expect(notify).toHaveBeenCalledWith("Review pending edits first");
  });

  it("pins result subtabs across query re-executions, releases only unpinned snapshots, and enforces cap", async () => {
    const released: string[] = [];
    vi.spyOn(client, "call").mockImplementation(async (cmd, args) => {
      if (cmd === "release_result") {
        released.push((args as { resultId: string }).resultId);
        return undefined;
      }
      return undefined;
    });

    useStudio.getState().openQuery("conn-1", "SELECT * FROM orders;");
    const tabId = useStudio.getState().tabs[0].id;
    const r1 = makeResult("res-run-1", 0, 3);
    const r2 = makeResult("res-run-1b", 1, 2);

    useStudio.getState().updateTab(tabId, {
      status: "completed",
      results: [r1, r2],
      activeResultId: r1.id,
    });

    // Pin r1 -> becomes read-only snapshot with pinned label; r2 stays unpinned
    expect(useStudio.getState().togglePinResult(tabId, r1.id)).toBe(true);
    let tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
    expect(tab.pinnedResultIds).toEqual([r1.id]);
    expect(tab.pinnedResultLabels?.[r1.id]).toMatch(/Pinned #\d+ \(Q1\)/);
    expect(tab.results.find((r) => r.id === r1.id)?.editability?.status).toBe("readOnly");

    // Re-running query releases unpinned r2, keeps pinned r1, and focuses new result r3
    await act(async () => {
      await useStudio.getState().prepareQueryRunResults(tabId);
    });
    expect(released).toEqual([r2.id]);
    tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
    expect(tab.results.map((r) => r.id)).toEqual([r1.id]);
    expect(tab.activeResultId).toBeNull();

    const r3 = makeResult("res-run-2", 0, 10);
    useStudio.getState().applyEvents(
      tabId,
      [
        { type: "started", runId: "run-2" },
        { type: "result", result: r3 },
        { type: "finished", runId: "run-2", status: "completed", elapsedMs: 8 },
      ],
      tab.revision,
    );
    tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
    expect(tab.results.map((r) => r.id)).toEqual([r1.id, r3.id]);
    expect(tab.activeResultId).toBe(r3.id);

    // Zero-result DML run falls back activeResultId to pinned r1
    await act(async () => {
      await useStudio.getState().prepareQueryRunResults(tabId);
    });
    expect(released).toEqual([r2.id, r3.id]);
    tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
    useStudio.getState().applyEvents(
      tabId,
      [
        { type: "started", runId: "run-3" },
        { type: "statementCompleted", runId: "run-3", index: 0, affectedRows: "1", elapsedMs: 4 },
        { type: "finished", runId: "run-3", status: "completed", elapsedMs: 4 },
      ],
      tab.revision,
    );
    tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
    expect(tab.activeResultId).toBe(r1.id);

    // Closing pinned subtab directly releases r1
    useStudio.getState().closeResultSubtab(tabId, r1.id);
    expect(released).toEqual([r2.id, r3.id, r1.id]);
    tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
    expect(tab.results).toEqual([]);
    expect(tab.pinnedResultIds).toBeUndefined();

    // Enforces MAX_PINNED_RESULTS_PER_TAB
    const many = Array.from({ length: MAX_PINNED_RESULTS_PER_TAB + 1 }, (_, i) =>
      makeResult(`res-cap-${i}`, i, 1),
    );
    useStudio.getState().updateTab(tabId, {
      status: "completed",
      results: many,
      activeResultId: many[0].id,
    });
    for (let i = 0; i < MAX_PINNED_RESULTS_PER_TAB; i++) {
      expect(useStudio.getState().togglePinResult(tabId, many[i].id)).toBe(true);
    }
    expect(useStudio.getState().togglePinResult(tabId, many[MAX_PINNED_RESULTS_PER_TAB].id)).toBe(
      false,
    );
    expect(useStudio.getState().toast).toMatch(/at most 8 results/i);
  });
});
