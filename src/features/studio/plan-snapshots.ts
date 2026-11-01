import type { PreparedPlan, SessionInfo, SessionRuntimeState } from "../../ipc/generated";
import { parseQueryPlan } from "./query-plan";

export const MAX_PLAN_SNAPSHOTS = 10;
export const PLAN_SNAPSHOT_BUDGET = 2 * 1024 * 1024;
export interface PlanSource {
  engine: "postgres" | "mysql";
  serverVersion: string;
  connectionId: string;
  sessionId: string;
  context: string;
  query: string;
  executedSql: string;
}
export interface PlanSnapshot {
  id: number;
  ownerTabId: string;
  resultId: string;
  capturedAt: number;
  source: Readonly<PlanSource>;
  raw: string;
  bytes: number;
}

/** Use immutable executed SQL, never the current editor contents. Only the
 * exact generated single-query wrappers have a known underlying query. */
export function planSourceForExecution(
  plan: PreparedPlan,
  session: SessionInfo | undefined,
  runtime: SessionRuntimeState | undefined,
): PlanSource | undefined {
  if (
    !session ||
    !runtime ||
    plan.sessionId !== session.id ||
    runtime.sessionId !== session.id ||
    !["postgres", "mysql"].includes(session.engine) ||
    plan.statements.length !== 1
  )
    return;
  const executedSql = plan.statements[0].sql;
  const prefixes =
    session.engine === "postgres"
      ? ["EXPLAIN (FORMAT JSON)\n", "EXPLAIN (ANALYZE, FORMAT JSON)\n"]
      : ["EXPLAIN FORMAT=JSON\n", "ANALYZE FORMAT=JSON\n"];
  const prefix = prefixes.find((value) => executedSql.startsWith(value));
  if (!prefix) return;
  return {
    engine: session.engine as PlanSource["engine"],
    serverVersion: session.serverVersion,
    connectionId: session.connectionId,
    sessionId: session.id,
    context: JSON.stringify({
      database: runtime.database,
      role: runtime.role,
      activeContext: runtime.activeContext,
      searchPath: runtime.searchPath,
      transactionMode: runtime.transactionMode,
      transactionState: runtime.transactionState,
    }),
    query: executedSql.slice(prefix.length),
    executedSql,
  };
}

export function planMismatch(
  left: Readonly<PlanSource>,
  right: Readonly<PlanSource>,
): string | null {
  if (left.engine !== right.engine || left.serverVersion !== right.serverVersion)
    return "Different engine or server version";
  if (left.connectionId !== right.connectionId || left.sessionId !== right.sessionId)
    return "Different connection or session";
  if (left.context !== right.context) return "Different observed session context";
  if (left.query !== right.query) return "Different query text";
  return null;
}

/** Session-only, shared across tabs. The budget accounts for UTF-8 payloads,
 * including SQL/context; JS object overhead and display trees are separate. */
export class PlanSnapshotStore {
  private rows: readonly PlanSnapshot[] = [];
  private sequence = 0;
  private listeners = new Set<() => void>();
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  snapshot = () => this.rows;
  private replace(rows: readonly PlanSnapshot[]) {
    if (rows.length === this.rows.length && rows.every((row, i) => row === this.rows[i])) return;
    this.rows = rows;
    this.listeners.forEach((listener) => listener());
  }
  capture(ownerTabId: string, resultId: string, source: PlanSource, raw: string) {
    const existing = this.rows.find((row) => row.resultId === resultId);
    if (existing) return { saved: existing, evicted: 0 };
    const bytes = new TextEncoder().encode(
      JSON.stringify({ ownerTabId, resultId, source, raw }),
    ).length;
    if (bytes > PLAN_SNAPSHOT_BUDGET)
      throw new Error("Plan snapshot exceeds the shared 2 MiB budget.");
    const parsed = parseQueryPlan(raw);
    if ((source.engine === "postgres") !== (parsed.engine === "PostgreSQL"))
      throw new Error("Plan format does not match its execution engine.");
    const saved: PlanSnapshot = Object.freeze({
      id: ++this.sequence,
      ownerTabId,
      resultId,
      capturedAt: Date.now(),
      source: Object.freeze({ ...source }),
      raw,
      bytes,
    });
    const rows = [...this.rows, saved];
    let used = rows.reduce((total, row) => total + row.bytes, 0),
      evicted = 0;
    while (rows.length > MAX_PLAN_SNAPSHOTS || used > PLAN_SNAPSHOT_BUDGET) {
      used -= rows.shift()!.bytes;
      evicted++;
    }
    this.replace(rows);
    return { saved, evicted };
  }
  remove(id: number) {
    this.replace(this.rows.filter((row) => row.id !== id));
  }
  removeOwner(tabId: string) {
    this.replace(this.rows.filter((row) => row.ownerTabId !== tabId));
  }
  removeSession(sessionId: string) {
    this.replace(this.rows.filter((row) => row.source.sessionId !== sessionId));
  }
  removeConnection(connectionId: string) {
    this.replace(this.rows.filter((row) => row.source.connectionId !== connectionId));
  }
  clear = () => this.replace([]);
  get retainedBytes() {
    return this.rows.reduce((total, row) => total + row.bytes, 0);
  }
}

export const planSnapshots = new PlanSnapshotStore();
