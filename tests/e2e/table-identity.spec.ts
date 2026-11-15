import { randomUUID } from "node:crypto";
import { expect, test, type APIRequestContext, type Page } from "./fixtures";
import { tableFromIPC } from "apache-arrow";
import type {
  PreparedPlan,
  QueryEvent,
  SavedWorkspace,
  SessionInfo,
} from "../../src/ipc/generated";

const bridge = "http://127.0.0.1:1422";
const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
const secrets = { password: "joinery-local-only", sshPassword: null, keyPassphrase: null };
const fixtures = [
  {
    name: "PostgreSQL",
    engine: "postgres",
    schema: "public",
    port: Number(process.env.JOINERY_POSTGRES_PORT ?? 55432),
  },
  { name: "MySQL", engine: "mysql", schema: "joinery", port: 53306 },
  { name: "MariaDB", engine: "mysql", schema: "joinery", port: 53307 },
] as const;
type Fixture = (typeof fixtures)[number];

async function rpc<T>(request: APIRequestContext, command: string, args = {}): Promise<T> {
  const response = await request.post(`${bridge}/rpc/${command}`, { headers, data: args });
  expect(response.ok()).toBe(true);
  const payload = await response.json();
  expect(payload.error).toBeUndefined();
  return payload.result;
}

async function observerSQL(request: APIRequestContext, sessionId: string, sql: string) {
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

async function observerRows(
  request: APIRequestContext,
  sessionId: string,
  schema: string,
  table: string,
) {
  const events = await observerSQL(
    request,
    sessionId,
    `SELECT id, note FROM ${schema}.${table} ORDER BY id`,
  );
  const result = events.find((event) => event.type === "result");
  expect(result?.type).toBe("result");
  if (!result || result.type !== "result") throw new Error("Observer result missing");
  try {
    const response = await request.get(`${bridge}/result/${result.result.id}?start=0&count=10`, {
      headers,
    });
    expect(response.ok()).toBe(true);
    const rows = tableFromIPC(new Uint8Array(await response.body()));
    return Array.from({ length: rows.numRows }, (_, index) => [
      String(rows.getChildAt(0)!.get(index)),
      String(rows.getChildAt(1)!.get(index)),
    ]);
  } finally {
    await rpc(request, "release_result", { resultId: result.result.id });
  }
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

async function connectUI(page: Page, name: string, fixture: Fixture): Promise<SessionInfo> {
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Database engine").selectOption(fixture.engine);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(String(fixture.port));
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill(secrets.password);
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
  const connected = page.waitForResponse((response) =>
    response.url().endsWith("/rpc/connect_prepared"),
  );
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  const payload = await (await connected).json();
  expect(payload.error).toBeUndefined();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  return payload.result;
}

async function query(page: Page, schema: string, table: string) {
  const results = page.locator(".result-tabs [role=tab]");
  const previous = (await results.count()) ? await results.last().getAttribute("id") : null;
  await page
    .getByRole("textbox", { name: "SQL editor" })
    .fill(`SELECT id, note FROM ${schema}.${table}`);
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  await expect
    .poll(async () => {
      const id = (await results.count()) ? await results.last().getAttribute("id") : null;
      return Boolean(id && id !== previous);
    })
    .toBe(true);
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await results.last().click();
  await expect(page.getByTestId("result-provenance-badge")).toContainText("Editable result");
}

async function stageEdit(page: Page, value: string) {
  await expect(async () => {
    const box = (await page.locator(".grid-host").boundingBox())!;
    await page.mouse.dblclick(box.x + 44 + 110 + 70, box.y + 37 + 18);
    await expect(page.locator("#portal textarea, #portal input").first()).toBeVisible({
      timeout: 500,
    });
  }).toPass({ timeout: 10000 });
  const overlay = page.locator("#portal textarea, #portal input").first();
  await overlay.fill(value);
  await overlay.press("Enter");
  await page.getByRole("button", { name: "Review 1 edit", exact: true }).click();
}

for (const fixture of fixtures.filter((candidate) => candidate.engine === "mysql")) {
  test(`${fixture.name} completed approved browse keeps rows read-only after guard revocation`, async ({
    page,
    request,
  }) => {
    test.setTimeout(60000);
    await rpc(request, "disconnect_fixtures");
    const table = `readonly_e2e_${randomUUID().replaceAll("-", "")}`;
    const marker = `${table}_m`,
      fn = `${table}_f`;
    let observer: SessionInfo | undefined;
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    try {
      await page.goto("/");
      const actor = await connectUI(page, `Read-only ${fixture.name} ${table}`, fixture);
      observer = await rpc<SessionInfo>(request, "connect", { id: actor.connectionId, secrets });
      await observerSQL(
        request,
        observer.id,
        `CREATE TABLE joinery.${table} (id integer PRIMARY KEY, note text NOT NULL); INSERT INTO joinery.${table} VALUES (1,'original'); CREATE TABLE joinery.${marker} (value integer)`,
      );
      await observerSQL(
        request,
        observer.id,
        `CREATE FUNCTION joinery.${fn}(v integer) RETURNS integer MODIFIES SQL DATA BEGIN INSERT INTO joinery.${marker} VALUES(v); DO SLEEP(3 + 0*LENGTH('${fn}')); RETURN 1; END`,
      );
      await page.getByRole("button", { name: "Refresh schema", exact: true }).click();
      const button = page.getByRole("button", { name: table, exact: true });
      if (!(await button.isVisible())) await page.getByRole("button", { name: /^joinery/ }).click();
      await button.click();
      await expect(page.locator(".result-footer")).toContainText("1 rows loaded");
      const filters = page.getByRole("toolbar", { name: "Table filters" });
      await filters.getByLabel("WHERE clause").fill(`joinery.${fn}(id)=1`);
      await filters.getByRole("button", { name: "Run SQL clauses" }).click();
      const review = page.getByRole("dialog", { name: "Review this operation" });
      await expect(review).toBeVisible();
      expect(await scalar(request, observer.id, `SELECT COUNT(*) FROM joinery.${marker}`)).toBe(
        "0",
      );
      const submitted = page.waitForRequest((candidate) =>
        candidate.url().endsWith("/rpc/browse_table"),
      );
      const completed = page.waitForResponse((candidate) =>
        candidate.url().endsWith("/rpc/browse_table"),
      );
      await review.getByRole("button", { name: "Authorize and run", exact: true }).click();
      const payload = (await submitted).postDataJSON();
      await expect
        .poll(() =>
          scalar(
            request,
            observer!.id,
            `SELECT COUNT(*) FROM information_schema.PROCESSLIST WHERE ID <> CONNECTION_ID() AND STATE='User sleep' AND INFO LIKE '%${fn}%'`,
          ),
        )
        .toBe("1");
      // Revoke app guards after the real approved function starts, without
      // changing its table. The actual expiry case is covered by DB fixtures.
      await observerSQL(request, observer.id, "SET @joinery_owned_guard_revocation = 1");
      const response = await (await completed).json();
      expect(response.error).toBeUndefined();
      expect(response.result.result).toMatchObject({
        retainedRows: 1,
        editability: { status: "readOnly" },
      });
      expect(
        response.result.result.columns.every(
          (column: { editable: boolean; insertable: boolean }) =>
            !column.editable && !column.insertable,
        ),
      ).toBe(true);
      await expect(review).toHaveCount(0);
      await expect(page.locator(".result-footer")).toContainText("1 rows loaded");
      await expect(page.locator(".query-error")).toHaveCount(0);
      expect(
        await scalar(request, observer.id, `SELECT COUNT(*) FROM joinery.${marker} WHERE value=1`),
      ).toBe("1");
      expect(await scalar(request, actor.id, "SELECT 7")).toBe("7");
      if (!(await page.locator(".inspector").isVisible()))
        await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
      const box = (await page.locator(".grid-host").boundingBox())!;
      await page.mouse.click(box.x + 224, box.y + 55);
      await expect(page.locator(".inspector h3")).toHaveText("note");
      await expect(page.locator(".inspector-value")).toHaveText("original");
      await page.mouse.dblclick(box.x + 224, box.y + 55);
      const viewer = page.locator("#portal textarea, #portal input").first();
      await expect(viewer).toHaveValue("original");
      await expect(viewer).toBeDisabled();
      await expect(page.getByRole("button", { name: "Review 1 edit", exact: true })).toHaveCount(0);
      const replay = await request.post(`${bridge}/rpc/browse_table`, { headers, data: payload });
      expect((await replay.json()).error).toMatchObject({ code: "approvalExpired" });
      expect(await scalar(request, observer.id, `SELECT COUNT(*) FROM joinery.${marker}`)).toBe(
        "1",
      );
      expect(errors).toEqual([]);
    } finally {
      try {
        if (observer) {
          const cleanupErrors: unknown[] = [];
          for (const sql of [
            `DROP FUNCTION IF EXISTS joinery.${fn}`,
            `DROP TABLE IF EXISTS joinery.${marker}`,
            `DROP TABLE IF EXISTS joinery.${table}`,
          ]) {
            try {
              await observerSQL(request, observer.id, sql);
            } catch (error) {
              cleanupErrors.push(error);
            }
          }
          expect(
            await scalar(
              request,
              observer.id,
              `SELECT COUNT(*) FROM information_schema.TABLES WHERE TABLE_SCHEMA='joinery' AND TABLE_NAME IN ('${table}','${marker}')`,
            ),
          ).toBe("0");
          expect(cleanupErrors).toEqual([]);
          expect(
            await scalar(
              request,
              observer.id,
              `SELECT COUNT(*) FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA='joinery' AND ROUTINE_NAME='${fn}'`,
            ),
          ).toBe("0");
        }
      } finally {
        await rpc(request, "disconnect_fixtures");
      }
    }
  });
}

for (const fixture of fixtures) {
  for (const source of ["browse", "query", "CSV"] as const) {
    test(`${fixture.name} ${source} rejects a reviewed write after application DDL recreates the table`, async ({
      page,
      request,
    }) => {
      test.setTimeout(90000);
      const pageErrors: string[] = [];
      page.on("pageerror", (error) => pageErrors.push(error.message));
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
      const table = `identity_e2e_${randomUUID().replaceAll("-", "")}`;
      let observer: SessionInfo | undefined;
      let created = false;
      const submissions: { planId: string; approval: string; streamId: string }[] = [];
      const importPreparations: Record<string, unknown>[] = [];
      const streams = new Map<string, QueryEvent[]>();
      const reads: Promise<void>[] = [];
      page.on("request", (candidate) => {
        if (candidate.url().endsWith("/rpc/prepare_csv_import_batch"))
          importPreparations.push(candidate.postDataJSON());
        if (candidate.url().endsWith("/rpc/execute_plan"))
          submissions.push(candidate.postDataJSON());
      });
      page.on("response", (response) => {
        if (response.url().startsWith(`${bridge}/events/`)) {
          const id = response.url().split("/").at(-1)!;
          reads.push(
            response.json().then((events: QueryEvent[]) => {
              streams.set(id, [...(streams.get(id) ?? []), ...events]);
            }),
          );
        }
      });
      async function apply(expected: "failed" | "completed") {
        const review = page.getByRole("dialog", { name: "Review pending changes" });
        await expect(review).toBeVisible();
        await expect(review.locator(".sql-preview")).toContainText(table);
        const index = submissions.length;
        await review.getByRole("button", { name: "Apply changes", exact: true }).click();
        await expect
          .poll(async () => {
            await Promise.all(reads);
            const submitted = submissions[index];
            return (
              submitted &&
              streams.get(submitted.streamId)?.find((event) => event.type === "finished")
            );
          })
          .toMatchObject({ status: expected });
        await expect(review).toHaveCount(0);
        const submitted = submissions[index];
        const events = streams.get(submitted.streamId)!;
        if (expected === "failed") {
          expect(
            events.some((event) => event.type === "error" && event.error.code === "conflict"),
          ).toBe(true);
          expect(
            events.some((event) => event.type === "finished" && event.status === "completed"),
          ).toBe(false);
          // The rejected plan's original authorization cannot submit another write.
          const replay = await request.post(`${bridge}/rpc/execute_plan`, {
            headers,
            data: { ...submitted, streamId: randomUUID() },
          });
          expect((await replay.json()).error).toMatchObject({ code: "approvalExpired" });
        }
        return index;
      }
      try {
        await page.goto("/");
        const actor = await connectUI(page, `Identity ${fixture.name} ${source} ${table}`, fixture);
        observer = await rpc<SessionInfo>(request, "connect", { id: actor.connectionId, secrets });
        const create = `CREATE TABLE ${fixture.schema}.${table} (id integer PRIMARY KEY, note text NOT NULL); INSERT INTO ${fixture.schema}.${table} VALUES (1, 'before')`;
        created = true;
        await observerSQL(request, observer.id, create);
        if (source === "query") {
          await page.getByRole("button", { name: "New query tab", exact: true }).click();
          await query(page, fixture.schema, table);
        } else {
          await page.getByRole("button", { name: "Refresh schema", exact: true }).click();
          const button = page.getByRole("button", { name: table, exact: true });
          if (!(await button.isVisible()))
            await page.getByRole("button", { name: new RegExp(`^${fixture.schema}`) }).click();
          await button.click();
          await expect(page.locator(".result-footer")).toContainText("1 rows loaded");
        }
        const importer = page.getByRole("dialog", { name: "Import CSV into table" });
        async function previewCSV() {
          await rpc(request, "test_write_sandbox_file", {
            fileName: `${table}.csv`,
            contents: "id,note\n2,approved\n",
          });
          await page.getByRole("button", { name: "Import CSV…", exact: true }).click();
          await importer.getByRole("button", { name: "Choose CSV file…" }).click();
          await importer
            .getByRole("button", { name: "Review & import 1 row…", exact: true })
            .click();
        }
        if (source === "CSV") await previewCSV();
        else await stageEdit(page, "approved");
        await expect(page.getByRole("dialog", { name: "Review pending changes" })).toBeVisible();
        // A separate console uses the same application backend. MySQL/MariaDB DDL
        // must revoke all its retained generations before replacing this object.
        // The standalone diagnostic separately exercises external DDL and expiry.
        await observerSQL(request, observer.id, `DROP TABLE ${fixture.schema}.${table}; ${create}`);
        const rejectedIndex = await apply("failed");
        if (source === "CSV") {
          await expect(importer.getByRole("alert")).toContainText(
            "No rows from this batch were committed",
          );
          await expect(importer).not.toContainText("Imported 1 row into");
        } else {
          await expect(page.locator(".query-error")).toBeVisible();
          await expect(page.locator(".toast")).not.toContainText("Changes committed");
        }
        expect(await observerRows(request, observer.id, fixture.schema, table)).toEqual([
          ["1", "before"],
        ]);
        expect(submissions.length).toBe(rejectedIndex + 1);

        // A fresh read/preview of the replacement has its own valid identity.
        if (source === "CSV") {
          await importer.getByRole("button", { name: "Close", exact: true }).click();
          await page.getByRole("button", { name: "Refresh", exact: true }).click();
          await expect(page.locator(".result-footer")).toContainText("1 rows loaded");
          await previewCSV();
          const valid = importPreparations.at(-1)!;
          expect(typeof valid.previewToken).toBe("string");
          // Real named-IPC calls must reject absent/null ownership and an unknown
          // generation without disturbing the dialog's valid prepared control.
          for (const token of [undefined, null, ""]) {
            const invalid = { ...valid, previewToken: token };
            if (token === undefined) delete invalid.previewToken;
            const response = await request.post(`${bridge}/rpc/prepare_csv_import_batch`, {
              headers,
              data: invalid,
            });
            const payload = await response.json();
            expect(payload.result).toBeUndefined();
            expect(payload.error).toMatchObject({
              code: token === "" ? "conflict" : "invalidInput",
            });
          }
          expect(await observerRows(request, observer.id, fixture.schema, table)).toEqual([
            ["1", "before"],
          ]);
        } else {
          await page.getByRole("button", { name: "Discard", exact: true }).click();
          if (source === "query") await query(page, fixture.schema, table);
          else {
            const refreshed = page.waitForResponse((response) =>
              response.url().endsWith("/rpc/browse_table"),
            );
            await page.getByRole("button", { name: "Refresh", exact: true }).click();
            expect((await (await refreshed).json()).error).toBeUndefined();
            await expect(page.locator(".result-footer")).toContainText("1 rows loaded");
          }
          await stageEdit(page, "fresh control");
        }
        const controlIndex = await apply("completed");
        if (source === "CSV") {
          await expect(importer).toContainText(`Imported 1 row into ${fixture.schema}.${table}`);
          await importer.getByRole("button", { name: "Done", exact: true }).click();
        } else
          await expect(
            page.getByRole("status").filter({ hasText: "Changes committed" }),
          ).toHaveText("Changes committed");
        expect(await observerRows(request, observer.id, fixture.schema, table)).toEqual(
          source === "CSV"
            ? [
                ["1", "before"],
                ["2", "approved"],
              ]
            : [["1", "fresh control"]],
        );
        expect(submissions.length).toBe(controlIndex + 1);
        expect(pageErrors).toEqual([]);
      } finally {
        try {
          if (created && observer)
            await observerSQL(
              request,
              observer.id,
              `DROP TABLE IF EXISTS ${fixture.schema}.${table}`,
            );
        } finally {
          await rpc(request, "disconnect_fixtures");
        }
      }
    });
  }
}
