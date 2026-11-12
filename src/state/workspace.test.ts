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

test("recovery pause drains the current save and prevents queued or page-hide flushes", async () => {
  vi.useFakeTimers();
  let release!: () => void;
  const save = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          release = resolve;
        }),
    )
    .mockResolvedValue(undefined);
  const writer = new WorkspaceWriter(save, vi.fn());
  writer.schedule(snapshot("old-inflight"));
  const flushing = writer.flush();
  writer.schedule(snapshot("old-queued"));
  const paused = writer.pause();
  release();
  await Promise.all([flushing, paused]);
  writer.schedule(snapshot("old-edited"));
  await writer.flush();
  await vi.runAllTimersAsync();
  expect(save.mock.calls.map(([value]) => value.theme)).toEqual(["old-inflight"]);
  writer.discard();
  writer.resume();
  writer.schedule(snapshot("restored"));
  await writer.flush();
  expect(save.mock.calls.map(([value]) => value.theme)).toEqual(["old-inflight", "restored"]);
});

test("discarded obsolete saves cannot return after a late failed write", async () => {
  vi.useFakeTimers();
  let reject!: (error: Error) => void;
  const save = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise<void>((_, fail) => {
          reject = fail;
        }),
    )
    .mockResolvedValue(undefined);
  const writer = new WorkspaceWriter(save, vi.fn());
  writer.schedule(snapshot("obsolete"));
  const flushing = writer.flush();
  const paused = writer.pause();
  writer.discard();
  reject(new Error("late original failure"));
  await expect(flushing).rejects.toThrow("late original failure");
  await expect(paused).rejects.toThrow("late original failure");
  writer.resume();
  await writer.flush();
  expect(save).toHaveBeenCalledTimes(1);
});

test("durable close rejects paused snapshots while recovery's internal drain stays safe", async () => {
  const save = vi.fn().mockResolvedValue(undefined);
  const writer = new WorkspaceWriter(save, vi.fn());
  await writer.pause();
  writer.schedule(snapshot("unsaved-draft"));
  await writer.flush();
  await expect(writer.flushDurably()).rejects.toThrow("Resolve recovery before closing");
  expect(save).not.toHaveBeenCalled();
  writer.discard();
  writer.resume();
  writer.schedule(snapshot("fresh-restored"));
  await writer.flushDurably();
  expect(save.mock.calls.map(([value]) => value.theme)).toEqual(["fresh-restored"]);
});

test("a recovery pause arriving during durable close cannot report a saved workspace", async () => {
  let release!: () => void;
  const save = vi.fn(
    () =>
      new Promise<void>((resolve) => {
        release = resolve;
      }),
  );
  const writer = new WorkspaceWriter(save, vi.fn());
  writer.schedule(snapshot("in-flight"));
  const closing = writer.flushDurably();
  const paused = writer.pause();
  writer.schedule(snapshot("held"));
  release();
  await expect(closing).rejects.toThrow("Workspace saving is paused");
  await paused;
  expect(save).toHaveBeenCalledTimes(1);
  writer.discard();
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
