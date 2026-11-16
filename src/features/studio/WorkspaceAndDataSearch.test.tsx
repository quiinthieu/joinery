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
          sql: "SELECT *\nFROM pg_users\nLIMIT 100;",
        },
      });
    });

    const updatedWsTab = useStudio.getState().tabs.find((t) => t.id === wsTab.id)!;
    rerender(<WorkspaceSqlView tab={updatedWsTab} />);
    expect(screen.getByTestId("mock-workspace-grid").textContent).toContain("Grid rows: 2");

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
});
