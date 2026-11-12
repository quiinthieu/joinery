import { createRef } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { Workbench } from "./Workbench";
import { client } from "../ipc/client";
import { useStudio } from "../state/studio";
import {
  clientId,
  plan,
  receiptView,
  runId,
  session,
} from "../features/studio/SnowflakeConsole.test-fixtures";
import type { ConnectionProfile } from "../ipc/generated";
vi.mock("../features/studio/SqlEditor", () => ({ SqlEditor: () => <div>Static SQL editor</div> }));
vi.mock("../features/grid/DatabaseGrid", () => ({
  default: () => <div>Retained result grid</div>,
}));
vi.mock("../ipc/client", () => ({
  client: { call: vi.fn().mockResolvedValue(null) },
  messageOf: String,
}));
const initial = { ...useStudio.getState() };
beforeEach(() =>
  useStudio.setState(
    { ...initial, tabs: [], sessions: {}, consoleSessions: {}, snowflakeConsoleReceipts: {} },
    true,
  ),
);
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  useStudio.setState(initial, true);
});
function props() {
  useStudio.getState().setSession(session);
  const id = useStudio.getState().openQuery("profile", "SELECT 1; COMMIT");
  return {
    tab: useStudio.getState().tabs.find((t) => t.id === id)!,
    session,
    profile: {
      id: "profile",
      name: "Console",
      engine: "snowflake",
      safeMode: "confirm",
    } as ConnectionProfile,
    editorRef: createRef(),
    gridRef: createRef(),
    onRun: vi.fn(),
    onRunConsoleScript: vi.fn(),
    onRunConsoleSelected: vi.fn(),
    onExplain: vi.fn(),
    onAnalyze: vi.fn(),
    onOpenParameters: vi.fn(),
    onReview: vi.fn(),
    onTransactionControl: vi.fn(),
    onConfirm: vi.fn(),
    onSelect: vi.fn(),
    onOpenValue: vi.fn(),
    onOpenColumns: vi.fn(),
    onOpenExport: vi.fn(),
    onOpenImport: vi.fn(),
    onOpenCompare: vi.fn(),
    onNewSession: vi.fn(),
    onOpenSqlFile: vi.fn(),
    onSaveSqlFile: vi.fn(),
    onSaveToLibrary: vi.fn(),
    onToggleBookmark: vi.fn(),
    onQueryChange: vi.fn(),
  } as Parameters<typeof Workbench>[0];
}
test("console workbench exposes reviewed run/selection and suppresses generated actions without hiding the editor", async () => {
  const p = props();
  render(<Workbench {...p} />);
  await waitFor(() => expect(screen.getByText("Static SQL editor")).toBeTruthy());
  expect(screen.queryByRole("button", { name: /Browse relations/ })).toBeNull();
  expect(screen.queryByRole("button", { name: "Explain" })).toBeNull();
  expect(screen.queryByLabelText("Transaction mode")).toBeNull();
  expect(screen.queryByRole("button", { name: "Commit transaction" })).toBeNull();
  expect(
    screen.getByText(/Each review shows the execution and result-delivery limit/),
  ).toBeTruthy();
  expect(screen.getByText(/Safe outcome recording and settlement may take longer/)).toBeTruthy();
  expect(screen.queryByText(/fixed 65-second/)).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /Review and run/ }));
  expect(p.onRun).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole("button", { name: "Review selected server unit" }));
  expect(p.onRunConsoleSelected).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole("button", { name: "Run all statements" }));
  expect(p.onRunConsoleScript).toHaveBeenCalledOnce();
});
test("retained recording hold in another tab disables execution and cancels by stable receipt even before run ID", () => {
  const p = props();
  const cancel = vi.fn().mockResolvedValue(undefined);
  useStudio.setState({
    cancelSnowflakeConsole: cancel,
    snowflakeConsoleReceipts: {
      [clientId]: {
        clientRequestId: clientId,
        sessionId: session.id,
        lookupPending: false,
        view: receiptView({ pendingRecording: true }),
        tabId: "closed-tab",
      },
    },
  });
  render(<Workbench {...p} />);
  expect(
    (screen.getByRole("button", { name: "Run all statements" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("button", { name: "Review selected server unit" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: /Cancel/ }));
  expect(cancel).toHaveBeenCalledWith(clientId);
});
test("old metadata tab cannot mount SQL observers on a console connection", () => {
  const p = props();
  p.tab = {
    ...p.tab,
    kind: "object",
    object: { schema: "PUBLIC", name: "OLD", kind: "table", identity: null },
  };
  render(<Workbench {...p} />);
  expect(screen.getByRole("status").textContent).toContain("Metadata is disabled");
});

test("changing connection clears the tab without releasing receipt-owned native rows", async () => {
  const p = props();
  const result = {
    id: "receipt-result",
    runId,
    statementIndex: 0,
    columns: [],
    retainedRows: 1,
    status: "complete" as const,
    table: null,
  };
  p.tab = { ...p.tab, results: [result], activeResultId: result.id };
  useStudio.setState({
    snowflakeConsoleReceipts: {
      [clientId]: {
        clientRequestId: clientId,
        runId,
        view: receiptView({ results: [result] }),
        lookupPending: false,
      },
    },
  });
  render(<Workbench {...p} />);
  fireEvent.change(screen.getByRole("combobox", { name: "Query connection" }), {
    target: { value: "" },
  });
  await waitFor(() => expect(useStudio.getState().tabs[0].connectionId).toBeNull());
  expect(client.call).not.toHaveBeenCalledWith("release_result", { resultId: result.id });
  expect(useStudio.getState().snowflakeConsoleReceipts[clientId].view?.results).toEqual([result]);
});
