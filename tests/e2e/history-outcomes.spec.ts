import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdtempSync, mkdirSync, openSync, closeSync } from "node:fs";
import { createServer } from "node:net";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { randomUUID } from "node:crypto";
import { test, expect } from "./fixtures";
import type { HistoryEntry } from "../../src/ipc/generated";

const token = "joinery-disposable-browser-tests-only";
const headers = { Authorization: `Bearer ${token}`, "Content-Type": "application/json" };

async function unusedPort() {
  const server = createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("No isolated port");
  const port = address.port;
  await new Promise<void>((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
  return port;
}

async function stop(child: ChildProcess | undefined) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exit = once(child, "exit");
  child.kill("SIGKILL");
  await exit;
}

test("captured query outcome survives backend death and reopens in history without replay", async ({
  page,
}) => {
  test.setTimeout(60000);
  mkdirSync(".cache", { recursive: true });
  const root = mkdtempSync(path.resolve(".cache/history-outcome-browser-"));
  const port = await unusedPort();
  const base = `http://127.0.0.1:${port}`;
  let child: ChildProcess | undefined;
  let generation = 0;
  let activeGeneration: number | undefined;
  const retiredGenerations = new Set<number>();
  const forwarding = new Set<Promise<void>>();
  let tearingDown = false;
  async function retire() {
    if (activeGeneration !== undefined) retiredGenerations.add(activeGeneration);
    await stop(child);
  }
  async function start() {
    const nextGeneration = generation++;
    const log = openSync(path.join(root, `harness-${nextGeneration}.log`), "wx");
    try {
      child = spawn(path.resolve("src-tauri/target/debug/test-harness"), [], {
        env: {
          ...process.env,
          JOINERY_TEST_BRIDGE: "1",
          JOINERY_TEST_TOKEN: token,
          JOINERY_TEST_DATA_ROOT: root,
          JOINERY_TEST_PORT: String(port),
          JOINERY_POSTGRES_PORT: process.env.JOINERY_POSTGRES_PORT ?? "55432",
        },
        stdio: ["ignore", log, log],
      });
    } finally {
      closeSync(log);
    }
    await expect
      .poll(async () => {
        if (child?.exitCode !== null) throw new Error("Isolated harness exited; inspect its log");
        return fetch(`${base}/health`)
          .then((r) => r.ok)
          .catch(() => false);
      })
      .toBe(true);
    activeGeneration = nextGeneration;
  }
  async function rpc<T>(command: string, args = {}): Promise<T> {
    const response = await fetch(`${base}/rpc/${command}`, {
      method: "POST",
      headers,
      body: JSON.stringify(args),
    });
    const body = await response.json();
    expect(body.error).toBeUndefined();
    return body.result;
  }
  const dbPath = path.join(root, "joinery.sqlite");
  try {
    await start();
    // Route only this page to our explicitly owned, restartable backend. The
    // shared Playwright server and all user processes remain untouched.
    await page.route("http://127.0.0.1:1422/**", (route) => {
      // A failed fetch may finish after restart; retain the generation it used.
      const ownedGeneration = activeGeneration;
      const retired = () =>
        ownedGeneration !== undefined && retiredGenerations.has(ownedGeneration);
      const task = (async () => {
        try {
          if (retired()) {
            await route.abort("connectionfailed");
            return;
          }
          let response;
          try {
            response = await route.fetch({
              url: route.request().url().replace("http://127.0.0.1:1422", base),
            });
          } catch (error) {
            if (
              !retired() ||
              !(error instanceof Error) ||
              !/\b(?:ECONNRESET|ECONNREFUSED|EPIPE)\b|socket hang up/.test(error.message)
            ) {
              throw error;
            }
            await route.abort("connectionfailed");
            return;
          }
          await route.fulfill({ response });
        } catch (error) {
          if (
            !tearingDown ||
            !page.isClosed() ||
            !retired() ||
            !(error instanceof Error) ||
            !/Target page, context or browser has been closed/.test(error.message)
          ) {
            throw error;
          }
        }
      })();
      forwarding.add(task);
      return task.finally(() => forwarding.delete(task));
    });
    await page.goto("/");
    await page.getByRole("button", { name: "New connection", exact: true }).click();
    const name = `Durable history ${randomUUID()}`;
    await page.getByLabel("Connection name", { exact: true }).fill(name);
    await page.getByLabel("Database engine").selectOption("postgres");
    await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
    await page
      .getByLabel("Port", { exact: true })
      .fill(process.env.JOINERY_POSTGRES_PORT ?? "55432");
    await page.getByLabel("Database", { exact: true }).fill("joinery");
    await page.getByLabel("Username", { exact: true }).fill("joinery");
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByLabel("Require TLS and verify the server certificate").uncheck();
    await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
    await page.getByLabel(/Save SQL text in local history/).check();
    await page.getByRole("button", { name: "Save and connect", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();
    const sql = "/* exact original 雪 */ SELECT 'history recovery' AS marker";
    await page.getByRole("textbox", { name: "SQL editor" }).fill(sql);
    const database = new DatabaseSync(dbPath);
    try {
      database.exec(
        "CREATE TRIGGER reject_browser_history BEFORE UPDATE ON history_v8 BEGIN SELECT RAISE(ABORT,'isolated browser fault'); END;",
      );
    } finally {
      database.close();
    }
    const admitted = page.waitForResponse((r) => r.url().endsWith("/rpc/execute_plan"));
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    const admission = await (await admitted).json();
    expect(admission.error).toBeUndefined();
    const id = admission.result as string;
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await expect(page.locator(".toast")).toContainText("original outcome was captured locally");
    const pending = await rpc<HistoryEntry>("history_entry", { id });
    expect(pending).toMatchObject({ provenance: "admitted", status: null, sql });
    const before = new DatabaseSync(dbPath);
    let original: { phase: string; payload_json: string; capture_digest: string };
    try {
      original = before
        .prepare(
          "SELECT phase,payload_json,capture_digest FROM outcome_operations_v7 WHERE operation_id=?",
        )
        .get(id) as typeof original;
      expect(original.phase).toBe("captured");
      expect(original.capture_digest).toMatch(/^[a-f0-9]{64}$/);
    } finally {
      before.close();
    }
    await retire();
    const repair = new DatabaseSync(dbPath);
    try {
      repair.exec("DROP TRIGGER reject_browser_history;");
    } finally {
      repair.close();
    }
    await start();
    const expected = JSON.parse(original.payload_json).completion;
    const restored = await rpc<HistoryEntry>("history_entry", { id });
    // History omits absent typed evidence; the compact payload encodes null.
    // Compare every completion field after applying only that wire convention.
    expect({
      id: restored.id,
      sql: restored.sql,
      provenance: restored.provenance,
      status: restored.status,
      elapsedMs: restored.elapsedMs,
      executionSummary: restored.executionSummary,
      snowflakeDml: restored.snowflakeDml ?? null,
      snowflakeDdl: restored.snowflakeDdl ?? null,
    }).toEqual({ ...expected, id, sql });
    const commands: string[] = [];
    page.on("request", (r) => {
      if (r.url().includes("/rpc/")) commands.push(r.url().split("/rpc/")[1]);
    });
    await page.reload();
    await page.getByRole("button", { name: "Query history", exact: true }).click();
    await page.getByRole("textbox", { name: "Search history", exact: true }).fill(name);
    const row = page.locator(".history-list .history-entry");
    await expect(row).toHaveCount(1);
    await expect(row).toContainText("Completed");
    await expect(row).not.toContainText("Completion unknown");
    await row.click();
    await expect
      .poll(() =>
        page.evaluate(async () => {
          const modulePath = "/src/state/studio.ts";
          const { useStudio } = await import(modulePath);
          const state = useStudio.getState();
          return state.tabs.find((tab: { id: string }) => tab.id === state.activeTabId)?.sql;
        }),
      )
      .toBe(sql);
    expect(commands.filter((c) => /^(connect|prepare_execution|execute_plan)/.test(c))).toEqual([]);
    for (let restart = 0; restart < 2; restart++) {
      const audits = await rpc<{ id: string }[]>("audit_log");
      expect(audits.filter((a) => a.id === `${id}:finished`)).toHaveLength(1);
      expect(await rpc("history_entry", { id })).toEqual(restored);
      if (restart === 0) {
        await retire();
        await start();
      }
    }
  } finally {
    tearingDown = true;
    try {
      await retire();
    } finally {
      try {
        // Keep interception installed until this page closes: no cleanup request
        // may fall through to the shared backend while owned routes drain.
        await page.close();
      } finally {
        await Promise.all([...forwarding]);
      }
    }
  }
});
