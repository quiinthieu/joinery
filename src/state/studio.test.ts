import { beforeEach, expect, test, vi } from "vitest";
import { client } from "../ipc/client";
import type { PreparedPlan, SessionInfo, TablePage, TableSchema } from "../ipc/generated";
import type { Bootstrap, ConnectionProfile } from "../ipc/generated";
import { useStudio, querySessionForTab } from "./studio";
import { workspaceSnapshot } from "./workspace";
import { dirtyStore } from "../data/dirty-store";

vi.mock("../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));
const session: SessionInfo = {
  id: "session",
  connectionId: "profile",
  engine: "postgres",
  serverVersion: "fixture",
  latencyMs: 0,
  encrypted: false,
  capabilities: {
    sql: true,
    transactions: true,
    cancellation: true,
    tableBrowsing: true,
    tableEditing: true,
    tableFiltering: true,
    processes: true,
  },
};
function schema(name: string): TableSchema {
  return {
    table: { schema: "public", name: "fixture" },
    columns: [
      {
        id: "c0",
        name,
        databaseType: "text",
        kind: "text",
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: false,
        insertable: false,
        hasDefault: false,
      },
    ],
    primaryKey: [],
    foreignKeys: [],
    editable: false,
  };
}
function page(source: TableSchema): TablePage {
  return {
    result: {
      id: "result",
      runId: "browse",
      statementIndex: 0,
      columns: source.columns,
      retainedRows: 1,
      status: "complete",
      table: source.table,
    },
    nextKey: null,
    hasMore: false,
  };
}
function tableTab() {
  const id = useStudio.getState().openQuery("profile");
  useStudio.getState().updateTab(id, { kind: "table", table: schema("old").table });
  return id;
}
beforeEach(() => {
  vi.clearAllMocks();
  useStudio.setState({
    bootstrap: null,
    tabs: [],
    activeTabId: null,
    tableLayouts: [],
    catalog: {},
    objects: {},
    sessions: { profile: session },
    sessionPool: {},
    sessionStates: {},
    sessionContexts: {},
    sessionContextRevision: {},
  });
});

test("an isolated query session replaces only its tab binding and releases old results", () => {
  vi.mocked(client.call).mockResolvedValue(null);
  const id = useStudio.getState().openQuery("profile");
  useStudio
    .getState()
    .updateTab(id, { results: [page(schema("old")).result], dataSessionId: session.id });
  const isolated = { ...session, id: "isolated" };
  expect(useStudio.getState().bindQuerySession(id, isolated)).toBe(true);
  const state = useStudio.getState(),
    tab = state.tabs.find((t) => t.id === id)!;
  expect(querySessionForTab(tab, state.sessions)?.id).toBe("isolated");
  expect(state.sessions.profile.id).toBe("session");
  expect(tab.results).toEqual([]);
  expect(tab.dataSessionId).toBeNull();
  expect(client.call).toHaveBeenCalledWith("release_result", { resultId: "result" });
});

test("closing one isolated session never falls back to or retires a sibling session", () => {
  vi.mocked(client.call).mockResolvedValue(null);
  const id = useStudio.getState().openQuery("profile");
  const sibling = useStudio.getState().openQuery("profile");
  useStudio.getState().bindQuerySession(id, { ...session, id: "isolated" });
  useStudio.getState().bindQuerySession(sibling, { ...session, id: "sibling" });
  const revision = useStudio.getState().tabs.find((t) => t.id === id)!.revision;
  useStudio
    .getState()
    .applyEvents(
      id,
      [{ type: "sessionClosed", sessionId: "isolated", reason: "Cancelled" }],
      revision,
    );
  const state = useStudio.getState();
  expect(
    querySessionForTab(
      state.tabs.find((t) => t.id === id)!,
      state.sessions,
    ),
  ).toBeUndefined();
  expect(
    querySessionForTab(
      state.tabs.find((t) => t.id === sibling)!,
      state.sessions,
    )?.id,
  ).toBe("sibling");
  expect(state.sessions.profile.id).toBe("session");
});

test("closing an isolated query tab disconnects only its owned socket", () => {
  vi.mocked(client.call).mockResolvedValue(null);
  const id = useStudio.getState().openQuery("profile");
  useStudio.getState().bindQuerySession(id, { ...session, id: "isolated" });
  useStudio.getState().closeTab(id);
  expect(client.call).toHaveBeenCalledWith("disconnect", { sessionId: "isolated" });
  expect(client.call).not.toHaveBeenCalledWith("disconnect", { sessionId: "session" });
  expect(useStudio.getState().sessions.profile.id).toBe("session");
});

test("a late session connection is released when its target tab no longer exists", () => {
  vi.mocked(client.call).mockResolvedValue(null);
  const id = useStudio.getState().openQuery("profile");
  useStudio.getState().closeTab(id);
  expect(useStudio.getState().bindQuerySession(id, { ...session, id: "late" })).toBe(false);
  expect(client.call).toHaveBeenCalledWith("disconnect", { sessionId: "late" });
});

test("a stale close event cannot retire an explicitly reconnected query session", () => {
  vi.mocked(client.call).mockResolvedValue(null);
  const id = useStudio.getState().openQuery("profile");
  useStudio.getState().bindQuerySession(id, { ...session, id: "old" });
  useStudio.getState().retireSession("old");
  useStudio.getState().bindQuerySession(id, { ...session, id: "new" });
  useStudio.getState().retireSession("old");
  const state = useStudio.getState();
  expect(
    querySessionForTab(
      state.tabs.find((t) => t.id === id)!,
      state.sessions,
    )?.id,
  ).toBe("new");
});

test("shared-session loss preserves isolated sockets until a profile disconnect", () => {
  vi.mocked(client.call).mockResolvedValue(null);
  const id = useStudio.getState().openQuery("profile");
  useStudio.getState().bindQuerySession(id, { ...session, id: "isolated" });
  useStudio.getState().retireSession(session.id);
  let state = useStudio.getState();
  expect(
    querySessionForTab(
      state.tabs.find((t) => t.id === id)!,
      state.sessions,
    )?.id,
  ).toBe("isolated");
  state.disconnected("profile");
  state = useStudio.getState();
  expect(
    querySessionForTab(
      state.tabs.find((t) => t.id === id)!,
      state.sessions,
    ),
  ).toBeUndefined();
});

test("workspace recovery persists SQL drafts without live isolated-session bindings", () => {
  vi.mocked(client.call).mockResolvedValue(null);
  useStudio.setState({
    bootstrap: {
      profiles: [{ id: "profile", storeDrafts: true } as ConnectionProfile],
    } as Bootstrap,
  });
  const id = useStudio.getState().openQuery("profile", "SELECT 42");
  useStudio
    .getState()
    .bindQuerySession(id, { ...session, id: "transient-session-must-not-persist" });
  const saved = workspaceSnapshot();
  expect(saved.drafts).toEqual([
    { id, connectionId: "profile", title: "Query 1", sql: "SELECT 42" },
  ]);
  expect(JSON.stringify(saved)).not.toContain("transient-session-must-not-persist");
});

test("a slow earlier schema refresh cannot overwrite the latest table view", async () => {
  let resolveOld!: (value: TableSchema) => void;
  const delayed = new Promise<TableSchema>((resolve) => {
    resolveOld = resolve;
  });
  const fresh = schema("new_name");
  let schemas = 0;
  vi.mocked(client.call).mockImplementation(((command: string) => {
    if (command === "table_schema") return ++schemas === 1 ? delayed : Promise.resolve(fresh);
    if (command === "browse_table") return Promise.resolve(page(fresh));
    return Promise.resolve(null);
  }) as typeof client.call);
  const id = tableTab(),
    old = useStudio.getState().refreshTable(id);
  await useStudio.getState().refreshTable(id);
  resolveOld(schema("old_name"));
  await old;
  const result = useStudio.getState().tabs.find((tab) => tab.id === id)!;
  expect(result.schema?.columns[0].name).toBe("new_name");
  expect(result.pages).toHaveLength(1);
});

test("a page with changed column positions is rejected before it can be edited", async () => {
  vi.mocked(client.call).mockImplementation(((command: string) =>
    Promise.resolve(
      command === "table_schema"
        ? schema("old_name")
        : command === "browse_table"
          ? page(schema("new_name"))
          : null,
    )) as typeof client.call);
  const id = tableTab();
  await useStudio.getState().refreshTable(id);
  const result = useStudio.getState().tabs.find((tab) => tab.id === id)!;
  expect(result.pages).toHaveLength(0);
  expect(result.error?.message).toContain("schema changed");
  expect(client.call).toHaveBeenCalledWith("release_result", { resultId: "result" });
});

test("a new tab opened during startup cannot overwrite recoverable drafts", async () => {
  useStudio.setState({ bootstrap: null, tabs: [], activeTabId: null, sessions: {} });
  let resolve!: (bootstrap: Bootstrap) => void;
  vi.mocked(client.call).mockReturnValue(
    new Promise<Bootstrap>((done) => {
      resolve = done;
    }),
  );
  const pending = useStudio.getState().initialize();
  const fresh = useStudio.getState().openQuery(null, "SELECT 'new work'");
  const savedId = crypto.randomUUID(),
    connectionId = crypto.randomUUID();
  resolve({
    profiles: [{ id: connectionId, storeDrafts: true } as ConnectionProfile],
    groups: [],
    passphraseConfigured: false,
    supportedEngines: [],
    resultBudgetBytes: 0,
    rowLimit: 0,
    savedQueries: [],
    bookmarks: [],
    workspace: {
      theme: "contrast",
      density: "compact",
      sidebarVisible: true,
      inspectorVisible: false,
      activeDraftId: savedId,
      drafts: [{ id: savedId, connectionId, title: "Recovered", sql: "SELECT 'recovered'" }],
      tableLayouts: [],
    },
  });
  await pending;
  expect(useStudio.getState().tabs.map((tab) => tab.id)).toEqual([savedId, fresh]);
  expect(useStudio.getState().activeTabId).toBe(fresh);
  expect(useStudio.getState().sessions).toEqual({});
  expect(
    useStudio.getState().tabs.every((tab) => !tab.results.length && tab.status === "idle"),
  ).toBe(true);
});

test("changing the table query reloads from the first page with the query", async () => {
  const source = schema("label");
  vi.mocked(client.call).mockImplementation(((command: string) =>
    Promise.resolve(
      command === "table_schema" ? source : command === "browse_table" ? page(source) : null,
    )) as typeof client.call);
  const id = tableTab();
  await useStudio.getState().refreshTable(id);
  const query = {
    sort: [{ column: "label", descending: true }],
    filters: [],
    whereClause: null,
    orderClause: null,
  };
  expect(await useStudio.getState().setTableQuery(id, query)).toEqual({ kind: "applied" });
  const result = useStudio.getState().tabs.find((tab) => tab.id === id)!;
  expect(result.query).toEqual(query);
  expect(result.pages).toHaveLength(1);
  expect(client.call).toHaveBeenLastCalledWith("browse_table", {
    sessionId: "session",
    table: source.table,
    after: null,
    query,
    approval: null,
  });
  expect(client.call).toHaveBeenCalledWith("release_result", { resultId: "result" });
});

test("raw table SQL clauses request review when classified non-read and forward approval", async () => {
  const source = schema("label");
  const plan: PreparedPlan = {
    id: "plan-1",
    sessionId: "session",
    statements: [{ sql: "SELECT ...", kind: "unknown", missingWhere: false }],
    warnings: ["Unclassified expression"],
    requiresApproval: true,
    requiresPassphrase: false,
    confirmationText: null,
    expiresAt: 60,
  };
  vi.mocked(client.call).mockImplementation(((command: string) =>
    Promise.resolve(
      command === "table_schema"
        ? source
        : command === "browse_table"
          ? page(source)
          : command === "prepare_browse_table"
            ? plan
            : null,
    )) as typeof client.call);
  const id = tableTab();
  await useStudio.getState().refreshTable(id);
  const query = {
    sort: [],
    filters: [],
    whereClause: "  current_setting('server_version_num') <> ''  ",
    orderClause: null,
  };
  const outcome = await useStudio.getState().setTableQuery(id, query);
  expect(outcome).toEqual({
    kind: "review",
    plan,
    query: { ...query, whereClause: "current_setting('server_version_num') <> ''" },
  });
  expect(
    await useStudio.getState().setTableQuery(id, query, {
      approval: { planId: "plan-1", approval: "token-1" },
    }),
  ).toEqual({ kind: "applied" });
  expect(client.call).toHaveBeenLastCalledWith("browse_table", {
    sessionId: "session",
    table: source.table,
    after: null,
    query: { ...query, whereClause: "current_setting('server_version_num') <> ''" },
    approval: { planId: "plan-1", approval: "token-1" },
  });
});

test("pending edits block sort and filter changes", async () => {
  const source = schema("label");
  vi.mocked(client.call).mockImplementation(((command: string) =>
    Promise.resolve(
      command === "table_schema" ? source : command === "browse_table" ? page(source) : null,
    )) as typeof client.call);
  const id = tableTab();
  await useStudio.getState().refreshTable(id);
  dirtyStore.set({ resultId: "result", row: 0, column: "c0", original: "a", value: "b" });
  const outcome = await useStudio.getState().setTableQuery(id, {
    sort: [],
    filters: [{ column: "label", op: "isNull", value: null }],
    whereClause: null,
    orderClause: null,
  });
  expect(outcome.kind).toBe("refused");
  if (outcome.kind === "refused") expect(outcome.reason).toContain("pending edits");
  expect(useStudio.getState().tabs.find((tab) => tab.id === id)!.query.filters).toHaveLength(0);
  dirtyStore.clear("result");
});

test("opening the same table with a different query opens a separate tab", async () => {
  const source = schema("id");
  vi.mocked(client.call).mockImplementation(((command: string) =>
    Promise.resolve(
      command === "table_schema" ? source : command === "browse_table" ? page(source) : null,
    )) as typeof client.call);
  const table = source.table;
  await useStudio.getState().openTable("profile", table);
  const filtered = {
    sort: [],
    filters: [{ column: "id", op: "eq" as const, value: "42" }],
    whereClause: null,
    orderClause: null,
  };
  await useStudio.getState().openTable("profile", table, filtered);
  await useStudio.getState().openTable("profile", table, filtered);
  const tabs = useStudio.getState().tabs;
  expect(tabs).toHaveLength(2);
  expect(tabs[1].title).toBe("fixture · id = 42");
  expect(useStudio.getState().activeTabId).toBe(tabs[1].id);
});

test("table column layouts are included in workspace snapshots and purged when removing a connection", () => {
  useStudio.setState({
    bootstrap: {
      profiles: [{ id: "profile", storeDrafts: false } as ConnectionProfile],
    } as Bootstrap,
  });
  const table = { schema: "public", name: "customers" };
  useStudio.getState().setTableLayout("profile", table, {
    connectionId: "profile",
    table,
    columnOrder: ["email", "id"],
    hiddenColumns: ["metadata"],
    columnWidths: { email: 310 },
  });
  expect(workspaceSnapshot().tableLayouts).toHaveLength(1);
  expect(workspaceSnapshot().tableLayouts[0].columnOrder).toEqual(["email", "id"]);

  useStudio.getState().removeConnection("profile");
  expect(useStudio.getState().tableLayouts).toEqual([]);
  expect(workspaceSnapshot().tableLayouts).toEqual([]);
});

test("refreshCatalog gates database_objects on objectExplorer capability and clears objects on disconnect", async () => {
  vi.mocked(client.call).mockImplementation(((command: string) => {
    if (command === "catalog") {
      return Promise.resolve([
        {
          table: { schema: "public", name: "orders" },
          kind: "r",
          estimatedRows: 10,
          totalBytes: 1,
        },
      ]);
    }
    if (command === "database_objects") {
      return Promise.resolve([
        {
          object: { schema: "public", name: "calc", kind: "function", identity: "integer" },
          ownerTable: null,
          signature: "(integer) -> integer",
          estimatedRows: null,
          totalBytes: null,
        },
      ]);
    }
    return Promise.resolve(null);
  }) as typeof client.call);

  // Without objectExplorer capability (e.g. Snowflake / omitted mock), database_objects is never called
  await useStudio.getState().refreshCatalog("profile");
  expect(client.call).toHaveBeenCalledTimes(1);
  expect(client.call).toHaveBeenCalledWith("catalog", { sessionId: "session" });
  expect(useStudio.getState().objects.profile).toEqual([]);

  // With objectExplorer: true, database_objects is fetched and stored
  useStudio.setState({
    sessions: {
      profile: {
        ...session,
        capabilities: { ...session.capabilities, objectExplorer: true },
      },
    },
  });
  await useStudio.getState().refreshCatalog("profile");
  expect(client.call).toHaveBeenCalledWith("database_objects", { sessionId: "session" });
  expect(useStudio.getState().objects.profile).toHaveLength(1);

  useStudio.getState().disconnected("profile");
  expect(useStudio.getState().objects.profile).toBeUndefined();
});

test("openObject distinguishes overloaded routines and routes relations to table tabs", async () => {
  const source = schema("id");
  vi.mocked(client.call).mockImplementation(((command: string) =>
    Promise.resolve(
      command === "table_schema" ? source : command === "browse_table" ? page(source) : null,
    )) as typeof client.call);

  await useStudio
    .getState()
    .openObject(
      "profile",
      { schema: "public", name: "fixture", kind: "view", identity: null },
      "ddl",
    );
  expect(useStudio.getState().tabs).toHaveLength(1);
  expect(useStudio.getState().tabs[0].kind).toBe("table");
  expect(useStudio.getState().tabs[0].view).toBe("ddl");

  const fnInt = {
    schema: "public",
    name: "r3_calc",
    kind: "function" as const,
    identity: "integer",
  };
  const fnNum = {
    schema: "public",
    name: "r3_calc",
    kind: "function" as const,
    identity: "numeric, integer",
  };
  await useStudio.getState().openObject("profile", fnInt);
  await useStudio.getState().openObject("profile", fnNum);
  await useStudio.getState().openObject("profile", fnInt);

  const tabs = useStudio.getState().tabs;
  expect(tabs).toHaveLength(3);
  expect(tabs[1].kind).toBe("object");
  expect(tabs[1].title).toBe("r3_calc(integer)");
  expect(tabs[2].title).toBe("r3_calc(numeric, integer)");
  expect(useStudio.getState().activeTabId).toBe(tabs[1].id);
});

test("multiple consoles can attach to a named secondary session and only the last owner disconnects it on close", () => {
  vi.mocked(client.call).mockResolvedValue(null);
  const first = useStudio.getState().openQuery("profile", "SELECT 1");
  const second = useStudio.getState().openQuery("profile", "SELECT 2");
  const secondary = { ...session, id: "secondary-1" };

  expect(useStudio.getState().bindQuerySession(first, secondary)).toBe(true);
  useStudio.getState().renameQuerySession("secondary-1", "  Analytics   lane  ");
  expect(useStudio.getState().sessionPool.profile).toEqual([
    { info: secondary, label: "Analytics lane", primary: false },
  ]);

  expect(useStudio.getState().attachQuerySession(second, "secondary-1")).toBe(true);
  expect(
    querySessionForTab(
      useStudio.getState().tabs.find((t) => t.id === second)!,
      useStudio.getState().sessions,
    )?.id,
  ).toBe("secondary-1");

  // Closing the first tab while `second` is still attached does NOT disconnect `secondary-1`.
  useStudio.getState().closeTab(first);
  expect(client.call).not.toHaveBeenCalledWith("disconnect", { sessionId: "secondary-1" });
  expect(useStudio.getState().sessionPool.profile).toHaveLength(1);

  // Closing the last attached tab disconnects `secondary-1` and removes it from `sessionPool`.
  useStudio.getState().closeTab(second);
  expect(client.call).toHaveBeenCalledWith("disconnect", { sessionId: "secondary-1" });
  expect(useStudio.getState().sessionPool.profile).toBeUndefined();
});

test("retiring a shared secondary session marks every attached console disconnected without falling back to shared", () => {
  vi.mocked(client.call).mockResolvedValue(null);
  const first = useStudio.getState().openQuery("profile");
  const second = useStudio.getState().openQuery("profile");
  useStudio.getState().bindQuerySession(first, { ...session, id: "secondary-shared" });
  useStudio.getState().attachQuerySession(second, "secondary-shared");

  useStudio.getState().retireSession("secondary-shared");
  const state = useStudio.getState();
  for (const id of [first, second]) {
    const tab = state.tabs.find((t) => t.id === id)!;
    expect(tab.querySessionClosed).toBe(true);
    expect(querySessionForTab(tab, state.sessions)).toBeUndefined();
  }
  expect(state.sessions.profile.id).toBe("session");
});

test("sessionState events and context/transaction commands update runtime state and context revisions", async () => {
  const baseState = {
    sessionId: "session",
    transactionMode: "autoCommit" as const,
    transactionState: "idle" as const,
    contextKind: "schema" as const,
    activeContext: "public",
    database: "joinery",
    role: "joinery",
    searchPath: ["public", "pg_catalog"],
  };
  vi.mocked(client.call).mockImplementation(((command: string, args?: Record<string, unknown>) => {
    if (command === "session_contexts") {
      return Promise.resolve({ state: baseState, available: ["public", "analytics"] });
    }
    if (command === "switch_session_context") {
      const target = String(args?.target ?? "public");
      return Promise.resolve({
        state: {
          ...baseState,
          activeContext: target,
          searchPath: [target, "pg_catalog"],
        },
        available: ["public", "analytics"],
      });
    }
    if (command === "set_transaction_mode") {
      return Promise.resolve({
        ...useStudio.getState().sessionStates.session,
        transactionMode: "manual" as const,
      });
    }
    if (command === "commit_transaction" || command === "rollback_transaction") {
      return Promise.resolve({
        ...baseState,
        transactionMode: "manual" as const,
        transactionState: "idle" as const,
      });
    }
    return Promise.resolve(null);
  }) as typeof client.call);

  const tabId = useStudio.getState().openQuery("profile");
  await useStudio.getState().refreshSessionContexts("session");
  expect(useStudio.getState().sessionContexts.session).toEqual(["public", "analytics"]);
  expect(useStudio.getState().sessionStates.session.activeContext).toBe("public");

  await useStudio.getState().switchSessionContext("session", "analytics");
  expect(useStudio.getState().sessionStates.session.activeContext).toBe("analytics");
  expect(useStudio.getState().sessionContextRevision.session).toBe(1);

  await useStudio.getState().setTransactionMode("session", "manual");
  expect(useStudio.getState().sessionStates.session.transactionMode).toBe("manual");

  // QueryEvent::sessionState updates transactionState and bumps revision when searchPath changes.
  const rev = useStudio.getState().tabs.find((t) => t.id === tabId)!.revision;
  useStudio.getState().applyEvents(
    tabId,
    [
      {
        type: "sessionState",
        state: {
          ...useStudio.getState().sessionStates.session,
          transactionState: "inTransaction",
          activeContext: "public",
          searchPath: ["public", "pg_catalog"],
        },
      },
    ],
    rev,
  );
  expect(useStudio.getState().sessionStates.session.transactionState).toBe("inTransaction");
  expect(useStudio.getState().sessionContextRevision.session).toBe(2);

  await useStudio.getState().rollbackTransaction("session");
  expect(useStudio.getState().sessionStates.session.transactionState).toBe("idle");
  expect(useStudio.getState().sessionContextRevision.session).toBe(3);
});
