import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { StrictMode } from "react";
import type {
  BrowseCatalog,
  ColumnInfo,
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
  expect(screen.getByText(/Constraints have not been fetched/)).toBeTruthy();
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

it.each([
  "transaction",
  "transientTransaction",
  "metadata",
  "ddlCatalog",
  "schemaSwitch",
  "context",
])("suppresses a pending definition after %s changes the console context", async (change) => {
  installSchemaAuthority();
  let release!: (value: RelationDefinition) => void;
  const held = new Promise<RelationDefinition>((resolve) => {
    release = resolve;
  });
  vi.mocked(client.call).mockImplementation(async (method) => {
    if (method === "relation_catalog") return catalog;
    if (method === "define_relation") return held;
    throw new Error(`Unexpected ${method}`);
  });
  render(<RelationBrowser sessionId="s" onClose={vi.fn()} />);
  await screen.findByText("1 visible relations · DB");
  fireEvent.change(screen.getByLabelText("Relation"), { target: { value: "opaque-target" } });
  fireEvent.click(screen.getByRole("button", { name: "View definition" }));
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
    release(definition);
    await held;
  });
  expect(screen.queryByLabelText("Object DDL")).toBeNull();
  expect(screen.queryByRole("button", { name: "Copy DDL" })).toBeNull();
  expect(
    (screen.getByRole("button", { name: "View definition" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  expect(client.call).toHaveBeenCalledTimes(2);
});

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
