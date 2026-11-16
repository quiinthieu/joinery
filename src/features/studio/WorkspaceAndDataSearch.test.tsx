import { forwardRef } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ColumnInfo,
  ConnectionProfile,
  DataSearchReport,
  ResultInfo,
  SessionInfo,
  WorkspaceSqlResponse,
} from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import { WorkspaceSqlView, discoverMountableDatasets } from "./WorkspaceSqlView";
import { DataSearchView } from "./DataSearchView";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn(), range: vi.fn() },
  messageOf: (e: unknown) => String(e instanceof Error ? e.message : e),
}));

vi.mock("../grid/DatabaseGrid", () => ({
  default: forwardRef(({ result }: { result?: ResultInfo }, _ref) => (
    <div data-testid="mock-workspace-grid">Grid rows: {result?.retainedRows ?? 0}</div>
  )),
}));

const col = (name: string): ColumnInfo => ({
  id: name,
  name,
  databaseType: "text",
  kind: "text",
  nullable: true,
  primaryKey: name === "id",
  foreignKey: false,
  editable: false,
  insertable: false,
  hasDefault: false,
});

const pgProfile: ConnectionProfile = {
  id: "11111111-1111-4111-8111-111111111111",
  name: "PG Core",
  groupId: "local",
  engine: "postgres",
  host: "127.0.0.1",
  port: 5432,
  database: "app",
  username: "postgres",
  environment: "local",
  safeMode: "confirm",
  tlsMode: "disable",
  caCertPath: null,
  ssh: null,
  rememberPassword: false,
  storeQueryText: true,
  storeDrafts: true,
  snowflake: null,
};

const pgSession: SessionInfo = {
  id: "sess-pg",
  connectionId: pgProfile.id,
  engine: "postgres",
  serverVersion: "17.2",
  latencyMs: 1,
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

describe("WorkspaceSqlView and DataSearchView", () => {
  beforeEach(() => {
    vi.mocked(client.call).mockReset();
    useStudio.setState({
      bootstrap: {
        profiles: [pgProfile],
        groups: [],
        passphraseConfigured: false,
        supportedEngines: ["postgres", "mysql", "snowflake", "redis"],
        resultBudgetBytes: 64 * 1024 * 1024,
        rowLimit: 50_000,
        workspace: null,
        savedQueries: [],
        bookmarks: [],
        sqlFormatSettings: { keywordCase: "upper", indentWidth: 2, linesBetweenQueries: 1 },
        sqlSnippets: [],
        virtualForeignKeys: [],
      },
      sessions: { [pgProfile.id]: pgSession },
      selectedConnectionId: pgProfile.id,
      tabs: [],
      activeTabId: null,
      catalog: {
        [pgProfile.id]: [
          {
            table: { schema: "public", name: "customers" },
            kind: "r",
            estimatedRows: 10,
            totalBytes: 512,
          },
        ],
      },
    });
  });

  afterEach(() => cleanup());

  it("discovers mountable datasets, sanitizes sqlite_ prefixes, and excludes console-owned results", () => {
    useStudio.setState({
      snowflakeConsoleResultOwners: { "res-console": "snowflake-console:req-1" },
    });
    useStudio.getState().openQuery(pgProfile.id, "SELECT 1");
    const qTab = useStudio.getState().tabs[0]!;
    useStudio.getState().updateTab(qTab.id, {
      title: "sqlite_internal",
      results: [
        {
          id: "res-ordinary",
          runId: "run-1",
          statementIndex: 0,
          columns: [col("id"), col("email")],
          retainedRows: 3,
          status: "complete",
          table: null,
          editability: undefined,
        },
        {
          id: "res-console",
          runId: "snowflake-console:req-1",
          statementIndex: 1,
          columns: [col("secret")],
          retainedRows: 1,
          status: "complete",
          table: null,
          editability: undefined,
        },
      ],
    });

    const discovered = discoverMountableDatasets(
      useStudio.getState().tabs,
      new Map([[pgProfile.id, pgProfile.name]]),
    );
    expect(discovered).toHaveLength(1);
    expect(discovered[0]?.resultIds).toEqual(["res-ordinary"]);
    expect(discovered[0]?.defaultAlias).toBe("ds_sqlite_internal");
  });

  it("executes cross-source Workspace SQL and releases superseded result snapshots", async () => {
    useStudio.getState().openQuery(pgProfile.id, "SELECT * FROM users");
    const qTab = useStudio.getState().tabs[0]!;
    useStudio.getState().updateTab(qTab.id, {
      title: "pg_users",
      results: [
        {
          id: "res-users",
          runId: "run-1",
          statementIndex: 0,
          columns: [col("id"), col("email")],
          retainedRows: 2,
          status: "complete",
          table: null,
          editability: undefined,
        },
      ],
    });
    useStudio.getState().openUtility("workspaceSql");
    const wsTab = useStudio.getState().tabs.find((t) => t.kind === "workspaceSql")!;

    const firstResponse: WorkspaceSqlResponse = {
      result: {
        id: "wsql-res-1",
        runId: "workspace-sql-1",
        statementIndex: 0,
        columns: [col("id"), col("email")],
        retainedRows: 2,
        status: "complete",
        table: null,
        editability: { status: "readOnly", reason: "Cross-source workspace query result" },
      },
      elapsedMs: 4,
      mountedRows: 2,
      truncated: false,
    };
    const secondResponse: WorkspaceSqlResponse = {
      ...firstResponse,
      result: { ...firstResponse.result, id: "wsql-res-2", retainedRows: 1 },
    };

    vi.mocked(client.call)
      .mockResolvedValueOnce(firstResponse)
      .mockResolvedValueOnce(secondResponse)
      .mockResolvedValueOnce(undefined);

    const { rerender } = render(<WorkspaceSqlView tab={wsTab} />);
    fireEvent.click(screen.getByRole("button", { name: /Run workspace SQL/i }));

    await waitFor(() => {
      expect(client.call).toHaveBeenCalledWith("run_workspace_sql", {
        request: {
          mounts: [
            {
              alias: "pg_users",
              resultIds: ["res-users"],
              label: "PG Core: pg_users",
            },
          ],
          sql: 'SELECT *\nFROM "pg_users"\nLIMIT 100;',
        },
      });
    });

    const updatedWsTab = useStudio.getState().tabs.find((t) => t.id === wsTab.id)!;
    rerender(<WorkspaceSqlView tab={updatedWsTab} />);
    expect((await screen.findByTestId("mock-workspace-grid")).textContent).toContain(
      "Grid rows: 2",
    );

    fireEvent.click(screen.getByRole("button", { name: /Run workspace SQL/i }));
    await waitFor(() => {
      expect(client.call).toHaveBeenCalledWith("release_result", { resultId: "wsql-res-1" });
    });
  });

  it("runs bounded full-text data search and opens filtered table browser on hit", async () => {
    useStudio.getState().openUtility("dataSearch");
    const dsTab = useStudio.getState().tabs.find((t) => t.kind === "dataSearch")!;

    const report: DataSearchReport = {
      query: "ada@",
      matchMode: "contains",
      caseSensitive: false,
      scannedTables: 3,
      skippedTables: 0,
      totalRowHits: 1,
      truncated: false,
      elapsedMs: 9,
      matches: [
        {
          table: { schema: "public", name: "customers" },
          columns: [col("id"), col("email")],
          searchableColumns: ["id", "email"],
          rows: [{ matchedColumns: ["email"], row: ["1", "ada@example.com"] }],
          hasMoreInTable: false,
        },
      ],
    };
    vi.mocked(client.call).mockResolvedValueOnce(report);
    const openTableSpy = vi.spyOn(useStudio.getState(), "openTable").mockResolvedValue(undefined);

    render(<DataSearchView tab={dsTab} />);
    fireEvent.change(screen.getByLabelText("Search database values"), {
      target: { value: "ada@" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Search tables/i }));

    await waitFor(() => {
      expect(client.call).toHaveBeenCalledWith("search_database_data", {
        request: {
          sessionId: "sess-pg",
          query: "ada@",
          schemas: [],
          tables: [],
          matchMode: "contains",
          caseSensitive: false,
          maxTables: 25,
        },
      });
    });

    expect(screen.getByText("ada@example.com")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Open table/i }));
    expect(openTableSpy).toHaveBeenCalledWith(
      pgProfile.id,
      { schema: "public", name: "customers" },
      {
        sort: [],
        filters: [{ column: "email", op: "contains", value: "ada@" }],
        whereClause: null,
        orderClause: null,
      },
    );
  });

  it("allows clearing Workspace SQL textarea, routes detached result back to workspaceSql, and releases orphaned results", async () => {
    useStudio.getState().openUtility("workspaceSql");
    const wsTab = useStudio.getState().tabs.find((t) => t.kind === "workspaceSql")!;

    const { rerender } = render(<WorkspaceSqlView tab={wsTab} />);
    const textarea = screen.getByLabelText("Workspace SQL query") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "" } });
    rerender(<WorkspaceSqlView tab={useStudio.getState().tabs.find((t) => t.id === wsTab.id)!} />);
    expect((screen.getByLabelText("Workspace SQL query") as HTMLTextAreaElement).value).toBe("");

    const resp: WorkspaceSqlResponse = {
      result: {
        id: "wsql-detached-1",
        runId: "workspace-sql-1",
        statementIndex: 0,
        columns: [col("id")],
        retainedRows: 1,
        status: "truncated",
        table: null,
        editability: { status: "readOnly", reason: "Cross-source workspace query result" },
      },
      elapsedMs: 2,
      mountedRows: 1,
      truncated: true,
    };
    useStudio.getState().setWorkspaceSqlResult(wsTab.id, "SELECT 1 AS id;", resp);
    rerender(<WorkspaceSqlView tab={useStudio.getState().tabs.find((t) => t.id === wsTab.id)!} />);
    expect(screen.getByText("Truncated")).toBeTruthy();

    const detachedId = useStudio.getState().detachResultToTab(wsTab.id, "wsql-detached-1");
    expect(detachedId).toBeTruthy();
    useStudio.getState().updateTab(wsTab.id, { sql: "SELECT 999;" });
    const reopenedId = useStudio.getState().openOriginatingSql(detachedId!);
    expect(reopenedId).toBe(wsTab.id);
    expect(useStudio.getState().tabs.find((t) => t.id === wsTab.id)?.sql).toBe("SELECT 1 AS id;");

    // If the Workspace SQL tab was closed before an in-flight query returned, release_result is called immediately.
    useStudio.getState().closeTab(wsTab.id);
    vi.mocked(client.call).mockResolvedValue(undefined);
    useStudio.getState().setWorkspaceSqlResult(wsTab.id, "SELECT 1;", {
      ...resp,
      result: { ...resp.result, id: "wsql-orphan-99" },
    });
    expect(client.call).toHaveBeenCalledWith("release_result", { resultId: "wsql-orphan-99" });
  });

  it("excludes stateless Snowflake OAuth sessions and generates type-aware Snowflake search SQL", async () => {
    const sfOauth: ConnectionProfile = {
      ...pgProfile,
      id: "22222222-2222-4222-8222-222222222222",
      name: "SF OAuth",
      engine: "snowflake",
      database: "ANALYTICS",
      snowflake: {
        role: "ANALYST",
        warehouse: "WH",
        schema: "PUBLIC",
        authentication: "localApplicationOauth",
        reviewedDml: false,
        reviewedDdl: false,
        reviewedConsole: false,
      },
    };
    const sfSdk: ConnectionProfile = {
      ...sfOauth,
      id: "33333333-3333-4333-8333-333333333333",
      name: "SF SDK",
      snowflake: { ...sfOauth.snowflake!, authentication: "externalBrowser" },
    };
    useStudio.setState((s) => ({
      bootstrap: s.bootstrap ? { ...s.bootstrap, profiles: [sfOauth, sfSdk] } : null,
      selectedConnectionId: sfSdk.id,
      sessions: {
        [sfOauth.id]: {
          ...pgSession,
          id: "sess-sf-oauth",
          connectionId: sfOauth.id,
          engine: "snowflake",
          capabilities: {
            ...pgSession.capabilities,
            relationBrowsing: false,
            oauthBrowser: true,
          },
        },
        [sfSdk.id]: {
          ...pgSession,
          id: "sess-sf-sdk",
          connectionId: sfSdk.id,
          engine: "snowflake",
          capabilities: {
            ...pgSession.capabilities,
            relationBrowsing: true,
            oauthBrowser: false,
          },
        },
      },
    }));
    useStudio.getState().openUtility("dataSearch");
    const dsTab = useStudio.getState().tabs.find((t) => t.kind === "dataSearch")!;

    const report: DataSearchReport = {
      query: '{"tier":"VIP',
      matchMode: "startsWith",
      caseSensitive: true,
      scannedTables: 1,
      skippedTables: 0,
      totalRowHits: 1,
      truncated: false,
      elapsedMs: 12,
      matches: [
        {
          table: { schema: "PUBLIC", name: "EVENTS" },
          columns: [{ ...col("PAYLOAD"), databaseType: "OBJECT", kind: "json" }],
          searchableColumns: ["PAYLOAD"],
          rows: [{ matchedColumns: ["PAYLOAD"], row: ['{"tier":"VIP"}'] }],
          hasMoreInTable: false,
        },
      ],
    };
    vi.mocked(client.call).mockResolvedValueOnce(report);

    render(<DataSearchView tab={dsTab} />);
    const connSelect = screen.getByLabelText("Search connection") as HTMLSelectElement;
    expect(Array.from(connSelect.options).map((o) => o.text)).toEqual(["SF SDK (snowflake)"]);

    fireEvent.change(screen.getByLabelText("Search database values"), {
      target: { value: '{"tier":"VIP' },
    });
    fireEvent.click(screen.getByRole("button", { name: /Search tables/i }));
    await screen.findByText('{"tier":"VIP"}');

    fireEvent.click(screen.getByRole("button", { name: /Open SQL query/i }));
    const openedQueryTab = useStudio.getState().tabs.find((t) => t.kind === "query")!;
    expect(openedQueryTab.sql).toContain('FROM "ANALYTICS"."PUBLIC"."EVENTS"');
    expect(openedQueryTab.sql).toContain('STARTSWITH(TO_JSON("PAYLOAD"), \'{"tier":"VIP\')');
  });

  it("resolves Snowflake database from live session state when profile.database is empty and preserves multi-column PG/MySQL filters", async () => {
    const sfBlankDb: ConnectionProfile = {
      ...pgProfile,
      id: "44444444-4444-4444-8444-444444444444",
      name: "SF Blank Profile DB",
      engine: "snowflake",
      database: "",
      snowflake: {
        role: "ANALYST",
        warehouse: "WH",
        schema: "PUBLIC",
        authentication: "externalBrowser",
        reviewedDml: false,
        reviewedDdl: false,
        reviewedConsole: false,
      },
    };
    useStudio.setState((s) => ({
      bootstrap: s.bootstrap ? { ...s.bootstrap, profiles: [pgProfile, sfBlankDb] } : null,
      selectedConnectionId: sfBlankDb.id,
      sessions: {
        [pgProfile.id]: pgSession,
        [sfBlankDb.id]: {
          ...pgSession,
          id: "sess-sf-live",
          connectionId: sfBlankDb.id,
          engine: "snowflake",
          capabilities: {
            ...pgSession.capabilities,
            relationBrowsing: true,
          },
        },
      },
      sessionStates: {
        "sess-sf-live": {
          sessionId: "sess-sf-live",
          transactionMode: "autoCommit",
          transactionState: "idle",
          contextKind: "schema",
          activeContext: "PUBLIC",
          database: "LIVE_SNOWFLAKE_DB",
          role: "ANALYST",
          warehouse: "WH",
          searchPath: ["LIVE_SNOWFLAKE_DB.PUBLIC"],
        },
      },
    }));

    useStudio.getState().openUtility("dataSearch");
    const sfTab = useStudio.getState().tabs.find((t) => t.kind === "dataSearch")!;

    const sfReport: DataSearchReport = {
      query: "O'Reilly\\HQ",
      matchMode: "contains",
      caseSensitive: false,
      scannedTables: 1,
      skippedTables: 0,
      totalRowHits: 1,
      truncated: false,
      elapsedMs: 5,
      matches: [
        {
          table: { schema: "PUBLIC", name: "LOCATIONS" },
          columns: [col("NAME"), { ...col("COORDS"), databaseType: "GEOGRAPHY", kind: "unknown" }],
          searchableColumns: ["NAME", "COORDS"],
          rows: [
            {
              matchedColumns: ["NAME", "COORDS"],
              row: ["O'Reilly\\HQ", "SRID=4326;POINT(-122.4 37.7)"],
            },
          ],
          hasMoreInTable: false,
        },
      ],
    };
    vi.mocked(client.call).mockResolvedValueOnce(sfReport);

    const { unmount } = render(<DataSearchView tab={sfTab} />);
    fireEvent.change(screen.getByLabelText("Search database values"), {
      target: { value: "O'Reilly\\HQ" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Search tables/i }));
    await screen.findByText("SRID=4326;POINT(-122.4 37.7)");

    fireEvent.click(screen.getByRole("button", { name: /Open SQL query/i }));
    const sfQueryTab = useStudio.getState().tabs.find((t) => t.kind === "query")!;
    expect(sfQueryTab.sql).toContain('FROM "LIVE_SNOWFLAKE_DB"."PUBLIC"."LOCATIONS"');
    expect(sfQueryTab.sql).toContain(
      "CONTAINS(LOWER(TO_VARCHAR(\"NAME\")), LOWER('O''Reilly\\\\HQ'))",
    );
    expect(sfQueryTab.sql).toContain(
      "OR CONTAINS(LOWER(ST_ASEWKT(\"COORDS\")), LOWER('O''Reilly\\\\HQ'))",
    );
    unmount();

    // Multi-column PostgreSQL match must generate an OR whereClause rather than dropping filters
    useStudio.getState().openUtility("dataSearch");
    const pgTabId = useStudio
      .getState()
      .tabs.filter((t) => t.kind === "dataSearch")
      .at(-1)!.id;
    useStudio.getState().updateTab(pgTabId, { connectionId: pgProfile.id });
    const pgTab = useStudio.getState().tabs.find((t) => t.id === pgTabId)!;
    const pgMultiReport: DataSearchReport = {
      query: "100%_vip!",
      matchMode: "contains",
      caseSensitive: false,
      scannedTables: 1,
      skippedTables: 0,
      totalRowHits: 1,
      truncated: false,
      elapsedMs: 3,
      matches: [
        {
          table: { schema: "public", name: "customers" },
          columns: [col("email"), col("notes")],
          searchableColumns: ["email", "notes"],
          rows: [{ matchedColumns: ["email", "notes"], row: ["100%_vip!", "100%_vip!"] }],
          hasMoreInTable: false,
        },
      ],
    };
    vi.mocked(client.call).mockResolvedValueOnce(pgMultiReport);
    const openTableSpy = vi.spyOn(useStudio.getState(), "openTable").mockResolvedValue(undefined);

    render(<DataSearchView tab={pgTab} />);
    fireEvent.change(screen.getByLabelText("Search database values"), {
      target: { value: "100%_vip!" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Search tables/i }));
    await screen.findAllByText("100%_vip!");

    fireEvent.click(screen.getByRole("button", { name: /Open table/i }));
    expect(openTableSpy).toHaveBeenCalledWith(
      pgProfile.id,
      { schema: "public", name: "customers" },
      {
        sort: [],
        filters: [],
        whereClause:
          "(CAST(\"email\" AS TEXT) ILIKE '%100!%!_vip!!%' ESCAPE '!' OR CAST(\"notes\" AS TEXT) ILIKE '%100!%!_vip!!%' ESCAPE '!')",
        orderClause: null,
      },
    );
  });

  it("quotes SQLite reserved-keyword aliases in WorkspaceSqlView and reacts to console session changes", () => {
    useStudio.getState().openQuery(pgProfile.id, "SELECT * FROM orders");
    const q1 = useStudio.getState().tabs[0]!;
    useStudio.getState().updateTab(q1.id, {
      title: "order",
      results: [
        {
          id: "res-order",
          runId: "run-ord",
          statementIndex: 0,
          columns: [col("id")],
          retainedRows: 1,
          status: "complete",
          table: null,
          editability: undefined,
        },
      ],
    });
    useStudio.getState().openQuery(pgProfile.id, "SELECT * FROM groups");
    const q2 = useStudio.getState().tabs[1]!;
    useStudio.getState().updateTab(q2.id, {
      title: "group",
      results: [
        {
          id: "res-group",
          runId: "run-grp",
          statementIndex: 0,
          columns: [col("id")],
          retainedRows: 1,
          status: "complete",
          table: null,
          editability: undefined,
        },
      ],
    });
    useStudio.getState().openUtility("workspaceSql");
    const wsTab = useStudio.getState().tabs.find((t) => t.kind === "workspaceSql")!;

    const { rerender } = render(<WorkspaceSqlView tab={wsTab} />);
    const textarea = screen.getByLabelText("Workspace SQL query") as HTMLTextAreaElement;
    expect(textarea.value).toContain('FROM "order" a\nJOIN "group" b ON a."id" = b."id"');

    // Marking the session as a reviewed Snowflake console session without mutating `tabs` immediately hides its datasets
    useStudio.setState({ consoleSessions: { "sess-pg": true } });
    rerender(<WorkspaceSqlView tab={wsTab} />);
    expect((screen.getByLabelText("Workspace SQL query") as HTMLTextAreaElement).value).toBe(
      "SELECT 1 AS ready;",
    );
  });
});
