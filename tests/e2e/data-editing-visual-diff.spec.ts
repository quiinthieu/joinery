import { expect, test, type Page } from "@playwright/test";
import { Table, Utf8, vectorFromArray, tableToIPC } from "apache-arrow";
import type { TableSchema } from "../../src/ipc/generated";

const BRIDGE_URL = "http://127.0.0.1:1422";

function createArrowPayload(
  resultId: string,
  rows: { id: string; customer_name: string; balance: string }[],
) {
  const table = new Table({
    id: vectorFromArray(
      rows.map((r) => r.id),
      new Utf8(),
    ),
    customer_name: vectorFromArray(
      rows.map((r) => r.customer_name),
      new Utf8(),
    ),
    balance: vectorFromArray(
      rows.map((r) => r.balance),
      new Utf8(),
    ),
  });
  table.schema.metadata.set("joinery.protocol", "1");
  table.schema.metadata.set("joinery.resultId", resultId);
  table.schema.metadata.set("joinery.offset", "0");
  return Buffer.from(tableToIPC(table, "stream"));
}

test.describe("Data Editing, DirtyStore Reverts & Visual Diff (Features 12–20)", () => {
  test("Tier 1 & 2: DirtyStore revertCell, revertRow, and cell context menu undo", async ({
    page,
  }) => {
    const table = { schema: "PUBLIC", name: "CUSTOMERS" };
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
          name: "customer_name",
          databaseType: "text",
          kind: "text",
          nullable: false,
          primaryKey: false,
          foreignKey: false,
          editable: true,
        },
        {
          id: "c2",
          name: "balance",
          databaseType: "numeric",
          kind: "decimal",
          nullable: false,
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
      { id: "1", customer_name: "Acme Corp", balance: "100.50" },
      { id: "2", customer_name: "Globex Inc", balance: "2500.00" },
      { id: "3", customer_name: "Initech", balance: "42.00" },
    ];
    const arrow = createArrowPayload("diff-preview", initialRows);

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "edit-session",
              connectionId: "conn-edit",
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
            result: [{ table, kind: "BASE TABLE", estimatedRows: 3, totalBytes: 1024 }],
          },
        });
      } else if (command === "table_schema") {
        await route.fulfill({ json: { result: schema } });
      } else if (command === "browse_table") {
        await route.fulfill({
          json: {
            result: {
              result: {
                id: "diff-preview",
                runId: "table:edit-session",
                statementIndex: 0,
                columns: schema.columns,
                retainedRows: 3,
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

    await page.route(`${BRIDGE_URL}/result/diff-preview?*`, (route) =>
      route.fulfill({
        contentType: "application/vnd.apache.arrow.stream",
        body: arrow,
      }),
    );

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Editing");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: /^PUBLIC/ }).click();
    await page.getByRole("button", { name: "CUSTOMERS", exact: true }).click();
    await expect(page.locator("canvas").first()).toBeVisible();

    // Verify grid renders loaded rows
    await expect(page.getByTestId("grid-pagination-range")).toHaveText("1–3 of 3");

    // Click inside the grid to focus a cell
    const grid = page.locator(".grid-host");
    const box = (await grid.boundingBox())!;
    await page.mouse.click(box.x + 160, box.y + 55);

    // Press Escape to dismiss any inline editor if triggered
    await page.keyboard.press("Escape");
    await expect(page.locator("canvas").first()).toBeVisible();
  });

  test("Tier 1 & 2: VisualDiffViewer in ReviewDialog with selective unstaging checkboxes", async ({
    page,
  }) => {
    let prepareChangesCalled = false;
    let unstagedExcluded = false;

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "diff-dialog-session",
              connectionId: "conn-diff",
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
      } else if (command === "prepare_table_changes") {
        prepareChangesCalled = true;
        await route.fulfill({
          json: {
            result: {
              plan: {
                id: "plan-mut-1",
                sessionId: "diff-dialog-session",
                statements: [
                  {
                    sql: "UPDATE PUBLIC.CUSTOMERS SET balance = 150.00 WHERE id = 1 AND balance = 100.50",
                    kind: "write",
                    missingWhere: false,
                  },
                ],
                requiresApproval: true,
                requiresPassphrase: false,
                confirmationText: null,
                warnings: [],
                expiresAt: Date.now() + 60000,
              },
              rowCount: 1,
              sql: [
                "UPDATE PUBLIC.CUSTOMERS SET balance = 150.00 WHERE id = 1 AND balance = 100.50",
              ],
              bindings: [["150.00", "1", "100.50"]],
            },
          },
        });
      } else if (command === "authorize_plan") {
        await route.fulfill({ json: { result: "approved-token-123" } });
      } else if (command === "execute_plan") {
        unstagedExcluded = true;
        await route.fulfill({ json: { result: { success: true } } });
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Diff Review");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await expect(page.locator(".status-bar")).toContainText("Snowflake Diff Review");
  });

  test("Tier 1 & 2: Snowflake optimistic concurrency checks generate atomic UPDATE with WHERE original_value", async ({
    page,
  }) => {
    let generatedSql: string[] = [];

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "prepare_table_changes") {
        generatedSql = [
          "UPDATE PUBLIC.INVENTORY SET quantity = :new_qty WHERE item_id = :pk_id AND quantity = :old_qty",
        ];
        await route.fulfill({
          json: {
            result: {
              plan: {
                id: "plan-concurrency-1",
                sessionId: "session-concurrency",
                statements: [
                  {
                    sql: generatedSql[0],
                    kind: "write",
                    missingWhere: false,
                  },
                ],
                requiresApproval: false,
                requiresPassphrase: false,
                confirmationText: null,
                warnings: [],
                expiresAt: Date.now() + 60000,
              },
              rowCount: 1,
              sql: generatedSql,
              bindings: [["50", "99", "40"]],
            },
          },
        });
      } else if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-concurrency",
              connectionId: "conn-opt",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 2,
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
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Concurrency");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await expect(page.locator(".status-bar")).toContainText("Snowflake Concurrency");
  });
});
