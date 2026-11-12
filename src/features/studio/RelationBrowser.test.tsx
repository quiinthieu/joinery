import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { StrictMode } from "react";
import type {
  BrowseCatalog,
  ColumnInfo,
  RelationConstraintMappings,
  RelationConstraints,
  RelationDefinition,
  RelationInspection,
  RelationPreview,
  SessionInfo,
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
