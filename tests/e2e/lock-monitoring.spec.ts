import { expect, test } from "@playwright/test";

const PG_PORT = Number(process.env.JOINERY_POSTGRES_PORT ?? "55432");

test("PostgreSQL processes view shows a lock wait and its root blocker", async ({
  page,
  request,
}) => {
  test.setTimeout(90000);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
  await request.post("http://127.0.0.1:1422/rpc/disconnect_fixtures", { headers, data: {} });
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Database engine").selectOption("postgres");
  await page.getByLabel("Connection name", { exact: true }).fill("Lock fixture");
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(String(PG_PORT));
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  const editor = page.getByRole("textbox", { name: "SQL editor" });
  const runAll = page.getByRole("button", { name: "Run all statements", exact: true });

  await page.getByRole("button", { name: "New query tab" }).click();
  const holderTab = (await page.getByRole("tab", { selected: true }).getAttribute("id"))!;
  await editor.fill("SELECT pg_advisory_lock(424242)");
  await runAll.click();
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();

  await page.getByRole("button", { name: "New query tab" }).click();
  const waiterTab = (await page.getByRole("tab", { selected: true }).getAttribute("id"))!;
  await page.getByRole("button", { name: "New isolated session", exact: true }).click();
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".query-session")).toContainText("Isolated session");
  await editor.fill("SELECT pg_advisory_lock(424242) AS waited");
  await runAll.click();
  await expect(page.locator(`#${waiterTab} .tab-running`)).toBeVisible();

  await page.getByRole("button", { name: "Processes" }).click();
  const waits = page.getByRole("region", { name: "Lock waits" });
  await expect(waits.locator("[data-lock-wait]")).toHaveCount(1);
  await expect(waits).toContainText("advisory · ExclusiveLock");
  await expect(waits).toContainText("pg_advisory_lock(424242) AS waited");
  const blocker = (await waits.locator("[data-lock-wait] td").nth(1).textContent())!.trim();
  await expect(page.locator(`[data-process="${blocker}"]`)).toContainText("Blocking 1");

  await page.locator(`#${holderTab}`).click();
  await editor.fill("SELECT pg_advisory_unlock(424242)");
  await runAll.click();
  await expect(page.locator(`#${waiterTab} .tab-running`)).toHaveCount(0);
  await page.getByRole("tab", { name: "Processes" }).click();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(waits).toContainText("No sessions are waiting on locks.");
  expect(errors).toEqual([]);
});
