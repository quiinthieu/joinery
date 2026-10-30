import { afterEach, expect, test, vi } from "vitest";
import { WorkspaceWriter } from "./workspace";
import type { SavedWorkspace } from "../ipc/generated";

const snapshot = (theme: string): SavedWorkspace => ({
  theme,
  density: "compact",
  sidebarVisible: true,
  inspectorVisible: false,
  activeDraftId: null,
  drafts: [],
  tableLayouts: [],
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
