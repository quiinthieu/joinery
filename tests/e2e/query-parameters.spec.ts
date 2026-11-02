import { expect, test, type Page } from "@playwright/test";

async function connect(page: Page, engine: "PostgreSQL" | "MySQL" | "MariaDB") {
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(`${engine} parameters`);
  await page
    .getByLabel("Database engine")
    .selectOption(engine === "PostgreSQL" ? "postgres" : "mysql");
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page
    .getByLabel("Port", { exact: true })
    .fill(
      engine === "PostgreSQL"
        ? (process.env.JOINERY_POSTGRES_PORT ?? "55432")
        : engine === "MySQL"
          ? "53306"
          : "53307",
    );
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("confirm");
  await page.getByLabel(/Save SQL text in local history/).check();
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
}

for (const engine of ["PostgreSQL", "MySQL", "MariaDB"] as const) {
  test(`${engine} binds reviewed writes, explicit NULL and injection text without persisting values`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await connect(page, engine);
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const table = "r5_browser_parameters";
    const secret = "PARAM_SECRET_881'; DROP TABLE r5_browser_parameters; -- 🧪";
    await editor.fill(
      `CREATE TEMPORARY TABLE ${table} (id BIGINT PRIMARY KEY, note VARCHAR(200), empty_value VARCHAR(20))`,
    );
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await page.getByRole("button", { name: "Authorize and run", exact: true }).click();
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await editor.fill(`INSERT INTO ${table} VALUES (:id, :secret, :nil)`);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    const params = page.getByRole("dialog", { name: "Query parameters" });
    await params.getByLabel("Type for parameter id").selectOption("integer");
    await params.getByLabel("Value for parameter id").fill("9007199254740993");
    await params.getByLabel("Value for parameter secret").fill(secret);
    await params.getByLabel("NULL for parameter nil").check();
    await params.getByRole("button", { name: "Run query", exact: true }).click();
    const review = page.getByRole("dialog", { name: "Review this operation" });
    await expect(review.getByLabel("Bound query parameters")).toContainText(secret);
    await expect(review.getByLabel("Bound query parameters")).toContainText("NULL");
    await review.getByRole("button", { name: "Authorize and run", exact: true }).click();
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await editor.fill(`SELECT id, note, empty_value FROM ${table} WHERE id = :lookup`);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await params.getByLabel("Type for parameter lookup").selectOption("integer");
    await params.getByLabel("Value for parameter lookup").fill("9007199254740993");
    await params.getByRole("button", { name: "Run query", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("1 rows");
    await expect(page.locator(".query-error")).toHaveCount(0);
    await page.getByRole("button", { name: "Query history", exact: true }).click();
    await expect(page.locator(".history-list")).toContainText(":secret");
    await expect(page.locator(".history-list")).not.toContainText(secret);
    expect(errors).toEqual([]);
  });

  test(`${engine} parameters flow through Explain and Analyze with binding-aware snapshots`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await connect(page, engine);
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const query = `SELECT * FROM ${engine === "PostgreSQL" ? "orders" : "sample"} WHERE id = :lookup`;
    await editor.fill(query);
    const run = async (mode: "Explain" | "Analyze", value: string) => {
      await page.getByRole("button", { name: mode, exact: true }).click();
      const params = page.getByRole("dialog", { name: "Query parameters" });
      await params.getByLabel("Type for parameter lookup").selectOption("integer");
      await params.getByLabel("Value for parameter lookup").fill(value);
      await params.getByRole("button", { name: "Run query", exact: true }).click();
      if (mode === "Analyze") {
        const review = page.getByRole("dialog", { name: "Review this operation" });
        await expect(review).toContainText("executes");
        await expect(review.getByLabel("Bound query parameters")).toContainText(value);
        await review.getByRole("button", { name: "Authorize and run", exact: true }).click();
      }
      await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
      await expect(page.locator(".query-error")).toHaveCount(0);
      await page.getByRole("button", { name: "View plan", exact: true }).click();
    };
    await run("Explain", "1");
    await page.getByRole("button", { name: "Keep snapshot", exact: true }).click();
    await run("Analyze", "1");
    const baseline = page.getByLabel("Compare with snapshot");
    const snapshotId = (await baseline.locator("option").nth(1).getAttribute("value"))!;
    await baseline.selectOption(snapshotId);
    await expect(page.getByRole("region", { name: "Plan comparison", exact: true })).toBeVisible();
    await run("Explain", "2");
    const mismatch = baseline.locator(`option[value="${snapshotId}"]`);
    await expect(mismatch).toHaveAttribute("disabled", "");
    await expect(mismatch).toContainText("Different bound parameters");
    await expect(editor).toHaveText(query);
    expect(errors).toEqual([]);
  });
}
