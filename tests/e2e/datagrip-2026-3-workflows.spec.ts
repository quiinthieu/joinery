import { expect, test, type Page } from "./fixtures";

const PG_PORT = process.env.JOINERY_POSTGRES_PORT ?? "55432";

async function connectPostgresWithHistory(page: Page, name: string) {
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(PG_PORT);
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  const storeText = page.getByLabel(/Store executed query text/i);
  if ((await storeText.count()) > 0 && !(await storeText.isChecked())) {
    await storeText.check();
  }
  const storeDrafts = page.getByLabel(/Restore open query drafts/i);
  if ((await storeDrafts.count()) > 0 && !(await storeDrafts.isChecked())) {
    await storeDrafts.check();
  }
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(name);
}

test("exercises Transpose/Record views, detached result tabs, Workspace SQL, Data Search, Virtual FKs, and SQL Local History", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));

  await connectPostgresWithHistory(page, "DG 2026.3 Studio");

  // 1. Run a SQL query and inspect Transpose & Record views + detach result tab
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  await editor.fill(
    "SELECT 101 AS account_id, 'ada@example.test' AS owner_email UNION ALL SELECT 102, 'grace@example.test';",
  );
  await page.getByRole("button", { name: "Run", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("2 rows");

  await page.getByRole("button", { name: "Transpose", exact: true }).click();
  await expect(page.getByTestId("transpose-grid-view")).toContainText("ada@example.test");

  await page.getByRole("button", { name: "Record", exact: true }).click();
  await expect(page.getByTestId("record-card-view")).toContainText("ada@example.test");
  await page.getByRole("button", { name: "Next record", exact: true }).click();
  await expect(page.getByTestId("record-card-view")).toContainText("grace@example.test");

  await page.getByRole("button", { name: "Open in tab", exact: true }).click();
  await expect(page.getByRole("button", { name: "Reopen originating SQL" })).toBeVisible();
  await page.getByRole("button", { name: "Reopen originating SQL" }).click();

  // 2. SQL Local History snapshot, diff, and restore
  await page.getByRole("button", { name: "Open SQL Local History", exact: true }).click();
  const historyDialog = page.getByRole("dialog");
  await historyDialog.getByRole("button", { name: "Save snapshot" }).click();
  await historyDialog.getByRole("button", { name: "Close", exact: true }).click();

  await editor.fill("SELECT 999 AS overwritten;");
  await page.getByRole("button", { name: "Open SQL Local History", exact: true }).click();
  await expect(page.getByTestId("local-history-diff")).toBeVisible();
  await page.getByRole("button", { name: "Restore to editor" }).click();
  await expect(editor).toContainText("ada@example.test");

  // 3. Cross-source Workspace SQL over retained result snapshot
  await page.getByRole("button", { name: "Workspace SQL" }).click();
  await expect(page.getByLabel("Workspace SQL query")).toContainText('FROM "');
  await page.getByRole("button", { name: "Run workspace SQL" }).click();
  await expect(page.locator(".workspace-sql-view")).toContainText("mounted rows");

  // 4. Full-text Data Search across disposable PostgreSQL catalog
  await page.getByRole("button", { name: "Data search" }).click();
  await page.getByLabel("Search database values").fill("odd_%");
  await page.getByRole("button", { name: "Search tables" }).click();
  await expect(page.locator(".data-search-view")).toContainText("browse_sort");

  // 5. Virtual Foreign Key registration on table Structure view
  await page.getByRole("button", { name: /^public/ }).click();
  await page.getByRole("button", { name: "orders", exact: true }).click();
  await page.getByRole("button", { name: "Structure", exact: true }).click();
  await page.getByRole("button", { name: "Add virtual FK…" }).click();
  const vfkDialog = page.getByRole("dialog");
  await vfkDialog.getByLabel("Referenced table").fill("customers");
  await vfkDialog.getByLabel("Source column 1").fill("customer_id");
  await vfkDialog.getByLabel("Referenced column 1").fill("id");
  await vfkDialog.getByRole("button", { name: "Save virtual FK" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".structure-view, .object-inspector-scroll").first()).toContainText(
    "virtual",
  );

  expect(errors).toEqual([]);
});
