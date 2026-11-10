import { test, expect } from "./fixtures";
import type { Bootstrap, ConnectionProfile } from "../../src/ipc/generated";

const bridge = "http://127.0.0.1:1422/rpc/";
const headers = { Authorization: "Bearer joinery-disposable-browser-tests-only" };

// Real profile persistence through the offline harness. Its authentication
// firewall rejects both Snowflake routes before reserving a connection attempt.
test("new Snowflake SSO profiles persist when offline authentication is refused", async ({
  page,
  request,
}) => {
  const name = `Offline SDK ${crypto.randomUUID().slice(0, 8)}`;
  const calls: string[] = [];
  page.on("request", (request) => {
    if (request.url().startsWith(bridge)) calls.push(request.url().slice(bridge.length));
  });
  await page.goto("/");
  await page.getByRole("button", { name: "New connection", exact: true }).click();
  await page.getByLabel("Database engine").selectOption("snowflake");
  await expect(page.getByLabel("Snowflake authentication")).toHaveValue("externalBrowser");
  await expect(page.getByText(/metadata browsing is temporarily unavailable/)).toBeVisible();
  await page.getByLabel("Connection name", { exact: true }).fill(name);
  await page
    .getByLabel("Snowflake account hostname")
    .fill("joinery-offline.invalid.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("DB");
  await page.getByLabel("Username", { exact: true }).fill("login@example.test");
  await page.getByLabel("Role", { exact: true }).fill("READ_ONLY");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH");
  await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("does not authenticate Snowflake profiles");
  await expect(page.getByRole("button", { name: "Save and connect", exact: true })).toBeEnabled();
  const response = await request.post(`${bridge}bootstrap`, { headers, data: {} });
  const { result } = (await response.json()) as { result: Bootstrap };
  const saved = result.profiles.find((profile) => profile.name === name)!;
  expect(saved.snowflake?.authentication).toBe("externalBrowser");
  expect(calls.filter((command) => command === "connect_prepared")).toHaveLength(0);
  expect(calls).not.toContain("catalog");
  expect(calls).not.toContain("execute");
  await page.reload();
  await page.getByRole("button", { name, exact: true }).hover();
  await page.getByRole("button", { name: `Edit ${name}`, exact: true }).click();
  await expect(page.getByLabel("Snowflake authentication")).toHaveValue("externalBrowser");
  await expect(page.getByLabel("Role", { exact: true })).toHaveValue("READ_ONLY");
  await expect(page.getByLabel("Warehouse", { exact: true })).toHaveValue("WH");
});

test("legacy Snowflake profiles retain OAuth until explicitly changed", async ({
  page,
  request,
}) => {
  const profile = {
    id: crypto.randomUUID(),
    name: `Legacy OAuth ${crypto.randomUUID().slice(0, 8)}`,
    groupId: "local",
    engine: "snowflake",
    host: "joinery-offline.invalid.snowflakecomputing.com",
    port: 443,
    database: "DB",
    username: "login@example.test",
    environment: "local",
    safeMode: "readOnly",
    tlsMode: "verifyFull",
    caCertPath: null,
    ssh: null,
    rememberPassword: false,
    storeQueryText: false,
    storeDrafts: false,
    snowflake: { role: "READ_ONLY", warehouse: "WH", schema: "PUBLIC" },
  };
  const response = await request.post(`${bridge}save_profile`, {
    headers,
    data: { profile, passphrase: null },
  });
  const payload = await response.json();
  expect(payload.error).toBeUndefined();
  expect((payload.result as ConnectionProfile).snowflake?.authentication).toBe(
    "localApplicationOauth",
  );
  await page.goto("/");
  await page.getByRole("button", { name: profile.name, exact: true }).hover();
  await page.getByRole("button", { name: `Edit ${profile.name}`, exact: true }).click();
  await expect(page.getByLabel("Snowflake authentication")).toHaveValue("localApplicationOauth");
  await page.getByLabel("Snowflake authentication").selectOption("externalBrowser");
  await expect(page.getByLabel("Role", { exact: true })).toHaveValue("READ_ONLY");
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("does not authenticate Snowflake profiles");
  await page.reload();
  await page.getByRole("button", { name: profile.name, exact: true }).hover();
  await page.getByRole("button", { name: `Edit ${profile.name}`, exact: true }).click();
  await expect(page.getByLabel("Snowflake authentication")).toHaveValue("externalBrowser");
});
