import { test, expect } from "@playwright/test";

test("opted-in drafts and preferences recover without reconnecting or executing", async ({
  page,
  request,
}) => {
  const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
  const saved = async () =>
    (
      await (
        await request.post("http://127.0.0.1:1422/rpc/bootstrap", { headers, data: {} })
      ).json()
    ).result.workspace;
  const name = `Draft fixture ${Date.now()}`;
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(process.env.JOINERY_POSTGRES_PORT ?? "55432");
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  const optIn = page.getByLabel(/Restore open query drafts after restart/);
  await expect(optIn).not.toBeChecked();
  await optIn.check();
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab", exact: true }).click();
  const sql = "SELECT 'restored_draft_fixture';";
  await page.getByRole("textbox", { name: "SQL editor" }).fill(sql);
  await page.getByRole("button", { name: /^Preferences/ }).click();
  await page.getByRole("button", { name: "Paper light", exact: true }).click();
  await page.getByLabel("Display density", { exact: true }).selectOption("compact");
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await expect
    .poll(async () => {
      const workspace = await saved();
      return (
        workspace?.theme === "light" &&
        workspace?.density === "compact" &&
        workspace?.drafts.some((draft: { sql: string }) => draft.sql === sql)
      );
    })
    .toBe(true);
  await page.getByRole("button", { name, exact: true }).hover();
  await page.getByRole("button", { name: `Disconnect ${name}`, exact: true }).click();
  const forbidden: string[] = [];
  const record = (req: { url(): string }) => {
    if (/\/rpc\/(connect|execute_plan|prepare_execution)$/.test(req.url()))
      forbidden.push(req.url());
  };
  page.on("request", record);
  await page.reload();
  await expect(page.getByRole("textbox", { name: "SQL editor" })).toHaveText(sql);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.locator("html")).toHaveAttribute("data-density", "compact");
  await expect(page.locator(".status-bar")).toContainText("disconnected");
  await expect(page.locator(".result-footer")).toContainText("0 rows");
  expect(forbidden).toEqual([]);
  page.off("request", record);
  await page.getByRole("button", { name, exact: true }).hover();
  await page.getByRole("button", { name: `Edit ${name}`, exact: true }).click();
  await page.getByLabel(/Restore open query drafts after restart/).uncheck();
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect.poll(async () => (await saved())?.drafts.length).toBe(0);
  await page.reload();
  await expect(page.getByRole("tablist", { name: "Workspaces" }).getByRole("tab")).toHaveCount(0);
});

test("custom shortcuts, pinned tabs, close other/reopen tab, and layout state persist across reload", async ({
  page,
  request,
}) => {
  const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
  const saved = async () =>
    (
      await (
        await request.post("http://127.0.0.1:1422/rpc/bootstrap", { headers, data: {} })
      ).json()
    ).result.workspace;
  const name = `Layout & Shortcuts ${Date.now()}`;
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(process.env.JOINERY_POSTGRES_PORT ?? "55432");
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel(/Restore open query drafts after restart/).check();
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // Open three query tabs, pin the first one, and close others from the third
  await page.getByRole("button", { name: "New query tab", exact: true }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill("SELECT 'pinned_tab';");
  await page
    .getByRole("button", { name: /^Pin Query / })
    .first()
    .click();

  await page.getByRole("button", { name: "New query tab", exact: true }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill("SELECT 'ephemeral_tab';");
  await page.getByRole("button", { name: "New query tab", exact: true }).click();
  await page.getByRole("textbox", { name: "SQL editor" }).fill("SELECT 'active_kept_tab';");

  // Close other tabs -> keeps pinned tab and active tab, closes ephemeral_tab
  await page.getByRole("button", { name: "Close other tabs", exact: true }).click();
  const tabs = page.getByRole("tablist", { name: "Workspaces" }).getByRole("tab");
  await expect(tabs).toHaveCount(2);

  // Reopen closed tab -> brings back ephemeral_tab
  await page.getByRole("button", { name: "Reopen closed tab", exact: true }).click();
  await expect(tabs).toHaveCount(3);
  await expect(page.getByRole("textbox", { name: "SQL editor" })).toContainText("ephemeral_tab");

  // Configure custom shortcut in Preferences and verify conflict detection
  await page.getByRole("button", { name: /^Preferences/ }).click();
  const paletteShortcutInput = page.getByLabel("Shortcut for Open command palette", {
    exact: true,
  });
  await paletteShortcutInput.fill("Mod+T");
  await paletteShortcutInput.press("Enter");
  await expect(page.getByRole("alert")).toContainText('already assigned to "New query tab"');

  await paletteShortcutInput.fill("Mod+J");
  await paletteShortcutInput.press("Enter");
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await expect(page.locator(".command-trigger kbd")).toHaveText("⌘ J");

  // Collapse sidebar and ensure inspector is visible, then wait for workspace persistence
  const inspectorToggle = page.getByRole("button", { name: "Toggle inspector", exact: true });
  if ((await inspectorToggle.getAttribute("aria-expanded")) !== "true") {
    await inspectorToggle.click();
  }
  const sidebarToggle = page.getByRole("button", { name: "Toggle sidebar", exact: true });
  if ((await sidebarToggle.getAttribute("aria-expanded")) !== "false") {
    await sidebarToggle.click();
  }
  await expect(page.locator(".app-shell")).toHaveClass(/sidebar-hidden/);

  await expect
    .poll(async () => {
      const workspace = await saved();
      return (
        workspace?.sidebarVisible === false &&
        workspace?.sidebarCollapsed === true &&
        workspace?.inspectorVisible === true &&
        workspace?.shortcuts?.openPalette === "Mod+J" &&
        workspace?.drafts?.some((d: { pinned?: boolean }) => d.pinned === true)
      );
    })
    .toBe(true);

  await sidebarToggle.click();
  await expect(page.locator(".app-shell")).not.toHaveClass(/sidebar-hidden/);
  await page.getByRole("button", { name, exact: true }).hover();
  await page.getByRole("button", { name: `Disconnect ${name}`, exact: true }).click();
  await sidebarToggle.click();
  await expect(page.locator(".app-shell")).toHaveClass(/sidebar-hidden/);
  await expect
    .poll(async () => {
      const workspace = await saved();
      return workspace?.sidebarVisible === false && workspace?.sidebarCollapsed === true;
    })
    .toBe(true);

  await page.reload();
  await expect(page.locator(".app-shell")).toHaveClass(/sidebar-hidden/);
  await expect(page.getByLabel("Selected cell inspector")).toBeVisible();
  await expect(page.locator(".command-trigger kbd")).toHaveText("⌘ J");
  await expect(page.getByRole("button", { name: /^Unpin Query / })).toHaveCount(1);

  // Restore sidebar and clean up drafts/shortcuts for subsequent tests
  await page.getByRole("button", { name: "Toggle sidebar", exact: true }).click();
  await page.getByRole("button", { name: /^Preferences/ }).click();
  await page.getByRole("button", { name: "Reset all shortcuts", exact: true }).click();
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page.getByRole("button", { name, exact: true }).hover();
  await page.getByRole("button", { name: `Edit ${name}`, exact: true }).click();
  await page.getByLabel(/Restore open query drafts after restart/).uncheck();
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});
