import { test, expect, type Page } from "@playwright/test";

const PG_PORT = Number(process.env.JOINERY_POSTGRES_PORT ?? "55432");

async function connectPostgres(page: Page, name: string) {
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Database engine").selectOption("postgres");
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(String(PG_PORT));
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(name);
}

test("table maintenance dialog previews and runs reviewed ANALYZE from Structure view and refreshes storage stats", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await page.goto("/");
  await connectPostgres(page, "A06 Maintenance PG");

  const setupSql = `DROP TABLE IF EXISTS public.e2e_a06_maint CASCADE;
CREATE TABLE public.e2e_a06_maint (
  id BIGINT PRIMARY KEY,
  category TEXT NOT NULL,
  note TEXT NOT NULL
);
INSERT INTO public.e2e_a06_maint VALUES (1, 'alpha', 'row-1'), (2, 'beta', 'row-2');`;

  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill(setupSql);
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("affected");

  await page.getByRole("button", { name: "Refresh tables" }).click();

  try {
    await page
      .getByRole("button", { name: /e2e_a06_maint/ })
      .first()
      .click();
    await page.getByRole("button", { name: "Structure", exact: true }).click();

    const storageSection = page.getByRole("region", { name: "Storage and activity" });
    await expect(storageSection).toBeVisible();

    await storageSection.getByRole("button", { name: "Table maintenance" }).click();
    const dialog = page.getByRole("dialog", { name: /Table Maintenance — public\.e2e_a06_maint/ });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByLabel("Generated maintenance SQL preview")).toContainText(
      'ANALYZE "public"."e2e_a06_maint";',
    );

    // Preview VACUUM options and then switch back to ANALYZE
    await dialog.getByRole("radio", { name: "VACUUM" }).click();
    await expect(dialog.getByLabel("Generated maintenance SQL preview")).toContainText(
      'VACUUM (ANALYZE) "public"."e2e_a06_maint";',
    );
    await dialog.getByRole("radio", { name: "ANALYZE" }).click();

    await dialog.getByRole("button", { name: "Review & authorize…" }).click();
    const reviewDialog = page.getByRole("dialog", { name: "Review this operation" });
    await expect(reviewDialog).toBeVisible();
    await reviewDialog.getByRole("button", { name: "Authorize and run" }).click();

    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(storageSection).toBeVisible();
    expect(errors).toEqual([]);
  } finally {
    await page.getByRole("button", { name: "New query tab" }).click();
    await page
      .getByRole("textbox", { name: "SQL editor" })
      .fill("DROP TABLE IF EXISTS public.e2e_a06_maint CASCADE;");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  }
});
