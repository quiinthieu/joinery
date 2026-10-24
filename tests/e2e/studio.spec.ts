import { test, expect, type Page } from "@playwright/test";

async function connect(page: Page, name: string, safeMode = "silent") {
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill("55432");
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption(safeMode);
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(name);
}

test("connects and runs SQL with separate binary-backed result tabs", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await connect(page, "Query fixture");
  await page.getByRole("button", { name: "New query tab" }).click();
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  await editor.fill(
    "SELECT 9223372036854775807::bigint AS exact_integer, NULL::text AS absent; SELECT 'hello; world' AS message;",
  );
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-tabs").getByRole("button")).toHaveCount(2);
  await expect(page.locator(".result-footer")).toContainText("1 rows");
  await expect(page.locator(".query-error")).toHaveCount(0);
  await expect(page.locator("canvas").first()).toBeVisible();
  expect(errors).toEqual([]);
  await page.screenshot({ path: ".cache/screenshots/sql-studio.png", fullPage: true });
});

test("reviews an inline change and reads the committed value back", async ({ page }) => {
  const changedValue = `browser_${Date.now()}`;
  await connect(page, "Grid fixture");
  await page.getByRole("button", { name: /^public/ }).click();
  await page.getByRole("button", { name: "orders", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("500 rows loaded");
  const grid = page.locator(".grid-host");
  await expect(grid.locator("canvas").first()).toBeVisible();
  const box = (await grid.boundingBox())!;
  await expect(async () => {
    await page.mouse.dblclick(box.x + 44 + 110 + 110 + 70, box.y + 37 + 18);
    await expect(page.locator("#portal textarea, #portal input").first()).toBeVisible({
      timeout: 500,
    });
  }).toPass({ timeout: 10000 });
  const overlay = page.locator("#portal textarea, #portal input").first();
  await overlay.fill(changedValue);
  await overlay.press("Enter");
  await page.getByRole("button", { name: "Review 1 edit", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Review pending changes" })).toBeVisible();
  await expect(page.locator(".sql-preview")).toContainText('UPDATE "public"."orders"');
  await page.getByRole("button", { name: "Apply changes", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("status")).toContainText("Changes committed");
  await expect(page.locator(".dirty-count")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  await page
    .getByRole("textbox", { name: "SQL editor" })
    .fill("SELECT status FROM public.orders WHERE id=1;");
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("1 rows");
  await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
  const resultBox = (await page.locator(".grid-host").boundingBox())!;
  await expect(async () => {
    await page.mouse.click(resultBox.x + 100, resultBox.y + 55);
    await expect(page.locator(".inspector-value")).toContainText(changedValue, { timeout: 500 });
  }).toPass();
  await page.screenshot({ path: ".cache/screenshots/table-grid.png", fullPage: true });
});

test("guards writes, cancels a long query, and opens a million-row benchmark", async ({ page }) => {
  await connect(page, "Guard fixture", "guard");
  await page.getByRole("button", { name: "New query tab" }).click();
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  await editor.fill("UPDATE public.orders SET notes=notes WHERE id=-1;");
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await editor.fill("SELECT pg_sleep(10);");
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await page.getByRole("button", { name: /Cancel/ }).click();
  await expect(page.locator(".status-bar")).toContainText("disconnected");
  await page.getByRole("button", { name: "Jump to anything" }).click();
  await page.getByRole("button", { name: /Grid benchmark/ }).click();
  await expect(page.locator(".result-footer")).toContainText("1,000,000 rows");
  await expect(page.locator("canvas").first()).toBeVisible();
  const box = (await page.locator(".grid-host").boundingBox())!;
  await page.mouse.move(box.x + 300, box.y + 150);
  await page.mouse.wheel(0, 15000);
  await expect
    .poll(() =>
      page
        .locator(".grid-host")
        .evaluate((root) =>
          Math.max(0, ...Array.from(root.querySelectorAll("*")).map((node) => node.scrollTop)),
        ),
    )
    .toBeGreaterThan(1000);
  await page.screenshot({ path: ".cache/screenshots/benchmark.png", fullPage: true });
});

test("manages nested groups and reads local audit entries", async ({ page }) => {
  const group = `Analytics ${Date.now()}`;
  await connect(page, "Audit fixture");
  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill("SELECT 1;");
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("1 rows");
  await page.getByRole("button", { name: /^Preferences/ }).click();
  await page.getByLabel("Group name", { exact: true }).fill(group);
  await page.getByRole("combobox", { name: "Parent group", exact: true }).selectOption("local");
  await page.getByRole("button", { name: "Add group", exact: true }).click();
  await expect(
    page.getByRole("button", { name: `Rename group ${group}`, exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await expect(page.getByRole("button", { name: group, exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Query history", exact: true }).click();
  await page.getByRole("button", { name: "Audit log", exact: true }).click();
  await expect(page.locator(".audit-table")).toContainText("Audit fixture");
  await expect(page.locator(".audit-table")).toContainText("Completed");
  await page.screenshot({ path: ".cache/screenshots/audit-log.png", fullPage: true });
});
