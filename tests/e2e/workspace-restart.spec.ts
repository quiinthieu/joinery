import { expect, test } from "./fixtures";
import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";

test("backend process restart restores opted-in drafts without sessions, results, or SQL replay", async ({
  page,
  request,
}) => {
  test.setTimeout(90000);
  const root = await mkdtemp(path.join(tmpdir(), "joinery-restart-"));
  const reservation = createServer();
  reservation.listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  if (!address || typeof address === "string") throw new Error("Missing fixture port");
  const port = address.port;
  await new Promise<void>((resolve, reject) =>
    reservation.close((error) => (error ? reject(error) : resolve())),
  );
  const bridge = `http://127.0.0.1:${port}`;
  const token = "joinery-disposable-browser-tests-only";
  const binary = path.resolve(
    process.env.CARGO_TARGET_DIR ?? "src-tauri/target",
    "debug",
    process.platform === "win32" ? "test-harness.exe" : "test-harness",
  );
  let child: ChildProcess | undefined;
  let diagnostics = "";
  const stop = async () => {
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    const exited = once(child, "exit");
    // Deliberate abrupt loss: this must reopen persisted SQLite/WAL state,
    // rather than benefiting from an orderly shutdown or in-memory sessions.
    child.kill("SIGKILL");
    await exited;
  };
  const start = async () => {
    diagnostics = "";
    child = spawn(binary, [], {
      env: {
        ...process.env,
        JOINERY_TEST_BRIDGE: "1",
        JOINERY_TEST_TOKEN: token,
        JOINERY_TEST_DATA_ROOT: root,
        JOINERY_TEST_PORT: String(port),
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    child.on("error", (error) => {
      diagnostics += error.message;
    });
    for (const stream of [child.stdout, child.stderr]) {
      stream?.on("data", (chunk: Buffer) => {
        diagnostics = (diagnostics + chunk.toString()).slice(-4000);
      });
    }
    await expect
      .poll(
        async () => {
          if (child?.exitCode !== null || child?.signalCode !== null)
            throw new Error(`Restart fixture exited: ${diagnostics}`);
          return request
            .get(`${bridge}/health`, { timeout: 1000 })
            .then((response) => response.ok())
            .catch(() => false);
        },
        { timeout: 15000 },
      )
      .toBe(true);
  };
  const rpc = async (command: string) => {
    const response = await request.post(`${bridge}/rpc/${command}`, {
      headers: { Authorization: `Bearer ${token}` },
      data: {},
    });
    expect(response.ok()).toBe(true);
    const payload = await response.json();
    expect(payload.error).toBeUndefined();
    return payload.result;
  };
  // Forward every bridge byte to a real isolated Rust process; no RPC or
  // database response is synthesized. The shared suite backend stays running.
  await page.route("http://127.0.0.1:1422/**", async (route) => {
    const url = new URL(route.request().url());
    const response = await route.fetch({ url: bridge + url.pathname + url.search });
    await route.fulfill({ response });
  });
  try {
    await start();
    await page.goto("/");
    await page.getByRole("button", { name: "New connection", exact: true }).click();
    await page.getByLabel("Connection name", { exact: true }).fill("Restart fixture");
    await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
    await page
      .getByLabel("Port", { exact: true })
      .fill(process.env.JOINERY_POSTGRES_PORT ?? "55432");
    await page.getByLabel("Database", { exact: true }).fill("joinery");
    await page.getByLabel("Username", { exact: true }).fill("joinery");
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByLabel("Require TLS and verify the server certificate").uncheck();
    await page.getByLabel(/Restore open query drafts after restart/).check();
    await page.getByRole("button", { name: "Save and connect", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: "New query tab", exact: true }).click();
    const sql = "SELECT reverse('restart_fixture_result') AS private_result;";
    await page.getByLabel("Transaction mode", { exact: true }).selectOption("manual");
    await page.getByRole("textbox", { name: "SQL editor" }).fill(sql);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await expect(page.locator(".result-footer")).toContainText("1 rows");
    await expect(page.locator(".tx-badge")).toHaveText("In transaction");
    await expect.poll(async () => (await rpc("resource_usage")).sessionCount).toBe(1);
    expect((await rpc("resource_usage")).resultCount).toBeGreaterThan(0);
    await page.getByRole("button", { name: /^Preferences/ }).click();
    await page.getByRole("button", { name: "Paper light", exact: true }).click();
    await page.getByLabel("Display density", { exact: true }).selectOption("compact");
    await page.getByRole("button", { name: "Done", exact: true }).click();
    await expect
      .poll(async () => {
        const workspace = (await rpc("bootstrap")).workspace;
        return (
          workspace?.theme === "light" &&
          workspace?.density === "compact" &&
          workspace?.drafts.some((draft: { sql: string }) => draft.sql === sql)
        );
      })
      .toBe(true);

    const originalPid = child!.pid;
    // No Disconnect action: the process still owns an open transaction/result.
    await stop();
    await start();
    expect(child!.pid).not.toBe(originalPid);
    const forbidden: string[] = [];
    page.on("request", (req) => {
      if (/\/rpc\/(connect|prepare_execution|execute_plan)$/.test(req.url()))
        forbidden.push(req.url());
    });
    await page.reload();
    await expect(page.getByRole("textbox", { name: "SQL editor" })).toHaveText(sql);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.locator("html")).toHaveAttribute("data-density", "compact");
    await expect(page.locator(".status-bar")).toContainText("disconnected");
    await expect(page.locator(".result-footer")).toContainText("0 rows");
    await expect(page.locator(".result-tabs [role=tab]")).toHaveCount(0);
    await expect(page.locator(".tx-badge")).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Run all statements", exact: true }),
    ).toBeDisabled();
    expect(await rpc("resource_usage")).toMatchObject({
      sessionCount: 0,
      resultCount: 0,
      retainedBytes: 0,
      activeRuns: 0,
    });
    expect(forbidden).toEqual([]);
  } finally {
    try {
      await page.close();
    } finally {
      await stop();
      await rm(root, { recursive: true, force: true });
    }
  }
});
