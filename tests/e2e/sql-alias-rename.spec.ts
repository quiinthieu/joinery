import { expect, test, type Locator, type Page } from "@playwright/test";
import { tableFromIPC } from "apache-arrow";

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
  test(`${engine} CTE rename preserves ordered scopes, explicit aliases and query results`, async ({
    page,
  }) => {
    const errors: string[] = [];
    let executions = 0;
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("request", (request) => {
      if (/\/rpc\/(prepare_execution|execute_plan)$/.test(request.url())) executions++;
    });
    await connect(page, engine, port);
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const table = engine === "PostgreSQL" ? "public.orders" : "joinery.sample";
    const q = engine === "PostgreSQL" ? '"' : "`";
    const body = `SELECT id FROM ${table} ORDER BY id LIMIT 2`;
    const nested = (name: string) =>
      engine === "PostgreSQL"
        ? `WITH ${name} AS (${body}) SELECT ${name}.id FROM ${name} WHERE EXISTS (WITH prior_rows AS (SELECT ${name}.id FROM ${name}), src AS (SELECT id FROM prior_rows) SELECT src.id FROM src) ORDER BY ${name}.id`
        : `WITH ${name} AS (${body}), prior_rows AS (SELECT ${name}.id FROM ${name}) SELECT ${name}.id FROM ${name} WHERE EXISTS (SELECT prior_rows.id FROM prior_rows) ORDER BY ${name}.id`;
    for (const [original, renamed, name, count] of [
      [
        `WITH src(id) AS (${body}), next_rows AS (SELECT src.id FROM src) SELECT src.id, x.id FROM src JOIN next_rows x ON x.id = src.id ORDER BY src.id`,
        `WITH renamed(id) AS (${body}), next_rows AS (SELECT renamed.id FROM renamed) SELECT renamed.id, x.id FROM renamed JOIN next_rows x ON x.id = renamed.id ORDER BY renamed.id`,
        "renamed",
        7,
      ],
      [nested("src"), nested("renamed"), "renamed", 6],
      [
        `WITH ${q}Old${q} AS (${body}) SELECT ${q}Old${q}.id FROM ${q}Old${q} ORDER BY ${q}Old${q}.id`,
        `WITH ${q}New${q}${q}Name${q} AS (${body}) SELECT ${q}New${q}${q}Name${q}.id FROM ${q}New${q}${q}Name${q} ORDER BY ${q}New${q}${q}Name${q}.id`,
        `New${q}Name`,
        4,
      ],
    ] as const) {
      await placeCaret(editor, original.replace("WITH ", "WITH |"));
      const run = async () => {
        const response = page.waitForResponse((r) => r.url().endsWith("/rpc/execute_plan"));
        const result = page.waitForResponse(
          (r) => r.ok() && /\/result\/[^?]+\?start=0&/.test(r.url()),
        );
        await editor.press("ControlOrMeta+Enter");
        expect((await response).ok()).toBe(true);
        await expect(page.locator(".result-footer")).toContainText("2 rows");
        await expect(page.locator(".run-button")).toBeVisible();
        await expect(page.locator(".completion-context")).not.toContainText("Loading");
        await expect(page.locator(".query-error")).toHaveCount(0);
        const values = tableFromIPC(new Uint8Array(await (await result).body()));
        return Array.from({ length: values.numRows }, (_, row) =>
          values.schema.fields.map((_, column) => String(values.getChildAt(column)!.get(row))),
        );
      };
      const originalRows = await run();
      expect(originalRows.map((row) => row[0])).toEqual(
        engine === "PostgreSQL" ? ["1", "2"] : ["9007199254740992", "9007199254740993"],
      );
      const before = executions;
      await editor.press("Shift+F6");
      const dialog = page.getByRole("dialog", { name: "Rename SQL CTE" });
      await expect(dialog).toContainText(`${count} local occurrences`);
      if (count === 7) {
        await dialog.getByLabel("New CTE name").fill("next_rows");
        await expect(dialog.getByRole("alert")).toContainText("conflict");
        await expect(dialog.getByRole("button", { name: "Apply rename" })).toBeDisabled();
        await dialog.getByLabel("New CTE name").fill("x");
        await expect(dialog.getByRole("alert")).toContainText("conflict");
      }
      await dialog.getByLabel("New CTE name").fill(name);
      await dialog.getByLabel("New CTE name").press("ControlOrMeta+Enter");
      await expect(dialog).toBeVisible();
      await expect(dialog.getByLabel("CTE rename preview")).toBeVisible();
      await expect(page.locator('.cm-content[aria-label="SQL editor"]')).toHaveText(original);
      await dialog.getByRole("button", { name: "Cancel" }).click();
      await expect(editor).toHaveText(original);
      await page.getByRole("button", { name: "Rename SQL alias or CTE", exact: true }).click();
      await dialog.getByLabel("New CTE name").fill(name);
      await dialog.getByRole("button", { name: "Apply rename" }).click();
      await expect(editor).toHaveText(renamed);
      expect(executions).toBe(before);
      expect(await run()).toEqual(originalRows);
      await editor.press("ControlOrMeta+z");
      await expect(editor).toHaveText(original);
    }
    if (engine !== "PostgreSQL") {
      for (const localName of ["src", "local_rows"]) {
        const unsupported = `WITH src AS (${body}) SELECT src.id FROM src WHERE EXISTS (WITH prior_rows AS (SELECT src.id FROM src), ${localName} AS (SELECT id FROM prior_rows) SELECT ${localName}.id FROM ${localName}) ORDER BY src.id`;
        await placeCaret(editor, unsupported.replace("WITH ", "WITH |"));
        const before = executions;
        await editor.press("Shift+F6");
        await expect(page.getByRole("dialog", { name: "Rename SQL CTE" })).toHaveCount(0);
        await expect(page.getByRole("alert")).toContainText("cannot safely analyze");
        await expect(editor).toHaveText(unsupported);
        expect(executions).toBe(before);
      }
    }
    await placeCaret(editor, `WITH |src AS (${body}) SELECT src.id FROM src`);
    await editor.press("Shift+F6");
    const dialog = page.getByRole("dialog", { name: "Rename SQL CTE" });
    await expect(dialog).toBeVisible();
    await page.evaluate(async () => {
      const { EditorView } = await import("/node_modules/@codemirror/view/dist/index.js");
      EditorView.findFromDOM(document.querySelector(".cm-editor")!)!.dispatch({
        changes: { from: 0, insert: "-- changed\n" },
      });
    });
    await expect(dialog).toHaveCount(0);
    await placeCaret(editor, `WITH |src AS (${body}) SELECT src.id FROM src`);
    await editor.press("Shift+F6");
    await expect(dialog).toBeVisible();
    await page.evaluate(() =>
      (
        document.querySelector('[aria-label="Refresh completion context"]') as HTMLButtonElement
      ).click(),
    );
    await expect(dialog).toHaveCount(0);
    await placeCaret(editor, "WITH RECURSIVE |src AS (SELECT 1 AS id) SELECT src.id FROM src");
    await editor.press("Shift+F6");
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole("alert")).toContainText("cannot safely analyze");
    expect(errors).toEqual([]);
  });

  test(`${engine} ordered JOIN renames preserve correlated query results`, async ({ page }) => {
    const errors: string[] = [];
    let executions = 0;
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("request", (request) => {
      if (/\/rpc\/(prepare_execution|execute_plan)$/.test(request.url())) executions++;
    });
    await connect(page, engine, port);
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const table = engine === "PostgreSQL" ? "public.orders" : "joinery.sample";
    const prefix = `WITH src AS (SELECT id FROM ${table} ORDER BY id LIMIT 2) `;
    const min = "(SELECT MIN(id) FROM src)",
      max = "(SELECT MAX(id) FROM src)";
    for (const [body, renamedBody] of [
      [
        `SELECT o.id FROM src o WHERE EXISTS (SELECT o.id FROM src x JOIN src y ON y.id = o.id AND y.id = ${min} JOIN src o ON o.id = x.id WHERE o.id = ${max})`,
        `SELECT renamed.id FROM src renamed WHERE EXISTS (SELECT o.id FROM src x JOIN src y ON y.id = renamed.id AND y.id = ${min} JOIN src o ON o.id = x.id WHERE o.id = ${max})`,
      ],
      [
        `SELECT o.id FROM src o WHERE EXISTS (SELECT o.id FROM src o, src x JOIN src y ON EXISTS (SELECT 1 WHERE y.id = o.id AND y.id = ${min}) WHERE o.id = ${max})`,
        `SELECT renamed.id FROM src renamed WHERE EXISTS (SELECT o.id FROM src o, src x JOIN src y ON EXISTS (SELECT 1 WHERE y.id = renamed.id AND y.id = ${min}) WHERE o.id = ${max})`,
      ],
      [
        `SELECT o.id FROM src o WHERE EXISTS (SELECT o.id FROM src o JOIN src x JOIN src y ON y.id = o.id AND y.id = ${min} ON x.id = o.id WHERE o.id = ${max})`,
        `SELECT renamed.id FROM src renamed WHERE EXISTS (SELECT o.id FROM src o JOIN src x JOIN src y ON y.id = renamed.id AND y.id = ${min} ON x.id = o.id WHERE o.id = ${max})`,
      ],
    ]) {
      const original = prefix + body;
      const run = async () => {
        const response = page.waitForResponse((r) => r.url().endsWith("/rpc/execute_plan"));
        await editor.press("ControlOrMeta+Enter");
        expect((await response).ok()).toBe(true);
        await expect(page.locator(".result-footer")).toContainText("1 rows");
        // Results can arrive before the finished/session-context events. A preview
        // started during that transition must invalidate rather than apply stale edits.
        await expect(page.locator(".run-button")).toBeVisible();
        await expect(page.locator(".completion-context")).not.toContainText("Loading");
        await expect(page.locator(".query-error")).toHaveCount(0);
      };
      await placeCaret(editor, prefix + body.replace("SELECT o.id", "SELECT |o.id"));
      await run();
      const before = executions;
      await editor.press("Shift+F6");
      const dialog = page.getByRole("dialog", { name: "Rename SQL alias" });
      await expect(dialog).toContainText("3 local occurrences");
      await dialog.getByLabel("New alias").fill("y");
      await expect(dialog.getByRole("alert")).toContainText("capture");
      await expect(dialog.getByRole("button", { name: "Apply rename" })).toBeDisabled();
      await dialog.getByLabel("New alias").fill("renamed");
      await expect(page.locator('.cm-content[aria-label="SQL editor"]')).toHaveText(original);
      await dialog.getByRole("button", { name: "Apply rename" }).click();
      await expect(editor).toHaveText(prefix + renamedBody);
      expect(executions).toBe(before);
      await run();
      await editor.press("ControlOrMeta+z");
      await expect(editor).toHaveText(original);
    }
    expect(errors).toEqual([]);
  });

  test(`${engine} alias rename previews, rejects capture and applies as one undo without execution`, async ({
    page,
  }) => {
    const errors: string[] = [],
      executions: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("request", (request) => {
      if (/\/rpc\/(prepare_execution|execute_plan)$/.test(request.url()))
        executions.push(request.url());
    });
    await connect(page, engine, port);
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const table = engine === "PostgreSQL" ? "public.orders" : "joinery.sample";
    const original = `SELECT o.id, o.* FROM ${table} o WHERE EXISTS (SELECT o.id FROM ${table} o) AND EXISTS (SELECT x.id FROM ${table} x WHERE x.id = o.id); -- o.id`;
    await placeCaret(editor, original.replace("SELECT o.id", "SELECT |o.id"));
    await editor.press("Shift+F6");
    const dialog = page.getByRole("dialog", { name: "Rename SQL alias" });
    await expect(dialog).toContainText("4 local occurrences");
    await dialog.getByLabel("New alias").fill("x");
    await expect(dialog.getByRole("alert")).toContainText("capture");
    await expect(dialog.getByRole("button", { name: "Apply rename" })).toBeDisabled();
    await dialog.getByLabel("New alias").fill("order_rows");
    await dialog.getByLabel("New alias").press("ControlOrMeta+Shift+Enter");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByLabel("Alias rename preview")).toContainText("o → order_rows");
    // The modal correctly hides the editor from the accessibility tree.
    await expect(page.locator('.cm-content[aria-label="SQL editor"]')).toHaveText(original);
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(editor).toBeFocused();
    await expect(editor).toHaveText(original);
    await page.getByRole("button", { name: "Rename SQL alias or CTE", exact: true }).click();
    await dialog.getByLabel("New alias").fill("order_rows");
    await dialog.getByRole("button", { name: "Apply rename" }).click();
    await expect(dialog).toHaveCount(0);
    await expect(editor).toHaveText(
      original
        .replace("SELECT o.id, o.*", "SELECT order_rows.id, order_rows.*")
        .replace(`${table} o WHERE`, `${table} order_rows WHERE`)
        .replace("x.id = o.id", "x.id = order_rows.id"),
    );
    await expect(editor).toBeFocused();
    await editor.press("ControlOrMeta+z");
    await expect(editor).toHaveText(original);
    // A programmatic document update while the modal is open invalidates the preview.
    await editor.press("Shift+F6");
    await expect(dialog).toBeVisible();
    await page.evaluate(async () => {
      const { EditorView } = await import("/node_modules/@codemirror/view/dist/index.js");
      const view = EditorView.findFromDOM(document.querySelector(".cm-editor")!);
      view!.dispatch({ changes: { from: 0, insert: "-- changed\n" } });
    });
    await expect(dialog).toHaveCount(0);
    await expect(editor).toContainText("-- changed");
    await placeCaret(editor, `SELECT |o.id FROM ${table} o`);
    await editor.press("Shift+F6");
    await expect(dialog).toBeVisible();
    // Simulate a context refresh arriving while the modal owns keyboard focus.
    await page.evaluate(() =>
      (
        document.querySelector('[aria-label="Refresh completion context"]') as HTMLButtonElement
      ).click(),
    );
    await expect(dialog).toHaveCount(0);
    await expect(page.locator(".completion-context")).toContainText("joinery");
    await placeCaret(editor, "SELECT |o.id FROM orders o UNION SELECT x.id FROM orders x");
    await editor.press("Shift+F6");
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole("alert")).toContainText("cannot safely analyze");
    const quote = engine === "PostgreSQL" ? '"' : "`";
    await placeCaret(editor, `SELECT |${quote}Old${quote}.id FROM ${table} ${quote}Old${quote}`);
    await editor.press("Shift+F6");
    await dialog.getByLabel("New alias").fill(`New${quote}Name`);
    await dialog.getByRole("button", { name: "Apply rename" }).click();
    await expect(editor).toHaveText(
      `SELECT ${quote}New${quote}${quote}Name${quote}.id FROM ${table} ${quote}New${quote}${quote}Name${quote}`,
    );
    expect(executions).toEqual([]);
    expect(errors).toEqual([]);
  });
}
