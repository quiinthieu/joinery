import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { SQLite, sql } from "@codemirror/lang-sql";
import type {
  ConnectionProfile,
  DataCompareReport,
  ObjectDetails,
  ResultInfo,
  SessionInfo,
} from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";
import { DataCompareDialog } from "./DataCompareDialog";
import { generateDataSyncSql } from "./data-sync-sql";
import { prepareColumnInspection, suggestColumnQuickFixes } from "../studio/sql-inspections";
import { SqlColumnInspectionDialog } from "../studio/SqlColumnInspectionDialog";
import { ObjectInspectorView } from "../studio/ObjectInspectorView";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn(), listen: vi.fn(async () => () => {}) },
  messageOf: (e: unknown) => String(e instanceof Error ? e.message : e),
}));

afterEach(cleanup);

function makeCompareReport(): DataCompareReport {
  return {
    leftLabel: "Staging",
    rightLabel: "Production",
    leftTruncated: true,
    rightTruncated: false,
    keyColumns: ["id"],
    alignedColumns: [
      {
        name: "id",
        leftName: "id",
        rightName: "order_id",
        leftType: "bigint",
        rightType: "bigint",
        kindMismatch: false,
        isKey: true,
      },
      {
        name: "status",
        leftName: "status",
        rightName: "order_status",
        leftType: "text",
        rightType: "varchar(64)",
        kindMismatch: false,
        isKey: false,
      },
      {
        name: "payload_hash",
        leftName: "payload_hash",
        rightName: "payload_hash",
        leftType: "bytea",
        rightType: "varbinary(32)",
        kindMismatch: false,
        isKey: false,
      },
    ],
    unalignedLeftColumns: [],
    unalignedRightColumns: ["audit_note"],
    leftTotalRows: 3,
    rightTotalRows: 3,
    identicalCount: 1,
    differentCount: 1,
    onlyLeftCount: 1,
    onlyRightCount: 1,
    leftAnomalies: {
      nullKeyCount: 1,
      nullKeySamples: [],
      duplicateKeyRows: 0,
      duplicateDistinctKeys: 0,
      excludedCounterpartRows: 0,
      duplicateKeySamples: [],
    },
    rightAnomalies: {
      nullKeyCount: 0,
      nullKeySamples: [],
      duplicateKeyRows: 0,
      duplicateDistinctKeys: 0,
      excludedCounterpartRows: 0,
      duplicateKeySamples: [],
    },
    diffs: [
      {
        kind: "onlyLeft",
        keyValues: ["9007199254740993"],
        cells: [],
        leftRow: ["9007199254740993", "paid", "0xdeadbeef"],
        rightRow: null,
      },
      {
        kind: "different",
        keyValues: ["42"],
        cells: [
          {
            column: "status",
            left: "shipped",
            right: "pending",
            typeMismatch: false,
          },
        ],
        leftRow: ["42", "shipped", "0xcafe"],
        rightRow: ["42", "pending", "0xcafe"],
      },
      {
        kind: "onlyRight",
        keyValues: ["99"],
        cells: [],
        leftRow: null,
        rightRow: ["99", "obsolete", null],
      },
    ],
    diffsTruncated: false,
  };
}

describe("Data Compare Sync SQL, SQLite column inspection & quick-fixes, and Snowflake codegen", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useStudio.setState({
      tabs: [],
      activeTabId: null,
      sessions: {},
      catalog: {},
      toast: null,
    });
  });

  it("generates bidirectional dialect-quoted sync SQL preserving 64-bit integers, binary literals, and safety warnings", () => {
    const report = makeCompareReport();

    // 1. Left -> Right into MySQL target (default: INSERT + UPDATE, no DELETE)
    const lr = generateDataSyncSql({
      report,
      direction: "leftToRight",
      engine: "mysql",
      targetTable: { schema: "shop", name: "orders" },
      includeInsert: true,
      includeUpdate: true,
      includeDelete: false,
    });
    expect(lr.ok).toBe(true);
    if (!lr.ok) return;
    expect(lr.insertCount).toBe(1);
    expect(lr.updateCount).toBe(1);
    expect(lr.deleteCount).toBe(0);
    expect(lr.warnings.length).toBeGreaterThanOrEqual(2);
    expect(lr.sql).toContain("-- WARNING:");
    expect(lr.sql).toContain(
      "INSERT INTO `shop`.`orders` (`order_id`, `order_status`, `payload_hash`) VALUES (9007199254740993, 'paid', X'deadbeef');",
    );
    // Only modified column (`order_status`) is in SET; unchanged `payload_hash` is preserved untouched!
    expect(lr.sql).toContain(
      "UPDATE `shop`.`orders` SET `order_status` = 'shipped' WHERE `order_id` = 42;",
    );
    expect(lr.sql).not.toContain("DELETE FROM");

    // 2. Right -> Left into PostgreSQL target with DELETE enabled
    const rl = generateDataSyncSql({
      report,
      direction: "rightToLeft",
      engine: "postgres",
      targetTable: { schema: "public", name: "staging_orders" },
      includeInsert: true,
      includeUpdate: true,
      includeDelete: true,
    });
    expect(rl.ok).toBe(true);
    if (!rl.ok) return;
    expect(rl.insertCount).toBe(1);
    expect(rl.updateCount).toBe(1);
    expect(rl.deleteCount).toBe(1);
    expect(rl.sql).toContain(
      'INSERT INTO "public"."staging_orders" ("id", "status", "payload_hash") VALUES (99, \'obsolete\', NULL);',
    );
    expect(rl.sql).toContain(
      'UPDATE "public"."staging_orders" SET "status" = \'pending\' WHERE "id" = 42;',
    );
    expect(rl.sql).toContain(
      'DELETE FROM "public"."staging_orders" WHERE "id" = 9007199254740993;',
    );
  });

  it("ranks column typo quick-fixes and supports SQLite column inspection with implicit rowid", () => {
    const candidates = ["id", "customer_id", "customer_email", "created_at", "total_cents"];
    expect(suggestColumnQuickFixes("custmoer_id", candidates)[0]).toBe("customer_id");
    expect(suggestColumnQuickFixes("customerid", candidates)[0]).toBe("customer_id");
    expect(suggestColumnQuickFixes("email", candidates)[0]).toBe("customer_email");
    // Unrelated short typo does not suggest random columns
    expect(suggestColumnQuickFixes("zzz_unknown_999", ["id", "name"])).toEqual([]);

    // SQLite column inspection: mixed-case table lookup + missing typo column caught
    const sqliteDoc = "SELECT o.custmoer_id, o.total_cents FROM MyOrders o;";
    const sqliteState = EditorState.create({
      doc: sqliteDoc,
      extensions: [sql({ dialect: SQLite })],
    });
    const plan = prepareColumnInspection(sqliteState, 10, {
      engine: "sqlite",
      tables: [
        {
          table: { schema: "main", name: "MyOrders" },
          kind: "table",
          estimatedRows: 5,
          totalBytes: null,
        },
      ],
      unqualifiedTables: [{ schema: "main", name: "MyOrders" }],
      caseSensitiveTableNames: false,
    });
    const inspection = plan.finish(sqliteState, [
      {
        table: { schema: "main", name: "MyOrders" },
        primaryKey: ["id"],
        foreignKeys: [],
        inboundForeignKeys: [],
        editable: true,
        columns: candidates.map((c) => ({
          id: c,
          name: c,
          databaseType: "TEXT",
          kind: "text",
          nullable: true,
          primaryKey: c === "id",
          foreignKey: false,
          editable: true,
          insertable: true,
          hasDefault: false,
        })),
      },
    ]);
    expect(inspection.issues).toHaveLength(1);
    expect(inspection.issues[0].name).toBe("custmoer_id");

    // Clicking the Did you mean quick-fix chip selects customer_id and applies
    const onApply = vi.fn();
    render(
      <SqlColumnInspectionDialog
        result={inspection}
        state={sqliteState}
        onApply={onApply}
        onClose={vi.fn()}
      />,
    );
    const chip = screen.getByRole("button", { name: "customer_id" });
    fireEvent.click(chip);
    fireEvent.click(screen.getByRole("button", { name: "Apply column correction" }));
    expect(onApply).toHaveBeenCalledWith(inspection.issues[0], "customer_id");
  });

  it("exposes SQL codegen buttons on Snowflake ObjectInspectorView (full CRUD on base tables, SELECT-only on dynamic tables)", async () => {
    const sfSession: SessionInfo = {
      id: "sf-sess-1",
      connectionId: "sf-conn-1",
      engine: "snowflake",
      serverVersion: "8.40.0",
      latencyMs: 25,
      encrypted: true,
      consoleMode: false,
      capabilities: {
        sql: true,
        transactions: false,
        explicitTransactionControls: true,
        cancellation: true,
        tableBrowsing: false,
        relationBrowsing: true,
        tableEditing: false,
        tableFiltering: false,
        processes: false,
        objectExplorer: true,
        schemaDiagram: true,
        schemaDiff: true,
        ddlEditing: false,
        snowflakeReviewedDdl: true,
        snowflakeGridEditing: true,
        lockInspection: false,
        explainAnalyze: false,
        oauthBrowser: true,
      },
    };

    const baseDetails: ObjectDetails = {
      object: { schema: "PUBLIC", name: "ORDERS", kind: "table", identity: null },
      ownerTable: null,
      comment: null,
      ddl: { status: "available", ddl: "CREATE TABLE ORDERS (ID NUMBER);", truncated: false },
      indexes: [],
      constraints: [],
      triggers: [],
      routine: null,
      sequence: null,
      snowflakeTable: {
        subtype: "base",
        columns: [
          {
            id: "ID",
            name: "ID",
            databaseType: "NUMBER(38,0)",
            kind: "integer",
            nullable: false,
            primaryKey: true,
            foreignKey: false,
            editable: true,
            insertable: true,
            hasDefault: false,
          },
          {
            id: "STATUS",
            name: "STATUS",
            databaseType: "VARCHAR",
            kind: "text",
            nullable: true,
            primaryKey: false,
            foreignKey: false,
            editable: true,
            insertable: true,
            hasDefault: false,
          },
        ],
      },
      uses: [],
      usedBy: [],
    };

    const baseTab: StudioTab = {
      id: "obj-tab-1",
      title: "ORDERS",
      kind: "object",
      connectionId: "sf-conn-1",
      sql: "",
      table: null,
      object: baseDetails.object,
      view: "structure",
      status: "idle",
      results: [],
      activeResultId: null,
      pages: [],
      pageKeys: {},
      loadedRows: 0,
      hasMore: false,
      schema: null,
      nextKey: null,
      loadingPage: false,
      query: { filters: [], sort: [], whereClause: null, orderClause: null },
      runId: null,
      dataSessionId: null,
      querySession: null,
      querySessionClosed: false,
      fileToken: null,
      fileName: null,
      savedSql: null,
      savedQueryId: null,
      elapsedMs: 0,
      metrics: {},
      error: null,
      revision: 1,
    };

    const { rerender } = render(
      <ObjectInspectorView tab={baseTab} session={sfSession} providedDetails={baseDetails} />,
    );
    expect(screen.getByRole("button", { name: "Generate SELECT SQL" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Generate UPDATE SQL" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Generate UPDATE SQL" }));
    await waitFor(() => {
      const opened = useStudio.getState().tabs.find((t) => t.kind === "query");
      expect(opened?.sql).toBe(
        'UPDATE "PUBLIC"."ORDERS"\nSET\n  "STATUS" = :STATUS\nWHERE "ID" = :ID;',
      );
    });

    // Dynamic table only offers SELECT codegen
    rerender(
      <ObjectInspectorView
        tab={baseTab}
        session={sfSession}
        providedDetails={{
          ...baseDetails,
          snowflakeTable: { ...baseDetails.snowflakeTable!, subtype: "dynamic" },
        }}
      />,
    );
    expect(screen.getByRole("button", { name: "Generate SELECT SQL" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Generate UPDATE SQL" })).toBeNull();
  });

  it("renders DataCompareDialog sync SQL controls, toggles DELETE extra, and opens reviewable sync SQL in editor", async () => {
    const compareCols = [
      {
        id: "id",
        name: "id",
        databaseType: "bigint",
        kind: "integer" as const,
        nullable: false,
        primaryKey: true,
        foreignKey: false,
        editable: true,
        insertable: true,
        hasDefault: false,
      },
      {
        id: "status",
        name: "status",
        databaseType: "text",
        kind: "text" as const,
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: true,
        insertable: true,
        hasDefault: false,
      },
      {
        id: "payload_hash",
        name: "payload_hash",
        databaseType: "bytea",
        kind: "binary" as const,
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: true,
        insertable: true,
        hasDefault: false,
      },
    ];
    const mkResult = (id: string, schema: string, name: string): ResultInfo => ({
      id,
      runId: `run-${id}`,
      statementIndex: 0,
      columns: compareCols,
      retainedRows: 1,
      status: "complete",
      table: { schema, name },
      editability: {
        status: "editable",
        table: { schema, name },
        primaryKey: ["id"],
      },
    });

    const profiles: ConnectionProfile[] = [
      {
        id: "pg-stage",
        name: "Staging PG",
        groupId: "default",
        engine: "postgres",
        host: "localhost",
        port: 5432,
        database: "stage_db",
        username: "app",
        environment: "local",
        safeMode: "confirm",
        tlsMode: "disable",
        caCertPath: null,
        ssh: null,
        rememberPassword: false,
        storeQueryText: true,
        storeDrafts: true,
        snowflake: null,
      },
      {
        id: "mysql-prod",
        name: "Prod MySQL",
        groupId: "default",
        engine: "mysql",
        host: "localhost",
        port: 3306,
        database: "shop",
        username: "app",
        environment: "staging",
        safeMode: "guard",
        tlsMode: "disable",
        caCertPath: null,
        ssh: null,
        rememberPassword: false,
        storeQueryText: true,
        storeDrafts: true,
        snowflake: null,
      },
    ];

    const queryTabLeft: StudioTab = {
      id: "q-left",
      title: "Staging Console",
      kind: "query",
      connectionId: "pg-stage",
      sql: "SELECT * FROM public.staging_orders;",
      table: null,
      object: null,
      view: "data",
      status: "completed",
      results: [mkResult("res-left", "public", "staging_orders")],
      activeResultId: "res-left",
      pinnedResultIds: ["res-left"],
      pinnedResultLabels: { "res-left": "Pinned #1 (Q1)" },
      pages: [],
      pageKeys: {},
      loadedRows: 0,
      hasMore: false,
      schema: null,
      nextKey: null,
      loadingPage: false,
      query: { filters: [], sort: [], whereClause: null, orderClause: null },
      runId: null,
      dataSessionId: null,
      querySession: null,
      querySessionClosed: false,
      fileToken: null,
      fileName: null,
      savedSql: null,
      savedQueryId: null,
      elapsedMs: 3,
      metrics: {},
      error: null,
      revision: 1,
    };

    const queryTabRight: StudioTab = {
      ...queryTabLeft,
      id: "q-right",
      title: "Prod Console",
      connectionId: "mysql-prod",
      results: [mkResult("res-right", "shop", "orders")],
      activeResultId: "res-right",
      pinnedResultIds: [],
      pinnedResultLabels: {},
    };

    useStudio.setState({
      tabs: [queryTabLeft, queryTabRight],
      activeTabId: "q-left",
    });

    vi.mocked(client.call).mockImplementation(async (method) => {
      if (method === "compare_data") return makeCompareReport();
      throw new Error(`Unexpected RPC: ${method}`);
    });

    const onClose = vi.fn();
    render(
      <DataCompareDialog
        initialTab={queryTabLeft}
        tabs={[queryTabLeft, queryTabRight]}
        profiles={profiles}
        sessions={{}}
        catalog={{}}
        onCatalogLoaded={vi.fn()}
        onClose={onClose}
      />,
    );

    // Pinned snapshot label is surfaced in the snapshot selector
    expect(screen.getAllByText(/Pinned #1 \(Q1\)/).length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: /Compare datasets/i }));

    const syncRegion = await screen.findByRole("region", { name: "Data synchronization SQL" });
    expect(within(syncRegion).getByText("2 stmts")).toBeTruthy();

    // Enabling DELETE extra includes the extra target row statement
    fireEvent.click(within(syncRegion).getByLabelText("DELETE extra"));
    expect(within(syncRegion).getByText("3 stmts")).toBeTruthy();

    // Open Sync SQL in editor opens a reviewable console bound to the target connection (mysql-prod) without executing
    fireEvent.click(within(syncRegion).getByRole("button", { name: "Open Sync SQL in editor" }));
    expect(onClose).toHaveBeenCalledTimes(1);

    const openedTabs = useStudio.getState().tabs;
    expect(openedTabs).toHaveLength(3);
    const syncTab = openedTabs[2];
    expect(syncTab.kind).toBe("query");
    expect(syncTab.connectionId).toBe("mysql-prod");
    expect(syncTab.status).toBe("idle");
    expect(syncTab.sql).toContain("INSERT INTO `shop`.`orders`");
    expect(syncTab.sql).toContain("DELETE FROM `shop`.`orders` WHERE `order_id` = 99;");
  });
});
