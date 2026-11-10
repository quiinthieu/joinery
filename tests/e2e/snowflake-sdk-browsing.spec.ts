import { expect, test } from "./fixtures";
import type {
  BrowseCatalog,
  ColumnInfo,
  ConnectionProfile,
  RelationInspection,
  RelationPreview,
} from "../../src/ipc/generated";

// Synthetic RPCs exercise the real UI only; no Snowflake SSO/account SQL occurs.
test("SDK console browses qualified targets, inspects, pages a preview and rejects a failed refresh", async ({
  page,
}) => {
  const sessionId = "synthetic-browse-session";
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  let profile!: ConnectionProfile;
  let failRefresh = false;
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
      result = catalog;
    } else if (command === "inspect_relation") {
      expect(args).toEqual({
        sessionId,
        request: { revision: "catalog-r1", targetId: "opaque-r1-target" },
      });
      result = inspection;
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
  await expect(dialog.getByRole("status").first()).toContainText("2 visible relations · DB");
  await dialog.getByLabel("Relation", { exact: true }).selectOption("shadowed-target");
  await expect(dialog.getByRole("button", { name: "Inspect relation" })).toBeDisabled();
  await expect(dialog.getByRole("note")).toContainText("shadowed");
  await dialog.getByLabel("Relation", { exact: true }).selectOption("opaque-r1-target");
  await dialog.getByRole("button", { name: "Inspect relation" }).click();
  await expect(dialog).toContainText("NUMBER(38,0)");
  await expect(dialog).toContainText("Constraints have not been fetched");
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
  expect(calls.filter((c) => c.command === "inspect_relation")).toHaveLength(1);
  for (const call of calls.filter((c) =>
    ["relation_catalog", "inspect_relation", "preview_relation"].includes(c.command),
  ))
    expect(call.args.sessionId).toBe(sessionId);
});
