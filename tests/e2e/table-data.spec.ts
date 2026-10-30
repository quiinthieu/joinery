import { test, expect, type Page } from "@playwright/test";

async function connect(page: Page, name: string) {
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill("55432");
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
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
