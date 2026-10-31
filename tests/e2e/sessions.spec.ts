import { expect, test } from "@playwright/test";

const PG_PORT = Number(process.env.JOINERY_POSTGRES_PORT ?? "55432");

for (const [engine, port] of [
  ["PostgreSQL", PG_PORT],
  ["MySQL", 53306],
  ["MariaDB", 53307],
] as const) {
  test(engine + " query tabs isolate session state and cancellation", async ({ page, request }) => {
    test.setTimeout(90000);
    const errors: string[] = [];
    let executions = 0;
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("request", (request) => {
      if (request.url().endsWith("/rpc/execute_plan")) executions++;
    });
    const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
    const rpc = async (command: string) => {
      const response = await request.post("http://127.0.0.1:1422/rpc/" + command, {
        headers,
        data: {},
      });
      expect(response.ok()).toBe(true);
      return (await response.json()).result;
    };
    await rpc("disconnect_fixtures");
    const postgres = engine === "PostgreSQL";
    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption(postgres ? "postgres" : "mysql");
    await page.getByLabel("Connection name", { exact: true }).fill(engine + " session fixture");
    await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
    await page.getByLabel("Port", { exact: true }).fill(String(port));
    await page.getByLabel("Database", { exact: true }).fill("joinery");
    await page.getByLabel("Username", { exact: true }).fill("joinery");
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByLabel("Safe Mode", { exact: true }).selectOption("silent");
    await page.getByLabel("Require TLS and verify the server certificate").uncheck();
    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: "New query tab" }).click();
    const sharedId = (await page.getByRole("tab", { selected: true }).getAttribute("id"))!;
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const selectMarker = postgres
      ? "SELECT current_setting('application_name')"
      : "SELECT @joinery_console_marker";
    const setMarker = (value: string) =>
      postgres
        ? "SET application_name='" + value + "'"
        : "SET @joinery_console_marker='" + value + "'";
    const run = async (sql: string, value?: string, statements = 1) => {
      const results = page.locator(".result-tabs [role=tab]");
      const previous = (await results.count()) ? await results.last().getAttribute("id") : null;
      await editor.fill(sql);
      await page.getByRole("button", { name: "Run all statements", exact: true }).click();
      await expect
        .poll(async () => {
          const id = (await results.count()) ? await results.last().getAttribute("id") : null;
          return Boolean(id && id !== previous);
        })
        .toBe(true);
      await expect(results).toHaveCount(statements);
      await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
      await results.last().click();
      await expect(page.locator(".result-footer")).toContainText("1 rows");
      await expect(page.locator(".query-error")).toHaveCount(0);
      if (value) {
        if (!(await page.locator(".inspector").isVisible()))
          await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
        const box = (await page.locator(".grid-host").boundingBox())!;
        await expect(async () => {
          await page.mouse.click(box.x + 90, box.y + 55);
          await expect(page.locator(".inspector-value")).toContainText(value, { timeout: 500 });
        }).toPass();
      }
    };
    await run(setMarker("shared") + "; " + selectMarker, "shared", 2);
    await page.getByRole("button", { name: "New query tab" }).click();
    const isolatedId = (await page.getByRole("tab", { selected: true }).getAttribute("id"))!;
    await expect(
      page.getByRole("button", { name: "New isolated session", exact: true }),
    ).toBeVisible();
    const signIn = async () => {
      await page.getByRole("button", { name: /^(New|Reconnect) isolated session$/ }).click();
      await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
      await page.getByRole("button", { name: "Connect", exact: true }).click();
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect(page.locator(".query-session")).toContainText("Isolated session");
      await expect.poll(async () => (await rpc("resource_usage")).sessionCount).toBe(2);
    };
    if (postgres) {
      let resume!: () => void, ready!: () => void;
      const held = new Promise<void>((resolve) => {
        resume = resolve;
      });
      const prepared = new Promise<void>((resolve) => {
        ready = resolve;
      });
      await page.route(
        "http://127.0.0.1:1422/rpc/prepare_execution",
        async (route) => {
          const response = await route.fetch();
          ready();
          await held;
          await route.fulfill({ response });
        },
        { times: 1 },
      );
      const before = executions;
      await editor.fill("SELECT 'stale shared plan'");
      await page.getByRole("button", { name: "Run all statements", exact: true }).click();
      await prepared;
      try {
        await signIn();
      } finally {
        resume();
      }
      await expect(page.locator(".toast")).toContainText("query session changed");
      expect(executions).toBe(before);
      await expect(page.locator(".result-tabs [role=tab]")).toHaveCount(0);
    } else await signIn();
    await run("BEGIN; " + setMarker("private") + "; " + selectMarker, "private", 3);
    await page.screenshot({
      path: ".cache/screenshots/isolated-" + engine.toLowerCase() + ".png",
      fullPage: true,
    });
    await page.locator("#" + sharedId).click();
    await expect(page.locator(".query-session")).toContainText("Shared session");
    await run(selectMarker, "shared");
    await page.locator("#" + isolatedId).click();
    await editor.fill(postgres ? "SELECT pg_sleep(30)" : "SELECT SLEEP(30)");
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".cancel-query")).toBeVisible();
    await page.locator("#" + sharedId).click();
    await run(selectMarker, "shared");
    await page.locator("#" + isolatedId).click();
    await expect(page.locator(".cancel-query")).toBeVisible();
    await page.locator(".cancel-query").click();
    await expect(page.locator(".query-session")).toContainText("disconnected");
    await expect(
      page.getByRole("button", { name: "Run all statements", exact: true }),
    ).toBeDisabled();
    await expect.poll(async () => (await rpc("resource_usage")).sessionCount).toBe(1);
    await page.locator("#" + sharedId).click();
    await run(selectMarker, "shared");
    await page.locator("#" + isolatedId).click();
    await signIn();
    await run("BEGIN; SELECT 1", undefined, 2);
    const close = page
      .locator("#" + isolatedId)
      .locator("..")
      .getByRole("button", { name: /^Close / });
    await close.click();
    await expect(page.getByRole("dialog")).toContainText("transactions");
    await page.getByRole("button", { name: "Keep working", exact: true }).click();
    await expect.poll(async () => (await rpc("resource_usage")).sessionCount).toBe(2);
    await close.click();
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    await expect(page.locator("#" + isolatedId)).toHaveCount(0);
    await expect.poll(async () => (await rpc("resource_usage")).sessionCount).toBe(1);
    await run(selectMarker, "shared");
    expect(errors).toEqual([]);
    await rpc("disconnect_fixtures");
  });
}
