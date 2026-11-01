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
    await page.getByRole("button", { name: "Rename SQL alias", exact: true }).click();
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
