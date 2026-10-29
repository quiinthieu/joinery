import { expect, test } from "vitest";
import { EditorState } from "@codemirror/state";
import { sql } from "@codemirror/lang-sql";
import type { EngineKind, TableInfo, TableSchema } from "../../ipc/generated";
import {
  catalogCompletions,
  columnCompletions,
  completionTarget,
  referencedTables,
  selectedStatement,
  tableLookup,
} from "./sql-context";
import { quoteIdentifier, readIdentifier, sqlDialect } from "./sql-identifiers";

export function query(text: string, engine: EngineKind = "postgres") {
  const position = text.indexOf("|");
  return EditorState.create({
    doc: text.replace("|", ""),
    selection: { anchor: position < 0 ? text.length : position },
    extensions: [sql({ dialect: sqlDialect(engine) })],
  });
}
export const info = (name: string, schema = "public"): TableInfo => ({
  table: { schema, name },
  kind: "BASE TABLE",
  estimatedRows: null,
  totalBytes: null,
});
const catalog = [info("orders"), info("customers")];
const lookup = tableLookup(catalog, "postgres");
const refs = (text: string) => referencedTables(query(text), lookup).references;

test("live schema order wins and never falls back outside the effective path", () => {
  const tables = [info("orders"), info("orders", "alternate"), info("private", "hidden")];
  const lookup = tableLookup(tables, "postgres", "joinery", undefined, [
    "pg_catalog",
    "alternate",
    "public",
  ]);
  expect(lookup.resolve([readIdentifier("orders")])).toEqual(tables[1].table);
  expect(lookup.resolve([readIdentifier("private")])).toBeUndefined();
  expect(lookup.resolve([readIdentifier("public"), readIdentifier("orders")])).toEqual(
    tables[0].table,
  );
});

test("server resolution preserves system names and inaccessible relation shadows", () => {
  const tables = [info("pg_type"), info("pg_type", "pg_catalog"), info("orders", "alternate")];
  const lookup = tableLookup(
    tables,
    "postgres",
    undefined,
    undefined,
    ["pg_catalog", "public", "alternate"],
    [tables[1].table],
  );
  expect(lookup.resolve([readIdentifier("pg_type")])).toEqual(tables[1].table);
  expect(lookup.resolve([readIdentifier("orders")])).toBeUndefined();
  expect(lookup.resolve([readIdentifier("alternate"), readIdentifier("orders")])).toEqual(
    tables[2].table,
  );
});

test("MySQL live table lookup follows server case rules and database.table paths", () => {
  const tables = [info("CaseOnly", "completion_alt")];
  const strict = tableLookup(
    tables,
    "mysql",
    "completion_alt",
    undefined,
    ["completion_alt"],
    [tables[0].table],
    true,
  );
  expect(strict.resolve([readIdentifier("caseonly")])).toBeUndefined();
  expect(strict.resolve([readIdentifier("CaseOnly")])).toEqual(tables[0].table);
  const folded = tableLookup(
    tables,
    "mysql",
    "completion_alt",
    undefined,
    ["completion_alt"],
    [tables[0].table],
    false,
  );
  expect(folded.resolve([readIdentifier("caseonly")])).toEqual(tables[0].table);
  expect(folded.resolve([readIdentifier("COMPLETION_ALT"), readIdentifier("CASEONLY")])).toEqual(
    tables[0].table,
  );
  expect(catalogCompletions(tables, folded).every((item) => (item.path?.length ?? 0) <= 2)).toBe(
    true,
  );
  expect(
    folded.resolve([
      readIdentifier("completion_alt"),
      readIdentifier("completion_alt"),
      readIdentifier("CaseOnly"),
    ]),
  ).toBeUndefined();
});

test("runs only the statement under the cursor and respects quoted semicolons", () => {
  expect(selectedStatement(query("-- context\nSELECT $$hé|llo; there$$; SELECT 2;"))).toBe(
    "SELECT $$héllo; there$$;",
  );
  expect(selectedStatement(query("SELECT 1; SELECT 'it''s; sa|fe';"))).toBe(
    "SELECT 'it''s; safe';",
  );
  expect(selectedStatement(query("SELECT 1; SELECT 3;|"))).toBe("SELECT 3;");
  const state = query("SELECT 1; SELECT 2;");
  expect(selectedStatement(state.update({ selection: { anchor: 0, head: 8 } }).state)).toBe(
    "SELECT 1",
  );
  expect(selectedStatement(query("SELECT $$one;|two$$; SELECT 2;", "snowflake"))).toBe(
    "SELECT $$one;two$$;",
  );
});

test.each(["postgres", "mysql", "snowflake"] as const)(
  "%s ignores comments and string literals",
  (engine) => {
    const state = query(
      "SELECT 'FROM orders o', 'it''s JOIN orders x' /* FROM orders y */ -- FROM orders z\n|",
      engine,
    );
    expect(referencedTables(state, tableLookup(catalog, engine)).references).toEqual([]);
    expect(completionTarget(query("SELECT 'FROM ord|ers'", engine), 16)).toBeNull();
  },
);
test.each(["postgres", "snowflake"] as const)(
  "%s ignores dollar strings including semicolons",
  (engine) => {
    expect(
      referencedTables(
        query("SELECT $$FROM orders o; JOIN orders x$$|", engine),
        tableLookup(catalog, engine),
      ).references,
    ).toEqual([]);
    const state = query("SELECT $$FROM ord|ers$$", engine);
    expect(completionTarget(state, state.selection.main.head)).toBeNull();
  },
);

test("isolates the caret statement, nested query block and UNION arm", () => {
  expect(
    refs("SELECT o. FROM orders o; SELECT o.| FROM customers o").map((r) => r.table.name),
  ).toEqual(["customers"]);
  expect(
    refs("SELECT o. FROM orders o WHERE EXISTS (SELECT c.| FROM customers c)").map(
      (r) => r.table.name,
    ),
  ).toEqual(["customers"]);
  expect(
    refs("SELECT o.| FROM orders o WHERE EXISTS (SELECT c.id FROM customers c)").map(
      (r) => r.table.name,
    ),
  ).toEqual(["orders"]);
  expect(
    refs("SELECT o. FROM orders o UNION ALL SELECT c.| FROM customers c").map((r) => r.table.name),
  ).toEqual(["customers"]);
  expect(
    refs("SELECT o.| FROM orders o UNION SELECT c.id FROM customers c").map((r) => r.table.name),
  ).toEqual(["orders"]);
  expect(
    refs("SELECT coalesce((SELECT c.| FROM customers c), 1) FROM orders o").map(
      (r) => r.table.name,
    ),
  ).toEqual(["customers"]);
});

test("CTEs and derived tables shadow physical tables, including in nested blocks", () => {
  expect(refs("WITH orders AS (SELECT 1 AS synthetic) SELECT o.| FROM orders o")).toEqual([]);
  expect(
    refs(
      "WITH RECURSIVE orders(x) AS (SELECT 1), customers AS (SELECT 2) SELECT c.| FROM customers c",
    ),
  ).toEqual([]);
  expect(
    refs("WITH orders AS (SELECT 1) SELECT (SELECT o.| FROM orders o) FROM customers c"),
  ).toEqual([]);
  expect(refs("SELECT orders.| FROM (SELECT 1 AS synthetic) AS orders")).toEqual([]);
  expect(refs("SELECT o.| FROM orders(1) AS o")).toEqual([]);
  expect(
    refs("WITH orders AS (SELECT 1) SELECT o.| FROM public.orders o").map((r) => r.table.name),
  ).toEqual(["orders"]);
  expect(
    refs("WITH orders AS (SELECT c.| FROM customers c) SELECT * FROM orders").map(
      (r) => r.table.name,
    ),
  ).toEqual(["customers"]);
});

test("finds comma and explicit joins, rejects duplicate aliases and preserves aliases", () => {
  const result = refs("SELECT o.| FROM orders AS o, customers c WHERE o.id=c.id");
  expect(result.map((r) => r.table.name)).toEqual(["orders", "customers"]);
  expect(result[0].qualifiers).toEqual([{ path: ["o"], sql: ["o"] }]);
  expect(refs("SELECT o.| FROM orders o JOIN customers o ON true")).toEqual([]);
  expect(refs("SELECT o.| FROM orders o")).toEqual(
    refs("SELECT o.|, 12 FROM orders o WHERE id=12"),
  );
});

test("resolves quoted case twins, embedded dots and delimiters without collisions", () => {
  const tables = [info("orders"), info("Orders"), info("public.orders"), info('odd"table', "a.b")];
  const resolve = (text: string) =>
    referencedTables(query(text), tableLookup(tables, "postgres")).references[0]?.table;
  expect(resolve('SELECT o.| FROM "Orders" o')).toEqual(tables[1].table);
  expect(resolve("SELECT o.| FROM ORDERS o")).toEqual(tables[0].table);
  expect(resolve('SELECT o.| FROM "public.orders" o')).toEqual(tables[2].table);
  expect(resolve('SELECT "o""x".| FROM "a.b"."odd""table" AS "o""x"')).toEqual(tables[3].table);
  const mysql = info("odd`table", "a.b");
  const scope = referencedTables(
    query("SELECT `o``x`.| FROM `a.b`.`odd``table` AS `o``x`", "mysql"),
    tableLookup([mysql], "mysql"),
  );
  expect(scope.references).toEqual([
    { table: mysql.table, qualifiers: [{ path: ["o`x"], sql: ["`o``x`"] }] },
  ]);
});

test("requires qualification for ambiguous schemas and respects configured defaults", () => {
  const tables = [info("orders", "other"), info("orders")];
  expect(tableLookup(tables, "postgres").resolve([readIdentifier("orders")])).toEqual(
    tables[1].table,
  );
  expect(
    tableLookup(tables, "postgres", undefined, "absent").resolve([readIdentifier("orders")]),
  ).toBeUndefined();
  const snow = [info("ORDERS", "STAGING"), info("ORDERS", "PROD")];
  const snowLookup = tableLookup(snow, "snowflake", "DWH", "PROD");
  expect(snowLookup.resolve([readIdentifier("orders")])).toEqual(snow[1].table);
  expect(
    snowLookup.resolve([readIdentifier("dwh"), readIdentifier("prod"), readIdentifier("orders")]),
  ).toEqual(snow[1].table);
  expect(snowLookup.resolve([readIdentifier('"orders"')])).toBeUndefined();
  const twins = [info("Orders"), info("orders")];
  expect(tableLookup(twins, "mysql").resolve([readIdentifier("Orders")])).toEqual(twins[0].table);
  expect(tableLookup(twins, "mysql").resolve([readIdentifier("ORDERS")])).toBeUndefined();
  expect(tableLookup([twins[0]], "mysql").resolve([readIdentifier("ORDERS")])).toEqual(
    twins[0].table,
  );
});

test("catalog preserves case twins and structured paths", () => {
  const tables = [info("ORDERS", "PROD"), info("Orders", "Prod")];
  const items = catalogCompletions(tables, tableLookup(tables, "snowflake", "DWH"));
  expect(items.filter((i) => i.type === "namespace").map((i) => i.path)).toEqual([
    ["PROD"],
    ["DWH", "PROD"],
    ["Prod"],
    ["DWH", "Prod"],
    ["DWH"],
  ]);
  expect(items).toContainEqual({
    label: "DWH.PROD.ORDERS",
    path: ["DWH", "PROD", "ORDERS"],
    type: "type",
    detail: "table",
  });
});

test("catalog requires qualification for ambiguous short table names", () => {
  const tables = [info("orders", "a"), info("orders", "b")];
  const items = catalogCompletions(tables, tableLookup(tables, "postgres"));
  expect(items.some((item) => item.type === "type" && item.path?.length === 1)).toBe(false);
  expect(items.filter((item) => item.type === "type").map((item) => item.path)).toEqual([
    ["a", "orders"],
    ["b", "orders"],
  ]);
});

test("an unfinished nested query never inherits outer bindings", () => {
  expect(refs("SELECT o.id FROM orders o WHERE EXISTS (SELECT c.|")).toEqual([]);
});

test("does not infer renamed relation outputs or mixed INSERT SELECT scopes", () => {
  expect(refs("SELECT o.| FROM orders AS o(renamed)")).toEqual([]);
  expect(refs("INSERT INTO orders SELECT c.| FROM customers c")).toEqual([]);
  expect(refs("SELECT o.id FROM orders o UNION SELECT c.id FROM customers c ORDER BY c.|")).toEqual(
    [],
  );
});

test("DML references and unfinished qualified names remain conservative", () => {
  expect(refs("DELETE FROM orders o USING customers c WHERE c.|").map((r) => r.table.name)).toEqual(
    ["orders", "customers"],
  );
  expect(refs("UPDATE orders o SET notes=o.|").map((r) => r.table.name)).toEqual(["orders"]);
  const fake = tableLookup([info("u"), info("public")], "postgres");
  expect(referencedTables(query('SELECT x.| FROM U&"escaped" x'), fake).references).toEqual([]);
  expect(referencedTables(query("SELECT x.| FROM public."), fake).references).toEqual([]);
});

test("same-name tables remain available through distinct schema qualifiers", () => {
  const lookup = tableLookup([info("orders", "a"), info("orders", "b")], "postgres");
  const scope = referencedTables(
    query("SELECT a.orders.| FROM a.orders JOIN b.orders ON true"),
    lookup,
  );
  expect(scope.references.map((ref) => ref.qualifiers.map((q) => q.path))).toEqual([
    [["a", "orders"]],
    [["b", "orders"]],
  ]);
  expect(scope.allowBare).toBe(false);
});

test.each([
  ['SELECT "a.b" . "No|tes" FROM orders "a.b"', "postgres", ["a.b", "No"], '"a.b" . "Notes"'],
  ["SELECT `o``x`.`na|me` FROM orders `o``x`", "mysql", ["o`x", "na"], "`o``x`.`name`"],
  ["SELECT 你.列| FROM orders 你", "postgres", ["你", "列"], "你.列"],
  ["SELECT o.| FROM orders o", "postgres", ["o", ""], "o."],
] as const)("prefix ranges preserve the rest of %s", (text, engine, values, replaced) => {
  const state = query(text, engine);
  const target = completionTarget(state, state.selection.main.head)!;
  expect(target.parts.map((p) => p.value)).toEqual(values);
  expect(state.sliceDoc(target.from, target.to)).toBe(replaced);
  const edited = state
    .update({ changes: { from: target.from, to: target.to, insert: "accepted" } })
    .state.doc.toString();
  expect(edited).toBe(state.doc.toString().replace(replaced, "accepted"));
});

test("quotes reserved, mixed-case and delimiter-containing identifiers for each dialect", () => {
  expect(quoteIdentifier("orders", "postgres")).toBe("orders");
  expect(quoteIdentifier("Orders", "postgres")).toBe('"Orders"');
  expect(quoteIdentifier("select", "postgres")).toBe('"select"');
  expect(quoteIdentifier("odd`table", "mysql")).toBe("`odd``table`");
  expect(quoteIdentifier("ORDERS", "snowflake")).toBe("ORDERS");
  expect(quoteIdentifier("Orders", "snowflake")).toBe('"Orders"');
});

test("bounds references and declines oversized or procedural blocks", () => {
  const tables = Array.from({ length: 12 }, (_, i) => info(`table_${i}`));
  const scope = referencedTables(
    query(`SELECT t0.| FROM ${tables.map((t, i) => `${t.table.name} t${i}`).join(", ")}`),
    tableLookup(tables, "postgres"),
  );
  expect(scope.references).toHaveLength(8);
  expect(scope.allowBare).toBe(false);
  expect(refs(`SELECT o.|, '${"x".repeat(65536)}' FROM orders o`)).toEqual([]);
  expect(refs("DO $$BEGIN SELECT o.| FROM orders o; END$$")).toEqual([]);
});

test("partial metadata retains qualified columns but never invents ambiguous bare columns", () => {
  const scope = referencedTables(
    query("SELECT o.| FROM orders o JOIN customers c ON true"),
    lookup,
  );
  const schema = {
    table: catalog[0].table,
    columns: [{ name: "Notes", databaseType: "text" }],
  } as TableSchema;
  const items = columnCompletions(scope, [schema, null], "postgres");
  expect(items.map((i) => i.apply)).toEqual(['o."Notes"']);
  const both = columnCompletions(
    scope,
    [schema, { ...schema, table: catalog[1].table }],
    "postgres",
  );
  expect(both.map((i) => i.apply)).toEqual(['o."Notes"', 'c."Notes"']);
});
