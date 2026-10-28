import { expect, test, type Locator } from "@playwright/test";
import { mkdir, writeFile } from "node:fs/promises";

async function caret(editor: Locator, offset: number) {
  await editor.press("Escape");
  await editor.press("Home");
  for (let i = 0; i < offset; i++) await editor.press("ArrowRight");
}

for (const [engine, port] of [
  ["PostgreSQL", 55432],
  ["MySQL", 53306],
  ["MariaDB", 53307],
] as const) {
  test(`${engine} accepts escaped identifier completion and executes the resulting SELECT`, async ({
    page,
  }, testInfo) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const postgres = engine === "PostgreSQL";
    await page.goto("/");
    await page.getByRole("button", { name: "Connect a database" }).click();
    await page.getByLabel("Database engine").selectOption(postgres ? "postgres" : "mysql");
    await page.getByLabel("Connection name", { exact: true }).fill(`${engine} completion fixture`);
    await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
    await page.getByLabel("Port", { exact: true }).fill(String(port));
    await page.getByLabel("Database", { exact: true }).fill("joinery");
    await page.getByLabel("Username", { exact: true }).fill("joinery");
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    // These existing loopback fixtures deliberately expose a plaintext endpoint.
    // Production transport defaults and certificate verification are unchanged.
    await page.getByLabel("Require TLS and verify the server certificate").uncheck();
    await page.getByLabel("Safe Mode", { exact: true }).selectOption("readOnly");
    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: "New query tab" }).click();
    const editor = page.getByRole("textbox", { name: "SQL editor" });
    const alias = postgres ? '"a.b"' : "`a``b`";
    const table = postgres ? 'public."completion.names"' : "`joinery`.`completion names`";
    const prefix = postgres ? '"No"' : "`No`";
    const column = postgres ? '"No""tes"' : "`No``tes`";
    const source = `SELECT ${alias}.${prefix} FROM ${table} AS ${alias}`;
    await editor.fill(source);
    await caret(editor, source.indexOf(prefix) + 3);
    await editor.press("Control+Space");
    const option = page
      .locator(".cm-tooltip-autocomplete")
      .getByText(`${alias}.${column}`, { exact: true });
    await expect(option).toBeVisible();
    await option.click();
    await expect(editor).toHaveText(`SELECT ${alias}.${column} FROM ${table} AS ${alias}`);
    await page.getByRole("button", { name: "Run all statements", exact: true }).click();
    await expect(page.locator(".result-footer")).toContainText("1 rows");
    await expect(page.locator(".query-error")).toHaveCount(0);
    if (!(await page.locator(".inspector").isVisible()))
      await page.getByRole("button", { name: "Toggle inspector", exact: true }).click();
    const box = (await page.locator(".grid-host").boundingBox())!;
    await expect(async () => {
      await page.mouse.click(box.x + 90, box.y + 55);
      await expect(page.locator(".inspector-value")).toContainText("completion fixture", {
        timeout: 500,
      });
    }).toPass();

    // Reuse the same alias in a different statement. Only the active table's
    // columns may appear, even though both metadata records are available.
    const normalTable = postgres ? "public.orders" : "joinery.sample";
    const normalColumn = postgres ? "notes" : "note";
    const two = `SELECT x.* FROM ${table} x; SELECT x.no FROM ${normalTable} x`;
    await editor.fill(two);
    await caret(editor, two.lastIndexOf("x.no") + 4);
    await editor.press("Control+Space");
    await expect(page.locator(".cm-tooltip-autocomplete")).toContainText(`x.${normalColumn}`);
    await expect(page.locator(".cm-tooltip-autocomplete")).not.toContainText(column);

    // Warm, explicit completion latency includes CodeMirror scheduling and
    // worker round trips. This small fixture is not a native WebView benchmark.
    const samples: number[] = [];
    for (let i = 0; i < 20; i++) {
      await editor.press("Escape");
      await editor.evaluate((element) => {
        const root = element.ownerDocument;
        (window as unknown as { completionMeasurement: Promise<number> }).completionMeasurement =
          new Promise((resolve) => {
            let start = 0;
            const begin = () => {
              start = performance.now();
            };
            element.addEventListener("keydown", begin, { once: true, capture: true });
            const observer = new MutationObserver(() => {
              const tooltip = root.querySelector(".cm-tooltip-autocomplete");
              if (start && tooltip && tooltip.getBoundingClientRect().height) {
                observer.disconnect();
                resolve(performance.now() - start);
              }
            });
            observer.observe(root.body, { childList: true, subtree: true });
          });
      });
      await editor.press("Control+Space");
      samples.push(
        await page.evaluate(
          () =>
            (window as unknown as { completionMeasurement: Promise<number> }).completionMeasurement,
        ),
      );
    }
    samples.sort((a, b) => a - b);
    const report = {
      engine,
      browser: "headless Chromium",
      samples: samples.length,
      medianMs: samples[10],
      p95Ms: samples[18],
      targetMs: 50,
      scope: "warm explicit completion; small fixture catalog",
    };
    await mkdir(".cache/benchmarks", { recursive: true });
    await writeFile(
      `.cache/benchmarks/completion-${engine.toLowerCase()}.json`,
      JSON.stringify(report, null, 2) + "\n",
    );
    await testInfo.attach("completion-visible-latency", {
      body: JSON.stringify(report, null, 2),
      contentType: "application/json",
    });
    expect(errors).toEqual([]);
  });
}

test("measures bounded prefix lookup with a 100,000-name catalog", async ({ page }, testInfo) => {
  await page.goto("/");
  const measurement = await page.evaluate(async () => {
    // Vite serves the exact application module in this isolated test build.
    const path = "/src/data/completion-index.ts";
    const { CompletionIndex } = await import(/* @vite-ignore */ path);
    const index = new CompletionIndex();
    const items = Array.from({ length: 100000 }, (_, i) => ({
      label: `Order_${String(i).padStart(6, "0")}`,
      path: [`Order_${String(i).padStart(6, "0")}`],
      type: "type",
    }));
    const buildStart = performance.now();
    index.replace(items);
    const buildMs = performance.now() - buildStart;
    const timings: number[] = [];
    let capped = true;
    for (let i = 0; i < 1100; i++) {
      const start = performance.now();
      const result = index.search(
        [{ value: `ORDER_${String(i % 100).padStart(3, "0")}`, quoted: false }],
        "postgres",
      );
      if (i >= 100) timings.push(performance.now() - start);
      capped &&= result.length <= 100;
    }
    timings.sort((a, b) => a - b);
    return {
      names: items.length,
      samples: timings.length,
      buildMs,
      p95Ms: timings[949],
      targetMs: 5,
      capped,
      browser: navigator.userAgent,
      scope: "warm CompletionIndex lookup; excludes IPC and rendering",
    };
  });
  expect(measurement.capped).toBe(true);
  await mkdir(".cache/benchmarks", { recursive: true });
  await writeFile(
    ".cache/benchmarks/completion-index.json",
    JSON.stringify(measurement, null, 2) + "\n",
  );
  await testInfo.attach("completion-index-benchmark", {
    body: JSON.stringify(measurement, null, 2),
    contentType: "application/json",
  });
});
