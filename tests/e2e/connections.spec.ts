import { test, expect } from "@playwright/test";

const PG_PORT = Number(process.env.JOINERY_POSTGRES_PORT ?? "55432");

for (const method of ["mouse", "keyboard"] as const)
  test(`connection deletion by ${method} persists after reload`, async ({ page, request }) => {
    const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
    const profile = {
      id: crypto.randomUUID(),
      name: `Delete fixture ${Date.now()}`,
      groupId: "local",
      engine: "postgres",
      host: "127.0.0.1",
      port: PG_PORT,
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

test("long connection lists scroll inside the sidebar without shifting the workbench", async ({
  page,
  request,
}) => {
  const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };
  const ids: string[] = [];
  try {
    for (let i = 0; i < 28; i++) {
      const id = crypto.randomUUID();
      ids.push(id);
      const saved = await request.post("http://127.0.0.1:1422/rpc/save_profile", {
        headers,
        data: {
          profile: {
            id,
            name: `Overflow fixture ${i + 1}`,
            groupId: "local",
            engine: "postgres",
            host: "127.0.0.1",
            port: PG_PORT,
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
          },
          passphrase: null,
        },
      });
      expect((await saved.json()).error).toBeUndefined();
    }
    await page.goto("/");
    await page.getByRole("button", { name: /^Preferences/ }).click();
    await expect(page.getByRole("dialog", { name: "Preferences" })).toBeVisible();
    await page.keyboard.press("Escape");
    const metrics = await page.evaluate(() => {
      const root = document.getElementById("root")!;
      const shell = document.querySelector(".app-shell")!.getBoundingClientRect();
      const header = document.querySelector(".workspace-header")!.getBoundingClientRect();
      const groups = document.querySelector(".connection-groups") as HTMLElement;
      return {
        rootScrollTop: root.scrollTop,
        shellTop: shell.top,
        shellHeight: shell.height,
        viewportHeight: window.innerHeight,
        headerTop: header.top,
        groupsScrollable: groups.scrollHeight > groups.clientHeight,
      };
    });
    expect(metrics.groupsScrollable).toBe(true);
    expect(metrics.rootScrollTop).toBe(0);
    expect(metrics.shellTop).toBe(0);
    expect(metrics.shellHeight).toBe(metrics.viewportHeight);
    expect(metrics.headerTop).toBe(0);
  } finally {
    for (const id of ids)
      await request.post("http://127.0.0.1:1422/rpc/delete_profile", {
        headers,
        data: { id },
      });
  }
});
