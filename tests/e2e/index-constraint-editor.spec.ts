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

test("StructureView index and constraint editor creates and drops indexes and constraints through ReviewDialog", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await page.goto("/");
  await connectPostgres(page, "D05 Editor PG");

  const cleanupSql = `
    DROP TABLE IF EXISTS public.e2e_d05_orders CASCADE;
    DROP TABLE IF EXISTS public.e2e_d05_customers CASCADE;
  `;

  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill(`
    ${cleanupSql}
    CREATE TABLE public.e2e_d05_customers (
      id BIGINT PRIMARY KEY,
      email TEXT NOT NULL UNIQUE
    );
    CREATE TABLE public.e2e_d05_orders (
      id BIGINT PRIMARY KEY,
      customer_id BIGINT NOT NULL,
      status TEXT NOT NULL DEFAULT 'open',
      total_cents INTEGER NOT NULL DEFAULT 100
    );
  `);
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("affected");

  try {
    await page.getByRole("button", { name: "Refresh schema", exact: true }).click();
    const searchInput = page.getByLabel("Search tables", { exact: true });
    await searchInput.fill("e2e_d05_orders");
    await page
      .locator(".schema-tree")
      .getByRole("button", { name: "e2e_d05_orders", exact: true })
      .click();

    await page.getByRole("button", { name: "Structure", exact: true }).click();
    await expect(page.getByRole("button", { name: "Create index…" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Add constraint…" })).toBeVisible();

    // 1. Create a composite partial index with CONCURRENTLY
    await page.getByRole("button", { name: "Create index…" }).click();
    await expect(page.getByRole("dialog", { name: "Create index" })).toBeVisible();
    await page.getByRole("textbox", { name: "Index name" }).fill("e2e_d05_orders_cust_status_idx");
    await page.getByRole("combobox", { name: "Index column 1" }).selectOption("customer_id");
    await page.getByRole("button", { name: "Add column", exact: true }).click();
    await page.getByRole("combobox", { name: "Index column 2" }).selectOption("status");
    await page.getByRole("checkbox", { name: /CONCURRENTLY/ }).check();
    await page
      .getByRole("textbox", { name: "Partial index WHERE predicate" })
      .fill("status = 'open'");

    const sqlPreview = page.getByTestId("index-constraint-sql-preview");
    await expect(sqlPreview).toContainText(
      'CREATE INDEX CONCURRENTLY "e2e_d05_orders_cust_status_idx" ON "public"."e2e_d05_orders" USING btree ("customer_id", "status") WHERE status = \'open\';',
    );
    await page.getByRole("button", { name: "Review & authorize…" }).click();
    await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
    await page.getByRole("button", { name: "Authorize and run" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);

    const indexesTable = page.getByRole("table", { name: "Indexes" });
    await expect(indexesTable).toContainText("e2e_d05_orders_cust_status_idx");

    // 2. Add a FOREIGN KEY constraint with NOT VALID
    await page.getByRole("button", { name: "Add constraint…" }).click();
    await expect(page.getByRole("dialog", { name: "Add table constraint" })).toBeVisible();
    await page.getByRole("combobox", { name: "Constraint kind" }).selectOption("foreignKey");
    await page.getByRole("textbox", { name: "Constraint name" }).fill("e2e_d05_orders_customer_fk");
    await page.getByRole("combobox", { name: "Constraint column 1" }).selectOption("customer_id");
    await page
      .getByRole("combobox", { name: "Referenced table" })
      .selectOption("public.e2e_d05_customers");
    await page.getByRole("combobox", { name: "Referenced column 1" }).selectOption("id");
    await page
      .getByRole("combobox", { name: "Foreign key ON DELETE action" })
      .selectOption("CASCADE");
    await page.getByRole("checkbox", { name: /NOT VALID/ }).check();

    await expect(sqlPreview).toContainText(
      'ALTER TABLE "public"."e2e_d05_orders" ADD CONSTRAINT "e2e_d05_orders_customer_fk" FOREIGN KEY ("customer_id") REFERENCES "public"."e2e_d05_customers" ("id") ON DELETE CASCADE ON UPDATE NO ACTION NOT VALID;',
    );
    await page.getByRole("button", { name: "Review & authorize…" }).click();
    await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
    await page.getByRole("button", { name: "Authorize and run" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);

    const constraintsTable = page.getByRole("table", { name: "Constraints" });
    await expect(constraintsTable).toContainText("e2e_d05_orders_customer_fk");

    // 3. Drop the secondary index via StructureView
    await page.getByRole("button", { name: "Drop index e2e_d05_orders_cust_status_idx" }).click();
    await expect(
      page.getByRole("dialog", { name: "Drop index e2e_d05_orders_cust_status_idx" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Review & authorize…" }).click();
    await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
    await page.getByRole("button", { name: "Authorize and run" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(indexesTable).not.toContainText("e2e_d05_orders_cust_status_idx");
  } finally {
    if ((await page.getByRole("dialog").count()) > 0) {
      await page.keyboard.press("Escape");
    }
    await page.getByLabel("Search tables", { exact: true }).fill("");
    await page.getByRole("button", { name: "New query tab" }).click();
    await page.getByRole("textbox", { name: "SQL editor" }).fill(cleanupSql);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("affected");
  }
  expect(errors).toEqual([]);
});
