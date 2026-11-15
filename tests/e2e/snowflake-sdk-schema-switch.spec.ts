import { expect, test } from "./fixtures";
import type { ConnectionProfile, SessionContextOptions } from "../../src/ipc/generated";

const rpc = "http://127.0.0.1:1422/rpc/";

// Synthetic RPCs verify UI ownership and scheduling. Adapter tests separately
// exercise the production helper; neither fixture authenticates to Snowflake.
for (const outcome of ["success", "acknowledged-context-unavailable"] as const) {
  test(`SDK schema selection ${outcome} preserves request authority and excludes background work`, async ({
    page,
  }) => {
    await page.clock.install();
    const sessionId = `synthetic-schema-switch-${outcome}`;
    const target = 'Quoted."Schema';
    const calls: string[] = [];
    let profile!: ConnectionProfile;
    let schema = "S";
    let revision = 0;
    let lastOptions!: SessionContextOptions;
    let requestBody: unknown;
    let release!: () => void;
    let entered!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    const switching = new Promise<void>((resolve) => {
      entered = resolve;
    });
    const options = (): SessionContextOptions => ({
      state: {
        sessionId,
        transactionMode: "autoCommit",
        transactionState: "idle",
        contextKind: "schema",
        activeContext: schema,
        database: "DB",
        role: null,
        searchPath: [`DB.${schema}`],
        contextFingerprint: `fingerprint-${schema}`,
        transactionObservation: {
          status: "observed",
          transactionId: null,
          abortOnError: false,
          usability: "unknown",
        },
      },
      available: ["S", target],
      revision: `authority-${++revision}`,
    });
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
          result = "synthetic-schema-attempt";
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
        case "completion_catalog":
          result = {
            revision: `catalog-${schema}`,
            identity: {
              database: "DB",
              role: "",
              searchPath: [`DB.${schema}`],
              caseSensitiveTableNames: true,
              contextFingerprint: `fingerprint-${schema}`,
            },
            tables: [],
            unqualifiedTables: [],
          };
          break;
        case "session_contexts":
          lastOptions = options();
          result = lastOptions;
          break;
        case "switch_session_schema":
          requestBody = route.request().postDataJSON();
          entered();
          await held;
          if (outcome !== "success") {
            await route.fulfill({
              json: {
                error: {
                  code: "connectionClosed",
                  message:
                    "Snowflake acknowledged the schema change, but its final session context could not be verified. The session was closed; reconnect before continuing.",
                },
              },
            });
            return;
          }
          schema = target;
          lastOptions = options();
          result = lastOptions;
          break;
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
    await page.getByLabel("Connection name", { exact: true }).fill(sessionId);
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
    await page.getByRole("button", { name: "New query tab" }).click();
    const selector = page.getByLabel("Active schema", { exact: true });
    await expect(selector).toBeEnabled();
    await expect(selector).toHaveValue("S");
    const authority = lastOptions;
    await selector.selectOption(target);
    await switching;
    await expect(page.getByText("Changing schema…", { exact: true })).toBeVisible();
    expect(requestBody).toEqual({
      sessionId,
      request: {
        revision: authority.revision,
        database: "DB",
        schema: target,
        contextFingerprint: authority.state.contextFingerprint,
      },
    });
    const sessionWork = () =>
      calls.filter((command) =>
        [
          "health",
          "completion_catalog",
          "completion_schema",
          "session_contexts",
          "relation_catalog",
          "inspect_relation",
          "preview_relation",
        ].includes(command),
      );
    const admittedCalls = sessionWork();
    await page.clock.fastForward("00:31");
    expect(sessionWork()).toEqual(admittedCalls);
    expect(calls).not.toContain("switch_session_context");
    release();
    if (outcome === "success") {
      await expect(selector).toHaveValue(target);
      await expect(selector).toBeEnabled();
      await expect(page.getByText("Transaction state unknown", { exact: true })).toHaveCount(0);
      expect(calls.filter((command) => command === "switch_session_schema")).toHaveLength(1);
      expect(calls.filter((command) => command === "completion_catalog").length).toBeLessThan(6);
    } else {
      await expect(page.getByText(/Snowflake acknowledged the schema change/)).toBeVisible();
      await expect(selector).toHaveCount(0);
      expect(calls.filter((command) => command === "switch_session_schema")).toHaveLength(1);
    }
  });
}
