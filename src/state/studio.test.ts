import { beforeEach, expect, test, vi } from "vitest";
import { client } from "../ipc/client";
import type { SessionInfo, TablePage, TableSchema } from "../ipc/generated";
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
    sessions: { profile: session },
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
    workspace: {
      theme: "contrast",
      density: "compact",
      sidebarVisible: true,
      inspectorVisible: false,
      activeDraftId: savedId,
      drafts: [{ id: savedId, connectionId, title: "Recovered", sql: "SELECT 'recovered'" }],
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
  const query = { sort: [{ column: "label", descending: true }], filters: [] };
  expect(await useStudio.getState().setTableQuery(id, query)).toBeNull();
  const result = useStudio.getState().tabs.find((tab) => tab.id === id)!;
  expect(result.query).toEqual(query);
  expect(result.pages).toHaveLength(1);
  expect(client.call).toHaveBeenLastCalledWith("browse_table", {
    sessionId: "session",
    table: source.table,
    after: null,
    query,
  });
  expect(client.call).toHaveBeenCalledWith("release_result", { resultId: "result" });
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
  const reason = await useStudio
    .getState()
    .setTableQuery(id, { sort: [], filters: [{ column: "label", op: "isNull", value: null }] });
  expect(reason).toContain("pending edits");
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
  const filtered = { sort: [], filters: [{ column: "id", op: "eq" as const, value: "42" }] };
  await useStudio.getState().openTable("profile", table, filtered);
  await useStudio.getState().openTable("profile", table, filtered);
  const tabs = useStudio.getState().tabs;
  expect(tabs).toHaveLength(2);
  expect(tabs[1].title).toBe("fixture · id = 42");
  expect(useStudio.getState().activeTabId).toBe(tabs[1].id);
});
