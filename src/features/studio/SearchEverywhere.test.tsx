import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { HistoryEntry, HistoryPage, HistorySummary } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { SearchEverywhere } from "./SearchEverywhere";
import { beginHistoryClear } from "./history-access";

const studio = vi.hoisted(() => ({
  tabs: [],
  activeTabId: null,
  selectedConnectionId: null,
  bootstrap: { profiles: [] },
  sessions: {},
  catalog: {},
  objects: {},
  savedQueries: [],
  shortcuts: {},
  openQuery: vi.fn(),
  notify: vi.fn(),
  openUtility: vi.fn(),
}));
vi.mock("../../state/studio", () => ({
  useStudio: Object.assign(() => studio, { getState: () => studio }),
}));
vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));
vi.mock("./ObjectInspectorView", () => ({ ObjectKindGlyph: () => null }));
beforeEach(() => vi.resetAllMocks());
afterEach(cleanup);

const summary: HistorySummary = {
  id: "one",
  connectionId: "pg",
  connectionName: "Local",
  startedAt: 1,
  sqlPreview: "SELECT bounded_preview",
  sqlStored: true,
  statementCount: 1,
  status: "completed",
  elapsedMs: 2,
};
const page = (entries = [summary]): HistoryPage => ({ entries, nextCursor: null });
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}
function mount() {
  const onClose = vi.fn();
  return {
    ...render(
      <SearchEverywhere
        commands={[]}
        initialScope="history"
        onActivateTab={vi.fn()}
        onClose={onClose}
      />,
    ),
    onClose,
  };
}

test("queries bounded native history for the current query and preserves deep SQL matches", async () => {
  vi.mocked(client.call).mockResolvedValue(page());
  mount();
  fireEvent.change(screen.getByLabelText("Search commands"), {
    target: { value: "deep_tail_token" },
  });
  expect(await screen.findByRole("button", { name: /SELECT bounded_preview/ })).toBeTruthy();
  expect(client.call).toHaveBeenCalledExactlyOnceWith("history_page", {
    request: { search: "deep_tail_token", cursor: null, limit: 80 },
  });
  expect(studio.openQuery).not.toHaveBeenCalled();
});

test("keyboard activation retrieves exact SQL, keeps the dialog until it arrives, and does not execute", async () => {
  const detail = deferred<HistoryEntry | null>();
  vi.mocked(client.call).mockResolvedValueOnce(page()).mockReturnValueOnce(detail.promise);
  const { onClose } = mount();
  await screen.findByRole("button", { name: /SELECT bounded_preview/ });
  fireEvent.keyDown(screen.getByLabelText("Search commands"), { key: "Enter" });
  expect(client.call).toHaveBeenLastCalledWith("history_entry", { id: "one" });
  expect(onClose).not.toHaveBeenCalled();
  const exact = "SELECT /* exact, untruncated SQL */\n  99;";
  await act(async () => detail.resolve({ ...summary, sql: exact }));
  expect(studio.openQuery).toHaveBeenCalledExactlyOnceWith("pg", exact);
  expect(onClose).toHaveBeenCalledTimes(1);
  expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).toEqual([
    "history_page",
    "history_entry",
  ]);
});

test("history search errors remain visible and can be retried", async () => {
  vi.mocked(client.call)
    .mockRejectedValueOnce(new Error("search unavailable"))
    .mockResolvedValueOnce(page());
  mount();
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    expect.stringContaining("search unavailable"),
  );
  fireEvent.click(screen.getByRole("button", { name: "Retry history" }));
  expect(await screen.findByRole("button", { name: /SELECT bounded_preview/ })).toBeTruthy();
});

test.each([null, { ...summary, sql: null }, { ...summary, id: "foreign", sql: "SELECT wrong" }])(
  "missing, metadata-only and mismatched detail never close the dialog or open SQL",
  async (detail) => {
    vi.mocked(client.call).mockResolvedValueOnce(page()).mockResolvedValueOnce(detail);
    const { onClose } = mount();
    fireEvent.click(await screen.findByRole("button", { name: /SELECT bounded_preview/ }));
    await screen.findByRole("alert");
    expect(onClose).not.toHaveBeenCalled();
    expect(studio.openQuery).not.toHaveBeenCalled();
  },
);

test("detail transport failure stays in the open palette", async () => {
  vi.mocked(client.call)
    .mockResolvedValueOnce(page())
    .mockRejectedValueOnce(new Error("detail failed"));
  const { onClose } = mount();
  fireEvent.click(await screen.findByRole("button", { name: /SELECT bounded_preview/ }));
  await screen.findByText(/detail failed/);
  expect(onClose).not.toHaveBeenCalled();
});

test("query changes discard a stale native page", async () => {
  const old = deferred<HistoryPage>();
  vi.mocked(client.call)
    .mockReturnValueOnce(old.promise)
    .mockResolvedValueOnce(page([{ ...summary, id: "new", sqlPreview: "SELECT fresh" }]));
  mount();
  await waitFor(() => expect(client.call).toHaveBeenCalledTimes(1));
  fireEvent.change(screen.getByLabelText("Search commands"), { target: { value: "new_search" } });
  await screen.findByRole("button", { name: /SELECT fresh/ });
  await act(async () => old.resolve(page()));
  expect(screen.queryByRole("button", { name: /SELECT bounded_preview/ })).toBeNull();
});

test.each(["scope", "close", "unmount", "clear"])(
  "%s fences a pending detail response",
  async (action) => {
    const detail = deferred<HistoryEntry | null>();
    vi.mocked(client.call)
      .mockResolvedValueOnce(page())
      .mockReturnValueOnce(detail.promise)
      .mockResolvedValue(page([]));
    const view = mount();
    fireEvent.click(await screen.findByRole("button", { name: /SELECT bounded_preview/ }));
    let finish: (() => void) | undefined;
    if (action === "scope") fireEvent.click(screen.getByRole("tab", { name: "Actions" }));
    if (action === "close") fireEvent.click(screen.getByRole("button", { name: "Close dialog" }));
    if (action === "unmount") view.unmount();
    if (action === "clear")
      act(() => {
        finish = beginHistoryClear();
      });
    await act(async () => detail.resolve({ ...summary, sql: "SELECT stale" }));
    if (finish) act(() => finish?.());
    expect(studio.openQuery).not.toHaveBeenCalled();
  },
);

test("a bounded palette page offers full history without fetching SQL or executing", async () => {
  vi.mocked(client.call).mockResolvedValue({ ...page(), nextCursor: { startedAt: 1, id: "one" } });
  const { onClose } = mount();
  await screen.findByText(/More matching history is available/);
  fireEvent.click(screen.getByRole("button", { name: "Open query history" }));
  expect(studio.openUtility).toHaveBeenCalledExactlyOnceWith("history");
  expect(onClose).toHaveBeenCalledTimes(1);
  expect(client.call).toHaveBeenCalledTimes(1);
  expect(studio.openQuery).not.toHaveBeenCalled();
});

test("rejects over-budget UTF-8 search locally while retaining the palette", async () => {
  mount();
  fireEvent.change(screen.getByLabelText("Search commands"), {
    target: { value: "雪".repeat(171) },
  });
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    expect.stringContaining("512 UTF-8 bytes"),
  );
  expect(client.call).not.toHaveBeenCalled();
});
