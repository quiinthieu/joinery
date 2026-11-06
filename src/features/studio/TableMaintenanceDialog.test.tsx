import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ConnectionProfile,
  ObjectDetails,
  PreparedPlan,
  SessionInfo,
  TableMaintenanceAction,
  TableMaintenancePreview,
  TableSchema,
} from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import { StructureView } from "./UtilityViews";

function makeProfile(overrides?: Partial<ConnectionProfile>): ConnectionProfile {
  return {
    id: "conn-1",
    name: "Local Postgres",
    groupId: "local",
    engine: "postgres",
    environment: "local",
    host: "127.0.0.1",
    port: 5432,
    database: "app_db",
    username: "postgres",
    safeMode: "confirm",
    tlsMode: "disable",
    caCertPath: null,
    ssh: null,
    rememberPassword: false,
    storeQueryText: true,
    storeDrafts: true,
    snowflake: null,
    ...overrides,
  };
}

function makeSession(engine: "postgres" | "mysql" = "postgres"): SessionInfo {
  return {
    id: "sess-1",
    connectionId: "conn-1",
    engine,
    serverVersion: engine === "postgres" ? "PostgreSQL 18.0" : "MySQL 8.4.0",
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
      objectExplorer: true,
    },
  };
}

const sampleSchema: TableSchema = {
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
    {
      id: "status",
      name: "status",
      databaseType: "text",
      kind: "text",
      nullable: false,
      primaryKey: false,
      foreignKey: false,
      editable: true,
      insertable: true,
      hasDefault: false,
    },
  ],
  primaryKey: ["id"],
  foreignKeys: [],
  editable: true,
};

const sampleDetails: ObjectDetails = {
  object: { schema: "public", name: "orders", kind: "table", identity: null },
  ownerTable: null,
  comment: null,
  ddl: { status: "available", ddl: 'CREATE TABLE "public"."orders" ();', truncated: false },
  constraints: [],
  indexes: [],
  triggers: [],
  routine: null,
  sequence: null,
  storage: {
    estimatedRows: 250,
    tableBytes: 16384,
    indexBytes: 8192,
    freeBytes: null,
    totalBytes: 24576,
    liveTuples: 250,
    deadTuples: 45,
    sequentialScans: 12,
    indexScans: 84,
    lastAnalyze: null,
    lastVacuum: null,
    statsStale: true,
  },
  uses: [],
  usedBy: [],
};

describe("TableMaintenanceDialog and StorageStatsSection", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    vi.restoreAllMocks();
    useStudio.setState({
      bootstrap: {
        profiles: [makeProfile()],
        groups: [],
        passphraseConfigured: false,
        supportedEngines: ["postgres", "mysql", "redis", "snowflake"],
        resultBudgetBytes: 64 * 1024 * 1024,
        rowLimit: 500,
        workspace: null,
        savedQueries: [],
        bookmarks: [],
        sqlFormatSettings: {
          keywordCase: "upper",
          indentWidth: 2,
          linesBetweenQueries: 1,
        },
        sqlSnippets: [],
        scratchpads: [],
      },
      tabs: [],
      activeTabId: null,
      sessions: { "conn-1": makeSession("postgres") },
      sessionStates: {},
      catalog: {},
      objects: {},
      selectedConnectionId: "conn-1",
    });
  });

  it("opens from StorageStatsSection, previews PostgreSQL ANALYZE and VACUUM, and executes via ReviewDialog", async () => {
    const refreshCatalogSpy = vi.fn().mockResolvedValue(undefined);
    const onRefreshSpy = vi.fn();
    useStudio.setState({ refreshCatalog: refreshCatalogSpy });

    const callSpy = vi.spyOn(client, "call").mockImplementation(async (command, args) => {
      if (command === "preview_table_maintenance") {
        const action = (args as { action: TableMaintenanceAction }).action;
        if (action.type === "analyze") {
          const cols =
            action.columns && action.columns.length > 0
              ? ` (${action.columns.map((c) => `"${c}"`).join(", ")})`
              : "";
          const opts = action.verbose ? " (VERBOSE)" : "";
          return {
            statement: {
              sql: `ANALYZE${opts} "${action.table.schema}"."${action.table.name}"${cols};`,
              description: `Analyze table ${action.table.schema}.${action.table.name}`,
              objectKind: "table",
              objectName: action.table.name,
              destructive: false,
            },
            warnings: [
              "ANALYZE samples table pages and updates pg_statistic planner estimates while acquiring only a SHARE UPDATE EXCLUSIVE lock.",
            ],
            nonTransactional: false,
          } satisfies TableMaintenancePreview;
        }
        if (action.type === "vacuum") {
          const parts: string[] = [];
          if (action.full) parts.push("FULL");
          if (action.analyze) parts.push("ANALYZE");
          if (action.verbose) parts.push("VERBOSE");
          const opts = parts.length ? ` (${parts.join(", ")})` : "";
          return {
            statement: {
              sql: `VACUUM${opts} "${action.table.schema}"."${action.table.name}";`,
              description: `Vacuum table ${action.table.schema}.${action.table.name}`,
              objectKind: "table",
              objectName: action.table.name,
              destructive: action.full,
            },
            warnings: [
              "VACUUM cannot run inside a transaction block.",
              ...(action.full
                ? ["VACUUM FULL rewrites the entire table and holds an ACCESS EXCLUSIVE lock."]
                : []),
            ],
            nonTransactional: true,
          } satisfies TableMaintenancePreview;
        }
      }
      if (command === "prepare_table_maintenance") {
        return {
          id: "plan-maint-1",
          sessionId: "sess-1",
          requiresApproval: true,
          requiresPassphrase: false,
          confirmationText: null,
          expiresAt: Date.now() + 60_000,
          warnings: ["VACUUM cannot run inside a transaction block."],
          statements: [
            {
              sql: 'VACUUM (FULL, ANALYZE) "public"."orders";',
              kind: "ddl",
              missingWhere: false,
            },
          ],
        } satisfies PreparedPlan;
      }
      if (command === "authorize_plan") {
        return "approval-maint-1";
      }
      return undefined;
    });

    const executeSpy = vi
      .spyOn(client, "execute")
      .mockImplementation(async (_planId, _approval, onEvent) => {
        onEvent({
          type: "finished",
          runId: "run-maint-1",
          status: "completed",
          elapsedMs: 9,
        });
        return "run-maint-1";
      });

    render(
      <StructureView
        schema={sampleSchema}
        details={sampleDetails}
        connectionId="conn-1"
        onRefresh={onRefreshSpy}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Table maintenance" }));

    await waitFor(() => {
      expect(screen.getByLabelText("Generated maintenance SQL preview").textContent).toBe(
        'ANALYZE "public"."orders";',
      );
    });

    // Select column "status" and VERBOSE for ANALYZE
    fireEvent.click(screen.getByLabelText("Include VERBOSE output"));
    fireEvent.click(screen.getByLabelText("Analyze column status"));

    await waitFor(() => {
      expect(screen.getByLabelText("Generated maintenance SQL preview").textContent).toBe(
        'ANALYZE (VERBOSE) "public"."orders" ("status");',
      );
    });

    // Switch to VACUUM and toggle VACUUM FULL
    fireEvent.click(screen.getByRole("radio", { name: "VACUUM" }));
    fireEvent.click(screen.getByLabelText("VACUUM FULL (rewrite table and lock exclusively)"));

    await waitFor(() => {
      expect(screen.getByLabelText("Generated maintenance SQL preview").textContent).toBe(
        'VACUUM (FULL, ANALYZE) "public"."orders";',
      );
      expect(
        screen.getByText(
          /VACUUM FULL rewrites the entire table and holds an ACCESS EXCLUSIVE lock/,
        ),
      ).toBeTruthy();
    });

    fireEvent.click(screen.getByRole("button", { name: "Review & authorize…" }));

    await waitFor(() => {
      expect(screen.getByRole("dialog", { name: "Review this operation" })).toBeTruthy();
    });
    expect(callSpy).toHaveBeenCalledWith(
      "prepare_table_maintenance",
      expect.objectContaining({
        sessionId: "sess-1",
        action: {
          type: "vacuum",
          table: { schema: "public", name: "orders" },
          analyze: true,
          verbose: false,
          full: true,
        },
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: "Authorize and run" }));

    await waitFor(() => {
      expect(executeSpy).toHaveBeenCalledWith(
        "plan-maint-1",
        "approval-maint-1",
        expect.any(Function),
      );
    });
    expect(refreshCatalogSpy).toHaveBeenCalledWith("conn-1");
    expect(onRefreshSpy).toHaveBeenCalledTimes(1);
  });

  it("blocks VACUUM submission in the dialog when the session has an open manual transaction", async () => {
    useStudio.setState({
      sessionStates: {
        "sess-1": {
          sessionId: "sess-1",
          transactionMode: "manual",
          transactionState: "inTransaction",
          contextKind: "schema",
          activeContext: "public",
          database: "app_db",
          role: "postgres",
          searchPath: ["public"],
        },
      },
    });

    vi.spyOn(client, "call").mockResolvedValue({
      statement: {
        sql: 'VACUUM (ANALYZE) "public"."orders";',
        description: "Vacuum and analyze table public.orders",
        objectKind: "table",
        objectName: "orders",
        destructive: false,
      },
      warnings: ["VACUUM cannot run inside a transaction block."],
      nonTransactional: true,
    } satisfies TableMaintenancePreview);

    render(<StructureView schema={sampleSchema} details={sampleDetails} connectionId="conn-1" />);

    fireEvent.click(screen.getByRole("button", { name: "Table maintenance" }));
    fireEvent.click(screen.getByRole("radio", { name: "VACUUM" }));

    await waitFor(() => {
      expect(screen.getByText(/currently has an open manual transaction/i)).toBeTruthy();
    });
    expect(
      (screen.getByRole("button", { name: "Review & authorize…" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });
});
