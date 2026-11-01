import { expect, test } from "@playwright/test";

for (const [label, engine, port] of [
  ["PostgreSQL", "postgres", Number(process.env.JOINERY_POSTGRES_PORT ?? "55432")],
  ["MySQL", "mysql", 53306],
  ["MariaDB", "mysql", 53307],
] as const) {
  test(`${label} estimated plan uses the query session and switches between tree, raw and grid`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption(engine);
    await page.getByLabel("Connection name", { exact: true }).fill(`${label} query plan`);
    await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
    await page.getByLabel("Port", { exact: true }).fill(String(port));
    await page.getByLabel("Database", { exact: true }).fill("joinery");
    await page.getByLabel("Username", { exact: true }).fill("joinery");
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByLabel("Require TLS and verify the server certificate").uncheck();
    await page.getByLabel("Safe Mode", { exact: true }).selectOption("readOnly");
    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: "New query tab" }).click();
    // Explain must use this tab's isolated session, not the grid/login session.
    await page.getByRole("button", { name: "New isolated session", exact: true }).click();
    await page
      .getByRole("dialog")
      .getByLabel("Password", { exact: true })
      .fill("joinery-local-only");
    await page.getByRole("dialog").getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.locator(".query-session")).toContainText("Isolated session");
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const sql = `SELECT * FROM ${engine === "postgres" ? "public.orders" : "joinery.sample"} LIMIT 1`;
    await editor.fill(sql);
    await page.getByRole("button", { name: "Explain", exact: true }).click();
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await expect(page.locator(".query-error")).toHaveCount(0);
    await expect(editor).toHaveText(sql);
    await page.getByRole("button", { name: "View plan", exact: true }).click();
    const plan = page.getByRole("region", { name: "Query execution plan" });
    await expect(plan).toContainText("Estimated query plan");
    await expect(plan.locator(".plan-tree")).toContainText("Estimated rows");
    await expect(plan).not.toContainText("Actual rows");
    await page.getByRole("button", { name: "Raw JSON", exact: true }).click();
    const raw = await plan.locator(".plan-raw").innerText();
    const parsed = JSON.parse(raw);
    expect(engine === "postgres" ? parsed[0].Plan : parsed.query_block).toBeTruthy();
    await page.getByRole("button", { name: "Tree", exact: true }).click();
    await expect(plan.locator(".plan-tree")).toBeVisible();
    await page.getByRole("button", { name: "Data grid", exact: true }).click();
    await expect(page.locator(".grid-host")).toBeVisible();

    // Explicitly select the full batch: no suffix may run outside EXPLAIN.
    await editor.fill("SELECT 1; SELECT 2");
    await editor.press("ControlOrMeta+a");
    await page.getByRole("button", { name: "Explain", exact: true }).click();
    await expect(page.locator(".query-error")).toContainText("one SELECT or WITH query");
    await expect(page.locator(".result-tabs [role=tab]")).toHaveCount(1);
    expect(errors).toEqual([]);
  });
}
