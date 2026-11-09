import { createHash } from "node:crypto";
import { test, expect, type Page } from "@playwright/test";

async function rpc<T>(page: Page, command: string, args: Record<string, unknown> = {}): Promise<T> {
  const response = await page.request.post(`http://127.0.0.1:1422/rpc/${command}`, {
    headers: { Authorization: "Bearer joinery-disposable-browser-tests-only" },
    data: args,
  });
  expect(response.ok()).toBeTruthy();
  const payload = await response.json();
  expect(payload.error).toBeUndefined();
  return payload.result as T;
}

async function storagePreference(page: Page, enabled: boolean) {
  await page.getByRole("button", { name: /^Preferences/ }).click();
  const dialog = page.getByRole("dialog", { name: "Preferences" });
  const checkbox = dialog.getByRole("checkbox", {
    name: "Temporary disk storage for large results",
  });
  // This preference remains unchanged until its asynchronous save succeeds.
  if ((await checkbox.isChecked()) !== enabled) await checkbox.click();
  if (enabled) await expect(checkbox).toBeChecked();
  else await expect(checkbox).not.toBeChecked();
  await expect(checkbox).toBeEnabled();
  await dialog.getByRole("button", { name: "Done", exact: true }).click();
}

test("opt-in large results reach the last row and export the complete retained snapshot", async ({
  page,
}) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/");
  try {
    await storagePreference(page, true);
    await page.reload();
    await page.getByRole("button", { name: /^Preferences/ }).click();
    const prefs = page.getByRole("dialog", { name: "Preferences" });
    await expect(
      prefs.getByRole("checkbox", { name: "Temporary disk storage for large results" }),
    ).toBeChecked();
    await prefs.getByRole("button", { name: "Done", exact: true }).click();
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Connection name", { exact: true }).fill("Million row fixture");
    await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
    await page
      .getByLabel("Port", { exact: true })
      .fill(process.env.JOINERY_POSTGRES_PORT ?? "55432");
    await page.getByLabel("Database", { exact: true }).fill("joinery");
    await page.getByLabel("Username", { exact: true }).fill("joinery");
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByLabel("Require TLS and verify the server certificate").uncheck();
    await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: "New query tab" }).click();
    await page
      .getByRole("textbox", { name: "SQL editor" })
      .fill("SELECT n AS id, repeat('x',96) AS payload FROM generate_series(1,1000000) n");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    const authorize = page.getByRole("button", { name: "Authorize and run", exact: true });
    await expect
      .poll(
        async () =>
          (await authorize.isVisible()) ||
          (await page.locator(".result-footer .status-dot.completed").isVisible()),
        { timeout: 120_000 },
      )
      .toBe(true);
    if (await authorize.isVisible()) await authorize.click();
    await expect(page.locator(".result-footer")).toContainText("1,000,000 rows", {
      timeout: 120_000,
    });
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await expect(page.locator(".truncated-label")).toHaveCount(0);
    await expect(page.locator(".status-bar")).toContainText("MiB temporary storage");
    if (!(await page.locator(".inspector").isVisible())) {
      await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
    }
    const grid = page.locator(".grid-host");
    await expect(grid.locator("canvas").first()).toBeVisible();
    const box = (await grid.boundingBox())!;
    await page.mouse.click(box.x + 90, box.y + 55);
    await expect(page.locator(".inspector h3")).toHaveText("id");
    await page.keyboard.press("ControlOrMeta+ArrowDown");
    await expect(page.locator(".inspector-value")).toHaveText("1000000");
    await page.keyboard.press("ArrowUp");
    await expect(page.locator(".inspector-value")).toHaveText("999999");
    const frontendBytes = await page.evaluate(async () => {
      const modulePath = "/src/data/result-cache.ts";
      const module = await import(modulePath);
      return module.resultCache.retainedBytes;
    });
    expect(frontendBytes).toBeGreaterThan(0);
    expect(frontendBytes).toBeLessThanOrEqual(32 * 1024 * 1024);
    // Disabling affects newly created results; the existing snapshot remains usable.
    await storagePreference(page, false);
    await rpc(page, "test_queue_save_file", { fileName: "million-browser.csv" });
    await page.getByRole("button", { name: "Export…", exact: true }).click();
    const exportDialog = page.getByRole("dialog", { name: "Export data" });
    await exportDialog.getByRole("button", { name: "Choose destination & export" }).click();
    await expect(exportDialog).toContainText("Exported million-browser.csv", { timeout: 120_000 });
    await expect(exportDialog).toContainText("1,000,000 rows");
    const summary = await rpc<{ bytes: number; lineBreaks: number; sha256: string }>(
      page,
      "test_sandbox_file_summary",
      { fileName: "million-browser.csv" },
    );
    const hash = createHash("sha256");
    hash.update("id,payload");
    const payload = "x".repeat(96);
    // The CSV writer separates records with LF and leaves the final record
    // unterminated. Hash every byte, including that established format choice.
    for (let n = 1; n <= 1_000_000; n++) hash.update(`\n${n},${payload}`);
    expect(summary.sha256).toBe(hash.digest("hex"));
    expect(summary.lineBreaks).toBe(1_000_000);
    expect(summary.bytes).toBeGreaterThan(64 * 1024 * 1024);
    await exportDialog.getByRole("button", { name: "Done", exact: true }).click();
    await page
      .getByRole("button", { name: /^Close Query/, exact: false })
      .last()
      .click();
    await expect
      .poll(
        async () =>
          (await rpc<{ temporaryResultBytes: number }>(page, "resource_usage"))
            .temporaryResultBytes,
      )
      .toBe(0);
    expect(errors).toEqual([]);
  } finally {
    await rpc(page, "set_temporary_result_storage", { enabled: false });
  }
});
