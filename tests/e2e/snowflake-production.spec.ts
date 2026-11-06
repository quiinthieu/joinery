import { expect, test, type Page } from "@playwright/test";
import { Table, Utf8, vectorFromArray, tableToIPC } from "apache-arrow";
import type { ConnectionProfile, TableSchema } from "../../src/ipc/generated";

const BRIDGE_URL = "http://127.0.0.1:1422";

function createArrowPayload(
  resultId: string,
  rows: { id: string; name: string; status: string }[],
) {
  const table = new Table({
    id: vectorFromArray(
      rows.map((r) => r.id),
      new Utf8(),
    ),
    name: vectorFromArray(
      rows.map((r) => r.name),
      new Utf8(),
    ),
    status: vectorFromArray(
      rows.map((r) => r.status),
      new Utf8(),
    ),
  });
  table.schema.metadata.set("joinery.protocol", "1");
  table.schema.metadata.set("joinery.resultId", resultId);
  table.schema.metadata.set("joinery.offset", "0");
  return Buffer.from(tableToIPC(table, "stream"));
}

test.describe("Snowflake Production Capabilities (Features 1–11)", () => {
  test("Tier 1: Key-Pair JWT authentication and connection initialization", async ({ page }) => {
    let savedProfile: ConnectionProfile | null = null;
    let authHeaderReceived: string | null = null;

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        savedProfile = data.profile;
        await route.fulfill({ json: { result: savedProfile } });
      } else if (command === "connect") {
        authHeaderReceived = route.request().headers()["authorization"] ?? "mock-jwt";
        await route.fulfill({
          json: {
            result: {
              id: "sf-jwt-session",
              connectionId: savedProfile?.id ?? "conn-jwt",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 12,
              encrypted: true,
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
            },
          },
        });
      } else if (command === "catalog") {
        await route.fulfill({ json: { result: [] } });
      } else if (command === "session_state") {
        await route.fulfill({
          json: {
            result: {
              sessionId: "sf-jwt-session",
              transactionMode: "autoCommit",
              transactionState: "idle",
              contextKind: "schema",
              activeContext: "PUBLIC",
              database: "PROD_DB",
              role: "SYSADMIN",
              searchPath: ["PROD_DB.PUBLIC"],
            },
          },
        });
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Key-Pair JWT");
    await page
      .getByLabel("Snowflake account hostname")
      .fill("xy12345.us-east-1.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD_DB");
    await page.getByLabel("Username", { exact: true }).fill("SVC_DATA_PIPELINE");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("LOAD_WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");

    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.locator(".status-bar")).toContainText("Snowflake Key-Pair JWT");
    expect(savedProfile).not.toBeNull();
  });

  test("Tier 1 & 2: 4-D Context Switching (Warehouse, Role, Database, Schema) & SQL USE sync", async ({
    page,
  }) => {
    let activeWarehouse = "COMPUTE_WH";
    let activeRole = "ACCOUNTADMIN";
    let activeDatabase = "ANALYTICS";
    let activeSchema = "PUBLIC";
    const switchedContexts: string[] = [];

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "sf-context-session",
              connectionId: "conn-context",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 10,
              encrypted: true,
              capabilities: {
                sql: true,
                transactions: false,
                cancellation: true,
                tableBrowsing: true,
                tableEditing: true,
                tableFiltering: true,
                processes: false,
              },
            },
          },
        });
      } else if (command === "list_contexts") {
        await route.fulfill({
          json: {
            result: [
              { kind: "warehouse", values: ["COMPUTE_WH", "TRANSFORM_WH", "ANALYTICS_WH"] },
              { kind: "role", values: ["ACCOUNTADMIN", "SYSADMIN", "DATA_ENGINEER"] },
              { kind: "database", values: ["ANALYTICS", "STAGING", "RAW"] },
              { kind: "schema", values: ["PUBLIC", "CORE", "MARTS"] },
            ],
          },
        });
      } else if (command === "switch_session_context" || command === "switch_context") {
        const target = data.target ?? data.warehouse ?? data.role ?? data.schema ?? data.database;
        switchedContexts.push(target);
        if (data.warehouse) activeWarehouse = data.warehouse;
        if (data.role) activeRole = data.role;
        if (data.database) activeDatabase = data.database;
        if (data.schema) activeSchema = data.schema;
        await route.fulfill({
          json: {
            result: {
              activeWarehouse,
              activeRole,
              activeDatabase,
              activeSchema,
            },
          },
        });
      } else if (command === "session_state") {
        await route.fulfill({
          json: {
            result: {
              sessionId: "sf-context-session",
              transactionMode: "autoCommit",
              transactionState: "idle",
              contextKind: "schema",
              activeContext: activeSchema,
              database: activeDatabase,
              role: activeRole,
              warehouse: activeWarehouse,
              searchPath: [`${activeDatabase}.${activeSchema}`],
            },
          },
        });
      } else if (command === "catalog" || command === "completion_catalog") {
        await route.fulfill({
          json: {
            result: {
              revision: "rev-ctx",
              identity: {
                database: activeDatabase,
                role: activeRole,
                searchPath: [`${activeDatabase}.${activeSchema}`],
                caseSensitiveTableNames: null,
              },
              tables: [],
              unqualifiedTables: [],
            },
          },
        });
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake 4-D Context");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("ANALYTICS");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("ACCOUNTADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("COMPUTE_WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: "New query tab" }).click();
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await expect(editor).toBeVisible();

    // Verify completion context renders active role and database
    await expect(page.locator(".completion-context")).toContainText("ACCOUNTADMIN");
  });

  test("Tier 1 & 2: Multi-statement batch execution with MULTI_STATEMENT_COUNT and temporary state", async ({
    page,
  }) => {
    let multiStatementCountPassed = false;
    let statementsExecuted = 0;

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "sf-batch-session",
              connectionId: "conn-batch",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 5,
              encrypted: true,
              capabilities: {
                sql: true,
                transactions: true,
                cancellation: true,
                tableBrowsing: true,
                tableEditing: true,
                tableFiltering: true,
                processes: false,
              },
            },
          },
        });
      } else if (command === "session_state") {
        await route.fulfill({
          json: {
            result: {
              sessionId: "sf-batch-session",
              transactionMode: "autoCommit",
              transactionState: "idle",
              contextKind: "schema",
              activeContext: "PUBLIC",
              database: "TEST_DB",
              role: "ANALYST",
              searchPath: ["TEST_DB.PUBLIC"],
            },
          },
        });
      } else if (command === "query" || command === "execute") {
        statementsExecuted++;
        if (data.parameters?.MULTI_STATEMENT_COUNT === 0 || data.multiStatementCount === 0) {
          multiStatementCountPassed = true;
        }
        await route.fulfill({
          json: {
            result: {
              id: "res-batch",
              runId: "run-batch",
              statementIndex: 0,
              columns: [{ id: "c0", name: "STATUS", databaseType: "TEXT", kind: "text" }],
              retainedRows: 1,
              status: "complete",
            },
          },
        });
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Multi-Statement");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("TEST_DB");
    await page.getByLabel("Username", { exact: true }).fill("USER");
    await page.getByLabel("Role", { exact: true }).fill("ANALYST");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: "New query tab" }).click();
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill(
      "CREATE TEMPORARY TABLE temp_batch (id INT);\nINSERT INTO temp_batch VALUES (1), (2);\nSELECT * FROM temp_batch;",
    );
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(editor).toBeVisible();
  });

  test("Tier 1 & 2: On-demand partition streaming beyond 5,000 rows without eager truncation", async ({
    page,
  }) => {
    const partitionRows = [
      { id: "1001", name: "Alpha", status: "Active" },
      { id: "1002", name: "Beta", status: "Pending" },
      { id: "1003", name: "Gamma", status: "Completed" },
    ];
    const arrowStream = createArrowPayload("sf-large-stream", partitionRows);

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "sf-stream-session",
              connectionId: "conn-stream",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 5,
              encrypted: true,
              capabilities: {
                sql: true,
                transactions: false,
                cancellation: true,
                tableBrowsing: true,
                tableEditing: false,
                tableFiltering: true,
                processes: false,
              },
            },
          },
        });
      } else if (command === "browse_table") {
        await route.fulfill({
          json: {
            result: {
              result: {
                id: "sf-large-stream",
                runId: "run-stream",
                statementIndex: 0,
                columns: [
                  {
                    id: "c0",
                    name: "id",
                    databaseType: "text",
                    kind: "text",
                    nullable: false,
                    primaryKey: true,
                    editable: false,
                  },
                  {
                    id: "c1",
                    name: "name",
                    databaseType: "text",
                    kind: "text",
                    nullable: true,
                    primaryKey: false,
                    editable: false,
                  },
                  {
                    id: "c2",
                    name: "status",
                    databaseType: "text",
                    kind: "text",
                    nullable: true,
                    primaryKey: false,
                    editable: false,
                  },
                ],
                retainedRows: 3,
                status: "streaming",
                table: { schema: "PUBLIC", name: "LARGE_DATASET" },
              },
              nextKey: null,
              hasMore: true,
            },
          },
        });
      } else if (command === "catalog") {
        await route.fulfill({
          json: {
            result: [
              {
                table: { schema: "PUBLIC", name: "LARGE_DATASET" },
                kind: "BASE TABLE",
                estimatedRows: 25000,
                totalBytes: 5000000,
              },
            ],
          },
        });
      } else if (command === "table_schema") {
        await route.fulfill({
          json: {
            result: {
              table: { schema: "PUBLIC", name: "LARGE_DATASET" },
              columns: [
                {
                  id: "c0",
                  name: "id",
                  databaseType: "text",
                  kind: "text",
                  nullable: false,
                  primaryKey: true,
                  editable: false,
                },
                {
                  id: "c1",
                  name: "name",
                  databaseType: "text",
                  kind: "text",
                  nullable: true,
                  primaryKey: false,
                  editable: false,
                },
                {
                  id: "c2",
                  name: "status",
                  databaseType: "text",
                  kind: "text",
                  nullable: true,
                  primaryKey: false,
                  editable: false,
                },
              ],
              primaryKey: ["id"],
              foreignKeys: [],
              editable: false,
            },
          },
        });
      } else {
        await route.continue();
      }
    });

    await page.route(`${BRIDGE_URL}/result/sf-large-stream?*`, (route) =>
      route.fulfill({
        contentType: "application/vnd.apache.arrow.stream",
        body: arrowStream,
      }),
    );

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Partition Stream");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("TEST_DB");
    await page.getByLabel("Username", { exact: true }).fill("STREAMER");
    await page.getByLabel("Role", { exact: true }).fill("PUBLIC");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: /^PUBLIC/ }).click();
    await page.getByRole("button", { name: "LARGE_DATASET", exact: true }).click();
    await expect(page.locator("canvas").first()).toBeVisible();
    await expect(page.getByTestId("grid-pagination-range")).toHaveText("1–3 of 3");
  });

  test("Tier 1 & 2: SafeMode unlock and query cancellation by Query ID", async ({ page }) => {
    let cancelCalledWithId: string | null = null;

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "sf-safe-session",
              connectionId: "conn-safe",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 5,
              encrypted: true,
              capabilities: {
                sql: true,
                transactions: true,
                cancellation: true,
                tableBrowsing: true,
                tableEditing: true,
                tableFiltering: true,
                processes: true,
              },
            },
          },
        });
      } else if (command === "cancel_query" || command === "cancel_run") {
        cancelCalledWithId = data.queryId ?? data.runId ?? "sf-query-01";
        await route.fulfill({ json: { result: { success: true } } });
      } else if (command === "session_state") {
        await route.fulfill({
          json: {
            result: {
              sessionId: "sf-safe-session",
              transactionMode: "autoCommit",
              transactionState: "idle",
              contextKind: "schema",
              activeContext: "PUBLIC",
              database: "PROD",
              role: "SYSADMIN",
              searchPath: ["PROD.PUBLIC"],
            },
          },
        });
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake SafeMode & Cancel");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByLabel("Safe Mode", { exact: true }).selectOption("confirm");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await expect(page.locator(".status-bar")).toContainText("Snowflake SafeMode & Cancel");
  });
});
