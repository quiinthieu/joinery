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

test("Sidebar and StructureView table/column DDL editor creates, alters, and drops tables and columns through ReviewDialog", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await page.goto("/");
  await connectPostgres(page, "N08 Editor PG");

  const cleanupSql = `DROP TABLE IF EXISTS public.e2e_n08_invoices CASCADE;`;

  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill(cleanupSql);
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("affected");

  try {
    // 1. Create table from Sidebar schema action
    await page.getByRole("button", { name: "Create table in public" }).click();
    await expect(page.getByRole("dialog", { name: "Create table" })).toBeVisible();

    await page.getByLabel("Table name").fill("e2e_n08_invoices");
    await page.getByLabel("Table comment").fill("E2E invoice table");
    await page.getByLabel("Column 2 name").fill("note_text");

    const preview = page.getByLabel("Generated DDL preview");
    await expect(preview).toContainText('CREATE TABLE "public"."e2e_n08_invoices"');
    await expect(preview).toContainText(
      'COMMENT ON TABLE "public"."e2e_n08_invoices" IS \'E2E invoice table\';',
    );

    await page.getByRole("button", { name: "Review & authorize…" }).click();
    await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
    await page.getByRole("button", { name: "Authorize and run" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);

    // Newly created table opens automatically; switch to Structure view
    await page.getByRole("button", { name: "Structure", exact: true }).click();
    await expect(page.getByRole("button", { name: "Add column…" })).toBeVisible();

    // 2. Add a column
    await page.getByRole("button", { name: "Add column…" }).click();
    await expect(page.getByRole("dialog", { name: "Add column" })).toBeVisible();
    await page.getByLabel("Column name").fill("amount_text");
    await page.getByLabel("Column data type").fill("text");
    await page.getByLabel("Column default expression").fill("'100'");
    await expect(page.getByLabel("Generated DDL preview")).toContainText(
      'ALTER TABLE "public"."e2e_n08_invoices" ADD COLUMN "amount_text" text DEFAULT \'100\';',
    );
    await page.getByRole("button", { name: "Review & authorize…" }).click();
    await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
    await page.getByRole("button", { name: "Authorize and run" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);

    await expect(page.getByRole("button", { name: "Modify column amount_text" })).toBeVisible();

    // 3. Modify and rename column with USING cast
    await page.getByRole("button", { name: "Modify column amount_text" }).click();
    await expect(page.getByRole("dialog", { name: "Modify column" })).toBeVisible();
    await page.getByLabel("Column name").fill("amount_cents");
    await page.getByLabel("Column data type").fill("bigint");
    await page.getByLabel("Column default expression").fill("0");
    await page.getByLabel("PostgreSQL USING cast expression").fill('"amount_text"::bigint');
    await expect(page.getByLabel("Generated DDL preview")).toContainText(
      'ALTER TABLE "public"."e2e_n08_invoices" ALTER COLUMN "amount_text" TYPE bigint USING "amount_text"::bigint',
    );
    await expect(page.getByLabel("Generated DDL preview")).toContainText(
      'ALTER TABLE "public"."e2e_n08_invoices" RENAME COLUMN "amount_text" TO "amount_cents";',
    );
    await page.getByRole("button", { name: "Review & authorize…" }).click();
    await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
    await page.getByRole("button", { name: "Authorize and run" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);

    await expect(page.getByRole("button", { name: "Modify column amount_cents" })).toBeVisible();

    // 4. Drop column note_text
    await page.getByRole("button", { name: "Drop column note_text" }).click();
    await expect(page.getByRole("dialog", { name: "Drop column" })).toBeVisible();
    await page.getByRole("button", { name: "Review & authorize…" }).click();
    await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
    await page.getByRole("button", { name: "Authorize and run" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Drop column note_text" })).toHaveCount(0);

    // 5. Drop the table via StructureView
    await page.getByRole("button", { name: "Drop table e2e_n08_invoices" }).click();
    await expect(page.getByRole("dialog", { name: "Drop table" })).toBeVisible();
    await page.getByRole("button", { name: "Review & authorize…" }).click();
    await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
    await page.getByRole("button", { name: "Authorize and run" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
  } finally {
    if ((await page.getByRole("dialog").count()) > 0) {
      await page.keyboard.press("Escape");
    }
    await page.getByRole("button", { name: "New query tab" }).click();
    await page.getByRole("textbox", { name: "SQL editor" }).fill(cleanupSql);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("affected");
  }
  expect(errors).toEqual([]);
});
