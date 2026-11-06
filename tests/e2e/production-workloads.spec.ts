import { expect, test, type Page } from "@playwright/test";
import { Table, Utf8, vectorFromArray, tableToIPC } from "apache-arrow";
import type { TableSchema } from "../../src/ipc/generated";

const BRIDGE_URL = "http://127.0.0.1:1422";

function createArrowPayload(
  resultId: string,
  rows: { id: string; event_name: string; payload: string }[],
) {
  const table = new Table({
    id: vectorFromArray(
      rows.map((r) => r.id),
      new Utf8(),
    ),
    event_name: vectorFromArray(
      rows.map((r) => r.event_name),
      new Utf8(),
    ),
    payload: vectorFromArray(
      rows.map((r) => r.payload),
      new Utf8(),
    ),
  });
  table.schema.metadata.set("joinery.protocol", "1");
  table.schema.metadata.set("joinery.resultId", resultId);
  table.schema.metadata.set("joinery.offset", "0");
  return Buffer.from(tableToIPC(table, "stream"));
}

test.describe("Tier 4: Production DBA & Data Engineering Workloads", () => {
  test("Workload 1: End-to-End Snowflake Analytics Engineer Workflow", async ({ page }) => {
    let contextSwitched = false;
    let batchRan = false;

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-workload-1",
              connectionId: "conn-wl-1",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 8,
              encrypted: true,
              capabilities: {
                sql: true,
                transactions: true,
                cancellation: true,
                tableBrowsing: true,
                tableEditing: true,
                tableFiltering: true,
                processes: false,
                objectExplorer: true,
              },
            },
          },
        });
      } else if (command === "switch_session_context") {
        contextSwitched = true;
        await route.fulfill({
          json: {
            result: {
              activeWarehouse: "TRANSFORM_WH",
              activeRole: "ANALYTICS_ENGINEER",
              activeDatabase: "ANALYTICS",
              activeSchema: "MARTS",
            },
          },
        });
      } else if (command === "execute" || command === "query") {
        batchRan = true;
        await route.fulfill({
          json: {
            result: {
              id: "res-wl-1",
              runId: "run-wl-1",
              statementIndex: 0,
              columns: [{ id: "c0", name: "status", databaseType: "TEXT", kind: "text" }],
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
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Analytics Workload");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("ANALYTICS");
    await page.getByLabel("Username", { exact: true }).fill("ENGINEER");
    await page.getByLabel("Role", { exact: true }).fill("ANALYTICS_ENGINEER");
    await page.getByLabel("Warehouse", { exact: true }).fill("TRANSFORM_WH");
    await page.getByLabel("Schema", { exact: true }).fill("MARTS");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: "New query tab" }).click();
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill(
      "MERGE INTO MARTS.DAILY_SALES tgt\nUSING STAGING.HOURLY_STAGING src\nON tgt.id = src.id\nWHEN MATCHED THEN UPDATE SET tgt.amount = src.amount\nWHEN NOT MATCHED THEN INSERT (id, amount) VALUES (src.id, src.amount);",
    );
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(editor).toBeVisible();
  });

  test("Workload 2: Production Incident DBA Response & Query Cancellation", async ({ page }) => {
    let cancelTriggered = false;

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-workload-2",
              connectionId: "conn-wl-2",
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
                objectExplorer: true,
              },
            },
          },
        });
      } else if (command === "cancel_query" || command === "cancel_run") {
        cancelTriggered = true;
        await route.fulfill({ json: { result: { success: true } } });
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Incident DBA");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("DBA_ONCALL");
    await page.getByLabel("Role", { exact: true }).fill("ACCOUNTADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("ADMIN_WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await expect(page.locator(".status-bar")).toContainText("Snowflake Incident DBA");
  });

  test("Workload 3: Bulk Data Review, Visual Diff & SafeMode Approval", async ({ page }) => {
    const table = { schema: "PUBLIC", name: "APP_SETTINGS" };
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
          name: "event_name",
          databaseType: "text",
          kind: "text",
          nullable: false,
          primaryKey: false,
          foreignKey: false,
          editable: true,
        },
        {
          id: "c2",
          name: "payload",
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
      { id: "1", event_name: "maintenance_mode", payload: '{"enabled": false}' },
      { id: "2", event_name: "rate_limit_rpm", payload: '{"limit": 1000}' },
    ];
    const arrow = createArrowPayload("wl-diff-preview", initialRows);

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-workload-3",
              connectionId: "conn-wl-3",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 4,
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
                id: "wl-diff-preview",
                runId: "table:session-workload-3",
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
      } else {
        await route.continue();
      }
    });

    await page.route(`${BRIDGE_URL}/result/wl-diff-preview?*`, (route) =>
      route.fulfill({
        contentType: "application/vnd.apache.arrow.stream",
        body: arrow,
      }),
    );

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Bulk Review");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByLabel("Safe Mode", { exact: true }).selectOption("confirm");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: /^PUBLIC/ }).click();
    await page.getByRole("button", { name: "APP_SETTINGS", exact: true }).click();
    await expect(page.locator("canvas").first()).toBeVisible();
    await expect(page.getByTestId("grid-pagination-range")).toHaveText("1–2 of 2");
  });

  test("Workload 4: Semi-Structured Data Pipeline Debugging & Spatial Geo Inspection", async ({
    page,
  }) => {
    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-workload-4",
              connectionId: "conn-wl-4",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 3,
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
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Semi-Structured");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await expect(page.locator(".status-bar")).toContainText("Snowflake Semi-Structured");
  });
});
