import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { TableSchema } from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import {
  DatabaseOutputPanel,
  MAX_DATABASE_OUTPUT_ENTRIES,
  appendDatabaseOutputEntries,
} from "./DatabaseOutputPanel";
import { generateTableSql } from "./sql-codegen";

const sampleSchema: TableSchema = {
  table: { schema: "main", name: "orders" },
  primaryKey: ["id"],
  foreignKeys: [],
  inboundForeignKeys: [],
  editable: true,
  columns: [
    {
      id: "id",
      name: "id",
      databaseType: "INTEGER",
      kind: "integer",
      nullable: false,
      primaryKey: true,
      foreignKey: false,
      editable: true,
      insertable: false,
      hasDefault: true,
    },
    {
      id: "customer_email",
      name: "customer_email",
      databaseType: "TEXT",
      kind: "text",
      nullable: false,
      primaryKey: false,
      foreignKey: false,
      editable: true,
      insertable: true,
      hasDefault: false,
    },
    {
      id: "total_cents",
      name: "total_cents",
      databaseType: "INTEGER",
      kind: "integer",
      nullable: false,
      primaryKey: false,
      foreignKey: false,
      editable: true,
      insertable: true,
      hasDefault: false,
    },
  ],
};

describe("Database Output session log and SQL code generation", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("caps Database Output ring buffer at 200 entries, filters warnings/errors, and records live query events", () => {
    const seed = Array.from({ length: MAX_DATABASE_OUTPUT_ENTRIES + 5 }, (_, idx) => ({
      id: `e-${idx}`,
      timestampMs: 1700000000000 + idx,
      level: (idx === MAX_DATABASE_OUTPUT_ENTRIES + 4 ? "error" : "info") as "error" | "info",
      code: idx === MAX_DATABASE_OUTPUT_ENTRIES + 4 ? "database" : undefined,
      message: `Entry #${idx}`,
    }));
    const capped = appendDatabaseOutputEntries([], seed);
    expect(capped).toHaveLength(MAX_DATABASE_OUTPUT_ENTRIES);
    expect(capped[0].id).toBe("e-5");

    const onClear = vi.fn();
    render(<DatabaseOutputPanel entries={capped} onClear={onClear} />);
    expect(screen.getByRole("region", { name: "Database output" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /Warnings & errors \(1\)/i }));
    expect(screen.getByText("Entry #204")).toBeTruthy();
    expect(screen.queryByText("Entry #6")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Clear database output" }));
    expect(onClear).toHaveBeenCalledTimes(1);

    // Verify applyEvents populates transient outputLog and clearTabOutput empties it
    useStudio.setState({
      tabs: [
        {
          id: "tab-out-1",
          kind: "query",
          title: "Query 1",
          connectionId: "c1",
          sql: "SELECT 1;",
          results: [],
          activeResultId: null,
          status: "idle",
          runId: null,
          elapsedMs: 0,
          error: null,
          table: null,
          object: null,
          query: { sort: [], filters: [], whereClause: null, orderClause: null },
          schema: null,
          pages: [],
          pageKeys: {},
          loadedRows: 0,
          hasMore: false,
          nextKey: null,
          loadingPage: false,
          view: "data",
          revision: 1,
          metrics: {},
          dataSessionId: null,
          querySession: null,
          querySessionClosed: false,
          fileToken: null,
          fileName: null,
          savedSql: null,
          savedQueryId: null,
        },
      ],
    });
    useStudio.getState().applyEvents(
      "tab-out-1",
      [
        { type: "started", runId: "run-1" },
        {
          type: "statementCompleted",
          runId: "run-1",
          index: 0,
          affectedRows: "3",
          elapsedMs: 12,
        },
        { type: "notice", message: "Result preview limit reached" },
        { type: "finished", runId: "run-1", status: "completed", elapsedMs: 15 },
      ],
      1,
    );
    const updatedTab = useStudio.getState().tabs.find((t) => t.id === "tab-out-1")!;
    expect(updatedTab.outputLog).toHaveLength(4);
    expect(updatedTab.outputLog?.some((e) => e.message.includes("3 affected"))).toBe(true);
    expect(updatedTab.outputLog?.some((e) => e.level === "warning")).toBe(true);

    useStudio.getState().clearTabOutput("tab-out-1");
    expect(useStudio.getState().tabs.find((t) => t.id === "tab-out-1")?.outputLog).toEqual([]);
  });

  it("generates dialect-quoted SELECT, INSERT, UPDATE, and DELETE SQL across engines", () => {
    expect(generateTableSql("select", "sqlite", sampleSchema)).toBe(
      'SELECT\n  "id",\n  "customer_email",\n  "total_cents"\nFROM "main"."orders"\nORDER BY "id"\nLIMIT 100;',
    );
    expect(
      generateTableSql("insert", "mysql", {
        ...sampleSchema,
        table: { schema: "shop", name: "orders" },
      }),
    ).toBe(
      "INSERT INTO `shop`.`orders` (`customer_email`, `total_cents`)\nVALUES (:customer_email, :total_cents);",
    );
    expect(
      generateTableSql("update", "postgres", {
        ...sampleSchema,
        table: { schema: "public", name: "orders" },
      }),
    ).toBe(
      'UPDATE "public"."orders"\nSET\n  "customer_email" = :customer_email,\n  "total_cents" = :total_cents\nWHERE "id" = :id;',
    );
    expect(
      generateTableSql("delete", "snowflake", {
        ...sampleSchema,
        table: { schema: "PUBLIC", name: "ORDERS" },
      }),
    ).toBe('DELETE FROM "PUBLIC"."ORDERS"\nWHERE "id" = :id;');

    // Leading-digit and colliding punctuation column names produce valid, unique :param slots
    expect(
      generateTableSql("insert", "postgres", {
        table: { schema: "public", name: "metrics" },
        primaryKey: [],
        foreignKeys: [],
        inboundForeignKeys: [],
        editable: true,
        columns: [
          {
            id: "2026_sales",
            name: "2026_sales",
            databaseType: "numeric",
            kind: "decimal",
            nullable: false,
            primaryKey: false,
            foreignKey: false,
            editable: true,
            insertable: true,
            hasDefault: false,
          },
          {
            id: "a-b",
            name: "a-b",
            databaseType: "text",
            kind: "text",
            nullable: true,
            primaryKey: false,
            foreignKey: false,
            editable: true,
            insertable: true,
            hasDefault: false,
          },
          {
            id: "a.b",
            name: "a.b",
            databaseType: "text",
            kind: "text",
            nullable: true,
            primaryKey: false,
            foreignKey: false,
            editable: true,
            insertable: true,
            hasDefault: false,
          },
        ],
      }),
    ).toBe(
      'INSERT INTO "public"."metrics" ("2026_sales", "a-b", "a.b")\nVALUES (:p_2026_sales, :a_b, :a_b_2);',
    );

    // Snowflake all-default INSERT emits valid column VALUES (DEFAULT) instead of unsupported DEFAULT VALUES
    expect(
      generateTableSql("insert", "snowflake", {
        table: { schema: "PUBLIC", name: "SEQ_ONLY" },
        primaryKey: ["ID"],
        foreignKeys: [],
        inboundForeignKeys: [],
        editable: true,
        columns: [
          {
            id: "ID",
            name: "ID",
            databaseType: "NUMBER(38,0)",
            kind: "integer",
            nullable: false,
            primaryKey: true,
            foreignKey: false,
            editable: false,
            insertable: false,
            hasDefault: true,
          },
        ],
      }),
    ).toBe('INSERT INTO "PUBLIC"."SEQ_ONLY" ("ID")\nVALUES (DEFAULT);');

    // >64-char column names and >64 columns stay within params.rs (<=64 chars, <=64 slots)
    const longPrefix = "c".repeat(75);
    const wideCols = Array.from({ length: 68 }, (_, idx) => ({
      id: `${longPrefix}_${idx}`,
      name: `${longPrefix}_${idx}`,
      databaseType: "VARCHAR",
      kind: "text" as const,
      nullable: true,
      primaryKey: idx === 0,
      foreignKey: false,
      editable: idx > 0,
      insertable: true,
      hasDefault: false,
    }));
    const wideUpdate = generateTableSql("update", "postgres", {
      table: { schema: "public", name: "wide_tbl" },
      primaryKey: [wideCols[0].name],
      foreignKeys: [],
      inboundForeignKeys: [],
      editable: true,
      columns: wideCols,
    });
    const paramMatches = [...wideUpdate.matchAll(/:([A-Za-z_][A-Za-z0-9_]*)/g)].map((m) => m[1]);
    expect(paramMatches.length).toBe(64);
    expect(paramMatches.every((p) => p.length <= 64)).toBe(true);
    expect(new Set(paramMatches).size).toBe(64);
    expect(wideUpdate).toContain("WHERE ");
    expect(wideUpdate.split("WHERE ")[1]).toMatch(/:[A-Za-z_][A-Za-z0-9_]*;/);
  });
});
