import { randomUUID } from "node:crypto";
import { expect, test, type APIRequestContext, type Page } from "@playwright/test";
import { tableFromIPC } from "apache-arrow";
import type {
  PreparedPlan,
  QueryEvent,
  SavedWorkspace,
  SessionInfo,
} from "../../src/ipc/generated";

const bridge = "http://127.0.0.1:1422";
const headers = {
  Authorization: "Bearer joinery-disposable-browser-tests-only",
};
const secrets = {
  password: "joinery-local-only",
  sshPassword: null,
  keyPassphrase: null,
};
const fixturePort = Number(process.env.JOINERY_POSTGRES_PORT ?? 55432);

async function rpc<T>(request: APIRequestContext, command: string, args = {}): Promise<T> {
  const response = await request.post(`${bridge}/rpc/${command}`, {
    headers,
    data: args,
  });
  expect(response.ok()).toBe(true);
  const payload = await response.json();
  expect(payload.error).toBeUndefined();
  return payload.result;
}

async function observerSQL(request: APIRequestContext, sessionId: string, sql: string) {
  const plan = await rpc<PreparedPlan>(request, "prepare_execution", {
    sessionId,
    sql,
  });
  const streamId = randomUUID();
  await rpc(request, "execute_plan", {
    planId: plan.id,
    streamId,
    approval: null,
  });
  const events: QueryEvent[] = [];
  await expect
    .poll(async () => {
      const response = await request.get(`${bridge}/events/${streamId}`, {
        headers,
      });
      expect(response.ok()).toBe(true);
      events.push(...(await response.json()));
      return events.some((event) => event.type === "finished");
    })
    .toBe(true);
  expect(events.filter((event) => event.type === "error")).toEqual([]);
  expect(events.find((event) => event.type === "finished")).toMatchObject({
    status: "completed",
  });
  return events;
}

async function scalar(request: APIRequestContext, sessionId: string, sql: string) {
  const events = await observerSQL(request, sessionId, sql);
  const event = events.find((event) => event.type === "result");
  if (!event || event.type !== "result") throw new Error("Observer result missing");
  try {
    const response = await request.get(`${bridge}/result/${event.result.id}?start=0&count=1`, {
      headers,
    });
    expect(response.ok()).toBe(true);
    const rows = tableFromIPC(new Uint8Array(await response.body()));
    expect(rows.numRows).toBe(1);
    return String(rows.getChildAt(0)!.get(0));
  } finally {
    await rpc(request, "release_result", { resultId: event.result.id });
  }
}

async function connectUI(page: Page, name: string, port: number, mode: "silent" | "confirm") {
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Database engine").selectOption("postgres");
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(String(port));
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill(secrets.password);
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption(mode);
  const connected = page.waitForResponse((response) =>
    response.url().endsWith("/rpc/connect_prepared"),
  );
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  const payload = await (await connected).json();
  expect(payload.error).toBeUndefined();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  return payload.result as SessionInfo;
}

for (const fault of ["acknowledgement", "fatal"] as const) {
  test(`reviewed raw PostgreSQL browse retires after ${fault} COMMIT loss without replay`, async ({
    page,
    request,
  }) => {
    test.setTimeout(90000);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await rpc(request, "disconnect_fixtures");
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
    const proxy = await rpc<{ id: string; port: number }>(request, "test_commit_proxy_start");
    const table = `browse_fault_${randomUUID().replaceAll("-", "")}`;
    const marker = `${table}_marker`;
    const fn = `${table}_mark`;
    let observer: SessionInfo | undefined;
    let created = false;
    let observing = false;
    const browseRequests: unknown[] = [];
    page.on("request", (candidate) => {
      if (observing && candidate.url().endsWith("/rpc/browse_table"))
        browseRequests.push(candidate.postDataJSON());
    });
    try {
      await page.goto("/");
      const setup = await connectUI(page, `Setup ${table}`, fixturePort, "silent");
      observer = await rpc<SessionInfo>(request, "connect", {
        id: setup.connectionId,
        secrets,
      });
      created = true;
      await observerSQL(
        request,
        observer.id,
        `CREATE TABLE public.${table} (id integer PRIMARY KEY, note text NOT NULL); INSERT INTO public.${table} VALUES (1,'original'); CREATE TABLE public.${marker} (value integer); CREATE FUNCTION public.${fn}(v integer) RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$ BEGIN INSERT INTO public.${marker} VALUES(v); RETURN true; END $$`,
      );
      await rpc(request, "disconnect", { sessionId: setup.id });
      await page.reload();
      const actor = await connectUI(page, `Raw browse ${fault}`, proxy.port, "confirm");
      const button = page.getByRole("button", { name: table, exact: true });
      if (!(await button.isVisible())) await page.getByRole("button", { name: /^public/ }).click();
      await button.click();
      await expect(page.locator(".result-footer")).toContainText("1 rows loaded");
      const filters = page.getByRole("toolbar", { name: "Table filters" });
      await filters.getByLabel("WHERE clause").fill(`public.${fn}(id)`);
      await filters.getByRole("button", { name: "Run SQL clauses" }).click();
      const review = page.getByRole("dialog", {
        name: "Review this operation",
      });
      await expect(review).toBeVisible();
      expect(await scalar(request, observer.id, `SELECT count(*) FROM public.${marker}`)).toBe("0");
      const baseline = await rpc<{ commits: number }>(request, "test_commit_proxy_arm", {
        id: proxy.id,
        fault,
      });
      observing = true;
      const response = page.waitForResponse((response) =>
        response.url().endsWith("/rpc/browse_table"),
      );
      await review.getByRole("button", { name: "Authorize and run", exact: true }).click();
      const payload = await (await response).json();
      expect(payload.error).toMatchObject({ code: "outcomeUnknown" });
      expect(payload.result).toBeUndefined();
      await expect(review).toHaveCount(0);
      await expect(page.getByRole("alert")).toContainText(
        "Reviewed raw expressions may have committed",
      );
      await expect(page.locator(".status-bar")).toContainText("disconnected");
      await expect(page.locator(".result-footer .status-dot.completed")).toHaveCount(0);
      await expect(page.locator(".toast")).not.toContainText("Changes committed");
      // Only the independent direct observer remains; the proxied UI session retired.
      expect((await rpc<{ sessionCount: number }>(request, "resource_usage")).sessionCount).toBe(1);
      await page.getByRole("button", { name: "New query tab", exact: true }).click();
      await expect(
        page.getByRole("button", { name: "Run all statements", exact: true }),
      ).toBeDisabled();
      // The surviving observer performs a real fresh roundtrip and reads the committed marker.
      expect(
        await scalar(request, observer.id, `SELECT count(*) FROM public.${marker} WHERE value=1`),
      ).toBe("1");
      expect(await scalar(request, observer.id, `SELECT count(*) FROM public.${marker}`)).toBe("1");
      expect(await scalar(request, observer.id, "SELECT 7")).toBe("7");
      const status = await rpc<{
        commits: number;
        acknowledged: boolean;
        triggered: boolean;
      }>(request, "test_commit_proxy_status", { id: proxy.id });
      expect(status).toMatchObject({ acknowledged: true, triggered: true });
      expect(status.commits - baseline.commits).toBe(1);
      expect(browseRequests).toHaveLength(1);
      expect(browseRequests[0]).toMatchObject({ sessionId: actor.id });
      expect(errors).toEqual([]);
    } finally {
      observing = false;
      try {
        if (created && observer)
          await observerSQL(
            request,
            observer.id,
            `DROP FUNCTION IF EXISTS public.${fn}(integer); DROP TABLE IF EXISTS public.${marker}; DROP TABLE IF EXISTS public.${table}`,
          );
      } finally {
        try {
          await rpc(request, "test_commit_proxy_stop", { id: proxy.id });
        } finally {
          await rpc(request, "disconnect_fixtures");
        }
      }
    }
  });
}
