import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ConnectionProfile,
  IndexConstraintPreview,
  ObjectDetails,
  PreparedPlan,
  SessionInfo,
  TableInfo,
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

function makeSession(): SessionInfo {
  return {
    id: "sess-1",
    connectionId: "conn-1",
    engine: "postgres",
    serverVersion: "PostgreSQL 18.0",
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
      id: "customer_id",
      name: "customer_id",
      databaseType: "bigint",
      kind: "integer",
      nullable: false,
      primaryKey: false,
      foreignKey: true,
      editable: true,
      insertable: true,
      hasDefault: false,
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
      hasDefault: true,
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
  constraints: [
    {
      name: "orders_pkey",
      kind: "primaryKey",
      columns: ["id"],
      target: null,
      targetColumns: [],
      definition: "PRIMARY KEY (id)",
    },
    {
      name: "orders_status_chk",
      kind: "check",
      columns: ["status"],
      target: null,
      targetColumns: [],
      definition: "CHECK ((status <> ''::text))",
    },
  ],
  indexes: [
    {
      name: "orders_pkey",
      method: "btree",
      unique: true,
      primary: true,
      expressions: ["id"],
      predicate: null,
      definition: "CREATE UNIQUE INDEX orders_pkey ON public.orders USING btree (id)",
    },
    {
      name: "orders_status_idx",
      method: "btree",
      unique: false,
      primary: false,
      expressions: ["status"],
      predicate: null,
      definition: "CREATE INDEX orders_status_idx ON public.orders USING btree (status)",
    },
  ],
  triggers: [],
  routine: null,
  sequence: null,
  uses: [],
  usedBy: [],
};

const sampleCatalog: TableInfo[] = [
  {
    table: { schema: "public", name: "orders" },
    kind: "table",
    estimatedRows: 10,
    totalBytes: 8192,
  },
  {
    table: { schema: "public", name: "customers" },
    kind: "table",
    estimatedRows: 5,
    totalBytes: 8192,
  },
];

describe("StructureView Index & Constraint Editor", () => {
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
      sessions: { "conn-1": makeSession() },
      catalog: { "conn-1": sampleCatalog },
      objects: {},
      selectedConnectionId: "conn-1",
    });
  });

  it("creates a composite partial index with CONCURRENTLY and routes execution through ReviewDialog", async () => {
    const refreshCatalogSpy = vi.fn().mockResolvedValue(undefined);
    const onRefreshSpy = vi.fn();
    useStudio.setState({ refreshCatalog: refreshCatalogSpy });

    const callSpy = vi.spyOn(client, "call").mockImplementation(async (command, args) => {
      if (command === "preview_index_constraint_ddl") {
        const action = (
          args as {
            action: {
              kind: string;
              expressions?: string[];
              predicate?: string | null;
              concurrently?: boolean;
            };
          }
        ).action;
        const cols = (action.expressions ?? ["id"]).map((c) => `"${c}"`).join(", ");
        const conc = action.concurrently ? "CONCURRENTLY " : "";
        const pred = action.predicate ? ` WHERE ${action.predicate}` : "";
        return {
          statement: {
            sql: `CREATE INDEX ${conc}"orders_customer_status_idx" ON "public"."orders" USING btree (${cols})${pred};`,
            description: "Create index orders_customer_status_idx on orders",
            objectKind: "index",
            objectName: "orders.orders_customer_status_idx",
            destructive: false,
          },
          warnings: [
            "CREATE INDEX CONCURRENTLY avoids blocking writes on the table, but cannot run inside a transaction block and may leave an INVALID index if interrupted.",
          ],
        } satisfies IndexConstraintPreview;
      }
      if (command === "prepare_index_constraint_ddl") {
        return {
          id: "plan-idx-1",
          sessionId: "sess-1",
          requiresApproval: true,
          requiresPassphrase: false,
          confirmationText: null,
          expiresAt: Date.now() + 60_000,
          warnings: [
            "DDL changes database structure and may lock tables",
            "CREATE INDEX CONCURRENTLY avoids blocking writes on the table, but cannot run inside a transaction block and may leave an INVALID index if interrupted.",
          ],
          statements: [
            {
              sql: 'CREATE INDEX CONCURRENTLY "orders_customer_status_idx" ON "public"."orders" USING btree ("customer_id", "status") WHERE status = \'open\';',
              kind: "ddl",
              missingWhere: false,
            },
          ],
        } satisfies PreparedPlan;
      }
      if (command === "authorize_plan") {
        return "approval-token-1";
      }
      return undefined;
    });

    const executeSpy = vi
      .spyOn(client, "execute")
      .mockImplementation(async (_planId, _approval, onEvent) => {
        onEvent({
          type: "finished",
          runId: "run-1",
          status: "completed",
          elapsedMs: 8,
        });
        return "run-1";
      });

    render(
      <StructureView
        schema={sampleSchema}
        details={sampleDetails}
        connectionId="conn-1"
        onRefresh={onRefreshSpy}
      />,
    );

    // Primary key backing index shows Constraint-backed; secondary index shows Drop...
    expect(screen.getByText("Constraint-backed")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Drop index orders_status_idx" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Create index…" }));
    expect(screen.getByRole("dialog", { name: "Create index" })).toBeTruthy();

    fireEvent.change(screen.getByLabelText("Index name"), {
      target: { value: "orders_customer_status_idx" },
    });
    fireEvent.change(screen.getByLabelText("Index column 1"), {
      target: { value: "customer_id" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add column" }));
    fireEvent.change(screen.getByLabelText("Index column 2"), {
      target: { value: "status" },
    });
    fireEvent.click(screen.getByLabelText(/CONCURRENTLY/));
    fireEvent.change(screen.getByLabelText("Partial index WHERE predicate"), {
      target: { value: "status = 'open'" },
    });

    await waitFor(() => {
      expect(screen.getByLabelText("Generated DDL preview").textContent).toContain(
        'CREATE INDEX CONCURRENTLY "orders_customer_status_idx" ON "public"."orders" USING btree ("customer_id", "status") WHERE status = \'open\';',
      );
    });

    fireEvent.click(screen.getByRole("button", { name: "Review & authorize…" }));

    await waitFor(() => {
      expect(screen.getByRole("dialog", { name: "Review this operation" })).toBeTruthy();
    });
    expect(callSpy).toHaveBeenCalledWith(
      "prepare_index_constraint_ddl",
      expect.objectContaining({
        sessionId: "sess-1",
        action: expect.objectContaining({
          type: "createIndex",
          name: "orders_customer_status_idx",
          concurrently: true,
          expressions: ["customer_id", "status"],
          predicate: "status = 'open'",
        }),
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: "Authorize and run" }));

    await waitFor(() => {
      expect(executeSpy).toHaveBeenCalledWith(
        "plan-idx-1",
        "approval-token-1",
        expect.any(Function),
      );
    });
    expect(onRefreshSpy).toHaveBeenCalledTimes(1);
    expect(refreshCatalogSpy).toHaveBeenCalledWith("conn-1");
  });

  it("builds a FOREIGN KEY constraint with NOT VALID and opens the generated DDL in a SQL tab", async () => {
    vi.spyOn(client, "call").mockImplementation(async (command, args) => {
      if (command === "table_schema") {
        return {
          table: { schema: "public", name: "customers" },
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
        } satisfies TableSchema;
      }
      if (command === "preview_index_constraint_ddl") {
        const action = (
          args as {
            action: { name: string; kind: string; constraintKind?: string; notValid?: boolean };
          }
        ).action;
        return {
          statement: {
            sql: `ALTER TABLE "public"."orders" ADD CONSTRAINT "${action.name}" FOREIGN KEY ("customer_id") REFERENCES "public"."customers" ("id") ON DELETE CASCADE${action.notValid ? " NOT VALID" : ""};`,
            description: `Add constraint ${action.name} on orders`,
            objectKind: "table",
            objectName: `orders.${action.name}`,
            destructive: false,
          },
          warnings: [
            "NOT VALID skips scanning existing rows and only enforces the foreign key on new or updated rows until VALIDATE CONSTRAINT is run.",
          ],
        } satisfies IndexConstraintPreview;
      }
      return undefined;
    });

    render(<StructureView schema={sampleSchema} details={sampleDetails} connectionId="conn-1" />);

    fireEvent.click(screen.getByRole("button", { name: "Add constraint…" }));
    expect(screen.getByRole("dialog", { name: "Add table constraint" })).toBeTruthy();

    fireEvent.change(screen.getByLabelText("Constraint kind"), {
      target: { value: "foreignKey" },
    });
    fireEvent.change(screen.getByLabelText("Constraint name"), {
      target: { value: "orders_customer_fk" },
    });
    fireEvent.change(screen.getByLabelText("Constraint column 1"), {
      target: { value: "customer_id" },
    });
    fireEvent.change(screen.getByLabelText("Foreign key ON DELETE action"), {
      target: { value: "CASCADE" },
    });
    fireEvent.click(screen.getByLabelText(/NOT VALID/));

    await waitFor(() => {
      expect(screen.getByLabelText("Generated DDL preview").textContent).toContain(
        'ALTER TABLE "public"."orders" ADD CONSTRAINT "orders_customer_fk" FOREIGN KEY ("customer_id") REFERENCES "public"."customers" ("id") ON DELETE CASCADE NOT VALID;',
      );
    });

    fireEvent.click(screen.getByRole("button", { name: "Open in SQL editor" }));
    expect(useStudio.getState().tabs).toHaveLength(1);
    expect(useStudio.getState().tabs[0]?.sql).toContain(
      'ADD CONSTRAINT "orders_customer_fk" FOREIGN KEY ("customer_id")',
    );
  });

  it("disables index and constraint mutation buttons when connection safeMode is readOnly", () => {
    useStudio.setState({
      bootstrap: {
        profiles: [makeProfile({ safeMode: "readOnly" })],
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
    });

    render(<StructureView schema={sampleSchema} details={sampleDetails} connectionId="conn-1" />);

    expect(
      (screen.getByRole("button", { name: "Add constraint…" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "Create index…" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "Drop index orders_status_idx" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    expect(
      (
        screen.getByRole("button", {
          name: "Drop constraint orders_status_chk",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
  });
});
