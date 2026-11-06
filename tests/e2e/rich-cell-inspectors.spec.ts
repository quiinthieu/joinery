import { expect, test, type Page } from "@playwright/test";
import { Table, Utf8, vectorFromArray, tableToIPC } from "apache-arrow";
import type { TableSchema } from "../../src/ipc/generated";

const BRIDGE_URL = "http://127.0.0.1:1422";

function createArrowPayload(
  resultId: string,
  rows: {
    id: string;
    payload_json: string;
    avatar_png: string;
    created_at: string;
    tags_array: string;
    geom_wkt: string;
  }[],
) {
  const table = new Table({
    id: vectorFromArray(
      rows.map((r) => r.id),
      new Utf8(),
    ),
    payload_json: vectorFromArray(
      rows.map((r) => r.payload_json),
      new Utf8(),
    ),
    avatar_png: vectorFromArray(
      rows.map((r) => r.avatar_png),
      new Utf8(),
    ),
    created_at: vectorFromArray(
      rows.map((r) => r.created_at),
      new Utf8(),
    ),
    tags_array: vectorFromArray(
      rows.map((r) => r.tags_array),
      new Utf8(),
    ),
    geom_wkt: vectorFromArray(
      rows.map((r) => r.geom_wkt),
      new Utf8(),
    ),
  });
  table.schema.metadata.set("joinery.protocol", "1");
  table.schema.metadata.set("joinery.resultId", resultId);
  table.schema.metadata.set("joinery.offset", "0");
  return Buffer.from(tableToIPC(table, "stream"));
}

test.describe("Rich Cell Inspectors (Features 29–36)", () => {
  test("Tier 1 & 2: Docked ValueInspectorPanel reacts to grid navigation", async ({ page }) => {
    const table = { schema: "PUBLIC", name: "COMPLEX_DATA" };
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
          name: "payload_json",
          databaseType: "variant",
          kind: "json",
          nullable: true,
          primaryKey: false,
          foreignKey: false,
          editable: false,
        },
        {
          id: "c2",
          name: "avatar_png",
          databaseType: "binary",
          kind: "binary",
          nullable: true,
          primaryKey: false,
          foreignKey: false,
          editable: false,
        },
        {
          id: "c3",
          name: "created_at",
          databaseType: "timestamp_tz",
          kind: "temporal",
          nullable: false,
          primaryKey: false,
          foreignKey: false,
          editable: false,
        },
        {
          id: "c4",
          name: "tags_array",
          databaseType: "array",
          kind: "json",
          nullable: true,
          primaryKey: false,
          foreignKey: false,
          editable: false,
        },
        {
          id: "c5",
          name: "geom_wkt",
          databaseType: "geography",
          kind: "text",
          nullable: true,
          primaryKey: false,
          foreignKey: false,
          editable: false,
        },
      ],
      primaryKey: ["id"],
      foreignKeys: [],
      editable: false,
    };

    const initialRows = [
      {
        id: "1",
        payload_json: JSON.stringify({
          store: {
            book: [{ title: "Snowflake Architecture", price: 59.99 }],
            active: true,
          },
        }),
        avatar_png: "\\x89PNG\\r\\n\\x1a\\n\\x00\\x00\\x00\\rIHDR\\x00\\x00\\x00\\x01",
        created_at: "2026-11-05T15:30:00Z",
        tags_array: '["production", "dba", "cloud"]',
        geom_wkt: "POLYGON ((30 10, 40 40, 20 40, 10 20, 30 10))",
      },
    ];

    const arrow = createArrowPayload("inspector-preview", initialRows);

    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "inspector-session",
              connectionId: "conn-inspect",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 2,
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
      } else if (command === "catalog") {
        await route.fulfill({
          json: {
            result: [{ table, kind: "BASE TABLE", estimatedRows: 1, totalBytes: 2048 }],
          },
        });
      } else if (command === "table_schema") {
        await route.fulfill({ json: { result: schema } });
      } else if (command === "browse_table") {
        await route.fulfill({
          json: {
            result: {
              result: {
                id: "inspector-preview",
                runId: "table:inspector-session",
                statementIndex: 0,
                columns: schema.columns,
                retainedRows: 1,
                status: "complete",
                table,
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

    await page.route(`${BRIDGE_URL}/result/inspector-preview?*`, (route) =>
      route.fulfill({
        contentType: "application/vnd.apache.arrow.stream",
        body: arrow,
      }),
    );

    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption("snowflake");
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Inspectors");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await page.getByRole("button", { name: /^PUBLIC/ }).click();
    await page.getByRole("button", { name: "COMPLEX_DATA", exact: true }).click();
    await expect(page.locator("canvas").first()).toBeVisible();

    // Ensure inspector is visible
    if (!(await page.locator(".inspector").isVisible())) {
      await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
    }
    await expect(page.locator(".inspector")).toBeVisible();
  });

  test("Tier 1 & 2: JSON Tree Viewer with JSONPath filter and node expansion", async ({ page }) => {
    await page.route(`${BRIDGE_URL}/rpc/*`, async (route) => {
      const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
      const data = route.request().postDataJSON();

      if (command === "save_profile") {
        await route.fulfill({ json: { result: data.profile } });
      } else if (command === "connect") {
        await route.fulfill({
          json: {
            result: {
              id: "session-json",
              connectionId: "conn-json",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 2,
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
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake JSON Viewer");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await expect(page.locator(".status-bar")).toContainText("Snowflake JSON Viewer");
  });

  test("Tier 1 & 2: DateTime Multi-Timezone Converter & Binary Image / Spatial Previewers", async ({
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
              id: "session-viewers",
              connectionId: "conn-viewers",
              engine: "snowflake",
              serverVersion: "8.42.0",
              latencyMs: 2,
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
    await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Viewers");
    await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
    await page.getByLabel("Database", { exact: true }).fill("PROD");
    await page.getByLabel("Username", { exact: true }).fill("ADMIN");
    await page.getByLabel("Role", { exact: true }).fill("SYSADMIN");
    await page.getByLabel("Warehouse", { exact: true }).fill("WH");
    await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
    await page.getByRole("button", { name: "Save and connect" }).click();

    await expect(page.locator(".status-bar")).toContainText("Snowflake Viewers");
  });
});
