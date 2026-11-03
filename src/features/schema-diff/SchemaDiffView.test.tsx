import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SchemaDiffReport, SchemaSnapshot, SessionInfo } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";
import { SchemaDiffView, filterSchemaDiffEntries, schemaSnapshots } from "./SchemaDiffView";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (e: unknown) => String(e instanceof Error ? e.message : e),
}));

const session: SessionInfo = {
  id: "session-pg-1",
  connectionId: "profile-pg",
  engine: "postgres",
  serverVersion: "PostgreSQL 17.2",
  latencyMs: 1,
  encrypted: false,
  capabilities: {
    sql: true,
    transactions: true,
    cancellation: true,
    tableBrowsing: true,
    tableEditing: true,
    tableFiltering: true,
    processes: true,
    objectExplorer: true,
  },
};

const leftSnapshot: SchemaSnapshot = {
  engine: "postgres",
  serverVersion: "PostgreSQL 17.2",
  database: "joinery",
  schema: "public",
  role: "joinery",
  tables: [
    {
      name: "orders",
      kind: "table",
      comment: null,
      partitionKey: null,
      viewDefinition: null,
      columns: [
        {
          name: "id",
          dataType: "bigint",
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
  unsupportedObjects: [
    {
      name: "order_state",
      objectType: "enum type",
      reason: "Custom enum types are not modeled by relational schema snapshots.",
    },
  ],
  truncated: false,
  restricted: false,
};

const rightSnapshot: SchemaSnapshot = {
  ...leftSnapshot,
  schema: "staging",
  unsupportedObjects: [],
};

const sampleReport: SchemaDiffReport = {
  leftLabel: "postgres:public",
  rightLabel: "postgres:staging",
  leftEngine: "postgres",
  rightEngine: "postgres",
  engineMismatch: false,
  leftServerVersion: "PostgreSQL 17.2",
  rightServerVersion: "PostgreSQL 17.2",
  leftTruncated: false,
  rightTruncated: false,
  leftRestricted: true,
  rightRestricted: false,
  identicalCount: 3,
  onlyLeftCount: 1,
  onlyRightCount: 1,
  modifiedCount: 1,
  leftUnsupported: leftSnapshot.unsupportedObjects,
  rightUnsupported: [],
  entries: [
    {
      objectKind: "table",
      name: "Order Items",
      quotedName: '"Order Items"',
      identity: null,
      kind: "modified",
      summary: "2 member changes",
      memberDiffs: [
        {
          category: "column",
          name: "Unit Price",
          quotedName: '"Unit Price"',
          kind: "modified",
          leftValue: "numeric(10,2) NOT NULL DEFAULT 0",
          rightValue: "numeric(12,2) NOT NULL DEFAULT 0",
        },
        {
          category: "index",
          name: "order_items_price_idx",
          quotedName: '"order_items_price_idx"',
          kind: "onlyRight",
          leftValue: null,
          rightValue: 'UNIQUE btree ("Unit Price")',
        },
      ],
    },
    {
      objectKind: "table",
      name: "legacy_audit",
      quotedName: '"legacy_audit"',
      identity: null,
      kind: "onlyLeft",
      summary: "Present only in left schema",
      memberDiffs: [],
    },
    {
      objectKind: "view",
      name: "staging_rollup_v",
      quotedName: '"staging_rollup_v"',
      identity: null,
      kind: "onlyRight",
      summary: "Present only in right schema",
      memberDiffs: [],
    },
  ],
};

const diffTab = {
  id: "schema-diff-tab",
  kind: "schemaDiff",
  title: "public diff",
  connectionId: "profile-pg",
  diagramSchema: "public",
} as unknown as StudioTab;

beforeEach(() => {
  schemaSnapshots.clear();
  useStudio.setState({
    bootstrap: {
      profiles: [
        {
          id: "profile-pg",
          name: "Local PG",
          groupId: "local",
          engine: "postgres",
          host: "127.0.0.1",
          port: 55432,
          database: "joinery",
          username: "joinery",
          environment: "local",
          safeMode: "readOnly",
          tlsMode: "disable",
          caCertPath: null,
          ssh: null,
          rememberPassword: false,
          storeQueryText: false,
          storeDrafts: false,
          snowflake: null,
        },
      ],
      groups: [],
      supportedEngines: ["postgres", "mysql", "redis", "snowflake"],
      hasPassphrase: false,
      keychainAvailable: true,
    } as never,
    sessions: { "profile-pg": session },
    catalog: {
      "profile-pg": [
        {
          table: { schema: "public", name: "orders" },
          kind: "table",
          estimatedRows: 10,
          totalBytes: 8192,
        },
        {
          table: { schema: "staging", name: "orders" },
          kind: "table",
          estimatedRows: 10,
          totalBytes: 8192,
        },
      ],
    },
    objects: {},
  });
  vi.mocked(client.call).mockReset();
});

afterEach(() => {
  cleanup();
  schemaSnapshots.clear();
});

it("loads snapshots, renders member diffs and unsupported disclosures, filters, captures snapshots and opens objects", async () => {
  vi.mocked(client.call).mockImplementation(async (command, payload) => {
    if (command === "schema_snapshot") {
      const req = payload as { schema: string };
      return req.schema === "staging" ? rightSnapshot : leftSnapshot;
    }
    if (command === "compare_schemas") {
      return sampleReport;
    }
    if (command === "generate_migration_ddl") {
      return {
        engine: "postgres",
        targetSchema: "public",
        statements: [
          {
            sql: 'DROP TABLE "public"."legacy_audit";',
            description: "Drop table legacy_audit",
            objectKind: "table",
            objectName: "legacy_audit",
            destructive: true,
          },
          {
            sql: 'ALTER TABLE "public"."Order Items" ALTER COLUMN "Unit Price" TYPE numeric(12,2);',
            description: "Change type of Order Items.Unit Price",
            objectKind: "table",
            objectName: "Order Items.Unit Price",
            destructive: false,
          },
        ],
        script:
          'DROP TABLE "public"."legacy_audit";\n\nALTER TABLE "public"."Order Items" ALTER COLUMN "Unit Price" TYPE numeric(12,2);',
        truncated: false,
        destructiveCount: 1,
        warnings: [
          "1 destructive statement(s) (DROP) detected; review carefully before executing.",
        ],
        unsupported: [],
      };
    }
    if (command === "generate_schema_ddl") {
      return {
        engine: "postgres",
        targetSchema: "public",
        statements: [
          {
            sql: 'CREATE TABLE "public"."orders" (\n  "id" bigint NOT NULL\n);',
            description: "Create table orders",
            objectKind: "table",
            objectName: "orders",
            destructive: false,
          },
        ],
        script: 'CREATE TABLE "public"."orders" (\n  "id" bigint NOT NULL\n);',
        truncated: false,
        destructiveCount: 0,
        warnings: [],
        unsupported: [],
      };
    }
    throw new Error(`Unexpected command: ${command}`);
  });

  const onOpenObject = vi.fn();
  render(<SchemaDiffView tab={diffTab} onOpenObject={onOpenObject} />);

  await screen.findByText(/Identical: 3/);
  expect(screen.getByText(/withheld by database privileges/)).toBeTruthy();
  expect(screen.getByLabelText("Unsupported schema objects disclosure").textContent).toContain(
    "order_state",
  );

  // Modified object is auto-expanded with quoted column and index member diffs
  const memberTable = screen.getByRole("table", { name: "Member differences for Order Items" });
  expect(memberTable.textContent).toContain('"Unit Price"');
  expect(memberTable.textContent).toContain("numeric(10,2) NOT NULL DEFAULT 0");
  expect(memberTable.textContent).toContain("numeric(12,2) NOT NULL DEFAULT 0");

  // Filter by kind: Only in left
  fireEvent.click(screen.getByRole("button", { name: "Only in left (1)" }));
  expect(screen.getByTestId("schema-diff-object-legacy_audit")).toBeTruthy();
  expect(screen.queryByTestId("schema-diff-object-Order Items")).toBeNull();

  // Reset filter to All differences and search by member name
  fireEvent.click(screen.getByRole("button", { name: "All differences (3)" }));
  fireEvent.change(screen.getByLabelText("Filter schema differences"), {
    target: { value: "order_items_price_idx" },
  });
  expect(screen.getByTestId("schema-diff-object-Order Items")).toBeTruthy();
  expect(screen.queryByTestId("schema-diff-object-legacy_audit")).toBeNull();

  // Inspect object callback
  const inspectButtons = screen.getAllByRole("button", { name: /Inspect/ });
  fireEvent.click(inspectButtons[0]!);
  expect(onOpenObject).toHaveBeenCalledWith("profile-pg", {
    schema: "public",
    name: "Order Items",
    kind: "table",
    identity: null,
  });

  // Capture left snapshot and verify it appears in the captured snapshots strip
  fireEvent.click(screen.getByRole("button", { name: "Capture left schema snapshot" }));
  await waitFor(() =>
    expect(screen.getByLabelText("Captured schema snapshots").textContent).toContain("public"),
  );

  // Preview Migration DDL (Left -> Right) and verify destructive warning & SQL script
  fireEvent.click(screen.getByRole("button", { name: "Migrate Left → Right DDL" }));
  const ddlPreview = await screen.findByTestId("schema-ddl-preview");
  expect(ddlPreview.textContent).toContain("1 destructive");
  expect(ddlPreview.textContent).toContain("1 destructive statement(s) (DROP) detected");
  expect(screen.getByTestId("schema-ddl-script").textContent).toContain(
    'DROP TABLE "public"."legacy_audit";',
  );

  // Switch to Full Left Schema DDL
  fireEvent.click(screen.getByRole("button", { name: "Full Left Schema DDL" }));
  await waitFor(() =>
    expect(screen.getByTestId("schema-ddl-script").textContent).toContain(
      'CREATE TABLE "public"."orders"',
    ),
  );
});

it("filters diff entries by kind and member values accurately", () => {
  expect(filterSchemaDiffEntries(sampleReport.entries, "onlyRight", "").map((e) => e.name)).toEqual(
    ["staging_rollup_v"],
  );
  expect(
    filterSchemaDiffEntries(sampleReport.entries, "all", "numeric(12,2)").map((e) => e.name),
  ).toEqual(["Order Items"]);
});
