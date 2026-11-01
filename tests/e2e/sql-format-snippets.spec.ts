import { expect, test, type Page } from "@playwright/test";

async function connect(page: Page, engine: "PostgreSQL" | "MySQL" | "MariaDB") {
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(`${engine} format snippets`);
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
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("readOnly");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  await expect(page.locator(".completion-context")).toContainText("joinery");
}

for (const engine of ["PostgreSQL", "MySQL", "MariaDB"] as const) {
  test(`${engine} selection formatting and snippet tab stops persist without executing automatically`, async ({
    page,
  }) => {
    const errors: string[] = [],
      executions: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("request", (r) => {
      if (/\/rpc\/(prepare_execution|execute_plan)$/.test(r.url())) executions.push(r.url());
    });
    await connect(page, engine);
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await page.getByRole("button", { name: /^Preferences/ }).click();
    const prefs = page.getByRole("dialog", { name: "Preferences" });
    await prefs.getByLabel("SQL keyword case").selectOption("lower");
    await prefs.getByLabel("SQL indent width").selectOption("4");
    await prefs.getByLabel("Lines between SQL queries").selectOption("2");
    const trigger = `probe_${engine.toLowerCase()}`;
    await prefs.getByLabel("Snippet trigger").fill(trigger);
    await prefs
      .getByLabel("Snippet engine")
      .selectOption(engine === "PostgreSQL" ? "postgres" : "mysql");
    await prefs.getByLabel("Snippet title").fill(`${engine} read probe`);
    await prefs.getByLabel("Snippet template").fill("SELECT ${1:1} AS ${2:answer};${0}");
    await prefs.getByRole("button", { name: "Add snippet" }).click();
    await expect(prefs.getByRole("list", { name: "Custom SQL snippets" })).toContainText(trigger);
    await prefs.getByRole("button", { name: "Done", exact: true }).click();

    // Select the second statement, including its newline, with keyboard input.
    const before = "SELECT   1   AS untouched;";
    await editor.fill(
      `${before}\nSELECT id, total FROM ${engine === "PostgreSQL" ? "public.orders" : "joinery.sample"};\n-- 🧪 preserved suffix`,
    );
    await editor.press("ControlOrMeta+Home");
    await editor.press("ArrowDown");
    await editor.press("Home");
    await editor.press("Shift+ArrowDown");
    await page.getByRole("button", { name: "Format", exact: true }).click();
    await expect(editor.locator(".cm-line").first()).toHaveText(before);
    await expect(editor.locator(".cm-line").nth(1)).toHaveText("select");
    await expect(editor.locator(".cm-line").nth(2)).toHaveText("    id,");
    await expect(editor.locator(".cm-line").last()).toHaveText("-- 🧪 preserved suffix");
    await editor.press("ControlOrMeta+z");
    await expect(editor.locator(".cm-line")).toHaveCount(3);

    // Expand without execution, move between fields, then execute the explicit read.
    await editor.fill(trigger);
    await editor.press("End");
    await editor.press("Control+Space");
    await page.locator(".cm-tooltip-autocomplete li", { hasText: trigger }).first().click();
    await expect(editor).toHaveText("SELECT 1 AS answer;");
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("1");
    await editor.press("Tab");
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("answer");
    await editor.press("Shift+Tab");
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe("1");
    await editor.press("7");
    await expect(editor).toHaveText("SELECT 7 AS answer;");
    expect(executions).toEqual([]);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("1 rows");
    await expect(page.locator(".query-error")).toHaveCount(0);

    // Formatted binds still reach the normal parameter review; quoted/comment
    // colons remain literal text rather than creating extra parameter slots.
    await editor.fill("SELECT :Probe AS answer, ':literal 🧪 日本語' AS note; -- :ignored");
    await page.getByRole("button", { name: "Format", exact: true }).click();
    await expect(editor).toContainText(":Probe as answer");
    await expect(editor).toContainText("':literal 🧪 日本語'");
    await expect(editor).toContainText("-- :ignored");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    const params = page.getByRole("dialog", { name: "Query parameters" });
    await params.getByLabel("Type for parameter Probe").selectOption("integer");
    await params.getByLabel("Value for parameter Probe").fill("7");
    await expect(params.getByLabel(/^Value for parameter/)).toHaveCount(1);
    await params.getByRole("button", { name: "Run query", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("1 rows");
    await expect(page.locator(".query-error")).toHaveCount(0);

    // A real bootstrap after reload proves both settings and templates reached SQLite.
    await page.reload();
    await page.getByRole("button", { name: /^Preferences/ }).click();
    await expect(prefs.getByLabel("SQL keyword case")).toHaveValue("lower");
    await expect(prefs.getByLabel("SQL indent width")).toHaveValue("4");
    await expect(prefs.getByLabel("Lines between SQL queries")).toHaveValue("2");
    await expect(prefs.getByRole("list", { name: "Custom SQL snippets" })).toContainText(trigger);
    await prefs.getByRole("button", { name: `Edit snippet ${trigger}` }).click();
    await prefs.getByLabel("Snippet title").fill("Updated probe");
    await prefs.getByRole("button", { name: "Update snippet" }).click();
    await expect(prefs.getByRole("list", { name: "Custom SQL snippets" })).toContainText(
      "Updated probe",
    );
    await prefs.getByRole("button", { name: `Delete snippet ${trigger}` }).click();
    await expect(prefs.getByRole("button", { name: `Edit snippet ${trigger}` })).toHaveCount(0);
    expect(errors).toEqual([]);
  });
}
