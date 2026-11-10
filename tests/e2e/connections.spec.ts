import { test, expect, type APIRequestContext, type Page } from "@playwright/test";

const PG_PORT = Number(process.env.JOINERY_POSTGRES_PORT ?? "55432");
const BRIDGE = "http://127.0.0.1:1422";
const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };

async function rpc<T>(request: APIRequestContext, command: string, args = {}): Promise<T> {
  const response = await request.post(`${BRIDGE}/rpc/${command}`, { headers, data: args });
  expect(response.ok()).toBe(true);
  const payload = await response.json();
  expect(payload.error).toBeUndefined();
  return payload.result as T;
}

function deferred<T = void>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

async function startPostgresConnection(page: Page, name: string) {
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Database engine").selectOption("postgres");
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page.getByLabel("Port", { exact: true }).fill(String(PG_PORT));
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("readOnly");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
}

for (const method of ["mouse", "keyboard"] as const)
  test(`connection deletion by ${method} persists after reload`, async ({ page, request }) => {
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
      if (/\/rpc\/(save_profile|connect|prepare_connection|connect_prepared)$/.test(request.url()))
        forbidden.push(request.url());
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

test("cancel retires a prepared PostgreSQL attempt before sign-in dispatch", async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const prepared = deferred<{ attemptId: string }>();
  const releasePrepare = deferred();
  const prepareRouteFinished = deferred();
  const cancelSent = deferred<{ attemptId: string }>();
  const profileName = `Cancel before sign-in ${crypto.randomUUID().slice(0, 8)}`;
  let profileId: string | undefined;
  let prepareHeld = false;
  page.on("pageerror", (error) => errors.push(error.message));
  await rpc(request, "disconnect_fixtures");
  const sessionBaseline = (await rpc<{ sessionCount: number }>(request, "resource_usage"))
    .sessionCount;
  await page.route(`${BRIDGE}/rpc/*`, async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const args = route.request().postDataJSON() as Record<string, unknown>;
    calls.push({ command, args });
    if (command === "save_profile") {
      profileId = (args.profile as { id: string }).id;
    }
    if (command === "prepare_connection") {
      const response = await route.fetch();
      const body = await response.body();
      const payload = JSON.parse(body.toString()) as { result: string; error?: unknown };
      expect(payload.error).toBeUndefined();
      prepareHeld = true;
      prepared.resolve({ attemptId: payload.result });
      await releasePrepare.promise;
      await route.fulfill({ response, body });
      prepareRouteFinished.resolve(undefined);
      return;
    }
    if (command === "cancel_connection_attempt") {
      const response = await route.fetch();
      const payload = await response.json();
      expect(payload.error).toBeUndefined();
      cancelSent.resolve({ attemptId: args.attemptId as string });
      await route.fulfill({ response });
      return;
    }
    await route.continue();
  });

  try {
    await startPostgresConnection(page, profileName);
    const { attemptId } = await prepared.promise;
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    releasePrepare.resolve(undefined);

    await expect(cancelSent.promise).resolves.toEqual({ attemptId });
    await expect
      .poll(
        async () => (await rpc<{ sessionCount: number }>(request, "resource_usage")).sessionCount,
      )
      .toBe(sessionBaseline);
    expect(calls.map(({ command }) => command)).not.toContain("connect_prepared");
    await expect(page.locator(".status-bar")).not.toContainText(profileName);
    expect(errors).toEqual([]);
  } finally {
    releasePrepare.resolve(undefined);
    if (prepareHeld) await prepareRouteFinished.promise;
    await rpc(request, "disconnect_fixtures");
    if (profileId) await rpc(request, "delete_profile", { id: profileId }).catch(() => undefined);
  }
});

test("late PostgreSQL connect success after cancel disconnects only its returned session", async ({
  page,
  request,
}) => {
  test.setTimeout(90000);
  const errors: string[] = [];
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const published = deferred<{
    attemptId: string;
    session: { id: string; connectionId: string };
  }>();
  const releaseConnect = deferred();
  const connectRouteFinished = deferred();
  const cancelSent = deferred<{ attemptId: string }>();
  const disconnected = deferred<{ sessionId: string }>();
  const profileName = `Cancel after publish ${crypto.randomUUID().slice(0, 8)}`;
  let profileId: string | undefined;
  let connectHeld = false;
  page.on("pageerror", (error) => errors.push(error.message));
  await rpc(request, "disconnect_fixtures");
  const sessionBaseline = (await rpc<{ sessionCount: number }>(request, "resource_usage"))
    .sessionCount;
  await page.route(`${BRIDGE}/rpc/*`, async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const args = route.request().postDataJSON() as Record<string, unknown>;
    calls.push({ command, args });
    if (command === "save_profile") {
      profileId = (args.profile as { id: string }).id;
    }
    if (command === "connect_prepared") {
      const response = await route.fetch();
      const body = await response.body();
      const payload = JSON.parse(body.toString()) as {
        result: { id: string; connectionId: string };
        error?: unknown;
      };
      expect(payload.error).toBeUndefined();
      connectHeld = true;
      published.resolve({ attemptId: args.attemptId as string, session: payload.result });
      await releaseConnect.promise;
      await route.fulfill({ response, body });
      connectRouteFinished.resolve(undefined);
      return;
    }
    if (command === "cancel_connection_attempt") {
      const response = await route.fetch();
      const payload = await response.json();
      expect(payload.error).toBeUndefined();
      cancelSent.resolve({ attemptId: args.attemptId as string });
      await route.fulfill({ response });
      return;
    }
    if (command === "disconnect") disconnected.resolve({ sessionId: args.sessionId as string });
    await route.continue();
  });

  try {
    await startPostgresConnection(page, profileName);
    const { attemptId, session } = await published.promise;
    expect(profileId).toBeTruthy();
    expect(session.connectionId).toBe(profileId);
    await expect
      .poll(
        async () => (await rpc<{ sessionCount: number }>(request, "resource_usage")).sessionCount,
      )
      .toBe(sessionBaseline + 1);

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    const cancel = await cancelSent.promise;
    expect(cancel.attemptId).toBe(attemptId);
    expect(session.id).toBe(attemptId);
    await expect
      .poll(
        async () => (await rpc<{ sessionCount: number }>(request, "resource_usage")).sessionCount,
      )
      .toBe(sessionBaseline);
    releaseConnect.resolve(undefined);

    await expect(disconnected.promise).resolves.toEqual({ sessionId: session.id });
    await connectRouteFinished.promise;
    await expect
      .poll(
        async () => (await rpc<{ sessionCount: number }>(request, "resource_usage")).sessionCount,
      )
      .toBe(sessionBaseline);
    expect(calls.map(({ command }) => command)).not.toContain("catalog");
    expect(calls.map(({ command }) => command)).not.toContain("session_state");
    await expect(page.locator(".status-bar")).not.toContainText(profileName);
    expect(calls.find(({ command }) => command === "disconnect")?.args.sessionId).toBe(session.id);
    expect(errors).toEqual([]);
  } finally {
    releaseConnect.resolve(undefined);
    if (connectHeld) await connectRouteFinished.promise;
    await rpc(request, "disconnect_fixtures");
    if (profileId) await rpc(request, "delete_profile", { id: profileId }).catch(() => undefined);
  }
});

test("synthetic SSH preflight cancellation retires the real prepared reservation", async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const prepared = deferred<{ attemptId: string }>();
  const probeReceived = deferred<{ attemptId: string }>();
  const releaseProbe = deferred();
  const probeRouteFinished = deferred();
  const cancelSent = deferred<{ attemptId: string }>();
  const profileName = `Synthetic SSH cancel ${crypto.randomUUID().slice(0, 8)}`;
  const syntheticKey = {
    host: "synthetic-bastion.invalid",
    port: 22,
    fingerprint: "SHA256:synthetic-test-key",
    previousFingerprint: null,
    challengeId: "synthetic-challenge",
  };
  let profileId: string | undefined;
  let probeHeld = false;
  page.on("pageerror", (error) => errors.push(error.message));
  await rpc(request, "disconnect_fixtures");
  const sessionBaseline = (await rpc<{ sessionCount: number }>(request, "resource_usage"))
    .sessionCount;
  await page.route(`${BRIDGE}/rpc/*`, async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const args = route.request().postDataJSON() as Record<string, unknown>;
    calls.push({ command, args });
    if (command === "save_profile") {
      const profile = args.profile as { id: string; ssh: unknown };
      profileId = profile.id;
      const response = await route.fetch({
        postData: JSON.stringify({
          ...args,
          profile: { ...profile, ssh: null },
        }),
      });
      const payload = await response.json();
      expect(payload.error).toBeUndefined();
      await route.fulfill({ response, json: { ...payload, result: profile } });
      return;
    }
    if (command === "prepare_connection") {
      const response = await route.fetch();
      const payload = await response.json();
      expect(payload.error).toBeUndefined();
      prepared.resolve({ attemptId: payload.result as string });
      await route.fulfill({ response });
      return;
    }
    if (command === "probe_ssh") {
      probeHeld = true;
      probeReceived.resolve({ attemptId: args.attemptId as string });
      await releaseProbe.promise;
      await route.fulfill({ json: { result: syntheticKey } });
      probeRouteFinished.resolve(undefined);
      return;
    }
    if (command === "cancel_connection_attempt") {
      const response = await route.fetch();
      const payload = await response.json();
      expect(payload.error).toBeUndefined();
      await route.fulfill({ response });
      cancelSent.resolve({ attemptId: args.attemptId as string });
      return;
    }
    await route.continue();
  });

  try {
    await page.goto("/");
    await page.getByRole("button", { name: "New connection", exact: true }).click();
    await page.getByLabel("Database engine").selectOption("postgres");
    await page.getByLabel("Connection name", { exact: true }).fill(profileName);
    await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
    await page.getByLabel("Port", { exact: true }).fill(String(PG_PORT));
    await page.getByLabel("Database", { exact: true }).fill("joinery");
    await page.getByLabel("Username", { exact: true }).fill("joinery");
    await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
    await page.getByLabel("Safe Mode", { exact: true }).selectOption("readOnly");
    await page.getByLabel("Require TLS and verify the server certificate").uncheck();
    await page.getByLabel("Connect through an SSH bastion").check();
    await page.getByLabel("SSH host", { exact: true }).fill(syntheticKey.host);
    await page.getByLabel("SSH port", { exact: true }).fill(String(syntheticKey.port));
    await page.getByLabel("SSH username", { exact: true }).fill("synthetic-user");
    await page.getByRole("button", { name: "Save and connect", exact: true }).click();

    const { attemptId } = await prepared.promise;
    const probed = await probeReceived.promise;
    expect(probed.attemptId).toBe(attemptId);
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(cancelSent.promise).resolves.toEqual({ attemptId });
    // Independently prove the real backend retired the reservation. A bare
    // cancellation acknowledgement and unchanged session count cannot prove it.
    const retired = await request.post(`${BRIDGE}/rpc/connect_prepared`, {
      headers,
      data: { attemptId, secrets: { password: null, sshPassword: null, keyPassphrase: null } },
    });
    expect((await retired.json()).error?.code).toBe("cancelled");
    releaseProbe.resolve(undefined);
    await probeRouteFinished.promise;

    expect(calls.map(({ command }) => command)).not.toContain("connect_prepared");
    expect(calls.map(({ command }) => command)).not.toContain("trust_ssh_key");
    await expect
      .poll(
        async () => (await rpc<{ sessionCount: number }>(request, "resource_usage")).sessionCount,
      )
      .toBe(sessionBaseline);
    await expect(page.getByRole("dialog", { name: "Verify SSH host" })).toHaveCount(0);
    await expect(page.locator(".status-bar")).not.toContainText(profileName);
    expect(errors).toEqual([]);
  } finally {
    releaseProbe.resolve(undefined);
    if (probeHeld) await probeRouteFinished.promise;
    await rpc(request, "disconnect_fixtures");
    if (profileId) await rpc(request, "delete_profile", { id: profileId }).catch(() => undefined);
  }
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
