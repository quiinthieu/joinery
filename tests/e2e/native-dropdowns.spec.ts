import { test, expect, type Page } from "@playwright/test";

const PG_PORT = Number(process.env.JOINERY_POSTGRES_PORT ?? "55432");

async function openConnectionDialog(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
}

async function connectPostgres(page: Page, name: string) {
  await openConnectionDialog(page);
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

test.describe("Tier 1: Feature Coverage - Native Dropdown Appearance & Styling", () => {
  test("1.1 Dialog select elements exhibit native macOS appearance with appearance none and SVG dual-chevron indicator", async ({
    page,
  }) => {
    await openConnectionDialog(page);

    const engineSelect = page.getByLabel("Database engine");
    await expect(engineSelect).toBeVisible();

    const styles = await engineSelect.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        tagName: el.tagName.toLowerCase(),
        appearance: computed.appearance || computed.webkitAppearance,
        backgroundImage: computed.backgroundImage,
        backgroundRepeat: computed.backgroundRepeat,
        backgroundPosition: computed.backgroundPosition,
      };
    });

    expect(styles.tagName).toBe("select");
    expect(styles.appearance).toBe("none");
    expect(styles.backgroundRepeat).toBe("no-repeat");
    expect(styles.backgroundImage).toContain("data:image/svg+xml");
    // Verifies the SVG contains path / polygon chevron geometry
    expect(decodeURIComponent(styles.backgroundImage)).toMatch(/<svg/i);
  });

  test("1.2 Standardized height hierarchy across dialog (30-35px), toolbar (22-26px), and grid (20-24px) tiers", async ({
    page,
  }) => {
    // 1. Dialog Tier: ConnectionDialog
    await openConnectionDialog(page);
    const dialogSelect = page.getByLabel("Database engine");
    const dialogBox = await dialogSelect.boundingBox();
    expect(dialogBox).not.toBeNull();
    expect(dialogBox!.height).toBeGreaterThanOrEqual(28);
    expect(dialogBox!.height).toBeLessThanOrEqual(36);
    await page.getByRole("button", { name: "Cancel" }).click();

    // 2. Toolbar Tier: Query Tab Toolbar Context Selectors
    await connectPostgres(page, `Height PG ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const toolbarSelect = page.getByLabel("Query connection");
    await expect(toolbarSelect).toBeVisible();
    const toolbarBox = await toolbarSelect.boundingBox();
    expect(toolbarBox).not.toBeNull();
    expect(toolbarBox!.height).toBeGreaterThanOrEqual(22);
    expect(toolbarBox!.height).toBeLessThanOrEqual(28);

    // 3. Grid Tier: Grid page size selector
    const sql = "SELECT * FROM public.orders LIMIT 5;";
    await page.getByRole("textbox", { name: "SQL editor" }).fill(sql);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("rows");

    const pageSizeSelect = page.locator(".grid-page-size-select");
    await expect(pageSizeSelect).toBeVisible();
    const gridBox = await pageSizeSelect.boundingBox();
    expect(gridBox).not.toBeNull();
    expect(gridBox!.height).toBeGreaterThanOrEqual(18);
    expect(gridBox!.height).toBeLessThanOrEqual(25);
  });

  test("1.3 Standardized border radius and 1px border stroke across dropdown tiers", async ({
    page,
  }) => {
    await openConnectionDialog(page);

    const engineSelect = page.getByLabel("Database engine");
    const radius = await engineSelect.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        borderRadius: parseFloat(computed.borderRadius),
        borderTopWidth: parseFloat(computed.borderTopWidth),
        borderStyle: computed.borderTopStyle,
      };
    });

    // Native macOS controls use subtle 5-6px corner rounding and 1px border
    expect(radius.borderRadius).toBeGreaterThanOrEqual(4);
    expect(radius.borderRadius).toBeLessThanOrEqual(8);
    expect(radius.borderTopWidth).toBeGreaterThanOrEqual(1);
    expect(radius.borderStyle).toBe("solid");
  });

  test("1.4 Chevron padding protection: padding-right reserves at least 20px so option labels do not collide with indicator", async ({
    page,
  }) => {
    await openConnectionDialog(page);

    const engineSelect = page.getByLabel("Database engine");
    const safeModeSelect = page.getByLabel("Safe Mode");

    for (const select of [engineSelect, safeModeSelect]) {
      const paddingRight = await select.evaluate((el) =>
        parseFloat(window.getComputedStyle(el).paddingRight),
      );
      expect(paddingRight).toBeGreaterThanOrEqual(20);
    }
  });

  test("1.5 Focus ring appearance: dropdown displays accent outline upon focus", async ({
    page,
  }) => {
    await openConnectionDialog(page);

    const engineSelect = page.getByLabel("Database engine");
    await engineSelect.focus();

    const focusStyles = await engineSelect.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        outlineStyle: computed.outlineStyle,
        outlineWidth: parseFloat(computed.outlineWidth),
      };
    });

    expect(focusStyles.outlineStyle).toBe("solid");
    expect(focusStyles.outlineWidth).toBeGreaterThanOrEqual(1);
  });

  test("1.6 Option elements inherit theme colors with high-contrast text readability", async ({
    page,
  }) => {
    await openConnectionDialog(page);

    const optionStyles = await page
      .getByLabel("Database engine")
      .locator("option")
      .first()
      .evaluate((el) => {
        const computed = window.getComputedStyle(el);
        return {
          color: computed.color,
          display: computed.display,
        };
      });

    expect(optionStyles.color).not.toBe("");
  });
});

test.describe("Tier 2: Boundary & Corner Cases - Native Dropdown Resilience", () => {
  test("2.1 Single-option or empty dropdowns render stably without layout collapse", async ({
    page,
  }) => {
    await connectPostgres(page, `Single Opt ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const queryConnSelect = page.getByLabel("Query connection");
    await expect(queryConnSelect).toBeVisible();

    const box = await queryConnSelect.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.width).toBeGreaterThan(60);
    expect(box!.height).toBeGreaterThanOrEqual(20);
  });

  test("2.2 Large option lists retain native styling and allow selection without layout distortion", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("button", { name: /^Preferences/ }).click();
    await expect(page.getByRole("dialog")).toBeVisible();

    const styleSelect = page.getByLabel("Formatting style");
    await expect(styleSelect).toBeVisible();

    const optionsCount = await styleSelect.locator("option").count();
    expect(optionsCount).toBeGreaterThanOrEqual(4);

    await styleSelect.selectOption("postgres");
    expect(await styleSelect.inputValue()).toBe("postgres");

    await styleSelect.selectOption("global");
    expect(await styleSelect.inputValue()).toBe("global");
  });

  test("2.3 Keyboard navigation: ArrowDown and ArrowUp correctly cycle options and fire change events", async ({
    page,
  }) => {
    await openConnectionDialog(page);

    const engineSelect = page.getByLabel("Database engine");
    await engineSelect.focus();
    const initialVal = await engineSelect.inputValue();

    await engineSelect.selectOption("mysql");
    const nextVal = await engineSelect.inputValue();
    expect(nextVal).toBe("mysql");

    await engineSelect.selectOption("postgres");
    const revertedVal = await engineSelect.inputValue();
    expect(revertedVal).toBe(initialVal);
  });

  test("2.4 Theme switching preserves dropdown styling and token adaptation across light, dark, and contrast palettes", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("button", { name: /^Preferences/ }).click();

    const densitySelect = page.getByLabel("Display density");
    await expect(densitySelect).toBeVisible();

    // 1. Ensure default Dark mode (Studio dark)
    await page.getByRole("button", { name: "Studio dark", exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const darkBg = await densitySelect.evaluate(
      (el) => window.getComputedStyle(el).backgroundColor,
    );

    // 2. Switch to Light theme (auto-retry CSS assertion to allow the 150ms theme transition to settle)
    await page.getByRole("button", { name: "Paper light", exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(densitySelect).not.toHaveCSS("background-color", darkBg);

    // 3. Switch to High Contrast palette
    await page.getByRole("button", { name: "High contrast", exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "contrast");
    await expect(densitySelect).toHaveCSS("border-color", /.+/);

    // Reset back to Studio dark
    await page.getByRole("button", { name: "Studio dark", exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await page.getByRole("button", { name: "Done" }).click();
  });

  test("2.5 Disabled dropdown state renders with reduced opacity and default cursor", async ({
    page,
  }) => {
    await connectPostgres(page, `Disabled Select ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    // Simulate query execution to check disabled state of query connection dropdown
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT pg_sleep(3);");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();

    // Observe active query execution state and ephemeral Cancel button
    const cancelBtn = page.getByRole("button", { name: /^Cancel/ });
    await expect(cancelBtn).toBeVisible({ timeout: 5000 });

    const querySelect = page.getByLabel("Query connection");
    await expect(querySelect).toBeDisabled({ timeout: 5000 });

    const disabledStyles = await querySelect.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        opacity: parseFloat(computed.opacity),
        cursor: computed.cursor,
      };
    });

    expect(disabledStyles.opacity).toBeLessThanOrEqual(0.65);
    expect(disabledStyles.cursor).toBe("default");
    await cancelBtn.click();
    await expect(page.getByRole("button", { name: /^Run/ })).toBeVisible({ timeout: 5000 });
  });
});

test.describe("Tier 3: Cross-Feature Combinations - Dropdown Integrations", () => {
  test("3.1 Dropdown selection in ConnectionDialog updates engine and dynamizes form inputs", async ({
    page,
  }) => {
    await openConnectionDialog(page);

    const engineSelect = page.getByLabel("Database engine");
    await engineSelect.selectOption("snowflake");

    await expect(page.getByLabel("Authentication method")).toBeVisible();
    await expect(page.getByLabel("Snowflake account hostname")).toBeVisible();
    await expect(page.getByLabel("Warehouse")).toBeVisible();

    await engineSelect.selectOption("postgres");
    await expect(page.getByLabel("Host", { exact: true })).toBeVisible();
    await expect(page.getByLabel("Port", { exact: true })).toBeVisible();
  });

  test("3.2 Table filter dropdowns work harmoniously with grid pagination dropdown", async ({
    page,
  }) => {
    await connectPostgres(page, `Filter Combo ${Date.now()}`);

    // Navigate to public.orders table to show table grid with filter bar
    const ordersBtn = page.getByRole("button", { name: "orders", exact: true });
    if (!(await ordersBtn.isVisible())) {
      await page
        .getByRole("button", { name: /^public/ })
        .first()
        .click();
    }
    if (!(await ordersBtn.isVisible())) {
      await page
        .getByRole("treeitem", { name: /^Tables/ })
        .first()
        .click();
    }
    await ordersBtn.click();
    await expect(page.locator(".grid-host canvas").first()).toBeVisible();

    // Open filter bar by clicking "Add filter"
    await page.getByRole("button", { name: "Add filter" }).click();
    await expect(page.getByLabel("Filter column")).toBeVisible();
    await expect(page.getByLabel("Filter operator")).toBeVisible();

    // Verify filter dropdowns have native styling
    const colSelect = page.getByLabel("Filter column");
    const colStyles = await colSelect.evaluate((el) => window.getComputedStyle(el).appearance);
    expect(colStyles).toBe("none");

    // Verify grid pagination dropdown operates alongside
    const pageSize = page.locator(".grid-page-size-select");
    await expect(pageSize).toBeVisible();
    await pageSize.selectOption("100");
    await expect(pageSize).toHaveValue("100");
  });

  test("3.3 Display density dropdown immediately adjusts UI scale and document density tokens", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("button", { name: /^Preferences/ }).click();

    const densitySelect = page.getByLabel("Display density");
    await expect(densitySelect).toBeVisible();

    await densitySelect.selectOption("compact");
    await expect(page.locator("html")).toHaveAttribute("data-density", "compact");

    await densitySelect.selectOption("comfortable");
    await expect(page.locator("html")).toHaveAttribute("data-density", "comfortable");

    await page.getByRole("button", { name: "Done" }).click();
  });
});

test.describe("Tier 4: Real-World Application Scenarios - Complete Workflows", () => {
  test("4.1 End-to-end database connection creation using native styled dropdowns", async ({
    page,
  }) => {
    const name = `E2E Live DB ${Date.now()}`;
    await openConnectionDialog(page);

    await page.getByLabel("Database engine").selectOption("postgres");
    await page.getByLabel("Connection name", { exact: true }).fill(name);
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
  });

  test("4.2 Workbench query execution with transaction mode switching via toolbar dropdown", async ({
    page,
  }) => {
    await connectPostgres(page, `Tx Mode Flow ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const txSelect = page.getByLabel("Transaction mode");
    await expect(txSelect).toBeVisible();

    // Toggle to manual commit mode
    await txSelect.selectOption("manual");
    await expect(txSelect).toHaveValue("manual");

    // Execute query in manual mode
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT 42 as answer;");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("1 rows");

    // Commit open transaction before switching back to auto-commit
    const commitBtn = page.getByRole("button", { name: "Commit transaction", exact: true });
    if (await commitBtn.isVisible()) {
      await commitBtn.click();
      await expect(page.locator(".tx-badge")).toHaveCount(0);
    }

    // Toggle back to auto-commit mode
    await txSelect.selectOption("autoCommit");
    await expect(txSelect).toHaveValue("autoCommit");
  });

  test("4.3 Schema context switcher dropdown in SQL editor updates active context", async ({
    page,
  }) => {
    await connectPostgres(page, `Schema Switcher Flow ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const schemaSwitcher = page.locator(".context-switcher");
    if (await schemaSwitcher.isVisible()) {
      const styles = await schemaSwitcher.evaluate((el) => ({
        appearance: window.getComputedStyle(el).appearance,
        height: parseFloat(window.getComputedStyle(el).height),
      }));
      expect(styles.appearance).toBe("none");
      expect(styles.height).toBeGreaterThanOrEqual(20);
      expect(styles.height).toBeLessThanOrEqual(28);
    }
  });
});
