import { expect, test } from "@playwright/test";
import { Table, Utf8, vectorFromArray, tableToIPC } from "apache-arrow";
import type { ConnectionProfile, TableSchema } from "../../src/ipc/generated";

test("Snowflake catalog, lazy completion and read-only preview work together", async ({ page }) => {
  const errors: string[] = [];
  const calls: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const table = { schema: "PUBLIC", name: "ORDERS" };
  const schema: TableSchema = {
    table,
    columns: [
      {
        id: "c0",
        name: "ID",
        databaseType: "fixed",
        kind: "integer",
        nullable: false,
        primaryKey: false,
        foreignKey: false,
        editable: false,
      },
      {
        id: "c1",
        name: "NOTES",
        databaseType: "text",
        kind: "text",
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: false,
      },
    ],
    primaryKey: [],
    foreignKeys: [],
    editable: false,
  };
  let profile: ConnectionProfile;
  // Only this isolated test intercepts the API boundary. SSO and production
  // credentials are never used. Rust adapter behavior has separate HTTP tests.
  await page.route("http://127.0.0.1:1422/rpc/*", async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    calls.push(command);
    let result: unknown;
    switch (command) {
      case "save_profile":
        profile = route.request().postDataJSON().profile;
        result = profile;
        break;
      case "connect":
        result = {
          id: "sf-session",
          connectionId: profile.id,
          engine: "snowflake",
          serverVersion: "mock",
          latencyMs: 1,
          encrypted: true,
          capabilities: {
            sql: true,
            transactions: false,
            cancellation: true,
            tableBrowsing: true,
            tableEditing: false,
            processes: false,
          },
        };
        break;
      case "catalog":
        result = [{ table, kind: "BASE TABLE", estimatedRows: 9000, totalBytes: 4000 }];
        break;
      case "table_schema":
        result = schema;
        break;
      case "browse_table":
        result = {
          result: {
            id: "sf-preview",
            runId: "table:sf-session",
            statementIndex: 0,
            columns: schema.columns,
            retainedRows: 2,
            status: "truncated",
            table,
          },
          nextKey: null,
          hasMore: false,
        };
        break;
      case "health":
        result = 1;
        break;
      case "release_result":
        result = null;
        break;
      default:
        await route.continue();
        return;
    }
    await route.fulfill({ json: { result } });
  });
  const arrow = new Table({
    c0: vectorFromArray(["9223372036854775808", "2"], new Utf8()),
    c1: vectorFromArray([null, ""], new Utf8()),
  });
  arrow.schema.metadata.set("joinery.protocol", "1");
  arrow.schema.metadata.set("joinery.resultId", "sf-preview");
  arrow.schema.metadata.set("joinery.offset", "0");
  const binary = tableToIPC(arrow, "stream");
  await page.route("http://127.0.0.1:1422/result/sf-preview?*", (route) =>
    route.fulfill({
      contentType: "application/vnd.apache.arrow.stream",
      body: Buffer.from(binary),
    }),
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Database engine").selectOption("snowflake");
  await page.getByLabel("Connection name", { exact: true }).fill("Snowflake fixture");
  await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("DB");
  await page.getByLabel("Username", { exact: true }).fill("FIXTURE");
  await page.getByLabel("Role", { exact: true }).fill("READ_ONLY");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH");
  await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: /^PUBLIC/ }).click();
  await page.getByRole("button", { name: "ORDERS", exact: true }).click();
  await expect(page.locator("canvas").first()).toBeVisible();
  await expect(page.locator(".result-caption")).toContainText("Read-only preview");
  await expect(page.locator(".result-footer")).toContainText("unordered preview");
  await expect(page.locator(".result-footer")).toContainText("Preview limit reached");
  if (!(await page.locator(".inspector").isVisible()))
    await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
  const grid = page.locator(".grid-host");
  const box = (await grid.boundingBox())!;
  await expect(async () => {
    await page.mouse.click(box.x + 90, box.y + 55);
    await expect(page.locator(".inspector-value")).toContainText("9223372036854775808", {
      timeout: 500,
    });
  }).toPass({ timeout: 10000 });
  await page.screenshot({ path: ".cache/screenshots/snowflake-browser.png", fullPage: true });
  await page.mouse.dblclick(box.x + 180, box.y + 55);
  const valueViewer = page.locator("#portal textarea, #portal input").first();
  await expect(valueViewer).not.toBeEditable();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: /^Review \d+ edit/ })).toHaveCount(0);
  await page.getByRole("button", { name: "Structure", exact: true }).click();
  await expect(page.locator(".structure-table")).toContainText("NOTES");
  await page.getByRole("button", { name: "New query tab" }).click();
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  await expect(editor).toContainText("CURRENT_VERSION()");
  await editor.fill("SELECT * FROM ");
  await editor.press("End");
  await page.keyboard.type("PUB");
  await expect(
    page.locator(".cm-tooltip-autocomplete").getByText("PUBLIC", { exact: true }),
  ).toBeVisible();
  await page.locator(".cm-tooltip-autocomplete").getByText("PUBLIC", { exact: true }).click();
  await expect(editor).toContainText("FROM PUBLIC.");
  await page.keyboard.type("OR");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("PUBLIC.ORDERS");
  await editor.fill("SELECT * FROM DB.PUBLIC.");
  await editor.press("End");
  await page.keyboard.type("OR");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("DB.PUBLIC.ORDERS");
  await editor.fill("SELECT o. FROM DB.PUBLIC.ORDERS o");
  await expect.poll(() => calls.filter((c) => c === "table_schema").length).toBeGreaterThan(1);
  await editor.press("Home");
  for (let i = 0; i < 9; i++) await editor.press("ArrowRight");
  await page.keyboard.type("N");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("o.NOTES");
  await expect(async () => {
    await editor.press("Control+Space");
    await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("o.NOTES", {
      timeout: 500,
    });
  }).toPass({ timeout: 10000 });
  expect(calls.filter((c) => c === "browse_table")).toHaveLength(1);
  expect(calls).not.toContain("prepare_changes");
  expect(errors).toEqual([]);
});

test("approved Snowflake sign-in remains connected when table discovery fails", async ({
  page,
}) => {
  const calls: string[] = [];
  let profile: ConnectionProfile;
  let catalogCalls = 0;
  let finishDiscovery!: () => void;
  const discovery = new Promise<void>((resolve) => {
    finishDiscovery = resolve;
  });
  await page.route("http://127.0.0.1:1422/rpc/*", async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    calls.push(command);
    if (command === "save_profile") {
      profile = route.request().postDataJSON().profile;
      await route.fulfill({ json: { result: profile } });
    } else if (command === "connect") {
      await route.fulfill({
        json: {
          result: {
            id: "approved-sso",
            connectionId: profile.id,
            engine: "snowflake",
            serverVersion: "fixture",
            latencyMs: 1,
            encrypted: true,
            capabilities: {
              sql: true,
              transactions: false,
              cancellation: true,
              tableBrowsing: true,
              tableEditing: false,
              processes: false,
            },
          },
        },
      });
    } else if (command === "catalog") {
      if (++catalogCalls === 1) {
        await discovery;
        await route.fulfill({
          json: {
            error: {
              code: "permissionDenied",
              message: "Catalog unavailable for this role",
            },
          },
        });
      } else
        await route.fulfill({
          json: {
            result: [
              {
                table: { schema: "PUBLIC", name: "ORDERS" },
                kind: "BASE TABLE",
                estimatedRows: 0,
                totalBytes: 0,
              },
            ],
          },
        });
    } else if (command === "health") await route.fulfill({ json: { result: 1 } });
    else await route.continue();
  });
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Database engine").selectOption("snowflake");
  await page.getByLabel("Connection group").selectOption("production");
  await page.getByLabel("Connection name", { exact: true }).fill("Approved SSO fixture");
  await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("ANALYTICS");
  await page.getByLabel("Username", { exact: true }).fill("ANALYST");
  await page.getByLabel("Role", { exact: true }).fill("READ_ONLY");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH");
  await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText("Approved SSO fixture");
  finishDiscovery();
  await expect(page.getByRole("status")).toContainText("Could not load tables");
  await expect(page.getByRole("status")).toContainText("Catalog unavailable for this role");
  await page.getByRole("button", { name: "New query tab", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "SQL editor" })).toBeVisible();
  await page.getByRole("button", { name: "Refresh schema", exact: true }).click();
  await page.getByRole("button", { name: /^PUBLIC/ }).click();
  await expect(page.getByRole("button", { name: "ORDERS", exact: true })).toBeVisible();
  expect(calls.filter((command) => command === "connect")).toHaveLength(1);
  expect(calls).not.toContain("prepare_execution");
});
