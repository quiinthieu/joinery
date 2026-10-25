import { test, expect } from "@playwright/test";

for (const [engine, port] of [
  ["MySQL", 53306],
  ["MariaDB", 53307],
] as const) {
  test(`${engine} connects, browses, reviews an edit and reads it through SQL`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const value = `${engine.toLowerCase()}_browser_${Date.now()}`;
    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Connection name", { exact: true }).fill(`${engine} fixture`);
    await page.getByLabel("Database engine").selectOption("mysql");
    await expect(page.getByLabel("Port", { exact: true })).toHaveValue("3306");
    await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
    await page.getByLabel("Port", { exact: true }).fill(String(port));
    await page.getByLabel("Database", { exact: true }).fill("joinery");
    await page.getByLabel("Username", { exact: true }).fill("joinery");
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByLabel("Require TLS and verify the server certificate").uncheck();
    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.locator(".status-bar")).toContainText(`${engine} fixture`);
    await page.getByRole("button", { name: /^joinery/ }).click();
    await page.getByRole("button", { name: "sample", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("500 rows loaded");
    const grid = page.locator(".grid-host");
    await expect(grid.locator("canvas").first()).toBeVisible();
    const box = (await grid.boundingBox())!;
    await expect(async () => {
      await page.mouse.dblclick(box.x + 44 + 110 + 200 + 70, box.y + 37 + 18);
      await expect(page.locator("#portal textarea, #portal input").first()).toBeEditable({
        timeout: 500,
      });
    }).toPass({ timeout: 10000 });
    const overlay = page.locator("#portal textarea, #portal input").first();
    await overlay.fill(value);
    await overlay.press("Enter");
    await page.getByRole("button", { name: "Review 1 edit", exact: true }).click();
    await expect(page.getByRole("dialog", { name: "Review pending changes" })).toBeVisible();
    await expect(page.locator(".sql-preview")).toContainText("UPDATE `joinery`.`sample`");
    await expect(page.locator(".sql-preview")).toContainText("CAST(? AS UNSIGNED)");
    await page.getByRole("button", { name: "Apply changes", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.getByRole("status")).toContainText("Changes committed");
    await page.getByRole("button", { name: "New query tab" }).click();
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await expect(editor).toContainText("DATABASE()");
    await editor.fill(
      "SELECT note, CAST(18446744073709551615 AS UNSIGNED) AS exact_integer FROM sample WHERE id=9007199254740992;",
    );
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("1 rows");
    await expect(page.locator(".query-error")).toHaveCount(0);
    if (!(await page.locator(".inspector").isVisible()))
      await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
    const resultBox = (await page.locator(".grid-host").boundingBox())!;
    await expect(async () => {
      await page.mouse.click(resultBox.x + 90, resultBox.y + 55);
      await expect(page.locator(".inspector-value")).toContainText(value, { timeout: 500 });
    }).toPass();
    await editor.fill("SELECT s.no FROM `joinery`.`sample` AS s");
    await expect(async () => {
      await editor.press("Escape");
      await editor.press("Home");
      for (let i = 0; i < 11; i++) await editor.press("ArrowRight");
      await editor.press("Control+Space");
      await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("s.note", {
        timeout: 500,
      });
    }).toPass({ timeout: 10000 });
    expect(errors).toEqual([]);
    await page.screenshot({
      path: `.cache/screenshots/${engine.toLowerCase()}-studio.png`,
      fullPage: true,
    });
  });
}
