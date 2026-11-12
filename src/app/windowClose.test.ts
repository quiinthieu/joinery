import { expect, test, vi } from "vitest";
import { createWindowCloser, lockWindowInteraction } from "./windowClose";
import { WorkspaceWriter } from "../state/workspace";

function fixture() {
  let pending = false;
  let approval: (() => Promise<void>) | undefined;
  const order: string[] = [];
  const unlock = vi.fn();
  const actions = {
    hasPendingWork: () => pending,
    flush: vi.fn(async () => {
      order.push("flush");
    }),
    cancelConnections: vi.fn(async () => {
      order.push("cancel");
    }),
    shutdown: vi.fn(async () => {
      order.push("shutdown");
    }),
    destroy: vi.fn(async () => {
      order.push("destroy");
    }),
    confirm: vi.fn((approve: () => Promise<void>) => {
      approval = approve;
    }),
    lockInteraction: vi.fn(() => unlock),
  };
  return {
    actions,
    order,
    unlock,
    close: createWindowCloser(actions),
    setPending: () => {
      pending = true;
    },
    approve: () => approval!(),
  };
}

test("new pending work during workspace flush requires approval before shutdown", async () => {
  const f = fixture();
  let flushed!: () => void;
  f.actions.flush.mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        flushed = resolve;
      }),
  );
  const closing = f.close();
  f.setPending();
  flushed();
  await closing;
  expect(f.actions.confirm).toHaveBeenCalledOnce();
  expect(f.actions.shutdown).not.toHaveBeenCalled();
  expect(f.actions.destroy).not.toHaveBeenCalled();
  expect(f.unlock).toHaveBeenCalledOnce();
  await f.approve();
  expect(f.order).toEqual(["flush", "cancel", "shutdown", "destroy"]);
});

test("pending work stays open until explicit approval and duplicate close is ignored", async () => {
  const f = fixture();
  f.setPending();
  await f.close();
  expect(f.actions.flush).not.toHaveBeenCalled();
  let flushed!: () => void;
  f.actions.flush.mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        flushed = resolve;
      }),
  );
  const approved = f.approve();
  await f.close();
  expect(f.actions.confirm).toHaveBeenCalledOnce();
  flushed();
  await approved;
  expect(f.actions.destroy).toHaveBeenCalledOnce();
});

for (const step of ["flush", "cancelConnections", "shutdown"] as const)
  test(`${step} failure keeps the window open and allows a retry`, async () => {
    const f = fixture();
    f.actions[step].mockRejectedValueOnce(new Error("Interrupted close"));
    await expect(f.close()).rejects.toThrow("Interrupted close");
    expect(f.actions.destroy).not.toHaveBeenCalled();
    expect(f.unlock).toHaveBeenCalledOnce();
    await f.close();
    expect(f.actions.destroy).toHaveBeenCalledOnce();
  });

test("closing blocks input and shortcuts and restores prior interaction state", () => {
  const root = document.createElement("div");
  root.id = "root";
  document.body.append(root);
  const clicked = vi.fn(),
    shortcut = vi.fn();
  root.addEventListener("click", clicked);
  document.addEventListener("keydown", shortcut);
  const unlock = lockWindowInteraction();
  try {
    expect(root.inert).toBe(true);
    root.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }),
    );
    expect(clicked).not.toHaveBeenCalled();
    expect(shortcut).not.toHaveBeenCalled();
    unlock();
    expect(root.inert).toBe(false);
    root.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(clicked).toHaveBeenCalledOnce();
    expect(shortcut).toHaveBeenCalledOnce();
  } finally {
    unlock();
    document.removeEventListener("keydown", shortcut);
    root.remove();
  }
});

test("even approved close cannot treat a paused workspace drain as durable", async () => {
  const f = fixture();
  const writer = new WorkspaceWriter(vi.fn(), vi.fn());
  await writer.pause();
  f.actions.flush.mockImplementation(() => writer.flushDurably());
  f.setPending();
  await f.close();
  await expect(f.approve()).rejects.toThrow("Resolve recovery before closing");
  expect(f.actions.shutdown).not.toHaveBeenCalled();
  expect(f.actions.destroy).not.toHaveBeenCalled();
  writer.resume();
  await f.approve();
  expect(f.actions.destroy).toHaveBeenCalledOnce();
});
