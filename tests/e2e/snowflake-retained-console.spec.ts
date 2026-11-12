import { expect, test } from "./fixtures";
import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { tableFromIPC } from "apache-arrow";
import type {
  Bootstrap,
  SnowflakeConsolePlan,
  SnowflakeConsoleRunView,
  SnowflakeConsoleRequestLookup,
  SnowflakeConsoleReceiptPage,
  SessionInfo,
  ErrorCode,
} from "../../src/ipc/generated";

// Real browser -> loopback Rust Studio -> real helper/framing/ResultStore.
// Only the SDK is synthetic. No Snowflake login, database effects or performance
// qualification is claimed by this test.
test.use({ actionTimeout: 10_000 });

test("retained console preserves adjacency, recovers a lost start response and restores receipts without replay", async ({
  page,
  request,
}, testInfo) => {
  test.setTimeout(120_000);
  const root = await mkdtemp(path.join(tmpdir(), "joinery-retained-console-"));
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
  const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
  const binary = path.resolve(
    process.env.CARGO_TARGET_DIR ?? "src-tauri/target",
    "debug",
    "test-harness",
  );
  let child: ChildProcess | undefined;
  let diagnostics = "";
  const rpc = async <T>(command: string, args: Record<string, unknown> = {}): Promise<T> => {
    const response = await request.post(`${bridge}/rpc/${command}`, { headers, data: args });
    expect(response.ok()).toBe(true);
    const body = await response.json();
    expect(body.error).toBeUndefined();
    return body.result;
  };
  const start = async () => {
    diagnostics = "";
    child = spawn(binary, [], {
      env: {
        ...process.env,
        JOINERY_TEST_BRIDGE: "1",
        JOINERY_TEST_TOKEN: "joinery-disposable-browser-tests-only",
        JOINERY_TEST_DATA_ROOT: root,
        JOINERY_TEST_PORT: String(port),
        JOINERY_TEST_SYNTHETIC_SNOWFLAKE: "1",
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    child.on("error", (error) => {
      diagnostics += error.message;
    });
    for (const stream of [child.stdout, child.stderr])
      stream?.on("data", (chunk: Buffer) => {
        diagnostics = (diagnostics + chunk.toString()).slice(-4000);
      });
    await expect
      .poll(
        async () => {
          if (child?.exitCode !== null || child?.signalCode !== null)
            throw new Error(`Synthetic console exited: ${diagnostics}`);
          return request
            .get(`${bridge}/health`, { timeout: 1000 })
            .then((response) => response.ok())
            .catch(() => false);
        },
        { timeout: 20_000 },
      )
      .toBe(true);
  };
  const stop = async () => {
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    const exited = once(child, "exit");
    await rpc("test_synthetic_console_shutdown");
    let deadline: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        exited,
        new Promise<never>((_, reject) => {
          deadline = setTimeout(() => reject(new Error("Synthetic shutdown timed out")), 10_000);
        }),
      ]);
    } finally {
      clearTimeout(deadline);
    }
    expect(child.exitCode).toBe(0);
  };
  const commands: string[] = [];
  const starts: string[] = [];
  const reviews: SnowflakeConsolePlan[] = [];
  let pendingBridgeRequests = 0;
  await page.route("http://127.0.0.1:1422/**", async (route) => {
    pendingBridgeRequests += 1;
    try {
      const url = new URL(route.request().url());
      const command = url.pathname.split("/").at(-1)!;
      commands.push(command);
      if (command === "start_snowflake_console")
        starts.push(route.request().postDataJSON().request.clientRequestId);
      const response = await route.fetch({ url: bridge + url.pathname + url.search });
      if (command === "prepare_snowflake_console") reviews.push((await response.json()).result);
      await route.fulfill({ response });
    } finally {
      pendingBridgeRequests -= 1;
    }
  });
  const lookupReceipt = async (id: string) =>
    rpc<SnowflakeConsoleRunView | null>("snowflake_console_run", { clientRequestId: id });
  const receipt = async (id: string) => {
    const view = await lookupReceipt(id);
    expect(view).not.toBeNull();
    return view!;
  };
  const run = async (sql: string) => {
    const before = starts.length;
    await page.getByRole("textbox", { name: "SQL editor" }).fill(sql);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Review this operation" });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText("120-second execution and result-delivery limit");
    expect(reviews.at(-1)?.timeoutSeconds).toBe(120);
    await dialog.getByRole("button", { name: "Authorize and run", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect.poll(() => starts.length).toBe(before + 1);
    const id = starts[before];
    await expect.poll(async () => (await lookupReceipt(id))?.run.state).toBe("terminal");
    await expect.poll(async () => (await lookupReceipt(id))?.pendingRecording).toBe(false);
    return receipt(id);
  };
  const values = async (view: SnowflakeConsoleRunView) => {
    const result = view.results.at(-1)!;
    const response = await request.get(`${bridge}/result/${result.id}?start=0&count=20`, {
      headers,
    });
    expect(response.ok()).toBe(true);
    return tableFromIPC(new Uint8Array(await response.body()))
      .getChildAt(0)
      ?.toArray();
  };
  try {
    await start();
    await page.goto("/");
    const profileName = "Synthetic retained Snowflake console";
    await page.getByRole("button", { name: profileName, exact: true }).hover();
    await page.getByRole("button", { name: `Edit ${profileName}`, exact: true }).click();
    const settings = page.getByRole("dialog", { name: "Edit connection", exact: true });
    const queryLimit = settings.getByLabel("Console query limit (seconds)", { exact: true });
    await expect(queryLimit).toHaveValue("65");
    await queryLimit.fill("120");
    const connecting = page.waitForResponse(
      (response) => response.url() === "http://127.0.0.1:1422/rpc/connect_prepared",
      { timeout: 10_000 },
    );
    await settings.getByRole("button", { name: "Save and connect", exact: true }).click();
    const connected = (await (await connecting).json()) as {
      result?: Pick<SessionInfo, "id">;
      error?: { code: ErrorCode };
    };
    const connectOutcome = {
      errorCode: connected.error?.code ?? null,
      sessionReturned: typeof connected.result?.id === "string" && connected.result.id.length > 0,
    };
    if (connectOutcome.errorCode !== null || !connectOutcome.sessionReturned) {
      // Only this owned synthetic child's bounded diagnostics and the native
      // error code are attached. Native error messages may contain private data.
      await testInfo.attach("synthetic-console-connect-failure", {
        body: JSON.stringify({ ...connectOutcome, helperDiagnostics: diagnostics.slice(-4000) }),
        contentType: "application/json",
      });
    }
    expect(connectOutcome).toEqual({ errorCode: null, sessionReturned: true });
    await expect(settings).toHaveCount(0);
    expect(
      (await rpc<Bootstrap>("bootstrap")).profiles.find((profile) => profile.name === profileName)
        ?.snowflake?.reviewedConsoleTimeoutSeconds,
    ).toBe(120);
    await page.getByRole("button", { name: profileName, exact: true }).hover();
    await expect(
      page.getByRole("button", { name: "Disconnect Synthetic retained Snowflake console" }),
    ).toBeVisible();
    await expect(page.locator(".status-bar")).toContainText("Synthetic retained Snowflake console");
    await page.getByRole("button", { name: "New query tab", exact: true }).click();
    await expect(
      page.getByText(/Reviewed Snowflake console · One retained physical session/),
    ).toBeVisible();
    await expect(page.getByLabel("Transaction mode", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Explain", exact: true })).toHaveCount(0);
    const baseline = await rpc<{ queries: string; sdkEvents: string }>(
      "test_synthetic_console_evidence",
    );
    const seen: string[] = [];
    const check = async (sql: string) => {
      seen.push(
        ...sql
          .split(";")
          .map((unit) => unit.trim())
          .filter(Boolean),
      );
      const view = await run(sql);
      expect(view.run.terminal?.reason).toBe("completed");
      const evidence = await rpc<{ queries: string; sdkEvents: string }>(
        "test_synthetic_console_evidence",
      );
      expect(evidence.queries.slice(baseline.queries.length).trim().split("\n")).toEqual(seen);
      expect(evidence.sdkEvents.split("\n").filter((event) => event === "connect")).toHaveLength(1);
      return view;
    };
    await check("SET CONSOLE_VALUE = 'same session'");
    expect(await values(await check("SELECT $CONSOLE_VALUE"))).toEqual(["same session"]);
    await check("SHOW TABLES");
    expect(await values(await check("SELECT * FROM TABLE(RESULT_SCAN(LAST_QUERY_ID()))"))).toEqual([
      "CONSOLE_TEMP",
    ]);
    await check("CREATE TEMPORARY TABLE CONSOLE_TEMP (V NUMBER)");
    await check("BEGIN");
    await check("INSERT INTO CONSOLE_TEMP VALUES (7)");
    expect(await values(await check("SELECT * FROM CONSOLE_TEMP"))).toEqual(["7"]);
    await check("ROLLBACK");
    expect((await check("SELECT * FROM CONSOLE_TEMP")).results.at(-1)?.retainedRows).toBe(0);

    // Lose only the start HTTP response after actual native admission. Lookup
    // recovers the original run; it never resends SQL or fabricates completion.
    let dropped = false;
    await page.route("http://127.0.0.1:1422/rpc/start_snowflake_console", async (route) => {
      const response = await route.fetch({ url: `${bridge}/rpc/start_snowflake_console` });
      expect(response.ok()).toBe(true);
      starts.push(route.request().postDataJSON().request.clientRequestId);
      dropped = true;
      await route.abort("connectionreset");
    });
    const recovered = await run("SELECT 99");
    expect(dropped).toBe(true);
    expect(await values(recovered)).toEqual(["99"]);
    const count = starts.length;
    const evidence = await rpc<{ queries: string }>("test_synthetic_console_evidence");
    expect(evidence.queries.slice(baseline.queries.length).trim().split("\n")).toEqual([
      ...seen,
      "SELECT 99",
    ]);
    await expect(
      page.getByRole("button", { name: "Run all statements", exact: true }),
    ).toBeEnabled();
    await page.getByRole("textbox", { name: "SQL editor" }).fill("");
    await page.locator(".workspace-tab.active .tab-close").click();
    await page.getByRole("button", { name: /^Console receipts \(/ }).click();
    await page
      .getByRole("button", {
        name: `Receipt ${recovered.run.metadata.clientRequestId}`,
        exact: true,
      })
      .click();
    await expect(page.getByRole("region", { name: "Snowflake console receipts" })).toContainText(
      "acknowledged",
    );
    await page.setViewportSize({ width: 1024, height: 720 });
    const details = page.getByRole("region", { name: "Console receipt details", exact: true });
    await expect(details).toBeVisible();
    await details.focus();
    await details.press("End");
    await expect.poll(() => details.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    await page
      .getByRole("article", { name: `Console receipt ${recovered.run.metadata.clientRequestId}` })
      .getByRole("button", { name: "Open retained results", exact: true })
      .click();
    await expect(page.locator('.workspace-tabs [role="tab"][aria-selected="true"]')).toHaveText(
      "Console retained results",
    );
    await expect(page.getByRole("textbox", { name: "SQL editor" })).toHaveText("");
    expect(await values(await receipt(recovered.run.metadata.clientRequestId))).toEqual(["99"]);
    const originalRun = recovered.run;
    const recoveredArticle = page.getByRole("article", {
      name: `Console receipt ${recovered.run.metadata.clientRequestId}`,
    });
    await expect(
      recoveredArticle.getByRole("button", { name: "Release retained result rows", exact: true }),
    ).toBeEnabled();
    await recoveredArticle
      .getByRole("button", { name: "Release retained result rows", exact: true })
      .click();
    await recoveredArticle
      .getByRole("button", { name: "Confirm row release", exact: true })
      .click();
    await expect
      .poll(async () => (await receipt(recovered.run.metadata.clientRequestId)).results.length)
      .toBe(0);
    expect((await receipt(recovered.run.metadata.clientRequestId)).run).toEqual(originalRun);
    expect(
      (
        await request.get(`${bridge}/result/${recovered.results.at(-1)!.id}?start=0&count=1`, {
          headers,
        })
      ).ok(),
    ).toBe(false);
    expect(commands).not.toContain("release_result");
    expect(
      commands.some((command) =>
        [
          "health",
          "session_contexts",
          "session_state",
          "completion_catalog",
          "relation_catalog",
          "execute_plan",
        ].includes(command),
      ),
    ).toBe(false);

    const ids = [...starts];
    // The native request really lacks approval, and commits its refusal before
    // this route discards the response. The browser must recover that SQLite
    // proof without repeating start or sending any helper SQL.
    await page.unroute("http://127.0.0.1:1422/rpc/start_snowflake_console");
    let refusedId = "";
    await page.route("http://127.0.0.1:1422/rpc/start_snowflake_console", async (route) => {
      const body = route.request().postDataJSON();
      refusedId = body.request.clientRequestId;
      starts.push(refusedId);
      const response = await route.fetch({
        url: `${bridge}/rpc/start_snowflake_console`,
        postData: JSON.stringify({ ...body, request: { ...body.request, approval: null } }),
      });
      expect(response.ok()).toBe(true);
      const result = (await response.json()).result;
      expect(result.kind).toBe("notAdmitted");
      expect(result.clientRequestId).toBe(refusedId);
      expect(result.code).toBe("approvalRequired");
      await route.abort("connectionreset");
    });
    await page.getByRole("textbox", { name: "SQL editor" }).fill("SELECT 12345");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    const review = page.getByRole("dialog", { name: "Review this operation" });
    await review.getByRole("button", { name: "Authorize and run", exact: true }).click();
    await expect(review).toHaveCount(0);
    await expect.poll(() => refusedId).not.toBe("");
    await expect
      .poll(
        async () =>
          (
            await rpc<SnowflakeConsoleRequestLookup>("snowflake_console_request", {
              clientRequestId: refusedId,
            })
          ).kind,
      )
      .toBe("notAdmitted");
    await expect(
      page.getByRole("button", { name: "Run all statements", exact: true }),
    ).toBeEnabled();
    expect((await rpc<{ queries: string }>("test_synthetic_console_evidence")).queries).toBe(
      evidence.queries,
    );
    expect(starts).toHaveLength(count + 1);
    await page.getByRole("textbox", { name: "SQL editor" }).fill("");
    const negative = page.getByRole("article", { name: `Console receipt ${refusedId}` });
    await page.getByRole("button", { name: "Refresh receipts", exact: true }).click();
    await page.getByRole("button", { name: `Receipt ${refusedId}`, exact: true }).click();
    await expect(negative).toContainText("No SQL was admitted");
    await details.focus();
    await details.press("Home");
    await expect(negative).toBeInViewport();
    const reconcileRefusal = async () => {
      const response = page.waitForResponse(
        (response) =>
          response.url().endsWith("/rpc/snowflake_console_request") &&
          response.request().postDataJSON().clientRequestId === refusedId,
      );
      await negative
        .getByRole("button", { name: "Reconcile receipt (no SQL)", exact: true })
        .click();
      expect((await (await response).json()).result).toMatchObject({
        kind: "notAdmitted",
        clientRequestId: refusedId,
        code: "approvalRequired",
      });
    };
    const lookups = commands.filter((command) => command === "snowflake_console_request").length;
    await reconcileRefusal();
    await expect
      .poll(() => commands.filter((command) => command === "snowflake_console_request").length)
      .toBeGreaterThan(lookups);
    expect((await rpc<{ queries: string }>("test_synthetic_console_evidence")).queries).toBe(
      evidence.queries,
    );
    // More than one page and more than the full-detail cache. All account SQL
    // still goes only to this isolated synthetic SDK.
    await page.unroute("http://127.0.0.1:1422/rpc/start_snowflake_console");
    for (let n = 0; n < 15; n++) {
      const extra = await run("SELECT 1");
      ids.push(extra.run.metadata.clientRequestId);
    }
    await page.getByRole("textbox", { name: "SQL editor" }).fill("");
    await page.getByRole("button", { name: "Refresh receipts", exact: true }).click();
    const inventory = page.getByRole("list", { name: "Durable console receipt inventory" });
    await expect(inventory.getByRole("listitem")).toHaveCount(25);
    await page.getByRole("button", { name: "Next receipts", exact: true }).click();
    await expect(inventory.getByRole("listitem")).toHaveCount(2);
    await page.getByRole("button", { name: "Previous receipts", exact: true }).click();
    await expect(inventory.getByRole("listitem")).toHaveCount(25);
    const noSqlBeforeArchive = await rpc("test_synthetic_console_evidence");
    await page.route(
      "http://127.0.0.1:1422/rpc/set_snowflake_console_receipt_archive",
      async (route) => {
        commands.push("set_snowflake_console_receipt_archive");
        const response = await route.fetch({
          url: `${bridge}/rpc/set_snowflake_console_receipt_archive`,
        });
        expect((await response.json()).error).toBeUndefined();
        await route.abort("connectionreset");
      },
    );
    const refusalRow = inventory
      .getByRole("listitem")
      .filter({ has: page.getByRole("button", { name: `Receipt ${refusedId}`, exact: true }) });
    await refusalRow.getByRole("button", { name: "Archive receipt", exact: true }).click();
    await expect(page.getByText(/Current local preference is archived/)).toBeVisible();
    await expect(page.getByRole("button", { name: "Refresh receipts", exact: true })).toBeFocused();
    await page.unroute("http://127.0.0.1:1422/rpc/set_snowflake_console_receipt_archive");
    await page.getByLabel("Receipt visibility", { exact: true }).selectOption("archived");
    await expect(inventory.getByRole("listitem")).toHaveCount(1);
    await inventory.getByRole("button", { name: "Unarchive receipt", exact: true }).click();
    await expect(inventory.getByRole("listitem")).toHaveCount(0);
    await page.getByLabel("Receipt visibility", { exact: true }).selectOption("all");
    expect(await rpc("test_synthetic_console_evidence")).toEqual(noSqlBeforeArchive);
    const allReceipts = await rpc<SnowflakeConsoleReceiptPage>("snowflake_console_receipts", {
      request: {
        filter: { visibility: "all", kind: null, connectionId: null },
        cursor: null,
        limit: 100,
      },
    });
    expect(allReceipts.receipts).toHaveLength(27);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            JSON.parse(localStorage.getItem("joinery.snowflakeConsoleReceipts.v1") ?? "[]").length,
        ),
      )
      .toBe(0);
    await page.goto("about:blank");
    await expect.poll(() => pendingBridgeRequests).toBe(0);
    await stop();
    await start();
    commands.length = 0;
    await page.goto("/");
    expect(
      (await rpc<Bootstrap>("bootstrap")).profiles.find((profile) => profile.name === profileName)
        ?.snowflake?.reviewedConsoleTimeoutSeconds,
    ).toBe(120);
    await expect(page.getByRole("region", { name: "Snowflake console receipts" })).toBeVisible();
    for (const id of ids) {
      const restored = await receipt(id);
      expect(restored.run.state).toBe("terminal");
      expect(restored.results).toEqual([]);
    }
    const afterRestart = await rpc<{ queries: string; sdkEvents: string }>(
      "test_synthetic_console_evidence",
    );
    expect(afterRestart.queries).toBe("");
    expect(afterRestart.sdkEvents).toBe("");
    const restoredNegative = await rpc<SnowflakeConsoleRequestLookup>("snowflake_console_request", {
      clientRequestId: refusedId,
    });
    expect(restoredNegative).toMatchObject({
      kind: "notAdmitted",
      clientRequestId: refusedId,
      code: "approvalRequired",
    });
    expect(starts).toHaveLength(count + 16);
    expect(
      commands.some((command) =>
        [
          "connect",
          "connect_prepared",
          "prepare_snowflake_console",
          "start_snowflake_console",
        ].includes(command),
      ),
    ).toBe(false);
    await page.getByRole("button", { name: /^Console receipts \(/ }).click();
    await page.getByRole("button", { name: `Receipt ${ids.at(-1)}`, exact: true }).click();
    await expect(page.getByRole("region", { name: "Snowflake console receipts" })).toContainText(
      "does not mean the statements returned zero rows",
    );
    await page.getByRole("button", { name: `Receipt ${refusedId}`, exact: true }).click();
    await expect(negative).toContainText("No SQL was admitted");
    await reconcileRefusal();
    expect(await rpc("test_synthetic_console_evidence")).toMatchObject({
      queries: "",
      sdkEvents: "",
    });
  } finally {
    await page.close();
    try {
      await stop();
    } finally {
      if (child && child.exitCode === null && child.signalCode === null) {
        const exited = once(child, "exit");
        child.kill("SIGKILL");
        await exited;
      }
      await rm(root, { recursive: true, force: true });
    }
  }
});
