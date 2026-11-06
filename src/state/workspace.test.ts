import { afterEach, expect, test, vi } from "vitest";
import { WorkspaceWriter, workspaceSnapshot } from "./workspace";
import { useStudio } from "./studio";
import type { SavedWorkspace } from "../ipc/generated";

const snapshot = (theme: string): SavedWorkspace => ({
  theme,
  density: "compact",
  sidebarVisible: true,
  sidebarCollapsed: false,
  inspectorVisible: false,
  activeDraftId: null,
  drafts: [],
  tableLayouts: [],
  shortcuts: {},
});
afterEach(() => vi.useRealTimers());

test("draft saves are serialized, coalesce edits, and flush before shutdown", async () => {
  vi.useFakeTimers();
  let release!: () => void;
  const first = new Promise<void>((resolve) => {
    release = resolve;
  });
  const save = vi
    .fn()
    .mockImplementationOnce(() => first)
    .mockResolvedValue(undefined);
  const writer = new WorkspaceWriter(save, vi.fn());
  writer.schedule(snapshot("dark"));
  const flushing = writer.flush();
  writer.schedule(snapshot("light"));
  writer.schedule(snapshot("contrast"));
  expect(save).toHaveBeenCalledTimes(1);
  release();
  await flushing;
  expect(save.mock.calls.map(([workspace]) => workspace.theme)).toEqual(["dark", "contrast"]);
  await vi.runAllTimersAsync();
  expect(save).toHaveBeenCalledTimes(2);
});

test("a failed save remains available for an explicit flush retry", async () => {
  vi.useFakeTimers();
  const save = vi
    .fn()
    .mockRejectedValueOnce(new Error("fixture failure"))
    .mockResolvedValue(undefined);
  const writer = new WorkspaceWriter(save, vi.fn());
  writer.schedule(snapshot("contrast"));
  await expect(writer.flush()).rejects.toThrow("fixture failure");
  await writer.flush();
  expect(save).toHaveBeenCalledTimes(2);
});

test("workspaceSnapshot includes pinned draft state, sidebarCollapsed, and custom shortcuts", () => {
  useStudio.setState({
    bootstrap: {
      profiles: [
        {
          id: "conn-1",
          name: "Local",
          groupId: "local",
          engine: "postgres",
          host: "127.0.0.1",
          port: 5432,
          database: "joinery",
          username: "joinery",
          environment: "local",
          safeMode: "silent",
          tlsMode: "disable",
          caCertPath: null,
          ssh: null,
          rememberPassword: false,
          storeQueryText: true,
          storeDrafts: true,
          snowflake: null,
        },
      ],
      groups: [],
      workspace: null,
      passphraseConfigured: false,
      supportedEngines: ["postgres", "mysql", "redis", "snowflake"],
      resultBudgetBytes: 64 * 1024 * 1024,
      rowLimit: 500,
      savedQueries: [],
      bookmarks: [],
      sqlFormatSettings: {
        keywordCase: "upper",
        indentWidth: 2,
        linesBetweenQueries: 1,
      },
      sqlSnippets: [],
      scratchpads: [],
    },
    tabs: [],
    activeTabId: null,
    sidebarVisible: false,
    sidebarCollapsed: true,
    inspectorVisible: true,
    shortcuts: { openPalette: "Mod+J" },
  });
  const tabId = useStudio.getState().openQuery("conn-1", "SELECT 42;");
  useStudio.getState().togglePinTab(tabId);

  const snap = workspaceSnapshot();
  expect(snap.sidebarVisible).toBe(false);
  expect(snap.sidebarCollapsed).toBe(true);
  expect(snap.inspectorVisible).toBe(true);
  expect(snap.shortcuts).toEqual({ openPalette: "Mod+J" });
  expect(snap.drafts).toEqual([
    {
      id: tabId,
      connectionId: "conn-1",
      title: "Query 1",
      sql: "SELECT 42;",
      pinned: true,
    },
  ]);
});
