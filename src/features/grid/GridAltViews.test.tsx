import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ColumnInfo, ResultInfo, SavedTableLayout } from "../../ipc/generated";
import { dirtyStore } from "../../data/dirty-store";
import { resultCache } from "../../data/result-cache";
import { useStudio } from "../../state/studio";
import { RecordCardView, TransposeGridView } from "./GridAltViews";
import { TableColumnsDialog } from "./TableColumnsDialog";

const COLUMNS: ColumnInfo[] = [
  {
    id: "c0",
    name: "id",
    databaseType: "bigint",
    kind: "integer",
    nullable: false,
    primaryKey: true,
    foreignKey: false,
    editable: false,
    insertable: true,
    hasDefault: true,
  },
  {
    id: "c1",
    name: "customer_email",
    databaseType: "text",
    kind: "text",
    nullable: false,
    primaryKey: false,
    foreignKey: false,
    editable: true,
    insertable: true,
    hasDefault: false,
  },
  {
    id: "c2",
    name: "notes",
    databaseType: "text",
    kind: "text",
    nullable: true,
    primaryKey: false,
    foreignKey: false,
    editable: true,
    insertable: true,
    hasDefault: false,
  },
];

const SAMPLE_RESULT: ResultInfo = {
  id: "res-2026-3",
  runId: "run-1",
  statementIndex: 0,
  columns: COLUMNS,
  retainedRows: 2,
  status: "complete",
  table: { schema: "public", name: "orders" },
  editability: {
    status: "editable",
    table: { schema: "public", name: "orders" },
    primaryKey: ["id"],
  },
};

describe("DataGrip 2026.3 grid views, column pinning, and detached result tabs", () => {
  beforeEach(() => {
    dirtyStore.clear();
    vi.spyOn(resultCache, "ensure").mockResolvedValue();
    vi.spyOn(resultCache, "read").mockImplementation((_id, colId, row) => {
      if (colId === "c0") return { value: row === 0 ? "101" : "102", large: false };
      if (colId === "c1")
        return { value: row === 0 ? "alice@example.test" : "bob@example.test", large: false };
      return { value: row === 0 ? null : "", large: false };
    });
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("searches, pins, hides optional columns, and fits widths in TableColumnsDialog", () => {
    let latest = null as SavedTableLayout | null;
    const { rerender } = render(
      <TableColumnsDialog
        connectionId="11111111-1111-4111-8111-111111111111"
        table={{ schema: "public", name: "orders" }}
        columns={COLUMNS}
        layout={latest}
        onChange={(next) => {
          latest = next;
        }}
        onClose={() => {}}
      />,
    );

    fireEvent.change(screen.getByLabelText("Search columns"), { target: { value: "email" } });
    expect(screen.getByText("customer_email")).toBeTruthy();
    expect(screen.queryByText("notes")).toBeNull();

    fireEvent.click(screen.getByLabelText("Pin customer_email"));
    expect(latest?.pinnedColumns).toEqual(["customer_email"]);

    rerender(
      <TableColumnsDialog
        connectionId="11111111-1111-4111-8111-111111111111"
        table={{ schema: "public", name: "orders" }}
        columns={COLUMNS}
        layout={latest}
        onChange={(next) => {
          latest = next;
        }}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText("Pinned")).toBeTruthy();

    fireEvent.click(screen.getByText("Hide optional"));
    expect(latest?.hiddenColumns).toEqual(["notes"]);

    fireEvent.click(screen.getByLabelText("Fit columns to window"));
    expect(Object.keys(latest?.columnWidths ?? {}).length).toBeGreaterThan(0);
  });

  it("renders TransposeGridView and RecordCardView with NULL vs empty distinction and selective revert", () => {
    const connId = "11111111-1111-4111-8111-111111111111";
    useStudio.setState({
      sessions: {
        [connId]: {
          id: "sess-1",
          connectionId: connId,
          engine: "postgres",
          serverVersion: "18.0",
          latencyMs: 1,
          encrypted: false,
          capabilities: {
            sql: true,
            transactions: true,
            cancellation: true,
            tableBrowsing: true,
            tableEditing: true,
            tableFiltering: true,
            processes: true,
          },
        },
      },
    });
    const tabId = useStudio.getState().openQuery(connId, "SELECT * FROM public.orders;");
    useStudio.getState().updateTab(tabId, {
      results: [SAMPLE_RESULT],
      activeResultId: SAMPLE_RESULT.id,
      status: "completed",
      dataSessionId: "sess-1",
    });
    const tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

    const selected: string[] = [];
    const opened: string[] = [];
    const { unmount } = render(
      <TransposeGridView
        tab={tab}
        result={SAMPLE_RESULT}
        visibleColumns={COLUMNS}
        onSelect={(target) => {
          if (target) selected.push(`${target.column.name}:${target.row}`);
        }}
        onOpenValue={(target) => opened.push(`${target.column.name}:${target.value}`)}
      />,
    );

    expect(screen.getByText("alice@example.test")).toBeTruthy();
    expect(screen.getByText("NULL")).toBeTruthy();
    expect(screen.getByText("empty")).toBeTruthy();
    fireEvent.doubleClick(screen.getByText("alice@example.test"));
    expect(opened).toEqual(["customer_email:alice@example.test"]);
    unmount();

    dirtyStore.set({
      resultId: SAMPLE_RESULT.id,
      row: 0,
      column: "c1",
      original: "alice@example.test",
      value: "alicia@example.test",
    });

    render(
      <RecordCardView
        tab={tab}
        result={SAMPLE_RESULT}
        visibleColumns={COLUMNS}
        onSelect={() => {}}
        onOpenValue={() => {}}
      />,
    );
    expect(screen.getByText("alicia@example.test")).toBeTruthy();
    fireEvent.click(screen.getByLabelText("Revert customer_email"));
    expect(dirtyStore.get(SAMPLE_RESULT.id, 0, "c1")).toBeUndefined();
    expect(screen.getByText("alice@example.test")).toBeTruthy();

    fireEvent.click(screen.getByLabelText("Next record"));
    expect(screen.getByText("bob@example.test")).toBeTruthy();
    expect(screen.getByText("empty string")).toBeTruthy();
  });

  it("detaches a query result into a dedicated read-only result tab and reopens originating SQL", () => {
    const connId = "11111111-1111-4111-8111-111111111111";
    useStudio.setState({ tabs: [], activeTabId: null });
    const queryTabId = useStudio
      .getState()
      .openQuery(connId, "SELECT id, customer_email FROM public.orders;");
    useStudio.getState().updateTab(queryTabId, {
      results: [SAMPLE_RESULT],
      activeResultId: SAMPLE_RESULT.id,
      status: "completed",
    });
    useStudio.getState().setResultLayout(queryTabId, SAMPLE_RESULT.id, {
      connectionId: connId,
      table: { schema: "_result", name: SAMPLE_RESULT.id },
      columnOrder: [],
      hiddenColumns: [],
      pinnedColumns: ["customer_email"],
      columnWidths: {},
    });

    const resultTabId = useStudio.getState().detachResultToTab(queryTabId, SAMPLE_RESULT.id);
    expect(resultTabId).toBeTruthy();

    const afterDetach = useStudio.getState();
    const sourceAfter = afterDetach.tabs.find((t) => t.id === queryTabId)!;
    const resultTab = afterDetach.tabs.find((t) => t.id === resultTabId)!;
    expect(sourceAfter.results).toHaveLength(0);
    expect(resultTab.kind).toBe("result");
    expect(resultTab.results[0].id).toBe(SAMPLE_RESULT.id);
    expect(resultTab.results[0].editability?.status).toBe("readOnly");
    expect(resultTab.resultLayouts?.[SAMPLE_RESULT.id]?.pinnedColumns).toEqual(["customer_email"]);

    // While source SQL is unchanged, openOriginatingSql focuses the existing query tab.
    expect(useStudio.getState().openOriginatingSql(resultTabId!)).toBe(queryTabId);
    expect(useStudio.getState().activeTabId).toBe(queryTabId);

    // If source SQL was edited for another query, openOriginatingSql opens a fresh tab with the exact original SQL.
    useStudio.getState().updateTab(queryTabId, { sql: "SELECT 1;" });
    const freshQueryId = useStudio.getState().openOriginatingSql(resultTabId!);
    expect(freshQueryId).not.toBe(queryTabId);
    expect(useStudio.getState().tabs.find((t) => t.id === freshQueryId)?.sql).toBe(
      "SELECT id, customer_email FROM public.orders;",
    );
  });
});
