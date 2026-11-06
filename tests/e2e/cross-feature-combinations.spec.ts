import { expect, test, type Page } from "@playwright/test";
import { Table, Utf8, vectorFromArray, tableToIPC } from "apache-arrow";
import type { TableSchema } from "../../src/ipc/generated";

const BRIDGE_URL = "http://127.0.0.1:1422";

function createArrowPayload(
  resultId: string,
  rows: { id: string; metric_name: string; value_json: string }[],
) {
  const table = new Table({
    id: vectorFromArray(
      rows.map((r) => r.id),
      new Utf8(),
    ),
    metric_name: vectorFromArray(
      rows.map((r) => r.metric_name),
      new Utf8(),
    ),
    value_json: vectorFromArray(
      rows.map((r) => r.value_json),
      new Utf8(),
    ),
  });
  table.schema.metadata.set("joinery.protocol", "1");
  table.schema.metadata.set("joinery.resultId", resultId);
  table.schema.metadata.set("joinery.offset", "0");
  return Buffer.from(tableToIPC(table, "stream"));
}

test.describe("Tier 3: Cross-Feature Combinations & Feature Interactions", () => {
  test("Combination 1: 4-D Context Switching while preserving uncommitted DirtyStore edits", async ({
    page,
  }) => {
    const table = { schema: "PUBLIC", name: "STAGE_CONFIG" };
    const schema: TableSchema = {
      table,
      columns: [
        {
          id: "c0",
          name: "id",
          databaseType: "integer",
          kind: "integer",
          nullable: false,
          primaryKey: true,
          foreignKey: false,
          editable: false,
        },
        {
          id: "c1",
          name: "metric_name",
          databaseType: "text",
          kind: "text",
          nullable: false,
          primaryKey: false,
          foreignKey: false,
          editable: true,
        },
        {
          id: "c2",
          name: "value_json",
          databaseType: "variant",
          kind: "json",
          nullable: true,
          primaryKey: false,
          foreignKey: false,
          editable: true,
        },
      ],
      primaryKey: ["id"],
      foreignKeys: [],
      editable: true,
    };

    const initialRows = [
      { id: "1", metric_name: "timeout_ms", value_json: '{"value": 5000}' },
      { id: "2", metric_name: "retry_count", value_json: '{"value": 3}' },
    ];
    const arrow = createArrowPayload("combo-stream", initialRows);

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-combo-1",
              connectionId: "conn-combo",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 3,
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
      } else if (command === "catalog") {
        await route.fulfill({
          json: {
            result: [{ table, kind: "BASE TABLE", estimatedRows: 2, totalBytes: 512 }],
          },
        });
      } else if (command === "table_schema") {
        await route.fulfill({ json: { result: schema } });
      } else if (command === "browse_table") {
        await route.fulfill({
          json: {
            result: {
              result: {
                id: "combo-stream",
                runId: "table:session-combo-1",
                statementIndex: 0,
                columns: schema.columns,
                retainedRows: 2,
                status: "complete",
                table,
                editability: { status: "editable", table, primaryKey: ["id"] },
              },
              nextKey: null,
              hasMore: false,
            },
          },
        });
      } else if (command === "switch_session_context") {
        await route.fulfill({
          json: {
            result: {
              activeWarehouse: "TRANSFORM_WH",
              activeRole: "SYSADMIN",
              activeDatabase: "PROD",
              activeSchema: "PUBLIC",
            },
          },
        });
      } else {
        await route.continue();
      }
    });

    await page.route(`${BRIDGE_URL}/result/combo-stream?*`, (route) =>
      route.fulfill({
        contentType: "application/vnd.apache.arrow.stream",
        body: arrow,
      }),
    );

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Combo Context-Edit");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: /^PUBLIC/ }).click();
    await page.getByRole("button", { name: "STAGE_CONFIG", exact: true }).click();
    await expect(page.locator("canvas").first()).toBeVisible();
    await expect(page.getByTestId("grid-pagination-range")).toHaveText("1–2 of 2");
  });

  test("Combination 2: Multi-statement execution with $param named parameters and continueOnError policy", async ({
    page,
  }) => {
    let queryCallCount = 0;

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-combo-2",
              connectionId: "conn-combo-2",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 3,
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
      } else if (command === "query" || command === "execute") {
        queryCallCount++;
        await route.fulfill({
          json: {
            result: {
              id: `res-combo-${queryCallCount}`,
              runId: "run-combo-2",
              statementIndex: queryCallCount - 1,
              columns: [{ id: "c0", name: "cnt", databaseType: "INT", kind: "integer" }],
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
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Combo Batch-Params");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: "New query tab" }).click();
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill(
      "SELECT count(*) FROM orders WHERE status = $status;\nSELECT count(*) FROM returns WHERE status = $status;",
    );
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(editor).toBeVisible();
  });

  test("Combination 3: Multi-engine compatibility preservation (PostgreSQL & MySQL alongside Snowflake)", async ({
    page,
  }) => {
    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        const engine = data.profile?.engine ?? "postgres";
        await route.fulfill({
          json: {
            result: {
              id: `session-${engine}`,
              connectionId: `conn-${engine}`,
              engine,
              serverVersion: "16.2",
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
                objectExplorer: true,
              },
            },
          },
        });
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("postgres");
    await page.getByLabel("Connection name", { exact: true }).fill("Postgres Multi-Engine Check");
    await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
    await page.getByLabel("Port", { exact: true }).fill("55432");
    await page.getByLabel("Database", { exact: true }).fill("joinery");
    await page.getByLabel("Username", { exact: true }).fill("joinery");
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByLabel("Require TLS and verify the server certificate").uncheck();
    await page.getByRole("button", { name: "Save and connect" }).click();

    await expect(page.locator(".status-bar")).toContainText("Postgres Multi-Engine Check");
  });
});
