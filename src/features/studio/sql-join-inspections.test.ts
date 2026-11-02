import { EditorState } from "@codemirror/state";
import { sql } from "@codemirror/lang-sql";
import { describe, expect, test } from "vitest";
import type { EngineKind, TableRef, TableSchema } from "../../ipc/generated";
import { sqlDialect } from "./sql-identifiers";
import type { SqlSymbolContext } from "./sql-navigation";
import { prepareColumnInspection } from "./sql-inspections";

const tables = ["orders", "customers", "regions"].map((name) => ({ schema: "public", name }));
const schema = (table: TableRef, names: string[]): TableSchema => ({
  table,
  columns: names.map((name) => ({
    id: name,
    name,
    databaseType: "text",
    kind: "text",
    nullable: true,
    primaryKey: false,
    foreignKey: false,
    editable: false,
    insertable: false,
    hasDefault: false,
  })),
  primaryKey: [],
  foreignKeys: [],
  editable: false,
});
const schemas = [
  schema(tables[0], ["id", "customer_id", "order_only"]),
  schema(tables[1], ["id", "region_id", "customer_only"]),
  schema(tables[2], ["id", "region_only"]),
];
const context = (engine: EngineKind = "postgres"): SqlSymbolContext => ({
  engine,
  caseSensitiveTableNames: true,
  tables: tables.map((table) => ({ table, kind: "table", estimatedRows: null, totalBytes: null })),
  unqualifiedTables: tables,
});
const stateOf = (doc: string, engine: EngineKind = "postgres") =>
  EditorState.create({ doc, extensions: [sql({ dialect: sqlDialect(engine) })] });
function inspect(doc: string, engine: EngineKind = "postgres") {
  const state = stateOf(doc, engine);
  const plan = prepareColumnInspection(state, 7, context(engine));
  return {
    state,
    plan,
    result: plan.finish(
      state,
      plan.tables.map((t) => schemas.find((s) => s.table.name === t.name)!),
    ),
  };
}

describe("ordered direct-table JOIN column inspections", () => {
  test.each(["postgres", "mysql"] as const)(
    "%s resolves projection, ON and WHERE to each owning table",
    (engine) => {
      const { state, plan, result } = inspect(
        "SELECT o.typo, c.typo FROM orders o JOIN customers c ON o.customer_id = c.id WHERE c.customer_only IS NOT NULL ORDER BY o.id",
        engine,
      );
      expect(plan.tables).toEqual(tables.slice(0, 2));
      expect(result.checked).toBe(6);
      expect(result.issues.map((i) => [i.qualifier, i.name, i.table.name])).toEqual([
        ["o", "typo", "orders"],
        ["c", "typo", "customers"],
      ]);
      expect(result.columnsFor(result.issues[0])).toContain("order_only");
      expect(result.columnsFor(result.issues[1])).toContain("customer_only");
      expect(() => result.preview(state, result.issues[0], "customer_only")).toThrow(
        /Choose a column/,
      );
      const quote = engine === "mysql" ? "`" : '"';
      expect(
        state
          .update({ changes: result.preview(state, result.issues[1], "customer_only") })
          .state.doc.toString(),
      ).toContain(`o.typo, c.${quote}customer_only${quote}`);
    },
  );
  test("retains each pending JOIN's actual operands", () => {
    const q =
      "SELECT o.id FROM orders o JOIN customers c JOIN regions r ON c.region_id = r.id ON o.customer_id = c.id";
    expect(inspect(q).result.issues).toEqual([]);
    expect(() => inspect(q.replace("c.region_id", "o.customer_id"))).toThrow(
      /unresolved qualifier/,
    );
  });
  test("does not expose later relations to earlier ON clauses or earlier comma groups to a JOIN", () => {
    for (const q of [
      "SELECT o.id FROM orders o JOIN customers c ON r.id = c.id JOIN regions r ON c.region_id = r.id",
      "SELECT o.id FROM orders o, customers c JOIN regions r ON o.id = r.id",
    ])
      expect(() => inspect(q)).toThrow(/unresolved qualifier/);
    expect(
      inspect(
        "SELECT o.id FROM orders o, customers c JOIN regions r ON c.region_id = r.id WHERE o.id = c.id",
      ).result.issues,
    ).toEqual([]);
  });
  test.each([
    "LEFT JOIN customers c ON o.id = c.id",
    "RIGHT OUTER JOIN customers c ON o.id = c.id",
    "FULL JOIN customers c ON o.id = c.id",
    "CROSS JOIN customers c",
    "NATURAL JOIN customers c",
    "JOIN customers c USING (id)",
  ])("handles direct-table %s", (join) => {
    expect(inspect(`SELECT o.id, c.id FROM orders o ${join}`).result.issues).toEqual([]);
  });
  test("preserves CROSS operand precedence", () => {
    expect(
      inspect("SELECT o.id FROM orders o CROSS JOIN customers c JOIN regions r ON o.id = r.id")
        .result.issues,
    ).toEqual([]);
  });
  test("deduplicates metadata for self joins without conflating references", () => {
    const { plan, result } = inspect(
      "SELECT a.typo, b.id FROM orders a JOIN orders b ON a.id = b.id",
    );
    expect(plan.tables).toEqual([tables[0]]);
    expect(result.checked).toBe(4);
    expect(result.issues[0].qualifier).toBe("a");
  });
  test("requires complete, unique metadata for every table", () => {
    const state = stateOf("SELECT o.typo FROM orders o JOIN customers c ON o.id = c.id");
    const plan = prepareColumnInspection(state, 7, context());
    for (const bad of [
      [schemas[0]],
      [schemas[0], schemas[0]],
      [schemas[0], { ...schemas[1], columns: [] }],
      [...schemas],
    ])
      expect(() => plan.finish(state, bad)).toThrow(/metadata is unavailable/);
    expect(plan.finish(state, [schemas[1], schemas[0]]).issues).toHaveLength(1);
  });
  test("inspects all 32 relation operands without the completion candidate cap", () => {
    const all = Array.from({ length: 33 }, (_, i) => ({ schema: "public", name: `table_${i}` }));
    const ctx = {
      ...context(),
      tables: all.map((table) => ({ table, kind: "table", estimatedRows: null, totalBytes: null })),
      unqualifiedTables: all,
    };
    const query = (count: number) =>
      `SELECT t${count - 1}.typo FROM ${all
        .slice(0, count)
        .map((t, i) => `${t.name} t${i}`)
        .join(", ")}`;
    const state = stateOf(query(32));
    const plan = prepareColumnInspection(state, 7, ctx);
    expect(plan.tables).toEqual(all.slice(0, 32));
    expect(
      plan.finish(
        state,
        plan.tables.map((t) => schema(t, ["id"])),
      ).issues[0].table,
    ).toEqual(all[31]);
    expect(() => prepareColumnInspection(stateOf(query(33)), 7, ctx)).toThrow(/bounded JOINs/);
  });
  test("withholds oversized or duplicate metadata instead of returning partial claims", () => {
    const state = stateOf("SELECT o.typo FROM orders o JOIN customers c ON o.id = c.id");
    const plan = prepareColumnInspection(state, 7, context());
    const names = Array.from({ length: 2048 }, (_, i) => `c${i}`);
    expect(plan.finish(state, [schema(tables[0], names), schemas[1]]).issues).toHaveLength(2);
    for (const bad of [
      schema(tables[0], [...names, "one_more"]),
      schema(tables[0], ["id", "id"]),
      {
        ...schemas[0],
        columns: [{ ...schemas[0].columns[0], databaseType: "x".repeat(4 * 1024 * 1024) }],
      },
    ])
      expect(() => plan.finish(state, [bad, schemas[1]])).toThrow(/metadata is unavailable/);
    const mysql = prepareColumnInspection(
      stateOf(state.doc.toString(), "mysql"),
      7,
      context("mysql"),
    );
    expect(() => mysql.finish(state, [schema(tables[0], ["id", "ID"]), schemas[1]])).toThrow(
      /metadata is unavailable/,
    );
    const many = Array.from({ length: 5 }, (_, i) => ({ schema: "public", name: `wide_${i}` }));
    const wideState = stateOf(
      `SELECT t0.typo FROM ${many.map((t, i) => `${t.name} t${i}`).join(", ")}`,
    );
    const widePlan = prepareColumnInspection(wideState, 7, {
      ...context(),
      tables: many.map((table) => ({
        table,
        kind: "table",
        estimatedRows: null,
        totalBytes: null,
      })),
      unqualifiedTables: many,
    });
    expect(
      widePlan.finish(
        wideState,
        many.map((t, i) => schema(t, names.slice(0, i === 4 ? 4 : 2047))),
      ),
    ).toBeTruthy();
    expect(() =>
      widePlan.finish(
        wideState,
        many.map((t) => schema(t, names)),
      ),
    ).toThrow(/metadata is unavailable/);
  });
  test("resolves an unqualified select column only when one joined relation owns it", () => {
    const unique = inspect(
      "SELECT customer_only FROM orders o JOIN customers c ON o.id = c.id",
    ).result;
    expect(unique.issues).toEqual([]);
    expect(unique.checked).toBe(3);
    const missing = stateOf("SELECT typo FROM orders o JOIN customers c ON o.id = c.id");
    expect(() =>
      prepareColumnInspection(missing, 7, context()).finish(missing, schemas.slice(0, 2)),
    ).toThrow(/unqualified column/);
    const ambiguous = stateOf("SELECT id FROM orders o JOIN customers c ON o.customer_id = c.id");
    expect(() =>
      prepareColumnInspection(ambiguous, 7, context()).finish(ambiguous, schemas.slice(0, 2)),
    ).toThrow(/unqualified column/);
    const where = inspect(
      "SELECT o.id FROM orders o JOIN customers c ON o.id = c.id WHERE customer_only = 1",
    ).result;
    expect(where.issues).toEqual([]);
    const whereMissing = stateOf(
      "SELECT o.id FROM orders o JOIN customers c ON o.id = c.id WHERE typo = 1",
    );
    expect(() =>
      prepareColumnInspection(whereMissing, 7, context()).finish(whereMissing, schemas.slice(0, 2)),
    ).toThrow(/unqualified column/);
    expect(
      inspect(
        "SELECT o.id FROM orders o JOIN customers c ON o.id = c.id WHERE LOWER(customer_only) = 1",
      ).result.issues,
    ).toEqual([]);
    const called = stateOf(
      "SELECT o.id FROM orders o JOIN customers c ON o.id = c.id WHERE LOWER(typo) = 1",
    );
    expect(() =>
      prepareColumnInspection(called, 7, context()).finish(called, schemas.slice(0, 2)),
    ).toThrow(/unqualified column/);
    expect(
      inspect("SELECT (customer_only) FROM orders o JOIN customers c ON o.id = c.id").result.issues,
    ).toEqual([]);
    const grouped = stateOf("SELECT (typo) FROM orders o JOIN customers c ON o.id = c.id");
    expect(() =>
      prepareColumnInspection(grouped, 7, context()).finish(grouped, schemas.slice(0, 2)),
    ).toThrow(/unqualified column/);
    const groupedId = stateOf("SELECT (id) FROM orders o JOIN customers c ON o.customer_id = c.id");
    expect(() =>
      prepareColumnInspection(groupedId, 7, context()).finish(groupedId, schemas.slice(0, 2)),
    ).toThrow(/unqualified column/);
  });
  test("inspects a derived JOIN operand by its projected names", () => {
    const { result } = inspect(
      "SELECT a.id, c.customer_only FROM orders a JOIN (SELECT id FROM customers) c ON a.id = c.id",
    );
    expect(result.tables.map((item) => item.name)).toEqual(["orders", "customers"]);
    expect(result.issues.map((issue) => [issue.outputOf, issue.name])).toEqual([
      ["c", "customer_only"],
    ]);
    expect(result.columnsFor(result.issues[0])).toEqual(["id"]);
  });
  test("visits references inside JOIN expressions without treating functions as JOINs", () => {
    const { result } = inspect(
      "SELECT o.id FROM orders o JOIN customers c ON LEFT(o.typo, 2) = c.customer_only AND COALESCE(c.id, 0) = o.id",
    );
    expect(result.checked).toBe(5);
    expect(result.issues.map((i) => i.qualifier + "." + i.name)).toEqual(["o.typo"]);
  });
  test.each([
    "SELECT a.id FROM orders a JOIN customers a ON a.id = a.id",
    "SELECT a.id FROM orders a JOIN customers c",
    "SELECT a.id FROM orders a LEFT JOIN customers c",
    "SELECT a.id FROM orders a JOIN customers c ON",
    "SELECT a.id FROM orders a JOIN customers c USING (id) AS x",
    "SELECT a.id FROM orders a JOIN customers c ON EXISTS (SELECT 1 FROM regions)",
    "SELECT a.id FROM orders a JOIN customers c ON a.id = c.id,",
    "SELECT a.id FROM orders a JOIN customers c ON a.id = c.id TABLESAMPLE SYSTEM (10)",
  ])("withholds malformed/unsupported relation scope: %s", (q) =>
    expect(() => inspect(q)).toThrow(),
  );
  test("keeps MySQL optional inner/CROSS conditions distinct from PostgreSQL", () => {
    expect(
      inspect("SELECT o.id, c.id FROM orders o JOIN customers c", "mysql").result.issues,
    ).toEqual([]);
    expect(
      inspect("SELECT o.id, c.id FROM orders o CROSS JOIN customers c ON o.id = c.id", "mysql")
        .result.issues,
    ).toEqual([]);
    expect(() =>
      inspect("SELECT o.id FROM orders o CROSS JOIN customers c ON o.id = c.id"),
    ).toThrow();
    expect(() =>
      inspect("SELECT o.id FROM orders o FULL JOIN customers c ON o.id = c.id", "mysql"),
    ).toThrow();
  });
});
