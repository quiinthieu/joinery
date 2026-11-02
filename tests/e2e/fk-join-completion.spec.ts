import { expect, test, type Locator } from "@playwright/test";

async function complete(editor: Locator, sql: string) {
  await editor.fill(sql);
  await editor.press("Escape");
  await editor.press("End");
  await editor.press("Control+Space");
}

for (const [engine, port] of [
  ["PostgreSQL", Number(process.env.JOINERY_POSTGRES_PORT ?? "55432")],
  ["MySQL", 53306],
  ["MariaDB", 53307],
] as const) {
  test(`${engine} inserts and executes composite outbound, inbound and ON FK completions`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/");
    await page.getByRole("button", { name: "New connection", exact: true }).click();
    await page.getByLabel("Connection name", { exact: true }).fill(`${engine} FK joins`);
    await page
      .getByLabel("Database engine")
      .selectOption(engine === "PostgreSQL" ? "postgres" : "mysql");
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
    await expect(page.locator(".completion-context")).toContainText("joinery");
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const popup = page.locator(".cm-tooltip-autocomplete");

    for (const [draft, option, expected] of [
      [
        "SELECT ln.note, ol.sku FROM line_notes ln JOIN o",
        /order_lines ol ON ln\.note_order = ol\.order_id AND ln\.note_line = ol\.line_no/,
        "ON ln.note_order = ol.order_id AND ln.note_line = ol.line_no",
      ],
      [
        "SELECT ol.sku FROM order_lines ol JOIN l",
        /line_notes (ln2?) ON ol\.order_id = \1\.note_order AND ol\.line_no = \1\.note_line/,
        engine === "PostgreSQL"
          ? "ON ol.order_id = ln2.note_order AND ol.line_no = ln2.note_line"
          : "ON ol.order_id = ln.note_order AND ol.line_no = ln.note_line",
      ],
      [
        "SELECT ln.note, ol.sku FROM line_notes ln JOIN order_lines ol ON ln.n",
        /ln\.note_order = ol\.order_id AND ln\.note_line = ol\.line_no/,
        "ON ln.note_order = ol.order_id AND ln.note_line = ol.line_no",
      ],
    ] as const) {
      await complete(editor, draft);
      const item = popup.locator("li", { hasText: option });
      await expect(item).toBeVisible();
      await item.click();
      await expect(editor).toContainText(expected);
      await expect(page.locator(".query-error")).toHaveCount(0);
      await page.getByRole("button", { name: "Run all statements", exact: true }).click();
      await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
      await expect(page.locator(".result-footer")).toContainText("1 rows");
      await expect(page.locator(".query-error")).toHaveCount(0);
    }
    expect(errors).toEqual([]);
  });
}
