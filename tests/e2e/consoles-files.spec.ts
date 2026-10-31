import { expect, test, type Page } from "@playwright/test";

const BRIDGE_URL = "http://127.0.0.1:1422";
const BRIDGE_TOKEN = "joinery-disposable-browser-tests-only";
const PG_PORT = Number(process.env.JOINERY_POSTGRES_PORT ?? "55432");

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

async function connectSqlEngine(
  page: Page,
  opts: {
    name: string;
    engine: "postgres" | "mysql";
    port: number;
    safeMode?: "readOnly" | "confirm" | "silent";
    storeQueryText?: boolean;
  },
) {
  const welcomeButton = page.getByRole("button", { name: "Connect a database" });
  if (await welcomeButton.isVisible()) {
    await welcomeButton.click();
  } else {
    await page.getByRole("button", { name: "New connection", exact: true }).click();
  }
  await page.getByLabel("Database engine").selectOption(opts.engine);
  await page.getByLabel("Connection name", { exact: true }).fill(opts.name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(String(opts.port));
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption(opts.safeMode ?? "silent");
  if (opts.storeQueryText) {
    await page.getByLabel(/Save SQL text in local history/).check();
  }
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".connection-row.selected")).toContainText(opts.name);
  await expect(page.locator(".connection-row.selected .online-dot")).toHaveCount(1);
}

async function runSqlAndReadFirstCell(
  page: Page,
  sql: string,
  expectedCell?: string,
  expectedStatements = 1,
) {
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  const results = page.locator(".result-tabs [role=tab]");
  const previous = (await results.count()) ? await results.last().getAttribute("id") : null;
  await editor.fill(sql);
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect
    .poll(async () => {
      const id = (await results.count()) ? await results.last().getAttribute("id") : null;
      return Boolean(id && id !== previous);
    })
    .toBe(true);
  await expect(results).toHaveCount(expectedStatements);
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await expect(page.locator(".query-error")).toHaveCount(0);
  if (expectedCell !== undefined) {
    await results.last().click();
    await expect(page.locator(".result-footer")).toContainText("1 rows");
    if (!(await page.locator(".inspector").isVisible())) {
      await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
    }
    const box = (await page.locator(".grid-host").boundingBox())!;
    await expect(async () => {
      await page.mouse.click(box.x + 90, box.y + 55);
      await expect(page.locator(".inspector-value")).toContainText(expectedCell, { timeout: 500 });
    }).toPass();
  }
}

for (const [engineLabel, engineKind, port] of [
  ["PostgreSQL", "postgres", PG_PORT],
  ["MySQL", "mysql", 53306],
] as const) {
  test(`${engineLabel} named sessions share manual transactions, isolate grid commits, and clean up on last close`, async ({
    page,
  }) => {
    test.setTimeout(90000);
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await harnessRpc(page, "disconnect_fixtures");
    await page.goto("/");
    await connectSqlEngine(page, {
      name: `${engineLabel} R4 Tx`,
      engine: engineKind,
      port,
      safeMode: "silent",
    });

    await page.getByRole("button", { name: "New query tab", exact: true }).click();
    const sharedTabId = (await page.getByRole("tab", { selected: true }).getAttribute("id"))!;

    try {
      await runSqlAndReadFirstCell(
        page,
        "DROP TABLE IF EXISTS r4_tx_items; " +
          "DROP TABLE IF EXISTS r4_grid_side; " +
          "CREATE TABLE r4_tx_items (id INT PRIMARY KEY, note VARCHAR(64) NOT NULL); " +
          "CREATE TABLE r4_grid_side (id INT PRIMARY KEY, label VARCHAR(64) NOT NULL); " +
          "SELECT 'ready'",
        "ready",
        5,
      );

      // Refresh explorer catalog so r4_grid_side is browsable as a table tab.
      await page.getByRole("button", { name: "Refresh schema", exact: true }).click();

      // Open Console 2, sign into a new secondary session, and rename it "Tx lane".
      await page.getByRole("button", { name: "New query tab", exact: true }).click();
      const txTabAId = (await page.getByRole("tab", { selected: true }).getAttribute("id"))!;
      await page.getByRole("button", { name: "New isolated session", exact: true }).click();
      await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
      await page.getByRole("button", { name: "Connect", exact: true }).click();
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect
        .poll(
          async () =>
            (await harnessRpc<{ sessionCount: number }>(page, "resource_usage")).sessionCount,
        )
        .toBe(2);

      await page.getByRole("button", { name: "Rename console session", exact: true }).click();
      const renameInput = page.getByLabel("Console session name", { exact: true });
      await renameInput.fill("Tx lane");
      await renameInput.press("Enter");
      await expect(page.locator(".query-session")).toHaveText("Isolated session · Tx lane");

      // Open Console 3 and attach it to the same "Tx lane" session without opening a 3rd backend session.
      await page.getByRole("button", { name: "New query tab", exact: true }).click();
      const txTabBId = (await page.getByRole("tab", { selected: true }).getAttribute("id"))!;
      await page.getByLabel("Console session", { exact: true }).selectOption({ label: "Tx lane" });
      await expect(page.locator(".query-session")).toHaveText("Isolated session · Tx lane");
      await expect
        .poll(
          async () =>
            (await harnessRpc<{ sessionCount: number }>(page, "resource_usage")).sessionCount,
        )
        .toBe(2);

      // Switch "Tx lane" to Manual commit mode on Console 2 and write an uncommitted row.
      await page.locator(`#${txTabAId}`).click();
      await page.getByLabel("Transaction mode", { exact: true }).selectOption("manual");
      await expect(page.getByLabel("Transaction mode", { exact: true })).toHaveValue("manual");

      await runSqlAndReadFirstCell(
        page,
        "INSERT INTO r4_tx_items (id, note) VALUES (1, 'uncommitted_lane'); SELECT note FROM r4_tx_items WHERE id = 1",
        "uncommitted_lane",
        2,
      );
      await expect(page.locator(".tx-badge")).toHaveText("In transaction");

      // Console 3 (attached to "Tx lane") immediately sees both the badge and the uncommitted row.
      await page.locator(`#${txTabBId}`).click();
      await expect(page.locator(".tx-badge")).toHaveText("In transaction");
      await runSqlAndReadFirstCell(page, "SELECT COUNT(*) FROM r4_tx_items", "1");

      // Console 1 ("Shared session") sees 0 rows and remains idle.
      await page.locator(`#${sharedTabId}`).click();
      await expect(page.locator(".tx-badge")).toHaveCount(0);
      await runSqlAndReadFirstCell(page, "SELECT COUNT(*) FROM r4_tx_items", "0");

      // Switching Manual -> Auto-commit while In transaction is rejected with a toast and stays Manual.
      await page.locator(`#${txTabAId}`).click();
      await page.getByLabel("Transaction mode", { exact: true }).selectOption("autoCommit");
      await expect(page.locator(".toast")).toContainText("Commit or roll back");
      await expect(page.getByLabel("Transaction mode", { exact: true })).toHaveValue("manual");

      // Commit a table-grid insert on r4_grid_side while "Tx lane" stays uncommitted.
      const schemaPattern = engineKind === "postgres" ? /^public/ : /^joinery/;
      const tableBtn = page.getByRole("button", { name: "r4_grid_side", exact: true });
      if (!(await tableBtn.isVisible())) {
        await page.getByRole("button", { name: schemaPattern }).first().click();
      }
      if (!(await tableBtn.isVisible())) {
        await page
          .getByRole("treeitem", { name: /^Tables/ })
          .first()
          .click();
      }
      await tableBtn.click();
      await expect(page.locator(".grid-host canvas").first()).toBeVisible();
      await page.getByRole("button", { name: "Insert row", exact: true }).click();
      await dispatchPaste(page, "101\tGridIndependent");
      await page.getByRole("button", { name: "Review 1 edit", exact: true }).click();
      const review = page.getByRole("dialog", { name: "Review pending changes" });
      await review.getByRole("button", { name: "Apply changes", exact: true }).click();
      await expect(review).toHaveCount(0);

      // Verify "Tx lane" is still In transaction and its row is still uncommitted on Shared session!
      await page.locator(`#${txTabAId}`).click();
      await expect(page.locator(".tx-badge")).toHaveText("In transaction");
      await page.locator(`#${sharedTabId}`).click();
      await runSqlAndReadFirstCell(page, "SELECT COUNT(*) FROM r4_tx_items", "0");
      await runSqlAndReadFirstCell(
        page,
        "SELECT label FROM r4_grid_side WHERE id = 101",
        "GridIndependent",
      );

      // Roll back "Tx lane" and verify the uncommitted row disappears.
      await page.locator(`#${txTabAId}`).click();
      await page.getByRole("button", { name: "Rollback transaction", exact: true }).click();
      await expect(page.locator(".tx-badge")).toHaveCount(0);
      await runSqlAndReadFirstCell(page, "SELECT COUNT(*) FROM r4_tx_items", "0");

      // Insert a new row in Manual mode, explicitly Commit, and verify Shared session sees it.
      await runSqlAndReadFirstCell(
        page,
        "INSERT INTO r4_tx_items (id, note) VALUES (2, 'committed_lane'); SELECT COUNT(*) FROM r4_tx_items",
        "1",
        2,
      );
      await expect(page.locator(".tx-badge")).toHaveText("In transaction");
      await page.getByRole("button", { name: "Commit transaction", exact: true }).click();
      await expect(page.locator(".tx-badge")).toHaveCount(0);

      await page.locator(`#${sharedTabId}`).click();
      await runSqlAndReadFirstCell(
        page,
        "SELECT note FROM r4_tx_items WHERE id = 2",
        "committed_lane",
      );

      // Closing Console 2 leaves "Tx lane" alive because Console 3 still owns it.
      await page
        .locator(`#${txTabAId}`)
        .locator("..")
        .getByRole("button", { name: /^Close / })
        .click();
      await expect(page.locator(`#${txTabAId}`)).toHaveCount(0);
      await expect
        .poll(
          async () =>
            (await harnessRpc<{ sessionCount: number }>(page, "resource_usage")).sessionCount,
        )
        .toBe(2);

      // Closing Console 3 (the last attached console) prompts and disconnects "Tx lane".
      await page
        .locator(`#${txTabBId}`)
        .locator("..")
        .getByRole("button", { name: /^Close / })
        .click();
      await expect(page.getByRole("dialog")).toContainText("transactions");
      await page.getByRole("button", { name: "Continue", exact: true }).click();
      await expect(page.locator(`#${txTabBId}`)).toHaveCount(0);
      await expect
        .poll(
          async () =>
            (await harnessRpc<{ sessionCount: number }>(page, "resource_usage")).sessionCount,
        )
        .toBe(1);
    } finally {
      await page.locator(`#${sharedTabId}`).click();
      const editor = page.getByRole("textbox", { name: "SQL editor" });
      await editor.fill("DROP TABLE IF EXISTS r4_tx_items; DROP TABLE IF EXISTS r4_grid_side;");
      await page.getByRole("button", { name: "Run all statements", exact: true }).click();
      await expect(page.locator(".status-dot.running")).toHaveCount(0);
      await harnessRpc(page, "disconnect_fixtures");
    }

    expect(errors).toEqual([]);
  });
}

test("active schema and database switcher isolates per-session search_path and live completion context", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));

  await harnessRpc(page, "disconnect_fixtures");
  await page.goto("/");
  await connectSqlEngine(page, {
    name: "PG Context R4",
    engine: "postgres",
    port: PG_PORT,
    safeMode: "silent",
  });

  await page.getByRole("button", { name: "New query tab", exact: true }).click();
  const sharedTabId = (await page.getByRole("tab", { selected: true }).getAttribute("id"))!;

  try {
    await runSqlAndReadFirstCell(
      page,
      "DROP SCHEMA IF EXISTS r4_ctx_schema CASCADE; " +
        "CREATE SCHEMA r4_ctx_schema; " +
        "CREATE TABLE r4_ctx_schema.r4_ctx_widget (sku TEXT PRIMARY KEY); " +
        "INSERT INTO r4_ctx_schema.r4_ctx_widget VALUES ('W-900'); " +
        "SELECT 'seeded'",
      "seeded",
      5,
    );

    // Open an isolated session on Console 2 so its context list immediately discovers r4_ctx_schema.
    await page.getByRole("button", { name: "New query tab", exact: true }).click();
    const isolatedTabId = (await page.getByRole("tab", { selected: true }).getAttribute("id"))!;
    await page.getByRole("button", { name: "New isolated session", exact: true }).click();
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);

    const schemaSelect = page.getByLabel("Active schema", { exact: true });
    await expect(schemaSelect).toBeVisible();
    await expect(schemaSelect).toHaveValue("public");
    // Ensure .completion-context does not leak <option> text from its sibling <select>.
    await expect(page.locator(".completion-context")).not.toContainText("r4_ctx_schema");

    await schemaSelect.selectOption("r4_ctx_schema");
    await expect(schemaSelect).toHaveValue("r4_ctx_schema");
    await expect(page.locator(".completion-context")).toContainText("r4_ctx_schema");

    // Unqualified query on Console 2 resolves r4_ctx_schema.r4_ctx_widget directly.
    await runSqlAndReadFirstCell(page, "SELECT sku FROM r4_ctx_widget", "W-900");

    // Console 1 ("Shared session") remains on public.
    await page.locator(`#${sharedTabId}`).click();
    await expect(page.getByLabel("Active schema", { exact: true })).toHaveValue("public");
    await expect(page.locator(".completion-context")).not.toContainText("r4_ctx_schema");
    await runSqlAndReadFirstCell(page, "SELECT current_schema()", "public");

    // Close Console 2 cleanly before verifying MySQL database switcher label.
    await page
      .locator(`#${isolatedTabId}`)
      .locator("..")
      .getByRole("button", { name: /^Close / })
      .click();
    await page.getByRole("button", { name: "Continue", exact: true }).click();

    // Connect MySQL 8.4 and verify Active database switcher is bound to "joinery".
    await connectSqlEngine(page, {
      name: "MySQL Context R4",
      engine: "mysql",
      port: 53306,
      safeMode: "silent",
    });
    await page.getByRole("button", { name: "New query tab", exact: true }).click();
    const dbSelect = page.getByLabel("Active database", { exact: true });
    await expect(dbSelect).toBeVisible();
    await expect(dbSelect).toHaveValue("joinery");
  } finally {
    await page.locator(`#${sharedTabId}`).click();
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill("DROP SCHEMA IF EXISTS r4_ctx_schema CASCADE;");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".status-dot.running")).toHaveCount(0);
    await harnessRpc(page, "disconnect_fixtures");
  }

  expect(errors).toEqual([]);
});

test("capability-gated SQL files, saved queries library, bookmarks, and recent locations work end to end", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));

  await harnessRpc(page, "disconnect_fixtures");
  await page.goto("/");
  await connectSqlEngine(page, {
    name: "PG Library R4",
    engine: "postgres",
    port: PG_PORT,
    safeMode: "silent",
    storeQueryText: true,
  });

  // 1. Seed a confined .sql file (test_write_sandbox_file also queues it for the next open dialog).
  await harnessRpc(page, "test_write_sandbox_file", {
    fileName: "analyst-r4.sql",
    contents: "SELECT 'from_disk_v1' AS stage;",
  });

  await page.getByRole("button", { name: "New query tab", exact: true }).click();
  await page.getByRole("button", { name: "Open SQL file", exact: true }).click();

  const fileTab = page.getByRole("tab", { selected: true });
  await expect(fileTab).toContainText("analyst-r4.sql");
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  await expect(editor).toContainText("SELECT 'from_disk_v1' AS stage;");
  await expect(fileTab.locator(".dirty-dot")).toHaveCount(0);

  // Edit the SQL file: dirty dot appears, Save button indicates dirty state, and workspace drafts exclude it.
  await editor.fill("SELECT 'from_disk_v2' AS stage;");
  await expect(fileTab.locator(".dirty-dot")).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Save SQL file", exact: true })).toContainText(
    "Save ●",
  );

  // Wait past the 400ms workspace debounce and verify file-backed SQL never leaks into SavedWorkspace.drafts.
  await page.waitForTimeout(550);
  const bootstrapAfterEdit = await harnessRpc<{ workspace: { drafts: { sql: string }[] } }>(
    page,
    "bootstrap",
  );
  expect(
    bootstrapAfterEdit.workspace.drafts.some((draft) => draft.sql.includes("from_disk_v2")),
  ).toBe(false);

  // Closing a dirty file-backed tab triggers the unsaved-changes confirmation guard.
  const fileTabId = (await fileTab.getAttribute("id"))!;
  await page
    .locator(`#${fileTabId}`)
    .locator("..")
    .getByRole("button", { name: /^Close / })
    .click();
  const dirtyConfirm = page.getByRole("dialog", { name: "Close analyst-r4.sql?" });
  await expect(dirtyConfirm).toContainText("Unsaved changes in analyst-r4.sql");
  await dirtyConfirm.getByRole("button", { name: "Keep working", exact: true }).click();
  await expect(dirtyConfirm).toHaveCount(0);

  // Save in-place reuses GrantKind::SqlFile without opening another file picker.
  await page.getByRole("button", { name: "Save SQL file", exact: true }).click();
  await expect(fileTab.locator(".dirty-dot")).toHaveCount(0);
  const diskV2 = await harnessRpc<{ exists: boolean; contents: string }>(
    page,
    "test_read_sandbox_file",
    { fileName: "analyst-r4.sql" },
  );
  expect(diskV2).toEqual({ exists: true, contents: "SELECT 'from_disk_v2' AS stage;" });

  // Edit again and Save As to a new capability-gated destination.
  await editor.fill("SELECT 'from_disk_v3' AS stage;");
  await expect(fileTab.locator(".dirty-dot")).toHaveCount(1);
  await harnessRpc(page, "test_queue_save_file", { fileName: "analyst-r4-copy.sql" });
  await page.getByRole("button", { name: "Save SQL file as", exact: true }).click();
  await expect(fileTab).toContainText("analyst-r4-copy.sql");
  await expect(fileTab.locator(".dirty-dot")).toHaveCount(0);

  const diskCopy = await harnessRpc<{ exists: boolean; contents: string }>(
    page,
    "test_read_sandbox_file",
    { fileName: "analyst-r4-copy.sql" },
  );
  expect(diskCopy).toEqual({ exists: true, contents: "SELECT 'from_disk_v3' AS stage;" });
  const diskOriginal = await harnessRpc<{ exists: boolean; contents: string }>(
    page,
    "test_read_sandbox_file",
    { fileName: "analyst-r4.sql" },
  );
  expect(diskOriginal).toEqual({ exists: true, contents: "SELECT 'from_disk_v2' AS stage;" });

  // 2. Save a reusable query to the library, bookmark it, close it, and reopen via SearchEverywhere.
  await page.getByRole("button", { name: "New query tab", exact: true }).click();
  await editor.fill("SELECT 'saved_revenue_r4' AS metric;");
  await page.getByRole("button", { name: "Save query to library", exact: true }).click();
  const saveDialog = page.getByRole("dialog", { name: "Save query to library" });
  await saveDialog.getByLabel("Query title", { exact: true }).fill("R4 Revenue Check");
  await saveDialog.getByRole("button", { name: "Save query", exact: true }).click();
  await expect(saveDialog).toHaveCount(0);

  const savedTab = page.getByRole("tab", { selected: true });
  await expect(savedTab).toContainText("R4 Revenue Check");
  await page.getByRole("button", { name: "Bookmark item", exact: true }).click();
  await expect(page.getByRole("button", { name: "Remove bookmark", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  // Close the saved-query tab and reopen it from SearchEverywhere ("Saved" scope).
  const savedTabId = (await savedTab.getAttribute("id"))!;
  await page
    .locator(`#${savedTabId}`)
    .locator("..")
    .getByRole("button", { name: /^Close / })
    .click();
  await expect(page.locator(`#${savedTabId}`)).toHaveCount(0);

  await page.getByRole("button", { name: "Jump to anything" }).click();
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await palette.getByRole("tab", { name: "Saved", exact: true }).click();
  await palette.getByRole("textbox", { name: "Search commands" }).fill("R4 Revenue Check");
  await expect(palette).toContainText("R4 Revenue Check");
  await palette.getByRole("textbox", { name: "Search commands" }).press("Enter");
  await expect(palette).toHaveCount(0);
  await expect(page.getByRole("tab", { selected: true })).toContainText("R4 Revenue Check");
  await expect(editor).toContainText("SELECT 'saved_revenue_r4' AS metric;");

  // 3. Open public.orders table, bookmark it, and verify Bookmarks + Recent locations in LibraryModal.
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
  await page.getByRole("button", { name: "Bookmark item", exact: true }).click();
  await expect(page.getByRole("button", { name: "Remove bookmark", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  await page
    .locator(".sidebar-tools")
    .getByRole("button", { name: /^Bookmarks/ })
    .click();
  const libraryDialog = page.getByRole("dialog", { name: "Bookmarks" });
  await expect(libraryDialog).toContainText("public.orders");
  await expect(libraryDialog).toContainText("R4 Revenue Check");

  await libraryDialog.getByRole("tab", { name: "Recent locations", exact: true }).click();
  const recentsDialog = page.getByRole("dialog", { name: "Recent locations" });
  await expect(recentsDialog).toContainText("public.orders");
  await expect(recentsDialog).toContainText("R4 Revenue Check");
  await expect(recentsDialog).toContainText("analyst-r4-copy.sql");
  await recentsDialog
    .getByRole("button", { name: /R4 Revenue Check/ })
    .first()
    .click();
  await expect(recentsDialog).toHaveCount(0);
  await expect(page.getByRole("tab", { selected: true })).toContainText("R4 Revenue Check");

  // Reload to verify Saved queries & Bookmarks persist while Recent locations stay ephemeral in memory.
  await page.reload();
  await expect(
    page.locator(".sidebar-tools").getByRole("button", { name: /^Saved queries/ }),
  ).toContainText("1");
  await expect(
    page.locator(".sidebar-tools").getByRole("button", { name: /^Bookmarks/ }),
  ).toContainText("2");

  await harnessRpc(page, "disconnect_fixtures");
  expect(errors).toEqual([]);
});
