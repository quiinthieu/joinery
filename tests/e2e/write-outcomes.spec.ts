import { randomUUID } from "node:crypto";
import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import type { QueryEvent, SavedWorkspace, SessionInfo } from "../../src/ipc/generated";

const bridge = "http://127.0.0.1:1422";
const fixturePort = Number(process.env.JOINERY_POSTGRES_PORT ?? 55432);

async function rpc<T>(request: APIRequestContext, command: string, args = {}): Promise<T> {
  const response = await request.post(`${bridge}/rpc/${command}`, {
    headers: { Authorization: "Bearer joinery-disposable-browser-tests-only" },
    data: args,
  });
  expect(response.ok()).toBe(true);
  const payload = await response.json();
  expect(payload.error).toBeUndefined();
  return payload.result;
}

async function connectUI(
  page: Page,
  name: string,
  port: number,
  openQuery = false,
): Promise<SessionInfo> {
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(String(port));
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  const connected = page.waitForResponse((response) => response.url().endsWith("/rpc/connect"));
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  const payload = await (await connected).json();
  expect(payload.error).toBeUndefined();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  // The footer describes the active tab. Give the observer a tab belonging to
  // its new connection instead of inspecting the retired writer's tab.
  if (openQuery) await page.getByRole("button", { name: "New query tab", exact: true }).click();
  await expect(page.locator(".status-bar")).toContainText(name);
  return payload.result;
}

async function run(page: Page, sql: string) {
  const results = page.locator(".result-tabs [role=tab]");
  const previous = (await results.count()) ? await results.last().getAttribute("id") : null;
  await page.getByRole("textbox", { name: "SQL editor" }).fill(sql);
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect
    .poll(async () => {
      const id = (await results.count()) ? await results.last().getAttribute("id") : null;
      return Boolean(id && id !== previous);
    })
    .toBe(true);
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await expect(page.locator(".query-error")).toHaveCount(0);
  await results.last().click();
}

async function inspectFirstCell(page: Page, value: string) {
  if (!(await page.locator(".inspector").isVisible()))
    await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
  await expect(async () => {
    const box = (await page.locator(".grid-host").boundingBox())!;
    await page.mouse.click(box.x + 90, box.y + 55);
    await expect(page.locator(".inspector-value")).toHaveText(value, { timeout: 500 });
  }).toPass({ timeout: 10000 });
}

// Cleanup uses the real core and only this test's validated fixture profile/table.
async function cleanupTable(request: APIRequestContext, profileId: string, table: string) {
  const session = await rpc<SessionInfo>(request, "connect", {
    id: profileId,
    secrets: { password: "joinery-local-only", sshPassword: null, keyPassphrase: null },
  });
  try {
    const plan = await rpc<{ id: string }>(request, "prepare_execution", {
      sessionId: session.id,
      sql: `DROP TABLE IF EXISTS public.${table}`,
    });
    const streamId = randomUUID();
    await rpc(request, "execute_plan", { planId: plan.id, streamId, approval: null });
    const events: QueryEvent[] = [];
    await expect
      .poll(async () => {
        const response = await request.get(`${bridge}/events/${streamId}`, {
          headers: { Authorization: "Bearer joinery-disposable-browser-tests-only" },
        });
        expect(response.ok()).toBe(true);
        events.push(...(await response.json()));
        return events.some((event) => event.type === "finished");
      })
      .toBe(true);
    expect(events.find((event) => event.type === "finished")).toMatchObject({
      status: "completed",
    });
  } finally {
    await rpc(request, "disconnect", { sessionId: session.id });
  }
}

for (const writer of ["grid", "CSV"] as const) {
  for (const fault of ["acknowledgement", "fatal"] as const) {
    test(`${writer} reviewed PostgreSQL write retires the session after ${fault} COMMIT loss without replay`, async ({
      page,
      request,
    }) => {
      test.setTimeout(90000);
      await rpc(request, "disconnect_fixtures");
      // The shared disposable harness retains preceding workspace tests' layout.
      // Reset only that workspace before this fresh page starts its save watcher.
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
      const table = `write_outcome_${randomUUID().replaceAll("-", "")}`;
      let fixtureProfileId: string | undefined;
      let created = false;
      let observing = false;
      let submissions = 0;
      const events: QueryEvent[] = [];
      const responses: Promise<void>[] = [];
      page.on("request", (candidate) => {
        if (observing && candidate.url().endsWith("/rpc/execute_plan")) submissions++;
      });
      page.on("response", (response) => {
        if (observing && response.url().startsWith(`${bridge}/events/`))
          responses.push(
            response.json().then((payload: QueryEvent[]) => {
              events.push(...payload);
            }),
          );
      });
      try {
        await page.goto("/");
        const fixture = await connectUI(page, `Setup ${table}`, fixturePort);
        fixtureProfileId = fixture.connectionId;
        await page.getByRole("button", { name: "New query tab", exact: true }).click();
        created = true;
        await run(
          page,
          `CREATE TABLE public.${table} (id integer PRIMARY KEY, note text NOT NULL); INSERT INTO public.${table} VALUES (1, 'before')`,
        );
        await rpc(request, "disconnect_fixtures");
        await page.reload();
        await connectUI(page, `Fault ${writer} ${fault}`, proxy.port);

        const changed = `${writer.toLowerCase()}_${fault}`;
        if (writer === "grid") {
          await page.getByRole("button", { name: "New query tab", exact: true }).click();
          await run(page, `SELECT id, note FROM public.${table}`);
          await expect(page.getByTestId("result-provenance-badge")).toContainText(
            "Editable result",
          );
          await expect(async () => {
            const box = (await page.locator(".grid-host").boundingBox())!;
            await page.mouse.dblclick(box.x + 44 + 110 + 70, box.y + 37 + 18);
            await expect(page.locator("#portal textarea, #portal input").first()).toBeVisible({
              timeout: 500,
            });
          }).toPass({ timeout: 10000 });
          const overlay = page.locator("#portal textarea, #portal input").first();
          await overlay.fill(changed);
          await overlay.press("Enter");
          await page.getByRole("button", { name: "Review 1 edit", exact: true }).click();
        } else {
          const tableButton = page.getByRole("button", { name: table, exact: true });
          if (!(await tableButton.isVisible()))
            await page.getByRole("button", { name: /^public/ }).click();
          await tableButton.click();
          await expect(page.locator(".result-footer")).toContainText("1 rows loaded");
          await rpc(request, "test_write_sandbox_file", {
            fileName: `${table}.csv`,
            contents: `id,note\n2,${changed}\n`,
          });
          await page.getByRole("button", { name: "Import CSV…", exact: true }).click();
          const importer = page.getByRole("dialog", { name: "Import CSV into table" });
          await importer.getByRole("button", { name: "Choose CSV file…" }).click();
          await importer
            .getByRole("button", { name: "Review & import 1 row…", exact: true })
            .click();
        }
        const review = page.getByRole("dialog", { name: "Review pending changes" });
        await expect(review).toBeVisible();
        await expect(review.locator(".sql-preview")).toContainText(table);
        await rpc(request, "test_commit_proxy_arm", { id: proxy.id, fault });
        observing = true;
        await review.getByRole("button", { name: "Apply changes", exact: true }).click();
        await expect(review).toHaveCount(0);
        await expect
          .poll(async () => {
            await Promise.all(responses);
            return events.some(
              (event) => event.type === "finished" && event.status === "outcomeUnknown",
            );
          })
          .toBe(true);
        expect(events.some((event) => event.type === "sessionClosed")).toBe(true);
        expect(
          events.some((event) => event.type === "error" && event.error.code === "outcomeUnknown"),
        ).toBe(true);
        expect(
          events.some((event) => event.type === "finished" && event.status === "completed"),
        ).toBe(false);

        if (writer === "CSV") {
          const importer = page.getByRole("dialog", { name: "Import CSV into table" });
          await expect(importer).toContainText("commit outcome unknown — import halted");
          await expect(importer).toContainText(
            "This batch may have committed. Verify the table before retrying.",
          );
          await expect(importer).not.toContainText("No rows from this batch were committed");
          await expect(importer).not.toContainText("Imported 1 row into");
          await expect(importer.getByRole("button", { name: /Review & import/ })).toBeDisabled();
          await importer.getByRole("button", { name: "Close", exact: true }).click();
        } else {
          await expect(page.locator(".query-error.uncertain")).toBeVisible();
          await expect(page.locator(".toast")).not.toContainText("Changes committed");
        }
        await expect(page.locator(".status-bar")).toContainText("disconnected");
        expect((await rpc<{ sessionCount: number }>(request, "resource_usage")).sessionCount).toBe(
          0,
        );
        await page.getByRole("button", { name: "New query tab", exact: true }).click();
        await expect(
          page.getByRole("button", { name: "Run all statements", exact: true }),
        ).toBeDisabled();
        expect(submissions).toBe(1);
        observing = false;

        // A new direct fixture connection observes the real committed value.
        await connectUI(page, `Verify ${table}`, fixturePort, true);
        await run(
          page,
          `SELECT string_agg(id::text || ':' || note, ',' ORDER BY id) FROM public.${table}`,
        );
        await inspectFirstCell(page, writer === "grid" ? `1:${changed}` : `1:before,2:${changed}`);
        const status = await rpc(request, "test_commit_proxy_status", { id: proxy.id });
        expect(status).toMatchObject({ commits: 1, acknowledged: true, triggered: true });
      } finally {
        observing = false;
        try {
          await rpc(request, "disconnect_fixtures");
          if (created && fixtureProfileId) await cleanupTable(request, fixtureProfileId, table);
        } finally {
          await rpc(request, "test_commit_proxy_stop", { id: proxy.id });
          await rpc(request, "disconnect_fixtures");
        }
      }
    });
  }
}
