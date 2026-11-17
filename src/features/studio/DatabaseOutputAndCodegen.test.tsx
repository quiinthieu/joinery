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
  });
});
