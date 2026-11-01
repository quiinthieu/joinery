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
  it("recognizes Snowflake generated execution without inventing comparable warehouse context", () => {
    const snowflake = {
      ...session,
      engine: "snowflake",
      serverVersion: "synthetic",
    } as SessionInfo;
    const execution = planSourceForExecution(
      prepared("EXPLAIN USING JSON\nSELECT 1"),
      snowflake,
      undefined,
    )!;
    expect(execution.engine).toBe("snowflake");
    expect(execution.query).toBe("SELECT 1");
    expect(execution.comparisonUnavailable).toContain("warehouse");
    expect(planMismatch(execution, execution)).toContain("warehouse");
    const store = new PlanSnapshotStore();
    expect(() =>
      store.capture(
        "t",
        "sf",
        execution,
        '{"GlobalStats":{},"Operations":[[{"id":0,"operation":"Result"}]]}',
      ),
    ).toThrow("warehouse");
    expect(store.snapshot()).toHaveLength(0);
    expect(
      planSourceForExecution(prepared("SELECT 'content'"), snowflake, runtime),
    ).toBeUndefined();
    // Exact engine checks must reject Snowflake JSON attached to a MySQL execution.
    expect(() =>
      store.capture(
        "t",
        "mismatch",
        { ...source, engine: "mysql" },
        '{"GlobalStats":{},"Operations":[[{"id":0,"operation":"Result"}]]}',
      ),
    ).toThrow("engine");
  });
  it("compares MariaDB generated estimates and actuals using the observed session", () => {
    const maria = { ...session, engine: "mysql", serverVersion: "11.4.13-MariaDB" } as SessionInfo;
    const estimated = planSourceForExecution(
      prepared("EXPLAIN FORMAT=JSON\nSELECT 1"),
      maria,
      runtime,
    )!;
    const actual = planSourceForExecution(
      prepared("ANALYZE FORMAT=JSON\nSELECT 1"),
      maria,
      runtime,
    )!;
    expect(actual.query).toBe("SELECT 1");
    expect(actual.executedSql).toBe("ANALYZE FORMAT=JSON\nSELECT 1");
    expect(planMismatch(estimated, actual)).toBeNull();
  });
  it("compares MySQL JSON estimates with exact TREE actuals without rewriting raw tokens", () => {
    const mysql = { ...session, engine: "mysql", serverVersion: "8.4.13" } as SessionInfo;
    const estimate = planSourceForExecution(
      prepared("EXPLAIN FORMAT=JSON\nSELECT 1"),
      mysql,
      runtime,
    )!;
    const actual = planSourceForExecution(
      prepared("EXPLAIN ANALYZE FORMAT=TREE\nSELECT 1"),
      mysql,
      runtime,
    )!;
    expect(planMismatch(estimate, actual)).toBeNull();
    const store = new PlanSnapshotStore();
    const tree =
      "-> Rows fetched before execution  (cost=0..0 rows=1) (actual time=84e-6..84e-6 rows=1 loops=1)\n";
    const saved = store.capture("tab", "tree", actual, tree).saved;
    expect(saved.raw).toBe(tree);
    expect(saved.source.executedSql).toBe("EXPLAIN ANALYZE FORMAT=TREE\nSELECT 1");
    store.removeSession(mysql.id);
    expect(store.retainedBytes).toBe(0);
  });
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
