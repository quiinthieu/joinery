import { test, expect, type Page } from "@playwright/test";

test("workspace tabs and editor size are keyboard accessible", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "New query tab" }).click();
  await page.getByRole("button", { name: "New query tab" }).click();
  const tabs = page.getByRole("tablist", { name: "Workspaces" });
  const first = tabs.getByRole("tab", { name: "Query 1", exact: true });
  const second = tabs.getByRole("tab", { name: "Query 2", exact: true });
  await second.focus();
  await second.press("ArrowRight");
  await expect(first).toBeFocused();
  await expect(first).toHaveAttribute("aria-selected", "true");
  await expect(second).toHaveAttribute("tabindex", "-1");
  await first.press("End");
  await expect(second).toBeFocused();
  const panel = page.getByRole("tabpanel", { name: "Query 2", exact: true });
  const divider = panel.getByRole("separator", { name: "Resize SQL editor" });
  await divider.focus();
  await divider.press("ArrowDown");
  await expect(divider).toHaveAttribute("aria-valuenow", "310");
  await divider.press("Home");
  await expect(divider).toHaveAttribute("aria-valuenow", "140");
  await page.getByRole("button", { name: "Jump to anything" }).click();
  await expect(page.locator(".main-shell")).toHaveAttribute("inert", "");
  await expect(page.getByRole("tablist", { name: "Workspaces" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(second).toBeVisible();
  const closeSecond = page.getByRole("button", { name: "Close Query 2", exact: true });
  await closeSecond.focus();
  await closeSecond.press("Enter");
  await expect(first).toBeFocused();
  await expect(first).toHaveAttribute("aria-controls", "workspace-panel");
});

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

test("keyboard palette keeps search focus, traps Tab and restores its trigger", async ({
  page,
}) => {
  await page.goto("/");
  const trigger = page.getByRole("button", { name: "Jump to anything" });
  await trigger.focus();
  await trigger.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  const search = palette.getByRole("textbox", { name: "Search commands" });
  await expect(palette).toBeVisible();
  // Wait beyond the Modal effect's scheduled focus, which used to override
  // React's autoFocus and move keyboard users onto the close button.
  await expect(async () => {
    await page.waitForTimeout(30);
    await expect(search).toBeFocused();
  }).toPass({ timeout: 1000 });
  await search.press("Shift+Tab");
  const close = palette.getByRole("button", { name: "Close dialog" });
  await expect(close).toBeFocused();
  await close.press("Shift+Tab");
  await expect(palette.getByRole("button").last()).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(close).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(palette).toHaveCount(0);
  await expect(trigger).toBeFocused();

  await trigger.press("Control+k");
  await expect(search).toBeFocused();
  await page.keyboard.insertText("Grid benchmark");
  await expect(palette.locator(".palette-commands button")).toHaveCount(1);
  await page.keyboard.press("Enter");
  await expect(palette).toHaveCount(0);
  await expect(page.locator(".result-footer")).toContainText("1,000,000 rows");
});

test("Snowflake form requires browser sign-in and read-only mode", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Database engine").selectOption("snowflake");
  await expect(page.getByLabel("Snowflake account hostname")).toBeVisible();
  await expect(page.getByLabel("Port", { exact: true })).toHaveValue("443");
  await expect(page.getByLabel("Role", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Warehouse", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Schema", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Safe Mode", { exact: true })).toHaveValue("readOnly");
  await expect(page.getByLabel("Safe Mode", { exact: true })).toBeDisabled();
  await expect(page.getByLabel("Password", { exact: true })).toHaveCount(0);
  await expect(page.getByLabel("Connect through an SSH bastion")).toHaveCount(0);
  await expect(page.getByText(/system browser for Snowflake sign-in/)).toBeVisible();
});

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
  await expect(page.getByRole("tablist", { name: "Query results" }).getByRole("tab")).toHaveCount(
    2,
  );
  const resultTabs = page.getByRole("tablist", { name: "Query results" }).getByRole("tab");
  await resultTabs.first().focus();
  await resultTabs.first().press("ArrowLeft");
  await expect(resultTabs.last()).toBeFocused();
  await expect(resultTabs.last()).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".result-footer")).toContainText("1 rows");
  await expect(page.locator(".query-error")).toHaveCount(0);
  await expect(page.locator("canvas").first()).toBeVisible();
  await resultTabs.first().click();
  if (!(await page.locator(".inspector").isVisible()))
    await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
  const box = (await page.locator(".grid-host").boundingBox())!;
  await expect(async () => {
    await page.mouse.click(box.x + 90, box.y + 55);
    await expect(page.locator(".inspector-value")).toContainText("9223372036854775807", {
      timeout: 500,
    });
  }).toPass();
  // Retried cell selection can open the read-only value viewer. Return to
  // the grid before asserting its keyboard navigation.
  if (await page.locator(".gdg-clip-region").isVisible()) {
    await page.keyboard.press("Escape");
    await expect(page.locator(".gdg-clip-region")).toHaveCount(0);
  }
  await expect(page.locator(".inspector h3")).toHaveText("exact_integer");
  await page.keyboard.press("ArrowRight");
  await expect(page.locator(".inspector h3")).toHaveText("absent");
  await expect(page.locator(".inspector-value")).toHaveText("NULL");
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
  if (!(await page.locator(".inspector").isVisible()))
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

test("repaints the frozen primary-key column when pages load after scrolling", async ({ page }) => {
  await connect(page, "Frozen key fixture");
  await page.getByRole("button", { name: /^public/ }).click();
  await page.getByRole("button", { name: "orders", exact: true }).click();
  const footer = page.locator(".result-footer");
  await expect(footer).toContainText("500 rows loaded");
  const grid = page.locator(".grid-host");
  const box = (await grid.boundingBox())!;
  // Scrolling reports a visible region that starts after the frozen column;
  // pages that arrive afterwards are repainted from that region.
  await page.mouse.move(box.x + 400, box.y + 200);
  const loadedRows = async () => {
    const text = (await footer.textContent()) ?? "";
    const match = text.match(/([\d,]+)\s+rows loaded/);
    return match ? Number(match[1].replaceAll(",", "")) : 0;
  };
  for (let i = 0; i < 8 && (await loadedRows()) < 2500; i++) {
    const before = await loadedRows();
    await page.mouse.wheel(0, 20000);
    await expect.poll(loadedRows, { timeout: 5000 }).toBeGreaterThan(before);
  }
  await expect.poll(loadedRows, { timeout: 10000 }).toBeGreaterThanOrEqual(2500);
  // Rows whose status cell has text but whose frozen id cell has none.
  const unpainted = () =>
    grid
      .locator("canvas")
      .first()
      .evaluate((canvas: HTMLCanvasElement) => {
        const ratio = canvas.width / canvas.getBoundingClientRect().width;
        const context = canvas.getContext("2d")!;
        const height = canvas.getBoundingClientRect().height;
        const ink = (x: number, y: number, width: number) => {
          const data = context.getImageData(x * ratio, y * ratio, width * ratio, 20 * ratio).data;
          const [r, g, b] = [data[0], data[1], data[2]];
          let count = 0;
          for (let i = 0; i < data.length; i += 4)
            if (Math.abs(data[i] - r) + Math.abs(data[i + 1] - g) + Math.abs(data[i + 2] - b) > 120)
              count++;
          return count;
        };
        let missing = 0;
        // 44 px row markers, 37 px header, 36 px rows; id and customer_id are
        // 110 px each, so status starts at 264 px.
        for (let y = 37 + 8; y + 28 < height; y += 36)
          if (ink(264 + 6, y, 90) > 10 && ink(44 + 6, y, 90) < 3) missing++;
        return missing;
      });
  await expect.poll(unpainted, { timeout: 5000 }).toBe(0);
});
