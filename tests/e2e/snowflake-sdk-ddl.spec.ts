import { expect, test, type Page } from "./fixtures";
import type {
  BrowseCatalog,
  ConnectionProfile,
  PreparedPlan,
  SessionRuntimeState,
  SnowflakeDdlAction,
  SnowflakeDdlKind,
  SnowflakeDdlResult,
} from "../../src/ipc/generated";

// Real browser UI with synthetic RPC replies. Native/SDK tests cover dispatch;
// no browser SSO, account data or Snowflake effects are claimed here.
const rpc = "http://127.0.0.1:1422/rpc/";
const sessionId = "synthetic-ddl-console";
const qualifiedTarget = '"DB"."SCRATCH"."DDL_TABLE"';
function runtime(): SessionRuntimeState {
  return {
    sessionId,
    database: "DB",
    role: "ANALYST",
    searchPath: ["DB.SCRATCH"],
    contextKind: "schema",
    activeContext: "SCRATCH",
    contextFingerprint: "stable",
    transactionMode: "autoCommit",
    transactionState: "idle",
    transactionObservation: {
      status: "observed",
      transactionId: null,
      abortOnError: false,
      usability: "unknown",
    },
    transactionControlAuthority: { status: "observed", revision: "tx", allowedActions: ["begin"] },
  };
}
const catalog: BrowseCatalog = {
  revision: "catalog",
  database: "DB",
  identity: {
    database: "DB",
    role: "ANALYST",
    searchPath: ["DB.SCRATCH"],
    contextFingerprint: "stable",
    caseSensitiveTableNames: true,
  },
  relations: [
    {
      targetId: "opaque",
      database: "DB",
      table: { schema: "SCRATCH", name: "DDL_TABLE" },
      tableType: "BASE TABLE",
      traits: {
        transient: true,
        temporary: false,
        iceberg: false,
        dynamic: false,
        hybrid: false,
        immutable: false,
      },
      availability: "available",
    },
  ],
};
async function setup(
  page: Page,
  kind: SnowflakeDdlKind,
  execute: () => Promise<SnowflakeDdlResult>,
) {
  const calls: Array<{ command: string; body: Record<string, any> }> = [];
  let profile!: ConnectionProfile;
  let action!: SnowflakeDdlAction;
  let failReconcile = true;
  const sql =
    kind === "createTable"
      ? `CREATE TABLE ${qualifiedTarget} ("ID" NUMBER(38, 0))`
      : kind === "addColumn"
        ? `ALTER TABLE ${qualifiedTarget} ADD COLUMN "NEW_COLUMN" NUMBER(38, 0)`
        : `DROP TABLE ${qualifiedTarget} RESTRICT`;
  await page.clock.install();
  await page.route(`${rpc}*`, async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const body = route.request().postDataJSON();
    calls.push({ command, body });
    let result: unknown;
    switch (command) {
      case "save_profile":
        profile = body.profile;
        expect(profile.snowflake?.reviewedDdl).toBe(true);
        expect(profile.snowflake?.reviewedDml).toBe(false);
        result = profile;
        break;
      case "prepare_connection":
        result = "attempt";
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
            snowflakeReviewedDdl: true,
            ddlEditing: false,
          },
        };
        break;
      case "session_contexts":
        result = { state: runtime(), available: ["SCRATCH"], revision: "schema" };
        break;
      case "completion_catalog":
        result = {
          revision: "completion",
          identity: catalog.identity,
          tables: [],
          unqualifiedTables: [],
        };
        break;
      case "health":
        result = 1;
        break;
      case "relation_catalog":
        result = catalog;
        break;
      case "inspect_relation":
        expect(body).toEqual({ sessionId, request: { revision: "catalog", targetId: "opaque" } });
        result = {
          revision: "inspection",
          targetId: "opaque",
          database: "DB",
          table: catalog.relations[0].table,
          constraints: "notFetched",
          columns: [],
        };
        break;
      case "preflight_snowflake_ddl":
        expect(body.request.sessionId).toBe(sessionId);
        action = body.request.action;
        expect(action.kind).toBe(kind);
        if (kind !== "createTable")
          expect(action).toMatchObject({ browseRevision: "catalog", targetId: "opaque" });
        if (kind === "addColumn")
          expect(action).toMatchObject({
            inspectionRevision: "inspection",
            column: { nullable: true },
          });
        result = {
          revision: "ddl-grant",
          kind,
          database: "DB",
          schema: "SCRATCH",
          name: "DDL_TABLE",
          qualifiedTarget,
          sql,
          warnings: ["Transient tables have no Fail-safe. DDL cannot be rolled back."],
        };
        break;
      case "prepare_snowflake_ddl":
        expect(body).toEqual({ sessionId, request: { revision: "ddl-grant", action, sql } });
        result = {
          id: "ddl-plan",
          sessionId,
          reviewedSnowflakeDdl: true,
          statements: [{ sql, kind: "ddl", missingWhere: false }],
          requiresApproval: true,
          requiresPassphrase: false,
          confirmationText: "DDL account",
          targetConfirmationText: kind === "dropTable" ? qualifiedTarget : undefined,
          warnings: ["DDL cannot be rolled back. No automatic retry or catalog refresh occurs."],
          expiresAt: Date.now() + 60000,
        } satisfies PreparedPlan;
        break;
      case "authorize_snowflake_ddl":
        expect(body).toEqual({
          planId: "ddl-plan",
          passphrase: null,
          confirmation: "DDL account",
          targetConfirmation: kind === "dropTable" ? qualifiedTarget : null,
        });
        result = "approval";
        break;
      case "execute_snowflake_ddl":
        expect(body).toEqual({ planId: "ddl-plan", approval: "approval" });
        result = await execute();
        break;
      case "refresh_snowflake_ddl_metadata":
        expect(body).toEqual({ sessionId });
        if (failReconcile) {
          failReconcile = false;
          await route.fulfill({
            json: {
              error: { code: "database", message: "Synthetic catalog denied; hold retained" },
            },
          });
          return;
        }
        result = { state: runtime(), available: ["SCRATCH"], revision: "reconciled" };
        break;
      case "disconnect":
        expect(body).toEqual({ sessionId });
        result = null;
        break;
      case "execute_plan":
      case "prepare_table_object_ddl":
      case "authorize_plan":
        throw new Error("DDL entered the generic SQL route");
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
  await page.getByLabel("Connection name", { exact: true }).fill("DDL account");
  await page
    .getByLabel("Snowflake account hostname")
    .fill("fixture.invalid.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("DB");
  await page.getByLabel("Username", { exact: true }).fill("ANALYST");
  await page.getByLabel("Role", { exact: true }).fill("ANALYST");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH");
  await page.getByLabel("Schema", { exact: true }).fill("SCRATCH");
  await page.getByLabel("Safe Mode").selectOption("guard");
  await page.getByRole("checkbox", { name: /Allow separately reviewed CREATE TABLE/ }).check();
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  await expect(page.getByLabel("Active schema", { exact: true })).toHaveValue("SCRATCH");
  await page.getByRole("button", { name: "Browse relations", exact: true }).click();
  await expect(page.getByLabel("Relation", { exact: true })).toBeEnabled();
  if (kind === "createTable") {
    await page.getByRole("button", { name: "Create table…", exact: true }).click();
    await page.getByLabel("Table name", { exact: true }).fill("DDL_TABLE");
  } else {
    await page.getByLabel("Relation", { exact: true }).selectOption("opaque");
    if (kind === "addColumn") {
      await page.getByRole("button", { name: "Inspect relation", exact: true }).click();
      await page.getByRole("button", { name: "Add nullable column…", exact: true }).click();
    } else await page.getByRole("button", { name: "Drop table…", exact: true }).click();
  }
  await page.getByRole("button", { name: "Generate & review SQL", exact: true }).click();
  const review = page.getByRole("dialog", { name: "Review this operation" });
  await expect(review.locator(".sql-preview")).toHaveText(sql);
  await expect(review).toContainText("cannot be rolled back");
  await review.getByLabel("Type DDL account to confirm", { exact: true }).fill("DDL account");
  if (kind === "dropTable") {
    await expect(review.getByRole("button", { name: "Authorize and run" })).toBeDisabled();
    await review.getByLabel("Exact table name to confirm DROP").fill(qualifiedTarget);
  }
  await review.getByRole("button", { name: "Authorize and run", exact: true }).click();
  return calls;
}
for (const kind of ["createTable", "addColumn", "dropTable"] as const) {
  test(`reviewed SDK ${kind} uses exact target, retains ACK and requires explicit catalog reconciliation`, async ({
    page,
  }) => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    const calls = await setup(page, kind, async () => {
      await held;
      return {
        kind,
        qualifiedTarget,
        statementOutcome: "succeeded",
        summaryStatus: "acknowledged",
        catalogReconciliationRequired: true,
        state: runtime(),
        retired: false,
        error: null,
      };
    });
    await expect(
      page.getByRole("button", { name: "Disconnect to stop write", exact: true }),
    ).toBeVisible();
    const observers = () =>
      calls.filter(({ command }) =>
        ["health", "session_contexts", "completion_catalog", "relation_catalog"].includes(command),
      ).length;
    const before = observers();
    await page.clock.fastForward("00:31");
    expect(observers()).toBe(before);
    release();
    const notice = page.getByRole("status", { name: "Snowflake schema change outcome" });
    await expect(notice).toContainText("Statement acknowledged");
    await expect(notice).toContainText(qualifiedTarget);
    await page.clock.fastForward("00:31");
    expect(observers()).toBe(before);
    await page.getByRole("button", { name: "Refresh completion context" }).click();
    await expect
      .poll(
        () => calls.filter(({ command }) => command === "refresh_snowflake_ddl_metadata").length,
      )
      .toBe(1);
    await expect(
      page.getByRole("button", { name: "Browse relations", exact: true }),
    ).toBeDisabled();
    await page.getByRole("button", { name: "Refresh completion context" }).click();
    await expect(page.getByRole("button", { name: "Browse relations", exact: true })).toBeEnabled();
    expect(calls.filter(({ command }) => command === "execute_snowflake_ddl")).toHaveLength(1);
    expect(calls.filter(({ command }) => command === "execute_plan")).toHaveLength(0);
  });
}
