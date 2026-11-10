import { expect, test, type Page } from "./fixtures";
import type {
  ConnectionProfile,
  ExplicitTransactionAction,
  PreparedPlan,
  SessionRuntimeState,
  TransactionControlResult,
} from "../../src/ipc/generated";

const rpc = "http://127.0.0.1:1422/rpc/";
const passThrough = Symbol("pass-through synthetic RPC");

function runtime(
  sessionId: string,
  revision: string,
  allowedActions: ExplicitTransactionAction[],
  overrides: Partial<SessionRuntimeState> = {},
): SessionRuntimeState {
  return {
    sessionId,
    transactionMode: "autoCommit",
    transactionState: "idle",
    contextKind: "schema",
    activeContext: "PUBLIC",
    database: "DB",
    role: null,
    searchPath: ["DB.PUBLIC"],
    contextFingerprint: `fingerprint-${revision}`,
    transactionObservation: {
      status: "observed",
      transactionId: null,
      abortOnError: false,
      usability: "unknown",
    },
    transactionControlAuthority: { status: "observed", revision, allowedActions },
    ...overrides,
  };
}

function boundaryResult(
  action: ExplicitTransactionAction,
  state: SessionRuntimeState,
  overrides: Partial<TransactionControlResult> = {},
): TransactionControlResult {
  return {
    action,
    boundaryOutcome: "acknowledged",
    state,
    retired: false,
    error: null,
    ...overrides,
  };
}

function controlPlan(action: ExplicitTransactionAction, sessionId: string): PreparedPlan {
  return {
    id: `synthetic-${action}-plan`,
    sessionId,
    statements: [
      {
        sql: action === "begin" ? "BEGIN TRANSACTION" : action === "commit" ? "COMMIT" : "ROLLBACK",
        kind: "transaction",
        missingWhere: false,
      },
    ],
    warnings: [
      "A command acknowledgement does not prove committed or rolled-back data effects. No automatic retry is performed.",
    ],
    requiresApproval: action === "begin",
    requiresPassphrase: false,
    confirmationText: null,
    expiresAt: Date.now() + 60_000,
  };
}

async function createSyntheticSnowflakeConsole(
  page: Page,
  handle: (command: string, body: Record<string, unknown>) => Promise<unknown | typeof passThrough>,
  firstBoundary: "begin" | "commit" = "begin",
  safeMode: ConnectionProfile["safeMode"] = "confirm",
) {
  await page.clock.install();
  await page.route(`${rpc}*`, async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const body = route.request().postDataJSON() as Record<string, unknown>;
    try {
      const result = await handle(command, body);
      if (result === passThrough) {
        await route.continue();
        return;
      }
      await route.fulfill({ json: { result } });
    } catch (error) {
      await route.fulfill({
        json: {
          error: {
            code: "policyBlocked",
            message: error instanceof Error ? error.message : "Unexpected synthetic RPC",
          },
        },
      });
    }
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Database engine").selectOption("snowflake");
  await page.getByLabel("Snowflake authentication").selectOption("externalBrowser");
  await page.getByLabel("Connection name", { exact: true }).fill("Synthetic SDK transactions");
  await page
    .getByLabel("Snowflake account hostname")
    .fill("fixture.invalid.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("DB");
  await page.getByLabel("Username", { exact: true }).fill("ANALYST");
  await page.getByLabel("Role", { exact: true }).fill("ANALYST");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH");
  await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
  await page.getByLabel("Safe Mode").selectOption(safeMode);
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  await expect(page.getByLabel("Active schema", { exact: true })).toHaveValue("PUBLIC");
  await expect(
    page.getByRole("button", {
      name: firstBoundary === "begin" ? "Begin transaction" : "Commit transaction",
    }),
  ).toBeEnabled();
}

// These RPC interceptions exercise UI scheduling and displayed command evidence only.
// The helper loopback suite separately qualifies protocol behavior; this never signs in.
test("synthetic SDK BEGIN reviews once, blocks competing work, then permits rollback-only recovery", async ({
  page,
}) => {
  const sessionId = "synthetic-snowflake-transactions";
  let profile!: ConnectionProfile;
  let observed = runtime(sessionId, "begin-authority", ["begin"]);
  const calls: Array<{ command: string; body: Record<string, any> }> = [];
  let releaseBegin!: () => void;
  let enteredBegin!: () => void;
  const heldBegin = new Promise<void>((resolve) => {
    releaseBegin = resolve;
  });
  const beginEntered = new Promise<void>((resolve) => {
    enteredBegin = resolve;
  });

  await createSyntheticSnowflakeConsole(page, async (command, body) => {
    calls.push({ command, body });
    switch (command) {
      case "save_profile":
        profile = body.profile as ConnectionProfile;
        expect(profile.safeMode).toBe("confirm");
        return profile;
      case "prepare_connection":
        return "synthetic-attempt";
      case "connect_prepared":
        return {
          id: sessionId,
          connectionId: profile.id,
          engine: "snowflake",
          serverVersion: "synthetic fixture",
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
      case "session_contexts":
        return { state: observed, available: ["PUBLIC", "OTHER"], revision: "schema-revision" };
      case "completion_catalog":
        return {
          revision: "completion-revision",
          identity: {
            database: "DB",
            role: "ANALYST",
            searchPath: ["DB.PUBLIC"],
            caseSensitiveTableNames: true,
            contextFingerprint: observed.contextFingerprint,
          },
          tables: [],
          unqualifiedTables: [],
        };
      case "health":
        return 1;
      case "relation_catalog":
        return {
          revision: "relation-revision",
          database: "DB",
          identity: {
            database: "DB",
            role: "ANALYST",
            searchPath: ["DB.PUBLIC"],
            caseSensitiveTableNames: true,
          },
          relations: [],
        };
      case "prepare_transaction_control": {
        const request = body.request as { action: ExplicitTransactionAction; revision: string };
        const action = request.action;
        expect(["begin", "rollback"]).toContain(action);
        expect(body).toEqual({
          sessionId,
          request: {
            action,
            revision: action === "begin" ? "begin-authority" : "rollback-only-authority",
          },
        });
        return controlPlan(action, sessionId);
      }
      case "authorize_plan":
        expect(body.planId).toBe("synthetic-begin-plan");
        return "synthetic-begin-approval";
      case "execute_transaction_control": {
        if (body.planId === "synthetic-rollback-plan") {
          expect(body).toEqual({ planId: "synthetic-rollback-plan" });
          observed = runtime(sessionId, "after-rollback", ["begin"]);
          return boundaryResult("rollback", observed);
        }
        expect(body).toEqual({
          planId: "synthetic-begin-plan",
          approval: "synthetic-begin-approval",
        });
        enteredBegin();
        await heldBegin;
        observed = runtime(sessionId, "rollback-only-authority", [], {
          transactionState: "unknown",
          transactionObservation: { status: "unavailable" },
          transactionControlAuthority: {
            status: "rollbackOnly",
            revision: "rollback-only-authority",
          },
        });
        return boundaryResult("begin", observed);
      }
      default:
        return passThrough;
    }
  });

  await page.getByRole("button", { name: "Browse relations", exact: true }).click();
  const relations = page.getByRole("dialog", { name: "Browse relations" });
  await expect(relations.getByRole("status")).toContainText("0 visible relations · DB");
  const relationCatalogCallsBeforeControl = calls.filter(
    (call) => call.command === "relation_catalog",
  ).length;
  // The modal correctly makes Workbench transaction controls inert. A user
  // must close it before beginning a boundary action.
  await expect(page.getByRole("button", { name: "Begin transaction" })).toBeHidden();
  await page.keyboard.press("Escape");
  await expect(relations).toHaveCount(0);
  await page.getByRole("button", { name: "Refresh completion context" }).click();
  await expect(page.getByRole("button", { name: "Begin transaction" })).toBeEnabled();
  await page.getByRole("button", { name: "Begin transaction" }).click();
  const review = page.getByRole("dialog", { name: "Review this operation" });
  await expect(review).toBeVisible();
  await expect(review.locator(".sql-preview")).toHaveText("BEGIN TRANSACTION");
  await expect(review).toContainText("A command acknowledgement does not prove");
  await expect(review).toContainText("No automatic retry is performed");
  await expect(review.getByRole("button", { name: "Authorize and run" })).toBeEnabled();
  expect(calls.filter((call) => call.command === "prepare_transaction_control")).toHaveLength(1);
  expect(calls.filter((call) => call.command === "authorize_plan")).toHaveLength(0);

  const authorize = review.getByRole("button", { name: "Authorize and run" });
  await authorize.focus();
  await expect(authorize).toBeFocused();
  await page.keyboard.press("Enter");
  await beginEntered;
  await expect(relations).toHaveCount(0);
  const quietCommands = () =>
    calls
      .filter((call) => ["health", "completion_catalog", "session_contexts"].includes(call.command))
      .map((call) => call.command);
  const beforePendingWindow = quietCommands();
  await expect(page.getByRole("button", { name: "Run all statements" })).toBeHidden();
  await expect(
    page.getByRole("button", { name: "Run all statements", includeHidden: true }),
  ).toBeDisabled();
  await expect(page.getByLabel("Active schema", { exact: true })).toBeDisabled();
  await page.clock.fastForward("00:31");
  expect(quietCommands()).toEqual(beforePendingWindow);
  expect(calls.filter((call) => call.command === "relation_catalog")).toHaveLength(
    relationCatalogCallsBeforeControl,
  );

  releaseBegin();
  await expect(
    page.getByText("BEGIN acknowledged; only rollback recovery is available", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Transaction state unknown", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("status").filter({ hasText: /^Only rollback recovery is available$/ }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Begin transaction" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Commit transaction" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Rollback transaction" })).toBeEnabled();
  await expect(
    page.getByText("Refresh completion context after the transaction action", { exact: true }),
  ).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Refresh completion context" })).toBeDisabled();
  expect(quietCommands()).toEqual(beforePendingWindow);

  const contextCallsBeforeRollback = calls.filter(
    (call) => call.command === "session_contexts",
  ).length;
  await page.getByRole("button", { name: "Rollback transaction" }).click();
  await expect(page.getByText("ROLLBACK acknowledged", { exact: true })).toBeVisible();
  expect(
    calls
      .filter((call) => call.command === "prepare_transaction_control")
      .map(({ body }) => (body.request as { action: ExplicitTransactionAction }).action),
  ).toEqual(["begin", "rollback"]);
  expect(calls.filter((call) => call.command === "authorize_plan")).toHaveLength(1);
  expect(calls.filter((call) => call.command === "execute_transaction_control")).toHaveLength(2);
  expect(
    calls.some(({ command }) =>
      [
        "prepare_execution",
        "execute",
        "set_transaction_mode",
        "commit_transaction",
        "rollback_transaction",
      ].includes(command),
    ),
  ).toBe(false);
  expect(calls.filter((call) => call.command === "session_contexts")).toHaveLength(
    contextCallsBeforeRollback,
  );
  expect(calls.filter((call) => call.command === "relation_catalog")).toHaveLength(
    relationCatalogCallsBeforeControl,
  );
  await expect(
    page.getByText("Refresh completion context after the transaction action", { exact: true }),
  ).toBeVisible();
  expect(quietCommands()).toEqual(beforePendingWindow);
});

test("synthetic SDK unknown boundary outcome retires without replay or reconnect", async ({
  page,
}) => {
  const sessionId = "synthetic-snowflake-unknown-boundary";
  let profile!: ConnectionProfile;
  const observed = runtime(sessionId, "begin-authority", ["begin"]);
  const calls: Array<{ command: string; body: Record<string, any> }> = [];

  await createSyntheticSnowflakeConsole(page, async (command, body) => {
    calls.push({ command, body });
    switch (command) {
      case "save_profile":
        profile = body.profile as ConnectionProfile;
        return profile;
      case "prepare_connection":
        return "synthetic-attempt";
      case "connect_prepared":
        return {
          id: sessionId,
          connectionId: profile.id,
          engine: "snowflake",
          serverVersion: "synthetic fixture",
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
      case "session_contexts":
        return { state: observed, available: ["PUBLIC"], revision: "schema-revision" };
      case "completion_catalog":
        return {
          revision: "completion-revision",
          identity: {
            database: "DB",
            role: "ANALYST",
            searchPath: ["DB.PUBLIC"],
            caseSensitiveTableNames: true,
            contextFingerprint: observed.contextFingerprint,
          },
          tables: [],
          unqualifiedTables: [],
        };
      case "health":
        return 1;
      case "prepare_transaction_control":
        return controlPlan("begin", sessionId);
      case "authorize_plan":
        return "synthetic-unknown-approval";
      case "execute_transaction_control":
        return boundaryResult(
          "begin",
          runtime(sessionId, "unknown", [], {
            transactionState: "unknown",
            transactionObservation: { status: "unavailable" },
          }),
          { boundaryOutcome: "unknown", retired: true },
        );
      default:
        return passThrough;
    }
  });

  await page.getByRole("button", { name: "Begin transaction" }).click();
  const review = page.getByRole("dialog", { name: "Review this operation" });
  await expect(review).toBeVisible();
  await review.getByRole("button", { name: "Authorize and run" }).click();
  await expect(
    page.getByText("BEGIN outcome unknown; session retired", { exact: true }),
  ).toBeVisible();
  await page.clock.fastForward("00:31");

  expect(calls.filter((call) => call.command === "prepare_transaction_control")).toHaveLength(1);
  expect(calls.filter((call) => call.command === "authorize_plan")).toHaveLength(1);
  expect(calls.filter((call) => call.command === "execute_transaction_control")).toHaveLength(1);
  expect(calls.filter((call) => call.command === "prepare_connection")).toHaveLength(1);
  expect(calls.filter((call) => call.command === "connect_prepared")).toHaveLength(1);
  expect(
    calls.some(({ command }) =>
      [
        "prepare_execution",
        "execute",
        "set_transaction_mode",
        "commit_transaction",
        "rollback_transaction",
      ].includes(command),
    ),
  ).toBe(false);
});

test("synthetic SDK immediate COMMIT acknowledgement does not trigger background observation", async ({
  page,
}) => {
  const sessionId = "synthetic-snowflake-immediate-commit";
  const current = runtime(sessionId, "commit-authority", ["commit"], {
    transactionMode: "manual",
    transactionState: "inTransaction",
    transactionObservation: {
      status: "observed",
      transactionId: "opaque-transaction-id",
      abortOnError: false,
      usability: "unknown",
    },
  });
  const calls: Array<{ command: string; body: Record<string, any> }> = [];
  let profile!: ConnectionProfile;
  const next = runtime(sessionId, "after-commit", []);

  await createSyntheticSnowflakeConsole(
    page,
    async (command, body) => {
      calls.push({ command, body });
      switch (command) {
        case "save_profile":
          profile = body.profile as ConnectionProfile;
          expect(profile.safeMode).toBe("silent");
          return profile;
        case "prepare_connection":
          return "synthetic-attempt";
        case "connect_prepared":
          return {
            id: sessionId,
            connectionId: profile.id,
            engine: "snowflake",
            serverVersion: "synthetic fixture",
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
        case "session_contexts":
          return { state: current, available: ["PUBLIC"], revision: "context-revision" };
        case "completion_catalog":
          return {
            revision: "completion-revision",
            identity: {
              database: "DB",
              role: "ANALYST",
              searchPath: ["DB.PUBLIC"],
              caseSensitiveTableNames: true,
              contextFingerprint: current.contextFingerprint,
            },
            tables: [],
            unqualifiedTables: [],
          };
        case "health":
          return 1;
        case "prepare_transaction_control":
          expect(body).toEqual({
            sessionId,
            request: { action: "commit", revision: "commit-authority" },
          });
          return controlPlan("commit", sessionId);
        case "execute_transaction_control":
          expect(body).toEqual({ planId: "synthetic-commit-plan" });
          return boundaryResult("commit", next);
        default:
          return passThrough;
      }
    },
    "commit",
    "silent",
  );

  const contextCallsBeforeCommit = calls.filter((call) =>
    ["session_contexts", "completion_catalog", "health"].includes(call.command),
  ).length;
  await page.getByRole("button", { name: "Commit transaction" }).click();
  await expect(page.getByText("COMMIT acknowledged", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Refresh completion context after the transaction action", { exact: true }),
  ).toBeVisible();
  await page.clock.fastForward("00:31");
  expect(calls.filter((call) => call.command === "authorize_plan")).toHaveLength(0);
  expect(
    calls.filter((call) =>
      ["session_contexts", "completion_catalog", "health"].includes(call.command),
    ),
  ).toHaveLength(contextCallsBeforeCommit);
  expect(
    calls.some(({ command }) =>
      [
        "prepare_execution",
        "execute",
        "set_transaction_mode",
        "commit_transaction",
        "rollback_transaction",
      ].includes(command),
    ),
  ).toBe(false);
});
