import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import type { ResultInfo } from "../../ipc/generated";
import { useStudio, type StudioTab } from "../../state/studio";
import DatabaseGrid from "./DatabaseGrid";

const mocks = vi.hoisted(() => ({
  listeners: new Set<() => void>(),
  cached: null as { value: string | null; large: boolean } | null,
  selectedRow: 0,
  studioState: {
    density: "compact",
    sessions: {} as Record<string, { id: string }>,
    bootstrap: { profiles: [] as { id: string; safeMode: string }[] },
    tableLayouts: [] as unknown[],
  },
}));

vi.mock("../../state/studio", () => ({
  useStudio: Object.assign(
    (selector: (state: typeof mocks.studioState) => unknown) => selector(mocks.studioState),
    {
      getState: () => mocks.studioState,
      setState: (patch: Partial<typeof mocks.studioState>) => {
        mocks.studioState = { ...mocks.studioState, ...patch };
      },
    },
  ),
  querySessionForTab: (
    tab: { connectionId: string | null },
    sessions: typeof mocks.studioState.sessions,
  ) => (tab.connectionId ? sessions[tab.connectionId] : undefined),
  isSnowflakeConsoleSession: () => false,
}));

vi.mock("../../data/result-cache", () => ({
  resultCache: {
    read: () => mocks.cached,
    ensure: vi.fn(async () => {}),
    error: () => undefined,
    subscribe: (listener: () => void) => {
      mocks.listeners.add(listener);
      return () => mocks.listeners.delete(listener);
    },
  },
}));

vi.mock("@glideapps/glide-data-grid", async () => {
  const React = await import("react");
  return {
    default: React.forwardRef((props: any, ref) => {
      React.useImperativeHandle(ref, () => ({ updateCells: vi.fn(), scrollTo: vi.fn() }));
      props.getCellContent([0, 0]);
      return (
        <button
          data-testid="mock-grid"
          onClick={() =>
            props.onGridSelectionChange({
              rows: { toArray: () => [] },
              columns: { toArray: () => [] },
              current: {
                cell: [0, mocks.selectedRow],
                range: { x: 0, y: mocks.selectedRow, width: 1, height: 1 },
                rangeStack: [],
              },
            })
          }
        />
      );
    }),
    GridCellKind: { Loading: 0, Text: 1, Boolean: 2 },
    CompactSelection: { empty: () => ({ toArray: () => [], length: 0 }) },
  };
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  mocks.listeners.clear();
  mocks.cached = null;
  mocks.selectedRow = 0;
  mocks.studioState.sessions = {};
  mocks.studioState.bootstrap.profiles = [];
});

test("a selected cell updates the inspector when its result page finishes loading", () => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  const session = { id: "session-1", connectionId: "connection-1" };
  useStudio.setState({
    density: "compact",
    sessions: { "connection-1": session } as any,
    bootstrap: { profiles: [{ id: "connection-1", safeMode: "normal" }] } as any,
    tableLayouts: [],
  });
  const column = {
    id: "column-1",
    name: "status",
    databaseType: "text",
    kind: "text",
    nullable: true,
    primaryKey: false,
    foreignKey: false,
    editable: true,
    insertable: true,
    hasDefault: false,
  } as any;
  const result = {
    id: "result-1",
    retainedRows: 2,
    columns: [column],
    editability: { status: "editable" },
  } as ResultInfo;
  const tab = {
    id: "tab-1",
    kind: "query",
    connectionId: "connection-1",
    dataSessionId: "session-1",
    querySession: null,
    querySessionClosed: false,
    status: "completed",
    revision: 1,
    results: [result],
    activeResultId: result.id,
    pages: [],
    loadedRows: 0,
    hasMore: false,
    table: null,
  } as unknown as StudioTab;
  const onSelect = vi.fn();

  render(
    <DatabaseGrid
      tab={tab}
      result={result}
      theme="dark"
      onSelect={onSelect}
      onOpenValue={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByTestId("mock-grid"));
  expect(onSelect).toHaveBeenLastCalledWith(null);
  mocks.selectedRow = 1;
  fireEvent.click(screen.getByTestId("mock-grid"));
  expect(onSelect).toHaveBeenLastCalledWith(null);

  mocks.cached = { value: "active", large: false };
  act(() => mocks.listeners.forEach((listener) => listener()));

  expect(onSelect).toHaveBeenLastCalledWith(
    expect.objectContaining({
      resultId: result.id,
      row: 1,
      column: expect.objectContaining({ id: column.id }),
      value: "active",
    }),
  );
  const callCount = onSelect.mock.calls.length;
  act(() => mocks.listeners.forEach((listener) => listener()));
  expect(onSelect).toHaveBeenCalledTimes(callCount);
});
