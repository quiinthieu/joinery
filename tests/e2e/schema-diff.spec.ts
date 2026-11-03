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

test("PostgreSQL schema diff compares live schemas, captures snapshots, and discloses unsupported objects", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await page.goto("/");
  await connectPostgres(page, "Diff PG");

  const cleanupSql = `
    DROP SCHEMA IF EXISTS e2e_diff_left CASCADE;
    DROP SCHEMA IF EXISTS e2e_diff_right CASCADE;
  `;

  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill(`
    ${cleanupSql}
    CREATE SCHEMA e2e_diff_left;
    CREATE SCHEMA e2e_diff_right;

    CREATE TYPE e2e_diff_left.invoice_state AS ENUM ('open', 'paid');

    CREATE TABLE e2e_diff_left."Line Items" (
      id BIGINT PRIMARY KEY,
      "Unit Price" NUMERIC(10, 2) NOT NULL DEFAULT 0,
      legacy_note TEXT
    );
    CREATE INDEX e2e_line_price_idx ON e2e_diff_left."Line Items" ("Unit Price");

    CREATE TABLE e2e_diff_right."Line Items" (
      id BIGINT PRIMARY KEY,
      "Unit Price" NUMERIC(12, 2) NOT NULL DEFAULT 0,
      discount_code TEXT,
      CONSTRAINT e2e_price_positive CHECK ("Unit Price" >= 0)
    );
    CREATE UNIQUE INDEX e2e_line_price_idx ON e2e_diff_right."Line Items" ("Unit Price");

    CREATE TABLE e2e_diff_left.only_left_archive (id INTEGER PRIMARY KEY);
    CREATE TABLE e2e_diff_right.only_right_staging (id INTEGER PRIMARY KEY);
  `);
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("affected");

  try {
    await page.getByRole("button", { name: "Refresh schema", exact: true }).click();
    await page.getByRole("button", { name: "Compare schema e2e_diff_left" }).click();

    await expect(page.getByRole("heading", { name: "Compare Schemas & Snapshots" })).toBeVisible();
    await page.getByLabel("Right schema name").fill("e2e_diff_right");
    await page.getByRole("button", { name: "Compare schemas", exact: true }).click();

    await expect(page.locator(".compare-provenance-bar")).toContainText("e2e_diff_left");
    await expect(page.locator(".compare-provenance-bar")).toContainText("e2e_diff_right");

    const unsupported = page.getByLabel("Unsupported schema objects disclosure");
    await expect(unsupported).toBeVisible();
    await expect(unsupported).toContainText("invoice_state");

    const lineItemsCard = page.getByTestId("schema-diff-object-Line Items");
    await expect(lineItemsCard).toBeVisible();
    await expect(lineItemsCard).toContainText('"Line Items"');
    const membersTable = lineItemsCard.getByRole("table", {
      name: "Member differences for Line Items",
    });
    await expect(membersTable).toContainText('"Unit Price"');
    await expect(membersTable).toContainText("legacy_note");
    await expect(membersTable).toContainText("discount_code");
    await expect(membersTable).toContainText("e2e_price_positive");

    // Filter by diff kind
    const filterGroup = page.getByRole("group", { name: "Diff kind filter" });
    await filterGroup.getByRole("button", { name: /Only in left/ }).click();
    await expect(page.getByTestId("schema-diff-object-only_left_archive")).toBeVisible();
    await expect(page.getByTestId("schema-diff-object-only_right_staging")).toHaveCount(0);

    await filterGroup.getByRole("button", { name: /Only in right/ }).click();
    await expect(page.getByTestId("schema-diff-object-only_right_staging")).toBeVisible();
    await expect(page.getByTestId("schema-diff-object-only_left_archive")).toHaveCount(0);

    await filterGroup.getByRole("button", { name: /All differences/ }).click();

    // Capture a snapshot of the left schema
    await page.getByRole("button", { name: "Capture left schema snapshot" }).click();
    const snapshotStrip = page.getByLabel("Captured schema snapshots");
    await expect(snapshotStrip).toBeVisible();
    await expect(snapshotStrip).toContainText("e2e_diff_left");

    // Preview Migration DDL (Left -> Right) and verify generated statements and destructive badge
    await page.getByRole("button", { name: "Migrate Left → Right DDL" }).click();
    const ddlPreview = page.getByTestId("schema-ddl-preview");
    await expect(ddlPreview).toBeVisible();
    await expect(ddlPreview).toContainText("destructive");
    const ddlScript = page.getByTestId("schema-ddl-script");
    await expect(ddlScript).toContainText('DROP TABLE "e2e_diff_left"."only_left_archive";');
    await expect(ddlScript).toContainText('CREATE TABLE "e2e_diff_left"."only_right_staging"');
    await expect(ddlScript).toContainText(
      'ALTER TABLE "e2e_diff_left"."Line Items" ALTER COLUMN "Unit Price" TYPE numeric(12,2);',
    );
  } finally {
    await page.getByRole("button", { name: "New query tab" }).click();
    await page.getByRole("textbox", { name: "SQL editor" }).fill(cleanupSql);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("affected");
  }
  expect(errors).toEqual([]);
});
