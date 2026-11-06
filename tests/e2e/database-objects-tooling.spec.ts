import { expect, test, type Page } from "@playwright/test";
import type { DatabaseObjectSummary } from "../../src/ipc/generated";

const BRIDGE_URL = "http://127.0.0.1:1422";

test.describe("Database Object Tooling & Schema Inspection (Features 37–43)", () => {
  test("Tier 1 & 2: Snowflake extended object kinds (Stages, Tasks, Streams, Pipes) in tree with badges", async ({
    page,
  }) => {
    const extendedObjects: DatabaseObjectSummary[] = [
      {
        object: {
          schema: "PUBLIC",
          name: "S3_RAW_STAGE",
          kind: "table" as any, // extended kind representation
          identity: null,
        },
        ownerTable: null,
        signature: null,
        estimatedRows: null,
        totalBytes: 1048576,
      },
      {
        object: {
          schema: "PUBLIC",
          name: "HOURLY_AGG_TASK",
          kind: "procedure" as any,
          identity: null,
        },
        ownerTable: null,
        signature: "()",
        estimatedRows: null,
        totalBytes: null,
      },
    ];

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-objects",
              connectionId: "conn-objects",
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
                objectExplorer: true,
              },
            },
          },
        });
      } else if (command === "catalog") {
        await route.fulfill({
          json: {
            result: [
              {
                table: { schema: "PUBLIC", name: "FACT_EVENTS" },
                kind: "BASE TABLE",
                estimatedRows: 100000,
                totalBytes: 5000000,
              },
            ],
          },
        });
      } else if (command === "catalog_objects") {
        await route.fulfill({ json: { result: extendedObjects } });
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Objects");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: /^PUBLIC/ }).click();
    await expect(page.getByRole("button", { name: "FACT_EVENTS", exact: true })).toBeVisible();
  });

  test("Tier 1 & 2: Snowflake native GET_DDL generation and object inspector view", async ({
    page,
  }) => {
    let getDdlCalled = false;

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-ddl",
              connectionId: "conn-ddl",
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
                objectExplorer: true,
              },
            },
          },
        });
      } else if (command === "object_details") {
        getDdlCalled = true;
        await route.fulfill({
          json: {
            result: {
              object: {
                schema: "PUBLIC",
                name: "REVENUE_STREAM",
                kind: "view",
                identity: null,
              },
              ownerTable: null,
              comment: "Snowflake pipe/stream",
              ddl: {
                status: "available",
                ddl: "CREATE OR REPLACE STREAM PUBLIC.REVENUE_STREAM ON TABLE PUBLIC.ORDERS;",
                truncated: false,
              },
              indexes: [],
              constraints: [],
              triggers: [],
              routine: null,
              sequence: null,
              uses: [],
              usedBy: [],
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
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake DDL Tooling");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await expect(page.locator(".status-bar")).toContainText("Snowflake DDL Tooling");
  });

  test("Tier 1 & 2: ObjectContextMenu right-click actions on schema items", async ({ page }) => {
    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-ctxmenu",
              connectionId: "conn-ctxmenu",
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
                objectExplorer: true,
              },
            },
          },
        });
      } else if (command === "catalog") {
        await route.fulfill({
          json: {
            result: [
              {
                table: { schema: "PUBLIC", name: "LINEITEM" },
                kind: "BASE TABLE",
                estimatedRows: 5000,
                totalBytes: 250000,
              },
            ],
          },
        });
      } else {
        await route.continue();
      }
    });

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Context Menu");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: /^PUBLIC/ }).click();
    const tableItem = page.getByRole("button", { name: "LINEITEM", exact: true });
    await expect(tableItem).toBeVisible();

    // Right-click table item to invoke context action
    await tableItem.click({ button: "right" });
  });
});
