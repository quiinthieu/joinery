import { expect, test, type Page } from "@playwright/test";
import { tableFromIPC } from "apache-arrow";

async function connect(page: Page, engine: string, port: number) {
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(`${engine} inspections`);
  await page
    .getByLabel("Database engine")
    .selectOption(engine === "PostgreSQL" ? "postgres" : "mysql");
  for (const [label, value] of [
    ["Host", "127.0.0.1"],
    ["Port", String(port)],
    ["Database", "joinery"],
    ["Username", "joinery"],
    ["Password", "joinery-local-only"],
  ])
    await page.getByLabel(label, { exact: true }).fill(value);
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("readOnly");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  await expect(page.locator(".completion-context")).toContainText("joinery");
  await expect(page.locator(".completion-context")).not.toContainText("Loading");
}

for (const [engine, port] of [
  ["PostgreSQL", Number(process.env.JOINERY_POSTGRES_PORT ?? "55432")],
  ["MySQL", 53306],
  ["MariaDB", 53307],
] as const) {
  test(`${engine} inspects fresh columns, explicitly corrects one occurrence and rejects stale or denied results`, async ({
    page,
  }) => {
    const errors: string[] = [];
    let executions = 0,
      metadataReads = 0;
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("request", (request) => {
      if (/\/rpc\/(prepare_execution|execute_plan)$/.test(request.url())) executions++;
      if (request.url().endsWith("/rpc/inspection_schema")) metadataReads++;
    });
    await connect(page, engine, port);
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const table = engine === "PostgreSQL" ? "public.orders" : "joinery.sample";
    const quote = engine === "PostgreSQL" ? '"' : "`";
    const correct = `SELECT o.id FROM ${table} o ORDER BY o.id LIMIT 2`;
    const bad = correct.replace("o.id", "o.typo");
    const corrected = correct.replace("o.id", `o.${quote}id${quote}`);
    const run = async () => {
      const response = page.waitForResponse((r) => r.url().endsWith("/rpc/execute_plan"));
      const result = page.waitForResponse((r) => /\/result\/[^?]+\?start=0&/.test(r.url()));
      await editor.press("ControlOrMeta+Enter");
      expect((await response).ok()).toBe(true);
      await expect(page.locator(".result-footer")).toContainText("2 rows");
      await expect(page.locator(".run-button")).toBeVisible();
      await expect(page.locator(".completion-context")).not.toContainText("Loading");
      await expect(page.locator(".query-error")).toHaveCount(0);
      const values = tableFromIPC(new Uint8Array(await (await result).body()));
      return Array.from({ length: values.numRows }, (_, row) =>
        String(values.getChildAt(0)!.get(row)),
      );
    };
    await editor.fill(correct);
    const beforeRows = await run();
    expect(beforeRows).toEqual(
      engine === "PostgreSQL" ? ["1", "2"] : ["9007199254740992", "9007199254740993"],
    );
    await editor.fill(bad);
    const before = executions;
    const inspect = page.getByRole("button", { name: "Inspect SQL columns", exact: true });
    const dialog = page.getByRole("dialog", { name: "SQL column inspection" });
    await inspect.click();
    await expect(dialog).toContainText("1 missing column reference");
    await expect(dialog).toContainText("2 qualified column references checked");
    const apply = dialog.getByRole("button", { name: "Apply column correction" });
    await expect(apply).toBeDisabled();
    await dialog.getByLabel("Replace with catalog column").selectOption("id");
    await expect(dialog.getByLabel("Column correction preview")).toContainText(
      `typo → ${quote}id${quote}`,
    );
    await dialog.getByLabel("Replace with catalog column").press("ControlOrMeta+Enter");
    await expect(dialog).toBeVisible();
    await expect(page.locator('.cm-content[aria-label="SQL editor"]')).toHaveText(bad);
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(editor).toHaveText(bad);
    await inspect.click();
    await expect(dialog).toBeVisible();
    await dialog.getByLabel("Replace with catalog column").selectOption("id");
    await apply.click();
    await expect(editor).toHaveText(corrected);
    expect(executions).toBe(before);
    expect(metadataReads).toBe(2);
    expect(await run()).toEqual(beforeRows);
    await editor.press("ControlOrMeta+z");
    await expect(editor).toHaveText(bad);
    await inspect.click();
    await expect(dialog).toBeVisible();
    await page.evaluate(async () => {
      const { EditorView } = await import("/node_modules/@codemirror/view/dist/index.js");
      EditorView.findFromDOM(document.querySelector(".cm-editor")!)!.dispatch({
        changes: { from: 0, insert: "-- changed\n" },
      });
    });
    await expect(dialog).toHaveCount(0);
    await editor.fill(bad);
    await inspect.click();
    await expect(dialog).toBeVisible();
    await page.evaluate(() =>
      (
        document.querySelector('[aria-label="Refresh completion context"]') as HTMLButtonElement
      ).click(),
    );
    await expect(dialog).toHaveCount(0);
    await expect(page.locator(".completion-context")).not.toContainText("Loading");
    await page.route("**/rpc/inspection_schema", (route) =>
      route.fulfill({
        status: 403,
        contentType: "application/json",
        body: JSON.stringify({
          code: "PermissionDenied",
          message: "Complete selectable column metadata is unavailable; no column claim was made",
        }),
      }),
    );
    const beforeDenied = executions;
    await inspect.click();
    await expect(page.getByRole("alert")).toContainText("Column inspection unavailable");
    await expect(dialog).toHaveCount(0);
    expect(executions).toBe(beforeDenied);
    await page.unroute("**/rpc/inspection_schema");
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route("**/rpc/inspection_schema", async (route) => {
      await gate;
      await route.continue();
    });
    const requested = page.waitForRequest("**/rpc/inspection_schema");
    await inspect.click();
    await requested;
    await editor.fill(correct);
    release();
    await expect(inspect).toBeEnabled();
    await page.unroute("**/rpc/inspection_schema");
    await inspect.click();
    await expect(dialog).toContainText("No missing columns among the qualified references checked");
    await dialog.getByRole("button", { name: "Close", exact: true }).last().click();
    let releaseNavigation!: () => void;
    const navigationGate = new Promise<void>((resolve) => {
      releaseNavigation = resolve;
    });
    await page.route("**/rpc/inspection_schema", async (route) => {
      await navigationGate;
      await route.continue();
    });
    const navigationRequest = page.waitForRequest("**/rpc/inspection_schema");
    await inspect.click();
    await navigationRequest;
    await editor.press("F1");
    const navigationResponse = page.waitForResponse("**/rpc/inspection_schema");
    releaseNavigation();
    await navigationResponse;
    await expect(inspect).toBeEnabled();
    await expect(dialog).toHaveCount(0);
    await page.unroute("**/rpc/inspection_schema");
    await editor.fill(`WITH c AS (SELECT id FROM ${table}) SELECT c.typo FROM c`);
    const readsBeforeUnsupported = metadataReads;
    await inspect.click();
    await expect(page.getByRole("alert")).toContainText("single direct-table SELECT");
    await expect(dialog).toHaveCount(0);
    expect(metadataReads).toBe(readsBeforeUnsupported);
    expect(executions).toBe(beforeDenied);
    expect(errors).toEqual([]);
  });
}
