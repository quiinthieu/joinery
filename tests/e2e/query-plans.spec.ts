import { expect, test, type Page } from "@playwright/test";

async function connectForAnalysis(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Connection name", { exact: true }).fill("PostgreSQL analysis");
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(process.env.JOINERY_POSTGRES_PORT ?? "55432");
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
}

for (const [label, engine, port] of [
  ["PostgreSQL", "postgres", Number(process.env.JOINERY_POSTGRES_PORT ?? "55432")],
  ["MySQL", "mysql", 53306],
  ["MariaDB", "mysql", 53307],
] as const) {
  test(`${label} estimated plan uses the query session and switches between tree, raw and grid`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption(engine);
    await page.getByLabel("Connection name", { exact: true }).fill(`${label} query plan`);
    await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
    await page.getByLabel("Port", { exact: true }).fill(String(port));
    await page.getByLabel("Database", { exact: true }).fill("joinery");
    await page.getByLabel("Username", { exact: true }).fill("joinery");
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByLabel("Require TLS and verify the server certificate").uncheck();
    await page.getByLabel("Safe Mode", { exact: true }).selectOption("readOnly");
    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: "New query tab" }).click();
    // Explain must use this tab's isolated session, not the grid/login session.
    await page.getByRole("button", { name: "New isolated session", exact: true }).click();
    await page
      .getByRole("dialog")
      .getByLabel("Password", { exact: true })
      .fill("joinery-local-only");
    await page.getByRole("dialog").getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.locator(".query-session")).toContainText("Isolated session");
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const sql = `SELECT * FROM ${engine === "postgres" ? "public.orders" : "joinery.sample"} LIMIT 1`;
    await editor.fill(sql);
    await page.getByRole("button", { name: "Explain", exact: true }).click();
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await expect(page.locator(".query-error")).toHaveCount(0);
    await expect(editor).toHaveText(sql);
    await page.getByRole("button", { name: "View plan", exact: true }).click();
    const plan = page.getByRole("region", { name: "Query execution plan" });
    await expect(plan).toContainText("Estimated query plan");
    await expect(plan.locator(".plan-tree")).toContainText("Estimated rows");
    await expect(plan).not.toContainText("Actual rows");
    await page.getByRole("button", { name: "Raw JSON", exact: true }).click();
    const raw = await plan.locator(".plan-raw").innerText();
    const parsed = JSON.parse(raw);
    expect(engine === "postgres" ? parsed[0].Plan : parsed.query_block).toBeTruthy();
    await page.getByRole("button", { name: "Tree", exact: true }).click();
    await expect(plan.locator(".plan-tree")).toBeVisible();
    await page.getByRole("button", { name: "Data grid", exact: true }).click();
    await expect(page.locator(".grid-host")).toBeVisible();

    // Explicitly select the full batch: no suffix may run outside EXPLAIN.
    await editor.fill("SELECT 1; SELECT 2");
    await editor.press("ControlOrMeta+a");
    await page.getByRole("button", { name: "Explain", exact: true }).click();
    await expect(page.locator(".query-error")).toContainText("one SELECT or WITH query");
    await expect(page.locator(".result-tabs [role=tab]")).toHaveCount(1);
    if (engine === "postgres") {
      await editor.fill("SELECT 1");
      await page.getByRole("button", { name: "Analyze", exact: true }).click();
      await expect(page.locator(".query-error")).toContainText("read-only");
      await expect(page.getByRole("dialog")).toHaveCount(0);
    } else {
      await expect(page.getByRole("button", { name: "Analyze", exact: true })).toHaveCount(0);
    }
    expect(errors).toEqual([]);
  });
}

test("PostgreSQL actual plan requires review on Silent, preserves SQL, and shows actual metrics", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  let executions = 0;
  page.on("request", (request) => {
    if (request.url().endsWith("/rpc/execute_plan")) executions++;
  });
  await connectForAnalysis(page);
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  const sql = "SELECT * FROM public.orders LIMIT 1";
  await editor.fill(sql);
  await page.getByRole("button", { name: "Analyze", exact: true }).click();
  const review = page.getByRole("dialog", { name: "Review this operation" });
  await expect(review).toContainText("EXPLAIN ANALYZE executes the query");
  await expect(review.locator(".sql-preview")).toHaveText(`EXPLAIN (ANALYZE, FORMAT JSON)\n${sql}`);
  expect(executions).toBe(0);
  await review.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(review).toHaveCount(0);
  await expect(page.locator(".result-tabs [role=tab]")).toHaveCount(0);
  expect(executions).toBe(0);

  await page.getByRole("button", { name: "Analyze", exact: true }).click();
  await review.getByRole("button", { name: "Authorize and run", exact: true }).click();
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  expect(executions).toBe(1);
  await expect(editor).toHaveText(sql);
  await page.getByRole("button", { name: "View plan", exact: true }).click();
  const plan = page.getByRole("region", { name: "Query execution plan" });
  await expect(plan).toContainText("Plan with actual measurements");
  await expect(plan).toContainText("Estimated rows");
  await expect(plan).toContainText("Actual rows per loop");
  await expect(plan).toContainText("Actual loops");
  await expect(plan).toContainText("Server execution");
  await page.getByRole("button", { name: "Raw JSON", exact: true }).click();
  const raw = JSON.parse(await plan.locator(".plan-raw").innerText());
  expect(raw[0].Plan["Actual Rows"]).toBe(1);
  expect(raw[0].Plan["Actual Loops"]).toBe(1);
  expect(raw[0]["Execution Time"]).toBeGreaterThanOrEqual(0);

  // Entering the SQL directly must hit the same mandatory review gate.
  await editor.fill("EXPLAIN (ANALYZE, FORMAT JSON) SELECT 1");
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(review).toContainText("EXPLAIN ANALYZE executes the query");
  expect(executions).toBe(1);
  await review.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(errors).toEqual([]);
});

test("PostgreSQL actual plan recovers from an error and cancellation retires its session", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await connectForAnalysis(page);
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  const analyze = async (sql: string) => {
    await editor.fill(sql);
    await page.getByRole("button", { name: "Analyze", exact: true }).click();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Authorize and run", exact: true })
      .click();
  };
  await analyze("SELECT 1 / 0");
  await expect(page.locator(".query-error")).toContainText("division by zero");
  await analyze("SELECT 1");
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await expect(page.locator(".query-error")).toHaveCount(0);
  await analyze("SELECT pg_sleep(10)");
  await expect(page.locator(".result-footer .status-dot.running")).toBeVisible();
  await page.getByRole("button", { name: /Cancel/ }).click();
  await expect(page.locator(".result-footer .status-dot.cancelled")).toBeVisible();
  await expect(page.locator(".query-session")).toContainText(/disconnected/i);
  await expect(page.getByRole("button", { name: "Analyze", exact: true })).toBeDisabled();
  expect(errors).toEqual([]);
});
