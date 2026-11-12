import { randomUUID } from "node:crypto";
import { tableFromIPC } from "apache-arrow";
import { test, expect, type APIRequestContext, type Page } from "./fixtures";
import type {
  HistoryEntry,
  PreparedPlan,
  QueryEvent,
  SavedWorkspace,
  SessionInfo,
} from "../../src/ipc/generated";

const bridge = "http://127.0.0.1:1422";
const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
const secrets = { password: "joinery-local-only", sshPassword: null, keyPassphrase: null };

async function rpc<T>(request: APIRequestContext, command: string, args = {}): Promise<T> {
  const response = await request.post(`${bridge}/rpc/${command}`, { headers, data: args });
  expect(response.ok()).toBe(true);
  const payload = await response.json();
  expect(payload.error).toBeUndefined();
  return payload.result;
}

// These calls exercise the actual shared executor and the disposable PostgreSQL fixture.
async function execute(request: APIRequestContext, sessionId: string, sql: string) {
  const plan = await rpc<PreparedPlan>(request, "prepare_execution", { sessionId, sql });
  const streamId = randomUUID();
  await rpc(request, "execute_plan", { planId: plan.id, streamId, approval: null });
  const events: QueryEvent[] = [];
  await expect
    .poll(async () => {
      const response = await request.get(`${bridge}/events/${streamId}`, { headers });
      expect(response.ok()).toBe(true);
      events.push(...(await response.json()));
      return events.some((event) => event.type === "finished");
    })
    .toBe(true);
  expect(events.filter((event) => event.type === "error")).toEqual([]);
  expect(events.find((event) => event.type === "finished")).toMatchObject({ status: "completed" });
  return events;
}

async function scalar(request: APIRequestContext, sessionId: string, sql: string) {
  const events = await execute(request, sessionId, sql);
  const result = events.find((event) => event.type === "result");
  if (!result || result.type !== "result") throw new Error("Missing observer result");
  try {
    const response = await request.get(`${bridge}/result/${result.result.id}?start=0&count=1`, {
      headers,
    });
    expect(response.ok()).toBe(true);
    const rows = tableFromIPC(new Uint8Array(await response.body()));
    expect(rows.numRows).toBe(1);
    return String(rows.getChildAt(0)!.get(0));
  } finally {
    await rpc(request, "release_result", { resultId: result.result.id });
  }
}

async function connectUI(page: Page, name: string): Promise<SessionInfo> {
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Database engine").selectOption("postgres");
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(process.env.JOINERY_POSTGRES_PORT ?? "55432");
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill(secrets.password);
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  await page.getByRole("checkbox", { name: /Save SQL text in local history/ }).uncheck();
  const connected = page.waitForResponse((response) =>
    response.url().endsWith("/rpc/connect_prepared"),
  );
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  const payload = await (await connected).json();
  expect(payload.error).toBeUndefined();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  return payload.result;
}

test("a PostgreSQL result limit reports the untouched script suffix in run, history and search", async ({
  page,
  request,
}) => {
  test.setTimeout(120000);
  await rpc(request, "save_workspace", {
    workspace: {
      theme: "dark",
      density: "comfortable",
      sidebarVisible: true,
      sidebarCollapsed: false,
      inspectorVisible: false,
      activeDraftId: null,
      drafts: [],
      tableLayouts: [],
      shortcuts: {},
    } satisfies SavedWorkspace,
  });
  const suffix = randomUUID().replaceAll("-", "");
  const table = `script_accounting_${suffix}`;
  const name = `Script accounting ${suffix}`;
  let profileId: string | undefined;
  let created = false;
  try {
    await page.goto("/");
    const session = await connectUI(page, name);
    profileId = session.connectionId;
    created = true;
    await execute(
      request,
      session.id,
      `CREATE TABLE public.${table} (value integer NOT NULL); INSERT INTO public.${table} VALUES (0)`,
    );
    await page.getByRole("button", { name: "New query tab", exact: true }).click();
    await page
      .getByRole("textbox", { name: "SQL editor" })
      .fill(
        `SELECT n FROM generate_series(1,1000001) n; SELECT 'unattempted_${suffix}' AS skipped; UPDATE public.${table} SET value = 1`,
      );
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    const summary = page.getByRole("status", { name: "Statement execution summary" });
    await expect(summary).toContainText(
      "Stopped · Planned: 3 · Attempted: 1 · Completed: 1 · Not attempted: 2",
      { timeout: 90000 },
    );
    await expect(summary).toContainText(
      "Result retention limit reached · Statements 2–3 were not attempted",
    );
    await expect(summary).toContainText("An attempted statement may not have reached the server");
    await expect(summary).toContainText(
      "Completed statements may still be uncommitted, and results may be truncated",
    );
    await expect(page.locator(".result-footer .status-dot.stopped")).toBeVisible();
    await expect(page.locator(".result-tabs [role=tab]")).toHaveCount(1);

    // Verify the unattempted write independently on a fresh fixture session.
    const observer = await rpc<SessionInfo>(request, "connect", { id: profileId, secrets });
    try {
      expect(await scalar(request, observer.id, `SELECT value FROM public.${table}`)).toBe("0");
    } finally {
      await rpc(request, "disconnect", { sessionId: observer.id });
    }
    const history = await rpc<HistoryEntry[]>(request, "history", { search: "" });
    const stopped = history.find(
      (entry) => entry.connectionId === profileId && entry.status === "stopped",
    );
    expect(stopped).toMatchObject({
      sql: null,
      statementCount: 3,
      executionSummary: {
        planned: 3,
        attempted: 1,
        completed: 1,
        notAttempted: 2,
        stopReason: "resultLimit",
      },
    });

    // A fresh run replaces the previous final accounting rather than retaining its suffix.
    await page.getByRole("textbox", { name: "SQL editor" }).fill("SELECT 7 AS fresh_run");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(summary).toContainText(
      "Completed · Planned: 1 · Attempted: 1 · Completed: 1 · Not attempted: 0",
    );
    await expect(summary).not.toContainText("Statements 2–3");
    await expect(summary).not.toContainText("Result retention limit");

    await page.getByRole("button", { name: "Query history", exact: true }).click();
    const entry = page
      .locator(".history-entry")
      .filter({ hasText: name })
      .filter({ hasText: "Statements 2–3 were not attempted" });
    await expect(entry).toHaveCount(1);
    await expect(entry.locator("code")).toHaveText("SQL text not stored");
    await expect(entry).toContainText(
      "Stopped · Planned: 3 · Attempted: 1 · Completed: 1 · Not attempted: 2",
    );
    await expect(entry.locator(".status-dot.stopped")).toBeVisible();

    await page.keyboard.press("Control+k");
    const palette = page.getByRole("dialog", { name: "Command palette" });
    await expect(palette).toBeVisible();
    await palette.getByRole("tab", { name: "History", exact: true }).click();
    await palette.getByRole("textbox", { name: "Search commands" }).fill(`${name} stopped`);
    const match = palette
      .getByRole("button")
      .filter({ hasText: "Statements 2–3 were not attempted" });
    await expect(match).toHaveCount(1);
    await expect(match).toContainText("SQL text not stored");
    await expect(match).toContainText("Not attempted: 2");
    await expect(match.locator(".palette-history-summary")).toHaveCSS("white-space", "normal");
    await expect(match).toContainText(
      "Attempts may not reach the server; completed statements may be uncommitted",
    );
    await palette.getByRole("textbox", { name: "Search commands" }).press("Enter");
    await expect(palette).toHaveCount(0);
    await expect(page.locator(".toast")).toContainText("Only execution metadata was saved");
  } finally {
    if (profileId && created) {
      const cleanup = await rpc<SessionInfo>(request, "connect", { id: profileId, secrets });
      try {
        await execute(request, cleanup.id, `DROP TABLE IF EXISTS public.${table}`);
      } finally {
        await rpc(request, "disconnect", { sessionId: cleanup.id });
      }
    }
  }
});
