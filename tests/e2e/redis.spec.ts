import { test, expect } from "@playwright/test";

test("Redis connects, scans keys, inspects typed values and reviews a byte-safe edit", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const prefix = `redis-e2e-${Date.now()}`;
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Connection name", { exact: true }).fill("Redis fixture");
  await page.getByLabel("Database engine").selectOption("redis");
  await expect(page.getByLabel("Port", { exact: true })).toHaveValue("6379");
  await expect(page.getByLabel("Database index", { exact: true })).toHaveValue("0");
  await expect(page.getByLabel("Username", { exact: true })).toHaveValue("default");
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill("56379");
  await page.getByLabel("Password", { exact: true }).fill("joinery-redis-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  const editor = page.getByRole("textbox", { name: "Redis command editor" });
  await expect(editor).toHaveText("PING");
  await editor.fill(
    `SET ${prefix}:string hello\nHSET ${prefix}:hash first one second two\nSADD ${prefix}:set a b\nRPUSH ${prefix}:list a b\nZADD ${prefix}:zset 1.5 first`,
  );
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
  await expect(page.locator(".safety-warning")).toContainText("without automatic rollback");
  await page.getByRole("button", { name: "Authorize and run", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await page.locator(".redis-tools summary").click();
  await page.getByRole("textbox", { name: "Key pattern", exact: true }).fill(`${prefix}:*`);
  await page.getByRole("button", { name: "Scan keys", exact: true }).click();
  await expect(page.locator(".redis-key-list button")).toHaveCount(5);
  await page
    .locator(".redis-key-list button")
    .filter({ hasText: `${prefix}:hash` })
    .click();
  await expect(page.locator(".redis-value-list")).toContainText("one");
  await page.getByLabel("Field, index or score").fill("first");
  const value = "updated 🐳\nUNLINK another";
  await page.getByLabel("Redis edit value", { exact: true }).fill(value);
  await page.getByRole("button", { name: "Review command", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Review this operation" })).toBeVisible();
  await expect(page.locator(".sql-preview")).toContainText("HSET");
  await expect(page.locator(".sql-preview")).toContainText("\\x0aUNLINK another");
  await page.getByRole("button", { name: "Authorize and run", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await page.getByRole("button", { name: "Inspect key", exact: true }).click();
  await expect(page.locator(".redis-value-list")).toContainText(value);
  await page
    .locator(".redis-key-list button")
    .filter({ hasText: `${prefix}:zset` })
    .click();
  await page.getByLabel("Field, index or score").fill("0 value\nUNLINK another");
  await page.getByRole("button", { name: "Review command", exact: true }).click();
  await expect(page.locator(".redis-error")).toContainText("finite sorted-set score");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await editor.fill("HS");
  await editor.press("Control+Space");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("HSET");
  await editor.press("Escape");
  await editor.fill("KEYS *");
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect(page.locator(".query-error")).toContainText("unavailable");
  await page.screenshot({ path: ".cache/screenshots/redis-workspace.png", fullPage: true });
  await editor.fill(
    `UNLINK ${prefix}:string ${prefix}:hash ${prefix}:set ${prefix}:list ${prefix}:zset`,
  );
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await page.getByRole("button", { name: "Authorize and run", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  expect(errors).toEqual([]);
});
