import { test, expect } from "@playwright/test";

test("opted-in drafts and preferences recover without reconnecting or executing", async ({
  page,
  request,
}) => {
  const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
  const saved = async () =>
    (
      await (
        await request.post("http://127.0.0.1:1422/rpc/bootstrap", { headers, data: {} })
      ).json()
    ).result.workspace;
  const name = `Draft fixture ${Date.now()}`;
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill("55432");
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  const optIn = page.getByLabel(/Restore open query drafts after restart/);
  await expect(optIn).not.toBeChecked();
  await optIn.check();
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab", exact: true }).click();
  const sql = "SELECT 'restored_draft_fixture';";
  await page.getByRole("textbox", { name: "SQL editor" }).fill(sql);
  await page.getByRole("button", { name: /^Preferences/ }).click();
  await page.getByRole("button", { name: "Paper light", exact: true }).click();
  await page.getByLabel("Display density", { exact: true }).selectOption("compact");
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await expect
    .poll(async () => {
      const workspace = await saved();
      return (
        workspace?.theme === "light" &&
        workspace?.density === "compact" &&
        workspace?.drafts.some((draft: { sql: string }) => draft.sql === sql)
      );
    })
    .toBe(true);
  await page.getByRole("button", { name, exact: true }).hover();
  await page.getByRole("button", { name: `Disconnect ${name}`, exact: true }).click();
  const forbidden: string[] = [];
  const record = (req: { url(): string }) => {
    if (/\/rpc\/(connect|execute_plan|prepare_execution)$/.test(req.url()))
      forbidden.push(req.url());
  };
  page.on("request", record);
  await page.reload();
  await expect(page.getByRole("textbox", { name: "SQL editor" })).toHaveText(sql);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.locator("html")).toHaveAttribute("data-density", "compact");
  await expect(page.locator(".status-bar")).toContainText("disconnected");
  await expect(page.locator(".result-footer")).toContainText("0 rows");
  expect(forbidden).toEqual([]);
  page.off("request", record);
  await page.getByRole("button", { name, exact: true }).hover();
  await page.getByRole("button", { name: `Edit ${name}`, exact: true }).click();
  await page.getByLabel(/Restore open query drafts after restart/).uncheck();
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect.poll(async () => (await saved())?.drafts.length).toBe(0);
  await page.reload();
  await expect(page.getByRole("tablist", { name: "Workspaces" }).getByRole("tab")).toHaveCount(0);
});
