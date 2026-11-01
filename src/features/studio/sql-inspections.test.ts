import { EditorState } from "@codemirror/state";
import { sql } from "@codemirror/lang-sql";
import { describe, expect, test } from "vitest";
import type { EngineKind, TableSchema } from "../../ipc/generated";
import { sqlDialect } from "./sql-identifiers";
import type { SqlSymbolContext } from "./sql-navigation";
import { prepareColumnInspection } from "./sql-inspections";

const table = { schema: "public", name: "orders" };
const context = (engine: EngineKind = "postgres"): SqlSymbolContext => ({
  engine,
  tables: [{ table, kind: "table", estimatedRows: null, totalBytes: null }],
  unqualifiedTables: [table],
});
const schema: TableSchema = {
  table,
  columns: ["id", "customer_id", "Mixed", 'a"b', "a`b", "select"].map((name) => ({
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
};
const stateOf = (doc: string, engine: EngineKind = "postgres") =>
  EditorState.create({ doc, extensions: [sql({ dialect: sqlDialect(engine) })] });
function inspect(doc: string, engine: EngineKind = "postgres") {
  const state = stateOf(doc, engine);
  return {
    state,
    result: prepareColumnInspection(state, 7, context(engine)).finish(state, schema),
  };
}

describe("bounded qualified-column inspections", () => {
  test("finds repeated missing references without inspecting comments, literals, wildcards or other statements", () => {
    const { result } = inspect(
      "SELECT o.id, abs(o.typo), o.*, 'o.literal' FROM public.orders o WHERE o.typo > 0 ORDER BY o.id; SELECT x.bad FROM other x -- o.comment",
    );
    expect(result.checked).toBe(4);
    expect(result.issues.map((issue) => issue.name)).toEqual(["typo", "typo"]);
  });
  test.each(["postgres", "mysql"] as const)(
    "%s previews one selected occurrence with exact catalog quoting and rejects untrusted/stale edits",
    (engine) => {
      const { state, result } = inspect("SELECT o.typo, o.typo FROM public.orders o", engine);
      const issue = result.issues[0];
      for (const column of ["id", "select", 'a"b', "a`b"]) {
        const quote = engine === "mysql" ? "`" : '"';
        const change = result.preview(state, issue, column);
        expect(state.update({ changes: change }).state.doc.toString()).toBe(
          `SELECT o.${quote}${column.split(quote).join(quote + quote)}${quote}, o.typo FROM public.orders o`,
        );
      }
      expect(() => result.preview(state, issue, "invented")).toThrow(/Choose a column/);
      expect(() => result.preview(state, { ...issue }, "id")).toThrow(/Choose a column/);
      expect(() =>
        result.preview(
          state.update({ changes: { from: 0, insert: "-- changed\n" } }).state,
          issue,
          "id",
        ),
      ).toThrow(/SQL changed/);
      expect(
        result.preview(state.update({ selection: { anchor: 0 } }).state, issue, "id"),
      ).toBeTruthy();
    },
  );
  test("uses PostgreSQL folded and quoted column names and MySQL case insensitive column names", () => {
    expect(
      inspect('SELECT o.ID, o."Mixed", o.mixed FROM orders o').result.issues.map((i) => i.name),
    ).toEqual(["mixed"]);
    expect(inspect("SELECT o.ID, o.`MIXED` FROM orders o", "mysql").result.issues).toEqual([]);
  });
  test("accepts an implicit table qualifier, quoted alias and expressions", () => {
    expect(inspect('SELECT coalesce("X".id, 0) FROM public.orders AS "X"').result.checked).toBe(1);
    expect(inspect("SELECT orders.id FROM orders").result.issues).toEqual([]);
  });
  test.each([
    "WITH c AS (SELECT id FROM orders) SELECT c.bad FROM c",
    "SELECT d.bad FROM (SELECT id FROM orders) d",
    "SELECT a.bad FROM orders a JOIN orders b ON a.id = b.id",
    "SELECT a.bad FROM orders a, orders b",
    "SELECT a.bad FROM orders a WHERE EXISTS (SELECT 1 FROM orders)",
    "SELECT a.bad FROM orders a UNION SELECT a.id FROM orders a",
    "SELECT a.bad FROM orders(a)",
    "SELECT a.bad FROM orders a(id)",
    "UPDATE orders a SET id = a.bad",
    "SELECT a.bad INTO output FROM orders a",
    "SELECT a.bad FROM orders a TABLESAMPLE SYSTEM (10)",
    "SELECT a.bad FROM orders a JOIN",
    "SELECT a.bad FROM orders a WHERE (",
    "SELECT a.payload.field FROM orders a",
    "SELECT a.bad FROM orders a WHERE other.bad > 0",
  ])("withholds unsupported or unresolved shapes: %s", (doc) => {
    const state = stateOf(doc);
    expect(() => prepareColumnInspection(state, 7, context())).toThrow();
  });
  test("requires uniquely resolved base tables and server-observed unqualified visibility", () => {
    const state = stateOf("SELECT o.bad FROM orders o");
    expect(() =>
      prepareColumnInspection(state, 7, { ...context(), unqualifiedTables: undefined }),
    ).toThrow(/metadata is unavailable/);
    expect(() =>
      prepareColumnInspection(state, 7, {
        ...context(),
        tables: [...context().tables, ...context().tables],
      }),
    ).toThrow(/unique direct base/);
    expect(() =>
      prepareColumnInspection(state, 7, { ...context(), unqualifiedTables: [] }),
    ).toThrow(/unique direct base/);
    expect(() =>
      prepareColumnInspection(state, 7, {
        ...context(),
        tables: context().tables.map((t) => ({ ...t, kind: "view" })),
      }),
    ).toThrow(/unique direct base/);
  });
  test("does not infer an absent column from mismatched, empty, duplicate or stale metadata", () => {
    const state = stateOf("SELECT o.bad FROM orders o");
    const plan = prepareColumnInspection(state, 7, context());
    for (const bad of [
      { ...schema, table: { ...table, name: "other" } },
      { ...schema, columns: [] },
      { ...schema, columns: [schema.columns[0], schema.columns[0]] },
    ]) {
      expect(() => plan.finish(state, bad)).toThrow(/metadata is unavailable/);
    }
    expect(() =>
      plan.finish(state.update({ changes: { from: 0, insert: " " } }).state, schema),
    ).toThrow(/SQL changed/);
  });
  test("does not treat qualified function names as columns", () => {
    expect(inspect("SELECT public.abs(o.id) FROM orders o").result.issues).toEqual([]);
  });
  test("recognizes PostgreSQL system columns and withholds MySQL implicit row identifiers", () => {
    expect(inspect("SELECT o.ctid, o.xmin, o.tableoid FROM orders o").result.issues).toEqual([]);
    const state = stateOf("SELECT o._rowid FROM orders o", "mysql");
    expect(() => prepareColumnInspection(state, 7, context("mysql"))).toThrow(
      /single direct-table/,
    );
  });
  test("withholds identifiers the server can truncate and honors MySQL alias case policy", () => {
    const state = stateOf(`SELECT o.${"x".repeat(64)} FROM orders o`);
    expect(() => prepareColumnInspection(state, 7, context())).toThrow();
    const mysql = stateOf("SELECT O.typo FROM orders o", "mysql");
    expect(() =>
      prepareColumnInspection(mysql, 7, { ...context("mysql"), caseSensitiveTableNames: true }),
    ).toThrow(/unresolved qualifier/);
    expect(
      prepareColumnInspection(mysql, 7, {
        ...context("mysql"),
        caseSensitiveTableNames: false,
      }).finish(mysql, schema).issues,
    ).toHaveLength(1);
  });
});
