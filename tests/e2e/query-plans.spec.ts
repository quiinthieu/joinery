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
    await page.getByText("Execution timing", { exact: true }).click();
    const timing = page.locator(".execution-timing");
    for (const label of [
      "Queue wait",
      "Backend active",
      "Backend total",
      "First row batch",
      "Client elapsed",
    ])
      await expect(
        timing
          .locator("dl > div")
          .filter({ has: page.getByText(label, { exact: true }) })
          .locator("dd"),
      ).toHaveText(/^\d+ ms$/);
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
    await page.getByRole("button", { name: "Keep snapshot", exact: true }).click();
    await expect(plan).toContainText("1/10 snapshots");
    await page.getByRole("button", { name: "Explain", exact: true }).click();
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await page.getByRole("button", { name: "View plan", exact: true }).click();
    const baseline = page.getByLabel("Compare with snapshot");
    const snapshotId = await baseline.locator("option").nth(1).getAttribute("value");
    await baseline.selectOption(snapshotId!);
    await expect(page.getByRole("region", { name: "Plan comparison", exact: true })).toBeVisible();
    await expect(plan.locator(".plan-tree")).toHaveCount(2);
    await page.getByRole("button", { name: "Raw JSON", exact: true }).click();
    await expect(plan.locator(".plan-raw")).toHaveCount(2);
    await page.getByRole("button", { name: "Remove selected snapshot", exact: true }).click();
    await expect(plan).toContainText("0/10 snapshots");
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

test("PostgreSQL snapshots compare estimates with actuals, reject mismatches and clear on disconnect", async ({
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
  const explain = async (query: string) => {
    await editor.fill(query);
    await page.getByRole("button", { name: "Explain", exact: true }).click();
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await page.getByRole("button", { name: "View plan", exact: true }).click();
  };
  await explain(sql);
  // Capturing after editing must retain the executed SQL, not editor text.
  await editor.fill("SELECT 2");
  await page.getByRole("button", { name: "Keep snapshot", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Query execution plan" }).getByRole("status"),
  ).toContainText("Kept snapshot");
  expect(executions).toBe(1);
  await editor.fill(sql);
  await page.getByRole("button", { name: "Analyze", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Authorize and run", exact: true })
    .click();
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await page.getByRole("button", { name: "View plan", exact: true }).click();
  const baseline = page.getByLabel("Compare with snapshot");
  await baseline.selectOption((await baseline.locator("option").nth(1).getAttribute("value"))!);
  await expect(page.getByRole("region", { name: "Snapshot plan", exact: true })).toContainText(
    "Estimated",
  );
  await expect(page.getByRole("region", { name: "Current plan", exact: true })).toContainText(
    "Actual rows per loop",
  );
  await page.getByRole("button", { name: "Raw JSON", exact: true }).click();
  const kept = JSON.parse(
    await page
      .getByRole("region", { name: "Snapshot plan", exact: true })
      .locator(".plan-raw")
      .innerText(),
  );
  expect(kept[0].Plan["Actual Rows"]).toBeUndefined();
  expect(executions).toBe(2);
  await page.getByRole("button", { name: "Tree", exact: true }).click();
  expect(executions).toBe(2);

  await explain("SELECT 2");
  await expect(baseline.locator("option").nth(1)).toHaveJSProperty("disabled", true);
  await expect(baseline.locator("option").nth(1)).toContainText("Different query text");
  await expect(page.getByRole("region", { name: "Plan comparison", exact: true })).toHaveCount(0);

  // Session cancellation must remove prior captures and prevent recapture.
  await editor.fill("SELECT pg_sleep(10)");
  await page.getByRole("button", { name: "Analyze", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Authorize and run", exact: true })
    .click();
  await expect(page.locator(".result-footer .status-dot.running")).toBeVisible();
  await page.getByRole("button", { name: /Cancel/ }).click();
  await expect(page.locator(".query-session")).toContainText(/disconnected/i);
  await page.getByRole("button", { name: "PostgreSQL analysis", exact: true }).click();
  await page.getByRole("dialog").getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByRole("dialog").getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await explain(sql);
  await expect(page.getByRole("region", { name: "Query execution plan" })).toContainText(
    "0/10 snapshots",
  );
  expect(errors).toEqual([]);
});

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

test("execution timing distinguishes empty/error results and keeps first batch through a later error", async ({
  page,
}) => {
  await connectForAnalysis(page);
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  const run = async (sql: string, status: string) => {
    await editor.fill(sql);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(`.result-footer .status-dot.${status}`)).toBeVisible();
  };
  await run("SELECT 1 WHERE false", "completed");
  await page.getByText("Execution timing", { exact: true }).click();
  const timing = page.locator(".execution-timing");
  const value = (label: string) =>
    timing
      .locator("dl > div")
      .filter({ has: page.getByText(label, { exact: true }) })
      .locator("dd");
  await expect(value("First row batch")).toHaveText("Unavailable");
  await expect(value("Backend total")).toHaveText(/^\d+ ms$/);
  await run("SELECT 1 / 0", "failed");
  await expect(value("First row batch")).toHaveText("Unavailable");
  await expect(value("Client elapsed")).toHaveText(/^\d+ ms$/);
  await run("SELECT 1 WHERE false; SELECT 42; SELECT 1 / 0", "failed");
  await expect(value("First row batch")).toHaveText(/^\d+ ms$/);
  await expect(page.locator(".query-error")).toContainText("division by zero");
});

test("a missing backend context observation withholds snapshot capture", async ({ page }) => {
  await connectForAnalysis(page);
  await page.route("**/events/*", async (route) => {
    const response = await route.fetch();
    const events = await response.json();
    await route.fulfill({
      response,
      json: events.map((event: { type: string }) =>
        event.type === "planContext" ? { ...event, state: null } : event,
      ),
    });
  });
  await page.getByRole("textbox", { name: "SQL editor" }).fill("SELECT 42");
  await page.getByRole("button", { name: "Explain", exact: true }).click();
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await page.getByRole("button", { name: "View plan", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Query execution plan" }).locator(".plan-tree"),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Keep snapshot", exact: true })).toBeDisabled();
});
