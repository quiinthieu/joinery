import { expect, test } from "./fixtures";
import type {
  ConnectionProfile,
  DatabaseObjectSummary,
  TableSchema,
} from "../../src/ipc/generated";

const rpc = "http://127.0.0.1:1422/rpc/";

test("synthetic Snowflake SDK completion invalidates stale metadata on fingerprint changes", async ({
  page,
}) => {
  const calls: string[] = [];
  const schemaCalls: Array<{ sessionId: string; revision: string }> = [];
  const catalogIdentities: Array<{
    revision: string;
    database: string;
    role: string;
    searchPath: string[];
    caseSensitiveTableNames: boolean;
    contextFingerprint: string;
  }> = [];
  const table = { schema: "PUBLIC", name: "ORDERS" };
  let profile!: ConnectionProfile;
  let completionCatalogCalls = 0;
  let snapshotFingerprint:
    "synthetic-fingerprint-a" | "synthetic-fingerprint-b" | "synthetic-fingerprint-c" =
    "synthetic-fingerprint-a";
  let failNextRefresh = false;
  let failNextSchema = false;
  let holdNextRevisionBSchema = false;
  let resolveOldSchema!: () => void;
  let announceOldSchema!: () => void;
  let releaseSameRevisionSchema!: () => void;
  let announceSameRevisionSchema!: () => void;
  let announceSameRevisionError!: () => void;
  const oldSchemaEntered = new Promise<void>((resolve) => {
    announceOldSchema = resolve;
  });
  const oldSchemaReleased = new Promise<void>((resolve) => {
    resolveOldSchema = resolve;
  });
  const sameRevisionSchemaEntered = new Promise<void>((resolve) => {
    announceSameRevisionSchema = resolve;
  });
  const sameRevisionSchemaReleased = new Promise<void>((resolve) => {
    releaseSameRevisionSchema = resolve;
  });
  const sameRevisionErrorServed = new Promise<void>((resolve) => {
    announceSameRevisionError = resolve;
  });

  const schema = (column: string): TableSchema => ({
    table,
    columns: [
      {
        id: "column-0",
        name: column,
        databaseType: "text",
        kind: "text",
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: false,
        insertable: false,
        hasDefault: false,
      },
    ],
    primaryKey: [],
    foreignKeys: [],
    editable: false,
  });

  // All connection and context responses in this test are synthetic. In
  // particular, externalBrowser is serialized but no browser SSO is performed.
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
          id: "synthetic-sdk-session",
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
            tableEditing: false,
            tableFiltering: false,
            processes: false,
          },
        };
        break;
      case "session_contexts":
        await route.fulfill({
          json: {
            error: {
              code: "unsupported",
              message: "Live Snowflake SDK session state is unsupported in this slice",
            },
          },
        });
        return;
      case "completion_catalog":
        completionCatalogCalls += 1;
        if (failNextRefresh) {
          failNextRefresh = false;
          await route.fulfill({
            json: {
              error: {
                code: "unsupported",
                message: "Synthetic context refresh is unsupported",
              },
            },
          });
          return;
        }
        const revision =
          snapshotFingerprint === "synthetic-fingerprint-a"
            ? "synthetic-revision-a"
            : snapshotFingerprint === "synthetic-fingerprint-b"
              ? "synthetic-revision-b"
              : "synthetic-revision-c";
        const identity = {
          database: "DB",
          role: "ANALYST",
          searchPath: ["DB.PUBLIC"],
          caseSensitiveTableNames: true,
          contextFingerprint: snapshotFingerprint,
        };
        catalogIdentities.push({ revision, ...identity });
        result = {
          revision,
          identity,
          tables: [{ table, kind: "BASE TABLE", estimatedRows: null, totalBytes: null }],
          unqualifiedTables: [table],
        };
        break;
      case "completion_schema": {
        const body = route.request().postDataJSON();
        const revision = String(body.revision ?? "");
        schemaCalls.push({ sessionId: String(body.sessionId), revision });
        expect(body.sessionId).toBe("synthetic-sdk-session");
        if (revision === "synthetic-revision-a") {
          announceOldSchema();
          await oldSchemaReleased;
          await route.fulfill({
            json: {
              error: {
                code: "connectionClosed",
                message: "Synthetic old context was invalidated",
                completionInvalidated: true,
              },
            },
          });
          return;
        } else if (revision === "synthetic-revision-b" && holdNextRevisionBSchema) {
          holdNextRevisionBSchema = false;
          announceSameRevisionSchema();
          await sameRevisionSchemaReleased;
          await route.fulfill({
            json: {
              error: {
                code: "connectionClosed",
                message: "Synthetic same-revision response was invalidated",
                completionInvalidated: true,
              },
            },
          });
          announceSameRevisionError();
          return;
        } else if (revision === "synthetic-revision-c" && failNextSchema) {
          failNextSchema = false;
          await route.fulfill({
            json: {
              error: {
                code: "connectionClosed",
                message: "Synthetic SDK session context changed",
                completionInvalidated: true,
              },
            },
          });
          return;
        } else if (revision === "synthetic-revision-c") {
          result = schema("RECOVERED_COLUMN");
        } else {
          expect(revision).toBe("synthetic-revision-b");
          result = schema("NEW_COLUMN");
        }
        break;
      }
      case "health":
        result = 1;
        break;
      case "release_result":
        result = null;
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
  await page.getByLabel("Connection name", { exact: true }).fill("Synthetic SDK completion");
  await page
    .getByLabel("Snowflake account hostname")
    .fill("fixture.invalid.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("DB");
  await page.getByLabel("Username", { exact: true }).fill("ANALYST");
  await page.getByLabel("Role", { exact: true }).fill("ANALYST");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH_A");
  await page.getByLabel("Schema", { exact: true }).fill("PUBLIC");
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".status-bar")).toContainText("Synthetic SDK completion");
  expect(profile.snowflake?.authentication).toBe("externalBrowser");

  await page.getByRole("button", { name: "New query tab" }).click();
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  await expect(editor).toBeVisible();
  await expect.poll(() => completionCatalogCalls).toBeGreaterThan(0);
  await expect(page.locator(".completion-context")).toContainText("ANALYST");

  await editor.fill("SELECT * FROM DB.PUBLIC.");
  await editor.press("End");
  await page.keyboard.type("OR");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText('DB."PUBLIC".ORDERS');

  await editor.fill("SELECT o. FROM DB.PUBLIC.ORDERS o");
  await editor.press("Home");
  for (let index = 0; index < 9; index++) await editor.press("ArrowRight");
  await editor.press("Control+Space");
  await expect(oldSchemaEntered).resolves.toBeUndefined();
  const catalogCallsBeforeRefresh = completionCatalogCalls;
  snapshotFingerprint = "synthetic-fingerprint-b";
  await page.getByRole("button", { name: "Refresh completion context" }).click();
  await expect.poll(() => completionCatalogCalls).toBeGreaterThan(catalogCallsBeforeRefresh);
  await expect
    .poll(() => catalogIdentities.some((item) => item.contextFingerprint === snapshotFingerprint))
    .toBe(true);
  expect(catalogIdentities.some((item) => item.revision === "synthetic-revision-a")).toBe(true);
  expect(catalogIdentities.at(-1)).toEqual({
    revision: "synthetic-revision-b",
    database: "DB",
    role: "ANALYST",
    searchPath: ["DB.PUBLIC"],
    caseSensitiveTableNames: true,
    contextFingerprint: "synthetic-fingerprint-b",
  });
  for (const identity of catalogIdentities) {
    expect(identity.database).toBe("DB");
    expect(identity.role).toBe("ANALYST");
    expect(identity.searchPath).toEqual(["DB.PUBLIC"]);
  }

  resolveOldSchema();
  await expect
    .poll(() => schemaCalls.filter((item) => item.revision === "synthetic-revision-a").length)
    .toBeGreaterThan(0);
  await expect
    .poll(() => schemaCalls.filter((item) => item.revision === "synthetic-revision-b").length)
    .toBeGreaterThan(0);
  expect(schemaCalls.every((item) => item.sessionId === "synthetic-sdk-session")).toBe(true);
  await editor.press("Home");
  for (let index = 0; index < 9; index++) await editor.press("ArrowRight");
  await page.keyboard.type("N");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("o.NEW_COLUMN");
  await expect(page.locator(".cm-tooltip-autocomplete")).not.toContainText("STALE_COLUMN");

  // A late invalidation from revision A must not clear the replacement B catalog or cache.
  await editor.fill("SELECT * FROM DB.PUBLIC.");
  await editor.press("End");
  await page.keyboard.type("OR");
  await editor.press("Control+Space");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText('DB."PUBLIC".ORDERS');

  // A manual refresh can replace a schema cache without changing backend revision.
  holdNextRevisionBSchema = true;
  const catalogCallsBeforeHeldB = completionCatalogCalls;
  await page.getByRole("button", { name: "Refresh completion context" }).click();
  await expect.poll(() => completionCatalogCalls).toBeGreaterThan(catalogCallsBeforeHeldB);
  await editor.fill("SELECT o. FROM DB.PUBLIC.ORDERS o");
  await editor.press("Home");
  for (let index = 0; index < 9; index++) await editor.press("ArrowRight");
  await editor.press("Control+Space");
  await expect(sameRevisionSchemaEntered).resolves.toBeUndefined();

  const catalogCallsBeforeSameB = completionCatalogCalls;
  await page.getByRole("button", { name: "Refresh completion context" }).click();
  await expect.poll(() => completionCatalogCalls).toBeGreaterThan(catalogCallsBeforeSameB);
  await editor.fill("SELECT o. FROM DB.PUBLIC.ORDERS o");
  await editor.press("Home");
  for (let index = 0; index < 9; index++) await editor.press("ArrowRight");
  await editor.press("Control+Space");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("o.NEW_COLUMN");
  releaseSameRevisionSchema();
  await expect(sameRevisionErrorServed).resolves.toBeUndefined();
  await expect(page.locator(".completion-context")).toContainText("ANALYST");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("o.NEW_COLUMN");

  // A current lazy-schema invalidation clears cached table and column suggestions until refresh.
  snapshotFingerprint = "synthetic-fingerprint-c";
  const catalogCallsBeforeC = completionCatalogCalls;
  await page.getByRole("button", { name: "Refresh completion context" }).click();
  await expect.poll(() => completionCatalogCalls).toBeGreaterThan(catalogCallsBeforeC);
  await expect(page.locator(".completion-context")).toContainText("ANALYST");
  failNextSchema = true;
  await editor.fill("SELECT o. FROM DB.PUBLIC.ORDERS o");
  await editor.press("Home");
  for (let index = 0; index < 9; index++) await editor.press("ArrowRight");
  await editor.press("Control+Space");
  await expect
    .poll(() => schemaCalls.filter((item) => item.revision === "synthetic-revision-c").length)
    .toBe(1);
  await expect(page.locator(".completion-context")).toContainText(
    "Completion unavailable: Synthetic SDK session context changed",
  );
  await editor.fill("SELECT * FROM DB.PUBLIC.");
  await editor.press("End");
  await page.keyboard.type("OR");
  await editor.press("Control+Space");
  await expect(page.locator(".cm-tooltip-autocomplete")).toHaveCount(0);
  await editor.fill("SELECT o. FROM DB.PUBLIC.ORDERS o");
  await editor.press("Home");
  for (let index = 0; index < 9; index++) await editor.press("ArrowRight");
  await editor.press("Control+Space");
  await expect(page.locator(".cm-tooltip-autocomplete")).toHaveCount(0);

  const catalogCallsBeforeRecovery = completionCatalogCalls;
  await page.getByRole("button", { name: "Refresh completion context" }).click();
  await expect.poll(() => completionCatalogCalls).toBeGreaterThan(catalogCallsBeforeRecovery);
  await expect(page.locator(".completion-context")).toContainText("ANALYST");
  await editor.fill("SELECT o. FROM DB.PUBLIC.ORDERS o");
  await editor.press("Home");
  for (let index = 0; index < 9; index++) await editor.press("ArrowRight");
  await editor.press("Control+Space");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("o.RECOVERED_COLUMN");

  failNextRefresh = true;
  await page.getByRole("button", { name: "Refresh completion context" }).click();
  await expect(page.locator(".completion-context")).toContainText(
    "Synthetic context refresh is unsupported",
  );
  await expect(page.locator(".cm-tooltip-autocomplete")).toHaveCount(0);

  expect(calls.filter((command) => command === "prepare_connection")).toHaveLength(1);
  expect(calls.filter((command) => command === "connect_prepared")).toHaveLength(1);
  expect(calls).not.toContain("catalog");
  expect(calls).not.toContain("table_schema");
  expect(calls).not.toContain("browse_table");
  expect(calls).not.toContain("prepare_changes");
  expect(calls).not.toContain("set_transaction_mode");
  expect(calls).not.toContain("commit_transaction");
  expect(calls).not.toContain("rollback_transaction");
});

test("a late PostgreSQL object list cannot republish navigation after completion invalidation", async ({
  page,
}) => {
  let holdNextObjects = false;
  let objectsResponses = 0;
  let completionCatalogResponses = 0;
  let objectsSignature: string | null = null;
  const fulfilledObjectSignatures: string[] = [];
  let announceObjectsRequest!: () => void;
  let releaseObjectsRequest!: () => void;
  const objectsRequestEntered = new Promise<void>((resolve) => {
    announceObjectsRequest = resolve;
  });
  const objectsRequestReleased = new Promise<void>((resolve) => {
    releaseObjectsRequest = resolve;
  });
  const staleSignature = "stale-object-list-marker";
  const initialSignature = "initial-editor-object-marker";
  const recoveredSignature = "recovered-object-list-marker";

  page.on("response", (response) => {
    if (new URL(response.url()).pathname.endsWith("/rpc/completion_catalog"))
      completionCatalogResponses++;
  });

  await page.route("**/rpc/database_objects", async (route) => {
    const responseSignature = objectsSignature;
    const response = await route.fetch();
    const body = (await response.json()) as {
      result: DatabaseObjectSummary[];
      error?: unknown;
    };
    expect(body.error).toBeUndefined();
    let result = body.result;
    if (responseSignature) {
      let foundFunction = false;
      result = body.result.map((summary) => {
        if (
          summary.object.schema === "public" &&
          summary.object.name === "inspector_touch_fn" &&
          summary.object.kind === "function"
        ) {
          foundFunction = true;
          return { ...summary, signature: responseSignature };
        }
        return summary;
      });
      expect(foundFunction).toBe(true);
    }
    if (holdNextObjects) {
      holdNextObjects = false;
      announceObjectsRequest();
      await objectsRequestReleased;
    }
    await route.fulfill({ response, json: { ...body, result } });
    objectsResponses++;
    if (responseSignature) fulfilledObjectSignatures.push(responseSignature);
  });
  await page.route("**/rpc/completion_schema", async (route) => {
    await route.fulfill({
      json: {
        error: {
          code: "connectionClosed",
          message: "Synthetic PostgreSQL context invalidation",
          completionInvalidated: true,
        },
      },
    });
  });
  await page.route("**/rpc/object_details", async (route) => {
    const { object } = route.request().postDataJSON();
    if (object?.name === "inspector_touch_fn") {
      await route.fulfill({
        json: {
          error: {
            code: "unsupported",
            message: "Synthetic routine details are unavailable",
          },
        },
      });
      return;
    }
    await route.continue();
  });

  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Database engine").selectOption("postgres");
  await page
    .getByLabel("Connection name", { exact: true })
    .fill(`Completion invalidation ${Date.now()}`);
  await page.getByLabel("Host", { exact: true }).fill("127.0.0.1");
  await page
    .getByLabel("Port", { exact: true })
    .fill(String(Number(process.env.JOINERY_POSTGRES_PORT ?? "55432")));
  await page.getByLabel("Database", { exact: true }).fill("joinery");
  await page.getByLabel("Username", { exact: true }).fill("joinery");
  await page.getByLabel("Password", { exact: true }).fill("joinery-local-only");
  await page.getByLabel("Require TLS and verify the server certificate").uncheck();
  await page.getByLabel("Safe Mode", { exact: true }).selectOption("readOnly");
  await page.getByRole("button", { name: "Save and connect" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("button", { name: /^public\b/ })).toBeVisible();

  // Sidebar connection discovery has settled; mark the separately loaded editor object list.
  objectsSignature = initialSignature;
  await page.getByRole("button", { name: "New query tab" }).click();
  await expect(page.locator(".completion-context")).toContainText("joinery");
  await expect.poll(() => fulfilledObjectSignatures).toContain(initialSignature);

  const editor = page.getByRole("textbox", { name: "SQL editor" });
  const functionSql = "SELECT public.inspector_touch_fn()";
  await editor.fill(functionSql);
  await editor.press("Home");
  for (let index = 0; index < functionSql.indexOf("inspector_touch_fn") + 4; index++)
    await editor.press("ArrowRight");
  await editor.press("F1");
  const docs = page.getByRole("region", { name: "Quick SQL documentation" });
  await expect(docs).toContainText(initialSignature);
  await page.getByRole("button", { name: "Close SQL symbol inspector" }).click();
  await editor.fill("SELECT 1");

  // Hold the editor's own response after its refresh, not the completed connection/sidebar load.
  objectsSignature = staleSignature;
  holdNextObjects = true;
  const catalogResponsesBeforeHeld = completionCatalogResponses;
  await page.getByRole("button", { name: "Refresh completion context" }).click();
  await expect.poll(() => completionCatalogResponses).toBeGreaterThan(catalogResponsesBeforeHeld);
  await expect(objectsRequestEntered).resolves.toBeUndefined();

  await editor.fill("SELECT o. FROM public.orders o");
  await page.keyboard.press("Control+Space");
  await expect(page.locator(".completion-context")).toContainText(
    "Completion unavailable: Synthetic PostgreSQL context invalidation",
  );
  const catalogResponsesAtInvalidation = completionCatalogResponses;
  releaseObjectsRequest();
  await expect.poll(() => fulfilledObjectSignatures).toContain(staleSignature);
  await expect.poll(() => completionCatalogResponses).toBe(catalogResponsesAtInvalidation);
  await editor.fill(functionSql);
  await editor.press("Home");
  for (let index = 0; index < functionSql.indexOf("inspector_touch_fn") + 4; index++)
    await editor.press("ArrowRight");
  await editor.press("F1");
  await expect(page.locator("body")).not.toContainText(staleSignature);
  await expect(page.locator(".completion-context")).toContainText(
    "Completion unavailable: Synthetic PostgreSQL context invalidation",
  );

  // A user refresh creates a new cache generation and can publish the current object list.
  objectsSignature = recoveredSignature;
  const objectsResponsesBeforeRefresh = objectsResponses;
  const catalogResponsesBeforeRecovery = completionCatalogResponses;
  await page.getByRole("button", { name: "Refresh completion context" }).click();
  await expect(page.locator(".completion-context")).toContainText("joinery");
  await expect.poll(() => objectsResponses).toBeGreaterThan(objectsResponsesBeforeRefresh);
  await expect
    .poll(() => completionCatalogResponses)
    .toBeGreaterThan(catalogResponsesBeforeRecovery);
  expect(fulfilledObjectSignatures).toContain(recoveredSignature);
  await editor.fill(functionSql);
  await editor.press("Home");
  for (let index = 0; index < functionSql.indexOf("inspector_touch_fn") + 4; index++)
    await editor.press("ArrowRight");
  await editor.press("F1");
  await expect(docs).toContainText(recoveredSignature);
});
