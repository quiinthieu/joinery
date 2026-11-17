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
    expect(document.querySelector(".result-quick-filter-count")?.textContent).toBe("2 / 4");

    fireEvent.change(screen.getByLabelText("Result filter column"), {
      target: { value: "c_name" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Match case in result filter" }));
    await waitFor(() => {
      expect(screen.getByTestId("mapped-indices").textContent).toBe("2");
    });
    expect(document.querySelector(".result-quick-filter-count")?.textContent).toBe("1 / 4");

    fireEvent.click(screen.getByRole("button", { name: "Reset view" }));
    expect(screen.getByTestId("mapped-indices").textContent).toBe("unfiltered");
    expect(document.querySelector(".result-quick-filter-count")).toBeNull();
    expect(callSpy).toHaveBeenCalled();

    // Surfaces inline error badge when query_result_view rejects (e.g. >100k row sort limit)
    vi.spyOn(client, "call").mockRejectedValueOnce(
      new Error("In-memory result sorting is capped at 100000 matching rows"),
    );
    fireEvent.click(screen.getByRole("button", { name: "Sort ID" }));
    await waitFor(() => {
      expect(document.querySelector(".result-quick-filter-error")?.textContent).toContain(
        "capped at 100000",
      );
    });
    fireEvent.click(screen.getByRole("button", { name: "Reset view" }));
    expect(document.querySelector(".result-quick-filter-error")).toBeNull();

    // Respects lockedReason on both toggleSort AND resetView when pending edits exist
    const notify = vi.fn();
    vi.spyOn(client, "call").mockResolvedValue({
      resultId: "res-locked",
      rowIndices: [0],
      matchedRows: 1,
      scannedRows: 5,
      truncated: false,
      elapsedMs: 1,
    });
    const { rerender } = render(
      <ResultFilterHarness
        result={makeResult("res-locked")}
        lockedReason={null}
        onNotify={notify}
      />,
    );
    const filterInputs = screen.getAllByLabelText("Filter result rows");
    fireEvent.change(filterInputs[1], { target: { value: "row0" } });
    await waitFor(() => {
      expect(screen.getAllByTestId("mapped-indices")[1].textContent).toBe("0");
    });
    rerender(
      <ResultFilterHarness
        result={makeResult("res-locked")}
        lockedReason="Review pending edits first"
        onNotify={notify}
      />,
    );
    const sortButtons = screen.getAllByRole("button", { name: "Sort ID" });
    fireEvent.click(sortButtons[1]);
    expect(notify).toHaveBeenCalledWith("Review pending edits first");
    fireEvent.click(screen.getByRole("button", { name: "Reset view" }));
    expect(notify).toHaveBeenCalledTimes(2);
    expect(screen.getAllByTestId("mapped-indices")[1].textContent).toBe("0");
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

    // Switching console session clears pinnedResultIds/Labels and preserves results referenced by another tab
    released.length = 0;
    const shared = makeResult("res-shared-detached", 0, 2);
    const localOnly = makeResult("res-local-only", 1, 2);
    useStudio.setState({
      sessionPool: {
        "conn-1": [
          {
            label: "Console #2",
            primary: false,
            info: {
              id: "sess-pool-2",
              connectionId: "conn-1",
              engine: "postgres",
              serverVersion: "18.0",
              latencyMs: 1,
              encrypted: false,
              consoleMode: false,
              capabilities: {
                sql: true,
                transactions: true,
                explicitTransactionControls: false,
                cancellation: true,
                tableBrowsing: true,
                relationBrowsing: false,
                tableEditing: true,
                tableFiltering: true,
                processes: true,
                objectExplorer: true,
                schemaDiagram: true,
                schemaDiff: true,
                ddlEditing: true,
                snowflakeReviewedDdl: false,
                snowflakeGridEditing: false,
                lockInspection: true,
                explainAnalyze: true,
                oauthBrowser: false,
              },
            },
          },
        ],
      },
    });
    const otherTabId = useStudio.getState().openQuery("conn-1", "SELECT 2;");
    useStudio.getState().updateTab(otherTabId, {
      kind: "result",
      status: "completed",
      results: [shared],
      activeResultId: shared.id,
    });
    useStudio.getState().updateTab(tabId, {
      status: "completed",
      results: [shared, localOnly],
      activeResultId: shared.id,
      pinnedResultIds: [shared.id],
      pinnedResultLabels: { [shared.id]: "Pinned #1" },
    });
    expect(useStudio.getState().attachQuerySession(tabId, "sess-pool-2")).toBe(true);
    expect(released).toEqual([localOnly.id]);
    tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
    expect(tab.results).toEqual([]);
    expect(tab.pinnedResultIds).toBeUndefined();
    expect(tab.pinnedResultLabels).toBeUndefined();
  });
});
