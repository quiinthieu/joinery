import { expect, test, type Page } from "./fixtures";
import type {
  ConnectionProfile,
  PreparedPlan,
  SessionRuntimeState,
  SnowflakeDmlResult,
} from "../../src/ipc/generated";

// UI scheduling/review evidence only. All SDK/database behavior is separately
// tested at native/helper boundaries; these RPC interceptions never sign in.
const rpc = "http://127.0.0.1:1422/rpc/";
const sessionId = "synthetic-reviewed-dml";
const sql = "UPDATE DB.PUBLIC.T SET NOTE = :note WHERE ID = :id";
function runtime(recovery = false): SessionRuntimeState {
  return {
    sessionId,
    transactionMode: "autoCommit",
    transactionState: recovery ? "unknown" : "inTransaction",
    contextKind: "schema",
    activeContext: "PUBLIC",
    database: "DB",
    role: "ANALYST",
    searchPath: ["DB.PUBLIC"],
    transactionObservation: recovery
      ? { status: "unavailable" }
      : { status: "observed", transactionId: "123", abortOnError: false, usability: "unknown" },
    transactionControlAuthority: recovery
      ? { status: "rollbackOnly", revision: "recovery" }
      : { status: "observed", revision: "authority", allowedActions: ["commit", "rollback"] },
  };
}
async function setup(page: Page, execute: () => Promise<SnowflakeDmlResult>) {
  const calls: Array<{ command: string; body: Record<string, any> }> = [];
  let profile!: ConnectionProfile;
  await page.clock.install();
  await page.route(`${rpc}*`, async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const body = route.request().postDataJSON();
    calls.push({ command, body });
    let result: unknown;
    switch (command) {
      case "save_profile":
        profile = body.profile;
        result = profile;
        expect(profile.snowflake?.reviewedDml).toBe(true);
        break;
      case "prepare_connection":
        result = "synthetic-attempt";
        break;
      case "connect_prepared":
        result = {
          id: sessionId,
          connectionId: profile.id,
          engine: "snowflake",
          serverVersion: "synthetic",
          latencyMs: 1,
          encrypted: true,
          capabilities: {
            sql: true,
            transactions: false,
            explicitTransactionControls: true,
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
        result = { state: runtime(), available: ["PUBLIC"], revision: "schema" };
        break;
      case "completion_catalog":
        result = {
          revision: "completion",
          identity: {
            database: "DB",
            role: "ANALYST",
            searchPath: ["DB.PUBLIC"],
            caseSensitiveTableNames: true,
          },
          tables: [],
          unqualifiedTables: [],
        };
        break;
      case "health":
        result = 1;
        break;
      case "disconnect":
        expect(body).toEqual({ sessionId });
        result = null;
        break;
      case "prepare_execution": {
        expect(body).toEqual({
          sessionId,
          sql,
          params: [
            { key: "note", kind: "text", value: "literal'; -- 雪" },
            { key: "id", kind: "integer", value: "9007199254740993" },
          ],
        });
        result = {
          id: "reviewed-dml",
          sessionId,
          reviewedSnowflakeDml: true,
          statements: [{ sql, kind: "write", missingWhere: false }],
          boundParameters: body.params,
          requiresApproval: true,
          requiresPassphrase: false,
          confirmationText: null,
          warnings: [
            "Concurrent schema changes or temporary objects can change name resolution. No automatic retry is performed. A statement acknowledgement does not prove committed data effects.",
          ],
          expiresAt: Date.now() + 60_000,
        } satisfies PreparedPlan;
        break;
      }
      case "authorize_plan":
        expect(body.planId).toBe("reviewed-dml");
        result = "exact-approval";
        break;
      case "execute_snowflake_dml":
        expect(body).toEqual({ planId: "reviewed-dml", approval: "exact-approval" });
        result = await execute();
        break;
      case "execute":
        throw new Error("DML must never enter generic execution");
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
  await page.getByLabel("Connection name", { exact: true }).fill("Synthetic reviewed writes");
  await page
    .getByLabel("Snowflake account hostname")
    .fill("fixture.invalid.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("DB");
  await page.getByLabel("Username", { exact: true }).fill("ANALYST");
  await page.getByLabel("Role", { exact: true }).fill("ANALYST");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH");
  await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
  await page.getByLabel("Safe Mode").selectOption("confirm");
  await page
    .getByRole("checkbox", {
      name: "Allow separately reviewed single INSERT, UPDATE, and DELETE statements",
    })
    .check();
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  await expect(page.getByLabel("Active schema", { exact: true })).toHaveValue("PUBLIC");
  return calls;
}
async function reviewAndRun(page: Page) {
  await page.getByRole("textbox", { name: "SQL editor" }).fill(sql);
  await page.getByRole("button", { name: "Run all statements", exact: true }).click();
  const parameters = page.getByRole("dialog", { name: "Query parameters" });
  await parameters.getByLabel("Value for parameter note").fill("literal'; -- 雪");
  await parameters.getByLabel("Type for parameter id").selectOption("integer");
  await parameters.getByLabel("Value for parameter id").fill("9007199254740993");
  await parameters.getByRole("button", { name: "Run query", exact: true }).click();
  const review = page.getByRole("dialog", { name: "Review this operation" });
  await expect(review.locator(".sql-preview")).toHaveText(sql);
  await expect(review.getByLabel("Bound query parameters")).toContainText("9007199254740993");
  await expect(review).toContainText("No automatic retry");
  await review.getByRole("button", { name: "Authorize and run" }).focus();
  await page.keyboard.press("Enter");
}

for (const outcome of ["normal", "recovery", "unknown", "disconnect"] as const) {
  test(`reviewed SDK DML ${outcome} uses parameters and exact review once, keeps metadata quiet and displays truthful evidence`, async ({
    page,
  }) => {
    let release!: () => void;
    let entered!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    const started = new Promise<void>((resolve) => {
      entered = resolve;
    });
    const calls = await setup(page, async () => {
      entered();
      await held;
      return {
        kind: "update",
        statementOutcome:
          outcome === "unknown" || outcome === "disconnect" ? "unknown" : "succeeded",
        summary:
          outcome === "unknown" || outcome === "disconnect"
            ? { status: "unavailable", affectedRows: null, multiJoinedRows: null }
            : { status: "validated", affectedRows: "9007199254740993", multiJoinedRows: "7" },
        state: runtime(outcome === "recovery"),
        retired: outcome === "unknown" || outcome === "disconnect",
        error: null,
      };
    });
    await reviewAndRun(page);
    await started;
    const quiet = () =>
      calls.filter(({ command }) =>
        ["session_contexts", "health", "completion_catalog"].includes(command),
      ).length;
    const before = quiet();
    await expect(
      page.getByRole("button", { name: "Run all statements", includeHidden: true }),
    ).toBeDisabled();
    await expect(page.getByRole("button", { name: "Disconnect to stop write" })).toBeVisible();
    await page.clock.fastForward("00:31");
    expect(quiet()).toBe(before);
    if (outcome === "disconnect") {
      await page.getByRole("button", { name: "Disconnect to stop write" }).click();
      await expect
        .poll(() => calls.filter(({ command }) => command === "disconnect").length)
        .toBe(1);
    }
    release();
    const notice = page.getByRole(
      outcome === "unknown" || outcome === "disconnect" ? "alert" : "status",
      { name: "Snowflake UPDATE outcome" },
    );
    await expect(notice).toBeVisible();
    if (outcome === "unknown" || outcome === "disconnect") {
      await expect(notice).toContainText("outcome is unknown");
      await expect(notice).toContainText("unavailable");
      await expect(notice).toContainText("retired");
    } else {
      await expect(notice).toContainText("Statement succeeded");
      await expect(notice).toContainText("9007199254740993");
      await expect(notice).toContainText("Multi-joined rows updated: 7");
      await expect(notice).toContainText("does not prove committed effects");
    }
    await page.clock.fastForward("00:31");
    expect(quiet()).toBe(before);
    expect(calls.filter(({ command }) => command === "execute_snowflake_dml")).toHaveLength(1);
    expect(calls.filter(({ command }) => command === "execute")).toHaveLength(0);
    if (outcome === "normal") {
      await page.getByRole("button", { name: "Refresh completion context" }).click();
      await expect
        .poll(() =>
          calls.some(
            ({ command, body }) => command === "session_contexts" && body.resumeMetadata === true,
          ),
        )
        .toBe(true);
    } else if (outcome === "recovery") {
      await expect(page.getByRole("button", { name: "Rollback transaction" })).toBeEnabled();
      await expect(page.getByRole("button", { name: "Commit transaction" })).toBeDisabled();
      await expect(page.getByRole("button", { name: "Refresh completion context" })).toBeDisabled();
    }
  });
}
