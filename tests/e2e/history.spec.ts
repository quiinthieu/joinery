import { createHash, randomUUID } from "node:crypto";
import { test, expect, type APIRequestContext, type Page } from "./fixtures";
import type { HistoryEntry, HistoryPage } from "../../src/ipc/generated";

const bridge = "http://127.0.0.1:1422";
const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
const exactSql = `SELECT 'history fixture';\n/* ${"雪🦀".repeat(1_000)}\nolderneedle CAFÉ 雪🦀\n*/`;

async function rpc<T>(request: APIRequestContext, command: string, args = {}): Promise<T> {
  const response = await request.post(`${bridge}/rpc/${command}`, { headers, data: args });
  expect(response.ok()).toBe(true);
  const body = await response.json();
  expect(body.error).toBeUndefined();
  return body.result;
}

async function seed(request: APIRequestContext) {
  await rpc(request, "clear_history");
  const result = await rpc<{ count: number; olderId: string; admittedId: string }>(
    request,
    "test_seed_history",
  );
  expect(result).toMatchObject({ count: 1_011 });
  return result;
}

async function openHistory(page: Page) {
  await page.getByRole("button", { name: "Query history", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Query history", exact: true })).toBeVisible();
}

async function activeSql(page: Page) {
  return page.evaluate(async () => {
    // Inspect the document state after real UI activation, rather than a
    // virtualized CodeMirror viewport which does not contain the entire file.
    const modulePath = "/src/state/studio.ts";
    const { useStudio } = await import(modulePath);
    const state = useStudio.getState();
    return state.tabs.find((tab: { id: string }) => tab.id === state.activeTabId)?.sql;
  });
}

test("clearing admitted PostgreSQL history survives late completion and renderer restart", async ({
  page,
  request,
}) => {
  test.setTimeout(60000);
  await rpc(request, "clear_history");
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  const name = `History admission ${randomUUID()}`;
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Database engine").selectOption("postgres");
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(process.env.JOINERY_POSTGRES_PORT ?? "55432");
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  await page.getByLabel(/Save SQL text in local history/).check();
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab", exact: true }).click();
  const queryTab = page.getByRole("tablist", { name: "Workspaces" }).getByRole("tab", {
    selected: true,
  });
  const queryTabId = (await queryTab.getAttribute("id"))!;
  const sql = "/* exact admission 雪 */\nSELECT pg_sleep(8); -- retained only until Clear\n";
  await page.getByRole("textbox", { name: "SQL editor" }).fill(sql);
  const admitted = page.waitForResponse((response) => response.url().endsWith("/rpc/execute_plan"));
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  const admission = await (await admitted).json();
  expect(admission.error).toBeUndefined();
  const runId = admission.result as string;
  expect(await rpc<HistoryEntry>(request, "history_entry", { id: runId })).toMatchObject({
    sql,
    provenance: "admitted",
    status: null,
    elapsedMs: null,
  });
  await openHistory(page);
  await page.getByRole("textbox", { name: "Search history", exact: true }).fill(name);
  const rows = page.locator(".history-list .history-entry");
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("Admitted; completion pending");
  await expect(rows).toContainText("Duration unavailable");
  await page.getByRole("button", { name: "Clear history", exact: true }).click();
  await page.getByRole("button", { name: "Clear local history", exact: true }).click();
  await expect(rows).toHaveCount(0);
  expect(await rpc(request, "history_entry", { id: runId })).toBeNull();

  // Finish the actual server query, then revisit both the native detail and UI.
  await page.locator(`[id="${queryTabId}"]`).click();
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible({
    timeout: 20000,
  });
  await expect(page.locator(".query-error")).toHaveCount(0);
  expect(await rpc(request, "history_entry", { id: runId })).toBeNull();
  await openHistory(page);
  await page.getByRole("textbox", { name: "Search history", exact: true }).fill(name);
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(rows).toHaveCount(0);
  await page.reload();
  await openHistory(page);
  await page.getByRole("textbox", { name: "Search history", exact: true }).fill(name);
  await expect(rows).toHaveCount(0);
  expect(await rpc(request, "history_entry", { id: runId })).toBeNull();
});

test("history pages tied timestamps, searches beyond 1000 and opens exact SQL without execution", async ({
  page,
  request,
}) => {
  const fixture = await seed(request);
  const commands: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/rpc/")) commands.push(request.url().split("/rpc/")[1]);
  });
  await page.goto("/");
  await openHistory(page);
  const rows = page.locator(".history-list .history-entry");
  await expect(rows).toHaveCount(50);
  await expect(rows.first()).toContainText("History fixture 1005");
  await expect(rows.nth(1)).toContainText("SQL text not stored");
  const firstPage = await rows.allTextContents();
  await page.getByRole("button", { name: "Next history page", exact: true }).click();
  await expect(rows.first()).toContainText("History fixture 0955");
  await expect(rows).toHaveCount(50);
  const secondPage = await rows.allTextContents();
  expect(secondPage.some((row) => firstPage.includes(row))).toBe(false);
  await page.getByRole("button", { name: "Previous history page", exact: true }).click();
  await expect(rows.first()).toContainText("History fixture 1005");

  const response = page.waitForResponse(async (response) => {
    if (!response.url().endsWith("/rpc/history_page")) return false;
    return response.request().postDataJSON().request.search === "olderneedle café";
  });
  await page.getByRole("textbox", { name: "Search history", exact: true }).fill("olderneedle café");
  const payload = (await (await response).json()).result as HistoryPage;
  expect(payload.entries).toHaveLength(1);
  expect(payload.entries[0].id).toBe(fixture.olderId);
  expect("sql" in payload.entries[0]).toBe(false);
  expect(Array.from(payload.entries[0].sqlPreview ?? "").length).toBeLessThanOrEqual(256);
  expect(payload.entries[0].sqlPreview).not.toContain("olderneedle");
  expect(JSON.stringify(payload).length).toBeLessThan(3_000);
  await expect(rows).toHaveCount(1);
  expect(commands).not.toContain("history_entry");
  await rows.first().click();
  await expect.poll(() => activeSql(page)).toBe(exactSql);
  const detail = await rpc<HistoryEntry>(request, "history_entry", { id: fixture.olderId });
  expect(createHash("sha256").update(detail.sql!).digest("hex")).toBe(
    createHash("sha256").update(exactSql).digest("hex"),
  );
  expect(commands).toContain("history_entry");
  expect(commands).not.toContain("history");
  expect(
    commands.filter((command) => /^(connect|prepare_execution|execute_plan)/.test(command)),
  ).toEqual([]);

  // A fresh renderer obtains the same retained SQLite entry.
  await page.reload();
  await openHistory(page);
  await page.getByRole("textbox", { name: "Search history", exact: true }).fill("olderneedle");
  await expect(rows).toHaveCount(1);
  await rows.first().click();
  await expect.poll(() => activeSql(page)).toBe(exactSql);
});

test("keyboard history palette finds preview-invisible SQL and distinguishes metadata from empty SQL", async ({
  page,
  request,
}) => {
  await seed(request);
  await page.goto("/");
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Command palette", exact: true });
  await palette.getByRole("tab", { name: "History", exact: true }).click();
  const search = palette.getByRole("textbox", { name: "Search commands", exact: true });
  await search.fill("olderneedle café");
  const match = palette.locator(".palette-commands button[data-palette-index]");
  await expect(match).toHaveCount(1);
  await expect(match).toContainText("History fixture 0000");
  await search.press("Enter");
  await expect(palette).toHaveCount(0);
  await expect.poll(() => activeSql(page)).toBe(exactSql);

  await openHistory(page);
  await page
    .getByRole("textbox", { name: "Search history", exact: true })
    .fill("History fixture 1004");
  const rows = page.locator(".history-list .history-entry");
  await expect(rows).toHaveCount(1);
  await rows.first().click();
  await expect(
    page.getByText("Only execution metadata was saved for this query.", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("textbox", { name: "Search history", exact: true })
    .fill("History fixture 1005");
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("History fixture 1005");
  await rows.first().click();
  await expect.poll(() => activeSql(page)).toBe("");
});

test("clearing history invalidates a delayed exact-detail response", async ({ page, request }) => {
  const fixture = await seed(request);
  await page.goto("/");
  await openHistory(page);
  await page.getByRole("textbox", { name: "Search history", exact: true }).fill("olderneedle");
  const row = page.locator(".history-list .history-entry");
  await expect(row).toHaveCount(1);
  let release!: () => void;
  const delayed = new Promise<void>((resolve) => {
    release = resolve;
  });
  let fetched!: () => void;
  const detailFetched = new Promise<void>((resolve) => {
    fetched = resolve;
  });
  await page.route("**/rpc/history_entry", async (route) => {
    const actual = await route.fetch();
    fetched();
    await delayed;
    await route.fulfill({ response: actual });
  });
  try {
    await row.click();
    await detailFetched;
    await page.getByRole("button", { name: "Clear history", exact: true }).click();
    await page.getByRole("button", { name: "Clear local history", exact: true }).click();
    await expect(
      page.getByRole("dialog", { name: "Clear local query history?", exact: true }),
    ).toHaveCount(0);
    const delivered = page.waitForResponse((response) =>
      response.url().endsWith("/rpc/history_entry"),
    );
    release();
    await (await delivered).finished();
    await page.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
    await expect(row).toHaveCount(0);
    expect(await rpc(request, "history_entry", { id: fixture.olderId })).toBeNull();
    await expect.poll(() => activeSql(page)).not.toBe(exactSql);
    await page.getByRole("button", { name: "Refresh", exact: true }).click();
    await expect(row).toHaveCount(0);
  } finally {
    release();
  }
});

test("history and keyboard palette preserve admitted and recovered provenance without executing saved SQL", async ({
  page,
  request,
}) => {
  const fixture = await seed(request);
  const commands: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/rpc/")) commands.push(request.url().split("/rpc/")[1]);
  });
  await page.goto("/");
  await openHistory(page);
  await page
    .getByRole("textbox", { name: "Search history", exact: true })
    .fill("Lifecycle fixture");
  const rows = page.locator(".history-list .history-entry");
  await expect(rows).toHaveCount(5);
  const admitted = rows.filter({ hasText: "Lifecycle fixture admitted" });
  const interrupted = rows.filter({ hasText: "Lifecycle fixture interrupted" });
  const restored = rows.filter({ hasText: "Lifecycle fixture restored" });
  const known = rows.filter({ hasText: "Lifecycle fixture known" });
  const duration = rows.filter({ hasText: "Lifecycle fixture duration" });
  await expect(admitted).toContainText("Admitted; completion pending");
  await expect(admitted).toContainText("Admission does not prove dispatch or server submission");
  await expect(interrupted).toContainText("Recovered after interruption · Completion unknown");
  await expect(restored).toContainText("Recovered from metadata restore · Completion unknown");
  for (const row of [admitted, interrupted, restored, duration]) {
    await expect(row).toContainText("Duration unavailable");
    await expect(row).not.toContainText(/\b0 ms\b/);
  }
  await expect(known).toContainText("Completed · Statement counts unknown");
  await expect(known).toContainText("17 ms");
  await expect(known).toContainText("Original recorded completion is preserved");
  await expect(duration).toContainText("Failed · Statement counts unknown");
  const stored = await rpc<HistoryEntry>(request, "history_entry", {
    id: fixture.admittedId,
  });
  expect(stored).toMatchObject({ provenance: "admitted", status: null, elapsedMs: null });
  expect(stored.sql).toContain("lifecycle_admitted");
  await admitted.click();
  await expect.poll(() => activeSql(page)).toBe(stored.sql);

  // Renderer restart reads the same SQLite provenance, without resuming work.
  await page.reload();
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Command palette", exact: true });
  await palette.getByRole("tab", { name: "History", exact: true }).click();
  const search = palette.getByRole("textbox", { name: "Search commands", exact: true });
  await search.fill("Lifecycle fixture");
  const matches = palette.locator(".palette-commands button[data-palette-index]");
  await expect(matches).toHaveCount(5);
  await expect(matches.filter({ hasText: "Lifecycle fixture interrupted" })).toContainText(
    "Recovered after interruption · Completion unknown",
  );
  await expect(matches.filter({ hasText: "Lifecycle fixture restored" })).toContainText(
    "Recovered from metadata restore · Completion unknown",
  );
  await expect(matches.filter({ hasText: "Lifecycle fixture known" })).toContainText("17 ms");
  await expect(matches.filter({ hasText: "Lifecycle fixture duration" })).toContainText(
    "Duration unavailable",
  );
  await search.fill("Lifecycle fixture admitted");
  await expect(matches).toHaveCount(1);
  await expect(matches).toContainText("Admitted; completion pending");
  await expect(matches).toContainText("Duration unavailable");
  await search.press("Enter");
  await expect(palette).toHaveCount(0);
  await expect.poll(() => activeSql(page)).toBe(stored.sql);
  expect(
    commands.filter((command) =>
      /^(connect|prepare_execution|execute_plan|prepare_snowflake_console|start_snowflake_console)/.test(
        command,
      ),
    ),
  ).toEqual([]);
});
