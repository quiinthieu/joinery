import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import type { HistorySummary } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { HistoryView } from "./UtilityViews";
import { beginHistoryClear } from "./history-access";
const studio = vi.hoisted(() => ({ openQuery: vi.fn(), notify: vi.fn() }));
vi.mock("../../state/studio", () => ({
  useStudio: Object.assign(vi.fn(), { getState: () => studio }),
}));

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));

afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

test("shows DML acknowledgement and exact counts in durable history without SQL text", async () => {
  const entry: HistorySummary = {
    provenance: "recorded",
    id: "dml-history",
    connectionId: "snowflake-profile",
    connectionName: "Warehouse",
    startedAt: 1,
    sqlPreview: null,
    sqlStored: false,
    statementCount: 1,
    status: "completed",
    elapsedMs: 142,
    snowflakeDml: {
      kind: "update",
      statementOutcome: "succeeded",
      summary: {
        status: "validated",
        affectedRows: "900719925474099312345",
        multiJoinedRows: "900719925474099312346",
      },
      retired: false,
    },
  };
  vi.mocked(client.call).mockResolvedValue({ entries: [entry], nextCursor: null });
  render(<HistoryView />);

  expect(await screen.findByText("Snowflake UPDATE · statement succeeded")).toBeTruthy();
  expect(screen.getByText("Affected rows: 900719925474099312345")).toBeTruthy();
  expect(screen.getByText("Multi-joined rows updated: 900719925474099312346")).toBeTruthy();
  expect(
    screen.getByText("Statement acknowledgement does not prove committed effects."),
  ).toBeTruthy();
  expect(screen.getByText("SQL text not stored")).toBeTruthy();
  expect(screen.getByText(/Statement counts unknown/)).toBeTruthy();
  expect(screen.queryByText(/UPDATE .* SET/i)).toBeNull();
});

test("history distinguishes stopped suffixes, complete plans and legacy unknown counts with SQL storage unchanged", async () => {
  const base: HistorySummary = {
    provenance: "recorded",
    id: "stopped",
    connectionId: "fixture",
    connectionName: "Stopped fixture",
    startedAt: 1,
    sqlPreview: "SELECT limited; SELECT skipped; UPDATE skipped;",
    sqlStored: true,
    statementCount: 999,
    status: "stopped",
    elapsedMs: 12,
    executionSummary: {
      planned: 3,
      attempted: 1,
      completed: 1,
      notAttempted: 2,
      stopReason: "resultLimit",
    },
  };
  vi.mocked(client.call).mockResolvedValue({
    entries: [
      base,
      {
        ...base,
        id: "complete",
        connectionName: "Complete fixture",
        sqlPreview: null,
        sqlStored: false,
        status: "completed",
        executionSummary: {
          planned: 2,
          attempted: 2,
          completed: 2,
          notAttempted: 0,
          stopReason: null,
        },
      },
      {
        ...base,
        id: "legacy",
        connectionName: "Legacy fixture",
        sqlPreview: null,
        sqlStored: false,
        status: "completed",
        executionSummary: undefined,
      },
    ],
    nextCursor: null,
  });
  render(<HistoryView />);
  const stopped = await screen.findByRole("button", { name: /Stopped fixture/ });
  expect(within(stopped).getByRole("status").textContent).toContain(
    "Stopped · Planned: 3 · Attempted: 1 · Completed: 1 · Not attempted: 2",
  );
  expect(stopped.textContent).toContain("Statements 2–3 were not attempted");
  expect(stopped.textContent).toContain(base.sqlPreview);
  expect(stopped.querySelector(".status-dot.stopped")).toBeTruthy();
  const completed = screen.getByRole("button", { name: /Complete fixture/ });
  expect(completed.textContent).toContain(
    "Completed · Planned: 2 · Attempted: 2 · Completed: 2 · Not attempted: 0",
  );
  expect(completed.textContent).toContain("SQL text not stored");
  const legacy = screen.getByRole("button", { name: /Legacy fixture/ });
  expect(legacy.textContent).toContain("Statement counts unknown");
  expect(legacy.textContent).not.toMatch(/Planned:|Attempted:|Completed:/);
  expect(screen.queryByText(/999/)).toBeNull();
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
const summary: HistorySummary = {
  provenance: "recorded",
  id: "one",
  connectionId: "pg",
  connectionName: "Page one",
  startedAt: 1,
  sqlPreview: "SELECT /* bounded preview */",
  sqlStored: true,
  statementCount: 1,
  status: "completed",
  elapsedMs: 1,
};
const page = (entries: HistorySummary[]) => ({ entries, nextCursor: null });

test("pages replace rows rather than accumulate and previous restores the opaque cursor", async () => {
  const cursor = { startedAt: 1, id: "one" };
  vi.mocked(client.call)
    .mockResolvedValueOnce({ entries: [summary], nextCursor: cursor })
    .mockResolvedValueOnce(page([{ ...summary, id: "two", connectionName: "Page two" }]))
    .mockResolvedValueOnce(page([summary]));
  render(<HistoryView />);
  await screen.findByRole("button", { name: /Page one/ });
  fireEvent.click(screen.getByRole("button", { name: "Next history page" }));
  await screen.findByRole("button", { name: /Page two/ });
  expect(screen.queryByRole("button", { name: /Page one/ })).toBeNull();
  expect(client.call).toHaveBeenNthCalledWith(2, "history_page", {
    request: { search: "", cursor, limit: 50 },
  });
  fireEvent.click(screen.getByRole("button", { name: "Previous history page" }));
  await screen.findByRole("button", { name: /Page one/ });
  expect(client.call).toHaveBeenNthCalledWith(3, "history_page", {
    request: { search: "", cursor: null, limit: 50 },
  });
});

test("a native deep SQL match remains visible and explicit activation fetches exact current SQL only", async () => {
  const full = "SELECT /* longer than preview */\n  42;";
  vi.mocked(client.call).mockImplementation(async (command) =>
    command === "history_entry" ? { ...summary, sql: full } : page([summary]),
  );
  render(<HistoryView />);
  fireEvent.change(screen.getByLabelText("Search history"), {
    target: { value: "deep_token_not_in_preview" },
  });
  const row = await screen.findByRole("button", { name: /Page one/ });
  expect(studio.openQuery).not.toHaveBeenCalled();
  fireEvent.click(row);
  await waitFor(() => expect(studio.openQuery).toHaveBeenCalledWith("pg", full));
  expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).toEqual([
    "history_page",
    "history_entry",
  ]);
});

test("refresh fences an earlier page response", async () => {
  const old = deferred<ReturnType<typeof page>>();
  vi.mocked(client.call)
    .mockReturnValueOnce(old.promise)
    .mockResolvedValueOnce(page([{ ...summary, connectionName: "Fresh page" }]));
  render(<HistoryView />);
  await waitFor(() => expect(client.call).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await screen.findByRole("button", { name: /Fresh page/ });
  await act(async () => old.resolve(page([summary])));
  expect(screen.queryByRole("button", { name: /Page one/ })).toBeNull();
});

test("clear fences an old page and detail, and never reopens SQL after clearing", async () => {
  const detail = deferred<unknown>();
  const oldPage = deferred<ReturnType<typeof page>>();
  vi.mocked(client.call)
    .mockResolvedValueOnce(page([summary]))
    .mockReturnValueOnce(detail.promise)
    .mockReturnValueOnce(oldPage.promise)
    .mockResolvedValueOnce(undefined)
    .mockResolvedValue(page([]));
  render(<HistoryView />);
  fireEvent.click(await screen.findByRole("button", { name: /Page one/ }));
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => expect(client.call).toHaveBeenCalledTimes(3));
  fireEvent.click(screen.getByRole("button", { name: "Clear history" }));
  fireEvent.click(screen.getByRole("button", { name: "Clear local history" }));
  await waitFor(() => expect(client.call).toHaveBeenCalledWith("clear_history"));
  await act(async () => {
    detail.resolve({ ...summary, sql: "SELECT stale" });
    oldPage.resolve(page([summary]));
  });
  await screen.findByText("No queries here yet");
  expect(studio.openQuery).not.toHaveBeenCalled();
  expect(screen.queryByRole("button", { name: /Page one/ })).toBeNull();
});

test.each([null, { ...summary, sql: null }])(
  "handles missing or metadata-only current detail without opening a tab",
  async (detail) => {
    vi.mocked(client.call)
      .mockResolvedValueOnce(page([summary]))
      .mockResolvedValueOnce(detail);
    render(<HistoryView />);
    fireEvent.click(await screen.findByRole("button", { name: /Page one/ }));
    if (detail)
      await waitFor(() =>
        expect(studio.notify).toHaveBeenCalledWith(
          "Only execution metadata was saved for this query.",
        ),
      );
    else await screen.findByRole("alert");
    expect(studio.openQuery).not.toHaveBeenCalled();
  },
);

test("detail errors are visible, identity changes rejected, and unmount fences pending opens", async () => {
  const late = deferred<unknown>();
  vi.mocked(client.call)
    .mockResolvedValueOnce(page([summary]))
    .mockRejectedValueOnce(new Error("detail unavailable"))
    .mockResolvedValueOnce({ ...summary, connectionId: "foreign", sql: "SELECT wrong" })
    .mockReturnValueOnce(late.promise);
  const view = render(<HistoryView />);
  const row = await screen.findByRole("button", { name: /Page one/ });
  fireEvent.click(row);
  await screen.findByText(/detail unavailable/);
  await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(row);
  await screen.findByText(/identity changed/);
  await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(row);
  view.unmount();
  await act(async () => late.resolve({ ...summary, sql: "SELECT late" }));
  expect(studio.openQuery).not.toHaveBeenCalled();
});

test("a clear from another history surface invalidates this pending detail", async () => {
  const late = deferred<unknown>();
  vi.mocked(client.call)
    .mockResolvedValueOnce(page([summary]))
    .mockReturnValueOnce(late.promise)
    .mockResolvedValue(page([]));
  render(<HistoryView />);
  fireEvent.click(await screen.findByRole("button", { name: /Page one/ }));
  let finish!: () => void;
  act(() => {
    finish = beginHistoryClear();
  });
  await act(async () => late.resolve({ ...summary, sql: "SELECT stale" }));
  act(() => finish());
  expect(studio.openQuery).not.toHaveBeenCalled();
});

test("stored empty SQL is reopened exactly rather than treated as missing text", async () => {
  vi.mocked(client.call)
    .mockResolvedValueOnce(page([{ ...summary, sqlPreview: "" }]))
    .mockResolvedValueOnce({ ...summary, sql: "" });
  render(<HistoryView />);
  fireEvent.click(await screen.findByRole("button", { name: /Empty SQL text/ }));
  await waitFor(() => expect(studio.openQuery).toHaveBeenCalledWith("pg", ""));
  expect(studio.notify).not.toHaveBeenCalled();
});

test("history renders admitted and recovered records honestly while preserving original terminals", async () => {
  const entries: HistorySummary[] = [
    {
      ...summary,
      id: "admitted",
      connectionName: "Admitted fixture",
      provenance: "admitted",
      status: null,
      elapsedMs: null,
    },
    {
      ...summary,
      id: "interrupted",
      connectionName: "Interrupted fixture",
      provenance: "interrupted",
      status: null,
      elapsedMs: null,
    },
    {
      ...summary,
      id: "restored",
      connectionName: "Restored fixture",
      provenance: "restoreRecovered",
      status: null,
      elapsedMs: null,
    },
    {
      ...summary,
      id: "known",
      connectionName: "Known fixture",
      provenance: "interrupted",
      status: "completed",
      elapsedMs: 17,
    },
    {
      ...summary,
      id: "duration",
      connectionName: "Duration fixture",
      provenance: "interrupted",
      status: "failed",
      elapsedMs: null,
    },
  ];
  vi.mocked(client.call).mockResolvedValue(page(entries));
  render(<HistoryView />);
  const admitted = await screen.findByRole("button", { name: /Admitted fixture/ });
  expect(admitted.textContent).toContain("Admitted; completion pending");
  expect(admitted.textContent).toContain("Admission does not prove dispatch or server submission");
  expect(
    admitted.querySelector(".status-dot.completed,.status-dot.failed,.status-dot.cancelled"),
  ).toBeNull();
  for (const name of ["Admitted", "Interrupted", "Restored", "Duration"]) {
    const row = screen.getByRole("button", { name: new RegExp(`${name} fixture`) });
    expect(row.textContent).toContain("Duration unavailable");
    expect(row.textContent).not.toMatch(/\b0 ms\b/);
  }
  expect(screen.getByRole("button", { name: /Interrupted fixture/ }).textContent).toContain(
    "Recovered after interruption · Completion unknown",
  );
  expect(screen.getByRole("button", { name: /Restored fixture/ }).textContent).toContain(
    "Recovered from metadata restore · Completion unknown",
  );
  const known = screen.getByRole("button", { name: /Known fixture/ });
  expect(known.textContent).toContain("Completed · Statement counts unknown");
  expect(known.textContent).toContain("17 ms");
  expect(known.textContent).toContain("Original recorded completion is preserved");
  expect(screen.getByRole("button", { name: /Duration fixture/ }).textContent).toContain(
    "Failed · Statement counts unknown",
  );
});

test("explicitly reopening admitted saved SQL opens an exact new draft without execution", async () => {
  const admitted: HistorySummary = {
    ...summary,
    connectionName: "Admitted reopen fixture",
    provenance: "admitted",
    status: null,
    elapsedMs: null,
  };
  const sql = "SELECT 'pending original';\n/* exact admitted text */";
  vi.mocked(client.call)
    .mockResolvedValueOnce(page([admitted]))
    .mockResolvedValueOnce({ ...admitted, sql });
  render(<HistoryView />);
  const row = await screen.findByRole("button", { name: /Admitted reopen fixture/ });
  expect(within(row).getByText("Admitted; completion pending")).toBeTruthy();
  fireEvent.click(row);
  await waitFor(() => expect(studio.openQuery).toHaveBeenCalledExactlyOnceWith("pg", sql));
  expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).toEqual([
    "history_page",
    "history_entry",
  ]);
});
