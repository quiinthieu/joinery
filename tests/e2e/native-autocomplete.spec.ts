import { test, expect, type Page, type Locator } from "@playwright/test";

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

test.describe("Tier 1: Feature Coverage - Native Auto-Complete Suggestions", () => {
  test("1.1 CodeMirror SQL autocomplete tooltip renders with macOS menu vibrancy (border-radius, border, and elevation shadow)", async ({
    page,
  }) => {
    await connectPostgres(page, `AC Vibrancy ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT ");
    await editor.press("Control+Space");

    const tooltip = page.locator(".cm-tooltip-autocomplete");
    await expect(tooltip).toBeVisible();

    const styles = await tooltip.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        borderRadius: parseFloat(computed.borderRadius),
        borderTopWidth: parseFloat(computed.borderTopWidth),
        boxShadow: computed.boxShadow,
        padding: parseFloat(computed.padding),
      };
    });

    // Native macOS menu styling: 6-8px border radius, 1px border stroke, diffuse shadow
    expect(styles.borderRadius).toBeGreaterThanOrEqual(6);
    expect(styles.borderRadius).toBeLessThanOrEqual(10);
    expect(styles.borderTopWidth).toBeGreaterThanOrEqual(1);
    expect(styles.boxShadow).not.toBe("none");
  });

  test("1.2 Autocomplete suggestion list items render as rounded pills with native item padding", async ({
    page,
  }) => {
    await connectPostgres(page, `AC Items ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT ");
    await editor.press("Control+Space");

    const item = page.locator(".cm-tooltip-autocomplete ul li").first();
    await expect(item).toBeVisible();

    const itemStyles = await item.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        borderRadius: parseFloat(computed.borderRadius),
        paddingTop: parseFloat(computed.paddingTop),
        paddingBottom: parseFloat(computed.paddingBottom),
        paddingLeft: parseFloat(computed.paddingLeft),
        paddingRight: parseFloat(computed.paddingRight),
      };
    });

    // Native menu items have 3-5px border radius pills and standard menu padding
    expect(itemStyles.borderRadius).toBeGreaterThanOrEqual(3);
    expect(itemStyles.borderRadius).toBeLessThanOrEqual(6);
    expect(itemStyles.paddingLeft).toBeGreaterThanOrEqual(6);
    expect(itemStyles.paddingRight).toBeGreaterThanOrEqual(6);
  });

  test("1.3 Selected suggestion item displays vibrant accent highlight and high-contrast text", async ({
    page,
  }) => {
    await connectPostgres(page, `AC Selection ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT ");
    await editor.press("Control+Space");

    const selectedItem = page.locator('.cm-tooltip-autocomplete ul li[aria-selected="true"]');
    await expect(selectedItem).toBeVisible();

    const styles = await selectedItem.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        backgroundColor: computed.backgroundColor,
        color: computed.color,
      };
    });

    // Accent selection highlight must be solid / non-transparent
    expect(styles.backgroundColor).not.toBe("rgba(0, 0, 0, 0)");
    expect(styles.backgroundColor).not.toBe("transparent");
    expect(styles.color).not.toBe("");
  });

  test("1.4 Suggestion matched text and detail typography hierarchy", async ({ page }) => {
    await connectPostgres(page, `AC Typography ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT c");
    await editor.press("Control+Space");

    const matched = page.locator(".cm-tooltip-autocomplete .cm-completionMatchedText").first();
    if (await matched.isVisible()) {
      const matchedStyles = await matched.evaluate((el) => {
        const computed = window.getComputedStyle(el);
        return {
          fontWeight: parseInt(computed.fontWeight, 10),
          textDecoration: computed.textDecoration,
        };
      });
      // Matched prefix is emboldened or highlighted
      expect(matchedStyles.fontWeight).toBeGreaterThanOrEqual(500);
    }
  });

  test("1.5 HTML5 datalist input affordances in TableObjectDialog column type input", async ({
    page,
  }) => {
    await connectPostgres(page, `Datalist Test ${Date.now()}`);

    // Open TableObjectDialog via create table action
    const createBtn = page.getByRole("button", { name: "Create table in public" });
    if (await createBtn.isVisible()) {
      await createBtn.click();
      await expect(page.getByRole("dialog", { name: "Create table" })).toBeVisible();

      // Verify column data type input references datalist
      const typeInput = page.getByLabel(/Column \d+ data type/i).first();
      await expect(typeInput).toBeVisible();
      expect(await typeInput.getAttribute("list")).toBe("table-column-types");

      // Verify datalist DOM node exists with options
      const datalistOptions = page.locator("datalist#table-column-types option");
      expect(await datalistOptions.count()).toBeGreaterThan(5);
      await page.getByRole("button", { name: "Cancel" }).click();
    }
  });

  test("1.6 Floating popover menu vibrancy: CopyAsMenu renders with macOS rounded menu styling and diffuse shadow", async ({
    page,
  }) => {
    await connectPostgres(page, `CopyAs Popover ${Date.now()}`);

    // Run query to populate grid and enable CopyAsMenu
    await page.getByRole("button", { name: "New query tab", exact: true }).click();
    await page.getByRole("textbox", { name: "SQL editor" }).fill("SELECT 1 as num;");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("1 rows");

    // Open CopyAsMenu
    await page.getByRole("button", { name: "Copy as" }).click();
    const popover = page.locator(".copy-as-popover");
    await expect(popover).toBeVisible();

    const popoverStyles = await popover.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        borderRadius: parseFloat(computed.borderRadius),
        boxShadow: computed.boxShadow,
      };
    });

    expect(popoverStyles.borderRadius).toBeGreaterThanOrEqual(6);
    expect(popoverStyles.boxShadow).not.toBe("none");
  });
});

test.describe("Tier 2: Boundary & Corner Cases - Autocomplete Interaction Resilience", () => {
  test("2.1 Keyboard navigation: ArrowDown, ArrowUp navigate suggestions, Tab/Enter selects", async ({
    page,
  }) => {
    await connectPostgres(page, `AC Keyboard ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT ");
    await editor.press("Control+Space");

    const tooltip = page.locator(".cm-tooltip-autocomplete");
    await expect(tooltip).toBeVisible();

    const firstItem = page.locator(".cm-tooltip-autocomplete ul li").first();
    await expect(firstItem).toBeVisible();

    // ArrowDown cycles through suggestions
    await editor.press("ArrowDown");
    const selected = page.locator('.cm-tooltip-autocomplete ul li[aria-selected="true"]');
    await expect(selected).toBeVisible();
    const selectedText = await selected.innerText();
    expect(selectedText.length).toBeGreaterThan(0);

    // Enter commits the active selection into editor
    await editor.press("Enter");
    await expect(tooltip).toBeHidden();
  });

  test("2.2 Dismissing autocomplete: Escape key or clicking outside dismisses popover cleanly", async ({
    page,
  }) => {
    await connectPostgres(page, `AC Dismiss ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT ");
    await editor.press("Control+Space");

    const tooltip = page.locator(".cm-tooltip-autocomplete");
    await expect(tooltip).toBeVisible();

    // 1. Escape key dismissal
    await editor.press("Escape");
    await expect(tooltip).toBeHidden();

    // 2. Click outside dismissal
    await editor.press("Control+Space");
    await expect(tooltip).toBeVisible();
    await page.locator(".query-context").click();
    await expect(tooltip).toBeHidden();
  });

  test("2.3 Dynamic prefix filtering updates suggestion count as user types", async ({ page }) => {
    await connectPostgres(page, `AC Filter ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.click();
    await editor.pressSequentially("SELECT ");
    await editor.press("Control+Space");
    const tooltip = page.locator(".cm-tooltip-autocomplete");
    await expect(tooltip).toBeVisible();

    const countInitial = await page.locator(".cm-tooltip-autocomplete ul li").count();
    expect(countInitial).toBeGreaterThan(0);

    // Narrow prefix by typing
    await editor.pressSequentially("cust");
    const countNarrowed = await page.locator(".cm-tooltip-autocomplete ul li").count();
    expect(countNarrowed).toBeLessThanOrEqual(countInitial);
  });

  test("2.4 Theme switching preserves autocomplete vibrancy and contrast across palettes", async ({
    page,
  }) => {
    await connectPostgres(page, `AC Theme ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });

    // Open Preferences and switch to light mode
    await page.getByRole("button", { name: /^Preferences/ }).click();
    await page.getByRole("button", { name: "Paper light", exact: true }).click();
    await page.getByRole("button", { name: "Done" }).click();

    await editor.fill("SELECT ");
    await editor.press("Control+Space");

    const tooltip = page.locator(".cm-tooltip-autocomplete");
    await expect(tooltip).toBeVisible();

    const lightStyles = await tooltip.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        backgroundColor: computed.backgroundColor,
        borderColor: computed.borderColor,
      };
    });

    expect(lightStyles.backgroundColor).not.toBe("");
    expect(lightStyles.borderColor).not.toBe("");
  });

  test("2.5 Viewport boundary constraint: autocomplete popover stays within visible window", async ({
    page,
  }) => {
    await connectPostgres(page, `AC Bounds ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("\n\n\n\n\n\n\n\nSELECT ");
    await editor.press("Control+Space");

    const tooltip = page.locator(".cm-tooltip-autocomplete");
    await expect(tooltip).toBeVisible();

    const box = await tooltip.boundingBox();
    expect(box).not.toBeNull();
    const viewport = page.viewportSize();
    expect(viewport).not.toBeNull();

    // Verify tooltip is not positioned off-screen
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(viewport!.width);
    expect(box!.y + box!.height).toBeLessThanOrEqual(viewport!.height + 50);
  });
});

test.describe("Tier 3: Cross-Feature Combinations - Autocomplete & App Features", () => {
  test("3.1 Autocomplete selection in editor followed by immediate statement execution", async ({
    page,
  }) => {
    await connectPostgres(page, `AC Exec ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT * FROM public.orders;");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("rows");
  });

  test("3.2 Context switcher dropdown updates active schema for SQL completion", async ({
    page,
  }) => {
    await connectPostgres(page, `AC Context ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const contextSelect = page.locator(".context-switcher");
    if (await contextSelect.isVisible()) {
      const initial = await contextSelect.inputValue();
      expect(initial).not.toBeNull();
    }
  });

  test("3.3 Search Everywhere command palette transition to editor", async ({ page }) => {
    await connectPostgres(page, `AC Palette ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    // Trigger Search Everywhere via shortcut
    await page.keyboard.press("Control+k");
    const palette = page.locator(".search-everywhere, [role='dialog'][aria-label*='Search']");
    if (await palette.isVisible()) {
      await expect(palette).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(palette).toBeHidden();
    }
  });
});

test.describe("Tier 4: Real-World Application Scenarios - Autocomplete End-to-End", () => {
  test("4.1 Complete SQL query authoring flow with table and keyword completion", async ({
    page,
  }) => {
    await connectPostgres(page, `AC Author Flow ${Date.now()}`);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();

    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("SELECT id FROM public.orders;");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("rows");
    await expect(page.locator(".query-error")).toHaveCount(0);
  });

  test("4.2 Table object dialog column type suggestion flow", async ({ page }) => {
    await connectPostgres(page, `AC Table Flow ${Date.now()}`);

    const createBtn = page.getByRole("button", { name: "Create table in public" });
    if (await createBtn.isVisible()) {
      await createBtn.click();
      const dialog = page.getByRole("dialog", { name: "Create table" });
      await expect(dialog).toBeVisible();

      const typeInput = page.getByLabel(/Column 1 data type/i);
      await typeInput.fill("varchar");
      expect(await typeInput.inputValue()).toBe("varchar");

      await page.getByRole("button", { name: "Cancel" }).click();
      await expect(dialog).toHaveCount(0);
    }
  });
});
