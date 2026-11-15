import { expect, test } from "./fixtures";
import type {
  BrowseCatalog,
  ColumnInfo,
  ConnectionProfile,
  RelationCheckConstraints,
  RelationConstraintMappings,
  RelationConstraints,
  RelationDefinition,
  RelationInspection,
  RelationPreview,
} from "../../src/ipc/generated";

// Synthetic RPCs exercise the real UI only; no Snowflake SSO/account SQL occurs.
test("SDK console views key mappings and DDL, rejects stale results, inspects and previews qualified targets", async ({
  page,
}) => {
  const sessionId = "synthetic-browse-session";
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  let profile!: ConnectionProfile;
  let failRefresh = false;
  let definitionMode: "available" | "denied" | "held" = "available";
  let releaseDefinition: (() => void) | undefined;
  let constraintsMode: "available" | "empty" | "denied" | "held" = "available";
  let releaseConstraints: (() => void) | undefined;
  let mappingsMode: "available" | "empty" | "denied" | "held" | "unsupported" = "available";
  let releaseMappings: (() => void) | undefined;
  let checksMode: "available" | "empty" | "denied" | "held" = "available";
  let releaseChecks: (() => void) | undefined;
  let dynamicTarget = false;
  const catalog: BrowseCatalog = {
    revision: "catalog-r1",
    database: "DB",
    identity: { database: "DB", role: "READER", searchPath: ["S"], caseSensitiveTableNames: true },
    relations: [
      {
        targetId: "opaque-r1-target",
        database: "DB",
        table: { schema: "S", name: 'Quoted "table' },
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
      {
        targetId: "shadowed-target",
        database: "DB",
        table: { schema: "S", name: "COLLISION" },
        tableType: "BASE TABLE",
        traits: {
          transient: false,
          temporary: false,
          iceberg: false,
          dynamic: false,
          hybrid: false,
          immutable: false,
        },
        availability: "shadowed",
      },
    ],
  };
  const referencedTarget = {
    ...catalog.relations[0],
    targetId: "opaque-qualified-reference",
    table: { schema: "P.dot", name: 'Quoted "parent' },
  };
  catalog.relations.push(referencedTarget, {
    ...referencedTarget,
    targetId: "opaque-same-name-decoy",
    table: { ...referencedTarget.table, schema: "S" },
  });
  const column: ColumnInfo = {
    id: "c0",
    name: "EXACT",
    databaseType: "NUMBER(38,0)",
    kind: "integer",
    nullable: true,
    primaryKey: false,
    foreignKey: false,
    editable: false,
    insertable: false,
    hasDefault: false,
  };
  const inspection: RelationInspection = {
    revision: "inspection-r1",
    targetId: "opaque-r1-target",
    database: "DB",
    table: catalog.relations[0].table,
    constraints: "notFetched",
    columns: [
      {
        name: "EXACT",
        ordinal: 3,
        fullType: "NUMBER(38,0)",
        dataType: "NUMBER",
        dataTypeAlias: null,
        kind: "column",
        resultKind: "integer",
        nullable: true,
        defaultExpression: null,
        descriptorDefaultExpression: null,
        writeDefaultExpression: null,
        writeDefaultStatus: "notReported",
        comment: "Synthetic comment",
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
    ],
  };
  const preview: RelationPreview = {
    columns: [column],
    rows: [
      ["9007199254740993"],
      [null],
      [""],
      ...Array.from({ length: 497 }, (_, i) => [String(i + 3)]),
    ],
    truncated: true,
  };
  const definition: RelationDefinition = {
    revision: catalog.revision,
    targetId: catalog.relations[0].targetId,
    database: catalog.database,
    table: catalog.relations[0].table,
    ddl: 'CREATE OR REPLACE TABLE "DB"."S"."Quoted ""table" ("EXACT" NUMBER(38,0));',
  };
  const checkConstraints: RelationCheckConstraints = {
    revision: catalog.revision,
    targetId: catalog.relations[0].targetId,
    database: catalog.database,
    table: catalog.relations[0].table,
    constraints: [
      {
        catalog: "DB",
        schema: "S",
        tableName: catalog.relations[0].table.name,
        name: 'CHECK "synthetic',
        checkClause: '"EXACT" > 0\nAND "label" <> \'雪 <img src=x onerror=alert(1)>\'',
      },
    ],
  };
  const keyConstraints: RelationConstraints = {
    revision: catalog.revision,
    targetId: catalog.relations[0].targetId,
    database: catalog.database,
    table: catalog.relations[0].table,
    constraints: [
      {
        catalog: "DB",
        schema: "S",
        name: "PK synthetic",
        kind: "primaryKey",
        isDeferrable: false,
        initiallyDeferred: false,
        enforced: false,
        rely: true,
        comment: "Declared only",
        referenceStatus: "notApplicable",
        reference: null,
      },
      {
        catalog: "DB",
        schema: "S",
        name: "FK synthetic",
        kind: "foreignKey",
        isDeferrable: false,
        initiallyDeferred: false,
        enforced: false,
        rely: false,
        comment: null,
        referenceStatus: "available",
        reference: {
          catalog: "PARENT",
          schema: "P",
          name: "UQ parent",
          matchOption: "FULL",
          updateRule: "NO ACTION",
          deleteRule: "CASCADE",
        },
      },
      {
        catalog: "DB",
        schema: "S",
        name: "FK hidden",
        kind: "foreignKey",
        isDeferrable: false,
        initiallyDeferred: false,
        enforced: false,
        rely: false,
        comment: null,
        referenceStatus: "notVisible",
        reference: null,
      },
    ],
  };
  const keyMappings: RelationConstraintMappings = {
    revision: catalog.revision,
    targetId: catalog.relations[0].targetId,
    database: catalog.database,
    table: catalog.relations[0].table,
    mappings: [
      {
        catalog: "DB",
        schema: "S",
        name: 'PK "synthetic',
        kind: "primaryKey",
        status: "available",
        columns: [
          { ordinal: 2, columnName: "second.part", referencedColumnName: null },
          { ordinal: 1, columnName: 'first"part', referencedColumnName: null },
        ],
        referencedTable: null,
      },
      {
        catalog: "DB",
        schema: "S",
        name: "FK synthetic",
        kind: "foreignKey",
        status: "available",
        columns: [
          { ordinal: 2, columnName: "second.part", referencedColumnName: "parent.second" },
          { ordinal: 1, columnName: 'first"part', referencedColumnName: 'parent"first' },
        ],
        referencedTable: {
          database: "DB",
          table: referencedTarget.table,
        },
      },
      {
        catalog: "DB",
        schema: "S",
        name: "FK hidden",
        kind: "foreignKey",
        status: "notVisible",
        columns: [],
        referencedTable: null,
      },
    ],
  };
  // Capture the app's copy request without touching the user's clipboard.
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          Object.assign(window, { copiedDefinition: text });
        },
      },
    });
  });
  await page.route("http://127.0.0.1:1422/rpc/*", async (route) => {
    const command = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const args = route.request().postDataJSON();
    calls.push({ command, args });
    let result: unknown;
    if (command === "save_profile") result = profile = args.profile;
    else if (command === "prepare_connection") result = "synthetic-attempt";
    else if (command === "connect_prepared")
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
          cancellation: true,
          tableBrowsing: false,
          relationBrowsing: true,
          tableEditing: false,
          tableFiltering: false,
          processes: false,
        },
      };
    else if (command === "session_contexts" || command === "completion_catalog") {
      await route.fulfill({
        json: { error: { code: "unsupported", message: "Synthetic context unavailable" } },
      });
      return;
    } else if (command === "relation_catalog") {
      if (failRefresh) {
        await route.fulfill({
          json: { error: { code: "database", message: "Synthetic refresh denied" } },
        });
        return;
      }
      result = {
        ...catalog,
        relations: catalog.relations.map((relation) => ({
          ...relation,
          traits: { ...relation.traits, dynamic: dynamicTarget },
        })),
      };
    } else if (command === "inspect_relation_check_constraints") {
      expect(args).toEqual({
        sessionId,
        request: { revision: "catalog-r1", targetId: "opaque-r1-target" },
      });
      if (checksMode === "denied") {
        await route.fulfill({
          json: { error: { code: "database", message: "Synthetic CHECK denied" } },
        });
        return;
      }
      if (checksMode === "held")
        await new Promise<void>((resolve) => {
          releaseChecks = resolve;
        });
      result = checksMode === "empty" ? { ...checkConstraints, constraints: [] } : checkConstraints;
    } else if (command === "inspect_relation_constraint_mappings") {
      expect(args).toEqual({
        sessionId,
        request: { revision: "catalog-r1", targetId: "opaque-r1-target" },
      });
      if (mappingsMode === "denied") {
        await route.fulfill({
          json: { error: { code: "database", message: "Synthetic key column mapping denied" } },
        });
        return;
      }
      if (mappingsMode === "held")
        await new Promise<void>((resolve) => {
          releaseMappings = resolve;
        });
      result =
        mappingsMode === "empty"
          ? { ...keyMappings, mappings: [] }
          : mappingsMode === "unsupported"
            ? {
                ...keyMappings,
                mappings: keyMappings.mappings.map((mapping) => ({
                  ...mapping,
                  status: "unsupported",
                  columns: [],
                  referencedTable: null,
                })),
              }
            : keyMappings;
    } else if (command === "inspect_relation_constraints") {
      expect(args).toEqual({
        sessionId,
        request: { revision: "catalog-r1", targetId: "opaque-r1-target" },
      });
      if (constraintsMode === "denied") {
        await route.fulfill({
          json: { error: { code: "database", message: "Synthetic key metadata denied" } },
        });
        return;
      }
      if (constraintsMode === "held")
        await new Promise<void>((resolve) => {
          releaseConstraints = resolve;
        });
      result =
        constraintsMode === "empty" ? { ...keyConstraints, constraints: [] } : keyConstraints;
    } else if (command === "define_relation") {
      expect(args).toEqual({
        sessionId,
        request: { revision: "catalog-r1", targetId: "opaque-r1-target" },
      });
      if (definitionMode === "denied") {
        await route.fulfill({
          json: { error: { code: "database", message: "Synthetic GET_DDL permission denied" } },
        });
        return;
      }
      if (definitionMode === "held")
        await new Promise<void>((resolve) => {
          releaseDefinition = resolve;
        });
      result = definition;
    } else if (command === "inspect_relation") {
      const targetId = args.request.targetId;
      expect(["opaque-r1-target", referencedTarget.targetId]).toContain(targetId);
      expect(args).toEqual({
        sessionId,
        request: { revision: "catalog-r1", targetId },
      });
      result =
        targetId === referencedTarget.targetId
          ? { ...inspection, targetId, table: referencedTarget.table }
          : inspection;
    } else if (command === "preview_relation") {
      expect(args).toEqual({
        sessionId,
        request: {
          revision: "catalog-r1",
          targetId: "opaque-r1-target",
          inspectionRevision: "inspection-r1",
        },
      });
      result = preview;
    } else if (command === "health") result = 1;
    else {
      await route.continue();
      return;
    }
    await route.fulfill({ json: { result } });
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Connect a database" }).click();
  await page.getByLabel("Database engine").selectOption("snowflake");
  await page.getByLabel("Snowflake authentication").selectOption("externalBrowser");
  await page.getByLabel("Connection name", { exact: true }).fill("Synthetic SDK browser");
  await page
    .getByLabel("Snowflake account hostname")
    .fill("fixture.invalid.snowflakecomputing.com");
  await page.getByLabel("Database", { exact: true }).fill("DB");
  await page.getByLabel("Username", { exact: true }).fill("READER");
  await page.getByLabel("Role", { exact: true }).fill("READER");
  await page.getByLabel("Warehouse", { exact: true }).fill("WH");
  await page.getByLabel("Schema", { exact: true }).fill("S");
  await page.getByRole("button", { name: "Save and connect", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "New query tab" }).click();
  const open = page.getByRole("button", { name: "Browse relations", exact: true });
  await open.click();
  const dialog = page.getByRole("dialog", { name: "Browse relations" });
  await expect(dialog.getByRole("status").first()).toContainText("4 visible relations · DB");
  await dialog.getByLabel("Relation", { exact: true }).selectOption("shadowed-target");
  await expect(dialog.getByRole("button", { name: "Inspect relation" })).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "View definition" })).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "View key constraints" })).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "View key column mappings" })).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "View CHECK constraints" })).toBeDisabled();
  await expect(dialog.getByRole("note")).toContainText("shadowed");
  await dialog.getByLabel("Find relation").fill('Quoted "table');
  await dialog.getByLabel("Relation", { exact: true }).selectOption("opaque-r1-target");
  const viewChecks = dialog.getByRole("button", { name: "View CHECK constraints" });
  const checksRegion = dialog.getByRole("region", { name: "Relation CHECK constraints" });
  const checksTable = checksRegion.getByRole("table", { name: "CHECK constraint inventory" });
  await expect(checksRegion).toContainText("CHECK constraints have not been fetched");
  await viewChecks.focus();
  await page.keyboard.press("Enter");
  await expect(checksTable.getByRole("rowheader")).toHaveText('"DB"."S"."CHECK ""synthetic"');
  await expect(checksTable.locator("pre")).toHaveText(checkConstraints.constraints[0].checkClause);
  expect(await checksTable.locator("pre").textContent()).toBe(
    checkConstraints.constraints[0].checkClause,
  );
  await expect(checksTable.locator("img,script")).toHaveCount(0);
  await expect(checksRegion.getByRole("note")).toContainText(
    "do not report enforcement or validation",
  );
  await expect(checksRegion.getByRole("note")).toContainText("grant row editing authority");
  await expect(checksRegion.getByRole("button")).toHaveCount(0);
  const viewMappings = dialog.getByRole("button", { name: "View key column mappings" });
  const mappingsRegion = dialog.getByRole("region", { name: "Relation key column mappings" });
  const mappingsTable = mappingsRegion.getByRole("table", { name: "Key column mapping inventory" });
  await expect(mappingsRegion).toContainText("Key column mappings have not been fetched");
  await viewMappings.focus();
  await page.keyboard.press("Enter");
  await expect(mappingsTable.getByRole("row").nth(1)).toContainText('"DB"."S"."PK ""synthetic"');
  await expect(mappingsTable.getByRole("row").nth(1)).toContainText('"first""part"');
  await expect(mappingsTable.getByRole("row").nth(1)).toContainText("Not applicable");
  await expect(mappingsTable.getByRole("row").nth(2)).toContainText('"second.part"');
  await expect(mappingsTable.getByRole("row").nth(3)).toContainText('"first""part"');
  await expect(mappingsTable.getByRole("row").nth(3)).toContainText(
    '"DB"."P.dot"."Quoted ""parent"',
  );
  await expect(mappingsTable.getByRole("row").nth(3)).toContainText('"parent""first"');
  await expect(mappingsTable.getByRole("row").nth(4)).toContainText('"parent.second"');
  await expect(mappingsTable.getByRole("row").nth(5)).toContainText("Not visible");
  await expect(mappingsRegion.getByRole("note")).toContainText("can lag schema changes");
  await expect(mappingsRegion.getByRole("note")).toContainText(
    "do not establish row editing safety",
  );
  const selectReference = mappingsRegion.getByRole("button", {
    name: 'Select referenced table "DB"."P.dot"."Quoted ""parent"',
    exact: true,
  });
  await expect(selectReference).toBeEnabled();
  expect(
    calls.filter((c) =>
      ["inspect_relation", "inspect_relation_constraints", "define_relation"].includes(c.command),
    ),
  ).toHaveLength(0);
  await expect(dialog.getByRole("option", { name: /P.dot.Quoted/ })).toHaveCount(0);
  await selectReference.focus();
  const beforeSelection = calls.length;
  await page.keyboard.press("Enter");
  expect(calls).toHaveLength(beforeSelection);
  await expect(dialog.getByLabel("Relation", { exact: true })).toHaveValue(
    referencedTarget.targetId,
  );
  await expect(dialog.getByLabel("Relation", { exact: true })).toBeFocused();
  await expect(dialog.getByLabel("Find relation")).toHaveValue("");
  await expect(mappingsTable).toHaveCount(0);
  await expect(checksTable).toHaveCount(0);
  await expect(checksRegion).toContainText("CHECK constraints have not been fetched");
  await dialog.getByRole("button", { name: "Inspect relation" }).click();
  await expect(dialog.getByRole("heading", { name: 'DB.P.dot.Quoted "parent' })).toBeVisible();
  expect(calls.filter((call) => call.command === "inspect_relation").at(-1)?.args).toEqual({
    sessionId,
    request: { revision: "catalog-r1", targetId: referencedTarget.targetId },
  });
  await dialog.getByLabel("Relation", { exact: true }).selectOption("opaque-r1-target");
  await viewMappings.click();
  await expect(mappingsTable).toBeVisible();
  const viewKeys = dialog.getByRole("button", { name: "View key constraints" });
  const keyRegion = dialog.getByRole("region", { name: "Relation key constraints" });
  const keyTable = keyRegion.getByRole("table", { name: "Key constraint inventory" });
  await expect(keyRegion).toContainText("Key constraints have not been fetched");
  await viewKeys.focus();
  await page.keyboard.press("Enter");
  await expect(keyTable).toContainText('"DB"."S"."PK synthetic"');
  await expect(keyTable).toContainText('"PARENT"."P"."UQ parent"');
  await expect(keyTable).toContainText("Not visible");
  await expect(keyTable).toContainText("CASCADE");
  await expect(keyRegion.getByRole("note")).toContainText("do not establish row editing safety");
  await expect(keyRegion.getByRole("note")).toContainText(
    "excludes column mappings, CHECK constraints, and NOT NULL",
  );
  await expect(keyRegion.getByRole("button")).toHaveCount(0);
  expect(calls.filter((c) => c.command === "define_relation")).toHaveLength(0);
  expect(calls.filter((c) => c.command === "inspect_relation")).toHaveLength(1);
  constraintsMode = "denied";
  await viewKeys.click();
  await expect(keyRegion.getByRole("alert")).toContainText("Synthetic key metadata denied");
  await expect(keyTable).toHaveCount(0);
  await expect(keyRegion).not.toContainText("No visible declared");
  constraintsMode = "empty";
  await viewKeys.click();
  await expect(keyRegion).toContainText(
    "No visible declared primary, unique, or foreign keys were returned",
  );
  await expect(keyRegion.getByRole("alert")).toHaveCount(0);
  constraintsMode = "held";
  await viewKeys.click();
  await expect.poll(() => !!releaseConstraints).toBe(true);
  await dialog.getByRole("button", { name: "Refresh relations" }).click();
  await expect(dialog.getByRole("status").first()).toContainText("4 visible relations · DB");
  const staleKeys = page.waitForResponse((response) =>
    response.url().endsWith("/rpc/inspect_relation_constraints"),
  );
  releaseConstraints!();
  await (await staleKeys).finished();
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      }),
  );
  await expect(keyTable).toHaveCount(0);
  await expect(viewKeys).toBeDisabled();
  constraintsMode = "available";
  await dialog.getByLabel("Relation", { exact: true }).selectOption("opaque-r1-target");
  await viewKeys.click();
  await expect(keyTable).toContainText("PK synthetic");
  await viewMappings.click();
  await expect(mappingsTable).toContainText("Not visible");
  // Native buttons expose the definition and exact copy flow to the keyboard,
  // independently of column inspection. The panel offers no execute/editor path.
  const viewDefinition = dialog.getByRole("button", { name: "View definition" });
  await viewDefinition.focus();
  await page.keyboard.press("Enter");
  const definitionRegion = dialog.getByRole("region", { name: "Relation definition" });
  await expect(definitionRegion.getByLabel("Object DDL")).toHaveText(definition.ddl);
  await expect(keyTable).toContainText("PK synthetic");
  await expect(definitionRegion.getByRole("note")).toContainText("CREATE OR REPLACE");
  await expect(definitionRegion.getByRole("note")).toContainText("replay is safe or complete");
  await expect(definitionRegion.getByRole("button")).toHaveCount(1);
  await definitionRegion.getByRole("button", { name: "Copy DDL" }).focus();
  await page.keyboard.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { copiedDefinition?: string }).copiedDefinition),
    )
    .toBe(definition.ddl);
  expect(calls.filter((c) => c.command === "inspect_relation")).toHaveLength(1);

  await viewChecks.click();
  await expect(checksTable).toBeVisible();
  checksMode = "denied";
  await viewChecks.click();
  await expect(checksRegion.getByRole("alert")).toContainText("Synthetic CHECK denied");
  await expect(checksTable).toHaveCount(0);
  await expect(checksRegion).not.toContainText("No visible CHECK constraints returned");
  await expect(keyTable).toContainText("PK synthetic");
  await expect(mappingsTable).toContainText("Not visible");
  await expect(definitionRegion.getByLabel("Object DDL")).toHaveText(definition.ddl);
  checksMode = "empty";
  await viewChecks.click();
  await expect(checksRegion).toContainText("No visible CHECK constraints returned");
  await expect(checksRegion.getByRole("alert")).toHaveCount(0);

  mappingsMode = "denied";
  await viewMappings.click();
  await expect(mappingsRegion.getByRole("alert")).toContainText(
    "Synthetic key column mapping denied",
  );
  await expect(mappingsTable).toHaveCount(0);
  await expect(keyTable).toContainText("PK synthetic");
  await expect(definitionRegion.getByLabel("Object DDL")).toHaveText(definition.ddl);
  mappingsMode = "empty";
  await viewMappings.click();
  await expect(mappingsRegion).toContainText("No visible key column mappings were returned");
  await expect(mappingsRegion.getByRole("alert")).toHaveCount(0);

  definitionMode = "denied";
  await viewDefinition.click();
  await expect(dialog.getByRole("alert")).toContainText("Synthetic GET_DDL permission denied");
  await expect(definitionRegion).toHaveCount(0);
  await expect(dialog.getByRole("button", { name: "Copy DDL" })).toHaveCount(0);
  await dialog.getByRole("button", { name: "Refresh relations" }).click();
  await expect(dialog.getByRole("status").first()).toContainText("4 visible relations · DB");
  await dialog.getByLabel("Relation", { exact: true }).selectOption("opaque-r1-target");
  definitionMode = "held";
  await viewDefinition.click();
  await expect.poll(() => !!releaseDefinition).toBe(true);
  await dialog.getByRole("button", { name: "Refresh relations" }).click();
  await expect(dialog.getByRole("status").first()).toContainText("4 visible relations · DB");
  const staleResponse = page.waitForResponse((response) =>
    response.url().endsWith("/rpc/define_relation"),
  );
  releaseDefinition!();
  await (await staleResponse).finished();
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      }),
  );
  await expect(viewDefinition).toBeDisabled();
  await expect(definitionRegion).toHaveCount(0);
  definitionMode = "available";
  dynamicTarget = true;
  mappingsMode = "unsupported";
  await dialog.getByRole("button", { name: "Refresh relations" }).click();
  await expect(dialog.getByRole("status").first()).toContainText("4 visible relations · DB");
  await dialog.getByLabel("Relation", { exact: true }).selectOption("opaque-r1-target");
  await viewMappings.click();
  await expect(mappingsTable.getByText("Unsupported", { exact: true })).toHaveCount(3);
  await expect(mappingsRegion).not.toContainText("No visible key column mappings were returned");
  dynamicTarget = false;
  await dialog.getByRole("button", { name: "Refresh relations" }).click();
  await expect(dialog.getByRole("status").first()).toContainText("4 visible relations · DB");
  await dialog.getByLabel("Relation", { exact: true }).selectOption("opaque-r1-target");
  mappingsMode = "held";
  await viewMappings.click();
  await expect.poll(() => !!releaseMappings).toBe(true);
  await dialog.getByRole("button", { name: "Refresh relations" }).click();
  await expect(dialog.getByRole("status").first()).toContainText("4 visible relations · DB");
  const staleMappings = page.waitForResponse((response) =>
    response.url().endsWith("/rpc/inspect_relation_constraint_mappings"),
  );
  releaseMappings!();
  await (await staleMappings).finished();
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      }),
  );
  await expect(mappingsTable).toHaveCount(0);
  await expect(viewMappings).toBeDisabled();
  mappingsMode = "available";
  await dialog.getByLabel("Relation", { exact: true }).selectOption("opaque-r1-target");
  checksMode = "held";
  await viewChecks.click();
  await expect.poll(() => !!releaseChecks).toBe(true);
  await dialog.getByRole("button", { name: "Refresh relations" }).click();
  await expect(dialog.getByRole("status").first()).toContainText("4 visible relations · DB");
  const staleChecks = page.waitForResponse((response) =>
    response.url().endsWith("/rpc/inspect_relation_check_constraints"),
  );
  releaseChecks!();
  await (await staleChecks).finished();
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      }),
  );
  await expect(checksTable).toHaveCount(0);
  await expect(checksRegion.getByRole("alert")).toHaveCount(0);
  await expect(viewChecks).toBeDisabled();
  checksMode = "available";
  await dialog.getByLabel("Relation", { exact: true }).selectOption("opaque-r1-target");
  await dialog.getByRole("button", { name: "Inspect relation" }).click();
  await expect(dialog).toContainText("NUMBER(38,0)");
  await expect(dialog).toContainText("Key constraints have not been fetched");
  await dialog.getByRole("button", { name: "Preview rows" }).click();
  await expect(dialog.getByRole("table", { name: "Read-only preview" })).toContainText(
    "9007199254740993",
  );
  await expect(dialog.getByLabel("NULL", { exact: true })).toBeVisible();
  await expect(dialog.getByLabel("Empty string", { exact: true })).toBeVisible();
  await dialog.getByRole("button", { name: "Next rows" }).click();
  await expect(dialog).toContainText("Rows 26–50");
  expect(calls.filter((c) => c.command === "preview_relation")).toHaveLength(1);
  failRefresh = true;
  await dialog.getByRole("button", { name: "Refresh relations" }).click();
  await expect(dialog.getByRole("alert")).toContainText("Synthetic refresh denied");
  await expect(dialog.getByRole("table", { name: "Read-only preview" })).toHaveCount(0);
  await expect(dialog.getByRole("button", { name: "Preview rows" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(open).toBeFocused();
  expect(calls.filter((c) => c.command === "inspect_relation")).toHaveLength(2);
  for (const call of calls.filter((c) =>
    [
      "relation_catalog",
      "define_relation",
      "inspect_relation_constraints",
      "inspect_relation_constraint_mappings",
      "inspect_relation_check_constraints",
      "inspect_relation",
      "preview_relation",
    ].includes(c.command),
  ))
    expect(call.args.sessionId).toBe(sessionId);
});
