import { createRef } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { EditorState } from "@codemirror/state";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectionProfile, ResultInfo, SessionInfo } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { resultCache } from "../../data/result-cache";
import { useStudio, type StudioTab } from "../../state/studio";
import { Workbench } from "../../app/Workbench";
import type { DatabaseGridHandle } from "../grid/DatabaseGrid";
import type { EditorHandle } from "./SqlEditor";
import { VirtualForeignKeyDialog } from "./VirtualForeignKeyDialog";
import { StructureView } from "./UtilityViews";
import { QueryPlanView } from "./QueryPlanView";
import { SqlColumnInspectionDialog } from "./SqlColumnInspectionDialog";
import { analyzeExtractCte, analyzeInlineCte } from "./sql-refactor";
import { prepareColumnInspection } from "./sql-inspections";
import { sqlDialect } from "./sql-identifiers";
import { ConnectionDialog } from "../connections/ConnectionDialog";
import { App } from "../../app/App";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  desktopAvailable: false,
  messageOf: (e: unknown) => String(e instanceof Error ? e.message : e),
}));

const pgProfile: ConnectionProfile = {
  id: "aaaaaaaa-1111-4111-8111-111111111111",
  name: "DG 2026.3 Studio",
  groupId: "local",
  engine: "postgres",
  host: "127.0.0.1",
  port: 55432,
  database: "joinery",
  username: "joinery",
  environment: "local",
  safeMode: "silent",
  tlsMode: "disable",
  caCertPath: null,
  ssh: null,
  snowflake: null,
  rememberPassword: false,
  storeQueryText: true,
  storeDrafts: true,
};

const pgSession: SessionInfo = {
  id: "sess-pg-1",
  connectionId: pgProfile.id,
  engine: "postgres",
  serverVersion: "PostgreSQL 18.0",
  latencyMs: 1,
  encrypted: false,
  consoleMode: false,
  capabilities: {
    sql: true,
    transactions: true,
    explicitTransactionControls: false,
    cancellation: true,
    tableBrowsing: true,
    relationBrowsing: false,
    tableEditing: true,
    tableFiltering: true,
    processes: true,
    objectExplorer: true,
    schemaDiagram: true,
    schemaDiff: true,
    ddlEditing: true,
    snowflakeReviewedDdl: false,
    snowflakeGridEditing: false,
    lockInspection: true,
    explainAnalyze: true,
    oauthBrowser: false,
  },
};

describe("DataGrip 2026.3 E2E ARIA & selector contracts", () => {
  beforeEach(() => {
    vi.mocked(client.call).mockReset();
    useStudio.setState({
      bootstrap: {
        profiles: [pgProfile],
        groups: [{ id: "local", name: "Local", environment: "local", parentId: null }],
        passphraseConfigured: false,
        supportedEngines: ["postgres", "mysql", "sqlite", "redis", "snowflake"],
        resultBudgetBytes: 64 * 1024 * 1024,
        rowLimit: 50_000,
        temporaryResultStorage: false,
        workspace: null,
        savedQueries: [],
        bookmarks: [],
        sqlFormatSettings: { keywordCase: "upper", indentWidth: 2, linesBetweenQueries: 1 },
        sqlSnippets: [],
        virtualForeignKeys: [],
      },
      sessions: { [pgProfile.id]: pgSession },
      selectedConnectionId: pgProfile.id,
      virtualForeignKeys: [],
      tabs: [],
      activeTabId: null,
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("matches VirtualForeignKeyDialog and StructureView ARIA labels used by Playwright", () => {
    render(
      <VirtualForeignKeyDialog
        connectionId={pgProfile.id}
        initialSource={{ schema: "public", name: "browse_sort" }}
        sourceColumns={["id", "bucket", "label"]}
        knownTables={[
          { schema: "public", name: "browse_sort" },
          { schema: "public", name: "customers" },
        ]}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Relationship name")).toBeTruthy();
    expect(screen.getByLabelText("Referenced table")).toBeTruthy();
    expect(screen.getByLabelText("Source column 1")).toBeTruthy();
    expect(screen.getByLabelText("Referenced column 1")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Save virtual FK" })).toBeTruthy();
    cleanup();

    useStudio.setState({
      virtualForeignKeys: [
        {
          id: "vfk-1",
          connectionId: pgProfile.id,
          name: "vfk_browse_sort_customers",
          source: { schema: "public", name: "browse_sort" },
          columns: ["id"],
          target: { schema: "public", name: "customers" },
          targetColumns: ["id"],
          updatedAtMs: 1,
        },
      ],
    });

    render(
      <StructureView
        connectionId={pgProfile.id}
        schema={{
          table: { schema: "public", name: "browse_sort" },
          columns: [
            {
              id: "id",
              name: "id",
              databaseType: "bigint",
              kind: "integer",
              nullable: false,
              primaryKey: true,
              foreignKey: true,
              editable: true,
              insertable: true,
              hasDefault: false,
            },
          ],
          primaryKey: ["id"],
          foreignKeys: [
            {
              name: "vfk_browse_sort_customers",
              columns: ["id"],
              target: { schema: "public", name: "customers" },
              targetColumns: ["id"],
              virtualRelationship: true,
            },
          ],
          inboundForeignKeys: [],
          editable: true,
        }}
      />,
    );

    const fkTable = screen.getByRole("table", { name: "Foreign keys" });
    expect(fkTable.textContent).toContain("vfk_browse_sort_customers");
    expect(
      screen.getByRole("button", { name: "Delete virtual foreign key vfk_browse_sort_customers" }),
    ).toBeTruthy();
  });

  it("exposes Open result in tab and Open originating SQL buttons in Workbench", async () => {
    vi.mocked(client.call).mockImplementation(async (method) => {
      if (method === "query_result_view") {
        return {
          resultId: "res-1",
          rowIndices: [1],
          matchedRows: 1,
          scannedRows: 2,
          truncated: false,
          elapsedMs: 1,
        };
      }
      return undefined;
    });
    const sampleResult: ResultInfo = {
      id: "res-1",
      runId: "run-1",
      statementIndex: 0,
      columns: [
        {
          id: "c0",
          name: "account_id",
          databaseType: "int4",
          kind: "integer",
          nullable: false,
          primaryKey: false,
          foreignKey: false,
          editable: false,
          insertable: false,
          hasDefault: false,
        },
      ],
      retainedRows: 2,
      status: "complete",
      table: null,
    };
    const queryTab: StudioTab = {
      id: "tab-q1",
      kind: "query",
      title: "Query 1",
      connectionId: pgProfile.id,
      sql: "SELECT 101 AS account_id;",
      table: null,
      view: "data",
      results: [sampleResult],
      activeResultId: sampleResult.id,
      pages: [],
      pageKeys: {},
      loadedRows: 0,
      hasMore: false,
      loadingPage: false,
      schema: null,
      nextKey: null,
      query: { sort: [], filters: [], whereClause: null, orderClause: null },
      status: "completed",
      runId: null,
      revision: 1,
      dataSessionId: pgSession.id,
      error: null,
      elapsedMs: 4,
      metrics: {},
      fileToken: null,
      fileName: null,
      savedSql: null,
      savedQueryId: null,
      object: null,
      querySession: null,
      querySessionClosed: false,
    };
    useStudio.setState({ tabs: [queryTab], activeTabId: queryTab.id });

    const editorRef = createRef<EditorHandle>();
    const gridRef = createRef<DatabaseGridHandle>();

    const { rerender, container } = render(
      <Workbench
        tab={queryTab}
        profile={pgProfile}
        session={pgSession}
        editorRef={editorRef}
        gridRef={gridRef}
        onRun={vi.fn()}
        onExplain={vi.fn()}
        onAnalyze={vi.fn()}
        onOpenParameters={vi.fn()}
        onReview={vi.fn()}
        onTransactionControl={vi.fn().mockResolvedValue(undefined)}
        onConfirm={vi.fn()}
        onSelect={vi.fn()}
        onOpenValue={vi.fn()}
        onOpenColumns={vi.fn()}
        onOpenExport={vi.fn()}
        onOpenImport={vi.fn()}
        onOpenCompare={vi.fn()}
        onNewSession={vi.fn()}
        onOpenSqlFile={vi.fn()}
        onSaveSqlFile={vi.fn()}
        onSaveToLibrary={vi.fn()}
        onOpenLocalHistory={vi.fn()}
        onToggleBookmark={vi.fn()}
        onQueryChange={vi.fn()}
      />,
    );

    expect(container.querySelector(".run-button")).toBeTruthy();
    const filterInput = screen.getByLabelText("Filter result rows");
    fireEvent.change(filterInput, { target: { value: "grace" } });
    await waitFor(() => {
      expect(container.querySelector(".result-quick-filter-count")?.textContent).toBe("1 / 2");
    });
    fireEvent.click(screen.getByRole("button", { name: "Reset view" }));
    expect(container.querySelector(".result-quick-filter-count")).toBeNull();

    const pinBtn = screen.getByRole("button", { name: "Pin result" });
    fireEvent.click(pinBtn);
    const updatedTab = useStudio.getState().tabs.find((t) => t.id === queryTab.id)!;
    rerender(
      <Workbench
        tab={updatedTab}
        profile={pgProfile}
        session={pgSession}
        editorRef={editorRef}
        gridRef={gridRef}
        onRun={vi.fn()}
        onExplain={vi.fn()}
        onAnalyze={vi.fn()}
        onOpenParameters={vi.fn()}
        onReview={vi.fn()}
        onTransactionControl={vi.fn().mockResolvedValue(undefined)}
        onConfirm={vi.fn()}
        onSelect={vi.fn()}
        onOpenValue={vi.fn()}
        onOpenColumns={vi.fn()}
        onOpenExport={vi.fn()}
        onOpenImport={vi.fn()}
        onOpenCompare={vi.fn()}
        onNewSession={vi.fn()}
        onOpenSqlFile={vi.fn()}
        onSaveSqlFile={vi.fn()}
        onSaveToLibrary={vi.fn()}
        onOpenLocalHistory={vi.fn()}
        onToggleBookmark={vi.fn()}
        onQueryChange={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "Unpin result" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: /Pinned #1/ })).toBeTruthy();

    const detachBtn = screen.getByRole("button", { name: "Open result in tab" });
    fireEvent.click(detachBtn);

    const detachedTab = useStudio.getState().tabs.find((t) => t.kind === "result");
    expect(detachedTab).toBeTruthy();

    rerender(
      <Workbench
        tab={detachedTab!}
        profile={pgProfile}
        session={pgSession}
        editorRef={editorRef}
        gridRef={gridRef}
        onRun={vi.fn()}
        onExplain={vi.fn()}
        onAnalyze={vi.fn()}
        onOpenParameters={vi.fn()}
        onReview={vi.fn()}
        onTransactionControl={vi.fn().mockResolvedValue(undefined)}
        onConfirm={vi.fn()}
        onSelect={vi.fn()}
        onOpenValue={vi.fn()}
        onOpenColumns={vi.fn()}
        onOpenExport={vi.fn()}
        onOpenImport={vi.fn()}
        onOpenCompare={vi.fn()}
        onNewSession={vi.fn()}
        onOpenSqlFile={vi.fn()}
        onSaveSqlFile={vi.fn()}
        onSaveToLibrary={vi.fn()}
        onOpenLocalHistory={vi.fn()}
        onToggleBookmark={vi.fn()}
        onQueryChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Open originating SQL" })).toBeTruthy();
  });

  it("renders SQLite EXPLAIN QUERY PLAN inside Query execution plan region and extracts CTE when caret is inside subquery", async () => {
    const planJson = JSON.stringify({
      sqliteQueryPlan: [{ id: 2, parent: 0, detail: "SCAN CONSTANT ROW" }],
    });
    const ensureSpy = vi.spyOn(resultCache, "ensure").mockResolvedValue(undefined);
    const readSpy = vi
      .spyOn(resultCache, "read")
      .mockReturnValue({ value: planJson, large: false });

    const planResult: ResultInfo = {
      id: "sqlite-plan-1",
      runId: "run-plan-1",
      statementIndex: 0,
      columns: [
        {
          id: "query_plan",
          name: "QUERY PLAN",
          databaseType: "JSON",
          kind: "json",
          nullable: false,
          primaryKey: false,
          foreignKey: false,
          editable: false,
          insertable: false,
          hasDefault: false,
        },
      ],
      retainedRows: 1,
      status: "complete",
      table: null,
    };

    render(<QueryPlanView result={planResult} ownerTabId="tab-1" canCapture={false} />);

    await waitFor(() => {
      const region = screen.getByRole("region", { name: "Query execution plan" });
      expect(region.textContent).toContain("SQLite EXPLAIN QUERY PLAN");
      expect(region.textContent).toContain("SCAN CONSTANT ROW");
    });

    ensureSpy.mockRestore();
    readSpy.mockRestore();

    const doc = "SELECT sub.v FROM (SELECT sqlite_version() AS v) AS sub;";
    const endState = EditorState.create({
      doc,
      selection: { anchor: doc.length },
      extensions: [sqlDialect("sqlite")],
    });
    expect(() => analyzeExtractCte(endState, "sqlite")).toThrow(/parenthesized/);

    const insidePos = doc.indexOf("sqlite_version");
    const insideState = EditorState.create({
      doc,
      selection: { anchor: insidePos },
      extensions: [sqlDialect("sqlite")],
    });
    const plan = analyzeExtractCte(insideState, "sqlite");
    expect(plan.replacementKind).toBe("derivedTable");
    const preview = plan.preview(insideState, "ver_cte");
    expect(preview.previewSql).toContain("WITH ver_cte AS");

    const inlineDoc =
      "WITH ver_cte AS (\n  SELECT sqlite_version() AS v\n)\nSELECT sub.v FROM ver_cte AS sub;";
    const inlineState = EditorState.create({
      doc: inlineDoc,
      selection: { anchor: inlineDoc.indexOf("ver_cte") },
      extensions: [sqlDialect("sqlite")],
    });
    const inlinePlan = analyzeInlineCte(inlineState, "sqlite");
    expect(inlinePlan.cteName).toBe("ver_cte");
    expect(inlinePlan.previewSql).not.toContain("WITH ver_cte AS");
  });

  it("exposes exact Database engine label on ConnectionDialog, restores Connect a database after draft profile deletion, and applies SQLite column quick-fix", async () => {
    render(<ConnectionDialog onClose={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Connection name", { exact: true }), {
      target: { value: "SQLite Memory Scratch" },
    });
    fireEvent.change(screen.getByLabelText("Database engine", { exact: true }), {
      target: { value: "sqlite" },
    });
    const sqlitePath = screen.getByLabelText("Database file path (or :memory:)", {
      exact: true,
    }) as HTMLInputElement;
    expect(sqlitePath.value).toBe(":memory:");
    fireEvent.change(screen.getByLabelText("Safe Mode", { exact: true }), {
      target: { value: "silent" },
    });
    cleanup();

    // Leaked storeDrafts profile hides Welcome "Connect a database"; deleting it restores Welcome.
    vi.mocked(client.call).mockResolvedValue(null);
    const baseBootstrap = useStudio.getState().bootstrap!;
    useStudio.setState({
      initialize: vi.fn().mockResolvedValue(undefined),
      tabs: [
        {
          id: "leaked-draft-1",
          kind: "query",
          title: "Query 1",
          connectionId: pgProfile.id,
          sql: "SELECT 101;",
          table: null,
          view: "data",
          results: [],
          activeResultId: null,
          pages: [],
          pageKeys: {},
          loadedRows: 0,
          hasMore: false,
          loadingPage: false,
          schema: null,
          nextKey: null,
          query: { sort: [], filters: [], whereClause: null, orderClause: null },
          status: "idle",
          runId: null,
          revision: 0,
          dataSessionId: null,
          error: null,
          elapsedMs: 0,
          metrics: {},
          fileToken: null,
          fileName: null,
          savedSql: null,
          savedQueryId: null,
          object: null,
          querySession: null,
          querySessionClosed: false,
        },
      ],
      activeTabId: "leaked-draft-1",
    });
    render(<App />);
    expect(screen.queryByRole("button", { name: "Connect a database" })).toBeNull();
    useStudio.getState().removeConnection(pgProfile.id);
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Connect a database" })).toBeTruthy();
    });
    cleanup();

    useStudio.setState({ bootstrap: baseBootstrap });

    // SQLite column typo quick-fix contract matching Test 2 lines 190-196.
    const sql = "SELECT u.custmoer_email FROM demo_users u;";
    const editorState = EditorState.create({
      doc: sql,
      selection: { anchor: sql.indexOf("custmoer_email") },
      extensions: [sqlDialect("sqlite")],
    });
    const inspectionPlan = prepareColumnInspection(editorState, editorState.selection.main.head, {
      engine: "sqlite",
      tables: [
        {
          table: { schema: "main", name: "demo_users" },
          kind: "table",
          estimatedRows: 0,
          totalBytes: null,
        },
      ],
      databaseObjects: [],
      schemas: new Map(),
      database: ":memory:",
      searchPath: ["main"],
      unqualifiedTables: [{ schema: "main", name: "demo_users" }],
    });
    const result = inspectionPlan.finish(editorState, [
      {
        table: { schema: "main", name: "demo_users" },
        columns: [
          {
            id: "id",
            name: "id",
            databaseType: "INTEGER",
            kind: "integer",
            nullable: false,
            primaryKey: true,
            foreignKey: false,
            editable: true,
            insertable: true,
            hasDefault: false,
          },
          {
            id: "customer_email",
            name: "customer_email",
            databaseType: "TEXT",
            kind: "text",
            nullable: true,
            primaryKey: false,
            foreignKey: false,
            editable: true,
            insertable: true,
            hasDefault: false,
          },
        ],
        primaryKey: ["id"],
        foreignKeys: [],
        inboundForeignKeys: [],
        editable: true,
      },
    ]);
    const onApply = vi.fn();
    render(
      <SqlColumnInspectionDialog
        result={result}
        state={editorState}
        onApply={onApply}
        onClose={vi.fn()}
      />,
    );
    const group = screen.getByRole("group", { name: "Column typo quick-fixes" });
    fireEvent.click(screen.getByRole("button", { name: "customer_email" }));
    expect(group).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Apply column correction" }));
    expect(onApply).toHaveBeenCalledWith(
      expect.objectContaining({ name: "custmoer_email" }),
      "customer_email",
    );
    expect(result.preview(editorState, result.issues[0], "customer_email").insert).toBe(
      '"customer_email"',
    );
  });
});
