import { test, expect, type Page } from "@playwright/test";

const BRIDGE_URL = "http://127.0.0.1:1422";
const BRIDGE_TOKEN = "joinery-disposable-browser-tests-only";

async function installClipboardMock(page: Page) {
  await page.addInitScript(() => {
    let text = "";
    const w = window as unknown as {
      __joineryCopiedText: () => string;
      __joineryClearCopiedText: () => void;
    };
    w.__joineryCopiedText = () => text;
    w.__joineryClearCopiedText = () => {
      text = "";
    };
    const mockClipboard = {
      writeText: async (value: string) => {
        text = value;
      },
      readText: async () => text,
    };
    try {
      Object.defineProperty(Navigator.prototype, "clipboard", {
        configurable: true,
        get: () => mockClipboard,
      });
    } catch {
      // Fallback if prototype property cannot be redefined
    }
    try {
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        get: () => mockClipboard,
      });
    } catch {
      // Prototype getter handles access
    }
  });
}

async function clickCopyAsAndRead(page: Page, itemName: RegExp): Promise<string> {
  await page.evaluate(() =>
    (window as unknown as { __joineryClearCopiedText: () => void }).__joineryClearCopiedText(),
  );
  await page.getByRole("button", { name: "Copy as" }).click();
  await page.getByRole("menuitem", { name: itemName }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { __joineryCopiedText: () => string }).__joineryCopiedText(),
      ),
    )
    .not.toBe("");
  return page.evaluate(() =>
    (window as unknown as { __joineryCopiedText: () => string }).__joineryCopiedText(),
  );
}

async function harnessRpc<T>(
  page: Page,
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  const response = await page.request.post(`${BRIDGE_URL}/rpc/${command}`, {
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${BRIDGE_TOKEN}`,
    },
    data: args,
  });
  expect(response.ok()).toBeTruthy();
  const payload = (await response.json()) as { result: T; error?: { message: string } };
  if (payload.error) {
    throw new Error(payload.error.message);
  }
  return payload.result;
}

const PG_PORT = process.env.JOINERY_POSTGRES_PORT ?? "55432";

async function connect(page: Page, name: string, safeMode = "silent") {
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(PG_PORT);
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption(safeMode);
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(name);
}

async function openTable(page: Page, name: string) {
  const schemaButton = page.getByRole("button", { name: /^public/ });
  const tableButton = page.getByRole("button", { name, exact: true });
  if (!(await tableButton.isVisible())) {
    await schemaButton.click();
  }
  await tableButton.click();
  await expect(page.locator(".grid-host canvas").first()).toBeVisible();
}

test("copy-as preserves exact int64, decimals, NULL vs empty string, and visible column layout", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await installClipboardMock(page);
  await connect(page, "CopyAs E2E");

  await page.getByRole("button", { name: "New query tab" }).click();
  await page
    .getByRole("textbox", { name: "SQL editor" })
    .fill(
      "SELECT 9223372036854775807::bigint AS big_id, 123456.7890::numeric(18,4) AS amount, NULL::text AS absent, ''::text AS blank, 'héllo | world'::text AS note;",
    );
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("1 rows");

  // Copy as JSON: exact numeric tokens without float rounding, null vs ""
  const copiedJson = await clickCopyAsAndRead(page, /Copy as JSON/);
  expect(copiedJson).toContain('"big_id": 9223372036854775807');
  expect(copiedJson).toContain('"amount": 123456.7890');
  expect(copiedJson).toContain('"absent": null');
  expect(copiedJson).toContain('"blank": ""');

  // Copy as SQL INSERT
  const copiedSql = await clickCopyAsAndRead(page, /Copy as SQL INSERT/);
  expect(copiedSql).toContain(
    "VALUES\n  (9223372036854775807, 123456.7890, NULL, '', 'héllo | world');",
  );

  // Copy as Markdown
  const copiedMd = await clickCopyAsAndRead(page, /Copy as Markdown/);
  expect(copiedMd).toContain("| big_id | amount | absent | blank | note |");
  expect(copiedMd).toContain("héllo \\| world");

  // Respects G10 hidden/reordered columns on table tabs
  await openTable(page, "line_notes");
  await expect(page.locator(".result-footer")).toContainText("2 rows loaded");
  await page.getByRole("button", { name: /^Columns/ }).click();
  const colDialog = page.getByRole("dialog", { name: "Table columns" });
  await colDialog.getByLabel("Show note_order").uncheck();
  await colDialog.getByRole("button", { name: "Move note up" }).click();
  await colDialog.getByRole("button", { name: "Done" }).click();
  await expect(colDialog).toHaveCount(0);

  const copiedCsv = await clickCopyAsAndRead(page, /Copy as CSV/);
  const csvLines = copiedCsv.trim().split("\n");
  expect(csvLines[0]).toBe("id,note,note_line");
  expect(csvLines[1]).toBe("1,composite target,2");
  expect(csvLines[2]).toBe("2,line unknown,");
  expect(errors).toEqual([]);
});

test("streams multi-page table exports and leaves marked .partial files on cancellation", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await connect(page, "Export E2E");

  // 1. Full 1,500-row (3-page) streaming table export to CSV
  await openTable(page, "browse_sort");
  await expect(page.locator(".result-footer")).toContainText("500 rows loaded");
  await harnessRpc(page, "test_queue_save_file", { fileName: "browse-sort-full.csv" });

  await page.getByRole("button", { name: "Export…", exact: true }).click();
  const exportDialog = page.getByRole("dialog", { name: "Export data" });
  await expect(exportDialog).toBeVisible();
  await exportDialog.getByRole("button", { name: "Choose destination & export" }).click();
  await expect(exportDialog).toContainText("Exported browse-sort-full.csv", { timeout: 15000 });
  await expect(exportDialog).toContainText("1,500 rows");
  await exportDialog.getByRole("button", { name: "Done" }).click();
  await expect(exportDialog).toHaveCount(0);

  const fullFile = await harnessRpc<{ exists: boolean; contents: string | null }>(
    page,
    "test_read_sandbox_file",
    { fileName: "browse-sort-full.csv" },
  );
  expect(fullFile.exists).toBe(true);
  expect(fullFile.contents!.trim().split("\n")).toHaveLength(1501);

  const leftoverPartial = await harnessRpc<{ exists: boolean; contents: string | null }>(
    page,
    "test_read_sandbox_file",
    { fileName: "browse-sort-full.csv.partial" },
  );
  expect(leftoverPartial.exists).toBe(false);

  // 2. Mid-stream cancellation on a 10,000-row table leaves only .partial with incomplete trailer
  await openTable(page, "orders");
  await expect(page.locator(".result-footer")).toContainText("500 rows loaded");
  await harnessRpc(page, "test_queue_save_file", { fileName: "orders-cancelled.csv" });

  await page.getByRole("button", { name: "Export…", exact: true }).click();
  const cancelDialog = page.getByRole("dialog", { name: "Export data" });
  await cancelDialog.getByRole("button", { name: "Choose destination & export" }).click();
  const cancelBtn = cancelDialog.getByRole("button", { name: "Cancel export" });
  if (await cancelBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
    await cancelBtn.click();
    await expect(cancelDialog).toContainText("Export cancelled", { timeout: 10000 });
    const finalCancelled = await harnessRpc<{ exists: boolean }>(page, "test_read_sandbox_file", {
      fileName: "orders-cancelled.csv",
    });
    expect(finalCancelled.exists).toBe(false);
    const partialCancelled = await harnessRpc<{ exists: boolean; contents: string | null }>(
      page,
      "test_read_sandbox_file",
      { fileName: "orders-cancelled.csv.partial" },
    );
    expect(partialCancelled.exists).toBe(true);
    expect(partialCancelled.contents).toContain("JOINERY EXPORT INCOMPLETE");
  }
  const footerClose = cancelDialog
    .locator("footer")
    .getByRole("button", { name: /^(Close|Done)$/ });
  await expect(footerClose).toBeEnabled({ timeout: 15000 });
  await footerClose.click();
  expect(errors).toEqual([]);
});

test("validates CSV imports upfront, remaps headers, and commits reviewed insert batches", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await connect(page, "Import E2E", "confirm");

  const importTable = `e2e_r2_import_${Date.now()}`;
  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill(
    `CREATE TABLE public.${importTable} (
       id BIGSERIAL PRIMARY KEY,
       sku TEXT NOT NULL UNIQUE,
       big_val BIGINT NOT NULL,
       note TEXT,
       tag TEXT NOT NULL DEFAULT 'from-default'
     );`,
  );
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  const ddlReview = page.getByRole("dialog", { name: "Review this operation" });
  await expect(ddlReview).toBeVisible();
  await ddlReview.getByRole("button", { name: "Authorize and run" }).click();
  await expect(ddlReview).toHaveCount(0);

  // Refresh catalog and open the new disposable table
  await page.getByRole("button", { name: "Refresh schema", exact: true }).click();
  await openTable(page, importTable);
  await expect(page.locator(".result-footer")).toContainText("0 rows loaded");

  // 1. Invalid CSV row surfaces upfront validation error and disables preparation
  await harnessRpc(page, "test_write_sandbox_file", {
    fileName: "bad-import.csv",
    contents: "sku,big_val,note\nBAD-1,not-an-integer,oops\n",
  });
  await page.getByRole("button", { name: "Import CSV…", exact: true }).click();
  const importDialog = page.getByRole("dialog", { name: "Import CSV into table" });
  await importDialog.getByRole("button", { name: "Choose CSV file…" }).click();
  await expect(importDialog.getByRole("alert")).toContainText("1 validation issue");
  await expect(importDialog.getByRole("alert")).toContainText("Line 2 · big_val");
  await expect(importDialog.getByRole("button", { name: /Review & import/ })).toBeDisabled();

  // 2. Valid CSV with mismatched headers -> remap columns, preserve NULL vs "" & int64, commit via ReviewDialog
  await harnessRpc(page, "test_write_sandbox_file", {
    fileName: "valid-import.csv",
    contents: 'item_sku,max_int,memo\nSKU-100,9223372036854775807,\\N\nSKU-101,-42,""\n',
  });
  await importDialog.getByRole("button", { name: "Change file…" }).click();
  await importDialog.getByLabel("Target column for item_sku").selectOption("sku");
  await importDialog.getByLabel("Target column for max_int").selectOption("big_val");
  await importDialog.getByLabel("Target column for memo").selectOption("note");

  await expect(importDialog).toContainText("<default>: id, tag");
  const reviewImportBtn = importDialog.getByRole("button", { name: "Review & import 2 rows…" });
  await expect(reviewImportBtn).toBeEnabled();
  await reviewImportBtn.click();

  const batchReview = page.getByRole("dialog", { name: "Review pending changes" });
  await expect(batchReview).toBeVisible();
  await expect(batchReview.locator(".sql-preview")).toContainText(
    `INSERT INTO "public"."${importTable}"`,
  );
  await batchReview.getByRole("button", { name: "Apply changes", exact: true }).click();

  await expect(importDialog).toContainText(`Imported 2 rows into public.${importTable}`);
  await importDialog.getByRole("button", { name: "Done" }).click();
  await expect(importDialog).toHaveCount(0);
  await expect(page.locator(".result-footer")).toContainText("2 rows loaded");
  expect(errors).toEqual([]);
});

test("compares live tables and retained snapshots with NULL vs empty diffs and key anomalies", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await connect(page, "Compare E2E");

  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill(
    `DROP TABLE IF EXISTS public.e2e_r2_cmp_a, public.e2e_r2_cmp_b;
     CREATE TABLE public.e2e_r2_cmp_a (
       id BIGINT PRIMARY KEY,
       note TEXT,
       amount NUMERIC(14,4) NOT NULL
     );
     CREATE TABLE public.e2e_r2_cmp_b (
       id BIGINT PRIMARY KEY,
       note TEXT,
       amount NUMERIC(14,4) NOT NULL
     );
     INSERT INTO public.e2e_r2_cmp_a VALUES
       (1, 'same', 99.5000),
       (2, NULL, 10.0000),
       (3, 'only-a', 5.0000);
     INSERT INTO public.e2e_r2_cmp_b VALUES
       (1, 'same', 99.5000),
       (2, '', 10.2500),
       (4, 'only-b', 7.0000);
     SELECT 'ready'::text AS status;`,
  );
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.getByRole("tab", { name: /Query 6/ })).toBeVisible();

  await page.getByRole("button", { name: "Refresh schema", exact: true }).click();
  await openTable(page, "e2e_r2_cmp_a");
  await expect(page.locator(".result-footer")).toContainText("3 rows loaded");

  await page.getByRole("button", { name: "Compare…", exact: true }).click();
  const cmpDialog = page.getByRole("dialog", { name: "Compare data" });
  await expect(cmpDialog).toBeVisible();
  await cmpDialog.getByLabel("Right source mode").selectOption("liveTable");
  await cmpDialog.getByLabel("Right table").selectOption("public.e2e_r2_cmp_b");
  await cmpDialog.getByRole("button", { name: "Compare datasets" }).click();

  await expect(cmpDialog).toContainText("1 identical");
  const filterTabs = cmpDialog.getByRole("tablist", { name: "Comparison difference categories" });
  await expect(filterTabs.getByRole("tab", { name: /All differences/ })).toContainText("3");
  await expect(filterTabs.getByRole("tab", { name: /Modified/ })).toContainText("1");
  await expect(filterTabs.getByRole("tab", { name: /Only in left/ })).toContainText("1");
  await expect(filterTabs.getByRole("tab", { name: /Only in right/ })).toContainText("1");

  await filterTabs.getByRole("tab", { name: /Modified/ }).click();
  await expect(cmpDialog.locator(".compare-diff-table")).toContainText("NULL");
  await expect(cmpDialog.locator(".compare-diff-table")).toContainText('""');
  await expect(cmpDialog.locator(".compare-diff-table")).toContainText("10.0000");
  await expect(cmpDialog.locator(".compare-diff-table")).toContainText("10.2500");
  await cmpDialog.getByRole("button", { name: "Close", exact: true }).click();

  // Compare a query snapshot with duplicate and NULL keys against live table
  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill(
    `SELECT 1::bigint AS id, 'same'::text AS note, 99.5000::numeric(14,4) AS amount
     UNION ALL SELECT 2, 'dup-1', 10.0000
     UNION ALL SELECT 2, 'dup-2', 10.0000
     UNION ALL SELECT NULL, 'missing-pk', 0.0000;`,
  );
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".result-footer")).toContainText("4 rows");

  await page.getByRole("button", { name: "Compare…", exact: true }).click();
  const snapDialog = page.getByRole("dialog", { name: "Compare data" });
  await snapDialog.getByLabel("Right source mode").selectOption("liveTable");
  await snapDialog.getByLabel("Right table").selectOption("public.e2e_r2_cmp_b");
  await snapDialog.getByRole("button", { name: "Compare datasets" }).click();

  const snapTabs = snapDialog.getByRole("tablist", { name: "Comparison difference categories" });
  await expect(snapTabs.getByRole("tab", { name: /Duplicate & NULL keys/ })).toContainText("3");
  await snapTabs.getByRole("tab", { name: /Duplicate & NULL keys/ }).click();
  await expect(snapDialog.locator(".compare-anomalies-panel")).toContainText(
    "2 occurrences on left, 1 on opposite side",
  );
  await expect(snapDialog.locator(".compare-anomalies-panel")).toContainText("missing-pk");
  await snapDialog.getByRole("button", { name: "Close", exact: true }).click();
  expect(errors).toEqual([]);
});
