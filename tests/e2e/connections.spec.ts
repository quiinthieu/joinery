import { test, expect } from "@playwright/test";

for (const method of ["mouse", "keyboard"] as const)
  test(`connection deletion by ${method} persists after reload`, async ({ page, request }) => {
    const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
    const profile = {
      id: crypto.randomUUID(),
      name: `Delete fixture ${Date.now()}`,
      groupId: "local",
      engine: "postgres",
      host: "127.0.0.1",
      port: 55432,
      database: "joinery",
      username: "joinery",
      environment: "local",
      safeMode: "readOnly",
      tlsMode: "disable",
      caCertPath: null,
      ssh: null,
      rememberPassword: false,
      storeQueryText: false,
      storeDrafts: true,
      snowflake: null,
    };
    const saved = await request.post("http://127.0.0.1:1422/rpc/save_profile", {
      headers,
      data: { profile, passphrase: null },
    });
    expect((await saved.json()).error).toBeUndefined();
    await page.goto("/");
    const row = page.getByRole("button", { name: profile.name, exact: true });
    await row.hover();
    await page.getByRole("button", { name: `Edit ${profile.name}`, exact: true }).click();
    await page.getByRole("button", { name: "Delete connection", exact: true }).click();
    const confirmation = page.getByRole("dialog", { name: "Delete connection?", exact: true });
    await expect(confirmation).toBeVisible();
    await expect(confirmation.getByText(profile.name, { exact: true })).toBeVisible();
    await expect(confirmation.getByRole("button", { name: "Save and connect" })).toHaveCount(0);
    await expect(confirmation.locator('input[type="password"]')).toHaveCount(0);
    await confirmation.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(page.getByRole("dialog", { name: "Edit connection" })).toBeVisible();
    await page.getByRole("button", { name: "Delete connection", exact: true }).click();
    const forbidden: string[] = [];
    page.on("request", (request) => {
      if (/\/rpc\/(save_profile|connect)$/.test(request.url())) forbidden.push(request.url());
    });
    const remove = confirmation.getByRole("button", {
      name: "Remove saved connection",
      exact: true,
    });
    if (method === "keyboard") {
      await remove.focus();
      await remove.press("Enter");
    } else await remove.click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(row).toHaveCount(0);
    expect(forbidden).toEqual([]);
    await page.reload();
    await expect(page.getByRole("button", { name: "New connection", exact: true })).toBeVisible();
    await expect(row).toHaveCount(0);
    const bootstrap = await request.post("http://127.0.0.1:1422/rpc/bootstrap", {
      headers,
      data: {},
    });
    expect((await bootstrap.json()).result.profiles).not.toContainEqual(
      expect.objectContaining({ id: profile.id }),
    );
  });
