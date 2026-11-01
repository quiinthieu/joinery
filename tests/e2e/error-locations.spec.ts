import { expect, test } from "@playwright/test";
import type { QueryEvent } from "../../src/ipc/generated";

for (const [engine, port] of [
  ["PostgreSQL", Number(process.env.JOINERY_POSTGRES_PORT ?? "55432")],
  ["MySQL", 53306],
  ["MariaDB", 53307],
] as const) {
  test(`${engine} delivers Unicode batch diagnostics and recovers after correction`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/");
    await page.getByRole("button", { name: "New connection", exact: true }).click();
    await page.getByLabel("Connection name", { exact: true }).fill(`${engine} diagnostics`);
    await page
      .getByLabel("Database engine")
      .selectOption(engine === "PostgreSQL" ? "postgres" : "mysql");
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

    const table = engine === "PostgreSQL" ? "orders" : "sample";
    const sql = `SELECT '🧪';\n  SELECT 'café', '日本語', '🧪🚀', missing_column FROM ${table}`;
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    await editor.fill(sql);
    const failedStream = page.waitForResponse(async (response) => {
      if (!response.url().includes("/events/")) return false;
      const events = (await response.json()) as QueryEvent[];
      return events.some((event) => event.type === "error");
    });
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".query-error")).toContainText("missing_column");
    const events = (await (await failedStream).json()) as QueryEvent[];
    const event = events.find((event) => event.type === "error");
    expect(event?.type).toBe("error");
    if (event?.type !== "error") throw new Error("Missing error event");
    expect(event.statementIndex).toBe(1);
    const location = event.error.location;
    expect(location?.statementIndex).toBe(1);
    expect(location?.sqlState).toBe(engine === "PostgreSQL" ? "42703" : "42S22");
    expect(location?.span.line).toBe(2);
    expect(sql.slice(location!.span.startUtf16, location!.span.endUtf16)).toBe(
      engine === "PostgreSQL" ? "missing_column" : sql.slice(sql.indexOf("SELECT 'café'")),
    );

    await editor.fill(sql.replace("missing_column", "42"));
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
    await expect(page.locator(".query-error")).toHaveCount(0);
    expect(errors).toEqual([]);
  });
}
