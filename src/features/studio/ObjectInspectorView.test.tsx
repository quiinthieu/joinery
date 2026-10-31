import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ObjectDetails, SessionInfo, TableSchema } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";
import { DdlPanel, ObjectInspectorView } from "./ObjectInspectorView";
import { StructureView } from "./UtilityViews";

function makeSession(objectExplorer: boolean | undefined): SessionInfo {
  return {
    id: "sess-1",
    connectionId: "conn-1",
    engine: "postgres",
    serverVersion: "PostgreSQL 16.4",
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
      ...(objectExplorer === undefined ? {} : { objectExplorer }),
    },
  };
}

describe("StructureView, DdlPanel, and ObjectInspectorView", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useStudio.setState({
      tabs: [],
      activeTabId: null,
      sessions: {},
      catalog: {},
      objects: {},
      selectedConnectionId: "conn-1",
    });
  });

  it("renders table constraints, expression/partial indexes, triggers, and clickable dependencies", () => {
    const openObjectSpy = vi.fn().mockResolvedValue(undefined);
    const openTableSpy = vi.fn().mockResolvedValue(undefined);
    useStudio.setState({
      openObject: openObjectSpy,
      openTable: openTableSpy,
    });

    const schema: TableSchema = {
      table: { schema: "public", name: "orders" },
      columns: [
        {
          id: "id",
          name: "id",
          databaseType: "bigint",
          kind: "integer",
          nullable: false,
          primaryKey: true,
          foreignKey: false,
          editable: true,
          insertable: true,
          hasDefault: true,
        },
      ],
      primaryKey: ["id"],
      foreignKeys: [],
      editable: true,
    };

    const details: ObjectDetails = {
      object: { schema: "public", name: "orders", kind: "table", identity: null },
      ownerTable: null,
      comment: null,
      ddl: { status: "available", ddl: 'CREATE TABLE "public"."orders" ();', truncated: false },
      constraints: [
        {
          name: "orders_total_positive",
          kind: "check",
          columns: ["total_cents"],
          target: null,
          targetColumns: [],
          definition: "CHECK ((total_cents >= 0))",
        },
        {
          name: "orders_customer_fkey",
          kind: "foreignKey",
          columns: ["customer_id"],
          target: { schema: "public", name: "customers" },
          targetColumns: ["id"],
          definition: "FOREIGN KEY (customer_id) REFERENCES customers(id)",
        },
      ],
      indexes: [
        {
          name: "orders_lower_note_idx",
          method: "btree",
          unique: false,
          primary: false,
          expressions: ["lower(note)"],
          predicate: "(status = 'open'::text)",
          definition: "CREATE INDEX orders_lower_note_idx ON public.orders (lower(note))",
        },
      ],
      triggers: [
        {
          name: "orders_audit_trg",
          tableName: "orders",
          timing: "AFTER",
          events: ["INSERT", "UPDATE"],
          orientation: "ROW",
          enabled: true,
          definition: "CREATE TRIGGER orders_audit_trg ...",
        },
      ],
      routine: null,
      sequence: null,
      uses: [
        {
          object: { schema: "public", name: "customers", kind: "table", identity: null },
          relation: "foreign key",
        },
      ],
      usedBy: [
        {
          object: { schema: "public", name: "active_orders_v", kind: "view", identity: null },
          relation: "view dependency",
        },
      ],
    };

    render(<StructureView schema={schema} details={details} connectionId="conn-1" />);

    expect(screen.getByRole("table", { name: "Constraints" })).toBeTruthy();
    expect(screen.getByText("CHECK ((total_cents >= 0))")).toBeTruthy();
    expect(screen.getByRole("table", { name: "Indexes" })).toBeTruthy();
    expect(screen.getByText("lower(note)")).toBeTruthy();
    expect(screen.getByText("WHERE (status = 'open'::text)")).toBeTruthy();
    expect(screen.getByRole("table", { name: "Triggers" })).toBeTruthy();
    expect(screen.getByText("INSERT OR UPDATE")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "public.customers (id)" }));
    expect(openTableSpy).toHaveBeenCalledWith(
      "conn-1",
      { schema: "public", name: "customers" },
      undefined,
      "structure",
    );

    fireEvent.click(screen.getByRole("button", { name: "orders_lower_note_idx" }));
    expect(openObjectSpy).toHaveBeenCalledWith(
      "conn-1",
      {
        schema: "public",
        name: "orders_lower_note_idx",
        kind: "index",
        identity: "orders",
      },
      "structure",
    );

    fireEvent.click(screen.getByRole("button", { name: /public\.active_orders_v/ }));
    expect(openObjectSpy).toHaveBeenCalledWith(
      "conn-1",
      {
        schema: "public",
        name: "active_orders_v",
        kind: "view",
        identity: null,
      },
      "structure",
    );
  });

  it("renders truncated DDL, opens DDL in SQL editor, and handles unavailable DDL", () => {
    const { rerender } = render(
      <DdlPanel
        connectionId="conn-1"
        ddlStatus={{
          status: "available",
          ddl: "CREATE OR REPLACE FUNCTION public.r3_calc(x integer) RETURNS integer AS $$ SELECT x; $$ LANGUAGE sql;",
          truncated: true,
        }}
      />,
    );

    expect(screen.getByText("Truncated to 256 KiB preview limit")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Open in SQL editor" }));
    expect(useStudio.getState().tabs).toHaveLength(1);
    expect(useStudio.getState().tabs[0]?.sql).toContain(
      "CREATE OR REPLACE FUNCTION public.r3_calc",
    );

    rerender(
      <DdlPanel
        connectionId="conn-1"
        ddlStatus={{
          status: "unavailable",
          reason: "Insufficient privileges to inspect routine body",
        }}
      />,
    );
    expect(screen.getByText("DDL unavailable")).toBeTruthy();
    expect(screen.getByText("Insufficient privileges to inspect routine body")).toBeTruthy();
  });

  it("fetches object_details only when objectExplorer capability is enabled", async () => {
    const callSpy = vi.spyOn(client, "call").mockResolvedValue({
      object: {
        schema: "public",
        name: "r3_calc",
        kind: "function",
        identity: "integer",
      },
      ownerTable: null,
      comment: "Overloaded integer calculator",
      ddl: {
        status: "available",
        ddl: "CREATE FUNCTION public.r3_calc(integer) RETURNS integer ...",
        truncated: false,
      },
      indexes: [],
      constraints: [],
      triggers: [],
      routine: {
        identityArguments: "integer",
        arguments: "amount integer",
        returnType: "integer",
        language: "sql",
        volatility: "IMMUTABLE",
        securityDefiner: false,
      },
      sequence: null,
      uses: [],
      usedBy: [],
    } satisfies ObjectDetails);

    const tab: StudioTab = {
      id: "tab-obj-1",
      title: "r3_calc(integer)",
      kind: "object",
      connectionId: "conn-1",
      sql: "",
      table: null,
      object: {
        schema: "public",
        name: "r3_calc",
        kind: "function",
        identity: "integer",
      },
      schema: null,
      view: "structure",
      results: [],
      activeResultId: null,
      pages: [],
      pageKeys: {},
      query: { filters: [], sort: [], whereClause: null, orderClause: null },
      loadedRows: 0,
      nextKey: null,
      hasMore: false,
      loadingPage: false,
      dataSessionId: null,
      querySession: null,
      querySessionClosed: false,
      fileToken: null,
      fileName: null,
      savedSql: null,
      savedQueryId: null,
      runId: null,
      status: "idle",
      error: null,
      metrics: {},
      elapsedMs: 0,
      revision: 0,
    };

    const { rerender } = render(<ObjectInspectorView tab={tab} session={makeSession(undefined)} />);
    expect(screen.getByText("Extended object details unavailable")).toBeTruthy();
    expect(callSpy).not.toHaveBeenCalled();

    rerender(<ObjectInspectorView tab={tab} session={makeSession(true)} />);
    await waitFor(() => {
      expect(screen.getByText("Overloaded integer calculator")).toBeTruthy();
    });
    expect(callSpy).toHaveBeenCalledWith("object_details", {
      sessionId: "sess-1",
      object: tab.object,
    });
    expect(screen.getByText("amount integer")).toBeTruthy();
    expect(screen.getByText("IMMUTABLE")).toBeTruthy();
  });
});
