import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { StrictMode } from "react";
import type {
  BrowseCatalog,
  ChangePreview,
  ColumnInfo,
  ConnectionProfile,
  RelationCheckConstraints,
  RelationConstraintMappings,
  RelationConstraints,
  RelationDefinition,
  RelationInspection,
  RelationPreview,
  SessionContextOptions,
  SessionInfo,
  SnowflakeGridResult,
} from "../../ipc/generated";
import { client } from "../../ipc/client";
import { RelationBrowser } from "./RelationBrowser";
import { useStudio } from "../../state/studio";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (e: unknown) => String(e instanceof Error ? e.message : e),
}));
const catalog: BrowseCatalog = {
  revision: "catalog-1",
  database: "DB",
  identity: { database: "DB", role: "reader", searchPath: ["S"], caseSensitiveTableNames: true },
  relations: [
    {
      targetId: "opaque-target",
      database: "DB",
      table: { schema: "S", name: "Quoted Table" },
      tableType: "BASE TABLE",
      traits: {
        temporary: false,
        transient: false,
        iceberg: false,
        dynamic: false,
        hybrid: false,
        immutable: false,
      },
      availability: "available",
    },
  ],
};
const column: ColumnInfo = {
  id: "c0",
  name: "exact",
  databaseType: "NUMBER(38,0)",
  kind: "integer",
  nullable: true,
  primaryKey: false,
  foreignKey: false,
  editable: false,
  insertable: false,
  hasDefault: false,
};
const inspection: RelationInspection = {
  revision: "inspection-1",
  targetId: "opaque-target",
  database: "DB",
  table: catalog.relations[0].table,
  constraints: "notFetched",
  columns: [
    {
      name: "exact",
      ordinal: 1,
      fullType: "NUMBER(38,0)",
      dataType: "NUMBER",
      dataTypeAlias: null,
      kind: "column",
      resultKind: "integer",
      nullable: true,
      defaultExpression: null,
      descriptorDefaultExpression: null,
      writeDefaultExpression: null,
      writeDefaultStatus: "notReported",
      comment: null,
      characterMaximumLength: null,
      characterOctetLength: null,
      numericPrecision: "38",
      numericPrecisionRadix: "10",
      numericScale: "0",
      identity: {
        isIdentity: false,
        generation: null,
        start: null,
        increment: null,
        ordered: null,
      },
      expression: null,
    },
  ],
};
const preview: RelationPreview = {
  columns: [column],
  rows: [["9007199254740993"], [null], [""]],
  truncated: true,
};
const definition: RelationDefinition = {
  revision: catalog.revision,
  targetId: catalog.relations[0].targetId,
  database: catalog.database,
  table: catalog.relations[0].table,
  ddl: 'CREATE OR REPLACE TABLE "DB"."S"."Quoted Table" ("EXACT" NUMBER(38,0));',
};
const keyConstraints: RelationConstraints = {
  revision: catalog.revision,
  targetId: catalog.relations[0].targetId,
  database: catalog.database,
  table: catalog.relations[0].table,
  constraints: [
    {
      catalog: "DB",
      schema: "S",
      name: "PK table",
      kind: "primaryKey",
      isDeferrable: false,
      initiallyDeferred: false,
      enforced: false,
      rely: true,
      comment: "Declared primary key",
      referenceStatus: "notApplicable",
      reference: null,
    },
    {
      catalog: "DB",
      schema: "S",
      name: "UQ table",
      kind: "unique",
      isDeferrable: true,
      initiallyDeferred: true,
      enforced: true,
      rely: false,
      comment: null,
      referenceStatus: "notApplicable",
      reference: null,
    },
    {
      catalog: "DB",
      schema: "S",
      name: 'FK "parent',
      kind: "foreignKey",
      isDeferrable: false,
      initiallyDeferred: false,
      enforced: false,
      rely: false,
      comment: "Visible reference",
      referenceStatus: "available",
      reference: {
        catalog: "OTHER",
        schema: "P",
        name: 'PK "parent',
        matchOption: "FULL",
        updateRule: "NO ACTION",
        deleteRule: "CASCADE",
      },
    },
    {
      catalog: "DB",
      schema: "S",
      name: "FK hidden",
      kind: "foreignKey",
      isDeferrable: false,
      initiallyDeferred: false,
      enforced: false,
      rely: false,
      comment: null,
      referenceStatus: "notVisible",
      reference: null,
    },
  ],
};
const keyMappings: RelationConstraintMappings = {
  revision: catalog.revision,
  targetId: catalog.relations[0].targetId,
  database: catalog.database,
  table: catalog.relations[0].table,
  mappings: [
    {
      catalog: "DB",
      schema: "S",
      name: "PK table",
      kind: "primaryKey",
      status: "available",
      columns: [
        { ordinal: 2, columnName: "b.second", referencedColumnName: null },
        { ordinal: 1, columnName: 'a"first', referencedColumnName: null },
      ],
      referencedTable: null,
    },
    {
      catalog: "DB",
      schema: "S",
      name: "UQ table",
      kind: "unique",
      status: "available",
      columns: [{ ordinal: 1, columnName: "unique value", referencedColumnName: null }],
      referencedTable: null,
    },
    {
      catalog: "DB",
      schema: "S",
      name: 'FK "parent',
      kind: "foreignKey",
      status: "available",
      columns: [
        { ordinal: 2, columnName: "b.second", referencedColumnName: "parent.second" },
        { ordinal: 1, columnName: 'a"first', referencedColumnName: 'parent"first' },
      ],
      referencedTable: { database: 'OTHER"DB', table: { schema: "P.dot", name: 'Parent "table' } },
    },
    {
      catalog: "DB",
      schema: "S",
      name: "FK hidden",
      kind: "foreignKey",
      status: "notVisible",
      columns: [],
      referencedTable: null,
    },
  ],
};
const checkConstraints: RelationCheckConstraints = {
  revision: catalog.revision,
  targetId: catalog.relations[0].targetId,
  database: catalog.database,
  table: catalog.relations[0].table,
  constraints: [
    {
      catalog: "DB",
      schema: "S",
      tableName: catalog.relations[0].table.name,
      name: 'CHECK "amount',
      checkClause: '"amount" > 0\nAND "label" <> \'雪 <img src=x onerror=alert(1)>\'',
    },
  ],
};
beforeEach(() => {
  vi.clearAllMocks();
  useStudio.setState({
    sessionStates: {},
    sessionContexts: {},
    sessionContextAuthorities: {},
    sessionContextSwitching: {},
    sessionContextRevision: {},
    sessionTransactionPending: {},
    sessionMetadataRefreshRequired: {},
    sessionDdlCatalogRefreshRequired: {},
    sessionSnowflakeGridPending: {},
    sessionSnowflakeGridTransactions: {},
    sessions: {},
    tabs: [],
  });
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "inspect_relation") return inspection;
    if (method === "preview_relation") return preview;
    if (method === "define_relation") return definition;
    if (method === "inspect_relation_constraints") return keyConstraints;
    if (method === "inspect_relation_constraint_mappings") return keyMappings;
    if (method === "inspect_relation_check_constraints") return checkConstraints;
    throw new Error(`Unexpected ${method}`);
  });
});
afterEach(cleanup);
it("opens one catalog request during StrictMode effect replay", async () => {
  render(
    <StrictMode>
      <RelationBrowser sessionId="s" onClose={vi.fn()} />
    </StrictMode>,
  );
  await screen.findByText("1 visible relations · DB");
  expect(client.call).toHaveBeenCalledTimes(1);
});

it.each(["pending", "displayed"])(
  "keeps the %s catalog when background context observation initializes an unknown runtime",
  async (phase) => {
    const session: SessionInfo = {
      id: "s",
      connectionId: "synthetic-profile",
      engine: "snowflake",
      serverVersion: "synthetic",
      latencyMs: 1,
      encrypted: true,
      capabilities: {
        sql: true,
        transactions: false,
        cancellation: true,
        tableBrowsing: false,
        relationBrowsing: true,
        tableEditing: false,
        tableFiltering: false,
        processes: false,
      },
    };
    useStudio.setState({ sessions: { [session.connectionId]: session } });
    let release!: (value: BrowseCatalog) => void;
    const held = new Promise<BrowseCatalog>((resolve) => {
      release = resolve;
    });
    vi.mocked(client.call).mockImplementation(async (method) => {
      if (method === "relation_catalog") return held;
      if (method === "session_contexts") throw new Error("Synthetic context unavailable");
      throw new Error(`Unexpected ${method}`);
    });
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await waitFor(() => expect(client.call).toHaveBeenCalledTimes(1));
    if (phase === "displayed") {
      await act(async () => {
        release(catalog);
        await held;
      });
      await screen.findByText("1 visible relations · DB");
    }
    // SqlEditor refreshes context after its background completion request.
    // A denied initial observation installs unknown defaults, not a new context.
    await act(async () => {
      expect(await useStudio.getState().refreshSessionContexts("s")).toBe(false);
    });
    expect(useStudio.getState().sessionStates.s.database).toBe("");
    expect(useStudio.getState().sessionContextRevision.s).toBeUndefined();
    if (phase === "pending") {
      await act(async () => {
        release(catalog);
        await held;
      });
    }
    await screen.findByText("1 visible relations · DB");
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
    expect(
      (screen.getByRole("button", { name: "View definition" }) as HTMLButtonElement).disabled,
    ).toBe(false);
    expect(vi.mocked(client.call).mock.calls.map(([method]) => method)).toEqual([
      "relation_catalog",
      "session_contexts",
    ]);
  },
);

function installSchemaAuthority() {
  const runtime = {
    sessionId: "s",
    transactionMode: "autoCommit" as const,
    transactionState: "idle" as const,
    contextKind: "schema" as const,
    activeContext: "S",
    database: "DB",
    role: null,
    searchPath: ["DB.S"],
    contextFingerprint: "observed-fingerprint",
    transactionObservation: {
      status: "observed" as const,
      transactionId: null,
      abortOnError: false,
      usability: "unknown" as const,
    },
  };
  useStudio.setState({
    sessionStates: { s: runtime },
    sessionContexts: { s: ["S"] },
    sessionContextAuthorities: {
      s: { state: runtime, available: ["S"], revision: "old-authority" },
    },
  });
}

it("revokes schema selection before each catalog, inspection and preview admission", async () => {
  const admitted: string[] = [];
  vi.mocked(client.call).mockImplementation(async (method) => {
    expect(useStudio.getState().sessionContextAuthorities.s).toBeUndefined();
    expect(useStudio.getState().sessionContexts.s).toEqual([]);
    admitted.push(method);
    if (method === "relation_catalog") return catalog;
    if (method === "inspect_relation") return inspection;
    if (method === "preview_relation") return preview;
    throw new Error(`Unexpected ${method}`);
  });
  installSchemaAuthority();
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  installSchemaAuthority();
  await selectAndInspect();
  installSchemaAuthority();
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  await screen.findByText("9007199254740993");
  expect(admitted).toEqual(["relation_catalog", "inspect_relation", "preview_relation"]);
});

it("does not admit relation SQL while a typed schema switch is pending", async () => {
  installSchemaAuthority();
  useStudio.setState({ sessionContextSwitching: { s: true } });
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Wait for the schema change to finish first",
  );
  expect(client.call).not.toHaveBeenCalled();
});

it("a rejected old refresh cannot erase a newer successful refresh", async () => {
  let rejectOld!: (error: Error) => void;
  const old = new Promise<BrowseCatalog>((_resolve, reject) => {
    rejectOld = reject;
  });
  vi.mocked(client.call).mockImplementationOnce(async () => old);
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await waitFor(() => expect(client.call).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
  await screen.findByText("1 visible relations · DB");
  await act(async () => {
    rejectOld(new Error("obsolete refresh"));
    await old.catch(() => {});
  });
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.getByText("1 visible relations · DB")).toBeTruthy();
});
async function selectAndInspect() {
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByText("DB.S.Quoted Table");
}
it("shows independent expression, descriptor and write defaults without conflating NULL or missing", async () => {
  const observed = structuredClone(inspection);
  observed.columns = [
    {
      ...observed.columns[0],
      name: "initial_and_write",
      defaultExpression: "2",
      descriptorDefaultExpression: "2",
      writeDefaultExpression: "3",
      writeDefaultStatus: "available",
    },
    {
      ...observed.columns[0],
      name: "reported_null",
      defaultExpression: "CURRENT_DATE()",
      descriptorDefaultExpression: null,
      writeDefaultExpression: null,
      writeDefaultStatus: "available",
    },
    {
      ...observed.columns[0],
      name: "reported_empty",
      writeDefaultExpression: "",
      writeDefaultStatus: "available",
    },
    { ...observed.columns[0], name: "not_reported" },
  ];
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "inspect_relation") return observed;
    throw new Error(`Unexpected ${method}`);
  });
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await selectAndInspect();
  const row = (name: string) => screen.getByRole("rowheader", { name }).closest("tr")!;
  expect(within(row("initial_and_write")).getAllByText("2")).toHaveLength(2);
  expect(within(row("initial_and_write")).getByText("3")).toBeTruthy();
  expect(within(row("reported_null")).getByText("CURRENT_DATE()")).toBeTruthy();
  expect(within(row("reported_null")).getByText("NULL")).toBeTruthy();
  expect(within(row("reported_empty")).getByText("“”")).toBeTruthy();
  expect(within(row("not_reported")).getByText("not reported")).toBeTruthy();
});
it("completes catalog → inspect → preview → refresh with opaque revisions and exact scalar rendering", async () => {
  render(<RelationBrowser sessionId="isolated-console" onClose={vi.fn()} />);
  await selectAndInspect();
  expect(screen.getByText(/Key constraints have not been fetched/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  await screen.findByText("9007199254740993");
  expect(screen.getByLabelText("NULL")).toBeTruthy();
  expect(screen.getByLabelText("Empty string")).toBeTruthy();
  expect(screen.getByText(/Preview limit reached/)).toBeTruthy();
  expect(client.call).toHaveBeenCalledWith("inspect_relation", {
    sessionId: "isolated-console",
    request: { revision: "catalog-1", targetId: "opaque-target" },
  });
  expect(client.call).toHaveBeenCalledWith("preview_relation", {
    sessionId: "isolated-console",
    request: {
      revision: "catalog-1",
      targetId: "opaque-target",
      inspectionRevision: "inspection-1",
    },
  });
  vi.mocked(client.call).mockRejectedValueOnce(new Error("Catalog unavailable"));
  fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
  expect(screen.queryByText("9007199254740993")).toBeNull();
  expect((await screen.findByRole("alert")).textContent).toContain("Catalog unavailable");
  expect((screen.getByRole("button", { name: "Preview rows" }) as HTMLButtonElement).disabled).toBe(
    true,
  );
});
it("clears an inspected target when the relation filter changes", async () => {
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await selectAndInspect();
  fireEvent.change(screen.getByLabelText("Find relation"), { target: { value: "absent" } });
  expect(screen.queryByText("DB.S.Quoted Table")).toBeNull();
  expect(screen.queryByRole("option", { name: /Quoted Table/ })).toBeNull();
  expect((screen.getByRole("button", { name: "Preview rows" }) as HTMLButtonElement).disabled).toBe(
    true,
  );
  expect(client.call).toHaveBeenCalledTimes(2);
});
it.each(["shadowed", "ambiguous", "unknownTraits", "unsupportedKind"] as const)(
  "keeps %s target visible without dispatching inspection",
  async (availability) => {
    vi.mocked(client.call).mockResolvedValue({
      ...catalog,
      relations: [{ ...catalog.relations[0], availability }],
    });
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await screen.findByText("1 visible relations · DB");
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
    expect(
      (screen.getByRole("button", { name: "Inspect relation" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "View definition" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "View key constraints" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "View key column mappings" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    expect(screen.getByRole("note").textContent).toContain(availability);
    expect(client.call).toHaveBeenCalledTimes(1);
  },
);
it("ignores a late inspection after refresh starts and does not clear a newer catalog", async () => {
  let release!: (value: RelationInspection) => void;
  const held = new Promise<RelationInspection>((resolve) => {
    release = resolve;
  });
  vi.mocked(client.call).mockImplementation(async (method) =>
    method === "inspect_relation" ? held : catalog,
  );
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await waitFor(() =>
    expect(client.call).toHaveBeenCalledWith("inspect_relation", expect.anything()),
  );
  fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
  await screen.findByText("1 visible relations · DB");
  await act(async () => {
    release(inspection);
    await held;
  });
  expect(screen.queryByText("DB.S.Quoted Table")).toBeNull();
  expect(screen.getByText("1 visible relations · DB")).toBeTruthy();
});
it("discards a previous console response after unmount/rebind", async () => {
  let release!: (value: BrowseCatalog) => void;
  const held = new Promise<BrowseCatalog>((resolve) => {
    release = resolve;
  });
  vi.mocked(client.call).mockImplementation(async (_method, args) =>
    (args as { sessionId: string }).sessionId === "old" ? held : { ...catalog, database: "NEW" },
  );
  const { rerender } = render(<RelationBrowser key="old" sessionId="old" onClose={vi.fn()} />);
  rerender(<RelationBrowser key="new" sessionId="new" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · NEW");
  await act(async () => {
    release(catalog);
    await held;
  });
  expect(screen.queryByText("1 visible relations · DB")).toBeNull();
});
it("pages retained rows and wide columns locally without SQL re-execution", async () => {
  const wide: RelationPreview = {
    columns: Array.from({ length: 21 }, (_, i) => ({ ...column, id: `c${i}`, name: `col${i}` })),
    rows: Array.from({ length: 26 }, (_, row) =>
      Array.from({ length: 21 }, (_, col) => `r${row}c${col}`),
    ),
    truncated: false,
  };
  vi.mocked(client.call).mockImplementation(async (method) =>
    method === "relation_catalog" ? catalog : method === "inspect_relation" ? inspection : wide,
  );
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await selectAndInspect();
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  await screen.findByText("r0c0");
  const grid = screen.getByRole("table", { name: "Read-only preview" });
  expect(within(grid).getAllByRole("row")).toHaveLength(26);
  fireEvent.click(screen.getByRole("button", { name: "Next rows" }));
  fireEvent.click(screen.getByRole("button", { name: "Next columns" }));
  expect(screen.getByText("r25c20")).toBeTruthy();
  expect(client.call).toHaveBeenCalledTimes(3);
});

async function selectAndDefine() {
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "View definition" }));
  return screen.findByRole("region", { name: "Relation definition" });
}

it("views and copies generated DDL before column inspection, without an editor or execute action", async () => {
  const descriptor = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  try {
    render(<RelationBrowser sessionId="isolated-console" onClose={vi.fn()} />);
    const region = await selectAndDefine();
    expect(client.call).toHaveBeenCalledWith("define_relation", {
      sessionId: "isolated-console",
      request: { revision: "catalog-1", targetId: "opaque-target" },
    });
    expect(client.call).toHaveBeenCalledTimes(2);
    expect(within(region).getByLabelText("Object DDL").textContent).toBe(definition.ddl);
    expect(within(region).getByRole("note").textContent).toContain("CREATE OR REPLACE");
    expect(within(region).getByRole("note").textContent).toContain("normalize types");
    expect(within(region).getByRole("note").textContent).toContain("hidden policy and tag");
    expect(within(region).getByRole("note").textContent).toContain("replay is safe or complete");
    const copy = within(region).getByRole("button", { name: "Copy DDL" });
    copy.focus();
    expect(document.activeElement).toBe(copy);
    fireEvent.click(copy);
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(definition.ddl));
    expect(within(region).getAllByRole("button")).toEqual([copy]);
    expect(screen.queryByRole("button", { name: /Open in SQL editor|Execute|Review/ })).toBeNull();
  } finally {
    if (descriptor) Object.defineProperty(navigator, "clipboard", descriptor);
    else Reflect.deleteProperty(navigator, "clipboard");
  }
});

it.each(["selection", "filter", "refresh"])("clears displayed DDL on %s", async (action) => {
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await selectAndDefine();
  if (action === "selection")
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "" } });
  else if (action === "filter")
    fireEvent.change(screen.getByLabelText("Find relation"), { target: { value: "absent" } });
  else fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
  expect(screen.queryByRole("region", { name: "Relation definition" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Copy DDL" })).toBeNull();
});

it("clears the old definition after a current permission failure", async () => {
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await selectAndDefine();
  vi.mocked(client.call).mockRejectedValueOnce(new Error("Insufficient privileges for GET_DDL"));
  fireEvent.click(screen.getByRole("button", { name: "View definition" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Insufficient privileges");
  expect(screen.queryByLabelText("Object DDL")).toBeNull();
  expect(screen.queryByRole("button", { name: "Copy DDL" })).toBeNull();
});

it.each(["revision", "targetId"] as const)(
  "rejects a definition with a mismatched %s",
  async (field) => {
    vi.mocked(client.call).mockImplementation(async (method) => {
      if (method === "relation_catalog") return catalog;
      if (method === "define_relation") return { ...definition, [field]: "different" };
      throw new Error(`Unexpected ${method}`);
    });
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await screen.findByText("1 visible relations · DB");
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
    fireEvent.click(screen.getByRole("button", { name: "View definition" }));
    expect((await screen.findByRole("alert")).textContent).toContain("Relation definition changed");
    expect(screen.queryByLabelText("Object DDL")).toBeNull();
  },
);

it.each(
  [
    "transaction",
    "transientTransaction",
    "metadata",
    "ddlCatalog",
    "schemaSwitch",
    "context",
  ].flatMap((change) =>
    ["definition", "constraints", "mappings"].map((workflow) => ({ change, workflow })),
  ),
)(
  "suppresses pending $workflow after $change changes the console context",
  async ({ change, workflow }) => {
    installSchemaAuthority();
    let release!: (
      value: RelationDefinition | RelationConstraints | RelationConstraintMappings,
    ) => void;
    const held = new Promise<RelationDefinition | RelationConstraints | RelationConstraintMappings>(
      (resolve) => {
        release = resolve;
      },
    );
    vi.mocked(client.call).mockImplementation(async (method) => {
      if (method === "relation_catalog") return catalog;
      if (
        method === "define_relation" ||
        method === "inspect_relation_constraints" ||
        method === "inspect_relation_constraint_mappings"
      )
        return held;
      throw new Error(`Unexpected ${method}`);
    });
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await screen.findByText("1 visible relations · DB");
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
    fireEvent.click(
      screen.getByRole("button", {
        name:
          workflow === "definition"
            ? "View definition"
            : workflow === "constraints"
              ? "View key constraints"
              : "View key column mappings",
      }),
    );
    await waitFor(() => expect(client.call).toHaveBeenCalledTimes(2));
    act(() => {
      if (change === "transaction") useStudio.setState({ sessionTransactionPending: { s: true } });
      else if (change === "transientTransaction") {
        useStudio.setState({ sessionTransactionPending: { s: true } });
        useStudio.setState({ sessionTransactionPending: {} });
      } else if (change === "metadata")
        useStudio.setState({ sessionMetadataRefreshRequired: { s: true } });
      else if (change === "ddlCatalog")
        useStudio.setState({ sessionDdlCatalogRefreshRequired: { s: true } });
      else if (change === "schemaSwitch")
        useStudio.setState({ sessionContextSwitching: { s: true } });
      else
        useStudio.setState((state) => ({
          sessionStates: {
            s: { ...state.sessionStates.s, activeContext: "OTHER", contextFingerprint: "changed" },
          },
        }));
    });
    await act(async () => {
      release(
        workflow === "definition"
          ? definition
          : workflow === "constraints"
            ? keyConstraints
            : keyMappings,
      );
      await held;
    });
    expect(screen.queryByLabelText("Object DDL")).toBeNull();
    expect(screen.queryByRole("button", { name: "Copy DDL" })).toBeNull();
    expect(screen.queryByRole("table", { name: "Key constraint inventory" })).toBeNull();
    expect(screen.queryByRole("table", { name: "Key column mapping inventory" })).toBeNull();
    expect(
      (screen.getByRole("button", { name: "View definition" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(client.call).toHaveBeenCalledTimes(2);
  },
);

it("clears an already visible definition when the observed console context changes", async () => {
  installSchemaAuthority();
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await selectAndDefine();
  act(() => useStudio.setState({ sessionContextRevision: { s: 1 } }));
  expect(screen.queryByLabelText("Object DDL")).toBeNull();
});

it("a late failed definition cannot erase a definition fetched after refresh", async () => {
  let reject!: (error: Error) => void;
  const held = new Promise<RelationDefinition>((_resolve, fail) => {
    reject = fail;
  });
  let definitions = 0;
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "define_relation") return ++definitions === 1 ? held : definition;
    throw new Error(`Unexpected ${method}`);
  });
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "View definition" }));
  fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
  await selectAndDefine();
  await act(async () => {
    reject(new Error("stale permission failure"));
    await held.catch(() => {});
  });
  expect(screen.getByLabelText("Object DDL").textContent).toBe(definition.ddl);
  expect(screen.queryByRole("alert")).toBeNull();
});

it("discards a pending definition when rebound to another console", async () => {
  let release!: (value: RelationDefinition) => void;
  const held = new Promise<RelationDefinition>((resolve) => {
    release = resolve;
  });
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "define_relation") return held;
    throw new Error(`Unexpected ${method}`);
  });
  const { rerender } = render(<RelationBrowser sessionId="old" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "View definition" }));
  rerender(<RelationBrowser sessionId="new" onClose={vi.fn()} />);
  await waitFor(() =>
    expect(client.call).toHaveBeenCalledWith("relation_catalog", { sessionId: "new" }),
  );
  await act(async () => {
    release(definition);
    await held;
  });
  expect(screen.queryByLabelText("Object DDL")).toBeNull();
  expect(screen.queryByRole("button", { name: "Copy DDL" })).toBeNull();
});

async function selectAndLoadConstraints() {
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "View key constraints" }));
  return screen.findByRole("table", { name: "Key constraint inventory" });
}

it("shows declared keys and referenced constraint identities independently of columns and DDL", async () => {
  render(<RelationBrowser sessionId="isolated-console" onClose={vi.fn()} />);
  const table = await selectAndLoadConstraints();
  expect(client.call).toHaveBeenCalledWith("inspect_relation_constraints", {
    sessionId: "isolated-console",
    request: { revision: "catalog-1", targetId: "opaque-target" },
  });
  expect(client.call).toHaveBeenCalledTimes(2);
  const row = (name: string) => within(table).getByRole("rowheader", { name }).closest("tr")!;
  expect(
    within(row('"DB"."S"."PK table"'))
      .getAllByRole("cell")
      .map((cell) => cell.textContent),
  ).toEqual([
    "Primary key",
    "no",
    "no",
    "no",
    "yes",
    "Declared primary key",
    "Not applicable",
    "—",
    "—",
    "—",
  ]);
  expect(
    within(row('"DB"."S"."UQ table"'))
      .getAllByRole("cell")
      .map((cell) => cell.textContent),
  ).toEqual(["Unique", "yes", "yes", "yes", "no", "—", "Not applicable", "—", "—", "—"]);
  expect(
    within(row('"DB"."S"."FK ""parent"'))
      .getAllByRole("cell")
      .map((cell) => cell.textContent),
  ).toEqual([
    "Foreign key",
    "no",
    "no",
    "no",
    "no",
    "Visible reference",
    '"OTHER"."P"."PK ""parent"',
    "FULL",
    "NO ACTION",
    "CASCADE",
  ]);
  expect(within(row('"DB"."S"."FK hidden"')).getByText("Not visible")).toBeTruthy();
  const region = screen.getByRole("region", { name: "Relation key constraints" });
  expect(within(region).getByRole("note").textContent).toContain(
    "do not establish row editing safety",
  );
  expect(within(region).getByRole("note").textContent).toContain(
    "excludes column mappings, CHECK constraints, and NOT NULL",
  );
  expect(within(region).queryByRole("button")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "View definition" }));
  await screen.findByLabelText("Object DDL");
  expect(screen.getByRole("table", { name: "Key constraint inventory" })).toBe(table);
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByRole("table", { name: "Column inspection" });
  expect(screen.getByRole("table", { name: "Key constraint inventory" })).toBe(table);
  expect(screen.getByLabelText("Object DDL").textContent).toBe(definition.ddl);
  expect(screen.queryByText("Key constraints have not been fetched.")).toBeNull();
});

it("distinguishes an empty key inventory from not fetched and unavailable, and retries denial", async () => {
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await selectAndLoadConstraints();
  vi.mocked(client.call).mockRejectedValueOnce(new Error("Synthetic key metadata denied"));
  fireEvent.click(screen.getByRole("button", { name: "View key constraints" }));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Key constraints are unavailable: Synthetic key metadata denied",
  );
  expect(screen.queryByRole("table", { name: "Key constraint inventory" })).toBeNull();
  expect(screen.queryByText(/No visible declared/)).toBeNull();
  expect(screen.queryByText("Key constraints have not been fetched.")).toBeNull();
  vi.mocked(client.call).mockResolvedValueOnce({ ...keyConstraints, constraints: [] });
  fireEvent.click(screen.getByRole("button", { name: "View key constraints" }));
  await screen.findByText("No visible declared primary, unique, or foreign keys were returned.");
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.queryByRole("table", { name: "Key constraint inventory" })).toBeNull();
  expect(client.call).toHaveBeenCalledTimes(4);
});

it.each(["revision", "targetId", "database", "schema", "name"])(
  "rejects key inventory with mismatched %s without retaining the previous inventory",
  async (field) => {
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await selectAndLoadConstraints();
    const result = structuredClone(keyConstraints);
    if (field === "schema" || field === "name") result.table[field] = "different";
    else if (field === "revision" || field === "targetId" || field === "database")
      result[field] = "different";
    vi.mocked(client.call).mockResolvedValueOnce(result);
    fireEvent.click(screen.getByRole("button", { name: "View key constraints" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Key constraint target changed",
    );
    expect(screen.queryByRole("table", { name: "Key constraint inventory" })).toBeNull();
  },
);

it.each(["selection", "filter", "refresh", "context"])(
  "clears displayed key inventory on %s",
  async (change) => {
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await selectAndLoadConstraints();
    if (change === "selection")
      fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "" } });
    else if (change === "filter")
      fireEvent.change(screen.getByLabelText("Find relation"), { target: { value: "absent" } });
    else if (change === "refresh")
      fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
    else act(() => useStudio.setState({ sessionContextRevision: { s: 1 } }));
    expect(screen.queryByRole("table", { name: "Key constraint inventory" })).toBeNull();
  },
);

it.each(["resolve", "reject"])(
  "a late key request %s cannot replace the newer inventory",
  async (outcome) => {
    let release!: (value: RelationConstraints) => void;
    let reject!: (error: Error) => void;
    const held = new Promise<RelationConstraints>((resolve, fail) => {
      release = resolve;
      reject = fail;
    });
    let count = 0;
    vi.mocked(client.call).mockImplementation(async (method) => {
      if (method === "relation_catalog") return catalog;
      if (method === "inspect_relation_constraints")
        return ++count === 1 ? held : { ...keyConstraints, constraints: [] };
      throw new Error(`Unexpected ${method}`);
    });
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await screen.findByText("1 visible relations · DB");
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
    fireEvent.click(screen.getByRole("button", { name: "View key constraints" }));
    fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
    await screen.findByText("1 visible relations · DB");
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
    fireEvent.click(screen.getByRole("button", { name: "View key constraints" }));
    await screen.findByText("No visible declared primary, unique, or foreign keys were returned.");
    await act(async () => {
      if (outcome === "resolve") release(keyConstraints);
      else reject(new Error("stale denial"));
      await held.catch(() => {});
    });
    expect(
      screen.getByText("No visible declared primary, unique, or foreign keys were returned."),
    ).toBeTruthy();
    expect(screen.queryByRole("table", { name: "Key constraint inventory" })).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  },
);

it.each(
  ["owner disappears", "session changes"].flatMap((change) =>
    ["constraints", "mappings"].map((workflow) => ({ change, workflow })),
  ),
)("discards pending $workflow when its $change", async ({ change, workflow }) => {
  const session: SessionInfo = {
    id: "s",
    connectionId: "synthetic-profile",
    engine: "snowflake",
    serverVersion: "synthetic",
    latencyMs: 1,
    encrypted: true,
    capabilities: {
      sql: true,
      transactions: false,
      cancellation: true,
      tableBrowsing: false,
      relationBrowsing: true,
      tableEditing: false,
      tableFiltering: false,
      processes: false,
    },
  };
  useStudio.setState({ sessions: { [session.connectionId]: session } });
  const ownerTabId = useStudio.getState().openQuery(session.connectionId);
  let release!: (value: RelationConstraints | RelationConstraintMappings) => void;
  const held = new Promise<RelationConstraints | RelationConstraintMappings>((resolve) => {
    release = resolve;
  });
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (
      method === "inspect_relation_constraints" ||
      method === "inspect_relation_constraint_mappings"
    )
      return held;
    throw new Error(`Unexpected ${method}`);
  });
  const { rerender } = render(
    <RelationBrowser sessionId="s" ownerTabId={ownerTabId} onClose={vi.fn()} />,
  );
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(
    screen.getByRole("button", {
      name: workflow === "constraints" ? "View key constraints" : "View key column mappings",
    }),
  );
  await waitFor(() => expect(client.call).toHaveBeenCalledTimes(2));
  if (change === "owner disappears") act(() => useStudio.setState({ tabs: [] }));
  else rerender(<RelationBrowser sessionId="other" ownerTabId={ownerTabId} onClose={vi.fn()} />);
  await act(async () => {
    release(workflow === "constraints" ? keyConstraints : keyMappings);
    await held;
  });
  expect(screen.queryByRole("table", { name: "Key constraint inventory" })).toBeNull();
  expect(screen.queryByRole("table", { name: "Key column mapping inventory" })).toBeNull();
  expect(
    (screen.getByRole("button", { name: "View key constraints" }) as HTMLButtonElement).disabled,
  ).toBe(true);
});

async function selectAndLoadMappings() {
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "View key column mappings" }));
  return screen.findByRole("table", { name: "Key column mapping inventory" });
}

it("views composite key mappings independently, in ordinal order with quoted FK pairs", async () => {
  render(<RelationBrowser sessionId="isolated-console" onClose={vi.fn()} />);
  const table = await selectAndLoadMappings();
  expect(client.call).toHaveBeenCalledWith("inspect_relation_constraint_mappings", {
    sessionId: "isolated-console",
    request: { revision: "catalog-1", targetId: "opaque-target" },
  });
  expect(client.call).toHaveBeenCalledTimes(2);
  const rows = within(table)
    .getAllByRole("row")
    .slice(1)
    .map((row) =>
      within(row)
        .getAllByRole("cell")
        .slice(0, -1)
        .map((cell) => cell.textContent),
    );
  expect(rows).toEqual([
    ["Primary key", "Available", "1", '"a""first"', "Not applicable", "Not applicable"],
    ["Primary key", "Available", "2", '"b.second"', "Not applicable", "Not applicable"],
    ["Unique", "Available", "1", '"unique value"', "Not applicable", "Not applicable"],
    [
      "Foreign key",
      "Available",
      "1",
      '"a""first"',
      '"OTHER""DB"."P.dot"."Parent ""table"',
      '"parent""first"',
    ],
    [
      "Foreign key",
      "Available",
      "2",
      '"b.second"',
      '"OTHER""DB"."P.dot"."Parent ""table"',
      '"parent.second"',
    ],
    ["Foreign key", "Not visible", "—", "—", "—", "—"],
  ]);
  expect(
    within(table)
      .getAllByRole("rowheader")
      .map((header) => header.textContent),
  ).toEqual([
    '"DB"."S"."PK table"',
    '"DB"."S"."PK table"',
    '"DB"."S"."UQ table"',
    '"DB"."S"."FK ""parent"',
    '"DB"."S"."FK ""parent"',
    '"DB"."S"."FK hidden"',
  ]);
  const region = screen.getByRole("region", { name: "Relation key column mappings" });
  expect(within(region).getByText(/Visible column mappings can lag/).textContent).toContain(
    "can lag schema changes",
  );
  expect(within(region).getByText(/Visible column mappings can lag/).textContent).toContain(
    "do not establish row editing safety",
  );
  expect(
    (within(region).getByRole("button", { name: /Select referenced table/ }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(within(region).getByText(/Cross-database navigation is not available yet/)).toBeTruthy();
});

it("mapping denial clears old mappings, preserves other metadata, and retries to explicit empty", async () => {
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await selectAndLoadMappings();
  fireEvent.click(screen.getByRole("button", { name: "View key constraints" }));
  const declarations = await screen.findByRole("table", { name: "Key constraint inventory" });
  fireEvent.click(screen.getByRole("button", { name: "View definition" }));
  await screen.findByLabelText("Object DDL");
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  const columns = await screen.findByRole("table", { name: "Column inspection" });
  vi.mocked(client.call).mockRejectedValueOnce(new Error("Synthetic mapping denied"));
  fireEvent.click(screen.getByRole("button", { name: "View key column mappings" }));
  const region = screen.getByRole("region", { name: "Relation key column mappings" });
  expect((await within(region).findByRole("alert")).textContent).toContain(
    "Key column mappings are unavailable: Synthetic mapping denied",
  );
  expect(within(region).queryByRole("table")).toBeNull();
  expect(within(region).queryByText(/No visible/)).toBeNull();
  expect(within(region).queryByText(/have not been fetched/)).toBeNull();
  expect(screen.getByRole("table", { name: "Key constraint inventory" })).toBe(declarations);
  expect(screen.getByRole("table", { name: "Column inspection" })).toBe(columns);
  expect(screen.getByLabelText("Object DDL").textContent).toBe(definition.ddl);
  vi.mocked(client.call).mockResolvedValueOnce({ ...keyMappings, mappings: [] });
  fireEvent.click(screen.getByRole("button", { name: "View key column mappings" }));
  await within(region).findByText("No visible key column mappings were returned.");
  expect(within(region).queryByRole("alert")).toBeNull();
  expect(screen.getByRole("table", { name: "Key constraint inventory" })).toBe(declarations);
});

it("shows dynamic key mappings as unsupported rather than empty or not fetched", async () => {
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog")
      return {
        ...catalog,
        relations: [
          { ...catalog.relations[0], traits: { ...catalog.relations[0].traits, dynamic: true } },
        ],
      };
    if (method === "inspect_relation_constraint_mappings")
      return {
        ...keyMappings,
        mappings: [
          { ...keyMappings.mappings[0], status: "unsupported", columns: [], referencedTable: null },
        ],
      };
    throw new Error(`Unexpected ${method}`);
  });
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  const table = await selectAndLoadMappings();
  expect(within(table).getByText("Unsupported")).toBeTruthy();
  expect(within(table).getByRole("rowheader").textContent).toBe('"DB"."S"."PK table"');
  expect(screen.queryByText("No visible key column mappings were returned.")).toBeNull();
  expect(screen.queryByText("Key column mappings have not been fetched.")).toBeNull();
});

it.each(["revision", "targetId", "database", "schema", "name"])(
  "rejects mappings with mismatched %s",
  async (field) => {
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await selectAndLoadMappings();
    const result = structuredClone(keyMappings);
    if (field === "schema" || field === "name") result.table[field] = "different";
    else if (field === "revision" || field === "targetId" || field === "database")
      result[field] = "different";
    vi.mocked(client.call).mockResolvedValueOnce(result);
    fireEvent.click(screen.getByRole("button", { name: "View key column mappings" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Key column mapping target changed",
    );
    expect(screen.queryByRole("table", { name: "Key column mapping inventory" })).toBeNull();
  },
);

it.each(["empty available", "missing FK table", "missing FK column"])(
  "rejects incomplete mapping data: %s",
  async (kind) => {
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await screen.findByText("1 visible relations · DB");
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
    const result = structuredClone(keyMappings);
    if (kind === "empty available") result.mappings[0].columns = [];
    else if (kind === "missing FK table") result.mappings[2].referencedTable = null;
    else result.mappings[2].columns[0].referencedColumnName = null;
    vi.mocked(client.call).mockResolvedValueOnce(result);
    fireEvent.click(screen.getByRole("button", { name: "View key column mappings" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Key column mappings are incomplete",
    );
    expect(screen.queryByRole("table", { name: "Key column mapping inventory" })).toBeNull();
    expect(screen.queryByText("No visible key column mappings were returned.")).toBeNull();
  },
);

it.each(["selection", "filter", "refresh", "context"])(
  "clears displayed mappings on %s",
  async (change) => {
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await selectAndLoadMappings();
    if (change === "selection")
      fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "" } });
    else if (change === "filter")
      fireEvent.change(screen.getByLabelText("Find relation"), { target: { value: "absent" } });
    else if (change === "refresh")
      fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
    else act(() => useStudio.setState({ sessionContextRevision: { s: 1 } }));
    expect(screen.queryByRole("table", { name: "Key column mapping inventory" })).toBeNull();
  },
);

it.each(["resolve", "reject"])(
  "a late mapping %s cannot replace mappings fetched after refresh",
  async (outcome) => {
    let release!: (value: RelationConstraintMappings) => void;
    let reject!: (error: Error) => void;
    const held = new Promise<RelationConstraintMappings>((resolve, fail) => {
      release = resolve;
      reject = fail;
    });
    let count = 0;
    vi.mocked(client.call).mockImplementation(async (method) => {
      if (method === "relation_catalog") return catalog;
      if (method === "inspect_relation_constraint_mappings")
        return ++count === 1 ? held : { ...keyMappings, mappings: [] };
      throw new Error(`Unexpected ${method}`);
    });
    render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
    await screen.findByText("1 visible relations · DB");
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
    fireEvent.click(screen.getByRole("button", { name: "View key column mappings" }));
    fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
    await screen.findByText("1 visible relations · DB");
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
    fireEvent.click(screen.getByRole("button", { name: "View key column mappings" }));
    await screen.findByText("No visible key column mappings were returned.");
    await act(async () => {
      if (outcome === "resolve") release(keyMappings);
      else reject(new Error("stale mapping denial"));
      await held.catch(() => {});
    });
    expect(screen.getByText("No visible key column mappings were returned.")).toBeTruthy();
    expect(screen.queryByRole("table", { name: "Key column mapping inventory" })).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  },
);

const referencedTarget: BrowseCatalog["relations"][number] = {
  ...catalog.relations[0],
  targetId: "opaque-qualified-destination",
  table: { schema: 'P."quoted', name: 'Parent "table' },
};
const referenceCatalog: BrowseCatalog = {
  ...catalog,
  relations: [
    catalog.relations[0],
    referencedTarget,
    {
      ...referencedTarget,
      targetId: "opaque-same-name-decoy",
      table: { ...referencedTarget.table, schema: "S" },
    },
  ],
};
const referenceMappings: RelationConstraintMappings = {
  ...keyMappings,
  mappings: [
    {
      ...keyMappings.mappings[2],
      referencedTable: { database: "DB", table: referencedTarget.table },
    },
  ],
};

function renderReferenceNavigation(
  retainedCatalog = referenceCatalog,
  resultMappings = referenceMappings,
) {
  const session: SessionInfo = {
    id: "s",
    connectionId: "synthetic-profile",
    engine: "snowflake",
    serverVersion: "synthetic",
    latencyMs: 1,
    encrypted: true,
    capabilities: {
      sql: true,
      transactions: false,
      cancellation: true,
      tableBrowsing: false,
      relationBrowsing: true,
      tableEditing: false,
      tableFiltering: false,
      processes: false,
    },
  };
  useStudio.setState({ sessions: { [session.connectionId]: session } });
  const ownerTabId = useStudio.getState().openQuery(session.connectionId);
  installSchemaAuthority();
  vi.mocked(client.call).mockImplementation(async (method, args) => {
    if (method === "relation_catalog") return retainedCatalog;
    if (method === "inspect_relation_constraint_mappings") return resultMappings;
    if (method === "inspect_relation_check_constraints") return checkConstraints;
    if (method === "define_relation") return definition;
    if (method === "inspect_relation_constraints") return keyConstraints;
    if (method === "preview_relation") return preview;
    if (method === "inspect_relation") {
      const id = (args as { request: { targetId: string } }).request.targetId;
      const target = retainedCatalog.relations.find((entry) => entry.targetId === id)!;
      return { ...inspection, targetId: id, table: target.table };
    }
    throw new Error(`Unexpected ${method}`);
  });
  return {
    view: render(<RelationBrowser sessionId="s" ownerTabId={ownerTabId} onClose={vi.fn()} />),
    ownerTabId,
    session,
  };
}

async function loadReferenceButton() {
  await screen.findByRole("option", { name: /S.Quoted Table/ });
  fireEvent.change(screen.getByLabelText("Find relation"), { target: { value: "Quoted Table" } });
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "View key column mappings" }));
  return screen.findByRole("button", { name: /^Select referenced table/ });
}

it("selects the exact quoted cross-schema opaque target from the full catalog without RPCs", async () => {
  renderReferenceNavigation();
  const button = await loadReferenceButton();
  expect(button.tagName).toBe("BUTTON");
  expect(button.getAttribute("aria-label")).toBe(
    'Select referenced table "DB"."P.""quoted"."Parent ""table"',
  );
  expect(screen.queryByRole("option", { name: /Parent/ })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "View definition" }));
  await screen.findByLabelText("Object DDL");
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByRole("table", { name: "Column inspection" });
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  await screen.findByRole("table", { name: "Read-only preview" });
  vi.mocked(client.call).mockRejectedValueOnce(new Error("Synthetic declaration denial"));
  fireEvent.click(screen.getByRole("button", { name: "View key constraints" }));
  await screen.findByRole("alert");
  const before = vi.mocked(client.call).mock.calls.slice();
  button.focus();
  fireEvent.click(button);
  expect(client.call).toHaveBeenCalledTimes(before.length);
  const selector = screen.getByLabelText("Relation") as HTMLSelectElement;
  expect(selector.value).toBe("opaque-qualified-destination");
  expect(document.activeElement).toBe(selector);
  expect((screen.getByLabelText("Find relation") as HTMLInputElement).value).toBe("");
  expect(screen.queryByLabelText("Object DDL")).toBeNull();
  expect(screen.queryByRole("table")).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.getByText("Key constraints have not been fetched.")).toBeTruthy();
  expect(screen.getByText("Key column mappings have not been fetched.")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByRole("table", { name: "Column inspection" });
  expect(client.call).toHaveBeenLastCalledWith("inspect_relation", {
    sessionId: "s",
    request: { revision: "catalog-1", targetId: "opaque-qualified-destination" },
  });
  expect(vi.mocked(client.call).mock.calls.map(([method]) => method)).toEqual([
    "relation_catalog",
    "inspect_relation_constraint_mappings",
    "define_relation",
    "inspect_relation",
    "preview_relation",
    "inspect_relation_constraints",
    "inspect_relation",
  ]);
});

it("selects a self-reference locally and clears old mapping details", async () => {
  renderReferenceNavigation(referenceCatalog, {
    ...referenceMappings,
    mappings: [
      {
        ...referenceMappings.mappings[0],
        referencedTable: { database: "DB", table: catalog.relations[0].table },
      },
    ],
  });
  const button = await loadReferenceButton();
  const before = vi.mocked(client.call).mock.calls.length;
  fireEvent.click(button);
  expect((screen.getByLabelText("Relation") as HTMLSelectElement).value).toBe("opaque-target");
  expect(document.activeElement).toBe(screen.getByLabelText("Relation"));
  expect(screen.queryByRole("table", { name: "Key column mapping inventory" })).toBeNull();
  expect(client.call).toHaveBeenCalledTimes(before);
});

it.each([
  "absent",
  "case mismatch",
  "ambiguous",
  "unavailable duplicate",
  "shadowed",
  "unknownTraits",
  "external database",
] as const)(
  "explains %s referenced destinations without selecting or issuing RPCs",
  async (scenario) => {
    const retained = structuredClone(referenceCatalog);
    const result = structuredClone(referenceMappings);
    let reason = "";
    if (scenario === "absent") {
      retained.relations = retained.relations.filter(
        (entry) => entry.targetId !== referencedTarget.targetId,
      );
      reason = "absent from this retained catalog";
    } else if (scenario === "case mismatch") {
      retained.relations[1].table.name = referencedTarget.table.name.toUpperCase();
      reason = "absent from this retained catalog";
    } else if (scenario === "ambiguous" || scenario === "unavailable duplicate") {
      retained.relations.push({
        ...retained.relations[1],
        targetId: "duplicate-exact-identity",
        availability: scenario === "ambiguous" ? "available" : "shadowed",
      });
      reason = "identity is ambiguous";
    } else if (scenario === "external database") {
      result.mappings[0].referencedTable!.database = "OTHER";
      reason = "Cross-database navigation is not available yet";
    } else {
      retained.relations[1].availability = scenario;
      reason = `unavailable (${scenario})`;
    }
    renderReferenceNavigation(retained, result);
    const button = (await loadReferenceButton()) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    const explanation = screen.getByText((text) => text.includes(reason));
    expect(button.getAttribute("aria-describedby")).toBe(explanation.id);
    const before = vi.mocked(client.call).mock.calls.length;
    fireEvent.click(button);
    expect((screen.getByLabelText("Relation") as HTMLSelectElement).value).toBe("opaque-target");
    expect(client.call).toHaveBeenCalledTimes(before);
  },
);

it("revokes referenced navigation after a committed session replacement and loads only its catalog", async () => {
  const { view, ownerTabId, session } = renderReferenceNavigation();
  await loadReferenceButton();
  const nextCatalog: BrowseCatalog = {
    ...referenceCatalog,
    revision: "replacement-catalog",
    relations: referenceCatalog.relations.map((entry) => ({
      ...entry,
      targetId: `replacement-${entry.targetId}`,
    })),
  };
  vi.mocked(client.call).mockImplementation(async (method, args) => {
    if (method === "relation_catalog" && (args as { sessionId: string }).sessionId === "other")
      return nextCatalog;
    throw new Error(`Unexpected ${method}`);
  });
  const before = vi.mocked(client.call).mock.calls.length;
  act(() => {
    useStudio.setState({
      sessions: { [session.connectionId]: { ...session, id: "other" } },
    });
  });
  // Commit the new props before checking the replacement session. A pending
  // rerender cannot revoke an otherwise valid currently installed handler.
  view.rerender(<RelationBrowser sessionId="other" ownerTabId={ownerTabId} onClose={vi.fn()} />);
  await waitFor(() => {
    expect(screen.getByRole("option", { name: /S.Quoted Table/ }).getAttribute("value")).toBe(
      "replacement-opaque-target",
    );
  });
  expect(screen.queryByRole("button", { name: /^Select referenced table/ })).toBeNull();
  expect(screen.queryByRole("table", { name: "Key column mapping inventory" })).toBeNull();
  expect((screen.getByLabelText("Relation") as HTMLSelectElement).value).toBe("");
  expect((screen.getByLabelText("Find relation") as HTMLInputElement).value).toBe("Quoted Table");
  expect(
    (screen.getByRole("button", { name: "Inspect relation" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  expect(vi.mocked(client.call).mock.calls.slice(before)).toEqual([
    ["relation_catalog", { sessionId: "other" }],
  ]);
});

it.each([
  "refresh",
  "owner",
  "owner revision",
  "context",
  "transaction hold",
  "metadata hold",
  "DDL metadata hold",
  "schema hold",
  "capability",
])("rejects still-connected stale activation after %s before React commits", async (change) => {
  const { view, session } = renderReferenceNavigation();
  const button = await loadReferenceButton();
  const filter = screen.getByLabelText("Find relation") as HTMLInputElement;
  filter.focus();
  const bubbled = vi.fn();
  view.container.addEventListener("click", bubbled);
  act(() => {
    if (change === "refresh")
      fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
    else if (change === "owner") useStudio.setState({ tabs: [] });
    else if (change === "owner revision")
      useStudio.setState((state) => ({
        tabs: state.tabs.map((tab) => ({ ...tab, revision: tab.revision + 1 })),
      }));
    else if (change === "context") useStudio.setState({ sessionContextRevision: { s: 1 } });
    else if (change === "capability")
      useStudio.setState({
        sessions: {
          [session.connectionId]: {
            ...session,
            capabilities: { ...session.capabilities, relationBrowsing: false },
          },
        },
      });
    else {
      const key =
        change === "transaction hold"
          ? "sessionTransactionPending"
          : change === "metadata hold"
            ? "sessionMetadataRefreshRequired"
            : change === "DDL metadata hold"
              ? "sessionDdlCatalogRefreshRequired"
              : "sessionContextSwitching";
      useStudio.setState({ [key]: { s: true } });
      useStudio.setState({ [key]: {} });
    }
    // The old React handler is still installed: dispatch bubbles through the
    // mounted root before this outer batch may commit the invalidation.
    expect(button.isConnected).toBe(true);
    const before = vi.mocked(client.call).mock.calls.length;
    bubbled.mockClear();
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(button.isConnected).toBe(true);
    expect(bubbled).toHaveBeenCalledTimes(1);
    expect(client.call).toHaveBeenCalledTimes(before);
  });
  expect(filter.value).toBe("Quoted Table");
  expect(document.activeElement).not.toBe(screen.getByLabelText("Relation"));
  expect((screen.getByLabelText("Relation") as HTMLSelectElement).value).not.toBe(
    "opaque-qualified-destination",
  );
});

async function selectAndLoadChecks() {
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  expect(screen.getByText("CHECK constraints have not been fetched.")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "View CHECK constraints" }));
  return screen.findByRole("table", { name: "CHECK constraint inventory" });
}

it("views CHECK clauses independently as escaped multiline Unicode text with quoted identities", async () => {
  const { container } = render(<RelationBrowser sessionId="isolated-console" onClose={vi.fn()} />);
  const table = await selectAndLoadChecks();
  expect(client.call).toHaveBeenLastCalledWith("inspect_relation_check_constraints", {
    sessionId: "isolated-console",
    request: { revision: "catalog-1", targetId: "opaque-target" },
  });
  expect(client.call).toHaveBeenCalledTimes(2);
  expect(within(table).getByRole("rowheader").textContent).toBe('"DB"."S"."CHECK ""amount"');
  expect(within(table).getAllByRole("cell")[0].textContent).toBe('"DB"."S"."Quoted Table"');
  expect(table.querySelector("pre")?.textContent).toBe(checkConstraints.constraints[0].checkClause);
  expect(container.querySelector("img, script")).toBeNull();
  const region = screen.getByRole("region", { name: "Relation CHECK constraints" });
  expect(within(region).queryByRole("button")).toBeNull();
  expect(within(region).getByRole("note").textContent).toContain(
    "do not report enforcement or validation",
  );
  expect(within(region).getByRole("note").textContent).toContain("establish engine eligibility");
  expect(within(region).getByRole("note").textContent).toContain("grant row editing authority");
});

it("CHECK denial clears only its old result, preserves other views, and retries to explicit empty", async () => {
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await selectAndLoadChecks();
  fireEvent.click(screen.getByRole("button", { name: "View key constraints" }));
  const keys = await screen.findByRole("table", { name: "Key constraint inventory" });
  fireEvent.click(screen.getByRole("button", { name: "View key column mappings" }));
  const mappings = await screen.findByRole("table", { name: "Key column mapping inventory" });
  fireEvent.click(screen.getByRole("button", { name: "View definition" }));
  await screen.findByLabelText("Object DDL");
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  const columns = await screen.findByRole("table", { name: "Column inspection" });
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  const rows = await screen.findByRole("table", { name: "Read-only preview" });
  vi.mocked(client.call).mockRejectedValueOnce(new Error("Synthetic CHECK denied"));
  fireEvent.click(screen.getByRole("button", { name: "View CHECK constraints" }));
  const region = screen.getByRole("region", { name: "Relation CHECK constraints" });
  expect((await within(region).findByRole("alert")).textContent).toContain(
    "CHECK constraints are unavailable: Synthetic CHECK denied",
  );
  expect(within(region).queryByRole("table")).toBeNull();
  expect(within(region).queryByText(/No visible|not been fetched/)).toBeNull();
  expect(screen.getByRole("table", { name: "Key constraint inventory" })).toBe(keys);
  expect(screen.getByRole("table", { name: "Key column mapping inventory" })).toBe(mappings);
  expect(screen.getByRole("table", { name: "Column inspection" })).toBe(columns);
  expect(screen.getByRole("table", { name: "Read-only preview" })).toBe(rows);
  expect(screen.getByLabelText("Object DDL").textContent).toBe(definition.ddl);
  vi.mocked(client.call).mockResolvedValueOnce({ ...checkConstraints, constraints: [] });
  fireEvent.click(screen.getByRole("button", { name: "View CHECK constraints" }));
  await within(region).findByText("No visible CHECK constraints returned.");
  expect(within(region).queryByRole("alert")).toBeNull();
  expect(within(region).queryByRole("table")).toBeNull();
});

it.each([
  "revision",
  "targetId",
  "database",
  "schema",
  "name",
  "row catalog",
  "row schema",
  "row tableName",
])("rejects CHECK results with mismatched %s before publication", async (field) => {
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await selectAndLoadChecks();
  const result = structuredClone(checkConstraints);
  if (field === "schema" || field === "name") result.table[field] = "different";
  else if (field === "revision" || field === "targetId" || field === "database")
    result[field] = "different";
  else if (field === "row catalog") result.constraints[0].catalog = "different";
  else if (field === "row schema") result.constraints[0].schema = "different";
  else result.constraints[0].tableName = "different";
  vi.mocked(client.call).mockResolvedValueOnce(result);
  fireEvent.click(screen.getByRole("button", { name: "View CHECK constraints" }));
  const region = screen.getByRole("region", { name: "Relation CHECK constraints" });
  expect((await within(region).findByRole("alert")).textContent).toContain(
    "CHECK constraint target changed",
  );
  expect(within(region).queryByRole("table")).toBeNull();
});

function changeCheckContext(change: string) {
  if (change === "context") useStudio.setState({ sessionContextRevision: { s: 1 } });
  else if (change === "owner disappears") useStudio.setState({ tabs: [] });
  else {
    const key =
      change === "transaction hold"
        ? "sessionTransactionPending"
        : change === "metadata hold"
          ? "sessionMetadataRefreshRequired"
          : change === "DDL metadata hold"
            ? "sessionDdlCatalogRefreshRequired"
            : "sessionContextSwitching";
    useStudio.setState({ [key]: { s: true } });
    useStudio.setState({ [key]: {} });
  }
}

it.each(["transaction hold", "metadata hold", "DDL metadata hold", "schema hold"])(
  "does not dispatch a still-connected CHECK action after a transient %s",
  async (change) => {
    const { view } = renderReferenceNavigation();
    await loadReferenceButton();
    const button = screen.getByRole("button", { name: "View CHECK constraints" });
    const bubbled = vi.fn();
    view.container.addEventListener("click", bubbled);
    const before = vi.mocked(client.call).mock.calls.length;
    act(() => {
      changeCheckContext(change);
      expect(button.isConnected).toBe(true);
      button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      expect(button.isConnected).toBe(true);
      expect(bubbled).toHaveBeenCalledTimes(1);
      expect(client.call).toHaveBeenCalledTimes(before);
    });
    expect(screen.queryByRole("table", { name: "CHECK constraint inventory" })).toBeNull();
  },
);

it.each([
  "selection",
  "filter",
  "refresh",
  "context",
  "owner disappears",
  "session replacement",
  "reference navigation",
  "transaction hold",
  "metadata hold",
  "DDL metadata hold",
  "schema hold",
])("clears displayed CHECK clauses on %s", async (change) => {
  const { view, ownerTabId } = renderReferenceNavigation();
  const referenceButton = await loadReferenceButton();
  fireEvent.click(screen.getByRole("button", { name: "View CHECK constraints" }));
  await screen.findByRole("table", { name: "CHECK constraint inventory" });
  if (change === "selection")
    fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "" } });
  else if (change === "filter")
    fireEvent.change(screen.getByLabelText("Find relation"), { target: { value: "absent" } });
  else if (change === "refresh")
    fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
  else if (change === "reference navigation") {
    const before = vi.mocked(client.call).mock.calls.length;
    fireEvent.click(referenceButton);
    expect((screen.getByLabelText("Relation") as HTMLSelectElement).value).toBe(
      referencedTarget.targetId,
    );
    expect(screen.getByText("CHECK constraints have not been fetched.")).toBeTruthy();
    expect(client.call).toHaveBeenCalledTimes(before);
  } else if (change === "session replacement")
    view.rerender(<RelationBrowser sessionId="other" ownerTabId={ownerTabId} onClose={vi.fn()} />);
  else act(() => changeCheckContext(change));
  expect(screen.queryByRole("table", { name: "CHECK constraint inventory" })).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();
});

it.each(
  [
    "refresh",
    "context",
    "owner disappears",
    "session replacement",
    "transaction hold",
    "metadata hold",
    "DDL metadata hold",
    "schema hold",
  ].flatMap((change) => ["resolve", "reject"].map((outcome) => ({ change, outcome }))),
)("discards a held CHECK $outcome after $change", async ({ change, outcome }) => {
  const { view, ownerTabId } = renderReferenceNavigation();
  await loadReferenceButton();
  let release!: (value: RelationCheckConstraints) => void;
  let reject!: (error: Error) => void;
  const held = new Promise<RelationCheckConstraints>((resolve, fail) => {
    release = resolve;
    reject = fail;
  });
  vi.mocked(client.call).mockReturnValueOnce(held);
  fireEvent.click(screen.getByRole("button", { name: "View CHECK constraints" }));
  expect(client.call).toHaveBeenLastCalledWith("inspect_relation_check_constraints", {
    sessionId: "s",
    request: { revision: catalog.revision, targetId: "opaque-target" },
  });
  if (change === "refresh")
    fireEvent.click(screen.getByRole("button", { name: "Refresh relations" }));
  else if (change === "session replacement")
    view.rerender(<RelationBrowser sessionId="other" ownerTabId={ownerTabId} onClose={vi.fn()} />);
  else act(() => changeCheckContext(change));
  await act(async () => {
    if (outcome === "resolve") release(checkConstraints);
    else reject(new Error("Stale CHECK denial"));
    await held.catch(() => {});
  });
  expect(screen.queryByRole("table", { name: "CHECK constraint inventory" })).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();
});

function setupWritableSnowflakeGrid() {
  const profile: ConnectionProfile = {
    id: "sf-write-profile",
    name: "Snowflake Sandbox",
    groupId: "",
    environment: "staging",
    engine: "snowflake",
    host: "org-acct.snowflakecomputing.com",
    port: 443,
    database: "DB",
    username: "USER",
    tlsMode: "verifyFull",
    caCertPath: null,
    safeMode: "confirm",
    rememberPassword: false,
    storeQueryText: true,
    storeDrafts: true,
    ssh: null,
    snowflake: {
      authentication: "externalBrowser",
      role: "DEV_ROLE",
      warehouse: "DEV_WH",
      schema: "S",
      reviewedDml: true,
      reviewedDdl: false,
      reviewedConsole: false,
    },
  };
  const session: SessionInfo = {
    id: "s",
    connectionId: profile.id,
    engine: "snowflake",
    serverVersion: "8.40.0",
    latencyMs: 5,
    encrypted: true,
    capabilities: {
      sql: true,
      transactions: false,
      cancellation: true,
      tableBrowsing: false,
      relationBrowsing: true,
      snowflakeGridEditing: true,
      tableEditing: false,
      tableFiltering: false,
      processes: false,
    },
  };
  useStudio.setState({
    bootstrap: {
      profiles: [profile],
      groups: [],
      passphraseConfigured: false,
      supportedEngines: ["snowflake"],
      resultBudgetBytes: 67_108_864,
      rowLimit: 50_000,
      workspace: null,
      savedQueries: [],
      bookmarks: [],
      sqlFormatSettings: {
        keywordCase: "upper",
        indentWidth: 2,
        linesBetweenQueries: 2,
      },
      sqlSnippets: [],
    },
    sessions: { [profile.id]: session },
  });
  return { profile, session };
}

it("stages inline updates, deletes, and CSV inserts through ReviewDialog and resumes session metadata", async () => {
  setupWritableSnowflakeGrid();
  const editablePreview: RelationPreview = {
    ...preview,
    editSnapshotId: "snap-auto-1",
  };
  const preparedPreview: ChangePreview = {
    plan: {
      id: "grid-plan-auto",
      sessionId: "s",
      reviewedSnowflakeGrid: true,
      statements: [
        {
          sql: 'DELETE FROM "DB"."S"."Quoted Table" WHERE ...',
          kind: "write",
          missingWhere: false,
        },
        { sql: 'UPDATE "DB"."S"."Quoted Table" SET ...', kind: "write", missingWhere: false },
        {
          sql: 'INSERT INTO "DB"."S"."Quoted Table" VALUES (...)',
          kind: "write",
          missingWhere: false,
        },
      ],
      warnings: ["Generated edits target the fully qualified SQL name shown above."],
      requiresApproval: true,
      requiresPassphrase: false,
      confirmationText: null,
      expiresAt: 1_900_000_000_000,
    },
    rowCount: 3,
    sql: [
      'DELETE FROM "DB"."S"."Quoted Table" WHERE "exact" IS NULL',
      'UPDATE "DB"."S"."Quoted Table" SET "exact" = CAST(? AS NUMBER(38,0))',
      'INSERT INTO "DB"."S"."Quoted Table" ("exact") VALUES (CAST(? AS NUMBER(38,0)))',
    ],
    bindings: [[], ["100"], ["200"]],
  };
  const autoCommitted: SnowflakeGridResult = {
    sessionId: "s",
    operationId: "op-auto-1",
    outcome: "committed",
    transactionId: null,
    recordingError: null,
    metadataRefreshRequired: true,
    retired: false,
  };
  const resumedOptions: SessionContextOptions = {
    state: {
      sessionId: "s",
      transactionMode: "autoCommit",
      transactionState: "idle",
      contextKind: "schema",
      activeContext: "S",
      database: "DB",
      role: "DEV_ROLE",
      searchPath: ["S"],
      contextFingerprint: "fp-resumed",
      transactionObservation: {
        status: "observed",
        transactionId: null,
        abortOnError: false,
        usability: "unknown",
      },
    },
    available: ["S"],
    revision: "ctx-rev-1",
  };

  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "inspect_relation") return inspection;
    if (method === "preview_relation") return editablePreview;
    if (method === "prepare_snowflake_grid_plan") return preparedPreview;
    if (method === "authorize_plan") return "approval-auto-1";
    if (method === "execute_snowflake_grid_plan") return autoCommitted;
    if (method === "session_contexts") return resumedOptions;
    throw new Error(`Unexpected ${method}`);
  });

  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByText("1 columns");
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  await screen.findByRole("table", { name: "Editable Snowflake preview" });

  fireEvent.change(screen.getByLabelText("Row 1 exact"), { target: { value: "100" } });
  fireEvent.click(screen.getByRole("button", { name: "Mark row 2 for delete" }));
  fireEvent.click(screen.getByRole("button", { name: "Import CSV rows…" }));
  fireEvent.change(screen.getByLabelText("CSV rows to stage"), {
    target: { value: "exact\n200\n" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Stage CSV rows" }));

  const reviewButton = screen.getByRole("button", { name: "Review 3 changes…" });
  expect((reviewButton as HTMLButtonElement).disabled).toBe(false);
  fireEvent.click(reviewButton);

  await screen.findByText("Review pending changes");
  expect(client.call).toHaveBeenCalledWith("prepare_snowflake_grid_plan", {
    request: {
      sessionId: "s",
      transactionId: null,
      preview: {
        revision: "catalog-1",
        targetId: "opaque-target",
        inspectionRevision: "inspection-1",
      },
      snapshotId: "snap-auto-1",
      mutations: [
        { kind: "delete", rowIndex: 1 },
        {
          kind: "update",
          rowIndex: 0,
          changes: [{ columnIndex: 0, value: { kind: "text", value: "100" } }],
        },
        {
          kind: "insert",
          values: [{ columnIndex: 0, value: { kind: "text", value: "200" } }],
        },
      ],
      mode: "automatic",
      rollbackPrior: false,
    },
  });

  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));
  await screen.findByRole("button", { name: "Resume session metadata" });
  expect(useStudio.getState().sessionMetadataRefreshRequired.s).toBe(true);

  fireEvent.click(screen.getByRole("button", { name: "Resume session metadata" }));
  await screen.findByText("1 visible relations · DB");
  expect(client.call).toHaveBeenCalledWith("session_contexts", {
    sessionId: "s",
    resumeMetadata: true,
  });
  expect(useStudio.getState().sessionMetadataRefreshRequired.s).toBe(false);
});

it("supports manual Snowflake grid transactions across pending preview reload, continuation, and commit", async () => {
  setupWritableSnowflakeGrid();
  const initialPreview: RelationPreview = {
    ...preview,
    editSnapshotId: "snap-manual-1",
  };
  const pendingPreview: RelationPreview = {
    columns: [column],
    rows: [["555"], [null], [""]],
    truncated: false,
    editSnapshotId: "snap-manual-2",
  };
  let prepareCalls = 0;
  let executeCalls = 0;
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "inspect_relation") return inspection;
    if (method === "preview_relation") return initialPreview;
    if (method === "preview_snowflake_grid_transaction_relation") return pendingPreview;
    if (method === "prepare_snowflake_grid_plan") {
      prepareCalls += 1;
      return {
        plan: {
          id: `manual-plan-${prepareCalls}`,
          sessionId: "s",
          reviewedSnowflakeGrid: true,
          statements: [{ sql: "UPDATE ...", kind: "write", missingWhere: false }],
          warnings: [],
          requiresApproval: true,
          requiresPassphrase: false,
          confirmationText: null,
          expiresAt: 1_900_000_000_000,
        },
        rowCount: 1,
        sql: ["UPDATE ..."],
        bindings: [["555"]],
      } satisfies ChangePreview;
    }
    if (method === "authorize_plan") return `approval-${prepareCalls}`;
    if (method === "execute_snowflake_grid_plan") {
      executeCalls += 1;
      return {
        sessionId: "s",
        operationId: `op-manual-${executeCalls}`,
        outcome: "pending",
        transactionId: "tx-manual-99",
        recordingError: null,
        metadataRefreshRequired: true,
        retired: false,
      } satisfies SnowflakeGridResult;
    }
    if (method === "finish_snowflake_grid_transaction") {
      return {
        sessionId: "s",
        operationId: "op-finish-commit",
        outcome: "committed",
        transactionId: "tx-manual-99",
        recordingError: null,
        metadataRefreshRequired: true,
        retired: false,
      } satisfies SnowflakeGridResult;
    }
    throw new Error(`Unexpected ${method}`);
  });

  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByText("1 columns");
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  await screen.findByRole("table", { name: "Editable Snowflake preview" });

  fireEvent.change(screen.getByLabelText("Grid transaction mode"), {
    target: { value: "manual" },
  });
  fireEvent.change(screen.getByLabelText("Row 1 exact"), { target: { value: "555" } });
  fireEvent.click(screen.getByRole("button", { name: "Review 1 change…" }));
  await screen.findByText("Review pending changes");
  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));

  const banner = await screen.findByRole("region", { name: "Pending Snowflake grid transaction" });
  expect(within(banner).getByText(/tx-manual-99/)).toBeTruthy();
  expect(
    (screen.getByRole("button", { name: "Refresh relations" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  expect((screen.getByRole("button", { name: "Preview rows" }) as HTMLButtonElement).disabled).toBe(
    true,
  );
  expect(screen.getByText("1 visible relations · DB")).toBeTruthy();

  fireEvent.click(within(banner).getByRole("button", { name: "Reload pending preview" }));
  await screen.findByRole("table", { name: "Editable Snowflake preview" });
  expect(client.call).toHaveBeenCalledWith("preview_snowflake_grid_transaction_relation", {
    transactionId: "tx-manual-99",
    request: {
      revision: "catalog-1",
      targetId: "opaque-target",
      inspectionRevision: "inspection-1",
    },
  });
  expect((screen.getByLabelText("Row 1 exact") as HTMLInputElement).value).toBe("555");

  fireEvent.click(screen.getByRole("button", { name: "Mark row 2 for delete" }));
  fireEvent.click(screen.getByRole("button", { name: "Review 1 change…" }));
  await screen.findByText("Review pending changes");
  expect(client.call).toHaveBeenLastCalledWith("prepare_snowflake_grid_plan", {
    request: {
      sessionId: "s",
      transactionId: "tx-manual-99",
      preview: {
        revision: "catalog-1",
        targetId: "opaque-target",
        inspectionRevision: "inspection-1",
      },
      snapshotId: "snap-manual-2",
      mutations: [{ kind: "delete", rowIndex: 1 }],
      mode: "manual",
      rollbackPrior: true,
    },
  });
  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));
  await screen.findByRole("region", { name: "Pending Snowflake grid transaction" });

  fireEvent.click(screen.getByRole("button", { name: "Commit transaction" }));
  await screen.findByRole("button", { name: "Resume session metadata" });
  expect(client.call).toHaveBeenCalledWith("finish_snowflake_grid_transaction", {
    transactionId: "tx-manual-99",
    action: "commit",
  });
  expect(useStudio.getState().sessionSnowflakeGridTransactions.s).toBeUndefined();
});

it("disables continuation and commit when recordingError is set while preserving protected Rollback", async () => {
  setupWritableSnowflakeGrid();
  useStudio.setState({
    sessionSnowflakeGridTransactions: {
      s: {
        transactionId: "tx-blocked-1",
        recordingError: "SQLite disk full during batch capture",
      },
    },
  });
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "finish_snowflake_grid_transaction") {
      return {
        sessionId: "s",
        operationId: "op-rollback-blocked",
        outcome: "rolledBack",
        transactionId: "tx-blocked-1",
        recordingError: "SQLite disk full during batch capture",
        metadataRefreshRequired: true,
        retired: false,
      } satisfies SnowflakeGridResult;
    }
    throw new Error(`Unexpected ${method}`);
  });

  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  const banner = await screen.findByRole("region", { name: "Pending Snowflake grid transaction" });
  expect(within(banner).getByRole("alert").textContent).toMatch(/SQLite disk full/);
  expect(
    (within(banner).getByRole("button", { name: "Reload pending preview" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(
    (within(banner).getByRole("button", { name: "Commit transaction" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  const rollbackBtn = within(banner).getByRole("button", {
    name: "Rollback transaction",
  }) as HTMLButtonElement;
  expect(rollbackBtn.disabled).toBe(false);

  fireEvent.click(rollbackBtn);
  await screen.findByRole("button", { name: "Resume session metadata" });
  expect(client.call).toHaveBeenCalledWith("finish_snowflake_grid_transaction", {
    transactionId: "tx-blocked-1",
    action: "rollback",
  });
  expect(useStudio.getState().sessionSnowflakeGridTransactions.s).toBeUndefined();
});

it("restores a fresh edit snapshot when cancelling ReviewDialog without losing staged edits", async () => {
  setupWritableSnowflakeGrid();
  const initialPreview: RelationPreview = {
    ...preview,
    editSnapshotId: "snap-cancel-1",
  };
  let prepareCalls = 0;
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "inspect_relation") return inspection;
    if (method === "preview_relation") return initialPreview;
    if (method === "prepare_snowflake_grid_plan") {
      prepareCalls += 1;
      return {
        plan: {
          id: `cancel-plan-${prepareCalls}`,
          sessionId: "s",
          reviewedSnowflakeGrid: true,
          statements: [{ sql: "UPDATE ...", kind: "write", missingWhere: false }],
          warnings: [],
          requiresApproval: true,
          requiresPassphrase: false,
          confirmationText: null,
          expiresAt: 1_900_000_000_000,
        },
        rowCount: 1,
        sql: ["UPDATE ..."],
        bindings: [["777"]],
      } satisfies ChangePreview;
    }
    if (method === "cancel_snowflake_grid_plan") return "snap-cancel-restored-2";
    if (method === "authorize_plan") return "approval-after-cancel";
    if (method === "execute_snowflake_grid_plan") {
      return {
        sessionId: "s",
        operationId: "op-after-cancel",
        outcome: "committed",
        transactionId: null,
        recordingError: null,
        metadataRefreshRequired: true,
        retired: false,
      } satisfies SnowflakeGridResult;
    }
    throw new Error(`Unexpected ${method}`);
  });

  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByText("1 columns");
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  await screen.findByRole("table", { name: "Editable Snowflake preview" });

  fireEvent.change(screen.getByLabelText("Row 1 exact"), { target: { value: "777" } });
  fireEvent.click(screen.getByRole("button", { name: "Review 1 change…" }));
  await screen.findByText("Review pending changes");

  // Cancel the ReviewDialog via the Cancel button.
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await screen.findByRole("table", { name: "Editable Snowflake preview" });
  expect(client.call).toHaveBeenCalledWith("cancel_snowflake_grid_plan", {
    planId: "cancel-plan-1",
  });
  // Staged value remains intact and the restored snapshot re-enables Review.
  expect((screen.getByLabelText("Row 1 exact") as HTMLInputElement).value).toBe("777");
  const reviewAgain = await screen.findByRole("button", { name: "Review 1 change…" });
  await vi.waitFor(() => {
    expect((reviewAgain as HTMLButtonElement).disabled).toBe(false);
  });

  fireEvent.click(reviewAgain);
  await screen.findByText("Review pending changes");
  expect(client.call).toHaveBeenLastCalledWith("prepare_snowflake_grid_plan", {
    request: {
      sessionId: "s",
      transactionId: null,
      preview: {
        revision: "catalog-1",
        targetId: "opaque-target",
        inspectionRevision: "inspection-1",
      },
      snapshotId: "snap-cancel-restored-2",
      mutations: [
        {
          kind: "update",
          rowIndex: 0,
          changes: [{ columnIndex: 0, value: { kind: "text", value: "777" } }],
        },
      ],
      mode: "automatic",
      rollbackPrior: false,
    },
  });

  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));
  await screen.findByRole("button", { name: "Resume session metadata" });
  // Applying changes must NOT invoke cancel_snowflake_grid_plan for cancel-plan-2.
  expect(client.call).not.toHaveBeenCalledWith("cancel_snowflake_grid_plan", {
    planId: "cancel-plan-2",
  });
});

it("applies server sort, filter, and 500-row offset pagination and binds edits to the active slice", async () => {
  setupWritableSnowflakeGrid();
  let previewCalls = 0;
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "inspect_relation") return inspection;
    if (method === "preview_relation") {
      previewCalls += 1;
      return {
        ...preview,
        truncated: previewCalls === 1,
        offset: previewCalls === 2 ? 500 : undefined,
        editSnapshotId: `snap-slice-${previewCalls}`,
      } satisfies RelationPreview;
    }
    if (method === "prepare_snowflake_grid_plan") {
      return {
        plan: {
          id: "sliced-plan-1",
          sessionId: "s",
          reviewedSnowflakeGrid: true,
          statements: [{ sql: "UPDATE ...", kind: "write", missingWhere: false }],
          warnings: [],
          requiresApproval: true,
          requiresPassphrase: false,
          confirmationText: null,
          expiresAt: 1_900_000_000_000,
        },
        rowCount: 1,
        sql: ["UPDATE ..."],
        bindings: [["888"]],
      } satisfies ChangePreview;
    }
    if (method === "cancel_snowflake_grid_plan") return "snap-slice-2";
    throw new Error(`Unexpected ${method}`);
  });

  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByText("1 columns");

  fireEvent.change(screen.getByLabelText("Preview sort column"), { target: { value: "exact" } });
  fireEvent.change(screen.getByLabelText("Preview sort direction"), {
    target: { value: "desc" },
  });
  fireEvent.change(screen.getByLabelText("Preview sort nulls"), { target: { value: "last" } });
  fireEvent.click(screen.getByRole("button", { name: "Add filter" }));
  fireEvent.change(screen.getByLabelText("Filter 1 operator"), { target: { value: "contains" } });
  fireEvent.change(screen.getByLabelText("Filter 1 value"), { target: { value: "9007" } });
  fireEvent.click(screen.getByRole("button", { name: "Apply sort & filters" }));

  await screen.findByRole("table", { name: "Editable Snowflake preview" });
  expect(client.call).toHaveBeenCalledWith("preview_relation", {
    sessionId: "s",
    request: {
      revision: "catalog-1",
      targetId: "opaque-target",
      inspectionRevision: "inspection-1",
      sort: { column: "exact", direction: "desc", nulls: "last" },
      filters: [{ column: "exact", operator: "contains", value: "9007" }],
    },
  });

  // Advance to the next 500-row server window and verify prepared edits carry the active slice.
  fireEvent.click(screen.getByRole("button", { name: "Next server page" }));
  await screen.findByText(/server offset 500/i);
  expect(client.call).toHaveBeenCalledWith("preview_relation", {
    sessionId: "s",
    request: {
      revision: "catalog-1",
      targetId: "opaque-target",
      inspectionRevision: "inspection-1",
      offset: 500,
      sort: { column: "exact", direction: "desc", nulls: "last" },
      filters: [{ column: "exact", operator: "contains", value: "9007" }],
    },
  });

  fireEvent.change(screen.getByLabelText("Row 1 exact"), { target: { value: "888" } });
  fireEvent.click(screen.getByRole("button", { name: "Review 1 change…" }));
  await screen.findByText("Review pending changes");
  expect(client.call).toHaveBeenLastCalledWith("prepare_snowflake_grid_plan", {
    request: {
      sessionId: "s",
      transactionId: null,
      preview: {
        revision: "catalog-1",
        targetId: "opaque-target",
        inspectionRevision: "inspection-1",
        offset: 500,
        sort: { column: "exact", direction: "desc", nulls: "last" },
        filters: [{ column: "exact", operator: "contains", value: "9007" }],
      },
      snapshotId: "snap-slice-2",
      mutations: [
        {
          kind: "update",
          rowIndex: 0,
          changes: [{ columnIndex: 0, value: { kind: "text", value: "888" } }],
        },
      ],
      mode: "automatic",
      rollbackPrior: false,
    },
  });
});

it("imports >500 CSV rows across reviewed 500-row batches inside one manual Snowflake transaction", async () => {
  setupWritableSnowflakeGrid();
  let prepareCalls = 0;
  let executeCalls = 0;
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "inspect_relation") return inspection;
    if (method === "preview_relation") {
      return { ...preview, editSnapshotId: "snap-csv-batch-1" } satisfies RelationPreview;
    }
    if (method === "preview_snowflake_grid_transaction_relation") {
      return { ...preview, editSnapshotId: "snap-csv-batch-2" } satisfies RelationPreview;
    }
    if (method === "prepare_snowflake_grid_plan") {
      prepareCalls += 1;
      return {
        plan: {
          id: `csv-plan-${prepareCalls}`,
          sessionId: "s",
          reviewedSnowflakeGrid: true,
          statements: [{ sql: "INSERT INTO ...", kind: "write", missingWhere: false }],
          warnings: [],
          requiresApproval: true,
          requiresPassphrase: false,
          confirmationText: null,
          expiresAt: 1_900_000_000_000,
        },
        rowCount: prepareCalls === 1 ? 500 : 2,
        sql: ["INSERT INTO ..."],
        bindings: [["1"]],
      } satisfies ChangePreview;
    }
    if (method === "authorize_plan") return `csv-approval-${prepareCalls}`;
    if (method === "execute_snowflake_grid_plan") {
      executeCalls += 1;
      return {
        sessionId: "s",
        operationId: `op-csv-${executeCalls}`,
        outcome: "pending",
        transactionId: "tx-csv-multi-1",
        recordingError: null,
        metadataRefreshRequired: false,
        retired: false,
      } satisfies SnowflakeGridResult;
    }
    if (method === "finish_snowflake_grid_transaction") {
      return {
        sessionId: "s",
        operationId: "op-csv-commit",
        outcome: "committed",
        transactionId: null,
        recordingError: null,
        metadataRefreshRequired: true,
        retired: false,
      } satisfies SnowflakeGridResult;
    }
    throw new Error(`Unexpected ${method}`);
  });

  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByText("1 columns");
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  await screen.findByRole("table", { name: "Editable Snowflake preview" });

  fireEvent.click(screen.getByRole("button", { name: "Import CSV rows…" }));
  const csvRows = Array.from({ length: 502 }, (_, idx) => `${idx + 1}`).join("\n");
  fireEvent.change(screen.getByLabelText("CSV rows to stage"), {
    target: { value: `exact\n${csvRows}` },
  });
  fireEvent.click(screen.getByRole("button", { name: "Stage CSV rows" }));

  // Batch 1 of 2 (500 rows) is staged in forced manual mode.
  await screen.findByText(/Batch 1 of 2 staged/i);
  fireEvent.click(screen.getByRole("button", { name: "Review batch 1/2 (500 rows)…" }));
  await screen.findByText("Review pending changes");
  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));

  // Pending preview auto-reloads and Batch 2 of 2 (2 rows) is ready for review.
  const reviewBatch2 = await screen.findByRole("button", { name: "Review batch 2/2 (2 rows)…" });
  await vi.waitFor(() => {
    expect((reviewBatch2 as HTMLButtonElement).disabled).toBe(false);
  });
  fireEvent.click(reviewBatch2);
  await screen.findByText("Review pending changes");
  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));

  // After all batches execute, Commit transaction commits the manual transaction atomically.
  const commitBtn = await screen.findByRole("button", { name: "Commit transaction" });
  await vi.waitFor(() => {
    expect((commitBtn as HTMLButtonElement).disabled).toBe(false);
  });
  fireEvent.click(commitBtn);
  await screen.findByRole("button", { name: "Resume session metadata" });
  expect(executeCalls).toBe(2);
  expect(client.call).toHaveBeenCalledWith("finish_snowflake_grid_transaction", {
    transactionId: "tx-csv-multi-1",
    action: "commit",
  });
});

it("clears the consumed editSnapshotId when a manual continuation batch returns rejected", async () => {
  setupWritableSnowflakeGrid();
  let prepareCalls = 0;
  let executeCalls = 0;
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "inspect_relation") return inspection;
    if (method === "preview_relation") {
      return { ...preview, editSnapshotId: "snap-init-1" } satisfies RelationPreview;
    }
    if (method === "preview_snowflake_grid_transaction_relation") {
      return { ...preview, editSnapshotId: "snap-cont-1" } satisfies RelationPreview;
    }
    if (method === "prepare_snowflake_grid_plan") {
      prepareCalls += 1;
      return {
        plan: {
          id: `cont-plan-${prepareCalls}`,
          sessionId: "s",
          reviewedSnowflakeGrid: true,
          statements: [{ sql: "UPDATE ...", kind: "write", missingWhere: false }],
          warnings: [],
          requiresApproval: true,
          requiresPassphrase: false,
          confirmationText: null,
          expiresAt: 1_900_000_000_000,
        },
        rowCount: 1,
        sql: ["UPDATE ..."],
        bindings: [["555"]],
      } satisfies ChangePreview;
    }
    if (method === "authorize_plan") return `cont-approval-${prepareCalls}`;
    if (method === "execute_snowflake_grid_plan") {
      executeCalls += 1;
      return {
        sessionId: "s",
        operationId: `op-cont-${executeCalls}`,
        outcome: executeCalls === 1 ? "pending" : "rejected",
        transactionId: "tx-open-cont",
        recordingError: null,
        metadataRefreshRequired: false,
        retired: false,
      } satisfies SnowflakeGridResult;
    }
    throw new Error(`Unexpected ${method}`);
  });

  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByText("1 columns");
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  await screen.findByRole("table", { name: "Editable Snowflake preview" });

  // Batch 1 opens the manual Snowflake grid transaction (outcome: pending).
  fireEvent.change(screen.getByLabelText("Grid transaction mode"), {
    target: { value: "manual" },
  });
  fireEvent.change(screen.getByLabelText("Row 1 exact"), { target: { value: "554" } });
  fireEvent.click(screen.getByRole("button", { name: "Review 1 change…" }));
  await screen.findByText("Review pending changes");
  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));

  // Reload pending preview (snap-cont-1), then submit a continuation batch that returns rejected.
  const banner = await screen.findByRole("region", { name: "Pending Snowflake grid transaction" });
  fireEvent.click(within(banner).getByRole("button", { name: "Reload pending preview" }));
  await vi.waitFor(() => {
    expect(screen.queryByText(/Click Reload pending preview above/i)).toBeNull();
  });

  fireEvent.change(screen.getByLabelText("Row 1 exact"), { target: { value: "555" } });
  fireEvent.click(screen.getByRole("button", { name: "Review 1 change…" }));
  await screen.findByText("Review pending changes");
  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));

  // After a rejected continuation, the consumed snapshot must be cleared so the table
  // reverts to read-only until Reload pending preview is clicked, rather than reusing snap-cont-1.
  await screen.findByText(
    /Preview edit snapshot was consumed\. Click Reload pending preview above/i,
  );
  expect(screen.getByRole("table", { name: "Read-only preview" })).toBeTruthy();
  expect(screen.queryByRole("table", { name: "Editable Snowflake preview" })).toBeNull();
});

it("inspects VARIANT :path trees, BINARY hex dumps, and GEOGRAPHY spatial previews and applies staged JSON edits", async () => {
  setupWritableSnowflakeGrid();
  const richInspection: RelationInspection = {
    ...inspection,
    columns: [
      {
        ...inspection.columns[0],
        name: "PAYLOAD",
        ordinal: 1,
        fullType: "VARIANT",
        dataType: "VARIANT",
        resultKind: "json",
        nullable: true,
      },
      {
        ...inspection.columns[0],
        name: "RAW_HASH",
        ordinal: 2,
        fullType: "BINARY(7)",
        dataType: "BINARY",
        resultKind: "binary",
        nullable: true,
      },
      {
        ...inspection.columns[0],
        name: "REGION_GEO",
        ordinal: 3,
        fullType: "GEOGRAPHY",
        dataType: "GEOGRAPHY",
        resultKind: "text",
        nullable: true,
      },
    ],
  };
  const richPreview: RelationPreview = {
    columns: [
      {
        id: "c0",
        name: "PAYLOAD",
        databaseType: "VARIANT",
        kind: "json",
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: false,
        insertable: false,
        hasDefault: false,
      },
      {
        id: "c1",
        name: "RAW_HASH",
        databaseType: "BINARY(7)",
        kind: "binary",
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: false,
        insertable: false,
        hasDefault: false,
      },
      {
        id: "c2",
        name: "REGION_GEO",
        databaseType: "TEXT",
        kind: "text",
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: false,
        insertable: false,
        hasDefault: false,
      },
    ],
    rows: [
      [
        '{"customer":{"orders":[{"id":42}]}}',
        "4A6F696E657279",
        '{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,3],[0,3],[0,0]]]}',
      ],
    ],
    truncated: false,
    editSnapshotId: "snap-rich-1",
  };
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "inspect_relation") return richInspection;
    if (method === "preview_relation") return richPreview;
    throw new Error(`Unexpected ${method}`);
  });

  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "Inspect relation" }));
  await screen.findByText("3 columns");
  fireEvent.click(screen.getByRole("button", { name: "Preview rows" }));
  await screen.findByRole("table", { name: "Editable Snowflake preview" });

  // 1. VARIANT :path tree + Format JSON + Apply to staged cell
  fireEvent.click(screen.getByRole("button", { name: "Inspect row 1 PAYLOAD" }));
  const treeViewer = await screen.findByTestId("json-tree-viewer");
  expect(within(treeViewer).getByText("PAYLOAD:customer.orders[0].id")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Copy path PAYLOAD:customer.orders[0].id" }));
  fireEvent.click(screen.getByRole("button", { name: "Format JSON" }));
  fireEvent.click(screen.getByRole("button", { name: "Apply to staged cell" }));
  await screen.findByRole("button", { name: "Review 1 change…" });

  // 2. BINARY raw hex dump
  fireEvent.click(screen.getByRole("button", { name: "Inspect row 1 RAW_HASH" }));
  const hexViewer = await screen.findByTestId("binary-hex-viewer");
  expect(hexViewer.textContent).toContain("|Joinery|");
  fireEvent.click(screen.getByRole("button", { name: "Back to preview" }));

  // 3. Writable GEOGRAPHY spatial 2D SVG preview and staged WKT edit
  fireEvent.click(screen.getByRole("button", { name: "Inspect row 1 REGION_GEO" }));
  const spatialViewer = await screen.findByTestId("spatial-viewer");
  expect(within(spatialViewer).getByText("Polygon")).toBeTruthy();
  expect(screen.getByTestId("spatial-svg-preview")).toBeTruthy();
  fireEvent.click(screen.getByRole("tab", { name: "Raw" }));
  fireEvent.change(screen.getByLabelText("Cell value"), {
    target: { value: "SRID=4326;POINT(-122.4194 37.7749)" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Apply to staged cell" }));
  await screen.findByRole("button", { name: "Review 1 change…" });
  expect(screen.getByRole("table", { name: "Editable Snowflake preview" })).toBeTruthy();
});
