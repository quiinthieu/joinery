import { createHash } from "node:crypto";
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
  expect(await rpc(request, "test_seed_history")).toMatchObject({ count: 1_006 });
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

test("history pages tied timestamps, searches beyond 1000 and opens exact SQL without execution", async ({
  page,
  request,
}) => {
  await seed(request);
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
  expect(payload.entries[0].id).toBe("history-fixture-0000");
  expect("sql" in payload.entries[0]).toBe(false);
  expect(Array.from(payload.entries[0].sqlPreview ?? "").length).toBeLessThanOrEqual(256);
  expect(payload.entries[0].sqlPreview).not.toContain("olderneedle");
  expect(JSON.stringify(payload).length).toBeLessThan(3_000);
  await expect(rows).toHaveCount(1);
  expect(commands).not.toContain("history_entry");
  await rows.first().click();
  await expect.poll(() => activeSql(page)).toBe(exactSql);
  const detail = await rpc<HistoryEntry>(request, "history_entry", { id: "history-fixture-0000" });
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
  await seed(request);
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
    expect(await rpc(request, "history_entry", { id: "history-fixture-0000" })).toBeNull();
    await expect.poll(() => activeSql(page)).not.toBe(exactSql);
    await page.getByRole("button", { name: "Refresh", exact: true }).click();
    await expect(row).toHaveCount(0);
  } finally {
    release();
  }
});
