import { describe, expect, it } from "vitest";
import type {
  ConnectionProfile,
  DatabaseObjectSummary,
  HistoryEntry,
  SessionInfo,
  TableInfo,
} from "../../ipc/generated";
import { buildSearchEverywhereResults } from "./search-everywhere";

function makeProfile(
  id: string,
  name: string,
  engine: ConnectionProfile["engine"] = "postgres",
): ConnectionProfile {
  return {
    id,
    name,
    groupId: "local",
    engine,
    host: "127.0.0.1",
    port: engine === "mysql" ? 3306 : 5432,
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
  };
}

function makeSession(id: string, connectionId: string, engine: SessionInfo["engine"]): SessionInfo {
  return {
    id,
    connectionId,
    engine,
    serverVersion: "16.4",
    latencyMs: 2,
    encrypted: false,
    capabilities: {
      sql: true,
      transactions: true,
      cancellation: true,
      tableBrowsing: true,
      tableEditing: true,
      tableFiltering: true,
      processes: true,
      objectExplorer: true,
    },
  };
}

describe("buildSearchEverywhereResults", () => {
  const pgProfile = makeProfile("pg-1", "Local Postgres", "postgres");
  const myProfile = makeProfile("my-1", "Local MySQL", "mysql");
  const offlineProfile = makeProfile("pg-offline", "Staging Warehouse", "postgres");

  const sessions: Record<string, SessionInfo | undefined> = {
    "pg-1": makeSession("sess-pg", "pg-1", "postgres"),
    "my-1": makeSession("sess-my", "my-1", "mysql"),
  };

  const catalog: Record<string, TableInfo[]> = {
    "pg-1": [
      {
        table: { schema: "public", name: "orders" },
        kind: "r",
        estimatedRows: 500,
        totalBytes: 65536,
      },
      {
        table: { schema: "analytics", name: "active_orders_v" },
        kind: "v",
        estimatedRows: null,
        totalBytes: null,
      },
    ],
    "my-1": [
      {
        table: { schema: "joinery", name: "sample" },
        kind: "BASE TABLE",
        estimatedRows: 500,
        totalBytes: 32768,
      },
      {
        table: { schema: "joinery", name: "orders" },
        kind: "BASE TABLE",
        estimatedRows: 10,
        totalBytes: 16384,
      },
    ],
    // Cached catalog for an offline profile must NOT appear in live object search
    "pg-offline": [
      {
        table: { schema: "public", name: "secret_offline_table" },
        kind: "r",
        estimatedRows: 1,
        totalBytes: 1024,
      },
    ],
  };

  const objects: Record<string, DatabaseObjectSummary[]> = {
    "pg-1": [
      {
        object: {
          schema: "public",
          name: "r3_calc",
          kind: "function",
          identity: "integer",
        },
        ownerTable: null,
        signature: "(integer) -> integer",
        estimatedRows: null,
        totalBytes: null,
      },
      {
        object: {
          schema: "public",
          name: "r3_calc",
          kind: "function",
          identity: "numeric, integer",
        },
        ownerTable: null,
        signature: "(numeric, integer) -> numeric",
        estimatedRows: null,
        totalBytes: null,
      },
    ],
    "my-1": [
      {
        object: {
          schema: "joinery",
          name: "sample_audit_trg",
          kind: "trigger",
          identity: "sample",
        },
        ownerTable: "sample",
        signature: "AFTER INSERT",
        estimatedRows: null,
        totalBytes: null,
      },
    ],
  };

  const history: HistoryEntry[] = [
    {
      id: "h-1",
      connectionId: "pg-1",
      connectionName: "Local Postgres",
      startedAt: 1700000000000,
      sql: "SELECT count(*) FROM public.orders WHERE status = 'paid';",
      statementCount: 1,
      status: "completed",
      elapsedMs: 4,
    },
  ];

  const commands = [
    { label: "New query tab", shortcut: "⌘ T", icon: "code", action: () => {} },
    { label: "Grid benchmark", shortcut: "1M rows", icon: "gauge", action: () => {} },
  ];

  it("indexes connected profiles across engines, excludes offline profiles, and boosts preferred connection", () => {
    const res = buildSearchEverywhereResults({
      query: "orders",
      scope: "objects",
      commands,
      tabs: [],
      activeTabId: null,
      preferredConnectionId: "my-1",
      profiles: [pgProfile, myProfile, offlineProfile],
      sessions,
      catalog,
      objects,
      history,
    });

    expect(res.connectedConnectionCount).toBe(2);
    expect(res.offlineConnectionCount).toBe(1);
    expect(res.items.map((i) => i.id)).toEqual([
      "obj:my-1:joinery:table:orders:",
      "obj:pg-1:public:table:orders:",
      "obj:pg-1:analytics:view:active_orders_v:",
    ]);

    const offlineCheck = buildSearchEverywhereResults({
      query: "secret_offline_table",
      scope: "all",
      commands,
      tabs: [],
      activeTabId: null,
      preferredConnectionId: "pg-1",
      profiles: [pgProfile, myProfile, offlineProfile],
      sessions,
      catalog,
      objects,
      history,
    });
    expect(offlineCheck.items).toHaveLength(0);
  });

  it("preserves overloaded routine signatures and multi-token cross-connection filtering", () => {
    const routines = buildSearchEverywhereResults({
      query: "r3_calc",
      scope: "objects",
      commands,
      tabs: [],
      activeTabId: null,
      preferredConnectionId: "pg-1",
      profiles: [pgProfile, myProfile],
      sessions,
      catalog,
      objects,
      history,
    });

    expect(routines.items.map((item) => item.label)).toEqual([
      "r3_calc(integer)",
      "r3_calc(numeric, integer)",
    ]);

    const multiToken = buildSearchEverywhereResults({
      query: "mysql sample",
      scope: "all",
      commands,
      tabs: [],
      activeTabId: null,
      preferredConnectionId: "pg-1",
      profiles: [pgProfile, myProfile],
      sessions,
      catalog,
      objects,
      history,
    });
    expect(multiToken.items[0]?.id).toBe("obj:my-1:joinery:table:sample:");
  });

  it("respects scope filters, avoids duplicating singleton utility tabs, and caps large result sets", () => {
    const benchmarkCheck = buildSearchEverywhereResults({
      query: "Grid benchmark",
      scope: "all",
      commands,
      tabs: [
        { id: "tab-bench", title: "Grid benchmark", kind: "benchmark", connectionId: null },
        { id: "tab-q1", title: "Query 1", kind: "query", connectionId: "pg-1" },
      ],
      activeTabId: "tab-bench",
      preferredConnectionId: "pg-1",
      profiles: [pgProfile],
      sessions: { "pg-1": sessions["pg-1"] },
      catalog,
      objects,
      history,
    });
    expect(benchmarkCheck.items).toHaveLength(1);
    expect(benchmarkCheck.items[0]?.kind).toBe("command");

    const historyScope = buildSearchEverywhereResults({
      query: "paid",
      scope: "history",
      commands,
      tabs: [],
      activeTabId: null,
      preferredConnectionId: "pg-1",
      profiles: [pgProfile],
      sessions: { "pg-1": sessions["pg-1"] },
      catalog,
      objects,
      history,
    });
    expect(historyScope.items).toHaveLength(1);
    expect(historyScope.items[0]?.kind).toBe("history");

    const hugeCatalog: TableInfo[] = Array.from({ length: 250 }, (_, idx) => ({
      table: { schema: "public", name: `metric_table_${idx}` },
      kind: "r",
      estimatedRows: null,
      totalBytes: null,
    }));
    const capped = buildSearchEverywhereResults({
      query: "metric_table",
      scope: "objects",
      commands,
      tabs: [],
      activeTabId: null,
      preferredConnectionId: "pg-1",
      profiles: [pgProfile],
      sessions: { "pg-1": sessions["pg-1"] },
      catalog: { "pg-1": hugeCatalog },
      objects: {},
      history: [],
    });
    expect(capped.items).toHaveLength(80);
  });
});
