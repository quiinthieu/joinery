import { describe, expect, test } from "vitest";
import type { LockWait } from "../../ipc/generated";
import { summarizeLocks } from "./lock-chains";

const wait = (waitingId: string, blockingIds: string[]): LockWait => ({
  waitingId,
  user: "app",
  waitSeconds: 1,
  lockType: "transactionid",
  lockMode: "ShareLock",
  object: null,
  waitEvent: "Lock:transactionid",
  query: "UPDATE t SET v = 1",
  blockingIds,
  ownSession: false,
});

describe("lock blocking chains", () => {
  test("finds root blockers and counts sessions blocked transitively", () => {
    const summary = summarizeLocks([
      wait("20", ["10"]),
      wait("30", ["20"]),
      wait("31", ["20", "11"]),
      wait("40", ["11"]),
    ]);
    expect(summary.roots).toEqual(["10", "11"]);
    expect(summary.blockedCount.get("10")).toBe(3);
    expect(summary.blockedCount.get("11")).toBe(2);
    expect(summary.blockedCount.get("20")).toBe(2);
    expect(summary.blocks.get("20")).toEqual(["30", "31"]);
    expect(summary.waitingOn.get("31")).toEqual(["20", "11"]);
    expect(summary.cycle).toEqual([]);
  });

  test("reports deadlock cycles without inventing a root", () => {
    const summary = summarizeLocks([wait("1", ["2"]), wait("2", ["1"]), wait("3", ["2"])]);
    expect(summary.roots).toEqual([]);
    expect(summary.cycle).toEqual(["1", "2"]);
    expect(summary.blockedCount.get("1")).toBe(2);
  });

  test("is empty when nothing waits", () => {
    const summary = summarizeLocks([]);
    expect(summary.roots).toEqual([]);
    expect(summary.blocks.size).toBe(0);
  });
});
