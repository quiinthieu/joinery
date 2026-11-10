import { expect, test } from "./fixtures";
import type { ConnectionProfile, SessionContextOptions } from "../../src/ipc/generated";

const rpc = "http://127.0.0.1:1422/rpc/";

// Synthetic RPCs exercise the real UI only; this does not perform Snowflake SSO or SQL.
test("synthetic SDK session observations stay explicit and block unsupported transaction actions", async ({
  page,
}) => {
  const profileName = "Synthetic SDK runtime state";
  const sessionId = "synthetic-sdk-observed-session";
  const calls: string[] = [];
  let profile!: ConnectionProfile;
  let contextCalls = 0;
  let failObservations = false;

  const observed: SessionContextOptions = {
    state: {
      sessionId,
      transactionMode: "autoCommit",
      transactionState: "inTransaction",
      contextKind: "schema",
      activeContext: "S",
      database: "DB",
      role: null,
      searchPath: ["DB.S"],
      transactionObservation: {
        status: "observed",
        transactionId: "opaque-transaction-id",
        abortOnError: false,
        usability: "unknown",
      },
    },
    available: ["OTHER"],
    revision: "opaque-session-context-revision",
  };

  await page.route(`${rpc}*`, async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    calls.push(command);
    let result: unknown;
    switch (command) {
      case "save_profile":
        profile = route.request().postDataJSON().profile;
        result = profile;
        break;
      case "prepare_connection":
        result = "synthetic-sdk-attempt";
        break;
      case "connect_prepared":
        result = {
          id: sessionId,
          connectionId: profile.id,
          engine: "snowflake",
          serverVersion: "synthetic fixture",
          latencyMs: 1,
          encrypted: true,
          capabilities: {
            sql: true,
            transactions: false,
            cancellation: true,
            tableBrowsing: false,
            relationBrowsing: true,
            tableEditing: false,
            tableFiltering: false,
            processes: false,
          },
        };
        break;
      case "session_contexts":
        contextCalls += 1;
        if (failObservations) {
          await route.fulfill({
            json: {
              error: {
                code: "unsupported",
                message: "Synthetic SDK runtime observation unavailable",
              },
            },
          });
          return;
        }
        result = observed;
        break;
      case "completion_catalog":
        await route.fulfill({
          json: {
            error: {
              code: "unsupported",
              message: "Synthetic SDK completion catalog unsupported",
            },
          },
        });
        return;
      case "health":
        result = 1;
        break;
      default:
        await route.continue();
        return;
    }
    await route.fulfill({ json: { result } });
  });

  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Database engine").selectOption("snowflake");
  await page.getByLabel("Snowflake authentication").selectOption("externalBrowser");
  await page.getByLabel("Connection name", { exact: true }).fill(profileName);
  await page
    .getByLabel("Snowflake account hostname")
    .fill("fixture.invalid.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("DB");
  await page.getByLabel("Username", { exact: true }).fill("ANALYST");
  await page.getByLabel("Role", { exact: true }).fill("ANALYST");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH");
  await page.getByLabel("Schema", { exact: true }).fill("S");
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(profileName);
  expect(profile.snowflake?.authentication).toBe("externalBrowser");

  await page.getByRole("button", { name: "New query tab" }).click();
  await expect.poll(() => contextCalls).toBeGreaterThan(0);
  await expect(page.getByLabel("Transaction mode", { exact: true })).toHaveValue("autoCommit");
  await expect(page.getByLabel("Transaction mode", { exact: true })).toBeDisabled();
  await expect(
    page.getByText("Open transaction · usability unknown", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Commit transaction", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Rollback transaction", exact: true }),
  ).toBeDisabled();

  const activeSchema = page.getByLabel("Active schema", { exact: true });
  await expect(activeSchema).toBeVisible();
  await expect(activeSchema).toBeDisabled();
  const unavailableSchema = activeSchema.locator('option[value="S"]');
  await expect(unavailableSchema).toHaveText("S (unavailable)");
  await expect(unavailableSchema).toBeDisabled();
  await expect(activeSchema.locator('option[value="OTHER"]')).toHaveCount(1);

  const callsBeforeFailure = contextCalls;
  failObservations = true;
  await page.getByRole("button", { name: "New query tab" }).click();
  await expect(page.getByRole("option", { name: "Mode unknown" })).toBeAttached();
  await expect(page.getByLabel("Transaction mode", { exact: true })).toHaveValue("unknown");
  await expect(page.getByLabel("Transaction mode", { exact: true })).toBeDisabled();
  await expect(page.getByText("Transaction state unknown", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Commit transaction", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Rollback transaction", exact: true }),
  ).toBeDisabled();
  expect(contextCalls).toBeGreaterThan(callsBeforeFailure);
  expect(contextCalls - callsBeforeFailure).toBeLessThanOrEqual(3);

  await page.getByRole("button", { name: profileName, exact: true }).hover();
  await page.getByRole("button", { name: `Disconnect ${profileName}`, exact: true }).click();
  const confirmation = page.getByRole("dialog", { name: `Disconnect ${profileName}?` });
  await expect(confirmation).toContainText("unknown transaction");
  await confirmation.getByRole("button", { name: "Keep working", exact: true }).click();
  await expect(confirmation).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText(profileName);

  expect(
    calls.filter((command) =>
      [
        "set_transaction_mode",
        "commit_transaction",
        "rollback_transaction",
        "switch_session_context",
        "disconnect",
      ].includes(command),
    ),
  ).toEqual([]);
});
