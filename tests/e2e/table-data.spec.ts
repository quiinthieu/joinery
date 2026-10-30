import { test, expect, type Page } from "@playwright/test";

async function connect(page: Page, name: string, safeMode?: string) {
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill("55432");
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  if (safeMode) await page.getByLabel("Safe Mode", { exact: true }).selectOption(safeMode);
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(name);
  if (!(await page.locator(".inspector").isVisible()))
    await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
}

async function openTable(page: Page, name: string) {
  await page.getByRole("button", { name: /^public/ }).click();
  await page.getByRole("button", { name, exact: true }).click();
  await expect(page.locator(".grid-host canvas").first()).toBeVisible();
}

// Row markers are 44 px wide and the header is 37 px; integer columns are 110 px.
const ROW_MARKERS = 44;
const HEADER = 37;

async function cellText(page: Page, x: number, row = 0) {
  const box = (await page.locator(".grid-host").boundingBox())!;
  let text = "";
  await expect(async () => {
    await page.mouse.click(box.x + ROW_MARKERS + x, box.y + HEADER + 18 + row * 36);
    text = (await page.locator(".inspector-value").textContent({ timeout: 500 })) ?? "";
    expect(text.length).toBeGreaterThan(0);
  }).toPass({ timeout: 10000 });
  return text.trim();
}

async function clickHeader(page: Page, x: number, shift = false) {
  const box = (await page.locator(".grid-host").boundingBox())!;
  // page.mouse has no modifier option; hold Shift on the keyboard instead.
  if (shift) await page.keyboard.down("Shift");
  await page.mouse.click(box.x + ROW_MARKERS + x, box.y + 18);
  if (shift) await page.keyboard.up("Shift");
}

test("sorts and filters table data on the server with stable keyset paging", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await connect(page, "Sort fixture");
  await openTable(page, "browse_sort");
  const footer = page.locator(".result-footer");
  await expect(footer).toContainText("500 rows loaded");
  await expect.poll(() => cellText(page, 55)).toBe("1");

  // bucket ascending (NULLs last), id tie-breaker: id 10 has bucket 0.
  await clickHeader(page, 110 + 55);
  await expect(page.locator(".sort-summary")).toContainText("Sorted by bucket ascending");
  await expect.poll(() => cellText(page, 55)).toBe("10");
  // Descending puts NULL buckets first: id 7 is the first multiple of 7.
  await clickHeader(page, 110 + 55);
  await expect(page.locator(".sort-summary")).toContainText("bucket descending");
  await expect.poll(() => cellText(page, 55)).toBe("7");
  await expect.poll(() => cellText(page, 110 + 55)).toBe("NULL");
  // Shift adds a secondary key without dropping the first.
  await clickHeader(page, 220 + 100, true);
  await expect(page.locator(".sort-summary")).toContainText(
    "Sorted by bucket descending, label ascending",
  );

  // LIKE metacharacters are literal: only labels containing "_%" match.
  const filters = page.getByRole("toolbar", { name: "Table filters" });
  await filters.getByRole("button", { name: "Add filter" }).click();
  await filters.getByLabel("Filter column").selectOption("label");
  await filters.getByLabel("Filter operator").selectOption("contains");
  await filters.getByLabel("Filter value").fill("_%");
  await filters.getByRole("button", { name: "Apply" }).click();
  await expect(filters).toContainText("label contains '_%'");
  await expect(footer).toContainText("105 rows loaded");
  await expect.poll(() => cellText(page, 220 + 100)).toMatch(/^odd_%\d+$/);

  await filters.getByRole("button", { name: "Remove filter label contains '_%'" }).click();
  await filters.getByRole("button", { name: "Clear sort" }).click();
  await expect(page.locator(".sort-summary")).toHaveCount(0);
  await expect.poll(() => cellText(page, 55)).toBe("1");

  // Scrolling continues through later keyset pages under a sort.
  await clickHeader(page, 110 + 55);
  const box = (await page.locator(".grid-host").boundingBox())!;
  await page.mouse.move(box.x + 300, box.y + 200);
  for (let i = 0; i < 12 && !(await footer.textContent())?.includes("1,500 rows"); i++)
    await page.mouse.wheel(0, 20000);
  await expect(footer).toContainText("1,500 rows loaded");
  expect(errors).toEqual([]);
  await page.screenshot({ path: ".cache/screenshots/table-sort-filter.png", fullPage: true });
});

test("pending edits lock sort and filter controls until discarded", async ({ page }) => {
  await connect(page, "Sort lock fixture");
  await openTable(page, "browse_sort");
  await expect(page.locator(".result-footer")).toContainText("500 rows loaded");
  const box = (await page.locator(".grid-host").boundingBox())!;
  await expect(async () => {
    await page.mouse.dblclick(box.x + ROW_MARKERS + 220 + 100, box.y + HEADER + 18);
    await expect(page.locator("#portal textarea, #portal input").first()).toBeVisible({
      timeout: 500,
    });
  }).toPass({ timeout: 10000 });
  const overlay = page.locator("#portal textarea, #portal input").first();
  await overlay.fill("pending");
  await overlay.press("Enter");
  await expect(page.getByRole("button", { name: "Review 1 edit", exact: true })).toBeVisible();
  const filters = page.getByRole("toolbar", { name: "Table filters" });
  await expect(filters).toContainText("Review or discard pending edits");
  await expect(filters.getByRole("button", { name: "Add filter" })).toBeDisabled();
  await clickHeader(page, 110 + 55);
  await expect(page.getByRole("status")).toContainText("Review or discard pending edits");
  await expect(page.locator(".sort-summary")).toHaveCount(0);
  await page.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(filters.getByRole("button", { name: "Add filter" })).toBeEnabled();
  await clickHeader(page, 110 + 55);
  await expect(page.locator(".sort-summary")).toContainText("bucket ascending");
});

test("follows single and composite foreign keys to referenced rows in new tabs", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await connect(page, "Foreign key fixture");
  const tabs = page.getByRole("tablist", { name: "Workspaces" });
  const footer = page.locator(".result-footer");

  // orders.id 1 has customer_id 2.
  await openTable(page, "orders");
  await expect(footer).toContainText("500 rows loaded");
  await expect.poll(() => cellText(page, 110 + 55)).toBe("2");
  await page.keyboard.press("ControlOrMeta+Shift+B");
  await expect(tabs.getByRole("tab", { name: "customers · id = 2" })).toBeVisible();
  await expect(footer).toContainText("1 rows loaded");
  await expect(page.getByRole("toolbar", { name: "Table filters" })).toContainText("id = '2'");
  await expect.poll(() => cellText(page, 55)).toBe("2");

  // line_notes 1 references order_lines (7, 2) through differently named columns.
  await page.getByRole("button", { name: "line_notes", exact: true }).click();
  await expect(footer).toContainText("2 rows loaded");
  await expect.poll(() => cellText(page, 220 + 55)).toBe("2");
  await page.getByRole("button", { name: "Go to referenced row", exact: true }).click();
  await expect(
    tabs.getByRole("tab", { name: "order_lines · order_id = 7, line_no = 2" }),
  ).toBeVisible();
  await expect(footer).toContainText("1 rows loaded");
  await expect.poll(() => cellText(page, 220 + 100)).toBe("SKU-7-2");

  // note_order belongs to two constraints, so a chooser appears.
  await tabs.getByRole("tab", { name: "line_notes", exact: true }).click();
  await expect.poll(() => cellText(page, 110 + 55)).toBe("7");
  await page.keyboard.press("ControlOrMeta+Shift+B");
  const chooser = page.getByRole("dialog", { name: "Go to referenced row" });
  await expect(chooser).toContainText("public.order_lines (order_id, line_no)");
  await chooser.getByRole("button", { name: /public\.orders \(id\)/ }).click();
  await expect(tabs.getByRole("tab", { name: "orders · id = 7" })).toBeVisible();
  await expect(footer).toContainText("1 rows loaded");

  // A NULL component references nothing.
  await tabs.getByRole("tab", { name: "line_notes", exact: true }).click();
  await expect.poll(() => cellText(page, 220 + 55, 1)).toBe("NULL");
  await page.keyboard.press("ControlOrMeta+Shift+B");
  await expect(page.getByRole("status")).toContainText("references no row");
  expect(errors).toEqual([]);
  await page.screenshot({ path: ".cache/screenshots/foreign-key-navigation.png", fullPage: true });
});

test("classifies raw table WHERE and ORDER BY clauses through Safe Mode gates", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));

  await connect(page, "Raw clause confirm", "confirm");
  await openTable(page, "browse_sort");
  const filters = page.getByRole("toolbar", { name: "Table filters" });
  const footer = page.locator(".result-footer");
  await expect(footer).toContainText("500 rows loaded");

  await filters.getByLabel("WHERE clause").fill("id IN (2, 10, 100)");
  await filters.getByLabel("ORDER BY clause").fill("id DESC");
  await filters.getByRole("button", { name: "Run SQL clauses" }).click();
  await expect(footer).toContainText("3 rows loaded");
  await expect(filters).toContainText("First 500 rows · custom ORDER BY");
  await expect.poll(() => cellText(page, 55, 0)).toBe("100");
  await expect.poll(() => cellText(page, 55, 1)).toBe("10");
  await expect.poll(() => cellText(page, 55, 2)).toBe("2");

  // Stacked statements are rejected before reaching the database.
  await filters.getByLabel("WHERE clause").fill("id = 1; SELECT 1");
  await filters.getByRole("button", { name: "Run SQL clauses" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Enter a single SQL expression without semicolons",
  );
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "Dismiss query error" }).click();

  // Unclassified functions require one-use ReviewDialog authorization on writable profiles.
  await filters
    .getByLabel("WHERE clause")
    .fill("id = 10 AND current_setting('server_version_num') <> ''");
  await filters.getByLabel("ORDER BY clause").fill("");
  await filters.getByRole("button", { name: "Run SQL clauses" }).click();
  const review = page.getByRole("dialog", { name: "Review this operation" });
  await expect(review).toBeVisible();
  await review.getByRole("button", { name: "Authorize and run" }).click();
  await expect(review).toHaveCount(0);
  await expect(footer).toContainText("1 rows loaded");
  await expect.poll(() => cellText(page, 55, 0)).toBe("10");

  await filters.getByRole("button", { name: "Reset SQL clauses" }).click();
  await expect(footer).toContainText("500 rows loaded");

  // ReadOnly Safe Mode blocks unclassified SQL clauses without opening ReviewDialog.
  await connect(page, "Raw clause readonly", "readOnly");
  await openTable(page, "browse_sort");
  await expect(footer).toContainText("500 rows loaded");
  await filters
    .getByLabel("WHERE clause")
    .fill("id = 10 AND current_setting('server_version_num') <> ''");
  await filters.getByRole("button", { name: "Run SQL clauses" }).click();
  await expect(page.getByRole("alert")).toContainText("This connection is read-only");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "Dismiss query error" }).click();

  await filters.getByLabel("WHERE clause").fill("id IN (2, 10)");
  await filters.getByRole("button", { name: "Run SQL clauses" }).click();
  await expect(footer).toContainText("2 rows loaded");
  expect(errors).toEqual([]);
});

test("navigates inbound referencing rows across single, multi-constraint, and composite keys", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await connect(page, "Inbound FK fixture");
  const tabs = page.getByRole("tablist", { name: "Workspaces" });
  const footer = page.locator(".result-footer");

  // Single inbound FK: customers.id = 2 is referenced by 10 orders.
  await openTable(page, "customers");
  await expect(footer).toContainText("500 rows loaded");
  await expect.poll(() => cellText(page, 55, 1)).toBe("2");
  await page.keyboard.press("ControlOrMeta+Shift+G");
  await expect(tabs.getByRole("tab", { name: "orders · customer_id = 2" })).toBeVisible();
  await expect(footer).toContainText("10 rows loaded");

  // Multiple inbound FKs: orders.id = 7 is referenced by both line_notes and order_lines.
  await page.getByRole("button", { name: "orders", exact: true }).click();
  await expect(footer).toContainText("500 rows loaded");
  await expect.poll(() => cellText(page, 55, 6)).toBe("7");
  await page.getByRole("button", { name: "Go to referencing rows", exact: true }).click();
  const chooser = page.getByRole("dialog", { name: "Go to referencing rows" });
  await expect(chooser).toContainText("public.line_notes (note_order)");
  await expect(chooser).toContainText("public.order_lines (order_id)");
  await chooser.getByRole("button", { name: /public\.order_lines \(order_id\)/ }).click();
  await expect(tabs.getByRole("tab", { name: "order_lines · order_id = 7" })).toBeVisible();
  await expect(footer).toContainText("3 rows loaded");

  // Composite inbound FK: order_lines (7, 2) is referenced by line_notes row 1.
  await expect.poll(() => cellText(page, 220 + 100, 1)).toBe("SKU-7-2");
  await page.keyboard.press("ControlOrMeta+Shift+G");
  await expect(
    tabs.getByRole("tab", { name: "line_notes · note_order = 7, note_line = 2" }),
  ).toBeVisible();
  await expect(footer).toContainText("1 rows loaded");
  await expect.poll(() => cellText(page, 330 + 100, 0)).toBe("composite target");

  // Table with no inbound foreign keys reports cleanly.
  await page.getByRole("button", { name: "browse_sort", exact: true }).click();
  await expect(footer).toContainText("500 rows loaded");
  await expect.poll(() => cellText(page, 55, 0)).toBe("1");
  await page.keyboard.press("ControlOrMeta+Shift+G");
  await expect(page.getByRole("status")).toContainText(
    "No visible tables reference public.browse_sort.",
  );
  expect(errors).toEqual([]);
});

test("reorders, hides, and resizes table columns with persistence across reload", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const profileName = `Column layout ${Date.now()}`;
  await connect(page, profileName);
  await openTable(page, "customers");
  const footer = page.locator(".result-footer");
  await expect(footer).toContainText("500 rows loaded");
  await expect.poll(() => cellText(page, 55, 0)).toBe("1");

  const saved = page.waitForResponse(
    (response) => response.url().endsWith("/rpc/save_workspace") && response.ok(),
  );
  await page.getByRole("button", { name: "Columns", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Table columns" });
  await dialog.getByLabel("Move email up").click();
  await dialog.getByLabel("Move email up").click();
  await dialog.getByLabel("Show metadata").uncheck();
  await dialog.getByLabel("Show created_at").uncheck();
  await dialog.getByLabel("Width for email").fill("320");
  await dialog.getByRole("button", { name: "Done", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await saved;

  await expect(page.getByRole("button", { name: "Columns (4/6)", exact: true })).toBeVisible();
  await expect.poll(() => cellText(page, 55, 0)).toBe("customer1@example.test");
  await expect(page.locator(".inspector h3")).toHaveText("email");

  // Reload the app, reconnect the saved profile, and verify the table layout persisted.
  await page.reload();
  await page.getByRole("button", { name: profileName, exact: true }).click();
  const reconnect = page.getByRole("dialog", { name: `Connect to ${profileName}` });
  await reconnect.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await reconnect.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(reconnect).toHaveCount(0);
  if (!(await page.locator(".inspector").isVisible()))
    await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();

  await openTable(page, "customers");
  await expect(footer).toContainText("500 rows loaded");
  await expect(page.getByRole("button", { name: "Columns (4/6)", exact: true })).toBeVisible();
  await expect.poll(() => cellText(page, 55, 0)).toBe("customer1@example.test");
  await expect(page.locator(".inspector h3")).toHaveText("email");

  // Verify persisted width, hidden columns, and the >= 1 visible column guard.
  await page.getByRole("button", { name: "Columns (4/6)", exact: true }).click();
  const reopened = page.getByRole("dialog", { name: "Table columns" });
  await expect(reopened.getByLabel("Width for email")).toHaveValue("320");
  await expect(reopened.getByLabel("Show metadata")).not.toBeChecked();
  await expect(reopened.getByLabel("Show created_at")).not.toBeChecked();
  await reopened.getByLabel("Show id").uncheck();
  await reopened.getByLabel("Show name").uncheck();
  await reopened.getByLabel("Show active").uncheck();
  await expect(reopened.getByLabel("Show email")).toBeDisabled();
  await reopened.getByRole("button", { name: "Reset column layout", exact: true }).click();
  await reopened.getByRole("button", { name: "Done", exact: true }).click();

  await expect(page.getByRole("button", { name: "Columns", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reset column layout" })).toHaveCount(0);
  await expect.poll(() => cellText(page, 55, 0)).toBe("1");
  expect(errors).toEqual([]);
});

async function dispatchPaste(page: Page, tsv: string) {
  await page.evaluate((payload) => {
    const dt = new DataTransfer();
    dt.setData("text/plain", payload);
    const event = new ClipboardEvent("paste", {
      clipboardData: dt,
      bubbles: true,
      cancelable: true,
    });
    (document.activeElement ?? window).dispatchEvent(event);
  }, tsv);
}

test("validates and applies multi-cell TSV paste atomically across reordered columns", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await connect(page, `Atomic paste ${Date.now()}`);
  await openTable(page, "customers");
  await expect(page.locator(".result-footer")).toContainText("500 rows loaded");

  // Hide read-only id and created_at, and move email ahead of name so visible columns are:
  // [email (230px), name (200px), active (200px), metadata (200px)].
  await page.getByRole("button", { name: "Columns", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Table columns" });
  await dialog.getByLabel("Show id").uncheck();
  await dialog.getByLabel("Show created_at").uncheck();
  await dialog.getByLabel("Move email up").click();
  await dialog.getByRole("button", { name: "Done", exact: true }).click();
  await expect(dialog).toHaveCount(0);

  // Select visual [0, 0] (email on row 1) and test an atomic rejection first:
  // row 1 is valid, row 2 attempts NULL (\N) in NOT NULL "name".
  await expect.poll(() => cellText(page, 80, 0)).toBe("customer1@example.test");
  await dispatchPaste(page, "valid.first@example.test\tAria Chen\nsecond@example.test\t\\N\n");
  await expect(page.getByRole("status")).toContainText('Row 2, column "name" does not allow NULL.');
  await expect(page.getByRole("button", { name: /^Review / })).toHaveCount(0);
  await expect(page.locator(".inspector-value")).toHaveText("customer1@example.test");

  // Also reject invalid JSON in row 2 without mutating row 1.
  await dispatchPaste(
    page,
    'valid.first@example.test\tAria Chen\tfalse\t{"tier":"pro"}\n\\N\tBram Ortiz\ttrue\t{broken\n',
  );
  await expect(page.getByRole("status")).toContainText(
    'Row 2, column "metadata" expects valid JSON.',
  );
  await expect(page.getByRole("button", { name: /^Review / })).toHaveCount(0);
  await expect(page.locator(".inspector-value")).toHaveText("customer1@example.test");

  // Now paste a valid 2x4 TSV matrix including \N NULL, boolean literals, and JSON.
  await dispatchPaste(
    page,
    'aria.chen@example.test\tAria Chen\t0\t{"tier":"enterprise"}\n\\N\tBram Ortiz\t0\t{"tier":"pro"}\n',
  );
  await expect(page.getByRole("button", { name: "Review 8 edits", exact: true })).toBeVisible();
  await expect(page.locator(".inspector-value")).toHaveText("aria.chen@example.test");
  await expect.poll(() => cellText(page, 230 + 80, 0)).toBe("Aria Chen");
  await expect.poll(() => cellText(page, 80, 1)).toBe("NULL");
  await expect.poll(() => cellText(page, 230 + 80, 1)).toBe("Bram Ortiz");

  await page.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(page.getByRole("button", { name: /^Review / })).toHaveCount(0);
  expect(errors).toEqual([]);
});
