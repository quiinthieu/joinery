import { describe, expect, it } from "vitest";
import type { PreparedPlan, SessionInfo, SessionRuntimeState } from "../../ipc/generated";
import {
  MAX_PLAN_SNAPSHOTS,
  PLAN_SNAPSHOT_BUDGET,
  PlanSnapshotStore,
  planMismatch,
  planSourceForExecution,
  type PlanSource,
} from "./plan-snapshots";

const source: PlanSource = {
  engine: "postgres",
  serverVersion: "18",
  connectionId: "c",
  sessionId: "s",
  context: "public",
  query: "SELECT 1",
  executedSql: "EXPLAIN (FORMAT JSON)\nSELECT 1",
};
const raw =
  '[{"Plan":{"Node Type":"Result","Plan Rows":1,"Total Cost":9007199254740993123},"unknown":{"future":12345678901234567890}}]';
const session = {
  engine: "postgres",
  serverVersion: "18",
  connectionId: "c",
  id: "s",
} as SessionInfo;
const runtime: SessionRuntimeState = {
  sessionId: "s",
  transactionMode: "autoCommit",
  transactionState: "idle",
  contextKind: "schema",
  activeContext: "public",
  database: "joinery",
  role: "fixture",
  searchPath: ["public"],
};
function prepared(sql: string): PreparedPlan {
  return {
    id: "p",
    sessionId: "s",
    statements: [{ sql, kind: "read", missingWhere: false }],
    requiresApproval: false,
    requiresPassphrase: false,
    confirmationText: null,
    warnings: [],
    expiresAt: 99999,
  };
}

describe("plan snapshot identity", () => {
  it("uses exact executed query bytes and recognizes only generated single-query wrappers", () => {
    const sql = "/* é */ SELECT 'a; b'; -- retain";
    const estimated = planSourceForExecution(
      prepared(`EXPLAIN (FORMAT JSON)\n${sql}`),
      session,
      runtime,
    )!;
    const actual = planSourceForExecution(
      prepared(`EXPLAIN (ANALYZE, FORMAT JSON)\n${sql}`),
      session,
      runtime,
    )!;
    expect(estimated.query).toBe(sql);
    expect(actual.executedSql).toBe(`EXPLAIN (ANALYZE, FORMAT JSON)\n${sql}`);
    expect(planMismatch(estimated, actual)).toBeNull();
    expect(planSourceForExecution(prepared("EXPLAIN SELECT 1"), session, runtime)).toBeUndefined();
    expect(
      planSourceForExecution(prepared(source.executedSql), session, undefined),
    ).toBeUndefined();
    expect(
      planSourceForExecution(prepared(source.executedSql), { ...session, id: "other" }, runtime),
    ).toBeUndefined();
    const batch = prepared(source.executedSql);
    batch.statements.push({ sql: "SELECT 2", kind: "read", missingWhere: false });
    expect(planSourceForExecution(batch, session, runtime)).toBeUndefined();
  });
  it("rejects differing engine, version, connection, session, context and exact query", () => {
    for (const patch of [
      { engine: "mysql" },
      { serverVersion: "17" },
      { connectionId: "other" },
      { sessionId: "other" },
      { context: "other" },
      { query: "SELECT 1 " },
    ] as Partial<PlanSource>[])
      expect(planMismatch(source, { ...source, ...patch })).toBeTruthy();
    const a = planSourceForExecution(prepared(source.executedSql), session, runtime)!;
    for (const r of [
      { ...runtime, role: "different" },
      { ...runtime, searchPath: ["private", "public"] },
    ]) {
      expect(
        planMismatch(a, planSourceForExecution(prepared(source.executedSql), session, r)!),
      ).toContain("context");
    }
    expect(
      planMismatch(a, planSourceForExecution(prepared(source.executedSql), session, runtime)!),
    ).toBeNull();
  });
});

describe("bounded plan snapshots", () => {
  it("preserves exact raw numeric tokens and unknown fields without retaining mutable sources", () => {
    const store = new PlanSnapshotStore(),
      input = { ...source };
    const { saved } = store.capture("tab", "r", input, raw);
    input.query = "changed";
    expect(saved.raw).toBe(raw);
    expect(saved.source.query).toBe(source.query);
    expect(Object.isFrozen(saved)).toBe(true);
    expect(Object.isFrozen(saved.source)).toBe(true);
    expect(store.capture("tab", "r", input, "invalid").saved).toBe(saved);
    expect(store.snapshot()).toHaveLength(1);
  });
  it("evicts oldest captures globally at count and byte limits, including SQL/context payloads", () => {
    const store = new PlanSnapshotStore();
    for (let i = 0; i <= MAX_PLAN_SNAPSHOTS; i++) store.capture(`tab${i}`, `r${i}`, source, raw);
    expect(store.snapshot()).toHaveLength(MAX_PLAN_SNAPSHOTS);
    expect(store.snapshot()[0].resultId).toBe("r1");
    store.clear();
    const large = { ...source, query: "é".repeat(220_000) };
    for (let i = 0; i < 8; i++) store.capture("tab", `large${i}`, large, raw);
    expect(store.retainedBytes).toBeLessThanOrEqual(PLAN_SNAPSHOT_BUDGET);
    expect(store.snapshot().length).toBeLessThan(8);
    expect(store.snapshot().at(-1)?.resultId).toBe("large7");
  });
  it("rejects invalid, mismatched, oversized and complex input without evicting good captures", () => {
    const store = new PlanSnapshotStore();
    store.capture("tab", "good", source, raw);
    const before = store.snapshot();
    for (const value of ["invalid", "{}", " ".repeat(512 * 1024 + 1), '{"query_block":{}}'])
      expect(() => store.capture("tab", "bad", source, value)).toThrow();
    let deep: object = { "Node Type": "Scan" };
    for (let i = 0; i < 42; i++) deep = { "Node Type": "Loop", Plans: [deep] };
    expect(() => store.capture("tab", "deep", source, JSON.stringify([{ Plan: deep }]))).toThrow(
      "complexity",
    );
    expect(() =>
      store.capture("tab", "large", { ...source, query: "é".repeat(PLAN_SNAPSHOT_BUDGET) }, raw),
    ).toThrow("2 MiB");
    expect(store.snapshot()).toBe(before);
  });
  it("releases only matching owner/session/connection captures and notifies subscribers", () => {
    const store = new PlanSnapshotStore();
    let notifications = 0;
    const stop = store.subscribe(() => notifications++);
    store.capture("a", "1", source, raw);
    store.capture("b", "2", { ...source, sessionId: "s2" }, raw);
    store.capture("c", "3", { ...source, connectionId: "c2", sessionId: "s3" }, raw);
    store.removeOwner("a");
    expect(store.snapshot().map((s) => s.resultId)).toEqual(["2", "3"]);
    store.removeSession("s2");
    expect(store.snapshot().map((s) => s.resultId)).toEqual(["3"]);
    store.removeConnection("c2");
    expect(store.retainedBytes).toBe(0);
    expect(notifications).toBe(6);
    stop();
    store.capture("a", "4", source, raw);
    store.clear();
    expect(notifications).toBe(6);
  });
});
