import { beforeEach, expect, test, vi } from "vitest";
import { client } from "../ipc/client";
import type {
  PreparedPlan,
  SessionInfo,
  SessionContextOptions,
  SessionRuntimeState,
  TablePage,
  TableSchema,
  TransactionControlResult,
  SnowflakeDmlResult,
  SnowflakeDdlResult,
  ExecutionSummary,
} from "../ipc/generated";
import type { Bootstrap, ConnectionProfile } from "../ipc/generated";
import {
  useStudio,
  metadataSessionForTab,
  querySessionForTab,
  isTabFileDirty,
  MAX_RECENT_LOCATIONS,
  sessionHasOpenTransaction,
  revokeSchemaSelectionBeforeWork,
  observeTemplateSessionContext,
} from "./studio";
import { workspaceSnapshot } from "./workspace";
import { dirtyStore } from "../data/dirty-store";
import { planSnapshots, type PlanSource } from "../features/studio/plan-snapshots";

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
  planSnapshots.clear();
  vi.clearAllMocks();
  useStudio.setState({
    bootstrap: null,
    tabs: [],
    activeTabId: null,
    closedTabs: [],
    tableLayouts: [],
    catalog: {},
    objects: {},
    sessions: { profile: session },
    sessionPool: {},
    sessionStates: {},
    sessionContexts: {},
    sessionContextRevision: {},
    sessionContextAuthorities: {},
    snowflakeContextAuthorities: {},
    snowflakeContextExpanded: {},
    sessionContextSwitching: {},
    sessionTransactionPending: {},
    sessionMetadataRefreshRequired: {},
    sessionDdlCatalogRefreshRequired: {},
    savedQueries: [],
    bookmarks: [],
    sqlFormatSettings: {
      keywordCase: "upper",
      indentWidth: 2,
      linesBetweenQueries: 1,
    },
    sqlSnippets: [],
    recentLocations: [],
    shortcuts: {},
    sidebarVisible: true,
    sidebarCollapsed: false,
    inspectorVisible: false,
  });
});

test("format preferences serialize writes and retain the latest successful settings on failure", async () => {
  const initial = useStudio.getState().sqlFormatSettings;
  const firstSettings = { ...initial, keywordCase: "lower" as const };
  const latestSettings = { ...firstSettings, indentWidth: 4 };
  let finishFirst!: (settings: typeof initial) => void;
  vi.mocked(client.call).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishFirst = resolve;
      }),
  );
  vi.mocked(client.call).mockRejectedValueOnce(new Error("Metadata disk full"));
  const first = useStudio.getState().saveSqlFormatSettings(firstSettings);
  const latest = useStudio.getState().saveSqlFormatSettings(latestSettings);
  const rejected = expect(latest).rejects.toThrow("Metadata disk full");
  await Promise.resolve();
  expect(client.call).toHaveBeenCalledTimes(1);
  expect(useStudio.getState().sqlFormatSettings).toEqual(latestSettings);
  finishFirst(firstSettings);
  await first;
  await rejected;
  expect(client.call).toHaveBeenCalledTimes(2);
  expect(useStudio.getState().sqlFormatSettings).toEqual(firstSettings);
});

test("plan captures survive result replacement but close/retire release their owners and metadata", () => {
  const state = useStudio.getState();
  const a = state.openQuery("profile"),
    b = state.openQuery("profile");
  const source: PlanSource = {
    engine: "postgres",
    serverVersion: "18",
    connectionId: "profile",
    sessionId: "session",
    context: "known",
    query: "SELECT 'snapshot-only-secret'",
    executedSql: "EXPLAIN (FORMAT JSON)\nSELECT 'snapshot-only-secret'",
  };
  const raw = '[{"Plan":{"Node Type":"Result","Plan Rows":1}}]';
  state.updateTab(a, { planSource: source });
  state.updateTab(b, { planSource: source });
  planSnapshots.capture(a, "ra", source, raw);
  planSnapshots.capture(b, "rb", source, raw);
  state.updateTab(a, { results: [], planSource: undefined });
  expect(planSnapshots.snapshot()).toHaveLength(2);
  expect(JSON.stringify(workspaceSnapshot())).not.toContain("snapshot-only-secret");
  state.closeTab(a);
  expect(planSnapshots.snapshot().map((row) => row.ownerTabId)).toEqual([b]);
  state.retireSession("session");
  expect(planSnapshots.retainedBytes).toBe(0);
  expect(useStudio.getState().tabs.find((tab) => tab.id === b)?.planSource).toBeUndefined();
});

test("disconnecting a connection clears its snapshots without affecting another connection", () => {
  const source: PlanSource = {
    engine: "postgres",
    serverVersion: "18",
    connectionId: "profile",
    sessionId: "session",
    context: "known",
    query: "SELECT 1",
    executedSql: "EXPLAIN (FORMAT JSON)\nSELECT 1",
  };
  const raw = '[{"Plan":{"Node Type":"Result"}}]';
  planSnapshots.capture("a", "ra", source, raw);
  planSnapshots.capture(
    "b",
    "rb",
    { ...source, connectionId: "other", sessionId: "other-session" },
    raw,
  );
  useStudio.getState().disconnected("profile");
  expect(planSnapshots.snapshot().map((row) => row.resultId)).toEqual(["rb"]);
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

for (const operation of ["refresh", "next", "evicted"] as const) {
  test(`a completed ${operation} page can safely narrow editing without losing its rows`, async () => {
    const source = schema("label");
    source.editable = true;
    source.primaryKey = ["label"];
    source.columns[0] = {
      ...source.columns[0],
      primaryKey: true,
      editable: true,
      insertable: true,
    };
    const readonly = page({
      ...source,
      editable: false,
      columns: source.columns.map((column) => ({ ...column, editable: false, insertable: false })),
    });
    readonly.result.editability = {
      status: "readOnly",
      reason: "The original edit lease expired. Refresh to edit again.",
    };
    vi.mocked(client.call).mockImplementation(((command: string) =>
      Promise.resolve(
        command === "table_schema" ? source : command === "browse_table" ? readonly : null,
      )) as typeof client.call);
    const id = tableTab();
    useStudio.getState().updateTab(id, {
      schema: source,
      dataSessionId: session.id,
      pageKeys: { 0: null },
      hasMore: true,
    });
    if (operation === "refresh") await useStudio.getState().refreshTable(id);
    else if (operation === "next") await useStudio.getState().loadNextPage(id);
    else await useStudio.getState().ensureTablePage(id, 0);
    const current = useStudio.getState().tabs.find((tab) => tab.id === id)!;
    expect(current.error).toBeNull();
    expect(current.pages).toHaveLength(1);
    expect(current.pages[0].result.retainedRows).toBe(1);
    expect(current.schema?.editable).toBe(false);
    expect(current.schema?.columns.every((column) => !column.editable && !column.insertable)).toBe(
      true,
    );
    expect(client.call).not.toHaveBeenCalledWith("release_result", {
      resultId: readonly.result.id,
    });
    expect(useStudio.getState().sessions.profile.id).toBe(session.id);
    expect(
      vi.mocked(client.call).mock.calls.filter(([command]) => command === "browse_table"),
    ).toHaveLength(1);
  });
}

test("a read-only page also removes table-level editing when column flags are already false", async () => {
  const source = { ...schema("label"), editable: true };
  const readonly = page(source);
  readonly.result.editability = { status: "readOnly", reason: "Expired fixture lease" };
  vi.mocked(client.call).mockImplementation(((command: string) =>
    Promise.resolve(
      command === "table_schema" ? source : command === "browse_table" ? readonly : null,
    )) as typeof client.call);
  const id = tableTab();
  await useStudio.getState().refreshTable(id);
  const current = useStudio.getState().tabs.find((tab) => tab.id === id)!;
  expect(current.pages).toHaveLength(1);
  expect(current.error).toBeNull();
  expect(current.schema?.editable).toBe(false);
});

test.each(["type", "nullability", "key", "table", "editable", "insertable"] as const)(
  "a read-only page still rejects changed %s metadata or capabilities",
  async (change) => {
    const source = schema("label");
    source.editable = true;
    source.columns[0] = { ...source.columns[0], editable: true, insertable: true };
    const readonly = page({
      ...source,
      columns: source.columns.map((column) => ({ ...column, editable: false, insertable: false })),
    });
    readonly.result.editability = { status: "readOnly", reason: "Expired fixture lease" };
    if (change === "type") readonly.result.columns[0].databaseType = "integer";
    if (change === "nullability") readonly.result.columns[0].nullable = false;
    if (change === "key") readonly.result.columns[0].primaryKey = true;
    if (change === "table") readonly.result.table = { ...source.table, name: "replacement" };
    if (change === "editable") readonly.result.columns[0].editable = true;
    if (change === "insertable") readonly.result.columns[0].insertable = true;
    vi.mocked(client.call).mockImplementation(((command: string) =>
      Promise.resolve(
        command === "table_schema" ? source : command === "browse_table" ? readonly : null,
      )) as typeof client.call);
    const id = tableTab();
    await useStudio.getState().refreshTable(id);
    const current = useStudio.getState().tabs.find((tab) => tab.id === id)!;
    expect(current.pages).toHaveLength(0);
    expect(current.error?.message).toContain("schema changed");
    expect(client.call).toHaveBeenCalledWith("release_result", { resultId: readonly.result.id });
  },
);

for (const operation of [
  "review",
  "schema",
  "next page",
  "evicted page",
  "approved browse",
] as const) {
  test.each(["connectionClosed", "outcomeUnknown", "conflict"] as const)(
    `${operation} preserves its %s error and retires only terminal browse sessions without retry`,
    async (code) => {
      const id = tableTab();
      const queryId = useStudio.getState().openQuery("profile", "SELECT 1");
      const otherId = useStudio.getState().openQuery("profile", "SELECT 2");
      useStudio.getState().bindQuerySession(otherId, { ...session, id: "unaffected" });
      const source: PlanSource = {
        engine: "postgres",
        serverVersion: "18",
        connectionId: "profile",
        sessionId: session.id,
        context: "known",
        query: "SELECT 1",
        executedSql: "EXPLAIN SELECT 1",
      };
      useStudio.getState().updateTab(queryId, { planSource: source });
      planSnapshots.capture(
        queryId,
        "browse-related-plan",
        source,
        '[{"Plan":{"Node Type":"Result"}}]',
      );
      useStudio.setState({
        sessionStates: { [session.id]: transactionRuntime(session.id) },
        sessionContexts: { [session.id]: ["public"] },
      });
      const tableSchema = schema("label");
      useStudio.getState().updateTab(id, {
        schema: tableSchema,
        dataSessionId: session.id,
        pageKeys: { 0: null },
        hasMore: true,
      });
      const error = {
        code,
        message: "The browse commit may have completed. Verify before retrying.",
      };
      const failingCommand =
        operation === "review"
          ? "prepare_browse_table"
          : operation === "schema"
            ? "table_schema"
            : "browse_table";
      vi.mocked(client.call).mockImplementation(((command: string) => {
        if (command === failingCommand) return Promise.reject(error);
        return Promise.resolve(command === "table_schema" ? tableSchema : null);
      }) as typeof client.call);
      const state = useStudio.getState();
      const rawQuery = { sort: [], filters: [], whereClause: "owned_mark(id)", orderClause: null };
      if (operation === "review") await state.setTableQuery(id, rawQuery);
      else if (operation === "schema") await state.refreshTable(id);
      else if (operation === "next page") await state.loadNextPage(id);
      else if (operation === "evicted page") await state.ensureTablePage(id, 0);
      else
        await state.setTableQuery(id, rawQuery, {
          approval: { planId: "approved", approval: "once" },
        });

      const latest = useStudio.getState();
      const table = latest.tabs.find((tab) => tab.id === id)!;
      expect(table.error).toEqual(error);
      expect(table.loadingPage).toBe(false);
      expect(
        vi.mocked(client.call).mock.calls.filter(([command]) => command === failingCommand),
      ).toHaveLength(1);
      const terminal = code === "connectionClosed" || code === "outcomeUnknown";
      expect(
        querySessionForTab(
          latest.tabs.find((tab) => tab.id === queryId)!,
          latest.sessions,
        )?.id,
      ).toBe(terminal ? undefined : session.id);
      expect(
        querySessionForTab(
          latest.tabs.find((tab) => tab.id === otherId)!,
          latest.sessions,
        )?.id,
      ).toBe("unaffected");
      expect(latest.tabs.find((tab) => tab.id === queryId)!.planSource).toEqual(
        terminal ? undefined : source,
      );
      expect(planSnapshots.snapshot()).toHaveLength(terminal ? 0 : 1);
      expect(latest.sessionStates[session.id]).toEqual(
        terminal ? undefined : transactionRuntime(session.id),
      );
      expect(latest.sessionContexts[session.id]).toEqual(terminal ? undefined : ["public"]);
      if (terminal) {
        const calls = vi.mocked(client.call).mock.calls.length;
        await latest.refreshTable(id);
        await latest.loadNextPage(id);
        await latest.ensureTablePage(id, 0);
        expect(client.call).toHaveBeenCalledTimes(calls);
      }
    },
  );
}

test("a late terminal page failure cannot retire or overwrite a replacement session", async () => {
  let reject!: (error: unknown) => void;
  vi.mocked(client.call).mockReturnValue(
    new Promise((_, fail) => {
      reject = fail;
    }),
  );
  const id = tableTab();
  useStudio
    .getState()
    .updateTab(id, { schema: schema("old"), dataSessionId: session.id, hasMore: true });
  const pending = useStudio.getState().loadNextPage(id);
  const replacement = { ...session, id: "replacement" };
  useStudio.setState({ sessions: { profile: replacement } });
  useStudio
    .getState()
    .updateTab(id, { revision: 1, dataSessionId: replacement.id, error: null, loadingPage: false });
  reject({ code: "outcomeUnknown", message: "Old browse outcome unknown" });
  await pending;
  expect(useStudio.getState().sessions.profile).toEqual(replacement);
  expect(useStudio.getState().tabs.find((tab) => tab.id === id)).toMatchObject({
    dataSessionId: replacement.id,
    error: null,
    loadingPage: false,
  });
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
    sqlFormatSettings: {
      keywordCase: "upper",
      indentWidth: 2,
      linesBetweenQueries: 1,
    },
    sqlSnippets: [],
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

test("refreshCatalog clears stale metadata without querying unsupported table browsing", async () => {
  useStudio.setState({
    sessions: {
      profile: {
        ...session,
        capabilities: { ...session.capabilities, tableBrowsing: false },
      },
    },
    catalog: {
      profile: [
        {
          table: { schema: "PUBLIC", name: "STALE_TABLE" },
          kind: "r",
          estimatedRows: 1,
          totalBytes: 8,
        },
      ],
    },
    objects: {
      profile: [
        {
          object: { schema: "PUBLIC", name: "STALE_FUNCTION", kind: "function", identity: null },
          ownerTable: null,
          signature: "() -> void",
          estimatedRows: null,
          totalBytes: null,
        },
      ],
    },
  });

  await useStudio.getState().refreshCatalog("profile");

  expect(client.call).not.toHaveBeenCalled();
  expect(useStudio.getState().catalog.profile).toBeUndefined();
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

function transactionRuntime(sessionId: string): SessionRuntimeState {
  return {
    sessionId,
    transactionMode: "manual",
    transactionState: "inTransaction",
    contextKind: "schema",
    activeContext: "public",
    database: "joinery",
    role: "joinery",
    searchPath: ["public", "pg_catalog"],
  };
}

function sdkRuntime(): SessionRuntimeState {
  return {
    ...transactionRuntime(session.id),
    role: null,
    transactionMode: "autoCommit",
    transactionState: "idle",
    contextFingerprint: "verified-context",
    transactionObservation: {
      status: "observed",
      transactionId: null,
      abortOnError: false,
      usability: "unknown",
    },
  };
}
function useSdkSession() {
  useStudio.setState({
    sessions: {
      profile: {
        ...session,
        engine: "snowflake",
        capabilities: { ...session.capabilities, transactions: false, relationBrowsing: true },
      },
    },
    sessionStates: { session: sdkRuntime() },
    sessionContexts: { session: ["public"] },
    sessionContextRevision: { session: 0 },
  });
}

function transactionControlPlan(): PreparedPlan {
  return {
    id: "typed-boundary-plan",
    sessionId: session.id,
    statements: [{ sql: "BEGIN TRANSACTION", kind: "transaction", missingWhere: false }],
    warnings: [],
    requiresApproval: false,
    requiresPassphrase: false,
    confirmationText: null,
    expiresAt: Date.now() + 60_000,
  };
}
function boundaryResult(
  overrides: Partial<TransactionControlResult> = {},
): TransactionControlResult {
  return {
    action: "begin",
    boundaryOutcome: "acknowledged",
    retired: false,
    error: null,
    state: {
      ...sdkRuntime(),
      transactionState: "inTransaction",
      transactionControlAuthority: {
        status: "observed",
        revision: "after-boundary",
        allowedActions: ["commit", "rollback"],
      },
    },
    ...overrides,
  };
}

test("typed transaction execution blocks competing work, dispatches once, and publishes its final observation without extra reads", async () => {
  useSdkSession();
  let resolve!: (value: TransactionControlResult) => void;
  const response = new Promise<TransactionControlResult>((done) => {
    resolve = done;
  });
  vi.mocked(client.call).mockReturnValue(response);
  const pending = useStudio.getState().executeTransactionControl(transactionControlPlan());
  expect(useStudio.getState().isSessionContextPending(session.id)).toBe(true);
  expect(() => revokeSchemaSelectionBeforeWork(session.id)).toThrow("transaction action");
  expect(await useStudio.getState().refreshSessionContexts(session.id)).toBe(false);
  await expect(
    useStudio.getState().executeTransactionControl(transactionControlPlan()),
  ).rejects.toThrow("transaction action");
  expect(client.call).toHaveBeenCalledExactlyOnceWith("execute_transaction_control", {
    planId: "typed-boundary-plan",
    approval: undefined,
  });
  const result = boundaryResult();
  resolve(result);
  expect(await pending).toEqual(result);
  expect(useStudio.getState().sessionStates.session).toEqual(result.state);
  expect(useStudio.getState().isSessionContextPending(session.id)).toBe(false);
  expect(client.call).toHaveBeenCalledTimes(1);
});

test("rollback-only recovery blocks ordinary work but permits a typed rollback with truthful command evidence", async () => {
  useSdkSession();
  const recovery: SessionRuntimeState = {
    ...sdkRuntime(),
    transactionState: "unknown",
    transactionObservation: { status: "unavailable" },
    transactionControlAuthority: { status: "rollbackOnly", revision: "recovery" },
  };
  useStudio.setState({ sessionStates: { session: recovery } });
  expect(() => revokeSchemaSelectionBeforeWork(session.id)).toThrow("Only rollback recovery");
  const result = boundaryResult({ action: "rollback", state: sdkRuntime() });
  vi.mocked(client.call).mockResolvedValue(result);
  const notify = vi.spyOn(useStudio.getState(), "notify");
  await useStudio.getState().executeTransactionControl(transactionControlPlan());
  expect(client.call).toHaveBeenCalledTimes(1);
  expect(notify).toHaveBeenCalledWith("ROLLBACK acknowledged");
  expect(useStudio.getState().sessionStates.session).toEqual(result.state);
});

test("late acknowledged transaction results cannot resurrect a disconnected session", async () => {
  useSdkSession();
  let resolve!: (value: TransactionControlResult) => void;
  vi.mocked(client.call).mockReturnValue(
    new Promise<TransactionControlResult>((done) => {
      resolve = done;
    }),
  );
  const pending = useStudio.getState().executeTransactionControl(transactionControlPlan());
  useStudio.getState().retireSession(session.id);
  resolve(boundaryResult());
  expect((await pending).boundaryOutcome).toBe("acknowledged");
  expect(useStudio.getState().sessions.profile).toBeUndefined();
  expect(useStudio.getState().sessionStates.session).toBeUndefined();
  expect(useStudio.getState().sessionTransactionPending.session).toBeUndefined();
  expect(client.call).toHaveBeenCalledTimes(1);
});

test("unknown boundary outcomes retire locally without replay or automatic observation", async () => {
  useSdkSession();
  vi.mocked(client.call).mockResolvedValue(
    boundaryResult({ boundaryOutcome: "unknown", retired: false }),
  );
  await useStudio.getState().executeTransactionControl(transactionControlPlan());
  expect(useStudio.getState().sessions.profile).toBeUndefined();
  expect(client.call).toHaveBeenCalledTimes(1);
});

test("delayed prior query state cannot steal a pending boundary's rollback recovery publication", async () => {
  useSdkSession();
  const tabId = useStudio.getState().openQuery("profile");
  const tab = useStudio.getState().tabs.find((item) => item.id === tabId)!;
  let resolve!: (value: TransactionControlResult) => void;
  vi.mocked(client.call).mockReturnValue(
    new Promise<TransactionControlResult>((done) => {
      resolve = done;
    }),
  );
  const pending = useStudio.getState().executeTransactionControl(transactionControlPlan());
  useStudio
    .getState()
    .applyEvents(tabId, [{ type: "sessionState", state: sdkRuntime() }], tab.revision);
  expect(useStudio.getState().sessionStates.session.transactionState).toBe("unknown");
  const result = boundaryResult({
    state: {
      ...sdkRuntime(),
      transactionState: "unknown",
      transactionObservation: { status: "unavailable" },
      transactionControlAuthority: { status: "rollbackOnly", revision: "recovery-after-ack" },
    },
  });
  resolve(result);
  await pending;
  useStudio
    .getState()
    .applyEvents(tabId, [{ type: "sessionState", state: sdkRuntime() }], tab.revision);
  expect(useStudio.getState().sessionStates.session).toEqual(result.state);
  expect(useStudio.getState().sessionMetadataRefreshRequired.session).toBe(true);
  expect(await useStudio.getState().refreshSessionContexts(session.id)).toBe(false);
  expect(client.call).toHaveBeenCalledTimes(1);
});

test("an unsettled query in another owning tab prevents typed control without discarding its completion", async () => {
  useSdkSession();
  const other = useStudio.getState().openQuery("profile");
  useStudio.getState().updateTab(other, { status: "running", runId: "prior-run" });
  const prior = useStudio.getState().tabs.find((tab) => tab.id === other)!;
  useStudio.getState().openQuery("profile");
  await expect(
    useStudio.getState().executeTransactionControl(transactionControlPlan()),
  ).rejects.toThrow("queries to finish");
  expect(client.call).not.toHaveBeenCalled();
  expect(useStudio.getState().tabs.find((tab) => tab.id === other)?.revision).toBe(prior.revision);
  useStudio
    .getState()
    .applyEvents(
      other,
      [{ type: "finished", runId: "prior-run", status: "completed", elapsedMs: 1 }],
      prior.revision,
    );
  expect(useStudio.getState().tabs.find((tab) => tab.id === other)?.status).not.toBe("running");
});

const quotedSchemaTarget = 'Quoted.雪"schema';
function schemaAuthority(overrides: Partial<SessionContextOptions> = {}): SessionContextOptions {
  return {
    state: sdkRuntime(),
    available: ["public", quotedSchemaTarget],
    revision: "full-server-revision-0123456789/opaque-token",
    ...overrides,
  };
}
function seedSchemaAuthority(options = schemaAuthority()) {
  useSdkSession();
  useStudio.setState({
    sessionStates: { session: options.state },
    sessionContexts: { session: options.available },
    sessionContextAuthorities: { session: options },
  });
  return options;
}
function deferredContextOptions() {
  let resolve!: (options: SessionContextOptions) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<SessionContextOptions>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

test("SDK schema selection sends full verified typed authority and revokes it synchronously", async () => {
  const authority = seedSchemaAuthority(
    schemaAuthority({
      state: {
        ...sdkRuntime(),
        database: 'Database.雪"name',
        contextFingerprint: "full-fingerprint/0123456789",
      },
    }),
  );
  const response = schemaAuthority({
    state: {
      ...authority.state,
      activeContext: quotedSchemaTarget,
      searchPath: [quotedSchemaTarget],
      contextFingerprint: "after-switch-fingerprint",
    },
    revision: "new-server-revision",
  });
  const pending = deferredContextOptions();
  vi.mocked(client.call).mockImplementationOnce((() => {
    const current = useStudio.getState();
    expect(current.sessionContextAuthorities.session).toBeUndefined();
    expect(current.sessionContexts.session).toEqual([]);
    expect(current.sessionContextSwitching.session).toBe(true);
    expect(current.sessionStates.session.transactionObservation).toEqual({ status: "unavailable" });
    expect(current.sessionStates.session.transactionState).toBe("unknown");
    expect(current.sessionContextRevision.session).toBe(1);
    expect(current.isSessionContextPending("session")).toBe(true);
    return pending.promise;
  }) as typeof client.call);
  const switching = useStudio.getState().switchSessionContext("session", quotedSchemaTarget);
  expect(client.call).toHaveBeenCalledExactlyOnceWith("switch_session_schema", {
    sessionId: "session",
    request: {
      revision: authority.revision,
      database: authority.state.database,
      schema: quotedSchemaTarget,
      contextFingerprint: authority.state.contextFingerprint,
    },
  });
  pending.resolve(response);
  await switching;
  const current = useStudio.getState();
  expect(current.sessionStates.session).toBe(response.state);
  expect(current.sessionContextAuthorities.session).toBe(response);
  expect(current.sessionContexts.session).toEqual(response.available);
  expect(current.sessionContextRevision.session).toBe(2);
  expect(current.sessionContextSwitching.session).toBe(false);
  expect(current.isSessionContextPending("session")).toBe(false);
});

test("SDK schema selection rejects missing, unavailable, unknown, open or stale authority without IPC", async () => {
  const cases: Array<[string, (options: SessionContextOptions) => void]> = [
    [
      "unavailable",
      (options) => {
        options.state.transactionObservation = { status: "unavailable" };
      },
    ],
    [
      "missing observation",
      (options) => {
        delete options.state.transactionObservation;
      },
    ],
    [
      "unknown",
      (options) => {
        options.state.transactionState = "unknown";
        options.state.transactionMode = "unknown";
      },
    ],
    [
      "open",
      (options) => {
        options.state.transactionState = "inTransaction";
      },
    ],
    [
      "failed",
      (options) => {
        options.state.transactionState = "failed";
      },
    ],
    [
      "opaque transaction",
      (options) => {
        options.state.transactionObservation = {
          status: "observed",
          transactionId: "opaque-TX-A7",
          abortOnError: false,
          usability: "unknown",
        };
      },
    ],
    [
      "not listed",
      (options) => {
        options.available = ["public"];
      },
    ],
    [
      "missing revision",
      (options) => {
        delete options.revision;
      },
    ],
    [
      "empty revision",
      (options) => {
        options.revision = "";
      },
    ],
    [
      "missing fingerprint",
      (options) => {
        delete options.state.contextFingerprint;
      },
    ],
  ];
  for (const [label, mutate] of cases) {
    const options = schemaAuthority();
    mutate(options);
    seedSchemaAuthority(options);
    await expect(
      useStudio.getState().switchSessionContext("session", quotedSchemaTarget),
      label,
    ).rejects.toThrow("Refresh session context");
    expect(client.call, label).not.toHaveBeenCalled();
    expect(useStudio.getState().isSessionContextPending("session"), label).toBe(false);
  }
  seedSchemaAuthority();
  useStudio.setState({ sessionContextAuthorities: {} });
  await expect(
    useStudio.getState().switchSessionContext("session", quotedSchemaTarget),
  ).rejects.toThrow("Refresh session context");
  seedSchemaAuthority();
  useStudio.setState({ sessionStates: { session: { ...sdkRuntime() } } });
  await expect(
    useStudio.getState().switchSessionContext("session", quotedSchemaTarget),
  ).rejects.toThrow("Refresh session context");
  expect(client.call).not.toHaveBeenCalled();
});

test("an admitted SDK schema switch blocks refresh, duplicate selection and new work until settled", async () => {
  seedSchemaAuthority();
  const pending = deferredContextOptions();
  vi.mocked(client.call).mockReturnValueOnce(pending.promise);
  const switching = useStudio.getState().switchSessionContext("session", quotedSchemaTarget);
  expect(await useStudio.getState().refreshSessionContexts("session")).toBe(false);
  await expect(useStudio.getState().refreshSessionContexts("session", true)).rejects.toThrow(
    "Wait for the schema change",
  );
  await expect(useStudio.getState().switchSessionContext("session", "public")).rejects.toThrow(
    "Wait for the schema change",
  );
  expect(() => revokeSchemaSelectionBeforeWork("session")).toThrow("Wait for the schema change");
  expect(client.call).toHaveBeenCalledTimes(1);
  pending.resolve(schemaAuthority());
  await switching;
  expect(useStudio.getState().sessionContextSwitching.session).toBe(false);
  expect(() => revokeSchemaSelectionBeforeWork("session")).not.toThrow();
  expect(useStudio.getState().sessionContextAuthorities.session).toBeUndefined();
  expect(useStudio.getState().sessionContexts.session).toEqual([]);
  await expect(
    useStudio.getState().switchSessionContext("session", quotedSchemaTarget),
  ).rejects.toThrow("Refresh session context");
  expect(client.call).toHaveBeenCalledTimes(1);
});

test("SDK refresh without a server revision cannot authorize a subsequent schema selection", async () => {
  seedSchemaAuthority();
  vi.mocked(client.call).mockResolvedValueOnce({
    state: sdkRuntime(),
    available: ["public", quotedSchemaTarget],
  });
  expect(await useStudio.getState().refreshSessionContexts("session", true)).toBe(true);
  expect(useStudio.getState().sessionContextAuthorities.session).toBeUndefined();
  await expect(
    useStudio.getState().switchSessionContext("session", quotedSchemaTarget),
  ).rejects.toThrow("Refresh session context");
  expect(client.call).toHaveBeenCalledExactlyOnceWith("session_contexts", { sessionId: "session" });
  expect(useStudio.getState().isSessionContextPending("session")).toBe(false);
});

test("a rejected SDK schema switch stays unknown and requires fresh authority before retry", async () => {
  seedSchemaAuthority();
  vi.mocked(client.call).mockRejectedValueOnce({ code: "conflict", message: "Authority expired" });
  await expect(
    useStudio.getState().switchSessionContext("session", quotedSchemaTarget),
  ).rejects.toMatchObject({ code: "conflict" });
  expect(useStudio.getState().sessions.profile.id).toBe("session");
  expect(useStudio.getState().sessionStates.session.transactionState).toBe("unknown");
  expect(useStudio.getState().sessionContextSwitching.session).toBe(false);
  expect(useStudio.getState().sessionContexts.session).toEqual([]);
  expect(useStudio.getState().sessionContextAuthorities.session).toBeUndefined();
  await expect(
    useStudio.getState().switchSessionContext("session", quotedSchemaTarget),
  ).rejects.toThrow("Refresh session context");
  expect(client.call).toHaveBeenCalledTimes(1);
  const fresh = schemaAuthority({ revision: "fresh-after-rejection" });
  vi.mocked(client.call).mockResolvedValueOnce(fresh);
  expect(await useStudio.getState().refreshSessionContexts("session", true)).toBe(true);
  expect(useStudio.getState().sessionContextAuthorities.session).toBe(fresh);
  expect(useStudio.getState().isSessionContextPending("session")).toBe(false);
});

for (const newerRefresh of [false, true]) {
  for (const lateFailure of [false, true]) {
    test(`work invalidation prevents old SDK refresh ${lateFailure ? "failure" : "success"} publishing${newerRefresh ? " over a newer refresh" : " authority"}`, async () => {
      seedSchemaAuthority();
      const oldResponse = deferredContextOptions();
      vi.mocked(client.call).mockReturnValueOnce(oldResponse.promise);
      const old = useStudio.getState().refreshSessionContexts("session", true);
      expect(useStudio.getState().sessionContextAuthorities.session).toBeUndefined();
      expect(useStudio.getState().isSessionContextPending("session")).toBe(true);
      revokeSchemaSelectionBeforeWork("session");
      const observedAfterWork = { ...sdkRuntime(), contextFingerprint: "observed-after-work" };
      useStudio.setState({ sessionStates: { session: observedAfterWork } });
      const latest = schemaAuthority({
        state: { ...sdkRuntime(), contextFingerprint: "new-refresh" },
        revision: "new-revision",
      });
      if (newerRefresh) {
        vi.mocked(client.call).mockResolvedValueOnce(latest);
        expect(await useStudio.getState().refreshSessionContexts("session", true)).toBe(true);
        // The old promise remains pending even though a new request completed.
        expect(useStudio.getState().isSessionContextPending("session")).toBe(true);
      }
      if (lateFailure) oldResponse.reject(new Error("obsolete observation denied"));
      else oldResponse.resolve(schemaAuthority());
      expect(await old).toBe(false);
      const current = useStudio.getState();
      expect(current.sessionStates.session).toBe(newerRefresh ? latest.state : observedAfterWork);
      expect(current.sessionContextAuthorities.session).toBe(newerRefresh ? latest : undefined);
      expect(current.sessionContexts.session).toEqual(newerRefresh ? latest.available : []);
      expect(current.isSessionContextPending("session")).toBe(false);
    });
  }
}

for (const lateFailure of [false, true]) {
  test(`retirement prevents pending SDK schema switch ${lateFailure ? "failure" : "success"} from publishing`, async () => {
    seedSchemaAuthority();
    const response = deferredContextOptions();
    vi.mocked(client.call).mockReturnValueOnce(response.promise);
    const switching = useStudio.getState().switchSessionContext("session", quotedSchemaTarget);
    useStudio.getState().retireSession("session");
    expect(useStudio.getState().isSessionContextPending("session")).toBe(false);
    if (lateFailure) response.reject({ code: "outcomeUnknown", message: "obsolete switch" });
    else response.resolve(schemaAuthority());
    await switching;
    const current = useStudio.getState();
    expect(current.sessions.profile).toBeUndefined();
    expect(current.sessionStates.session).toBeUndefined();
    expect(current.sessionContexts.session).toBeUndefined();
    expect(current.sessionContextAuthorities.session).toBeUndefined();
    expect(current.sessionContextSwitching.session).toBeUndefined();
  });
}

for (const code of ["outcomeUnknown", "connectionClosed"] as const) {
  test(`SDK schema switch ${code} retires the session without follow-up IPC`, async () => {
    seedSchemaAuthority();
    vi.mocked(client.call).mockRejectedValueOnce({ code, message: "switch could not be verified" });
    await expect(
      useStudio.getState().switchSessionContext("session", quotedSchemaTarget),
    ).rejects.toMatchObject({ code });
    const current = useStudio.getState();
    expect(current.sessions.profile).toBeUndefined();
    expect(current.sessionStates.session).toBeUndefined();
    expect(current.sessionContextAuthorities.session).toBeUndefined();
    expect(current.sessionContextSwitching.session).toBeUndefined();
    expect(current.isSessionContextPending("session")).toBe(false);
    expect(client.call).toHaveBeenCalledTimes(1);
  });
}

test("legacy schema selection retains its existing command and needs no SDK authority", async () => {
  const response = {
    state: { ...transactionRuntime("session"), activeContext: "analytics" },
    available: ["public", "analytics"],
  };
  vi.mocked(client.call).mockResolvedValueOnce(response);
  await useStudio.getState().switchSessionContext("session", "analytics");
  expect(client.call).toHaveBeenCalledExactlyOnceWith("switch_session_context", {
    sessionId: "session",
    target: "analytics",
  });
  expect(useStudio.getState().sessionStates.session).toEqual(response.state);
  expect(useStudio.getState().sessionContexts.session).toEqual(response.available);
  expect(useStudio.getState().sessionContextAuthorities.session).toBeUndefined();
  expect(useStudio.getState().sessionContextSwitching.session).toBe(false);
  expect(useStudio.getState().isSessionContextPending("session")).toBe(false);
});

test("legacy schema selection blocks refresh and competing work while its shared gate is pending", async () => {
  const pending = deferredContextOptions();
  vi.mocked(client.call).mockReturnValueOnce(pending.promise);
  const switching = useStudio.getState().switchSessionContext("session", "analytics");

  expect(useStudio.getState().sessionContextSwitching.session).toBe(true);
  expect(useStudio.getState().isSessionContextPending("session")).toBe(true);
  expect(await useStudio.getState().refreshSessionContexts("session")).toBe(false);
  await expect(useStudio.getState().switchSessionContext("session", "reporting")).rejects.toThrow(
    "Wait for the schema change",
  );
  expect(() => revokeSchemaSelectionBeforeWork("session")).toThrow("Wait for the schema change");
  expect(client.call).toHaveBeenCalledExactlyOnceWith("switch_session_context", {
    sessionId: "session",
    target: "analytics",
  });

  const response = {
    state: { ...transactionRuntime("session"), activeContext: "analytics" },
    available: ["public", "analytics"],
  };
  pending.resolve(response);
  await switching;
  expect(useStudio.getState().sessionContextSwitching.session).toBe(false);
  expect(useStudio.getState().isSessionContextPending("session")).toBe(false);
  expect(useStudio.getState().sessionStates.session).toEqual(response.state);
});

test("template observation returns a current context without publishing it or restoring authority", async () => {
  const originalRuntime = sdkRuntime();
  seedSchemaAuthority({
    ...schemaAuthority(),
    state: originalRuntime,
  });
  useStudio.setState({ sessionContextRevision: { session: 7 } });
  const response = deferredContextOptions();
  vi.mocked(client.call).mockReturnValueOnce(response.promise);

  const observing = observeTemplateSessionContext("session");
  const during = useStudio.getState();
  expect(during.sessionContextAuthorities.session).toBeUndefined();
  expect(during.sessionContexts.session).toEqual([]);
  expect(during.sessionStates.session).toBe(originalRuntime);
  expect(during.sessionContextRevision.session).toBe(7);
  expect(during.isSessionContextPending("session")).toBe(true);
  expect(client.call).toHaveBeenCalledExactlyOnceWith("session_contexts", { sessionId: "session" });

  const observed = {
    ...sdkRuntime(),
    activeContext: null,
    database: "different-database",
  };
  response.resolve({ state: observed, available: ["new-schema"], revision: "must-not-publish" });
  await expect(observing).resolves.toBe(observed);
  const after = useStudio.getState();
  expect(after.sessionStates.session).toBe(originalRuntime);
  expect(after.sessionContexts.session).toEqual([]);
  expect(after.sessionContextAuthorities.session).toBeUndefined();
  expect(after.sessionContextRevision.session).toBe(7);
  expect(after.isSessionContextPending("session")).toBe(false);
});

test("a superseded template observation cannot replace newer refresh state or ownership", async () => {
  seedSchemaAuthority();
  const oldResponse = deferredContextOptions();
  vi.mocked(client.call).mockReturnValueOnce(oldResponse.promise);
  const oldObservation = observeTemplateSessionContext("session");
  const latest = schemaAuthority({
    state: { ...sdkRuntime(), activeContext: "new-schema", contextFingerprint: "latest" },
    available: ["new-schema"],
    revision: "latest-revision",
  });
  vi.mocked(client.call).mockResolvedValueOnce(latest);

  expect(await useStudio.getState().refreshSessionContexts("session", true)).toBe(true);
  expect(useStudio.getState().isSessionContextPending("session")).toBe(true);
  expect(useStudio.getState().sessionContextAuthorities.session).toBe(latest);
  oldResponse.resolve(
    schemaAuthority({
      state: { ...sdkRuntime(), activeContext: "obsolete-schema" },
      available: ["obsolete-schema"],
      revision: "obsolete-revision",
    }),
  );
  await expect(oldObservation).rejects.toThrow("The console context changed");

  const current = useStudio.getState();
  expect(current.sessionStates.session).toBe(latest.state);
  expect(current.sessionContexts.session).toEqual(latest.available);
  expect(current.sessionContextAuthorities.session).toBe(latest);
  expect(current.isSessionContextPending("session")).toBe(false);
});

test("work invalidation rejects a late template observation without restoring authority", async () => {
  seedSchemaAuthority();
  const originalRuntime = useStudio.getState().sessionStates.session;
  const response = deferredContextOptions();
  vi.mocked(client.call).mockReturnValueOnce(response.promise);
  const observing = observeTemplateSessionContext("session");
  expect(useStudio.getState().isSessionContextPending("session")).toBe(true);

  revokeSchemaSelectionBeforeWork("session");
  expect(useStudio.getState().sessionContextAuthorities.session).toBeUndefined();
  expect(useStudio.getState().sessionContexts.session).toEqual([]);
  expect(useStudio.getState().isSessionContextPending("session")).toBe(true);
  response.resolve(
    schemaAuthority({
      state: { ...sdkRuntime(), activeContext: "late-schema" },
      available: ["late-schema"],
      revision: "late-revision",
    }),
  );
  await expect(observing).rejects.toThrow("The console context changed");

  const current = useStudio.getState();
  expect(current.sessionStates.session).toBe(originalRuntime);
  expect(current.sessionContexts.session).toEqual([]);
  expect(current.sessionContextAuthorities.session).toBeUndefined();
  expect(current.isSessionContextPending("session")).toBe(false);
});

test("retirement rejects a late template observation without recreating session state", async () => {
  seedSchemaAuthority();
  const response = deferredContextOptions();
  vi.mocked(client.call).mockReturnValueOnce(response.promise);
  const observing = observeTemplateSessionContext("session");
  expect(useStudio.getState().isSessionContextPending("session")).toBe(true);

  useStudio.getState().retireSession("session");
  expect(useStudio.getState().isSessionContextPending("session")).toBe(false);
  response.resolve(
    schemaAuthority({
      state: { ...sdkRuntime(), activeContext: "late-schema" },
      available: ["late-schema"],
    }),
  );
  await expect(observing).rejects.toThrow("The console context changed");
  expect(useStudio.getState().sessions.profile).toBeUndefined();
  expect(useStudio.getState().sessionStates.session).toBeUndefined();
  expect(useStudio.getState().sessionContexts.session).toBeUndefined();
  expect(useStudio.getState().sessionContextAuthorities.session).toBeUndefined();
  expect(useStudio.getState().isSessionContextPending("session")).toBe(false);
});

test("template observation is rejected during a schema switch without issuing another IPC", async () => {
  seedSchemaAuthority();
  const switchingResponse = deferredContextOptions();
  vi.mocked(client.call).mockReturnValueOnce(switchingResponse.promise);
  const switching = useStudio.getState().switchSessionContext("session", quotedSchemaTarget);

  await expect(observeTemplateSessionContext("session")).rejects.toThrow(
    "Wait for the schema change",
  );
  expect(useStudio.getState().sessionContextSwitching.session).toBe(true);
  expect(useStudio.getState().isSessionContextPending("session")).toBe(true);
  expect(client.call).toHaveBeenCalledExactlyOnceWith("switch_session_schema", {
    sessionId: "session",
    request: expect.any(Object),
  });
  switchingResponse.resolve(schemaAuthority());
  await switching;
  expect(useStudio.getState().sessionContextSwitching.session).toBe(false);
  expect(useStudio.getState().isSessionContextPending("session")).toBe(false);
  expect(client.call).toHaveBeenCalledTimes(1);
});
test("unknown or missing session transaction observation requires close confirmation", () => {
  expect(sessionHasOpenTransaction({}, "session")).toBe(true);
  expect(
    sessionHasOpenTransaction(
      { session: { ...sdkRuntime(), transactionState: "unknown" } },
      "session",
    ),
  ).toBe(true);
  expect(sessionHasOpenTransaction({ session: sdkRuntime() }, "session")).toBe(false);
  expect(sessionHasOpenTransaction({}, null)).toBe(false);
});
test("failed SDK observation replaces old success with unknown and clears choices", async () => {
  useSdkSession();
  vi.mocked(client.call).mockRejectedValueOnce(new Error("observation denied"));
  const refresh = useStudio.getState().refreshSessionContexts("session");
  expect(useStudio.getState().sessionStates.session.transactionState).toBe("unknown");
  expect(useStudio.getState().sessionContexts.session).toEqual([]);
  expect(await refresh).toBe(false);
  expect(useStudio.getState().sessionStates.session).toMatchObject({
    transactionMode: "unknown",
    transactionState: "unknown",
    role: null,
    database: "",
    activeContext: null,
    transactionObservation: { status: "unavailable" },
  });
  expect(useStudio.getState().sessionContextRevision.session).toBe(1);
});
test("stable SDK refresh preserves the context revision and nullable-role changes advance it", async () => {
  useSdkSession();
  vi.mocked(client.call).mockResolvedValueOnce({
    state: sdkRuntime(),
    available: ["public"],
    revision: "same-context-new-options",
  });
  expect(await useStudio.getState().refreshSessionContexts("session")).toBe(true);
  expect(useStudio.getState().sessionContextRevision.session).toBe(0);
  for (const role of ["NEW_ROLE", null]) {
    vi.mocked(client.call).mockResolvedValueOnce({
      state: { ...sdkRuntime(), role },
      available: ["public"],
      revision: "new-options",
    });
    await useStudio.getState().refreshSessionContexts("session");
    expect(useStudio.getState().sessionStates.session.role).toBe(role);
  }
  expect(useStudio.getState().sessionContextRevision.session).toBe(2);
});
test("retiring an SDK session invalidates its pending observation", async () => {
  useSdkSession();
  let resolve!: (value: unknown) => void;
  vi.mocked(client.call).mockReturnValueOnce(
    new Promise((yes) => {
      resolve = yes;
    }),
  );
  const pending = useStudio.getState().refreshSessionContexts("session");
  useStudio.getState().retireSession("session");
  resolve({ state: sdkRuntime(), available: ["public"] });
  expect(await pending).toBe(false);
  expect(useStudio.getState().sessionStates.session).toBeUndefined();
  expect(useStudio.getState().sessionContexts.session).toBeUndefined();
});
for (const lateFailure of [false, true]) {
  test(`older SDK observation ${lateFailure ? "failure" : "success"} cannot replace newer state`, async () => {
    useSdkSession();
    let resolve!: (value: unknown) => void;
    let reject!: (error: Error) => void;
    vi.mocked(client.call).mockReturnValueOnce(
      new Promise((yes, no) => {
        resolve = yes;
        reject = no;
      }),
    );
    const old = useStudio.getState().refreshSessionContexts("session", true);
    const newest = { ...sdkRuntime(), activeContext: "newest", contextFingerprint: "newest" };
    vi.mocked(client.call).mockResolvedValueOnce({
      state: newest,
      available: ["newest"],
      revision: "new-revision",
    });
    expect(await useStudio.getState().refreshSessionContexts("session")).toBe(true);
    if (lateFailure) reject(new Error("obsolete error"));
    else resolve({ state: sdkRuntime(), available: ["public"] });
    expect(await old).toBe(false);
    expect(useStudio.getState().sessionStates.session).toEqual(newest);
    expect(useStudio.getState().sessionContexts.session).toEqual(["newest"]);
  });
}
test("a query state event supersedes a pending observation without losing its result", async () => {
  useSdkSession();
  const tabId = useStudio.getState().openQuery("profile");
  const tab = useStudio.getState().tabs.find((tab) => tab.id === tabId)!;
  let resolve!: (value: unknown) => void;
  vi.mocked(client.call).mockReturnValueOnce(
    new Promise((yes) => {
      resolve = yes;
    }),
  );
  const old = useStudio.getState().refreshSessionContexts("session");
  const observed = {
    ...sdkRuntime(),
    transactionState: "inTransaction" as const,
    transactionObservation: {
      status: "observed" as const,
      transactionId: "opaque:18446744073709551616",
      abortOnError: true,
      usability: "unknown" as const,
    },
  };
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "sessionState", state: observed },
      { type: "statementCompleted", runId: "known-run", index: 0, affectedRows: "1", elapsedMs: 2 },
    ],
    tab.revision,
  );
  resolve({ state: sdkRuntime(), available: ["public"] });
  expect(await old).toBe(false);
  expect(useStudio.getState().sessionStates.session).toEqual(observed);
  expect(useStudio.getState().tabs.find((tab) => tab.id === tabId)!.metrics[0].affectedRows).toBe(
    "1",
  );
});
test("obsolete query state events cannot restore a successful SDK observation", () => {
  useSdkSession();
  const tabId = useStudio.getState().openQuery("profile");
  const tab = useStudio.getState().tabs.find((tab) => tab.id === tabId)!;
  useStudio.getState().updateTab(tabId, { revision: tab.revision + 1 });
  const unknown = {
    ...sdkRuntime(),
    transactionState: "unknown" as const,
    transactionMode: "unknown" as const,
    transactionObservation: { status: "unavailable" as const },
  };
  useStudio.setState({ sessionStates: { session: unknown } });
  useStudio
    .getState()
    .applyEvents(tabId, [{ type: "sessionState", state: sdkRuntime() }], tab.revision);
  expect(useStudio.getState().sessionStates.session).toEqual(unknown);
});

for (const action of ["commitTransaction", "rollbackTransaction"] as const) {
  for (const ownership of ["primary", "secondary"] as const) {
    test.each(["connectionClosed", "outcomeUnknown"] as const)(
      `${action} retires only its ${ownership} session on %s and preserves the original error`,
      async (code) => {
        const first = useStudio.getState().openQuery("profile");
        const second = useStudio.getState().openQuery("profile");
        const unaffected = useStudio.getState().openQuery("profile");
        const targetId = ownership === "primary" ? session.id : "target-secondary";
        const unaffectedId = ownership === "primary" ? "other-secondary" : session.id;
        if (ownership === "secondary") {
          useStudio.getState().bindQuerySession(first, { ...session, id: targetId });
          useStudio.getState().attachQuerySession(second, targetId);
        } else {
          useStudio.getState().bindQuerySession(unaffected, { ...session, id: unaffectedId });
        }
        useStudio.setState({
          sessionStates: {
            [targetId]: transactionRuntime(targetId),
            [unaffectedId]: transactionRuntime(unaffectedId),
          },
          sessionContexts: { [targetId]: ["public"], [unaffectedId]: ["public"] },
          sessionContextRevision: { [targetId]: 7, [unaffectedId]: 3 },
          toast: "existing notice",
        });
        const error = { code, message: "Check the database before retrying this transaction" };
        vi.mocked(client.call).mockRejectedValueOnce(error);

        await expect(useStudio.getState()[action](targetId)).rejects.toBe(error);

        const state = useStudio.getState();
        for (const tabId of [first, second]) {
          const tab = state.tabs.find((entry) => entry.id === tabId)!;
          expect(querySessionForTab(tab, state.sessions)).toBeUndefined();
          if (ownership === "secondary") expect(tab.querySessionClosed).toBe(true);
        }
        expect(
          querySessionForTab(
            state.tabs.find((entry) => entry.id === unaffected)!,
            state.sessions,
          )?.id,
        ).toBe(unaffectedId);
        expect(state.sessionStates[targetId]).toBeUndefined();
        expect(state.sessionContexts[targetId]).toBeUndefined();
        expect(state.sessionContextRevision[targetId]).toBeUndefined();
        expect(state.sessionStates[unaffectedId]).toEqual(transactionRuntime(unaffectedId));
        expect(state.sessionContextRevision[unaffectedId]).toBe(3);
        expect(state.toast).toBe("existing notice");
      },
    );
  }

  test.each(["database", "conflict", "connectionFailed"] as const)(
    `${action} preserves a usable transaction on recoverable %s errors`,
    async (code) => {
      const runtime = transactionRuntime(session.id);
      useStudio.setState({ sessionStates: { [session.id]: runtime }, toast: "existing notice" });
      const error = { code, message: "Transaction command was rejected" };
      vi.mocked(client.call).mockRejectedValueOnce(error);
      if (code !== "connectionFailed")
        vi.mocked(client.call).mockResolvedValueOnce({ state: runtime, available: ["public"] });

      await expect(useStudio.getState()[action](session.id)).rejects.toBe(error);

      expect(useStudio.getState().sessions.profile).toEqual(session);
      expect(useStudio.getState().sessionStates[session.id]).toEqual(runtime);
      expect(useStudio.getState().toast).toBe("existing notice");
    },
  );

  test.each(["database", "conflict"] as const)(
    `${action} reconciles rejected transaction state and invalidates changed identity for %s`,
    async (code) => {
      const before = {
        ...transactionRuntime(session.id),
        activeContext: "temporary",
        searchPath: ["temporary"],
      };
      const observed = { ...transactionRuntime(session.id), transactionState: "idle" as const };
      useStudio.setState({
        sessionStates: { [session.id]: before },
        sessionContextRevision: { [session.id]: 4 },
        toast: "existing notice",
      });
      const error = { code, message: "deferred constraint rejected COMMIT" };
      vi.mocked(client.call)
        .mockRejectedValueOnce(error)
        .mockResolvedValueOnce({ state: observed, available: ["public"] });
      await expect(useStudio.getState()[action](session.id)).rejects.toBe(error);
      expect(useStudio.getState().sessionStates[session.id]).toEqual(observed);
      expect(useStudio.getState().sessionContextRevision[session.id]).toBe(5);
      expect(useStudio.getState().sessions.profile).toEqual(session);
      expect(useStudio.getState().toast).toBe("existing notice");
      expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).toEqual([
        action === "commitTransaction" ? "commit_transaction" : "rollback_transaction",
        "session_contexts",
      ]);
      vi.mocked(client.call).mockResolvedValueOnce({ state: observed, available: ["public"] });
      expect(await useStudio.getState().refreshSessionContexts(session.id, true)).toBe(true);
      expect(useStudio.getState().sessionContextRevision[session.id]).toBe(5);
    },
  );

  test.each(["connectionClosed", "outcomeUnknown", "conflict", "connectionFailed"] as const)(
    `${action} preserves the original rejection when runtime refresh fails with %s`,
    async (code) => {
      const runtime = transactionRuntime(session.id);
      useStudio.setState({ sessionStates: { [session.id]: runtime } });
      const error = { code: "database", message: "original constraint error" };
      vi.mocked(client.call)
        .mockRejectedValueOnce(error)
        .mockRejectedValueOnce({ code, message: "refresh failed" });
      await expect(useStudio.getState()[action](session.id)).rejects.toBe(error);
      if (code === "connectionClosed" || code === "outcomeUnknown") {
        expect(useStudio.getState().sessions.profile).toBeUndefined();
        expect(useStudio.getState().sessionStates[session.id]).toBeUndefined();
      } else {
        expect(useStudio.getState().sessions.profile).toEqual(session);
        expect(useStudio.getState().sessionStates[session.id]).toEqual(runtime);
      }
    },
  );

  test(`${action} ignores success arriving after its session was retired`, async () => {
    let complete!: (runtime: SessionRuntimeState) => void;
    vi.mocked(client.call).mockImplementationOnce(
      () => new Promise((resolve) => (complete = resolve)),
    );
    useStudio.setState({
      sessionStates: { [session.id]: transactionRuntime(session.id) },
      toast: "Reconnect before continuing",
    });
    const pending = useStudio.getState()[action](session.id);
    useStudio.getState().retireSession(session.id);
    complete({ ...transactionRuntime(session.id), transactionState: "idle" });
    await pending;

    expect(useStudio.getState().sessionStates[session.id]).toBeUndefined();
    expect(useStudio.getState().toast).toBe("Reconnect before continuing");
  });
}

test("session context fingerprints invalidate completion revisions independently", () => {
  const tabId = useStudio.getState().openQuery("profile");
  const base = transactionRuntime("session");
  useStudio.setState({
    sessionStates: { session: base },
    sessionContextRevision: { session: 7 },
  });

  const emit = (runtime: SessionRuntimeState) => {
    const revision = useStudio.getState().tabs.find((tab) => tab.id === tabId)!.revision;
    useStudio.getState().applyEvents(tabId, [{ type: "sessionState", state: runtime }], revision);
  };

  // Older drivers can omit the fingerprint; explicit null means the same state.
  emit({ ...base, contextFingerprint: null } as unknown as SessionRuntimeState);
  expect(useStudio.getState().sessionContextRevision.session).toBe(7);

  // Schema, warehouse, policy, and secondary-role differences are represented
  // by the backend digest even when the legacy fields above stay identical.
  emit({ ...base, contextFingerprint: "schema=SALES;warehouse=WH_A;secondary=ALL" });
  expect(useStudio.getState().sessionContextRevision.session).toBe(8);
  emit({ ...base, contextFingerprint: "schema=SALES;warehouse=WH_A;secondary=ALL" });
  expect(useStudio.getState().sessionContextRevision.session).toBe(8);
  emit({ ...base, contextFingerprint: "schema=SALES;warehouse=WH_B;secondary=ALL" });
  expect(useStudio.getState().sessionContextRevision.session).toBe(9);
  emit({ ...base, contextFingerprint: "schema=SALES;warehouse=WH_B;secondary=NONE" });
  expect(useStudio.getState().sessionContextRevision.session).toBe(10);
  emit({ ...base, contextFingerprint: "schema=FINANCE;warehouse=WH_B;secondary=NONE" });
  expect(useStudio.getState().sessionContextRevision.session).toBe(11);
});

test("file-backed SQL tabs track dirty state, release grants on Save As and close, and never persist to workspace drafts", async () => {
  useStudio.setState({
    bootstrap: {
      profiles: [{ id: "profile", storeDrafts: true } as ConnectionProfile],
      savedQueries: [],
      bookmarks: [],
    } as unknown as Bootstrap,
  });
  let saveCalls = 0;
  vi.mocked(client.call).mockImplementation(((command: string) => {
    if (command === "open_sql_file") {
      return Promise.resolve({
        token: "sql-grant-1",
        fileName: "report.sql",
        content: "SELECT 1;",
      });
    }
    if (command === "save_sql_file") {
      saveCalls += 1;
      return Promise.resolve(
        saveCalls === 1
          ? { token: "sql-grant-1", fileName: "report.sql" }
          : { token: "sql-grant-2", fileName: "report-v2.sql" },
      );
    }
    return Promise.resolve(null);
  }) as typeof client.call);

  const regularId = useStudio.getState().openQuery("profile", "SELECT 'draft'");
  const fileTabId = (await useStudio.getState().openSqlFile("profile"))!;
  expect(fileTabId).toBeTruthy();

  let fileTab = useStudio.getState().tabs.find((t) => t.id === fileTabId)!;
  expect(isTabFileDirty(fileTab)).toBe(false);
  expect(workspaceSnapshot().drafts.map((d) => d.id)).toEqual([regularId]);

  useStudio.getState().updateTab(fileTabId, { sql: "SELECT 2;" });
  fileTab = useStudio.getState().tabs.find((t) => t.id === fileTabId)!;
  expect(isTabFileDirty(fileTab)).toBe(true);
  expect(workspaceSnapshot().drafts.map((d) => d.id)).toEqual([regularId]);

  // Save in-place reuses `sql-grant-1` and clears dirty state.
  expect(await useStudio.getState().saveSqlFile(fileTabId)).toBe(true);
  expect(client.call).toHaveBeenCalledWith("save_sql_file", {
    token: "sql-grant-1",
    suggestedName: "report.sql",
    content: "SELECT 2;",
  });
  expect(isTabFileDirty(useStudio.getState().tabs.find((t) => t.id === fileTabId)!)).toBe(false);

  // Save As passes `token: null`, rebinds to `sql-grant-2`, and releases `sql-grant-1`.
  expect(await useStudio.getState().saveSqlFile(fileTabId, { saveAs: true })).toBe(true);
  expect(client.call).toHaveBeenCalledWith("save_sql_file", {
    token: null,
    suggestedName: "report.sql",
    content: "SELECT 2;",
  });
  expect(client.call).toHaveBeenCalledWith("release_sql_file_grant", { token: "sql-grant-1" });

  // Closing the tab releases `sql-grant-2`.
  useStudio.getState().closeTab(fileTabId);
  expect(client.call).toHaveBeenCalledWith("release_sql_file_grant", { token: "sql-grant-2" });
});

test("saved queries, bookmarks, recent-locations ring buffer, and profile deletion privacy cleanup", async () => {
  useStudio.setState({
    bootstrap: {
      profiles: [
        {
          id: "local-conn",
          name: "Local DB",
          environment: "local",
          storeQueryText: true,
          storeDrafts: true,
        } as ConnectionProfile,
        {
          id: "prod-conn",
          name: "Prod DB",
          environment: "production",
          storeQueryText: false,
          storeDrafts: false,
        } as ConnectionProfile,
      ],
      savedQueries: [],
      bookmarks: [],
    } as unknown as Bootstrap,
    sessions: { "local-conn": { ...session, connectionId: "local-conn" } },
  });

  let sqSeq = 0;
  let bmSeq = 0;
  vi.mocked(client.call).mockImplementation(((command: string, args?: Record<string, unknown>) => {
    if (command === "save_saved_query") {
      const input = args?.input as {
        id: string | null;
        title: string;
        sql: string;
        connectionId: string | null;
      };
      return Promise.resolve({
        id: input.id ?? `sq-${++sqSeq}`,
        title: input.title,
        sql: input.sql,
        connectionId: input.connectionId,
        updatedAtMs: 1000 + sqSeq,
      });
    }
    if (command === "save_bookmark") {
      return Promise.resolve({
        id: `bm-${++bmSeq}`,
        label: String(args?.label ?? ""),
        target: args?.target,
        createdAtMs: 2000 + bmSeq,
      });
    }
    return Promise.resolve(null);
  }) as typeof client.call);

  const tabId = useStudio.getState().openQuery("local-conn", "SELECT 1");
  const localSq = await useStudio
    .getState()
    .saveQueryToLibrary(
      { id: null, title: "Local check", sql: "SELECT 1", connectionId: "local-conn" },
      tabId,
    );
  const prodSq = await useStudio.getState().saveQueryToLibrary({
    id: null,
    title: "Prod check",
    sql: "SELECT 2",
    connectionId: "prod-conn",
  });

  // Reopening an already-open saved query focuses the existing tab instead of duplicating.
  expect(useStudio.getState().openSavedQuery(localSq.id)).toBe(tabId);

  // Toggle bookmarks on an object and a saved query.
  expect(
    await useStudio.getState().toggleBookmark(
      {
        kind: "object",
        connectionId: "prod-conn",
        object: { schema: "public", name: "audit_log", kind: "table", identity: null },
      },
      "public.audit_log",
    ),
  ).toBe(true);
  expect(
    await useStudio
      .getState()
      .toggleBookmark({ kind: "savedQuery", savedQueryId: prodSq.id }, "Prod check"),
  ).toBe(true);
  expect(useStudio.getState().bookmarks).toHaveLength(2);

  // Opening a location on an offline connection returns `needsConnect` without throwing.
  const offlineOutcome = await useStudio.getState().openLocation({
    kind: "table",
    connectionId: "prod-conn",
    table: { schema: "public", name: "audit_log" },
  });
  expect(offlineOutcome.kind).toBe("needsConnect");

  // Removing a production connection purges its saved queries and bookmarks; removing a local connection detaches its saved query.
  useStudio.getState().removeConnection("prod-conn");
  expect(useStudio.getState().savedQueries.map((q) => q.id)).toEqual([localSq.id]);
  expect(useStudio.getState().bookmarks).toEqual([]);

  useStudio.getState().removeConnection("local-conn");
  expect(useStudio.getState().savedQueries[0]?.connectionId).toBeNull();

  // Recent locations ring buffer caps strictly at 50 items and deduplicates by key.
  for (let i = 0; i < 65; i += 1) {
    useStudio.getState().recordRecentLocation({
      key: `item:${i}`,
      label: `Item ${i}`,
      subtitle: "Fixture",
      target: { kind: "savedQuery", savedQueryId: localSq.id },
    });
  }
  expect(useStudio.getState().recentLocations).toHaveLength(MAX_RECENT_LOCATIONS);
  expect(useStudio.getState().recentLocations[0]?.key).toBe("item:64");
});

test.each([
  {
    status: "stopped",
    summary: { planned: 3, attempted: 1, completed: 1, notAttempted: 2, stopReason: "resultLimit" },
  },
  {
    status: "completed",
    summary: { planned: 1, attempted: 1, completed: 1, notAttempted: 0, stopReason: "resultLimit" },
  },
  {
    status: "cancelled",
    summary: { planned: 3, attempted: 0, completed: 0, notAttempted: 3, stopReason: "cancelled" },
  },
  {
    status: "failed",
    summary: { planned: 3, attempted: 1, completed: 0, notAttempted: 2, stopReason: "failed" },
  },
  {
    status: "outcomeUnknown",
    summary: {
      planned: 3,
      attempted: 1,
      completed: 0,
      notAttempted: 2,
      stopReason: "outcomeUnknown",
    },
  },
] as const)(
  "retains authoritative $status script accounting independently of result/statement-start events",
  ({ status, summary }) => {
    const state = useStudio.getState();
    const id = state.openQuery("profile");
    const revision = useStudio.getState().tabs.find((tab) => tab.id === id)!.revision;
    const retained = {
      ...page(schema("prefix")).result,
      runId: "run",
      statementIndex: 0,
      status: "truncated" as const,
    };
    state.applyEvents(
      id,
      [
        { type: "started", runId: "run" },
        { type: "statementStarted", runId: "run", index: 0 },
      ],
      revision,
    );
    if (summary.attempted > 0)
      state.applyEvents(id, [{ type: "result", result: retained }], revision);
    // statementStarted precedes actual driver dispatch and is not accounting.
    expect(useStudio.getState().tabs[0].executionSummary).toBeUndefined();
    state.applyEvents(
      id,
      [{ type: "finished", runId: "run", status, elapsedMs: 7, executionSummary: summary }],
      revision,
    );
    const tab = useStudio.getState().tabs[0];
    expect(tab.executionSummary).toEqual(summary);
    expect(tab.status).toBe(status);
    expect(tab.results).toEqual(summary.attempted > 0 ? [retained] : []);
    expect(tab.runId).toBeNull();
  },
);

test("foreign/stale finishes cannot replace script accounting and a new legacy finish remains unknown", () => {
  const state = useStudio.getState();
  const id = state.openQuery("profile");
  state.updateTab(id, { revision: 4 });
  const summary: ExecutionSummary = {
    planned: 3,
    attempted: 1,
    completed: 1,
    notAttempted: 2,
    stopReason: "resultLimit",
  };
  state.applyEvents(id, [{ type: "started", runId: "current" }], 4);
  state.applyEvents(
    id,
    [
      {
        type: "finished",
        runId: "foreign",
        status: "stopped",
        elapsedMs: 1,
        executionSummary: summary,
      },
    ],
    4,
  );
  state.applyEvents(
    id,
    [
      {
        type: "finished",
        runId: "current",
        status: "stopped",
        elapsedMs: 1,
        executionSummary: summary,
      },
    ],
    3,
  );
  expect(useStudio.getState().tabs[0]).toMatchObject({ status: "running", runId: "current" });
  expect(useStudio.getState().tabs[0].executionSummary).toBeUndefined();
  state.applyEvents(
    id,
    [
      {
        type: "finished",
        runId: "current",
        status: "stopped",
        elapsedMs: 1,
        executionSummary: summary,
      },
    ],
    4,
  );
  expect(useStudio.getState().tabs[0].executionSummary).toEqual(summary);
  state.applyEvents(id, [{ type: "started", runId: "next" }], 4);
  expect(useStudio.getState().tabs[0].executionSummary).toBeUndefined();
  state.applyEvents(
    id,
    [
      { type: "statementCompleted", runId: "next", index: 0, affectedRows: "99", elapsedMs: 1 },
      { type: "finished", runId: "next", status: "completed", elapsedMs: 1 },
    ],
    4,
  );
  expect(useStudio.getState().tabs[0].executionSummary).toBeUndefined();
});

test("script accounting is ephemeral and cleared when console ownership changes", () => {
  const state = useStudio.getState();
  const id = state.openQuery("profile");
  state.updateTab(id, {
    executionSummary: {
      planned: 3,
      attempted: 1,
      completed: 1,
      notAttempted: 2,
      stopReason: "resultLimit",
    },
  });
  expect(JSON.stringify(workspaceSnapshot())).not.toContain("executionSummary");
  state.bindQuerySession(id, session);
  expect(useStudio.getState().tabs[0].executionSummary).toBeUndefined();
});

test("execution timings retain measured zero, reject foreign/stale runs and stay out of restoration", () => {
  const state = useStudio.getState();
  const id = state.openQuery("profile");
  state.updateTab(id, { revision: 7, clientStartedAt: performance.now() - 100 });
  const metrics = {
    type: "runMetrics",
    runId: "run",
    queueMs: 40,
    activeMs: 0,
    firstRowMs: null,
    totalMs: 40,
  } as const;
  state.applyEvents(
    id,
    [
      { type: "started", runId: "run" },
      { ...metrics, runId: "other" },
    ],
    7,
  );
  expect(useStudio.getState().tabs[0].runMetrics).toBeUndefined();
  state.applyEvents(id, [metrics], 6);
  expect(useStudio.getState().tabs[0].runMetrics).toBeUndefined();
  state.applyEvents(
    id,
    [metrics, { type: "finished", runId: "run", status: "completed", elapsedMs: 40 }],
    7,
  );
  const tab = useStudio.getState().tabs[0];
  expect(tab.runMetrics?.activeMs).toBe(0);
  expect(tab.runMetrics?.firstRowMs).toBeNull();
  expect(tab.clientElapsedMs).toBeGreaterThanOrEqual(100);
  const persisted = JSON.stringify(workspaceSnapshot());
  expect(persisted).not.toContain("runMetrics");
  expect(persisted).not.toContain("clientStartedAt");
  expect(persisted).not.toContain("clientElapsedMs");
  state.bindQuerySession(id, session);
  expect(useStudio.getState().tabs[0].runMetrics).toBeUndefined();
  expect(useStudio.getState().tabs[0].clientElapsedMs).toBeUndefined();
});

test("console Structure tabs keep their session and never browse table data", () => {
  const state = useStudio.getState();
  const id = state.openQuery("profile");
  state.bindQuerySession(id, { ...session, id: "isolated" });
  const object = { schema: "public", name: "orders", kind: "table" as const, identity: null };
  state.openSessionObject(id, object, "id");
  let inspector = useStudio.getState().tabs.at(-1)!;
  expect(inspector.kind).toBe("object");
  expect(inspector.metadataOrigin).toEqual({
    tabId: id,
    sessionId: "isolated",
    contextRevision: 0,
  });
  expect(inspector.highlightedColumn).toBe("id");
  expect(client.call).not.toHaveBeenCalled();
  state.openSessionObject(id, object, "customer_id");
  expect(useStudio.getState().tabs).toHaveLength(2);
  inspector = useStudio.getState().tabs.at(-1)!;
  expect(inspector.highlightedColumn).toBe("customer_id");
  useStudio.setState({ sessionContextRevision: { isolated: 1 } });
  state.openSessionObject(id, object);
  expect(useStudio.getState().tabs).toHaveLength(3);
});

test("metadata loses its session after context changes, rebinding, or source closure", () => {
  const state = useStudio.getState();
  const id = state.openQuery("profile");
  state.bindQuerySession(id, { ...session, id: "isolated" });
  state.openSessionObject(
    id,
    { schema: "public", name: "orders", kind: "table", identity: null },
    "id",
    "observed",
  );
  const inspector = useStudio.getState().tabs.at(-1)!;
  expect(metadataSessionForTab(inspector, useStudio.getState())?.id).toBe("isolated");
  expect(querySessionForTab(inspector, state.sessions)).toBeUndefined();
  useStudio.setState({ sessionContextRevision: { isolated: 1 } });
  expect(metadataSessionForTab(inspector, useStudio.getState())).toBeUndefined();
  useStudio.setState({ sessionContextRevision: {} });
  state.attachQuerySession(id, null);
  expect(metadataSessionForTab(inspector, useStudio.getState())).toBeUndefined();
  state.bindQuerySession(id, { ...session, id: "isolated" });
  state.closeTab(id);
  expect(metadataSessionForTab(inspector, useStudio.getState())).toBeUndefined();
});

test("linked inspectors retain their origin and append DDL without replacing SQL or session ownership", () => {
  const state = useStudio.getState();
  const id = state.openQuery("profile", "SELECT 'unfinished work'; -- keep this");
  state.bindQuerySession(id, { ...session, id: "isolated" });
  state.updateTab(id, {
    fileToken: "grant",
    fileName: "work.sql",
    savedSql: "SELECT 'unfinished work';",
  });
  state.openSessionObject(
    id,
    { schema: "public", name: "orders", kind: "table", identity: null },
    undefined,
    "observed",
  );
  const first = useStudio.getState().tabs.at(-1)!;
  state.openLinkedSessionObject(first.id, {
    schema: "public",
    name: "orders_pkey",
    kind: "index",
    identity: "orders",
  });
  const linked = useStudio.getState().tabs.at(-1)!;
  expect(linked.metadataOrigin).toEqual(first.metadataOrigin);
  expect(linked.querySession).toBeNull();
  expect(state.appendSessionDdl(linked.id, "CREATE INDEX example ON orders(id);")).toBe(true);
  const source = useStudio.getState().tabs.find((t) => t.id === id)!;
  expect(source.sql).toBe(
    "SELECT 'unfinished work'; -- keep this\n\nCREATE INDEX example ON orders(id);",
  );
  expect(source.querySession?.id).toBe("isolated");
  expect(isTabFileDirty(source)).toBe(true);
  expect(useStudio.getState().activeTabId).toBe(id);
  expect(useStudio.getState().tabs).toHaveLength(3);
  expect(client.call).not.toHaveBeenCalled();
});

test.each(["rebound", "context", "closed", "running"])(
  "inspector actions reject a %s source without primary fallback",
  (change) => {
    const state = useStudio.getState();
    const id = state.openQuery("profile", "SELECT 42;");
    state.bindQuerySession(id, { ...session, id: "isolated" });
    state.openSessionObject(
      id,
      { schema: "public", name: "orders", kind: "table", identity: null },
      undefined,
      "observed",
    );
    const inspector = useStudio.getState().tabs.at(-1)!;
    if (change === "rebound") state.attachQuerySession(id, null);
    if (change === "context") useStudio.setState({ sessionContextRevision: { isolated: 1 } });
    if (change === "closed") state.closeTab(id);
    if (change === "running") state.updateTab(id, { status: "running" });
    const before = useStudio.getState().tabs;
    expect(state.appendSessionDdl(inspector.id, "DROP TABLE orders;")).toBe(false);
    if (change !== "running") {
      state.openLinkedSessionObject(inspector.id, {
        schema: "public",
        name: "customers",
        kind: "table",
        identity: null,
      });
      expect(useStudio.getState().tabs).toEqual(before);
    }
    expect(useStudio.getState().tabs.find((t) => t.id === id)?.sql ?? "SELECT 42;").toBe(
      "SELECT 42;",
    );
    expect(client.call).not.toHaveBeenCalledWith("prepare_execution", expect.anything());
  },
);

test("togglePinTab groups pinned tabs first and closeOtherTabs preserves pinned and dirty tabs", async () => {
  const id1 = useStudio.getState().openQuery("profile", "SELECT 1;");
  const id2 = useStudio.getState().openQuery("profile", "SELECT 2;");
  const id3 = useStudio.getState().openQuery("profile", "SELECT 3;");
  const id4 = useStudio.getState().openQuery("profile", "SELECT 4;");

  // Pin tab 3 -> it moves to the front of the tab strip
  useStudio.getState().togglePinTab(id3);
  expect(useStudio.getState().tabs.map((t) => t.id)).toEqual([id3, id1, id2, id4]);
  expect(useStudio.getState().tabs[0].pinned).toBe(true);

  // Mark tab 4 as a dirty SQL file tab
  useStudio.getState().updateTab(id4, {
    fileName: "dirty.sql",
    savedSql: "SELECT 0;",
    sql: "SELECT 4 -- modified;",
  });

  // Close other tabs keeping id2 -> id3 (pinned) and id4 (dirty) stay open, id1 closes
  const closedCount = useStudio.getState().closeOtherTabs(id2);
  expect(closedCount).toBe(1);
  expect(useStudio.getState().tabs.map((t) => t.id)).toEqual([id3, id2, id4]);
  expect(useStudio.getState().activeTabId).toBe(id2);
  expect(useStudio.getState().toast).toMatch(/Kept 1 tab/i);

  // Reopen closed tab restores id1
  const reopenedId = await useStudio.getState().reopenClosedTab();
  expect(reopenedId).toBe(id1);
  expect(useStudio.getState().tabs.map((t) => t.id)).toEqual([id3, id2, id4, id1]);
  expect(useStudio.getState().activeTabId).toBe(id1);
});

test("closedTabs stack is bounded to 10 entries and purged when connection is removed", () => {
  const openedIds: string[] = [];
  for (let i = 0; i < 12; i += 1) {
    openedIds.push(useStudio.getState().openQuery("profile", `SELECT ${i};`));
  }
  for (const id of openedIds) {
    useStudio.getState().closeTab(id);
  }
  expect(useStudio.getState().closedTabs).toHaveLength(10);
  expect(useStudio.getState().closedTabs[0].id).toBe(openedIds[11]);

  useStudio.getState().removeConnection("profile");
  expect(useStudio.getState().closedTabs).toHaveLength(0);
});

test("editable query result tracks pending edits in dirtyStore and cleans up when released", () => {
  vi.mocked(client.call).mockResolvedValue(null);
  const tabId = useStudio.getState().openQuery("profile");
  const editableResult = {
    ...page(schema("status")).result,
    id: "query-res-1",
    editability: {
      status: "editable" as const,
      table: { schema: "public", name: "orders" },
      primaryKey: ["id"],
    },
  };
  useStudio.getState().updateTab(tabId, {
    results: [editableResult],
    activeResultId: editableResult.id,
    dataSessionId: session.id,
  });

  dirtyStore.set({
    resultId: editableResult.id,
    row: 0,
    column: "c0",
    original: "pending",
    value: "completed",
  });
  dirtyStore.addInsertRow(editableResult.id);
  expect(dirtyStore.pendingCountForTab(editableResult.id, [editableResult.id])).toBe(2);

  useStudio.getState().closeTab(tabId);
  expect(dirtyStore.pendingCountForTab(editableResult.id, [editableResult.id])).toBe(0);
});

function dmlPlan(): PreparedPlan {
  return {
    ...transactionControlPlan(),
    id: "reviewed-dml",
    reviewedSnowflakeDml: true,
    statements: [
      { sql: "UPDATE DB.PUBLIC.T SET A = 1 WHERE ID = 2", kind: "write", missingWhere: false },
    ],
  };
}
function dmlResult(overrides: Partial<SnowflakeDmlResult> = {}): SnowflakeDmlResult {
  return {
    kind: "update",
    statementOutcome: "succeeded",
    summary: { status: "validated", affectedRows: "9007199254740993", multiJoinedRows: "7" },
    state: boundaryResult().state,
    retired: false,
    error: null,
    ...overrides,
  };
}

test("reviewed DML dispatches once, preserves results and exact counts, and holds automatic metadata until explicit refresh", async () => {
  useSdkSession();
  const tabId = useStudio.getState().openQuery("profile");
  const prior = page(schema("prior")).result;
  useStudio.getState().updateTab(tabId, { results: [prior], activeResultId: prior.id });
  let resolve!: (value: SnowflakeDmlResult) => void;
  vi.mocked(client.call).mockReturnValue(
    new Promise<SnowflakeDmlResult>((done) => {
      resolve = done;
    }),
  );
  const pending = useStudio.getState().executeSnowflakeDml(dmlPlan(), "approval", tabId);
  expect(useStudio.getState().tabs.find((tab) => tab.id === tabId)?.status).toBe("running");
  expect(await useStudio.getState().refreshSessionContexts(session.id)).toBe(false);
  await expect(useStudio.getState().executeSnowflakeDml(dmlPlan())).rejects.toThrow("finish first");
  expect(client.call).toHaveBeenCalledExactlyOnceWith("execute_snowflake_dml", {
    planId: "reviewed-dml",
    approval: "approval",
  });
  resolve(dmlResult());
  await pending;
  const tab = useStudio.getState().tabs.find((tab) => tab.id === tabId)!;
  expect(tab.status).toBe("completed");
  expect(tab.results).toEqual([prior]);
  expect(tab.snowflakeDmlResult?.summary.affectedRows).toBe("9007199254740993");
  expect(useStudio.getState().sessionStates.session).toEqual(dmlResult().state);
  expect(await useStudio.getState().refreshSessionContexts(session.id)).toBe(false);
  expect(client.call).toHaveBeenCalledTimes(1);
  vi.mocked(client.call).mockResolvedValue({
    state: sdkRuntime(),
    available: ["public"],
    revision: "explicit",
  });
  expect(await useStudio.getState().refreshSessionContexts(session.id, true)).toBe(true);
  expect(client.call).toHaveBeenLastCalledWith("session_contexts", {
    sessionId: session.id,
    resumeMetadata: true,
  });
  expect(useStudio.getState().sessionMetadataRefreshRequired.session).toBe(false);
});

test("reviewed DML rejects rollback-only and competing queries before invalidating their revisions", async () => {
  useSdkSession();
  const tabId = useStudio.getState().openQuery("profile");
  useStudio.getState().updateTab(tabId, { status: "running" });
  const revision = useStudio.getState().tabs[0].revision;
  await expect(useStudio.getState().executeSnowflakeDml(dmlPlan(), null, tabId)).rejects.toThrow(
    "queries to finish",
  );
  expect(useStudio.getState().tabs[0].revision).toBe(revision);
  useStudio.getState().updateTab(tabId, { status: "idle" });
  useStudio.setState({
    sessionStates: {
      session: {
        ...sdkRuntime(),
        transactionControlAuthority: { status: "rollbackOnly", revision: "recovery" },
      },
    },
  });
  await expect(useStudio.getState().executeSnowflakeDml(dmlPlan(), null, tabId)).rejects.toThrow(
    "Only rollback recovery",
  );
  expect(client.call).not.toHaveBeenCalled();
});

test("late DML ACK cannot resurrect a retired session or overwrite a repurposed tab", async () => {
  useSdkSession();
  const tabId = useStudio.getState().openQuery("profile");
  let resolve!: (value: SnowflakeDmlResult) => void;
  vi.mocked(client.call).mockReturnValue(
    new Promise<SnowflakeDmlResult>((done) => {
      resolve = done;
    }),
  );
  const pending = useStudio.getState().executeSnowflakeDml(dmlPlan(), null, tabId);
  useStudio.getState().retireSession(session.id);
  useStudio.getState().updateTab(tabId, {
    revision: useStudio.getState().tabs[0].revision + 1,
    status: "idle",
    sql: "SELECT later",
  });
  const notify = vi.spyOn(useStudio.getState(), "notify");
  resolve(dmlResult({ retired: true }));
  expect((await pending).statementOutcome).toBe("succeeded");
  expect(useStudio.getState().sessions.profile).toBeUndefined();
  expect(useStudio.getState().sessionStates.session).toBeUndefined();
  expect(useStudio.getState().tabs[0].status).toBe("idle");
  expect(useStudio.getState().tabs[0].snowflakeDmlResult).toBeUndefined();
  expect(notify).toHaveBeenCalledWith(
    expect.stringContaining("UPDATE succeeded; affected rows 9007199254740993; session retired"),
  );
});

test("unknown DML retires without replay and stale query observations cannot replace rollback recovery", async () => {
  for (const unknown of [false, true]) {
    useSdkSession();
    const tabId = useStudio.getState().openQuery("profile");
    const revision = useStudio.getState().tabs.find((tab) => tab.id === tabId)!.revision;
    const state: SessionRuntimeState = {
      ...sdkRuntime(),
      transactionObservation: { status: "unavailable" },
      transactionControlAuthority: { status: "rollbackOnly", revision: "recovery-dml" },
    };
    vi.mocked(client.call).mockResolvedValue(
      dmlResult({
        state,
        statementOutcome: unknown ? "unknown" : "succeeded",
        summary: { status: "unavailable", affectedRows: null, multiJoinedRows: null },
      }),
    );
    const result = await useStudio.getState().executeSnowflakeDml(dmlPlan(), null, tabId);
    useStudio
      .getState()
      .applyEvents(tabId, [{ type: "sessionState", state: sdkRuntime() }], revision);
    if (unknown) {
      expect(result.retired).toBe(true);
      expect(useStudio.getState().sessions.profile).toBeUndefined();
    } else expect(useStudio.getState().sessionStates.session).toEqual(state);
    expect(useStudio.getState().sessionTransactionPending.session).toBeUndefined();
    useStudio.setState({ tabs: [] });
  }
  expect(client.call).toHaveBeenCalledTimes(2);
});

function ddlPlan(): PreparedPlan {
  return {
    ...transactionControlPlan(),
    id: "reviewed-ddl",
    reviewedSnowflakeDdl: true,
    statements: [
      { sql: 'DROP TABLE "DB"."PUBLIC"."T" RESTRICT', kind: "ddl", missingWhere: false },
    ],
  };
}
function ddlResult(overrides: Partial<SnowflakeDdlResult> = {}): SnowflakeDdlResult {
  return {
    kind: "dropTable",
    qualifiedTarget: '"DB"."PUBLIC"."T"',
    statementOutcome: "succeeded",
    summaryStatus: "acknowledged",
    catalogReconciliationRequired: true,
    state: sdkRuntime(),
    retired: false,
    error: null,
    ...overrides,
  };
}
test("DDL preserves prior results, blocks competing work, and only explicit catalog reconciliation releases metadata", async () => {
  useSdkSession();
  const tabId = useStudio.getState().openQuery("profile");
  const prior = page(schema("before_ddl")).result;
  useStudio.getState().updateTab(tabId, { results: [prior], activeResultId: prior.id });
  let finish!: (value: SnowflakeDdlResult) => void;
  vi.mocked(client.call).mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  const pending = useStudio.getState().executeSnowflakeDdl(ddlPlan(), "approval", tabId);
  expect(useStudio.getState().sessionDdlCatalogRefreshRequired.session).toBe(true);
  expect(await useStudio.getState().refreshSessionContexts(session.id)).toBe(false);
  await expect(useStudio.getState().executeSnowflakeDdl(ddlPlan())).rejects.toThrow("finish first");
  expect(client.call).toHaveBeenCalledExactlyOnceWith("execute_snowflake_ddl", {
    planId: "reviewed-ddl",
    approval: "approval",
  });
  finish(ddlResult());
  await pending;
  expect(useStudio.getState().tabs[0].results).toEqual([prior]);
  expect(useStudio.getState().tabs[0].snowflakeDdlResult?.statementOutcome).toBe("succeeded");
  vi.mocked(client.call).mockRejectedValueOnce(new Error("Catalog denied"));
  await expect(useStudio.getState().refreshSessionContexts(session.id, true)).rejects.toThrow(
    "Catalog denied",
  );
  expect(useStudio.getState().sessionDdlCatalogRefreshRequired.session).toBe(true);
  vi.mocked(client.call).mockResolvedValueOnce({
    state: sdkRuntime(),
    available: ["public"],
    revision: "reconciled",
  });
  expect(await useStudio.getState().refreshSessionContexts(session.id, true)).toBe(true);
  expect(client.call).toHaveBeenLastCalledWith("refresh_snowflake_ddl_metadata", {
    sessionId: session.id,
  });
  expect(useStudio.getState().sessionDdlCatalogRefreshRequired.session).toBe(false);
  expect(useStudio.getState().sessionMetadataRefreshRequired.session).toBe(false);
});
test("unknown DDL retires once without replay and late ACK cannot overwrite a repurposed tab", async () => {
  useSdkSession();
  const tabId = useStudio.getState().openQuery("profile");
  let finish!: (value: SnowflakeDdlResult) => void;
  vi.mocked(client.call).mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  const pending = useStudio.getState().executeSnowflakeDdl(ddlPlan(), "approval", tabId);
  useStudio.getState().updateTab(tabId, {
    revision: useStudio.getState().tabs[0].revision + 1,
    sql: "SELECT later",
    status: "idle",
  });
  finish(ddlResult({ statementOutcome: "unknown", summaryStatus: "unavailable", retired: true }));
  await pending;
  expect(useStudio.getState().sessions.profile).toBeUndefined();
  expect(useStudio.getState().tabs[0].status).toBe("idle");
  expect(useStudio.getState().tabs[0].snowflakeDdlResult).toBeUndefined();
  expect(client.call).toHaveBeenCalledTimes(1);
});

test("DDL IPC outcomeUnknown stays unknown and never replays or observes", async () => {
  useSdkSession();
  const tabId = useStudio.getState().openQuery("profile");
  vi.mocked(client.call).mockRejectedValue({
    code: "outcomeUnknown",
    message: "Helper lost after dispatch",
  });
  await expect(
    useStudio.getState().executeSnowflakeDdl(ddlPlan(), "approval", tabId),
  ).rejects.toMatchObject({ code: "outcomeUnknown" });
  expect(useStudio.getState().tabs[0].status).toBe("outcomeUnknown");
  expect(useStudio.getState().sessions.profile).toBeUndefined();
  expect(client.call).toHaveBeenCalledTimes(1);
});
test("session reattachment and reconnect clear transient DDL notices", () => {
  useSdkSession();
  const tabId = useStudio.getState().openQuery("profile");
  const fresh = { ...session, id: "fresh-session" };
  useStudio.setState({
    sessionPool: { profile: [{ info: fresh, label: "Fresh session", primary: true }] },
  });
  useStudio.getState().updateTab(tabId, {
    snowflakeDdlResult: ddlResult({ retired: true }),
    querySession: { ...session, id: "old-session" },
    querySessionClosed: true,
  });
  expect(useStudio.getState().attachQuerySession(tabId, fresh.id)).toBe(true);
  expect(useStudio.getState().tabs[0].snowflakeDdlResult).toBeUndefined();
  useStudio.getState().updateTab(tabId, { snowflakeDdlResult: ddlResult() });
  useStudio.getState().resetConnectionResults("profile");
  expect(useStudio.getState().tabs[0].snowflakeDdlResult).toBeUndefined();
});

test("Snowflake session context catalog switches role, warehouse, database and schema with one-use authority and preserves session on pre-ACK SQL rejection", async () => {
  useSdkSession();
  const initialState: SessionRuntimeState = {
    ...sdkRuntime(),
    database: "DB",
    role: "ROLE",
    warehouse: "WH",
    activeContext: "PUBLIC",
    contextFingerprint: "fp-initial",
  };
  vi.mocked(client.call).mockResolvedValueOnce({
    state: initialState,
    availableRoles: ["ANALYST", "ROLE"],
    availableWarehouses: ["LARGE_WH", "WH"],
    availableDatabases: ["ANALYTICS_DB", "DB"],
    availableSchemas: ["OTHER", "PUBLIC"],
    revision: "ctx-rev-1",
  });
  expect(await useStudio.getState().refreshSnowflakeSessionContexts(session.id, true)).toBe(true);
  expect(useStudio.getState().snowflakeContextExpanded[session.id]).toBe(true);
  expect(useStudio.getState().snowflakeContextAuthorities[session.id]?.revision).toBe("ctx-rev-1");

  const switchedWarehouseState: SessionRuntimeState = {
    ...initialState,
    warehouse: "LARGE_WH",
    contextFingerprint: "fp-large-wh",
  };
  vi.mocked(client.call).mockResolvedValueOnce({
    state: switchedWarehouseState,
    availableRoles: ["ANALYST", "ROLE"],
    availableWarehouses: ["LARGE_WH", "WH"],
    availableDatabases: ["ANALYTICS_DB", "DB"],
    availableSchemas: ["OTHER", "PUBLIC"],
    revision: "ctx-rev-2",
  });
  await useStudio
    .getState()
    .switchSnowflakeSessionContext(session.id, { kind: "warehouse", warehouse: "LARGE_WH" });
  expect(client.call).toHaveBeenLastCalledWith("switch_snowflake_context", {
    sessionId: session.id,
    request: {
      revision: "ctx-rev-1",
      contextFingerprint: "fp-initial",
      target: { kind: "warehouse", warehouse: "LARGE_WH" },
    },
  });
  expect(useStudio.getState().sessionStates[session.id]?.warehouse).toBe("LARGE_WH");

  // Pre-ACK database error rejects the switch without retiring the SSO session, while consuming the revision.
  vi.mocked(client.call).mockRejectedValueOnce({
    code: "database",
    message: "Snowflake rejected the context switch",
  });
  await expect(
    useStudio
      .getState()
      .switchSnowflakeSessionContext(session.id, { kind: "role", role: "ANALYST" }),
  ).rejects.toMatchObject({ code: "database" });
  expect(useStudio.getState().sessions.profile?.id).toBe(session.id);
  expect(useStudio.getState().snowflakeContextAuthorities[session.id]).toBeUndefined();

  // Subsequent refreshSessionContexts automatically uses snowflake_session_contexts once expanded.
  vi.mocked(client.call).mockResolvedValueOnce({
    state: switchedWarehouseState,
    availableRoles: ["ANALYST", "ROLE"],
    availableWarehouses: ["LARGE_WH", "WH"],
    availableDatabases: ["ANALYTICS_DB", "DB"],
    availableSchemas: ["OTHER", "PUBLIC"],
    revision: "ctx-rev-3",
  });
  expect(await useStudio.getState().refreshSessionContexts(session.id, true)).toBe(true);
  expect(client.call).toHaveBeenLastCalledWith("snowflake_session_contexts", {
    sessionId: session.id,
  });

  // connectionClosed retires the session and purges snowflakeContextAuthorities + snowflakeContextExpanded.
  vi.mocked(client.call).mockRejectedValueOnce({
    code: "connectionClosed",
    message: "Unverified post-ACK readback",
  });
  await expect(
    useStudio.getState().switchSnowflakeSessionContext(session.id, {
      kind: "database",
      database: "ANALYTICS_DB",
    }),
  ).rejects.toMatchObject({ code: "connectionClosed" });
  expect(useStudio.getState().sessions.profile).toBeUndefined();
  expect(useStudio.getState().snowflakeContextAuthorities[session.id]).toBeUndefined();
  expect(useStudio.getState().snowflakeContextExpanded[session.id]).toBeUndefined();
});
