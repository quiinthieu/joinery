import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CompletionCatalog, SessionInfo, TableSchema } from "../../ipc/generated";
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

it("withholds DDL insertion when no originating console action is available", async () => {
  const { tab } = setup();
  vi.mocked(client.call).mockImplementation(async (method) =>
    method === "completion_catalog"
      ? catalog
      : method === "completion_schema"
        ? schema
        : {
            object: tab.object,
            ownerTable: null,
            comment: null,
            ddl: { status: "available", ddl: "CREATE TABLE orders (id bigint)", truncated: false },
            constraints: [],
            indexes: [],
            triggers: [],
            routine: null,
            sequence: null,
            uses: [],
            usedBy: [],
          },
  );
  render(<SessionStructureView tab={{ ...tab, view: "ddl" }} />);
  await screen.findByRole("button", { name: "Copy DDL" });
  expect(screen.queryByRole("button", { name: "Open in SQL editor" })).toBeNull();
});
