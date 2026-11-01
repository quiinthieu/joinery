import { expect, test, type Page } from "@playwright/test";
import { Table, Utf8, vectorFromArray, tableToIPC } from "apache-arrow";
import type {
  ConnectionProfile,
  PreparedPlan,
  QueryEvent,
  ResultInfo,
} from "../../src/ipc/generated";

// Synthetic IPC/Arrow boundary only. The Rust loopback transport test verifies
// the real SQL request separately; this suite never opens SSO or an account.
async function fixture(page: Page, raw: string) {
  let profile: ConnectionProfile;
  let plan: PreparedPlan;
  let runs = 0;
  const executed: string[] = [];
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("http://127.0.0.1:1422/rpc/*", async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const args = route.request().postDataJSON();
    let result: unknown;
    switch (command) {
      case "save_profile":
        profile = args.profile;
        result = profile;
        break;
      case "connect":
        result = {
          id: "sf-plan-session",
          connectionId: profile.id,
          engine: "snowflake",
          serverVersion: "synthetic",
          latencyMs: 1,
          encrypted: true,
          capabilities: {
            sql: true,
            transactions: false,
            cancellation: true,
            tableBrowsing: true,
            tableEditing: false,
            tableFiltering: false,
            processes: false,
          },
        };
        break;
      case "catalog":
        result = [];
        break;
      case "completion_catalog":
        await route.fulfill({
          json: { error: { code: "unsupported", message: "Synthetic completion unavailable" } },
        });
        return;
      case "session_state":
        result = {
          sessionId: "sf-plan-session",
          transactionMode: "autoCommit",
          transactionState: "idle",
          contextKind: "schema",
          activeContext: "PUBLIC",
          database: "LAB",
          role: "READ_ONLY",
          searchPath: ["LAB.PUBLIC"],
        };
        break;
      case "prepare_explain":
      case "prepare_execution":
        expect(args.sessionId).toBe("sf-plan-session");
        if (command === "prepare_explain") expect(args.analyze ?? false).toBe(false);
        plan = {
          id: `sf-plan-${runs}`,
          sessionId: args.sessionId,
          statements: [
            {
              sql: command === "prepare_explain" ? `EXPLAIN USING JSON\n${args.sql}` : args.sql,
              kind: "read",
              missingWhere: false,
            },
          ],
          requiresApproval: false,
          requiresPassphrase: false,
          confirmationText: null,
          warnings: [],
          expiresAt: Date.now() + 60000,
        };
        result = plan;
        break;
      case "execute_plan":
        executed.push(plan.statements[0].sql);
        result = `sf-run-${++runs}`;
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
  await page.route("http://127.0.0.1:1422/events/*", async (route) => {
    const runId = `sf-run-${runs}`;
    const result: ResultInfo = {
      id: `sf-content-${runs}`,
      runId,
      statementIndex: 0,
      columns: [
        {
          id: "c0",
          name: "content",
          databaseType: "TEXT",
          kind: "text",
          nullable: false,
          primaryKey: false,
          foreignKey: false,
          editable: false,
          insertable: false,
          hasDefault: false,
        },
      ],
      retainedRows: 1,
      status: "complete",
      table: null,
    };
    const events: QueryEvent[] = [
      { type: "started", runId },
      { type: "statementStarted", runId, index: 0 },
      { type: "result", result },
      { type: "statementCompleted", runId, index: 0, affectedRows: "1", elapsedMs: 1 },
      { type: "finished", runId, status: "completed", elapsedMs: 2 },
    ];
    await route.fulfill({ json: events });
  });
  await page.route("http://127.0.0.1:1422/result/sf-content-*", async (route) => {
    const resultId = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const table = new Table({ c0: vectorFromArray([raw], new Utf8()) });
    table.schema.metadata.set("joinery.protocol", "1");
    table.schema.metadata.set("joinery.resultId", resultId);
    table.schema.metadata.set("joinery.offset", "0");
    await route.fulfill({
      contentType: "application/vnd.apache.arrow.stream",
      body: Buffer.from(tableToIPC(table, "stream")),
    });
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Database engine").selectOption("snowflake");
  await page.getByLabel("Connection name", { exact: true }).fill("Snowflake plan fixture");
  await page.getByLabel("Snowflake account hostname").fill("org-account.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("LAB");
  await page.getByLabel("Username", { exact: true }).fill("FIXTURE");
  await page.getByLabel("Role", { exact: true }).fill("READ_ONLY");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH");
  await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  return { executed, errors };
}

const raw =
  '{"GlobalStats":{"partitionsTotal":4,"partitionsAssigned":1,"bytesAssigned":9007199254740993123},"Operations":[[{"id":0,"operation":"Result"},{"id":1,"operation":"Filter","parentOperators":[0]},{"id":2,"operation":"Aggregate","parentOperators":[0]},{"id":3,"operation":"TableScan","parentOperators":[1,2],"objects":["LAB.PUBLIC.EVENTS"],"partitionsAssigned":1,"bytesAssigned":4096}],[{"id":0,"operation":"Result"}]],"future":{"retain":true}}';

test("Snowflake Explain shows logical DAG steps, exact raw JSON and unavailable comparison", async ({
  page,
}) => {
  const state = await fixture(page, raw);
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  const sql = "SELECT * FROM LAB.PUBLIC.EVENTS LIMIT 1";
  await editor.fill(sql);
  await page.getByRole("button", { name: "Explain", exact: true }).click();
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await expect(editor).toHaveText(sql);
  await expect(page.getByRole("button", { name: "Analyze", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "View plan", exact: true }).click();
  const plan = page.getByRole("region", { name: "Query execution plan", exact: true });
  await expect(plan).toContainText("Snowflake logical plan");
  await expect(plan).toContainText("upper-bound estimates");
  await expect(plan).toContainText("Step 1");
  await expect(plan).toContainText("Step 2");
  // Expand collapsed logical descendants to inspect the shared reference.
  await plan
    .locator("details")
    .evaluateAll((nodes) => nodes.forEach((node) => ((node as HTMLDetailsElement).open = true)));
  await expect(plan).toContainText("Shared operator 3");
  await expect(plan).toContainText("outside exact integer range");
  await expect(plan.getByRole("button", { name: "Keep snapshot", exact: true })).toBeDisabled();
  await expect(plan).toContainText("observed warehouse and session context");
  await page.getByRole("button", { name: "Raw JSON", exact: true }).click();
  await expect(plan.locator(".plan-raw")).toHaveText(raw);
  await page.getByRole("button", { name: "Tree", exact: true }).click();
  await expect(plan.locator(".plan-tree")).toBeVisible();
  await page.screenshot({ path: ".cache/screenshots/snowflake-plan-browser.png", fullPage: true });
  expect(state.executed).toEqual([`EXPLAIN USING JSON\n${sql}`]);
  await page.getByRole("button", { name: "Data grid", exact: true }).click();
  await expect(page.locator(".grid-host")).toBeVisible();
  // A subsequent ordinary CONTENT result must not inherit the previous identity.
  await editor.fill("SELECT 'content' AS content");
  await page.locator(".run-button").click();
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await expect(page.getByRole("button", { name: "View plan", exact: true })).toHaveCount(0);
  expect(state.errors).toEqual([]);
});

test("unsupported Snowflake graph remains available in raw view and grid", async ({ page }) => {
  const malformed =
    '{"GlobalStats":{},"Operations":[[{"id":0,"operation":"Result","parentOperators":[99]}]]}';
  const state = await fixture(page, malformed);
  await page.getByRole("textbox", { name: "SQL editor" }).fill("SELECT 1");
  await page.getByRole("button", { name: "Explain", exact: true }).click();
  await page.getByRole("button", { name: "View plan", exact: true }).click();
  const plan = page.getByRole("region", { name: "Query execution plan", exact: true });
  await expect(plan.getByRole("alert")).toContainText("raw result is still available");
  await page.getByRole("button", { name: "Raw JSON", exact: true }).click();
  await expect(plan.locator(".plan-raw")).toHaveText(malformed);
  await page.getByRole("button", { name: "Data grid", exact: true }).click();
  await expect(page.locator(".grid-host")).toBeVisible();
  expect(state.executed).toEqual(["EXPLAIN USING JSON\nSELECT 1"]);
  expect(state.errors).toEqual([]);
});
