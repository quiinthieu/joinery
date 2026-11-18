import { expect, test, type APIRequestContext, type Locator, type Page } from "./fixtures";

const PG_PORT = process.env.JOINERY_POSTGRES_PORT ?? "55432";
const BRIDGE = "http://127.0.0.1:1422";
const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };

async function rpc<T>(request: APIRequestContext, command: string, args = {}): Promise<T> {
  const response = await request.post(`${BRIDGE}/rpc/${command}`, { headers, data: args });
  expect(response.ok()).toBe(true);
  const payload = (await response.json()) as { result: T; error?: unknown };
  expect(payload.error).toBeUndefined();
  return payload.result;
}

async function cleanupProfilesByName(
  page: Page,
  request: APIRequestContext,
  names: readonly string[],
) {
  await page.goto("about:blank").catch(() => undefined);
  await rpc(request, "disconnect_fixtures").catch(() => undefined);
  await rpc(request, "clear_local_history", { tabKey: null }).catch(() => undefined);
  const bootstrap = await rpc<{ profiles: Array<{ id: string; name: string }> }>(
    request,
    "bootstrap",
  ).catch(() => null);
  if (!bootstrap) return;
  for (const profile of bootstrap.profiles) {
    if (names.includes(profile.name)) {
      await rpc(request, "delete_profile", { id: profile.id }).catch(() => undefined);
    }
  }
}

async function placeCaret(editor: Locator, marked: string) {
  const pos = marked.indexOf("|");
  expect(pos).toBeGreaterThanOrEqual(0);
  await editor.fill(marked.replace("|", ""));
  await editor.press("ControlOrMeta+Home");
  for (const _ of marked.slice(0, pos)) {
    await editor.press("ArrowRight");
  }
}

async function connectPostgresWithHistory(page: Page, name: string) {
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(PG_PORT);
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  const storeText = page.getByLabel(/Save SQL text in local history/i);
  if (!(await storeText.isChecked())) {
    await storeText.check();
  }
  const storeDrafts = page.getByLabel(/Restore open query drafts after restart/i);
  if (!(await storeDrafts.isChecked())) {
    await storeDrafts.check();
  }
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(name);
  await page.getByRole("button", { name: "New query tab" }).click();
}

test("exercises Transpose/Record views, detached result tabs, Workspace SQL, Data Search, Virtual FKs, and SQL Local History", async ({
  page,
  request,
}) => {
  const profileName = "DG 2026.3 Studio";
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));

  try {
    await connectPostgresWithHistory(page, profileName);

    // 1. Run a SQL query and inspect Transpose & Record views + detach result tab
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill(
      "SELECT 101 AS account_id, 'ada@example.test' AS owner_email UNION ALL SELECT 102, 'grace@example.test';",
    );
    await page.locator(".run-button").click();
    await expect(page.locator(".result-footer")).toContainText("2 rows");

    await page.getByRole("button", { name: "Transpose", exact: true }).click();
    await expect(page.getByTestId("transpose-grid-view")).toContainText("ada@example.test");

    await page.getByRole("button", { name: "Record", exact: true }).click();
    await expect(page.getByTestId("record-card-view")).toContainText("ada@example.test");
    await page.getByRole("button", { name: "Next record", exact: true }).click();
    await expect(page.getByTestId("record-card-view")).toContainText("grace@example.test");

    // 1b. Local Result Quick Filter + Pinned Console Result subtab across runs
    await page.getByLabel("Filter result rows", { exact: true }).fill("grace");
    await expect(page.locator(".result-quick-filter-count")).toContainText("1 / 2");
    await page.getByRole("button", { name: "Reset view", exact: true }).click();

    await page.getByRole("button", { name: "Pin result", exact: true }).click();
    await expect(page.getByRole("button", { name: "Unpin result", exact: true })).toBeVisible();
    await expect(page.getByRole("tab", { name: /Pinned #1/ })).toBeVisible();

    await page.getByRole("button", { name: "Open result in tab", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Open originating SQL", exact: true }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Open originating SQL", exact: true }).click();

    // 2. SQL Local History snapshot, diff, and restore
    await page.getByRole("button", { name: "Open SQL Local History", exact: true }).click();
    const historyDialog = page.getByRole("dialog");
    await historyDialog.getByRole("button", { name: "Save snapshot" }).click();
    await historyDialog.getByRole("button", { name: "Close", exact: true }).click();

    await editor.fill("SELECT 999 AS overwritten;");
    await page.getByRole("button", { name: "Open SQL Local History", exact: true }).click();
    await expect(page.getByTestId("local-history-diff")).toBeVisible();
    await page.getByRole("button", { name: "Restore to editor" }).click();
    await expect(editor).toContainText("ada@example.test");

    // 3. Cross-source Workspace SQL over retained result snapshot
    await page.getByRole("button", { name: "Workspace SQL", exact: true }).click();
    await expect(page.getByLabel("Workspace SQL query")).toContainText('FROM "');
    await page.getByRole("button", { name: "Run workspace SQL" }).click();
    await expect(page.locator(".workspace-sql-view")).toContainText("mounted rows");

    // 4. Full-text Data Search across disposable PostgreSQL catalog
    await page.getByRole("button", { name: "Data search", exact: true }).click();
    await page.getByLabel("Search database values").fill("odd_%");
    await page.getByRole("button", { name: "Search tables" }).click();
    await expect(page.locator(".data-search-view")).toContainText("browse_sort");

    // 5. Virtual Foreign Key registration & removal on `browse_sort` (no physical FK to `customers`)
    await page.locator('[data-tree-node="schema:public"]').click();
    await page
      .locator(".schema-tree")
      .getByRole("button", { name: "browse_sort", exact: true })
      .click();
    await page.getByRole("button", { name: "Structure", exact: true }).click();
    await expect(page.getByRole("button", { name: "Generate SELECT SQL" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Generate INSERT SQL" })).toBeVisible();
    await page.getByRole("button", { name: "Add virtual FK…" }).click();
    const vfkDialog = page.getByRole("dialog");
    await vfkDialog
      .getByLabel("Relationship name", { exact: true })
      .fill("vfk_browse_sort_customers");
    await vfkDialog.getByLabel("Referenced table", { exact: true }).fill("customers");
    await vfkDialog.getByLabel("Source column 1", { exact: true }).fill("id");
    await vfkDialog.getByLabel("Referenced column 1", { exact: true }).fill("id");
    await vfkDialog.getByRole("button", { name: "Save virtual FK", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    const fkTable = page.getByRole("table", { name: "Foreign keys" });
    await expect(fkTable).toContainText("vfk_browse_sort_customers");
    await fkTable
      .getByRole("button", { name: "Delete virtual foreign key vfk_browse_sort_customers" })
      .click();
    await expect(fkTable).toHaveCount(0);

    expect(errors).toEqual([]);
  } finally {
    await cleanupProfilesByName(page, request, [profileName]);
  }
});

test("connects embedded SQLite (:memory:), inspects EXPLAIN QUERY PLAN, and extracts a CTE", async ({
  page,
  request,
}) => {
  const profileName = "SQLite Memory Scratch";
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));

  try {
    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Connection name", { exact: true }).fill(profileName);
    await page.getByLabel("Database engine", { exact: true }).selectOption("sqlite");
    await page.getByLabel("Database file path (or :memory:)", { exact: true }).fill(":memory:");
    await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.locator(".status-bar")).toContainText(profileName);

    await page.getByRole("button", { name: "New query tab" }).click();
    await expect(page.locator(".completion-context")).not.toContainText("Loading");
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await placeCaret(editor, "SELECT sub.v FROM (SELECT |sqlite_version() AS v) AS sub;");
    const runExplain = page.waitForResponse((r) => r.url().endsWith("/rpc/execute_plan"));
    await page.getByRole("button", { name: "Explain", exact: true }).click();
    expect((await runExplain).ok()).toBe(true);
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await expect(page.locator(".run-button")).toBeVisible();
    await expect(page.locator(".completion-context")).not.toContainText("Loading");
    await page.getByRole("button", { name: "View plan", exact: true }).click();
    await expect(page.getByRole("region", { name: "Query execution plan" })).toContainText(
      "EXPLAIN QUERY PLAN",
    );

    await placeCaret(editor, "SELECT sub.v FROM (SELECT |sqlite_version() AS v) AS sub;");
    await page.getByRole("button", { name: "Extract SQL CTE", exact: true }).click();
    const cteDialog = page.getByRole("dialog", { name: "Extract SQL CTE" });
    await cteDialog.getByLabel("CTE name", { exact: true }).fill("ver_cte");
    await cteDialog.getByRole("button", { name: "Extract CTE", exact: true }).click();
    await expect(cteDialog).toHaveCount(0);
    await expect(editor).toContainText("WITH ver_cte AS");

    await placeCaret(
      editor,
      "WITH |ver_cte AS (\n  SELECT sqlite_version() AS v\n)\nSELECT sub.v FROM ver_cte AS sub;",
    );
    await page.getByRole("button", { name: "Inline SQL CTE", exact: true }).click();
    const inlineDialog = page.getByRole("dialog", { name: /Inline SQL CTE/i });
    await inlineDialog.getByRole("button", { name: "Inline CTE", exact: true }).click();
    await expect(inlineDialog).toHaveCount(0);
    await expect(editor).not.toContainText("WITH ver_cte AS");

    const runSelect = page.waitForResponse((r) => r.url().endsWith("/rpc/execute_plan"));
    await page.locator(".run-button").click();
    expect((await runSelect).ok()).toBe(true);
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await expect(page.locator(".result-footer")).toContainText("1 row");
    await expect(page.locator(".run-button")).toBeVisible();
    await expect(page.locator(".completion-context")).not.toContainText("Loading");
    await page.getByRole("button", { name: "Database output" }).click();
    await expect(page.getByRole("region", { name: "Database output" })).toContainText(
      "Execution started",
    );

    // SQLite live column inspection with typo quick-fix suggestion chip
    await editor.fill("CREATE TABLE demo_users (id INTEGER PRIMARY KEY, customer_email TEXT);");
    const runDdl = page.waitForResponse((r) => r.url().endsWith("/rpc/execute_plan"));
    await page.locator(".run-button").click();
    expect((await runDdl).ok()).toBe(true);
    await expect(page.locator(".result-footer")).toContainText("0 rows");
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await expect(page.locator(".run-button")).toBeVisible();
    await expect(page.locator(".completion-context")).not.toContainText("Loading");
    const refreshCatalog = page.waitForResponse((r) => r.url().endsWith("/rpc/completion_catalog"));
    await page.getByRole("button", { name: "Refresh completion context", exact: true }).click();
    expect((await refreshCatalog).ok()).toBe(true);
    await expect(page.locator(".completion-context")).not.toContainText("Loading");

    await placeCaret(editor, "SELECT u.|custmoer_email FROM demo_users u;");
    await page.getByRole("button", { name: "Inspect SQL columns", exact: true }).click();
    const inspectDialog = page.getByRole("dialog", { name: "SQL column inspection" });
    await expect(inspectDialog).toBeVisible();
    const quickFixGroup = inspectDialog.getByRole("group", { name: "Column typo quick-fixes" });
    await expect(quickFixGroup).toBeVisible();
    await quickFixGroup.getByRole("button", { name: "customer_email", exact: true }).click();
    await inspectDialog
      .getByRole("button", { name: "Apply column correction", exact: true })
      .click();
    await expect(inspectDialog).toHaveCount(0);
    await expect(editor).toContainText('"customer_email"');

    expect(errors).toEqual([]);
  } finally {
    await cleanupProfilesByName(page, request, [profileName]);
  }
});
