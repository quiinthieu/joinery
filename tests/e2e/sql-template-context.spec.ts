import { expect, test, type Page, type Route } from "@playwright/test";
import { tableFromIPC } from "apache-arrow";

async function connect(page: Page, engine: "PostgreSQL" | "MySQL" | "MariaDB") {
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(`${engine} context templates`);
  await page
    .getByLabel("Database engine")
    .selectOption(engine === "PostgreSQL" ? "postgres" : "mysql");
  for (const [label, value] of [
    ["Host", "127.0.0.1"],
    [
      "Port",
      engine === "PostgreSQL"
        ? (process.env.JOINERY_POSTGRES_PORT ?? "55432")
        : engine === "MySQL"
          ? "53306"
          : "53307",
    ],
    ["Database", "joinery"],
    ["Username", "joinery"],
    ["Password", "joinery-local-only"],
  ])
    await page.getByLabel(label, { exact: true }).fill(value);
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  await expect(page.locator(".completion-context")).toContainText("joinery");
}

async function saveTemplate(page: Page, trigger: string, body: string, engine = "") {
  await page.getByRole("button", { name: /^Preferences/ }).click();
  const prefs = page.getByRole("dialog", { name: "Preferences" });
  await prefs.getByLabel("Snippet trigger").fill(trigger);
  await prefs.getByLabel("Snippet title").fill(`${trigger} test`);
  await prefs.getByLabel("Snippet engine").selectOption(engine);
  await prefs.getByLabel("Snippet template").fill(body);
  await prefs.getByRole("button", { name: "Add snippet" }).click();
  await expect(prefs.getByRole("list", { name: "Custom SQL snippets" })).toContainText(trigger);
  await prefs.getByRole("button", { name: "Done", exact: true }).click();
}

async function settleConsole(page: Page) {
  const context = page.locator(".completion-context");
  await expect(page.getByRole("button", { name: "Run all statements", exact: true })).toBeEnabled();
  await expect(context).not.toContainText("Loading");
  // A finished query can refresh console context one more time after the button re-enables.
  const late = await page
    .waitForResponse((response) => response.url().endsWith("/rpc/completion_catalog"), {
      timeout: 1000,
    })
    .then(() => true)
    .catch(() => false);
  if (late) await expect(context).not.toContainText("Loading");
}

async function openChooser(page: Page, trigger: string) {
  await page.getByRole("button", { name: "Insert SQL template", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Insert SQL template" });
  await dialog.getByLabel("SQL template", { exact: true }).selectOption(trigger);
  return dialog;
}

async function choose(page: Page, trigger: string) {
  await settleConsole(page);
  return openChooser(page, trigger);
}

async function fulfillWhenReleased(route: Route, response: Awaited<ReturnType<Route["fetch"]>>) {
  try {
    await route.fulfill({ response });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    // Refresh cancels the delayed context read before this handler resumes.
    if (!/already handled|closed|disposed/i.test(message)) throw error;
  }
}

for (const engine of ["PostgreSQL", "MySQL", "MariaDB"] as const) {
  test(`${engine} context templates stay on primary and isolated consoles and wrap SQL literally`, async ({
    page,
  }) => {
    const errors: string[] = [];
    let executions = 0;
    const contexts: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("request", (request) => {
      if (/\/rpc\/(prepare_execution|execute_plan)$/.test(request.url())) executions++;
      if (request.url().endsWith("/rpc/session_contexts")) contexts.push(request.postData()!);
    });
    await connect(page, engine);
    const postgres = engine === "PostgreSQL";
    const quote = postgres ? '"' : "`";
    const schema = postgres ? "public" : "joinery";
    const table = postgres ? "orders" : "sample";
    const editor = page.locator('.cm-content[aria-label="SQL editor"]');
    const primary = (await page.getByRole("tab", { selected: true }).getAttribute("id"))!;
    await saveTemplate(page, "context_probe", "{{unknown}}");
    await saveTemplate(
      page,
      "context_probe",
      `SELECT \${1:1} AS \${2:answer} FROM {{schema}}.${table} LIMIT 1; -- {{database}}\${0}`,
      postgres ? "postgres" : "mysql",
    );
    const expected = (name: string, database: string) =>
      `SELECT 1 AS answer FROM ${quote}${name}${quote}.${table} LIMIT 1; -- ${quote}${database}${quote}`;
    const expand = async () => {
      await settleConsole(page);
      await expect(page.locator(".completion-context")).toContainText("joinery");
      await editor.fill("context_probe");
      await editor.press("End");
      await editor.press("Control+Space");
      await page
        .locator(".cm-tooltip-autocomplete li", { hasText: "context_probe" })
        .first()
        .click();
    };
    const run = async () => {
      const result = page.waitForResponse(
        (response) => response.ok() && /\/result\/[^?]+\?start=0&/.test(response.url()),
      );
      await page.getByRole("button", { name: "Run all statements", exact: true }).click();
      await expect(page.locator(".result-footer")).toContainText("1 rows");
      await expect(page.locator(".query-error")).toHaveCount(0);
      const values = tableFromIPC(new Uint8Array(await (await result).body()));
      await expect(
        page.getByRole("button", { name: "Run all statements", exact: true }),
      ).toBeEnabled();
      await expect(page.locator(".completion-context")).not.toContainText("Loading");
      return String(values.getChildAt(0)!.get(0));
    };
    await editor.fill(`SELECT 1 AS answer FROM ${schema}.${table} LIMIT 1`);
    const baseline = await run();
    const beforePrimary = executions;
    await expand();
    await expect(editor).toHaveText(expected(schema, "joinery"));
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("1");
    await editor.press("Tab");
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("answer");
    await editor.press("Tab");
    expect(executions).toBe(beforePrimary);
    expect(await run()).toBe(baseline);
    await editor.press("ControlOrMeta+z");
    await expect(editor).toHaveText("context_probe");

    const selected = "SELECT 7 AS answer /* ${9:literal} {{database}} */";
    await editor.fill(selected);
    await editor.press("ControlOrMeta+a");
    const wrap = await choose(page, "wrap");
    await wrap.getByRole("button", { name: "Insert template", exact: true }).click();
    await expect(wrap).toHaveCount(0);
    await expect(editor).toHaveText(`SELECT *FROM (${selected}) selected_rows;`);
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("*");
    await editor.press("Tab");
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("selected_rows");
    await editor.press("Tab");
    expect(executions).toBe(beforePrimary + 2);
    const beforeWrapRead = executions;
    expect(await run()).toBe("7");
    expect(executions).toBe(beforeWrapRead + 2);
    await editor.press("ControlOrMeta+z");
    await expect(editor).toHaveText(selected);

    await page.getByRole("button", { name: "New query tab" }).click();
    await page.getByRole("button", { name: "New isolated session", exact: true }).click();
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.locator(".query-session")).toContainText("Isolated session");
    if (!postgres) {
      await editor.fill("SET ROLE completion_reader; USE completion_alt");
      await page.getByRole("button", { name: "Run all statements", exact: true }).click();
      await expect(page.locator(".completion-context")).toContainText("completion_reader");
    } else {
      await page
        .getByRole("combobox", { name: "Active schema", exact: true })
        .selectOption("completion_alt");
    }
    await expect(page.locator(".completion-context")).toContainText("completion_alt");
    await editor.fill("");
    const beforeIsolated = executions;
    const dialog = await choose(page, "context_probe");
    await dialog.getByRole("button", { name: "Insert template", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(editor).toHaveText(
      expected("completion_alt", postgres ? "joinery" : "completion_alt"),
    );
    expect(executions).toBe(beforeIsolated);
    expect(await run()).toBe(baseline);
    await page.locator(`#${primary}`).click();
    await expand();
    await expect(editor).toHaveText(expected(schema, "joinery"));
    expect(new Set(contexts).size).toBeGreaterThanOrEqual(2);
    expect(errors).toEqual([]);
  });

  test(`${engine} template failures and stale context responses leave SQL untouched`, async ({
    page,
  }) => {
    let executions = 0;
    page.on("request", (request) => {
      if (/\/rpc\/(prepare_execution|execute_plan)$/.test(request.url())) executions++;
    });
    await connect(page, engine);
    const editor = page.locator('.cm-content[aria-label="SQL editor"]');
    await saveTemplate(page, "unknown_probe", "{{unknown}}");
    await editor.fill("SELECT 9 AS untouched");
    await editor.press("End");
    let dialog = await choose(page, "unknown_probe");
    await dialog.getByRole("button", { name: "Insert template", exact: true }).click();
    await expect(dialog).toContainText("Unsupported template variable");
    await expect(editor).toHaveText("SELECT 9 AS untouched");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    dialog = await choose(page, "wrap");
    await dialog.getByRole("button", { name: "Insert template", exact: true }).click();
    await expect(dialog).toContainText("Select SQL");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await settleConsole(page);
    await page.route("**/rpc/session_contexts", async (route) => {
      const response = await route.fetch();
      const body = await response.json();
      await route.fulfill({
        response,
        json: {
          ...body,
          result: { ...body.result, state: { ...body.result.state, activeContext: null } },
        },
      });
    });
    dialog = await openChooser(page, "ctxsel");
    await dialog.getByRole("button", { name: "Insert template", exact: true }).click();
    await expect(dialog).toContainText("No observed current schema");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await page.unroute("**/rpc/session_contexts");
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    await settleConsole(page);
    await page.route("**/rpc/session_contexts", async (route) => {
      const response = await route.fetch();
      await gate;
      await fulfillWhenReleased(route, response);
    });
    dialog = await openChooser(page, "ctxsel");
    const requested = page.waitForRequest("**/rpc/session_contexts");
    await dialog.getByRole("button", { name: "Insert template", exact: true }).click();
    await requested;
    await page.evaluate(() =>
      (
        document.querySelector('[aria-label="Refresh completion context"]') as HTMLButtonElement
      ).click(),
    );
    await expect(dialog).toHaveCount(0);
    const response = page.waitForResponse("**/rpc/session_contexts");
    release();
    await response;
    await expect(page.locator(".completion-context")).not.toContainText("Loading");
    await expect(editor).toHaveText("SELECT 9 AS untouched");
    await page.unroute("**/rpc/session_contexts");
    let cancelRelease!: () => void;
    const cancelGate = new Promise<void>((resolve) => {
      cancelRelease = resolve;
    });
    await settleConsole(page);
    await page.route("**/rpc/session_contexts", async (route) => {
      const result = await route.fetch();
      await cancelGate;
      await fulfillWhenReleased(route, result);
    });
    dialog = await openChooser(page, "ctxsel");
    const cancelRequest = page.waitForRequest("**/rpc/session_contexts");
    await dialog.getByRole("button", { name: "Insert template", exact: true }).click();
    await cancelRequest;
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    const cancelResponse = page.waitForResponse("**/rpc/session_contexts");
    cancelRelease();
    await cancelResponse;
    await expect(editor).toHaveText("SELECT 9 AS untouched");
    expect(executions).toBe(0);
  });
}
