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
    result: prepareColumnInspection(state, 7, context(engine)).finish(state, [schema]),
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
    "WITH RECURSIVE c AS (SELECT id FROM orders) SELECT c.id FROM c",
    "WITH c(id) AS (SELECT id FROM orders) SELECT c.id FROM c",
    "WITH c AS (SELECT id FROM orders), d AS (SELECT id FROM orders) SELECT c.id FROM c",
    "WITH c AS (SELECT * FROM orders) SELECT c.id FROM c",
    "WITH c AS (SELECT id FROM orders a JOIN orders b ON a.id = b.id) SELECT c.id FROM c",
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
      expect(() => plan.finish(state, [bad])).toThrow(/metadata is unavailable/);
    }
    expect(() =>
      plan.finish(state.update({ changes: { from: 0, insert: " " } }).state, [schema]),
    ).toThrow(/SQL changed/);
  });
  test("does not treat qualified function names as columns", () => {
    expect(inspect("SELECT public.abs(o.id) FROM orders o").result.issues).toEqual([]);
  });
  test("recognizes PostgreSQL system columns and withholds MySQL implicit row identifiers", () => {
    expect(inspect("SELECT o.ctid, o.xmin, o.tableoid FROM orders o").result.issues).toEqual([]);
    const state = stateOf("SELECT o._rowid FROM orders o", "mysql");
    expect(() => prepareColumnInspection(state, 7, context("mysql"))).toThrow(
      /direct-table SELECT/,
    );
  });
  test("inspects one unqualified select-list column against its single owner", () => {
    const bare = inspect("SELECT typo FROM orders").result;
    expect(bare.issues.map((issue) => [issue.qualifier, issue.name, issue.outputOf])).toEqual([
      ["orders", "typo", undefined],
    ]);
    expect(bare.columnsFor(bare.issues[0])).toContain("customer_id");
    expect(bare.preview(stateOf("SELECT typo FROM orders"), bare.issues[0], "id").insert).toBe(
      '"id"',
    );
    const aliased = inspect("SELECT typo AS keep FROM orders o").result;
    expect(aliased.issues.map((issue) => issue.qualifier)).toEqual(["o"]);
    expect(
      stateOf("SELECT typo AS keep FROM orders o")
        .update({
          changes: aliased.preview(
            stateOf("SELECT typo AS keep FROM orders o"),
            aliased.issues[0],
            "id",
          ),
        })
        .state.doc.toString(),
    ).toBe('SELECT "id" AS keep FROM orders o');
    const derived = inspect("SELECT typo FROM (SELECT id FROM orders) d").result;
    expect(derived.issues.map((issue) => [issue.outputOf, issue.name])).toEqual([["d", "typo"]]);
    expect(derived.columnsFor(derived.issues[0])).toEqual(["id"]);
    expect(inspect("SELECT id FROM (SELECT id FROM orders) d").result.issues).toEqual([]);
    const state = stateOf("SELECT _rowid FROM orders", "mysql");
    expect(() => prepareColumnInspection(state, 7, context("mysql"))).toThrow(
      /direct-table SELECT/,
    );
  });
  test("inspects an unqualified name in WHERE, GROUP BY, HAVING and ORDER BY", () => {
    const where = inspect("SELECT o.id FROM orders o WHERE typo = 1").result;
    expect(where.issues.map((issue) => [issue.qualifier, issue.name])).toEqual([["o", "typo"]]);
    expect(where.columnsFor(where.issues[0])).toContain("id");
    expect(
      inspect("SELECT o.id FROM orders o GROUP BY typo").result.issues.map((i) => i.name),
    ).toEqual(["typo"]);
    expect(
      inspect("SELECT o.id FROM orders o HAVING typo > 1").result.issues.map((i) => i.name),
    ).toEqual(["typo"]);
    expect(
      inspect("SELECT o.id FROM orders o ORDER BY typo").result.issues.map((i) => i.name),
    ).toEqual(["typo"]);
    expect(inspect("SELECT o.id FROM orders o WHERE id = 1 ORDER BY id").result.issues).toEqual([]);
    expect(
      inspect("SELECT o.id FROM orders o WHERE LOWER(typo) = 1").result.issues.map((i) => i.name),
    ).toEqual(["typo"]);
    expect(inspect("SELECT LOWER(typo) FROM orders").result.issues.map((i) => i.name)).toEqual([
      "typo",
    ]);
    expect(inspect("SELECT o.id FROM orders o WHERE CAST(id AS text) = '1'").result.issues).toEqual(
      [],
    );
    expect(inspect("SELECT (typo) FROM orders").result.issues.map((i) => i.name)).toEqual(["typo"]);
    expect(inspect("SELECT ((typo)) FROM orders").result.issues.map((i) => i.name)).toEqual([
      "typo",
    ]);
    expect(inspect("SELECT (id) FROM orders").result.issues).toEqual([]);
    expect(() => inspect("SELECT (SELECT id FROM orders) FROM orders")).toThrow(
      /direct-table SELECT/,
    );
    const rowid = stateOf("SELECT o.id FROM orders o WHERE _rowid = 1", "mysql");
    expect(() => prepareColumnInspection(rowid, 7, context("mysql"))).toThrow(
      /direct-table SELECT/,
    );
  });
  test("inspects one CTE or derived query by its projected names", () => {
    const cte = inspect("WITH c AS (SELECT id FROM orders) SELECT c.bad FROM c").result;
    expect(cte.tables).toEqual([table]);
    expect(cte.issues.map((issue) => issue.name)).toEqual(["bad"]);
    expect(cte.issues[0].outputOf).toBe("c");
    expect(cte.columnsFor(cte.issues[0])).toEqual(["id"]);
    const aliased = inspect(
      "WITH c AS (SELECT id AS order_id FROM orders) SELECT x.customer_id FROM c x",
    ).result;
    expect(aliased.issues[0].outputOf).toBe("x");
    expect(aliased.columnsFor(aliased.issues[0])).toEqual(["order_id"]);
    const derived = inspect("SELECT d.bad FROM (SELECT id FROM orders) d").result;
    expect(derived.columnsFor(derived.issues[0])).toEqual(["id"]);
    const nested = inspect(
      "SELECT a.bad FROM (SELECT b.id FROM (SELECT id FROM orders) b) a",
    ).result;
    expect(nested.columnsFor(nested.issues[0])).toEqual(["id"]);
    const joined = inspect(
      "WITH c AS (SELECT a.id FROM orders a JOIN orders b ON a.id = b.id) SELECT c.bad, c.id FROM c",
    ).result;
    expect(joined.checked).toBe(5);
    expect(joined.issues).toHaveLength(1);
    expect(joined.columnsFor(joined.issues[0])).toEqual(["id"]);
  });
  test("keeps base-table corrections for references inside the query body", () => {
    const { result } = inspect(
      "WITH c AS NOT MATERIALIZED (SELECT o.id FROM orders o WHERE o.typo > 0) SELECT c.bad FROM c",
    );
    expect(result.issues.map((issue) => [issue.name, issue.outputOf])).toEqual([
      ["typo", undefined],
      ["bad", "c"],
    ]);
    expect(result.columnsFor(result.issues[0])).toEqual(
      schema.columns.map((column) => column.name),
    );
    expect(result.columnsFor(result.issues[1])).toEqual(["id"]);
    const state = stateOf('WITH c AS (SELECT "Mixed" AS "Keep" FROM orders) SELECT c.keep FROM c');
    const quoted = prepareColumnInspection(state, 7, context()).finish(state, [schema]);
    expect(quoted.issues.map((issue) => issue.name)).toEqual(["keep"]);
    expect(quoted.columnsFor(quoted.issues[0])).toEqual(["Keep"]);
  });
  test("does not offer unprojected catalog or system columns through a CTE", () => {
    const hidden = inspect(
      "WITH c AS (SELECT id FROM orders) SELECT c.ctid, c.customer_id FROM c",
    ).result;
    expect(hidden.issues.map((issue) => issue.name)).toEqual(["ctid", "customer_id"]);
    expect(hidden.columnsFor(hidden.issues[0])).toEqual(["id"]);
    expect(
      inspect("WITH c AS (SELECT o.ctid FROM orders o) SELECT c.ctid FROM c").result.issues,
    ).toEqual([]);
    const state = stateOf("WITH c AS (SELECT missing FROM orders) SELECT c.missing FROM c");
    expect(() => prepareColumnInspection(state, 7, context()).finish(state, [schema])).toThrow(
      /projected column/,
    );
    const shadow = {
      ...context(),
      tables: [
        ...context().tables,
        {
          table: { schema: "public", name: "c" },
          kind: "table",
          estimatedRows: null,
          totalBytes: null,
        },
      ],
    };
    const plan = prepareColumnInspection(
      stateOf("WITH c AS (SELECT id FROM orders) SELECT c.bad FROM c"),
      7,
      shadow,
    );
    expect(plan.tables).toEqual([table]);
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
      }).finish(mysql, [schema]).issues,
    ).toHaveLength(1);
  });
});
