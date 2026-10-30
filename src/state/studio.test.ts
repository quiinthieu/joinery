import { beforeEach, expect, test, vi } from "vitest";
import { client } from "../ipc/client";
import type { SessionInfo, TablePage, TableSchema } from "../ipc/generated";
import type { Bootstrap, ConnectionProfile } from "../ipc/generated";
import { useStudio } from "./studio";
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
  useStudio.setState({ tabs: [], activeTabId: null, sessions: { profile: session } });
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
