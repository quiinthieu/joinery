import { describe, expect, it } from "vitest";
import type {
  ConnectionProfile,
  DatabaseObjectSummary,
  HistorySummary,
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

  const history: HistorySummary[] = [
    {
      provenance: "recorded",
      id: "h-1",
      connectionId: "pg-1",
      connectionName: "Local Postgres",
      startedAt: 1700000000000,
      sqlPreview: "SELECT count(*) FROM public.orders WHERE status = 'paid';",
      sqlStored: true,
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

  it("ranks saved queries in All and Saved scopes with preferred-connection affinity boost", () => {
    const savedQueries = [
      {
        id: "sq-pg",
        title: "Daily Revenue Rollup",
        sql: "SELECT date_trunc('day', created_at), sum(amount) FROM public.orders GROUP BY 1;",
        connectionId: "pg-1",
        updatedAtMs: 1700000001000,
      },
      {
        id: "sq-my",
        title: "Daily Revenue Rollup",
        sql: "SELECT DATE(created_at), SUM(amount) FROM joinery.orders GROUP BY 1;",
        connectionId: "my-1",
        updatedAtMs: 1700000002000,
      },
    ];

    const savedOnly = buildSearchEverywhereResults({
      query: "revenue",
      scope: "saved",
      commands,
      tabs: [],
      activeTabId: null,
      preferredConnectionId: "my-1",
      profiles: [pgProfile, myProfile],
      sessions,
      catalog,
      objects,
      savedQueries,
      history,
    });
    expect(savedOnly.items.map((item) => item.id)).toEqual(["saved:sq-my", "saved:sq-pg"]);

    const allScope = buildSearchEverywhereResults({
      query: "Daily Revenue Rollup",
      scope: "all",
      commands,
      tabs: [],
      activeTabId: null,
      preferredConnectionId: "pg-1",
      profiles: [pgProfile, myProfile],
      sessions,
      catalog,
      objects,
      savedQueries,
      history,
    });
    expect(allScope.items[0]?.id).toBe("saved:sq-pg");
  });

  it("propagates contextual command badges and disabled reasons", () => {
    const res = buildSearchEverywhereResults({
      query: "export",
      scope: "actions",
      commands: [
        {
          label: "Export data to file…",
          shortcut: "CSV · JSON · SQL",
          icon: "dl",
          contextBadge: "Active tab",
          disabledReason: "Load table rows in Data view before exporting.",
          action: () => {},
        },
      ],
      tabs: [],
      activeTabId: null,
      preferredConnectionId: null,
      profiles: [],
      sessions: {},
      catalog: {},
      objects: {},
      history: [],
    });

    expect(res.items).toHaveLength(1);
    expect(res.items[0]).toMatchObject({
      kind: "command",
      label: "Export data to file…",
      contextBadge: "Active tab",
      disabledReason: "Load table rows in Data view before exporting.",
      subtitle: "Load table rows in Data view before exporting.",
    });
  });
});

describe("script history accounting", () => {
  const base: HistorySummary = {
    provenance: "recorded",
    id: "script",
    connectionId: "pg",
    connectionName: "Accounting fixture",
    startedAt: 1,
    sqlPreview: null,
    sqlStored: false,
    statementCount: 999,
    status: "completed",
    elapsedMs: 8,
  };
  function search(history: HistorySummary[], query = "") {
    return buildSearchEverywhereResults({
      query,
      scope: "history",
      commands: [],
      tabs: [],
      activeTabId: null,
      preferredConnectionId: null,
      profiles: [],
      sessions: {},
      catalog: {},
      objects: {},
      history,
    }).items;
  }

  it("keeps legacy counts unknown without inventing SQL or using the old planned count", () => {
    const [item] = search([base]);
    expect(item.label).toBe("SQL text not stored");
    expect(item.subtitle).toContain("Completed · Statement counts unknown");
    expect(item.subtitle).not.toContain("999");
    expect(item.subtitle).not.toContain("Attempted:");
  });

  it("shows and searches the stopped suffix even when SQL text was not stored", () => {
    const entry: HistorySummary = {
      ...base,
      status: "stopped",
      executionSummary: {
        planned: 3,
        attempted: 1,
        completed: 1,
        notAttempted: 2,
        stopReason: "resultLimit",
      },
    };
    const [item] = search([entry], "not attempted");
    expect(item.label).toBe("SQL text not stored");
    expect(item.subtitle).toContain(
      "Stopped · Planned: 3 · Attempted: 1 · Completed: 1 · Not attempted: 2",
    );
    expect(item.subtitle).toContain(
      "Result retention limit reached · Statements 2–3 were not attempted",
    );
    expect(item.subtitle).toContain(
      "Attempts may not reach the server; completed statements may be uncommitted",
    );
    expect(item.subtitle).not.toContain("999");
  });

  it.each([
    ["completed", null, "Completed"],
    ["cancelled", "cancelled", "Cancelled"],
    ["failed", "failed", "Failed"],
    ["outcomeUnknown", "outcomeUnknown", "Outcome unknown"],
  ] as const)(
    "keeps the %s outcome and authoritative counts in search results",
    (status, stopReason, label) => {
      const entry: HistorySummary = {
        ...base,
        status,
        sqlPreview: "SELECT\n  7",
        sqlStored: true,
        executionSummary: {
          planned: 1,
          attempted: 1,
          completed: status === "completed" ? 1 : 0,
          notAttempted: 0,
          stopReason,
        },
      };
      const [item] = search([entry]);
      expect(item.label).toBe("SELECT 7");
      expect(item.subtitle).toContain(
        `${label} · Planned: 1 · Attempted: 1 · Completed: ${entry.executionSummary!.completed} · Not attempted: 0`,
      );
      expect(entry.sqlPreview).toBe("SELECT\n  7");
    },
  );
});

// A 256-character summary cannot independently verify a match deep in stored SQL.
describe("native history matches", () => {
  const entry: HistorySummary = {
    provenance: "recorded",
    id: "deep",
    connectionId: "pg",
    connectionName: "Local",
    startedAt: 1,
    sqlPreview: "SELECT /* preview only */",
    sqlStored: true,
    statementCount: 1,
    status: "completed",
    elapsedMs: 1,
  };
  const input = {
    query: "far_beyond_preview",
    scope: "history" as const,
    commands: [],
    tabs: [],
    activeTabId: null,
    preferredConnectionId: null,
    profiles: [],
    sessions: {},
    catalog: {},
    objects: {},
    history: [entry],
  };
  it("preserves a native deep-SQL match for the exact requested query", () => {
    expect(
      buildSearchEverywhereResults({ ...input, historyMatchedQuery: input.query }).items,
    ).toHaveLength(1);
  });
  it("does not treat an old query's native matches as matches for a new query", () => {
    expect(
      buildSearchEverywhereResults({ ...input, historyMatchedQuery: "previous query" }).items,
    ).toHaveLength(0);
    expect(buildSearchEverywhereResults(input).items).toHaveLength(0);
  });
});

describe("history lifecycle metadata", () => {
  const entry: HistorySummary = {
    id: "lifecycle",
    connectionId: "pg",
    connectionName: "Local",
    startedAt: 1,
    sqlPreview: "SELECT original",
    sqlStored: true,
    statementCount: 1,
    provenance: "admitted",
    status: null,
    elapsedMs: null,
  };
  function search(history: HistorySummary[], query = "") {
    return buildSearchEverywhereResults({
      query,
      scope: "history",
      commands: [],
      tabs: [],
      activeTabId: null,
      preferredConnectionId: null,
      profiles: [],
      sessions: {},
      catalog: {},
      objects: {},
      history,
    }).items;
  }
  it("renders and searches admitted metadata without fabricating completion or duration", () => {
    const [item] = search([entry], "completion pending");
    expect(item.subtitle).toContain("Admitted; completion pending");
    expect(item.subtitle).toContain("Duration unavailable");
    expect(item.subtitle).toContain("Admission does not prove dispatch or server submission");
    expect(item.subtitle).not.toMatch(/undefined|\b0 ms\b|Completed ·/);
  });
  it.each(["interrupted", "restoreRecovered"] as const)(
    "searches %s unknown provenance and preserves exact known terminals",
    (provenance) => {
      const [unknown] = search([{ ...entry, provenance }], "recovered completion unknown");
      expect(unknown.subtitle).toContain("Completion unknown");
      const [known] = search([{ ...entry, provenance, status: "completed", elapsedMs: 17 }]);
      expect(known.subtitle).toContain("Completed · Statement counts unknown");
      expect(known.subtitle).toContain("17 ms");
      expect(known.subtitle).toContain("Original recorded completion is preserved");
    },
  );
});
