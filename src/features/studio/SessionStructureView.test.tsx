import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type {
  CompletionCatalog,
  ObjectDetails,
  SessionInfo,
  TableSchema,
} from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import { SessionStructureView } from "./SessionStructureView";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (e: unknown) => String(e instanceof Error ? e.message : e),
}));
const session: SessionInfo = {
  id: "isolated",
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
    objectExplorer: true,
  },
};
const catalog: CompletionCatalog = {
  revision: "observed",
  identity: {
    database: "fixture",
    role: "reader",
    searchPath: ["public"],
    caseSensitiveTableNames: true,
  },
  tables: [],
  unqualifiedTables: [],
};
const schema: TableSchema = {
  table: { schema: "public", name: "orders" },
  columns: [
    {
      id: "id",
      name: "isolated_id",
      databaseType: "bigint",
      kind: "integer",
      nullable: false,
      primaryKey: true,
      foreignKey: false,
      editable: false,
      insertable: false,
      hasDefault: true,
    },
  ],
  primaryKey: ["isolated_id"],
  foreignKeys: [],
  editable: false,
};
function setup() {
  const state = useStudio.getState();
  const id = state.openQuery("profile");
  state.bindQuerySession(id, session);
  state.openSessionObject(
    id,
    { schema: "public", name: "orders", kind: "table", identity: null },
    "isolated_id",
    "observed",
  );
  return { sourceId: id, tab: useStudio.getState().tabs.at(-1)! };
}
beforeEach(() => {
  vi.clearAllMocks();
  useStudio.setState({
    tabs: [],
    sessions: { profile: { ...session, id: "primary" } },
    sessionPool: {},
    sessionContextRevision: {},
  });
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "completion_catalog") return catalog;
    if (method === "completion_schema") return schema;
    if (method === "object_details") throw new Error("fixture metadata denied");
    throw new Error(`Unexpected ${method}`);
  });
});
afterEach(cleanup);
it("reads only the originating session and preserves columns when extended details are denied", async () => {
  const { tab } = setup();
  render(<SessionStructureView tab={tab} />);
  await screen.findByText("isolated_id");
  expect(screen.getByRole("alert").textContent).toContain("fixture metadata denied");
  expect(document.querySelector('tr[data-highlighted="true"]')).toBeTruthy();
  expect(client.call).toHaveBeenCalledWith("completion_schema", {
    sessionId: "isolated",
    table: schema.table,
    revision: "observed",
  });
  for (const [, args] of vi.mocked(client.call).mock.calls)
    expect(args).toMatchObject({ sessionId: "isolated" });
});
it("rejects a context changed before the metadata request", async () => {
  const { tab } = setup();
  vi.mocked(client.call).mockResolvedValue({ ...catalog, revision: "changed" });
  render(<SessionStructureView tab={tab} />);
  expect((await screen.findByRole("alert")).textContent).toContain("Console context changed");
  expect(client.call).toHaveBeenCalledTimes(1);
});
it("discards metadata if the observed context changes during the request", async () => {
  const { tab } = setup();
  let catalogs = 0;
  vi.mocked(client.call).mockImplementation(async (method) =>
    method === "completion_catalog"
      ? { ...catalog, revision: ++catalogs === 1 ? "observed" : "changed" }
      : method === "completion_schema"
        ? schema
        : Promise.reject(new Error("denied")),
  );
  render(<SessionStructureView tab={tab} />);
  expect((await screen.findByRole("alert")).textContent).toContain("Console context changed");
  expect(screen.queryByText("isolated_id")).toBeNull();
});
it.each(["closed", "context changed", "rebound"])(
  "rejects a late schema when the source is %s",
  async (condition) => {
    const { tab, sourceId } = setup();
    let release!: (s: TableSchema) => void;
    const held = new Promise<TableSchema>((resolve) => {
      release = resolve;
    });
    vi.mocked(client.call).mockImplementation(async (method) =>
      method === "completion_schema" ? held : catalog,
    );
    render(<SessionStructureView tab={tab} />);
    await waitFor(() =>
      expect(client.call).toHaveBeenCalledWith("completion_schema", expect.anything()),
    );
    act(() => {
      if (condition === "closed") useStudio.getState().closeTab(sourceId);
      else if (condition === "context changed")
        useStudio.setState({ sessionContextRevision: { isolated: 1 } });
      else useStudio.getState().attachQuerySession(sourceId, null);
    });
    await act(async () => {
      release(schema);
      await held;
    });
    expect(screen.getByText("Console context unavailable")).toBeTruthy();
    expect(screen.queryByText("isolated_id")).toBeNull();
    expect(client.call).not.toHaveBeenCalledWith("object_details", expect.anything());
  },
);

const ddl = "CREATE TABLE orders (id bigint)";
function detailsFor(object: ObjectDetails["object"]): ObjectDetails {
  return {
    object,
    ownerTable: object.kind === "index" ? schema.table : null,
    comment: null,
    ddl: { status: "available", ddl, truncated: false },
    constraints: [
      {
        name: "orders_customer_fk",
        kind: "foreignKey",
        columns: ["customer_id"],
        target: { schema: "public", name: "customers" },
        targetColumns: ["id"],
        definition: "FOREIGN KEY",
      },
    ],
    indexes: [
      {
        name: "orders_pkey",
        method: "btree",
        unique: true,
        primary: true,
        expressions: ["id"],
        predicate: null,
        definition: null,
      },
    ],
    triggers: [
      {
        name: "orders_trigger",
        tableName: "orders",
        timing: "AFTER",
        events: ["INSERT"],
        orientation: "ROW",
        enabled: true,
        definition: null,
      },
    ],
    routine: null,
    sequence: null,
    uses: [
      {
        object: { schema: "public", name: "customer_view", kind: "view", identity: null },
        relation: "reference",
      },
    ],
    usedBy: [],
  };
}
function allowDetails() {
  vi.mocked(client.call).mockImplementation(async (method, args) =>
    method === "completion_catalog"
      ? catalog
      : method === "completion_schema"
        ? schema
        : detailsFor((args as { object: ObjectDetails["object"] }).object),
  );
}
it("appends displayed DDL to the originating console without execution or replacing its SQL", async () => {
  const { tab, sourceId } = setup();
  useStudio.getState().updateTab(sourceId, { sql: "SELECT 'keep';" });
  allowDetails();
  render(<SessionStructureView tab={{ ...tab, view: "ddl" }} />);
  fireEvent.click(await screen.findByRole("button", { name: "Append DDL to source console" }));
  await waitFor(() => expect(useStudio.getState().activeTabId).toBe(sourceId));
  expect(useStudio.getState().tabs.find((t) => t.id === sourceId)?.sql).toBe(
    "SELECT 'keep';\n\n" + ddl,
  );
  expect(screen.queryByRole("button", { name: "Open in SQL editor" })).toBeNull();
  expect(
    vi
      .mocked(client.call)
      .mock.calls.every(([method]) =>
        ["completion_catalog", "completion_schema", "object_details"].includes(method),
      ),
  ).toBe(true);
});
it.each([
  ["foreign key", "public.customers (id)", "customers", "table"],
  ["index", "orders_pkey", "orders_pkey", "index"],
  ["trigger", "orders_trigger", "orders_trigger", "trigger"],
  ["dependency", "public.customer_view reference", "customer_view", "view"],
  ["owner", "public.orders", "orders", "table"],
])("follows a %s link on the frozen console origin", async (kind, label, name, objectKind) => {
  const { tab, sourceId } = setup();
  if (kind === "owner")
    tab.object = { schema: "public", name: "orders_pkey", kind: "index", identity: "orders" };
  allowDetails();
  render(<SessionStructureView tab={tab} />);
  fireEvent.click(
    kind === "dependency"
      ? await screen.findByTitle("Open view public.customer_view (reference)")
      : await screen.findByRole("button", { name: label }),
  );
  await waitFor(() =>
    expect(useStudio.getState().tabs.at(-1)?.object).toMatchObject({ name, kind: objectKind }),
  );
  expect(useStudio.getState().tabs.at(-1)?.metadataOrigin).toEqual(tab.metadataOrigin);
  expect(useStudio.getState().tabs.at(-1)?.metadataOrigin?.tabId).toBe(sourceId);
  expect(
    vi
      .mocked(client.call)
      .mock.calls.every(([, args]) => (args as { sessionId: string }).sessionId === "isolated"),
  ).toBe(true);
});
it.each([
  "catalog changed",
  "denied",
  "rebound",
  "closed",
  "context changed",
  "inspector refreshed",
])("rejects an in-flight action after %s", async (condition) => {
  const { tab, sourceId } = setup();
  useStudio.getState().updateTab(sourceId, { sql: "SELECT 'keep';" });
  allowDetails();
  render(<SessionStructureView tab={{ ...tab, view: "ddl" }} />);
  const button = await screen.findByRole("button", { name: "Append DDL to source console" });
  let release!: (value: CompletionCatalog) => void;
  let reject!: (error: Error) => void;
  const held = new Promise<CompletionCatalog>((resolve, fail) => {
    release = resolve;
    reject = fail;
  });
  vi.mocked(client.call).mockImplementation(async (method) =>
    method === "completion_catalog" ? held : schema,
  );
  fireEvent.click(button);
  act(() => {
    if (condition === "rebound") useStudio.getState().attachQuerySession(sourceId, null);
    if (condition === "closed") useStudio.getState().closeTab(sourceId);
    if (condition === "context changed")
      useStudio.setState({ sessionContextRevision: { isolated: 1 } });
    if (condition === "inspector refreshed")
      useStudio.getState().updateTab(tab.id, { revision: tab.revision + 1 });
  });
  await act(async () => {
    if (condition === "denied") reject(new Error("context access denied"));
    else
      release({ ...catalog, revision: condition === "catalog changed" ? "changed" : "observed" });
    await held.catch(() => {});
  });
  expect(useStudio.getState().activeTabId).toBe(tab.id);
  expect(useStudio.getState().tabs.find((t) => t.id === sourceId)?.sql ?? "SELECT 'keep';").toBe(
    "SELECT 'keep';",
  );
  if (condition === "catalog changed")
    expect(useStudio.getState().toast).toContain("Console context changed");
  if (condition === "denied") expect(useStudio.getState().toast).toBe("context access denied");
});
