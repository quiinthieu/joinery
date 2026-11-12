import { afterEach, expect, test, vi } from "vitest";
import type { SnowflakeConsoleReceipt } from "./SnowflakeConsole";
import { receiptView } from "./SnowflakeConsole.test-fixtures";
import { startConsoleReceiptReconciler } from "./SnowflakeConsoleReconciler";

const disposers: (() => void)[] = [];
afterEach(() => {
  disposers.splice(0).forEach((dispose) => dispose());
  vi.useRealTimers();
});

function id(index: number): string {
  return `00000000-0000-4000-8000-${index.toString(16).padStart(12, "0")}`;
}

function pending(clientRequestId: string): SnowflakeConsoleReceipt {
  return { clientRequestId, lookupPending: true };
}

function settled(clientRequestId: string): SnowflakeConsoleReceipt {
  const view = receiptView();
  view.run.metadata.clientRequestId = clientRequestId;
  return {
    clientRequestId,
    sessionId: view.run.metadata.sessionId,
    connectionId: view.run.metadata.connectionId,
    planDigest: view.run.metadata.planDigest,
    runId: view.run.metadata.runId,
    view,
    lookupPending: false,
  };
}

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test("the four receipt slots include handoff, and a pending ID never overlaps itself", async () => {
  vi.useFakeTimers();
  const receipts = Object.fromEntries(
    Array.from({ length: 6 }, (_, index) => [id(index), pending(id(index))]),
  );
  const heldHandoffs = Array.from({ length: 6 }, deferred);
  const refresh = vi.fn(async (key: string) => {
    receipts[key] = settled(key);
  });
  const handoff = vi.fn(async (key: string) => {
    await heldHandoffs[Number.parseInt(key.slice(-12), 16)].promise;
    receipts[key] = { ...receipts[key], inventoried: true, nativeSettled: true };
  });
  const dispose = startConsoleReceiptReconciler({
    getReceipts: () => receipts,
    refresh,
    handoff,
    discoverPending: vi.fn(async () => {}),
    onError: vi.fn(),
  });
  disposers.push(dispose);
  await vi.advanceTimersByTimeAsync(2_000);
  expect(refresh.mock.calls.map(([key]) => key)).toEqual([id(0), id(1), id(2), id(3)]);
  expect(handoff).toHaveBeenCalledTimes(4);
  heldHandoffs[0].resolve();
  await vi.advanceTimersByTimeAsync(2);
  expect(refresh.mock.calls.map(([key]) => key)).toEqual([id(0), id(1), id(2), id(3), id(4)]);
  expect(handoff).toHaveBeenCalledTimes(5);
  dispose();
  heldHandoffs.forEach((hold) => hold.resolve());
  await vi.advanceTimersByTimeAsync(60_000);
  expect(refresh).toHaveBeenCalledTimes(5);
});

test("10,000 legacy correlations drain in bounded batches with a timer yield between batches", async () => {
  vi.useFakeTimers();
  const receipts = Object.fromEntries(
    Array.from({ length: 10_000 }, (_, index) => [id(index), pending(id(index))]),
  );
  let active = 0;
  let maximum = 0;
  const refresh = vi.fn(async (key: string) => {
    active += 1;
    maximum = Math.max(maximum, active);
    receipts[key] = settled(key);
  });
  const handoff = vi.fn(async (key: string) => {
    delete receipts[key]; // Simulate the bounded native-confirmed detail cache.
    active -= 1;
  });
  const independentTimer = vi.fn();
  disposers.push(
    startConsoleReceiptReconciler({
      getReceipts: () => receipts,
      refresh,
      handoff,
      discoverPending: vi.fn(async () => {}),
      onError: vi.fn(),
    }),
  );
  setTimeout(independentTimer, 1);
  await vi.advanceTimersByTimeAsync(1);
  expect(independentTimer).toHaveBeenCalledOnce();
  expect(refresh.mock.calls.length).toBeLessThan(10_000);
  await vi.advanceTimersByTimeAsync(5_000);
  expect(refresh).toHaveBeenCalledTimes(10_000);
  expect(handoff).toHaveBeenCalledTimes(10_000);
  expect(maximum).toBe(4);
  expect(active).toBe(0);
});

test("null and swallowed lookup failures back off to sixty seconds without dropping correlations", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(0);
  const key = id(1);
  const receipts: Record<string, SnowflakeConsoleReceipt> = { [key]: pending(key) };
  const times: number[] = [];
  const refresh = vi.fn(async () => {
    times.push(Date.now());
    receipts[key] = {
      ...receipts[key],
      error: "No durable receipt found yet",
      lookupPending: true,
    };
  });
  const handoff = vi.fn(async () => {});
  const onError = vi.fn();
  disposers.push(
    startConsoleReceiptReconciler({
      getReceipts: () => receipts,
      refresh,
      handoff,
      discoverPending: vi.fn(async () => {}),
      onError,
    }),
  );
  await vi.advanceTimersByTimeAsync(123_000);
  expect(times).toEqual([0, 1_000, 3_000, 7_000, 15_000, 31_000, 63_000, 123_000]);
  expect(handoff).not.toHaveBeenCalled();
  expect(receipts[key].lookupPending).toBe(true);
  expect(onError).toHaveBeenCalledExactlyOnceWith("No durable receipt found yet");
});

test("live pending work polls each second and becomes quiet only after exact native handoff", async () => {
  vi.useFakeTimers();
  const key = id(2);
  const receipts: Record<string, SnowflakeConsoleReceipt> = { [key]: pending(key) };
  const refresh = vi.fn(async () => {
    if (refresh.mock.calls.length === 3) receipts[key] = settled(key);
  });
  const handoff = vi.fn(async () => {
    receipts[key] = { ...receipts[key], inventoried: true, nativeSettled: true };
  });
  disposers.push(
    startConsoleReceiptReconciler({
      getReceipts: () => receipts,
      refresh,
      handoff,
      discoverPending: vi.fn(async () => {}),
      onError: vi.fn(),
    }),
  );
  await vi.advanceTimersByTimeAsync(10_000);
  expect(refresh).toHaveBeenCalledTimes(3);
  expect(handoff).toHaveBeenCalledOnce();
  expect(receipts[key].nativeSettled).toBe(true);
});

test("identity and handoff failures remain visible and retry without inventing inventory proof", async () => {
  vi.useFakeTimers();
  const key = id(3);
  const receipts = { [key]: settled(key) };
  const refresh = vi.fn(async () => {});
  const handoff = vi.fn(async () => {
    throw new Error("Native receipt identity changed");
  });
  const onError = vi.fn();
  disposers.push(
    startConsoleReceiptReconciler({
      getReceipts: () => receipts,
      refresh,
      handoff,
      discoverPending: vi.fn(async () => {}),
      onError,
    }),
  );
  await vi.advanceTimersByTimeAsync(3_000);
  expect(handoff).toHaveBeenCalledTimes(3);
  expect(onError).toHaveBeenCalledExactlyOnceWith("Native receipt identity changed");
  expect(receipts[key].inventoried).toBeUndefined();
  expect(receipts[key].nativeSettled).toBeUndefined();
});

test("settled inventory caches are skipped but held or unconfirmed entries still reconcile", async () => {
  vi.useFakeTimers();
  const receipts: Record<string, SnowflakeConsoleReceipt> = {
    [id(0)]: { ...settled(id(0)), inventoried: true, nativeSettled: true },
    [id(1)]: { ...pending(id(1)), inventoried: true, nativeSettled: true },
    [id(2)]: { ...settled(id(2)), inventoried: true, nativeSettled: false },
    [id(3)]: { ...settled(id(3)), inventoried: false, nativeSettled: true },
  };
  const refresh = vi.fn(async (_key: string) => {});
  disposers.push(
    startConsoleReceiptReconciler({
      getReceipts: () => receipts,
      refresh,
      handoff: vi.fn(async () => {}),
      discoverPending: vi.fn(async () => {}),
      onError: vi.fn(),
    }),
  );
  await vi.advanceTimersByTimeAsync(0);
  expect(refresh.mock.calls.map(([key]) => key)).toEqual([id(1), id(2), id(3)]);
});

test("pending discovery is independent, nonoverlapping, and its new IDs use the receipt lane", async () => {
  vi.useFakeTimers();
  const receipts: Record<string, SnowflakeConsoleReceipt> = {};
  const hold = deferred();
  const discoverPending = vi.fn(async () => {
    await hold.promise;
    receipts[id(0)] = pending(id(0));
  });
  const refresh = vi.fn(async () => {});
  disposers.push(
    startConsoleReceiptReconciler({
      getReceipts: () => receipts,
      refresh,
      handoff: vi.fn(async () => {}),
      discoverPending,
      onError: vi.fn(),
    }),
  );
  await vi.advanceTimersByTimeAsync(30_000);
  expect(discoverPending).toHaveBeenCalledOnce();
  expect(refresh).not.toHaveBeenCalled();
  hold.resolve();
  await vi.advanceTimersByTimeAsync(1_001);
  expect(refresh).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(9_000);
  expect(discoverPending).toHaveBeenCalledTimes(2);
});

test("removed and settled attempts are pruned, so rediscovery gets a fresh attempt", async () => {
  vi.useFakeTimers();
  const key = id(4);
  const receipts: Record<string, SnowflakeConsoleReceipt> = { [key]: pending(key) };
  const refresh = vi.fn(async () => {
    throw new Error("Temporary lookup failure");
  });
  disposers.push(
    startConsoleReceiptReconciler({
      getReceipts: () => receipts,
      refresh,
      handoff: vi.fn(async () => {}),
      discoverPending: vi.fn(async () => {}),
      onError: vi.fn(),
    }),
  );
  await vi.advanceTimersByTimeAsync(7_000);
  expect(refresh).toHaveBeenCalledTimes(4);
  delete receipts[key];
  await vi.advanceTimersByTimeAsync(1_000);
  receipts[key] = pending(key);
  await vi.advanceTimersByTimeAsync(1_000);
  expect(refresh).toHaveBeenCalledTimes(5);
});

test("disposal before startup schedules no RPC, and disposal during refresh prevents handoff", async () => {
  vi.useFakeTimers();
  const receipts = { [id(1)]: settled(id(1)) };
  const hold = deferred();
  const refresh = vi.fn(async () => {
    await hold.promise;
  });
  const handoff = vi.fn(async () => {});
  const discoverPending = vi.fn(async () => {});
  const options = {
    getReceipts: () => receipts,
    refresh,
    handoff,
    discoverPending,
    onError: vi.fn(),
  };
  const beforeStart = startConsoleReceiptReconciler(options);
  beforeStart();
  await vi.advanceTimersByTimeAsync(0);
  expect(refresh).not.toHaveBeenCalled();
  expect(discoverPending).not.toHaveBeenCalled();
  const dispose = startConsoleReceiptReconciler(options);
  disposers.push(dispose);
  await vi.advanceTimersByTimeAsync(0);
  expect(refresh).toHaveBeenCalledOnce();
  expect(discoverPending).toHaveBeenCalledOnce();
  dispose();
  hold.resolve();
  await vi.advanceTimersByTimeAsync(60_000);
  expect(refresh).toHaveBeenCalledOnce();
  expect(handoff).not.toHaveBeenCalled();
  expect(discoverPending).toHaveBeenCalledOnce();
});

test("remount shares the four slots and cannot overlap a disposed owner's in-flight ID", async () => {
  vi.useFakeTimers();
  const receipts = Object.fromEntries(
    Array.from({ length: 8 }, (_, index) => [id(index), pending(id(index))]),
  );
  const firstHolds = Array.from({ length: 8 }, deferred);
  const nextHolds = Array.from({ length: 8 }, deferred);
  const running = new Set<string>();
  let maximum = 0;
  const enter = (key: string) => {
    expect(running.has(key)).toBe(false);
    running.add(key);
    maximum = Math.max(maximum, running.size);
  };
  const firstRefresh = vi.fn(async (key: string) => {
    enter(key);
    await firstHolds[Number.parseInt(key.slice(-12), 16)].promise;
    running.delete(key);
    receipts[key] = settled(key);
  });
  const nextRefresh = vi.fn(async (key: string) => {
    enter(key);
    await nextHolds[Number.parseInt(key.slice(-12), 16)].promise;
    running.delete(key);
    receipts[key] = settled(key);
  });
  const firstHandoff = vi.fn(async () => {});
  const first = startConsoleReceiptReconciler({
    getReceipts: () => receipts,
    refresh: firstRefresh,
    handoff: firstHandoff,
    discoverPending: vi.fn(async () => {}),
    onError: vi.fn(),
  });
  disposers.push(first);
  await vi.advanceTimersByTimeAsync(0);
  expect(firstRefresh).toHaveBeenCalledTimes(4);
  first();
  const next = startConsoleReceiptReconciler({
    getReceipts: () => receipts,
    refresh: nextRefresh,
    handoff: vi.fn(async () => {}),
    discoverPending: vi.fn(async () => {}),
    onError: vi.fn(),
  });
  disposers.push(next);
  await vi.advanceTimersByTimeAsync(2_000);
  expect(nextRefresh).not.toHaveBeenCalled();
  firstHolds[0].resolve();
  await vi.advanceTimersByTimeAsync(2);
  expect(nextRefresh).toHaveBeenCalledExactlyOnceWith(id(4));
  expect(firstHandoff).not.toHaveBeenCalled();
  expect(maximum).toBe(4);
  next();
  firstHolds.forEach((hold) => hold.resolve());
  nextHolds.forEach((hold) => hold.resolve());
  await vi.advanceTimersByTimeAsync(1);
  expect(running.size).toBe(0);
  expect(nextRefresh).toHaveBeenCalledOnce();
});

test("pending-discovery and state-reader errors are surfaced with bounded retries", async () => {
  vi.useFakeTimers();
  const onError = vi.fn();
  const getReceipts = vi.fn((): Record<string, SnowflakeConsoleReceipt> => {
    throw new Error("State unavailable");
  });
  const discoverPending = vi.fn(async () => {
    throw new Error("Pending identity invalid");
  });
  disposers.push(
    startConsoleReceiptReconciler({
      getReceipts,
      refresh: vi.fn(async () => {}),
      handoff: vi.fn(async () => {}),
      discoverPending,
      onError,
    }),
  );
  await vi.advanceTimersByTimeAsync(10_000);
  expect(getReceipts.mock.calls.length).toBeLessThanOrEqual(12);
  expect(discoverPending).toHaveBeenCalledTimes(2);
  expect(onError).toHaveBeenCalledWith("State unavailable");
  expect(onError).toHaveBeenCalledWith("Pending identity invalid");
});

test("a remount waits for the disposed owner's pending discovery to settle", async () => {
  vi.useFakeTimers();
  const hold = deferred();
  const firstDiscovery = vi.fn(async () => {
    await hold.promise;
  });
  const nextDiscovery = vi.fn(async () => {});
  const options = {
    getReceipts: () => ({}),
    refresh: vi.fn(async () => {}),
    handoff: vi.fn(async () => {}),
    onError: vi.fn(),
  };
  const first = startConsoleReceiptReconciler({ ...options, discoverPending: firstDiscovery });
  disposers.push(first);
  await vi.advanceTimersByTimeAsync(0);
  first();
  const next = startConsoleReceiptReconciler({ ...options, discoverPending: nextDiscovery });
  disposers.push(next);
  await vi.advanceTimersByTimeAsync(30_000);
  expect(firstDiscovery).toHaveBeenCalledOnce();
  expect(nextDiscovery).not.toHaveBeenCalled();
  hold.resolve();
  await vi.advanceTimersByTimeAsync(1_001);
  expect(nextDiscovery).toHaveBeenCalledOnce();
});
