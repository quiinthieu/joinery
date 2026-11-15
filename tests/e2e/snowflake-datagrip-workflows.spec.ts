import { expect, test } from "./fixtures";
import { Table, Utf8, tableToIPC, vectorFromArray } from "apache-arrow";
import type {
  BrowseCatalog,
  ChangePreview,
  ColumnInfo,
  ConnectionProfile,
  DatabaseObjectSummary,
  ObjectDetails,
  PreparedPlan,
  QueryEvent,
  RelationInspection,
  RelationPreview,
  ResultInfo,
  SchemaDiagram,
  SchemaSnapshot,
  SessionRuntimeState,
  SnowflakeContextCatalog,
  SnowflakeGridResult,
  TableSchema,
} from "../../src/ipc/generated";

const rpc = "http://127.0.0.1:1422/rpc/";

test("Snowflake SQL editor supports plan snapshot comparison, role/warehouse/database switching, and live column inspection", async ({
  page,
}) => {
  const sessionId = "sf-authoring-session";
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));

  let profile!: ConnectionProfile;
  let runs = 0;
  let currentRole = "ANALYST";
  let currentWarehouse = "WH_SMALL";
  let currentDatabase = "SANDBOX";
  let currentSchema = "BILLIE_TESTING";
  let contextRevision = 1;
  const switchCalls: unknown[] = [];
  let inspectionSchemaCalls = 0;

  const planRaw1 =
    '{"GlobalStats":{"partitionsTotal":4,"partitionsAssigned":2,"bytesAssigned":8192},"Operations":[[{"id":0,"operation":"Result"},{"id":1,"operation":"TableScan","parentOperators":[0],"objects":["SANDBOX.BILLIE_TESTING.ORDERS"],"partitionsAssigned":2,"bytesAssigned":8192}]]}';
  const planRaw2 =
    '{"GlobalStats":{"partitionsTotal":4,"partitionsAssigned":1,"bytesAssigned":4096},"Operations":[[{"id":0,"operation":"Result"},{"id":1,"operation":"Filter","parentOperators":[0],"expressions":["O.ID = 1"]},{"id":2,"operation":"TableScan","parentOperators":[1],"objects":["SANDBOX.BILLIE_TESTING.ORDERS"],"partitionsAssigned":1,"bytesAssigned":4096}]]}';

  const runtimeState = (): SessionRuntimeState => ({
    sessionId,
    database: currentDatabase,
    role: currentRole,
    warehouse: currentWarehouse,
    searchPath: currentDatabase && currentSchema ? [`${currentDatabase}.${currentSchema}`] : [],
    contextKind: "schema",
    activeContext: currentSchema,
    contextFingerprint: `fp-${currentRole}-${currentWarehouse}-${currentDatabase}-${currentSchema}`,
    transactionMode: "autoCommit",
    transactionState: "idle",
    transactionObservation: {
      status: "observed",
      transactionId: null,
      abortOnError: false,
      usability: "unknown",
    },
    transactionControlAuthority: {
      status: "observed",
      revision: `tx-${contextRevision}`,
      allowedActions: ["begin"],
    },
  });

  const contextCatalog = (): SnowflakeContextCatalog => ({
    state: runtimeState(),
    revision: `ctx-rev-${contextRevision}`,
    availableRoles: ["ANALYST", "DATA_ENG"],
    availableWarehouses: ["WH_SMALL", "WH_LARGE"],
    availableDatabases: ["SANDBOX", "ANALYTICS"],
    availableSchemas:
      currentDatabase === "ANALYTICS" ? ["CORE", "MART"] : ["BILLIE_TESTING", "PUBLIC"],
  });

  const completionIdentity = () => ({
    database: currentDatabase,
    role: currentRole,
    searchPath: [`${currentDatabase}.${currentSchema}`],
    contextFingerprint: runtimeState().contextFingerprint,
    caseSensitiveTableNames: true,
  });

  await page.route(`${rpc}*`, async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const args = route.request().postDataJSON();
    let result: unknown;
    switch (command) {
      case "save_profile":
        profile = args.profile;
        result = profile;
        break;
      case "prepare_connection":
        result = "sf-authoring-attempt";
        break;
      case "connect_prepared":
        result = {
          id: sessionId,
          connectionId: profile.id,
          engine: "snowflake",
          serverVersion: "synthetic-8.42",
          latencyMs: 1,
          encrypted: true,
          capabilities: {
            sql: true,
            transactions: false,
            explicitTransactionControls: true,
            cancellation: true,
            tableBrowsing: false,
            relationBrowsing: true,
            objectExplorer: true,
            tableEditing: false,
            tableFiltering: false,
            processes: false,
          },
        };
        break;
      case "session_contexts":
        result = {
          state: runtimeState(),
          available: contextCatalog().availableSchemas,
          revision: `schema-rev-${contextRevision}`,
        };
        break;
      case "snowflake_session_contexts":
        result = contextCatalog();
        break;
      case "switch_snowflake_context": {
        switchCalls.push(args.request);
        contextRevision += 1;
        const target = args.request.target;
        if (target.kind === "role") currentRole = target.role;
        else if (target.kind === "warehouse") currentWarehouse = target.warehouse;
        else if (target.kind === "database") {
          currentDatabase = target.database;
          currentSchema = target.database === "ANALYTICS" ? "CORE" : "BILLIE_TESTING";
        } else if (target.kind === "schema") {
          currentDatabase = target.database;
          currentSchema = target.schema;
        }
        result = contextCatalog();
        break;
      }
      case "completion_catalog":
        result = {
          revision: `comp-rev-${contextRevision}`,
          identity: completionIdentity(),
          tables: [
            {
              table: { schema: currentSchema, name: "ORDERS" },
              kind: "BASE TABLE; transient=no; temporary=no; iceberg=no; dynamic=no; hybrid=no; immutable=no",
              estimatedRows: 10,
            },
          ],
          unqualifiedTables: [{ schema: currentSchema, name: "ORDERS" }],
        };
        break;
      case "completion_schema":
      case "inspection_schema": {
        if (command === "inspection_schema") inspectionSchemaCalls += 1;
        const schema: TableSchema = {
          table: args.table,
          columns: [
            {
              id: "c0",
              name: "ORDER_ID",
              databaseType: "NUMBER(38,0)",
              kind: "integer",
              nullable: false,
              primaryKey: false,
              foreignKey: false,
              editable: false,
              insertable: false,
              hasDefault: false,
            },
            {
              id: "c1",
              name: "PAYLOAD",
              databaseType: "VARIANT",
              kind: "json",
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
        };
        result = schema;
        break;
      }
      case "prepare_explain": {
        const plan: PreparedPlan = {
          id: `sf-explain-plan-${runs}`,
          sessionId,
          statements: [
            {
              sql: `EXPLAIN USING JSON\n${args.sql}`,
              kind: "read",
              missingWhere: false,
            },
          ],
          requiresApproval: false,
          requiresPassphrase: false,
          confirmationText: null,
          warnings: [],
          expiresAt: Date.now() + 60000,
        };
        result = plan;
        break;
      }
      case "execute_plan":
        result = `sf-explain-run-${++runs}`;
        break;
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

  await page.route("http://127.0.0.1:1422/events/*", async (route) => {
    const runId = `sf-explain-run-${runs}`;
    const resultInfo: ResultInfo = {
      id: `sf-plan-content-${runs}`,
      runId,
      statementIndex: 0,
      columns: [
        {
          id: "c0",
          name: "content",
          databaseType: "TEXT",
          kind: "text",
          nullable: false,
          primaryKey: false,
          foreignKey: false,
          editable: false,
          insertable: false,
          hasDefault: false,
        },
      ],
      retainedRows: 1,
      status: "complete",
      table: null,
    };
    const events: QueryEvent[] = [
      { type: "started", runId },
      { type: "statementStarted", runId, index: 0 },
      {
        type: "planContext",
        runId,
        state: runtimeState(),
      },
      { type: "result", result: resultInfo },
      { type: "statementCompleted", runId, index: 0, affectedRows: "1", elapsedMs: 2 },
      { type: "finished", runId, status: "completed", elapsedMs: 3 },
    ];
    await route.fulfill({ json: events });
  });

  await page.route("http://127.0.0.1:1422/result/sf-plan-content-*", async (route) => {
    const resultId = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const rawJson = resultId.endsWith("-1") ? planRaw1 : planRaw2;
    const table = new Table({ c0: vectorFromArray([rawJson], new Utf8()) });
    table.schema.metadata.set("joinery.protocol", "1");
    table.schema.metadata.set("joinery.resultId", resultId);
    table.schema.metadata.set("joinery.offset", "0");
    await route.fulfill({
      contentType: "application/vnd.apache.arrow.stream",
      body: Buffer.from(tableToIPC(table, "stream")),
    });
  });

  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Database engine").selectOption("snowflake");
  await page.getByLabel("Snowflake authentication").selectOption("externalBrowser");
  await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Authoring");
  await page
    .getByLabel("Snowflake account hostname")
    .fill("fixture.invalid.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("SANDBOX");
  await page.getByLabel("Username", { exact: true }).fill("ANALYST");
  await page.getByLabel("Role", { exact: true }).fill("ANALYST");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH_SMALL");
  await page.getByLabel("Schema", { exact: true }).fill("BILLIE_TESTING");
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  await expect(page.locator(".completion-context")).toContainText("SANDBOX · ANALYST");

  // 1. Snowflake EXPLAIN USING JSON with observed planContext enables Keep snapshot & Plan comparison
  const editor = page.getByRole("textbox", { name: "SQL editor" });
  await editor.fill('SELECT O."ORDER_ID" FROM "SANDBOX"."BILLIE_TESTING"."ORDERS" O');
  await page.getByRole("button", { name: "Explain", exact: true }).click();
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await page.getByRole("button", { name: "View plan", exact: true }).click();
  const planRegion = page.getByRole("region", { name: "Query execution plan", exact: true });
  const keepSnapshot = planRegion.getByRole("button", { name: "Keep snapshot", exact: true });
  await expect(keepSnapshot).toBeEnabled();
  await keepSnapshot.click();
  await expect(planRegion).toContainText("1/10 snapshots");

  await page.getByRole("button", { name: "Explain", exact: true }).click();
  await expect(page.locator(".result-footer .status-dot.completed")).toBeVisible();
  await page.getByRole("button", { name: "View plan", exact: true }).click();
  const compareSelect = page.getByLabel("Compare with snapshot");
  const snapshotId = await compareSelect.locator("option").nth(1).getAttribute("value");
  await compareSelect.selectOption(snapshotId!);
  await expect(page.getByRole("region", { name: "Plan comparison", exact: true })).toBeVisible();
  await expect(planRegion.locator(".plan-tree")).toHaveCount(2);

  // 2. Live Snowflake SQL column inspection via inspection_schema
  await editor.fill('SELECT O.ORDR_TYPO FROM "SANDBOX"."BILLIE_TESTING"."ORDERS" O');
  await page.getByRole("button", { name: "Inspect SQL columns", exact: true }).click();
  const inspectDialog = page.getByRole("dialog", { name: "SQL column inspection" });
  await expect(inspectDialog).toContainText("1 missing column reference");
  await inspectDialog.getByLabel("Replace with catalog column").selectOption("ORDER_ID");
  await inspectDialog.getByRole("button", { name: "Apply column correction" }).click();
  await expect(editor).toHaveText('SELECT O."ORDER_ID" FROM "SANDBOX"."BILLIE_TESTING"."ORDERS" O');
  expect(inspectionSchemaCalls).toBeGreaterThanOrEqual(1);

  // 3. Guarded ROLE / WAREHOUSE / DATABASE / SCHEMA context switching
  await page.getByRole("button", { name: "Load Snowflake session contexts", exact: true }).click();
  await expect(page.getByLabel("Active role", { exact: true })).toHaveValue("ANALYST");
  await expect(page.getByLabel("Active warehouse", { exact: true })).toHaveValue("WH_SMALL");
  await expect(page.getByLabel("Active database", { exact: true })).toHaveValue("SANDBOX");
  await expect(page.getByLabel("Active schema", { exact: true })).toHaveValue("BILLIE_TESTING");

  await page.getByLabel("Active role", { exact: true }).selectOption("DATA_ENG");
  await expect(page.getByLabel("Active role", { exact: true })).toHaveValue("DATA_ENG");
  await page.getByLabel("Active warehouse", { exact: true }).selectOption("WH_LARGE");
  await expect(page.getByLabel("Active warehouse", { exact: true })).toHaveValue("WH_LARGE");
  await page.getByLabel("Active database", { exact: true }).selectOption("ANALYTICS");
  await expect(page.getByLabel("Active database", { exact: true })).toHaveValue("ANALYTICS");
  await expect(page.getByLabel("Active schema", { exact: true })).toHaveValue("CORE");
  await page.getByLabel("Active schema", { exact: true }).selectOption("MART");
  await expect(page.getByLabel("Active schema", { exact: true })).toHaveValue("MART");

  expect(switchCalls).toHaveLength(4);
  expect(errors).toEqual([]);
});

test("Snowflake RelationBrowser supports slice paging/sort/filter, cancel recovery, multi-batch CSV import, Object Inspector, and broader DDL", async ({
  page,
}) => {
  const sessionId = "sf-grid-ddl-session";
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));

  let profile!: ConnectionProfile;
  const previewRequests: unknown[] = [];
  let cancelCalls = 0;
  let gridRuns = 0;
  let finishCalls = 0;
  const ddlPreflightActions: unknown[] = [];

  const runtimeState = (): SessionRuntimeState => ({
    sessionId,
    database: "SANDBOX",
    role: "ANALYST",
    warehouse: "WH_SMALL",
    searchPath: ["SANDBOX.BILLIE_TESTING"],
    contextKind: "schema",
    activeContext: "BILLIE_TESTING",
    contextFingerprint: "fp-grid",
    transactionMode: "autoCommit",
    transactionState: "idle",
    transactionObservation: {
      status: "observed",
      transactionId: null,
      abortOnError: false,
      usability: "unknown",
    },
    transactionControlAuthority: {
      status: "observed",
      revision: "tx-1",
      allowedActions: ["begin"],
    },
  });

  const catalog: BrowseCatalog = {
    revision: "cat-1",
    database: "SANDBOX",
    identity: {
      database: "SANDBOX",
      role: "ANALYST",
      searchPath: ["SANDBOX.BILLIE_TESTING"],
      contextFingerprint: "fp-grid",
      caseSensitiveTableNames: true,
    },
    relations: [
      {
        targetId: "orders-target",
        database: "SANDBOX",
        table: { schema: "BILLIE_TESTING", name: "ORDERS" },
        tableType: "BASE TABLE",
        traits: {
          transient: false,
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

  const columns: ColumnInfo[] = [
    {
      id: "c0",
      name: "ID",
      databaseType: "NUMBER(38,0)",
      kind: "integer",
      nullable: false,
      primaryKey: false,
      foreignKey: false,
      editable: false,
      insertable: false,
      hasDefault: false,
    },
    {
      id: "c1",
      name: "PAYLOAD",
      databaseType: "VARIANT",
      kind: "json",
      nullable: true,
      primaryKey: false,
      foreignKey: false,
      editable: false,
      insertable: false,
      hasDefault: false,
    },
  ];

  const inspection: RelationInspection = {
    revision: "insp-1",
    targetId: "orders-target",
    database: "SANDBOX",
    table: { schema: "BILLIE_TESTING", name: "ORDERS" },
    constraints: "notFetched",
    columns: [
      {
        name: "ID",
        ordinal: 1,
        fullType: "NUMBER(38,0)",
        dataType: "NUMBER",
        dataTypeAlias: null,
        kind: "column",
        resultKind: "integer",
        nullable: false,
        defaultExpression: null,
        descriptorDefaultExpression: null,
        writeDefaultExpression: null,
        writeDefaultStatus: "notReported",
        comment: "Order key",
        characterMaximumLength: null,
        characterOctetLength: null,
        numericPrecision: "38",
        numericPrecisionRadix: "10",
        numericScale: "0",
        identity: {
          isIdentity: false,
          generation: null,
          start: null,
          increment: null,
          ordered: null,
        },
        expression: null,
      },
      {
        name: "PAYLOAD",
        ordinal: 2,
        fullType: "VARIANT",
        dataType: "VARIANT",
        dataTypeAlias: null,
        kind: "column",
        resultKind: "json",
        nullable: true,
        defaultExpression: null,
        descriptorDefaultExpression: null,
        writeDefaultExpression: null,
        writeDefaultStatus: "notReported",
        comment: "JSON payload",
        characterMaximumLength: null,
        characterOctetLength: null,
        numericPrecision: null,
        numericPrecisionRadix: null,
        numericScale: null,
        identity: {
          isIdentity: false,
          generation: null,
          start: null,
          increment: null,
          ordered: null,
        },
        expression: null,
      },
    ],
  };

  let snapSeq = 0;
  const makePreview = (truncated = false): RelationPreview => ({
    columns,
    rows: [
      ["1", '{"status":"new"}'],
      ["2", '{"status":"paid"}'],
    ],
    truncated,
    editSnapshotId: `snap-${++snapSeq}`,
  });

  const objectDetails: ObjectDetails = {
    object: {
      schema: "BILLIE_TESTING",
      name: "ORDERS",
      kind: "table",
      identity: null,
    },
    ownerTable: null,
    comment: "Order ledger",
    ddl: {
      status: "available",
      ddl: 'CREATE TABLE "SANDBOX"."BILLIE_TESTING"."ORDERS" ("ID" NUMBER(38,0), "PAYLOAD" VARIANT);',
      truncated: false,
    },
    indexes: [],
    constraints: [
      {
        name: "ORDERS_CUSTOMER_FK",
        kind: "foreignKey",
        definition: "FOREIGN KEY (ID) REFERENCES BILLIE_TESTING.CUSTOMERS (ID) NOT ENFORCED",
        columns: ["ID"],
        target: { schema: "BILLIE_TESTING", name: "CUSTOMERS" },
        targetColumns: ["ID"],
      },
    ],
    triggers: [],
    uses: [
      {
        object: {
          schema: "BILLIE_TESTING",
          name: "CUSTOMERS",
          kind: "table",
          identity: null,
        },
        relation: "foreign key",
      },
    ],
    usedBy: [],
    routine: null,
    sequence: null,
    trigger: null,
    storage: {
      totalBytes: 8192,
      tableBytes: 8192,
      indexBytes: null,
      toastBytes: null,
      estimatedRows: 2,
      liveTuples: null,
      deadTuples: null,
      deadRatio: null,
      statsStale: false,
      sequentialScans: null,
      indexScans: null,
      lastVacuum: null,
      lastAutoVacuum: null,
      lastAnalyze: null,
      lastAutoAnalyze: null,
    },
  };

  await page.route(`${rpc}*`, async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const args = route.request().postDataJSON();
    let result: unknown;
    switch (command) {
      case "save_profile":
        profile = args.profile;
        result = profile;
        break;
      case "prepare_connection":
        result = "sf-grid-attempt";
        break;
      case "connect_prepared":
        result = {
          id: sessionId,
          connectionId: profile.id,
          engine: "snowflake",
          serverVersion: "synthetic-8.42",
          latencyMs: 1,
          encrypted: true,
          capabilities: {
            sql: true,
            transactions: false,
            explicitTransactionControls: true,
            cancellation: true,
            tableBrowsing: false,
            relationBrowsing: true,
            objectExplorer: true,
            tableEditing: false,
            tableFiltering: false,
            processes: false,
            snowflakeReviewedDml: true,
            snowflakeReviewedDdl: true,
            snowflakeGridEditing: true,
            ddlEditing: false,
          },
        };
        break;
      case "session_contexts":
        result = { state: runtimeState(), available: ["BILLIE_TESTING"], revision: "ctx-1" };
        break;
      case "completion_catalog":
        result = {
          revision: "comp-1",
          identity: catalog.identity,
          tables: [],
          unqualifiedTables: [],
        };
        break;
      case "relation_catalog":
        result = catalog;
        break;
      case "inspect_relation":
        result = inspection;
        break;
      case "preview_relation":
        previewRequests.push(args.request);
        result = makePreview(true);
        break;
      case "prepare_snowflake_grid_plan": {
        const planId = `grid-plan-${++gridRuns}`;
        const sql = `INSERT INTO "SANDBOX"."BILLIE_TESTING"."ORDERS" ("ID", "PAYLOAD") SELECT CAST(? AS NUMBER(38, 0)), PARSE_JSON(?)`;
        const preview: ChangePreview = {
          plan: {
            id: planId,
            sessionId,
            reviewedSnowflakeGrid: true,
            statements: [
              {
                sql,
                kind: "write",
                missingWhere: false,
              },
            ],
            requiresApproval: true,
            requiresPassphrase: false,
            confirmationText: "Snowflake Grid",
            warnings: [],
            expiresAt: Date.now() + 60000,
          },
          rowCount: args.request.mutations.length,
          sql: [sql],
          bindings: [["1", '{"status":"new"}']],
        };
        result = preview;
        break;
      }
      case "cancel_snowflake_grid_plan":
        cancelCalls += 1;
        result = `snap-restored-${cancelCalls}`;
        break;
      case "authorize_plan":
        result = `approval-${gridRuns}`;
        break;
      case "execute_snowflake_grid_plan": {
        const mutationResult: SnowflakeGridResult = {
          sessionId,
          operationId: `op-${gridRuns}`,
          outcome: "pending",
          transactionId: "tx-manual-csv",
          recordingError: null,
          metadataRefreshRequired: false,
          retired: false,
        };
        result = mutationResult;
        break;
      }
      case "preview_snowflake_grid_transaction_relation":
        result = makePreview(false);
        break;
      case "finish_snowflake_grid_transaction": {
        finishCalls += 1;
        const finishResult: SnowflakeGridResult = {
          sessionId,
          operationId: "op-finish",
          outcome: "committed",
          transactionId: null,
          recordingError: null,
          metadataRefreshRequired: true,
          retired: false,
        };
        result = finishResult;
        break;
      }
      case "object_details":
        result = objectDetails;
        break;
      case "preflight_snowflake_ddl": {
        ddlPreflightActions.push(args.request.action);
        const action = args.request.action;
        const sql =
          action.kind === "renameColumn"
            ? 'ALTER TABLE "SANDBOX"."BILLIE_TESTING"."ORDERS" RENAME COLUMN "PAYLOAD" TO "EVENT_JSON"'
            : 'CREATE VIEW "SANDBOX"."BILLIE_TESTING"."ACTIVE_ORDERS_V" AS SELECT "ID" FROM "SANDBOX"."BILLIE_TESTING"."ORDERS"';
        result = {
          revision: "ddl-rev-1",
          kind: action.kind,
          database: "SANDBOX",
          schema: "BILLIE_TESTING",
          name: action.kind === "createView" ? "ACTIVE_ORDERS_V" : "ORDERS",
          qualifiedTarget:
            action.kind === "createView"
              ? '"SANDBOX"."BILLIE_TESTING"."ACTIVE_ORDERS_V"'
              : '"SANDBOX"."BILLIE_TESTING"."ORDERS"',
          sql,
          warnings: ["DDL cannot be rolled back."],
        };
        break;
      }
      case "prepare_snowflake_ddl": {
        const sql = args.request.sql as string;
        const ddlPlan: PreparedPlan = {
          id: "ddl-plan-1",
          sessionId,
          reviewedSnowflakeDdl: true,
          statements: [{ sql, kind: "ddl", missingWhere: false }],
          requiresApproval: true,
          requiresPassphrase: false,
          confirmationText: "Snowflake Grid",
          warnings: ["DDL cannot be rolled back."],
          expiresAt: Date.now() + 60000,
        };
        result = ddlPlan;
        break;
      }
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
  await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Grid");
  await page
    .getByLabel("Snowflake account hostname")
    .fill("fixture.invalid.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("SANDBOX");
  await page.getByLabel("Username", { exact: true }).fill("ANALYST");
  await page.getByLabel("Role", { exact: true }).fill("ANALYST");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH_SMALL");
  await page.getByLabel("Schema", { exact: true }).fill("BILLIE_TESTING");
  await page.getByLabel("Safe Mode").selectOption("guard");
  await page.getByRole("checkbox", { name: /Allow separately reviewed single INSERT/ }).check();
  await page.getByRole("checkbox", { name: /Allow separately reviewed CREATE TABLE/ }).check();
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();

  await page.getByRole("button", { name: "Browse relations", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Browse relations" });
  await dialog.getByLabel("Relation", { exact: true }).selectOption("orders-target");
  await dialog.getByRole("button", { name: "Inspect relation", exact: true }).click();
  await expect(dialog).toContainText("VARIANT");

  // 1. Server sort, filter, and 500-row offset pagination
  await dialog.getByLabel("Preview sort column").selectOption("ID");
  await dialog.getByLabel("Preview sort direction").selectOption("desc");
  await dialog.getByRole("button", { name: "Add filter", exact: true }).click();
  await dialog.getByLabel("Filter 1 column").selectOption("PAYLOAD");
  await dialog.getByLabel("Filter 1 operator").selectOption("contains");
  await dialog.getByLabel("Filter 1 value").fill("paid");
  await dialog.getByRole("button", { name: "Apply sort & filters", exact: true }).click();
  await expect(dialog).toContainText("Sorted by ID DESC · 1 filter active");

  await dialog.getByRole("button", { name: "Next server page", exact: true }).click();
  await expect(dialog).toContainText("server offset 500");
  expect(previewRequests.at(-1)).toMatchObject({
    offset: 500,
    sort: { column: "ID", direction: "desc" },
    filters: [{ column: "PAYLOAD", operator: "contains", value: "paid" }],
  });

  // 2. Inline JSON cell edit + ReviewDialog cancel recovery via cancel_snowflake_grid_plan
  await dialog.getByLabel("Row 1 PAYLOAD", { exact: true }).fill('{"status":"cancelled"}');
  await dialog.getByRole("button", { name: "Review 1 change…", exact: true }).click();
  const gridReviewDialog = page.getByRole("dialog", { name: "Review pending changes" });
  await expect(gridReviewDialog).toBeVisible();
  await gridReviewDialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect.poll(() => cancelCalls).toBe(1);
  // Staged override remains editable with a fresh snapshot!
  await expect(dialog.getByLabel("Row 1 PAYLOAD", { exact: true })).toHaveValue(
    '{"status":"cancelled"}',
  );
  await expect(dialog.getByRole("button", { name: "Review 1 change…", exact: true })).toBeEnabled();
  await dialog.getByRole("button", { name: "Discard staged edits", exact: true }).click();

  // 3. Multi-batch CSV import (>500 rows -> 2 reviewed batches in one manual transaction)
  const csvLines = ["ID,PAYLOAD"];
  for (let i = 1; i <= 502; i++) {
    csvLines.push(`${1000 + i},"{""seq"":${i}}"`);
  }
  await dialog.getByRole("button", { name: "Import CSV rows…", exact: true }).click();
  await dialog.getByLabel("CSV rows to stage").fill(csvLines.join("\n"));
  await dialog.getByRole("button", { name: "Stage CSV rows", exact: true }).click();
  await expect(dialog).toContainText(
    "Batch 1 of 2 staged (500 rows in this batch · 502 total CSV rows",
  );

  await dialog.getByRole("button", { name: "Review batch 1/2 (500 rows)…", exact: true }).click();
  await gridReviewDialog
    .getByLabel("Type Snowflake Grid to confirm", { exact: true })
    .fill("Snowflake Grid");
  await gridReviewDialog.getByRole("button", { name: "Apply changes", exact: true }).click();

  await expect(dialog).toContainText(
    "Batch 2 of 2 staged (2 rows in this batch · 502 total CSV rows",
  );
  await dialog.getByRole("button", { name: "Review batch 2/2 (2 rows)…", exact: true }).click();
  await gridReviewDialog
    .getByLabel("Type Snowflake Grid to confirm", { exact: true })
    .fill("Snowflake Grid");
  await gridReviewDialog.getByRole("button", { name: "Apply changes", exact: true }).click();

  await expect(
    dialog.getByRole("region", { name: "Pending Snowflake grid transaction" }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Commit transaction", exact: true }).click();
  await expect.poll(() => finishCalls).toBe(1);
  await expect(dialog).toHaveCount(0);
  await page.getByRole("button", { name: "Refresh completion context", exact: true }).click();
  await expect(page.getByRole("button", { name: "Browse relations", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Browse relations", exact: true }).click();
  await expect(dialog.getByRole("status").first()).toContainText("1 visible relations · SANDBOX");
  await dialog.getByLabel("Relation", { exact: true }).selectOption("orders-target");
  await dialog.getByRole("button", { name: "Inspect relation", exact: true }).click();
  await expect(dialog).toContainText("VARIANT");

  // 4. Expanded Snowflake DDL preflight & prepare (Rename column…)
  await dialog.getByRole("button", { name: "Rename column…", exact: true }).click();
  const ddlDialog = page.getByRole("dialog", { name: "Rename Snowflake column" });
  await ddlDialog.getByLabel("Target column", { exact: true }).selectOption("PAYLOAD");
  await ddlDialog.getByLabel("New column name", { exact: true }).fill("EVENT_JSON");
  await ddlDialog.getByRole("button", { name: "Generate & review SQL", exact: true }).click();
  const ddlReviewDialog = page.getByRole("dialog", { name: "Review this operation" });
  await expect(ddlReviewDialog.locator(".sql-preview")).toHaveText(
    'ALTER TABLE "SANDBOX"."BILLIE_TESTING"."ORDERS" RENAME COLUMN "PAYLOAD" TO "EVENT_JSON"',
  );
  await ddlReviewDialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await ddlDialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(ddlPreflightActions).toEqual([
    expect.objectContaining({
      kind: "renameColumn",
      columnName: "PAYLOAD",
      newName: "EVENT_JSON",
    }),
  ]);

  // 5. Inspect object -> ObjectInspectorView (Object constraints, dependencies, DDL, and Browse relations)
  await page.getByRole("button", { name: "Browse relations", exact: true }).click();
  await dialog.getByLabel("Relation", { exact: true }).selectOption("orders-target");
  await dialog.getByRole("button", { name: "Inspect object", exact: true }).click();
  await expect(dialog).toHaveCount(0);

  const constraintsSection = page.getByRole("region", { name: "Object constraints" });
  await expect(constraintsSection).toContainText("ORDERS_CUSTOMER_FK");
  await expect(constraintsSection).toContainText("BILLIE_TESTING.CUSTOMERS");
  await expect(page.getByRole("region", { name: "Object dependencies" })).toContainText(
    "BILLIE_TESTING.CUSTOMERS",
  );
  await expect(page.getByLabel("Object DDL")).toContainText(
    'CREATE TABLE "SANDBOX"."BILLIE_TESTING"."ORDERS"',
  );
  await page.getByRole("button", { name: "Browse relations", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Browse relations" })).toBeVisible();

  expect(errors).toEqual([]);
});

test("Snowflake supports schema diagrams, schema diff with Snowflake migration DDL, and rich VARIANT/BINARY/GEOGRAPHY cell inspection", async ({
  page,
}) => {
  const sessionId = "sf-diagram-diff-session";
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));

  let profile!: ConnectionProfile;
  const runtimeState = (): SessionRuntimeState => ({
    sessionId,
    database: "SANDBOX",
    role: "ANALYST",
    warehouse: "WH_SMALL",
    searchPath: ["SANDBOX.BILLIE_TESTING"],
    contextKind: "schema",
    activeContext: "BILLIE_TESTING",
    contextFingerprint: "fp-sf-diagram-diff",
    transactionMode: "autoCommit",
    transactionState: "idle",
    transactionObservation: {
      status: "observed",
      transactionId: null,
      abortOnError: false,
      usability: "unknown",
    },
    transactionControlAuthority: {
      status: "observed",
      revision: "tx-dd-1",
      allowedActions: ["begin"],
    },
  });

  const explorerObjects: DatabaseObjectSummary[] = [
    {
      object: { schema: "BILLIE_TESTING", name: "ORDERS", kind: "table", identity: null },
      ownerTable: null,
      signature: null,
      estimatedRows: 25,
      totalBytes: 4096,
    },
    {
      object: { schema: "STAGING", name: "ORDERS", kind: "table", identity: null },
      ownerTable: null,
      signature: null,
      estimatedRows: 10,
      totalBytes: 2048,
    },
  ];

  const diagram: SchemaDiagram = {
    schema: "BILLIE_TESTING",
    tables: [
      {
        table: { schema: "BILLIE_TESTING", name: "CUSTOMERS" },
        kind: "BASE TABLE",
        columns: [{ name: "ID", databaseType: "NUMBER(38, 0)", nullable: false, primaryKey: true }],
      },
      {
        table: { schema: "BILLIE_TESTING", name: "ORDERS" },
        kind: "BASE TABLE",
        columns: [
          { name: "ID", databaseType: "NUMBER(38, 0)", nullable: false, primaryKey: true },
          {
            name: "CUSTOMER_ID",
            databaseType: "NUMBER(38, 0)",
            nullable: false,
            primaryKey: false,
          },
          { name: "PAYLOAD", databaseType: "VARIANT", nullable: true, primaryKey: false },
        ],
      },
    ],
    foreignKeys: [
      {
        name: "ORDERS_CUSTOMER_FK",
        source: { schema: "BILLIE_TESTING", name: "ORDERS" },
        columns: ["CUSTOMER_ID"],
        target: { schema: "BILLIE_TESTING", name: "CUSTOMERS" },
        targetColumns: ["ID"],
      },
    ],
    truncated: false,
    restricted: false,
  };

  const makeSnapshot = (schema: string): SchemaSnapshot =>
    schema === "STAGING"
      ? {
          engine: "snowflake",
          serverVersion: "synthetic-8.42",
          database: "SANDBOX",
          schema: "STAGING",
          role: "ANALYST",
          tables: [
            {
              name: "ORDERS",
              kind: "table",
              comment: null,
              partitionKey: null,
              viewDefinition: null,
              columns: [
                {
                  name: "ID",
                  dataType: "NUMBER(38, 0)",
                  nullable: false,
                  defaultExpression: null,
                  identity: null,
                  generated: null,
                  ordinalPosition: 1,
                },
              ],
              constraints: [],
              indexes: [],
              triggers: [],
            },
          ],
          routines: [],
          sequences: [],
          unsupportedObjects: [],
          truncated: false,
          restricted: false,
        }
      : {
          engine: "snowflake",
          serverVersion: "synthetic-8.42",
          database: "SANDBOX",
          schema: "BILLIE_TESTING",
          role: "ANALYST",
          tables: [
            {
              name: "ORDERS",
              kind: "table",
              comment: "Order ledger",
              partitionKey: "ID",
              viewDefinition: null,
              columns: [
                {
                  name: "ID",
                  dataType: "NUMBER(38, 0)",
                  nullable: false,
                  defaultExpression: null,
                  identity: null,
                  generated: null,
                  ordinalPosition: 1,
                },
                {
                  name: "PAYLOAD",
                  dataType: "VARIANT",
                  nullable: true,
                  defaultExpression: null,
                  identity: null,
                  generated: null,
                  ordinalPosition: 2,
                },
              ],
              constraints: [],
              indexes: [],
              triggers: [],
            },
          ],
          routines: [],
          sequences: [
            {
              name: "ORDER_SEQ",
              dataType: "NUMBER",
              startValue: "1",
              minValue: "1",
              maxValue: "999999",
              incrementBy: "1",
              cacheSize: "1",
              cycle: false,
            },
          ],
          unsupportedObjects: [
            {
              name: "LIVE_EVENTS",
              objectType: "DYNAMIC TABLE",
              reason: "Snowflake dynamic tables cannot be migrated via standard CREATE TABLE DDL",
            },
          ],
          truncated: false,
          restricted: false,
        };

  const ordersDetails: ObjectDetails = {
    object: { schema: "BILLIE_TESTING", name: "ORDERS", kind: "table", identity: null },
    ownerTable: null,
    comment: "Order ledger",
    ddl: {
      status: "available",
      ddl: 'CREATE TABLE "SANDBOX"."BILLIE_TESTING"."ORDERS" ("ID" NUMBER(38,0), "PAYLOAD" VARIANT);',
      truncated: false,
    },
    indexes: [],
    constraints: [],
    triggers: [],
    uses: [],
    usedBy: [],
    routine: null,
    sequence: null,
    trigger: null,
    storage: null,
  };

  const browseCatalog: BrowseCatalog = {
    revision: "cat-dd-1",
    database: "SANDBOX",
    relations: [
      {
        targetId: "orders-rich-target",
        database: "SANDBOX",
        table: { schema: "BILLIE_TESTING", name: "ORDERS" },
        tableType: "BASE TABLE",
        traits: {
          transient: false,
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

  const richInspection: RelationInspection = {
    revision: "insp-dd-1",
    targetId: "orders-rich-target",
    database: "SANDBOX",
    table: { schema: "BILLIE_TESTING", name: "ORDERS" },
    constraints: "notFetched",
    columns: [
      {
        name: "PAYLOAD",
        ordinal: 1,
        fullType: "VARIANT",
        dataType: "VARIANT",
        dataTypeAlias: null,
        kind: "column",
        resultKind: "json",
        nullable: true,
        defaultExpression: null,
        descriptorDefaultExpression: null,
        writeDefaultExpression: null,
        writeDefaultStatus: "notReported",
        comment: null,
        characterMaximumLength: null,
        characterOctetLength: null,
        numericPrecision: null,
        numericPrecisionRadix: null,
        numericScale: null,
        identity: {
          isIdentity: false,
          generation: null,
          start: null,
          increment: null,
          ordered: null,
        },
        expression: null,
      },
      {
        name: "RAW_HASH",
        ordinal: 2,
        fullType: "BINARY(7)",
        dataType: "BINARY",
        dataTypeAlias: null,
        kind: "column",
        resultKind: "binary",
        nullable: true,
        defaultExpression: null,
        descriptorDefaultExpression: null,
        writeDefaultExpression: null,
        writeDefaultStatus: "notReported",
        comment: null,
        characterMaximumLength: null,
        characterOctetLength: null,
        numericPrecision: null,
        numericPrecisionRadix: null,
        numericScale: null,
        identity: {
          isIdentity: false,
          generation: null,
          start: null,
          increment: null,
          ordered: null,
        },
        expression: null,
      },
      {
        name: "REGION_GEO",
        ordinal: 3,
        fullType: "GEOGRAPHY",
        dataType: "GEOGRAPHY",
        dataTypeAlias: null,
        kind: "column",
        resultKind: "text",
        nullable: true,
        defaultExpression: null,
        descriptorDefaultExpression: null,
        writeDefaultExpression: null,
        writeDefaultStatus: "notReported",
        comment: null,
        characterMaximumLength: null,
        characterOctetLength: null,
        numericPrecision: null,
        numericPrecisionRadix: null,
        numericScale: null,
        identity: {
          isIdentity: false,
          generation: null,
          start: null,
          increment: null,
          ordered: null,
        },
        expression: null,
      },
    ],
  };

  const richPreview: RelationPreview = {
    columns: [
      {
        id: "c0",
        name: "PAYLOAD",
        databaseType: "VARIANT",
        kind: "json",
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: false,
        insertable: false,
        hasDefault: false,
      },
      {
        id: "c1",
        name: "RAW_HASH",
        databaseType: "BINARY(7)",
        kind: "binary",
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: false,
        insertable: false,
        hasDefault: false,
      },
      {
        id: "c2",
        name: "REGION_GEO",
        databaseType: "TEXT",
        kind: "text",
        nullable: true,
        primaryKey: false,
        foreignKey: false,
        editable: false,
        insertable: false,
        hasDefault: false,
      },
    ],
    rows: [
      [
        '{"customer":{"orders":[{"id":777}]}}',
        "4A6F696E657279",
        '{"type":"Polygon","coordinates":[[[-122.5,37.7],[-122.3,37.7],[-122.3,37.8],[-122.5,37.8],[-122.5,37.7]]]}',
      ],
    ],
    truncated: false,
    editSnapshotId: "snap-dd-1",
  };

  await page.route(`${rpc}*`, async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const args = route.request().postDataJSON();
    let result: unknown;
    switch (command) {
      case "save_profile":
        profile = args.profile;
        result = profile;
        break;
      case "prepare_connection":
        result = "sf-dd-attempt";
        break;
      case "connect_prepared":
        result = {
          id: sessionId,
          connectionId: profile.id,
          engine: "snowflake",
          serverVersion: "synthetic-8.42",
          latencyMs: 1,
          encrypted: true,
          capabilities: {
            sql: true,
            transactions: false,
            explicitTransactionControls: true,
            cancellation: true,
            tableBrowsing: false,
            relationBrowsing: true,
            objectExplorer: true,
            schemaDiagram: true,
            schemaDiff: true,
            ddlEditing: false,
            tableEditing: false,
            tableFiltering: false,
            processes: false,
            snowflakeReviewedDml: true,
            snowflakeReviewedDdl: true,
            snowflakeGridEditing: true,
          },
        };
        break;
      case "session_contexts":
        result = {
          state: runtimeState(),
          available: ["BILLIE_TESTING", "STAGING"],
          revision: "schema-dd-1",
        };
        break;
      case "snowflake_session_contexts":
        result = {
          state: runtimeState(),
          revision: "ctx-dd-1",
          availableRoles: ["ANALYST"],
          availableWarehouses: ["WH_SMALL"],
          availableDatabases: ["SANDBOX"],
          availableSchemas: ["BILLIE_TESTING", "STAGING"],
        };
        break;
      case "completion_catalog":
        result = {
          revision: "comp-dd-1",
          identity: {
            database: "SANDBOX",
            role: "ANALYST",
            searchPath: ["SANDBOX.BILLIE_TESTING"],
            contextFingerprint: "fp-sf-diagram-diff",
            caseSensitiveTableNames: true,
          },
          tables: [],
          unqualifiedTables: [],
        };
        break;
      case "database_objects":
        result = explorerObjects;
        break;
      case "schema_diagram":
        result = diagram;
        break;
      case "schema_snapshot":
        result = makeSnapshot(args.schema);
        break;
      case "object_details":
        result = ordersDetails;
        break;
      case "relation_catalog":
        result = browseCatalog;
        break;
      case "inspect_relation":
        result = richInspection;
        break;
      case "preview_relation":
        result = richPreview;
        break;
      case "health":
        result = 1;
        break;
      default:
        // Forward compare_schemas, generate_migration_ddl, and generate_schema_ddl to the real Rust test-harness!
        await route.continue();
        return;
    }
    await route.fulfill({ json: { result } });
  });

  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Database engine").selectOption("snowflake");
  await page.getByLabel("Snowflake authentication").selectOption("externalBrowser");
  await page.getByLabel("Connection name", { exact: true }).fill("Snowflake Architect");
  await page
    .getByLabel("Snowflake account hostname")
    .fill("fixture.invalid.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("SANDBOX");
  await page.getByLabel("Username", { exact: true }).fill("ANALYST");
  await page.getByLabel("Role", { exact: true }).fill("ANALYST");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH_SMALL");
  await page.getByLabel("Schema", { exact: true }).fill("BILLIE_TESTING");
  await page.getByLabel("Safe Mode").selectOption("guard");
  await page.getByRole("checkbox", { name: /Allow separately reviewed single INSERT/ }).check();
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // Load Snowflake objects in sidebar to expose schema actions
  await page.getByRole("button", { name: "Refresh schema", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Open schema diagram for BILLIE_TESTING", exact: true }),
  ).toBeVisible();

  // 1. Snowflake Schema Diagram + click node -> ObjectInspectorView
  await page
    .getByRole("button", { name: "Open schema diagram for BILLIE_TESTING", exact: true })
    .click();
  await expect(page.locator(".diagram-view")).toContainText("2 relations · 1 foreign keys");
  await page.getByRole("button", { name: "ORDERS, 3 columns", exact: true }).click();
  await expect(page.getByLabel("Object DDL")).toContainText(
    'CREATE TABLE "SANDBOX"."BILLIE_TESTING"."ORDERS"',
  );

  // 2. Snowflake Schema Diff + real Rust generate_migration_ddl + Open in SQL editor
  await page.getByRole("button", { name: "Compare schema STAGING", exact: true }).click();
  await expect(page.getByText(/Unsupported objects disclosed \(1\)/)).toBeVisible();
  await expect(page.getByText(/LIVE_EVENTS/)).toBeVisible();
  await page.getByRole("button", { name: "Migrate Left → Right DDL", exact: true }).click();
  const ddlPreview = page.getByTestId("schema-ddl-preview");
  await expect(ddlPreview).toContainText(
    'ALTER TABLE "STAGING"."ORDERS" ADD COLUMN "PAYLOAD" VARIANT',
  );
  await expect(ddlPreview).toContainText('ALTER TABLE "STAGING"."ORDERS" CLUSTER BY (ID)');
  await expect(ddlPreview).toContainText('CREATE SEQUENCE "STAGING"."ORDER_SEQ"');
  await expect(
    ddlPreview.getByRole("button", { name: "Review and apply migration" }),
  ).toBeDisabled();
  await ddlPreview.getByRole("button", { name: "Open generated DDL in SQL editor" }).click();
  await expect(page.getByRole("textbox", { name: "SQL editor" })).toContainText(
    'ALTER TABLE "STAGING"."ORDERS" CLUSTER BY (ID)',
  );

  // 3. Rich Cell Value Inspector inside RelationBrowser (VARIANT :path tree, BINARY hex, GEOGRAPHY SVG)
  await page.getByRole("button", { name: "Browse relations", exact: true }).click();
  const browseDialog = page.getByRole("dialog", { name: "Browse relations" });
  await browseDialog.getByLabel("Relation", { exact: true }).selectOption("orders-rich-target");
  await browseDialog.getByRole("button", { name: "Inspect relation", exact: true }).click();
  await browseDialog.getByRole("button", { name: "Preview rows", exact: true }).click();
  await expect(
    browseDialog.getByRole("table", { name: "Editable Snowflake preview" }),
  ).toBeVisible();

  await browseDialog.getByRole("button", { name: "Inspect row 1 PAYLOAD", exact: true }).click();
  await expect(page.getByTestId("json-tree-viewer")).toContainText("PAYLOAD:customer.orders[0].id");
  await page.getByRole("button", { name: "Format JSON", exact: true }).click();
  await page.getByRole("button", { name: "Apply to staged cell", exact: true }).click();
  await expect(
    browseDialog.getByRole("button", { name: "Review 1 change…", exact: true }),
  ).toBeEnabled();

  await browseDialog.getByRole("button", { name: "Inspect row 1 RAW_HASH", exact: true }).click();
  await expect(page.getByTestId("binary-hex-viewer")).toContainText("|Joinery|");
  await page.getByRole("button", { name: "Back to preview", exact: true }).click();

  await browseDialog.getByRole("button", { name: "Inspect row 1 REGION_GEO", exact: true }).click();
  await expect(page.getByTestId("spatial-viewer")).toContainText("Polygon");
  await expect(page.getByTestId("spatial-svg-preview")).toBeVisible();
  await page.getByRole("button", { name: "Close", exact: true }).click();

  expect(errors).toEqual([]);
});
