import { expect, test, type Locator, type Page } from "@playwright/test";

async function connect(
  page: Page,
  engine = "PostgreSQL",
  port = Number(process.env.JOINERY_POSTGRES_PORT ?? "55432"),
) {
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(`${engine} symbols`);
  await page
    .getByLabel("Database engine")
    .selectOption(engine === "PostgreSQL" ? "postgres" : "mysql");
  for (const [label, value] of [
    ["Host", "127.0.0.1"],
    ["Port", String(port)],
    ["Database", "joinery"],
    ["Username", "joinery"],
    ["Password", "joinery-local-only"],
  ]) {
    await page.getByLabel(label, { exact: true }).fill(value);
  }
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("readOnly");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  await expect(page.locator(".completion-context")).toContainText("joinery");
}

async function placeCaret(editor: Locator, marked: string) {
  const pos = marked.indexOf("|");
  expect(pos).toBeGreaterThanOrEqual(0);
  await editor.fill(marked.replace("|", ""));
  await editor.press("ControlOrMeta+Home");
  for (const _ of marked.slice(0, pos)) await editor.press("ArrowRight");
}

for (const [engine, port] of [
  ["PostgreSQL", Number(process.env.JOINERY_POSTGRES_PORT ?? "55432")],
  ["MySQL", 53306],
  ["MariaDB", 53307],
] as const) {
  test(`${engine} SQL docs, usages and definitions navigate without executing editor SQL`, async ({
    page,
  }) => {
    const errors: string[] = [];
    const executions: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("request", (r) => {
      if (/\/rpc\/(prepare_execution|execute_plan)$/.test(r.url())) executions.push(r.url());
    });
    await connect(page, engine, port);
    const queryId = (await page
      .locator('.workspace-tabs [role="tab"][aria-selected="true"]')
      .getAttribute("id"))!;
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const table = engine === "PostgreSQL" ? "public.orders" : "joinery.sample";
    await placeCaret(editor, `SELECT s.|id FROM ${table} s WHERE s.id > 0; -- 🧪 日本語`);
    await editor.press("F1");
    const docs = page.getByRole("region", { name: "Quick SQL documentation" });
    await expect(docs).toContainText(`${table}.id`);
    await expect(docs).toContainText("PRIMARY KEY");
    await expect(docs).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(editor).toBeFocused();

    await editor.press("Shift+F12");
    const usages = page.getByRole("region", { name: "SQL symbol usages" });
    await expect(usages.locator(".sql-nav-loc")).toHaveCount(2);
    await usages
      .getByRole("button", { name: /Ln 1, Col/ })
      .last()
      .click();
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("id");
    await page.keyboard.press("Escape");
    await editor.press("F12");
    await expect(page.locator('tr[data-highlighted="true"]')).toContainText("id");
    await expect(page.getByRole("button", { name: "Structure", exact: true })).toBeVisible();
    await page.locator(`#${queryId}`).click();

    await placeCaret(editor, `WITH recent AS (SELECT id FROM ${table}) SELECT * FROM |recent;`);
    await editor.press("F12");
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("recent");
    await placeCaret(editor, `SELECT |s.id FROM ${table} AS s;`);
    await editor.press("F12");
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("s");

    await placeCaret(editor, `SELECT * FROM |${table};`);
    await page.getByRole("button", { name: "Quick SQL documentation", exact: true }).click();
    await expect(docs).toContainText("Columns (");
    await editor.fill("SELECT 42;");
    await expect(docs).toHaveCount(0);
    expect(executions).toEqual([]);
    expect(errors).toEqual([]);
  });
}

test("PostgreSQL ignores stale schema/details replies and reports denied metadata", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await connect(page);
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  let release!: () => void;
  let received!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  const saw = new Promise<void>((resolve) => {
    received = resolve;
  });
  await page.route("**/rpc/completion_schema", async (route) => {
    const response = await route.fetch();
    received();
    await held;
    await route.fulfill({ response });
  });
  await placeCaret(editor, "SELECT o.|id FROM public.orders o;");
  await editor.press("F12");
  await saw;
  await editor.fill("SELECT 42;");
  release();
  await page.unrouteAll({ behavior: "wait" });
  await expect(page.locator('.workspace-tabs [role="tab"]')).toHaveCount(1);
  await expect(page.getByRole("region", { name: "Quick SQL documentation" })).toHaveCount(0);

  // A late schema reply from before Refresh must not populate the replacement cache.
  await page.getByRole("button", { name: "Refresh completion context", exact: true }).click();
  await expect(page.locator(".completion-context")).toContainText("joinery");
  let releaseRefresh!: () => void, receivedRefresh!: () => void;
  const heldRefresh = new Promise<void>((r) => {
    releaseRefresh = r;
  });
  const sawRefresh = new Promise<void>((r) => {
    receivedRefresh = r;
  });
  let beforeRefresh = true;
  await page.route("**/rpc/completion_schema", async (route) => {
    if (beforeRefresh) {
      const response = await route.fetch();
      const body = await response.json();
      for (const column of body.result.columns) column.databaseType = "STALE_METADATA";
      receivedRefresh();
      await heldRefresh;
      await route.fulfill({ response, json: body });
    } else {
      await route.continue();
    }
  });
  await placeCaret(editor, "SELECT o.|id FROM public.orders o;");
  await editor.press("F1");
  await sawRefresh;
  const refreshed = page.waitForResponse("**/rpc/completion_catalog");
  beforeRefresh = false;
  await page.getByRole("button", { name: "Refresh completion context", exact: true }).click();
  await refreshed;
  releaseRefresh();
  await page.unrouteAll({ behavior: "wait" });
  await expect(page.getByRole("region", { name: "Quick SQL documentation" })).toHaveCount(0);
  await editor.press("F1");
  await expect(page.getByRole("region", { name: "Quick SQL documentation" })).toContainText(
    "public.orders.id",
  );
  await expect(page.getByRole("region", { name: "Quick SQL documentation" })).not.toContainText(
    "STALE_METADATA",
  );
  await expect(page.getByRole("region", { name: "Quick SQL documentation" })).toContainText(
    "bigint",
  );
  await page.keyboard.press("Escape");

  let releaseDetails!: () => void, receivedDetails!: () => void;
  const heldDetails = new Promise<void>((r) => {
    releaseDetails = r;
  });
  const sawDetails = new Promise<void>((r) => {
    receivedDetails = r;
  });
  await page.route(
    "**/rpc/object_details",
    async (route) => {
      const response = await route.fetch();
      receivedDetails();
      await heldDetails;
      await route.fulfill({ response });
    },
    { times: 1 },
  );
  await placeCaret(editor, "SELECT * FROM public.|orders;");
  await editor.press("F1");
  await sawDetails;
  await expect(page.getByRole("region", { name: "Quick SQL documentation" })).toBeVisible();
  await page.keyboard.press("Escape");
  releaseDetails();
  await page.unrouteAll({ behavior: "wait" });
  await expect(page.getByRole("region", { name: "Quick SQL documentation" })).toHaveCount(0);

  await page.route("**/rpc/object_details", (route) =>
    route.fulfill({
      json: { error: { code: "database", message: "fixture metadata access denied" } },
    }),
  );
  await editor.press("F1");
  const docs = page.getByRole("region", { name: "Quick SQL documentation" });
  await expect(docs).toContainText("Catalog metadata unavailable: fixture metadata access denied");
  await docs.getByRole("button", { name: /Usages/ }).click();
  await expect(page.getByRole("region", { name: "SQL symbol usages" })).toContainText(
    "fixture metadata access denied",
  );
  expect(errors).toEqual([]);
});

test("isolated console docs stay on its session and do not open primary-session structure", async ({
  page,
}) => {
  await connect(page);
  await page.getByRole("button", { name: "New isolated session", exact: true }).click();
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".query-session")).toContainText("Isolated session");
  const reads: string[] = [];
  page.on("request", (request) => {
    if (/\/rpc\/(table_schema|browse_table)$/.test(request.url())) reads.push(request.url());
  });
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  await placeCaret(editor, "SELECT o.|id FROM public.orders o;");
  await editor.press("F1");
  const docs = page.getByRole("region", { name: "Quick SQL documentation" });
  await expect(docs).toContainText("public.orders.id");
  await expect(docs).toContainText("bigint");
  await page.keyboard.press("Escape");
  await editor.press("F12");
  await expect(
    page.getByRole("status").filter({
      hasText: "Object structure navigation is unavailable for this query session",
    }),
  ).toBeVisible();
  await expect(page.locator('.workspace-tabs [role="tab"]')).toHaveCount(1);
  expect(reads).toEqual([]);
});
