import { test, expect, type Page } from "@playwright/test";

const PG_PORT = Number(process.env.JOINERY_POSTGRES_PORT ?? "55432");

async function connect(page: Page, name: string, engine: "postgres" | "mysql", port: number) {
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Database engine").selectOption(engine);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(String(port));
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("readOnly");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(name);
}

test("PostgreSQL schema diagram draws composite keys, filters and opens tables by keyboard", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await page.goto("/");
  await connect(page, "Diagram PG", "postgres", PG_PORT);

  await page.getByRole("button", { name: "Open schema diagram for public" }).click();
  const diagram = page.getByRole("group", { name: "Relationship diagram for public" });
  await expect(diagram).toBeVisible();
  await expect(page.getByRole("tab", { name: "public diagram" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(diagram.getByRole("button", { name: "line_notes, 4 columns" })).toBeVisible();
  await expect(diagram.locator('[data-diagram-edge="line_notes_line_fk"] title')).toHaveText(
    "line_notes_line_fk: line_notes(note_order, note_line) → public.order_lines(order_id, line_no)",
  );
  await expect(diagram.locator('[data-diagram-edge="orders_customer_id_fkey"]')).toHaveCount(1);

  await page.getByLabel("Filter diagram tables").fill("order_lines");
  await expect(diagram.getByRole("button", { name: /^customers,/ })).toHaveCount(0);
  await expect(diagram.getByRole("button", { name: /^orders,/ })).toBeVisible();
  await page.getByLabel("Filter diagram tables").fill("");
  await expect(diagram.getByRole("button", { name: /^customers,/ })).toBeVisible();

  const customers = diagram.getByRole("button", { name: /^customers,/ });
  await customers.focus();
  await page.keyboard.press("ArrowRight");
  await expect(diagram.getByRole("button", { name: /^orders,/ })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("tab", { name: /orders/ }).first()).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(errors).toEqual([]);
});

test("MySQL and MariaDB schema diagrams list composite keys within the database", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await page.goto("/");
  for (const [name, port] of [
    ["Diagram MySQL", 53306],
    ["Diagram MariaDB", 53307],
  ] as const) {
    await connect(page, name, "mysql", port);
    await page.getByRole("button", { name: "Open schema diagram for joinery" }).click();
    const diagram = page.getByRole("group", { name: "Relationship diagram for joinery" });
    await expect(diagram.locator('[data-diagram-edge="line_notes_line_fk"] title')).toHaveText(
      "line_notes_line_fk: line_notes(note_order, note_line) → joinery.order_lines(order_id, line_no)",
    );
    await expect(diagram.getByRole("button", { name: /^child_rows,/ })).toBeVisible();
    await page.getByRole("button", { name: "Close joinery diagram" }).click();
  }
  expect(errors).toEqual([]);
});
