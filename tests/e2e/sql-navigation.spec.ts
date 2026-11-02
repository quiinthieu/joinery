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
    const docs = page.getByRole("region", { name: "Quick SQL documentation" });
    const usages = page.getByRole("region", { name: "SQL symbol usages" });
    // Start with an unloaded source schema: navigation must follow the body to fetch it.
    for (const marked of [
      `WITH recent AS (SELECT s.id AS order_key FROM ${table} s) SELECT r.|order_key FROM recent r WHERE r.order_key > 0;`,
      `WITH recent(order_key) AS (SELECT s.id FROM ${table} s) SELECT r.|order_key FROM recent r WHERE r.order_key > 0;`,
      `SELECT d.|order_key FROM (SELECT s.id AS order_key FROM ${table} s) d WHERE d.order_key > 0;`,
    ]) {
      await placeCaret(editor, marked);
      await editor.press("F1");
      await expect(docs).toContainText("OUTPUT COLUMN");
      await expect(docs).toContainText(`${table}.id`);
      await expect(docs).toContainText("Source type");
      await page.keyboard.press("Escape");
      await editor.press("Shift+F12");
      await expect(usages.locator(".sql-nav-loc")).toHaveCount(3);
      await page.keyboard.press("Escape");
      await editor.press("F12");
      await expect
        .poll(() => page.evaluate(() => window.getSelection()?.toString()))
        .toBe("order_key");
      await expect(page.locator('.workspace-tabs [role="tab"]')).toHaveCount(1);
    }
    await placeCaret(editor, `SELECT s.|id FROM ${table} s WHERE s.id > 0; -- 🧪 日本語`);
    await editor.press("F1");
    await expect(docs).toContainText(`${table}.id`);
    await expect(docs).toContainText("PRIMARY KEY");
    await expect(docs).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(editor).toBeFocused();

    await editor.press("Shift+F12");
    await expect(usages.locator(".sql-nav-loc")).toHaveCount(2);
    await usages
      .getByRole("button", { name: /Ln 1, Col/ })
      .last()
      .click();
    await expect.poll(() => page.evaluate(() => window.getSelection()?.toString())).toBe("id");
    await page.keyboard.press("Escape");
    await editor.press("F12");
    await expect(page.locator('tr[data-highlighted="true"]')).toContainText("id");
    await expect(page.getByRole("button", { name: "Structure", exact: true })).toBeVisible();
    await page.locator(`#${queryId}`).click();

    await placeCaret(editor, `WITH recent AS (SELECT id FROM ${table}) SELECT * FROM |recent;`);
    await editor.press("F12");
    await expect.poll(() => page.evaluate(() => window.getSelection()?.toString())).toBe("recent");
    await placeCaret(editor, `SELECT |s.id FROM ${table} AS s;`);
    await editor.press("F12");
    await expect.poll(() => page.evaluate(() => window.getSelection()?.toString())).toBe("s");

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
  const pendingSchemas: Promise<void>[] = [];
  let delayingSchemas = true;
  await page.route("**/rpc/completion_schema", async (route) => {
    if (!delayingSchemas) {
      await route.continue();
      return;
    }
    const finished = (async () => {
      const response = await route.fetch();
      received();
      await held;
      await route.fulfill({ response });
    })();
    pendingSchemas.push(finished);
    await finished;
  });
  await placeCaret(
    editor,
    "WITH recent AS (SELECT id AS order_key FROM public.orders) SELECT recent.|order_key FROM recent;",
  );
  await editor.press("F12");
  await saw;
  await editor.fill("SELECT 42;");
  delayingSchemas = false;
  release();
  await Promise.all(pendingSchemas);
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

test("isolated console Structure stays on its session without browsing primary data", async ({
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
  await expect(page.locator('tr[data-highlighted="true"]')).toContainText("id");
  await expect(page.locator('.workspace-tabs [role="tab"]')).toHaveCount(2);
  expect(reads).toEqual([]);
});

test("two isolated Structure inspectors remain separate from the primary console", async ({
  page,
}) => {
  const errors: string[] = [];
  const executions: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("request", (r) => {
    if (/\/rpc\/(browse_table|prepare_execution|execute_plan)$/.test(r.url()))
      executions.push(r.url());
  });
  await connect(page);
  const primaryTab = page.locator('.workspace-tabs [role="tab"][aria-selected="true"]');
  const primaryId = (await primaryTab.getAttribute("id"))!;
  const ids: string[] = [];
  const inspectorIds: string[] = [];
  for (let i = 0; i < 2; i++) {
    await page.getByRole("button", { name: "New query tab" }).click();
    await page.getByRole("button", { name: "New isolated session", exact: true }).click();
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.locator(".completion-context")).toContainText("joinery");
    const sourceId = (await page
      .locator('.workspace-tabs [role="tab"][aria-selected="true"]')
      .getAttribute("id"))!;
    const sessionId = await page.getByLabel("Console session", { exact: true }).inputValue();
    ids.push(sessionId);
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await placeCaret(editor, "SELECT o.|id FROM public.orders o;");
    const details = page.waitForRequest((r) => r.url().endsWith("/rpc/object_details"));
    await editor.press("F12");
    expect((await details).postDataJSON()).toMatchObject({ sessionId });
    await expect(page.locator('tr[data-highlighted="true"]')).toContainText("id");
    inspectorIds.push(
      (await page
        .locator('.workspace-tabs [role="tab"][aria-selected="true"]')
        .getAttribute("id"))!,
    );
    const follow = async (link: Locator, name: string, kind: string) => {
      const request = page.waitForRequest((r) => {
        if (!r.url().endsWith("/rpc/object_details")) return false;
        const object = r.postDataJSON().object;
        return object.name === name && object.kind === kind;
      });
      await link.click();
      expect((await request).postDataJSON()).toMatchObject({ sessionId, object: { name, kind } });
    };
    await follow(
      page
        .getByRole("table", { name: "Indexes" })
        .getByRole("button", { name: "orders_pkey", exact: true }),
      "orders_pkey",
      "index",
    );
    await expect(page.getByLabel("Index properties")).toContainText("btree");
    await follow(
      page
        .locator(".object-summary-head")
        .getByRole("button", { name: "public.orders", exact: true }),
      "orders",
      "table",
    );
    await follow(
      page
        .getByRole("table", { name: "Triggers" })
        .getByRole("button", { name: "inspector_touch", exact: true }),
      "inspector_touch",
      "trigger",
    );
    await expect(page.getByLabel("Trigger properties")).toContainText("INSERT");
    await follow(
      page
        .getByLabel("Object dependencies")
        .getByRole("button", { name: /public\.inspector_touch_fn/ }),
      "inspector_touch_fn",
      "function",
    );
    await expect(page.getByLabel("Routine properties")).toContainText("plpgsql");
    await page.locator(`#${sourceId}`).click();
    await placeCaret(editor, "SELECT o.|id FROM public.orders o;");
    await editor.press("F12");
    await follow(
      page
        .getByRole("table", { name: "Constraints" })
        .getByRole("button", { name: "public.customers (id)", exact: true }),
      "customers",
      "table",
    );
    await expect(page.getByRole("table", { name: "Constraints" })).toContainText("customers_pkey");
    await page.locator(`#${sourceId}`).click();
    await placeCaret(editor, "SELECT o.|id FROM public.orders o;");
    await editor.press("F12");
    await page.getByRole("button", { name: "DDL", exact: true }).click();
    await expect(page.locator(".ddl-panel")).toContainText("CREATE TABLE");
    const ddl = await page.getByLabel("Object DDL").innerText();
    await page.getByRole("button", { name: "Append DDL to source console", exact: true }).click();
    await expect(page.locator(`#${sourceId}`)).toHaveAttribute("aria-selected", "true");
    await expect
      .poll(async () => (await editor.locator(".cm-line").allTextContents()).join("\n"))
      .toBe("SELECT o.id FROM public.orders o;\n\n" + ddl);
    await expect(page.getByLabel("Console session", { exact: true })).toHaveValue(sessionId);
  }
  expect(ids[0]).not.toBe(ids[1]);
  expect(inspectorIds[0]).not.toBe(inspectorIds[1]);
  await page.locator(`#${primaryId}`).click();
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  await expect(page.locator(".query-session")).toContainText("Shared session");
  await placeCaret(editor, "SELECT o.|id FROM public.orders o;");
  const details = page.waitForRequest((r) => r.url().endsWith("/rpc/object_details"));
  await editor.press("F12");
  expect(ids).not.toContain((await details).postDataJSON().sessionId);
  await expect(page.locator('tr[data-highlighted="true"]')).toContainText("id");
  await page.getByRole("button", { name: "DDL", exact: true }).click();
  const ddl = await page.getByLabel("Object DDL").innerText();
  await page.getByRole("button", { name: "Append DDL to source console", exact: true }).click();
  await expect(page.locator(`#${primaryId}`)).toHaveAttribute("aria-selected", "true");
  await expect
    .poll(async () => (await editor.locator(".cm-line").allTextContents()).join("\n"))
    .toBe("SELECT o.id FROM public.orders o;\n\n" + ddl);
  await expect(page.locator(".query-session")).toContainText("Shared session");
  await expect(page.locator('.workspace-tabs [role="tab"]')).toHaveCount(14);
  expect(executions).toEqual([]);
  expect(errors).toEqual([]);
});
