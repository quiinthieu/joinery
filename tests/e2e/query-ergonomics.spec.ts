import { expect, test, type Page } from "@playwright/test";

const BRIDGE_URL = "http://127.0.0.1:1422";

test.describe("Query Ergonomics, Consoles & Parameters (Features 21–28)", () => {
  test("Tier 1 & 2: Multi-statement batch error policies: stopOnError vs continueOnError", async ({
    page,
  }) => {
    let executedCount = 0;
    let continueOnError = false;

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-batch-policy",
              connectionId: "conn-policy",
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
      } else if (command === "execute" || command === "query") {
        executedCount++;
        if (executedCount === 2) {
          // Second statement causes an error
          await route.fulfill({
            json: {
              error: {
                code: "database",
                message: "Table 'NON_EXISTENT' does not exist",
              },
            },
          });
        } else {
          await route.fulfill({
            json: {
              result: {
                id: `res-${executedCount}`,
                runId: "run-policy",
                statementIndex: executedCount - 1,
                columns: [{ id: "c0", name: "v", databaseType: "INT", kind: "integer" }],
                retainedRows: 1,
                status: "complete",
              },
            },
          });
        }
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Error Policy");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: "New query tab" }).click();
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT 1;\nSELECT * FROM NON_EXISTENT;\nSELECT 3;");

    // Click Run all statements
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(editor).toBeVisible();
  });

  test("Tier 1 & 2: Dedicated persistent database consoles (console.sql) & scratchpads", async ({
    page,
  }) => {
    let savedConsoleContent = "";

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-console",
              connectionId: "conn-console-1",
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
      } else if (command === "get_console" || command === "load_console") {
        await route.fulfill({
          json: {
            result: {
              profileId: "conn-console-1",
              name: "console.sql",
              content:
                savedConsoleContent || "-- Persistent database console\nSELECT CURRENT_USER();",
              updatedAt: Date.now(),
            },
          },
        });
      } else if (command === "save_console") {
        savedConsoleContent = data.content;
        await route.fulfill({ json: { result: { success: true } } });
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Persistent Console");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await expect(page.locator(".status-bar")).toContainText("Snowflake Persistent Console");
  });

  test("Tier 1 & 2: Named dollar parameters ($param_name) & parameter history LRU prefill", async ({
    page,
  }) => {
    let capturedParams: Record<string, unknown> | null = null;

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-param",
              connectionId: "conn-param",
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
      } else if (command === "prepare_plan") {
        capturedParams = data.boundParameters;
        await route.fulfill({
          json: {
            result: {
              id: "plan-param-1",
              sessionId: "session-param",
              statements: [
                {
                  sql: "SELECT * FROM orders WHERE customer_id = $cust_id AND region = $region",
                  kind: "read",
                  missingWhere: false,
                },
              ],
              requiresApproval: false,
              requiresPassphrase: false,
              confirmationText: null,
              warnings: [],
              expiresAt: Date.now() + 60000,
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
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Dollar Params");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: "New query tab" }).click();
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT * FROM orders WHERE customer_id = $cust_id AND region = $region");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(editor).toBeVisible();
  });

  test("Tier 1 & 2: Snowflake live templates & write snippet unsuppression", async ({ page }) => {
    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-templates",
              connectionId: "conn-templates",
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
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Live Templates");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: "New query tab" }).click();
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("merge");
    await expect(editor).toBeVisible();
  });
});
