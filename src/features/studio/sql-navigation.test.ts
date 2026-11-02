import { describe, expect, test } from "vitest";
import { EditorState } from "@codemirror/state";
import { sql } from "@codemirror/lang-sql";
import type {
  EngineKind,
  ErrorLocation,
  TableInfo,
  TableSchema,
  DatabaseObjectSummary,
} from "../../ipc/generated";
import { appErrorOf } from "../../ipc/client";
import { selectedStatementSlice } from "./sql-context";
import { sqlDialect } from "./sql-identifiers";
import {
  editorDocument,
  adjacentStatement,
  resolveDocumentErrorRange,
  resolveExecutionError,
  statementRanges,
  resolveSymbolAt,
  schemaCacheKey,
  tablesNeededForNavigationAt,
  analyzeAliasRename,
  analyzeSqlRename,
  type SqlSymbolContext,
} from "./sql-navigation";

describe("scope-correct CTE rename", () => {
  const analyze = analyzeSqlRename;
  test.each(["postgres", "mysql", "snowflake"] as const)(
    "%s edits the declaration, relation names and implicit qualifiers but preserves explicit aliases",
    (engine) => {
      const state = stateWithCaret(
        "WITH |c(id) AS (SELECT id FROM public.orders) SELECT c.id, x.id FROM c JOIN c x ON c.id = x.id; -- c\nSELECT 'c.id'",
        engine,
      );
      const target = analyze(state, state.selection.main.head, engine);
      const name = engine === "snowflake" ? "ROWS_CTE" : "rows_cte";
      expect(state.update({ changes: target.preview(state, name) }).state.doc.toString()).toBe(
        `WITH ${name}(id) AS (SELECT id FROM public.orders) SELECT ${name}.id, x.id FROM ${name} JOIN ${name} x ON ${name}.id = x.id; -- c\nSELECT 'c.id'`,
      );
      expect(target.occurrences).toHaveLength(5);
    },
  );

  test.each([
    "WITH |c AS (SELECT 1 AS id) SELECT c.id FROM c",
    "WITH c AS (SELECT 1 AS id) SELECT |c.id FROM c",
    "WITH c AS (SELECT 1 AS id) SELECT x.id FROM |c x",
  ])("starts at a declaration, implicit qualifier or explicitly aliased relation: %s", (marked) => {
    const state = stateWithCaret(marked);
    const target = analyze(state, state.selection.main.head, "postgres");
    expect(
      state.update({ changes: target.preview(state, "renamed") }).state.doc.toString(),
    ).toContain("WITH renamed AS");
  });

  test("follows ordered WITH visibility and preserves later nested shadowing and physical tables", () => {
    const state = stateWithCaret(
      "WITH |c AS (SELECT id FROM c), d AS (SELECT c.id FROM c) SELECT c.id FROM c WHERE EXISTS (WITH prior_rows AS (SELECT c.id FROM c), c AS (SELECT id FROM prior_rows) SELECT c.id FROM c) AND EXISTS (SELECT c.id FROM public.c)",
    );
    const target = analyze(state, state.selection.main.head, "postgres");
    expect(state.update({ changes: target.preview(state, "renamed") }).state.doc.toString()).toBe(
      "WITH renamed AS (SELECT id FROM c), d AS (SELECT renamed.id FROM renamed) SELECT renamed.id FROM renamed WHERE EXISTS (WITH prior_rows AS (SELECT renamed.id FROM renamed), c AS (SELECT id FROM prior_rows) SELECT c.id FROM c) AND EXISTS (SELECT c.id FROM public.c)",
    );
  });

  test("renames an inner declaration while preserving the outer CTE in its body", () => {
    const state = stateWithCaret(
      "WITH c AS (SELECT 1 AS id) SELECT c.id FROM c WHERE EXISTS (WITH |c AS (SELECT c.id FROM c) SELECT c.id FROM c)",
    );
    const target = analyze(state, state.selection.main.head, "postgres");
    expect(
      state.update({ changes: target.preview(state, "local_rows") }).state.doc.toString(),
    ).toBe(
      "WITH c AS (SELECT 1 AS id) SELECT c.id FROM c WHERE EXISTS (WITH local_rows AS (SELECT c.id FROM c) SELECT local_rows.id FROM local_rows)",
    );
  });

  test.each([
    "WITH |c AS (SELECT 1 AS id) SELECT c.id FROM c WHERE EXISTS (WITH prior_rows AS (SELECT c.id FROM c), c AS (SELECT id FROM prior_rows) SELECT c.id FROM c)",
    "WITH c AS (SELECT 1 AS id) SELECT c.id FROM c WHERE EXISTS (WITH |c AS (SELECT c.id FROM c) SELECT c.id FROM c)",
    "WITH |c AS (SELECT 1 AS id) SELECT c.id FROM c WHERE EXISTS (WITH c AS (SELECT 2 AS id) SELECT c.id FROM c)",
    "WITH |c AS (SELECT 1 AS id) SELECT c.id FROM c WHERE EXISTS (WITH n AS (SELECT c.id FROM c) SELECT n.id FROM n)",
    "WITH c AS (SELECT 1 AS id) SELECT c.id FROM c WHERE EXISTS (WITH |n AS (SELECT c.id FROM c) SELECT n.id FROM n)",
  ])("withholds MySQL-family nested WITH under an outer CTE scope: %s", (marked) => {
    const state = stateWithCaret(marked, "mysql");
    expect(() => analyze(state, state.selection.main.head, "mysql")).toThrow(/cannot safely/);
  });

  test("retains ordered JOIN visibility for implicit CTE qualifiers", () => {
    const state = stateWithCaret(
      "WITH |c AS (SELECT 1 AS id) SELECT c.id FROM c WHERE EXISTS (SELECT c.id FROM orders x JOIN orders y ON y.id = c.id JOIN orders c ON c.id = x.id)",
    );
    const target = analyze(state, state.selection.main.head, "postgres");
    expect(state.update({ changes: target.preview(state, "renamed") }).state.doc.toString()).toBe(
      "WITH renamed AS (SELECT 1 AS id) SELECT renamed.id FROM renamed WHERE EXISTS (SELECT c.id FROM orders x JOIN orders y ON y.id = renamed.id JOIN orders c ON c.id = x.id)",
    );
    expect(() => target.preview(state, "y")).toThrow(/capture|conflict/i);
  });

  test("keeps explicit relation aliases as independently renameable symbols", () => {
    const state = stateWithCaret("WITH c AS (SELECT 1 AS id) SELECT c.id FROM c |c");
    const target = analyze(state, state.selection.main.head, "postgres");
    expect(target.kind).toBe("alias");
    expect(
      state.update({ changes: target.preview(state, "local_alias") }).state.doc.toString(),
    ).toBe("WITH c AS (SELECT 1 AS id) SELECT local_alias.id FROM c local_alias");
  });

  test("follows CTE uses in derived bodies and permits a declaration without uses", () => {
    const state = stateWithCaret(
      "WITH |c AS (SELECT 1 AS id) SELECT x.id FROM (SELECT c.id FROM c) x",
    );
    const target = analyze(state, state.selection.main.head, "postgres");
    expect(state.update({ changes: target.preview(state, "renamed") }).state.doc.toString()).toBe(
      "WITH renamed AS (SELECT 1 AS id) SELECT x.id FROM (SELECT renamed.id FROM renamed) x",
    );
    const unused = stateWithCaret("WITH |c AS (SELECT 1) SELECT 2");
    expect(analyze(unused, unused.selection.main.head, "postgres").occurrences).toHaveLength(1);
  });

  test("supports PostgreSQL materialization and preserves explicit CTE output lists", () => {
    for (const modifier of ["MATERIALIZED", "NOT MATERIALIZED"]) {
      const state = stateWithCaret(
        `WITH |c(value) AS ${modifier} (SELECT 1) SELECT c.value FROM c`,
      );
      const target = analyze(state, state.selection.main.head, "postgres");
      expect(
        state.update({ changes: target.preview(state, "new_rows") }).state.doc.toString(),
      ).toBe(`WITH new_rows(value) AS ${modifier} (SELECT 1) SELECT new_rows.value FROM new_rows`);
    }
  });

  test("keeps schema-qualified physical relations and their aliases unchanged", () => {
    const state = stateWithCaret(
      "WITH |c AS (SELECT 1 AS id) SELECT a.id, b.id FROM c a JOIN public.n b ON a.id = b.id",
    );
    const target = analyze(state, state.selection.main.head, "postgres");
    expect(state.update({ changes: target.preview(state, "n") }).state.doc.toString()).toBe(
      "WITH n AS (SELECT 1 AS id) SELECT a.id, b.id FROM n a JOIN public.n b ON a.id = b.id",
    );
  });

  test.each([
    "WITH |c AS (SELECT 1), n AS (SELECT 2) SELECT * FROM c",
    "WITH |c AS (SELECT 1 AS id) SELECT c.id FROM c JOIN orders n ON c.id = n.id",
    "WITH |c AS (SELECT 1 AS id) SELECT x.id FROM c x JOIN n y ON true",
    "WITH |c AS (SELECT 1 AS id) SELECT c.id FROM c WHERE EXISTS (WITH n AS (SELECT 2 AS id) SELECT c.id FROM c)",
    "WITH n AS (SELECT 1 AS id) SELECT n.id FROM n WHERE EXISTS (WITH |c AS (SELECT 2 AS id) SELECT n.id FROM n JOIN c ON true)",
    "WITH |c AS (SELECT 1 AS id) SELECT c.id, n.unknown FROM c",
    "WITH |c AS (SELECT 1 AS id) SELECT c.id, n FROM c",
  ])("rejects declaration, relation, qualifier and unresolved-name capture: %s", (marked) => {
    const state = stateWithCaret(marked);
    const target = analyze(state, state.selection.main.head, "postgres");
    expect(() => target.preview(state, "n")).toThrow(/capture|conflict/i);
  });

  test.each(["postgres", "mysql", "snowflake"] as const)(
    "%s preserves delimiters and identifier limits",
    (engine) => {
      const q = engine === "mysql" ? "`" : '"';
      const state = stateWithCaret(
        `WITH |${q}Old${q} AS (SELECT 1 AS id) SELECT ${q}Old${q}.id FROM ${q}Old${q}`,
        engine,
      );
      const target = analyze(state, state.selection.main.head, engine);
      const name = `New${q}Name`;
      expect(target.preview(state, name).map((x) => x.insert)).toEqual(
        Array(3).fill(`${q}New${q}${q}Name${q}`),
      );
      expect(() => target.preview(state, "Old")).toThrow(/different/);
      expect(() => target.preview(state, "bad\nname")).toThrow();
      if (engine !== "snowflake")
        expect(() => target.preview(state, "あ".repeat(65))).toThrow(/length/);
      const changed = state.update({ changes: { from: 0, insert: " " } }).state;
      expect(() => target.preview(changed, "new_name")).toThrow(/changed/);
    },
  );

  test.each([
    "WITH RECURSIVE |c AS (SELECT 1 AS id) SELECT c.id FROM c",
    "WITH |c AS (DELETE FROM orders RETURNING id) SELECT * FROM c",
    "WITH |c AS (SELECT 1) SELECT * FROM c UNION SELECT * FROM c",
    "WITH |c AS (SELECT 1 AS id) SELECT c FROM c",
    "WITH |c AS (SELECT 1 AS id) SELECT c.id FROM c JOIN LATERAL (SELECT 1) x ON true",
    "WITH |c AS (SELECT 1 AS id) SELECT c.id FROM c x(id)",
    "WITH |c AS (SELECT 1), c AS (SELECT 2) SELECT * FROM c",
  ])("withholds recursive, unsupported or incomplete semantics: %s", (marked) => {
    const state = stateWithCaret(marked);
    expect(() => analyze(state, state.selection.main.head, "postgres")).toThrow();
  });

  test("never returns a capped CTE edit list", () => {
    const state = stateWithCaret(
      `WITH |c AS (SELECT 1 AS id) SELECT c.id${", c.id".repeat(501)} FROM c`,
    );
    expect(() => analyze(state, state.selection.main.head, "postgres")).toThrow(/limit|large/i);
  });
});

describe("scope-correct alias rename", () => {
  test("previews only the declaration and qualified references, preserving comments and other statements", () => {
    const state = stateWithCaret(
      "SELECT |o.id, o.* FROM public.orders AS o WHERE o.id > 0; -- o.id\nSELECT 'o.id';",
    );
    const target = analyzeAliasRename(state, state.selection.main.head, "postgres");
    const changes = target.preview(state, "order_rows");
    expect(changes).toHaveLength(4);
    expect(state.update({ changes }).state.doc.toString()).toBe(
      "SELECT order_rows.id, order_rows.* FROM public.orders AS order_rows WHERE order_rows.id > 0; -- o.id\nSELECT 'o.id';",
    );
  });

  test("preserves nested shadowing and follows proven correlated references", () => {
    const state = stateWithCaret(
      "SELECT |o.id FROM orders o WHERE EXISTS (SELECT o.id FROM customers o WHERE o.id > 0) AND EXISTS (SELECT x.id FROM customers x WHERE x.id = o.id)",
    );
    const target = analyzeAliasRename(state, state.selection.main.head, "postgres");
    expect(
      state.update({ changes: target.preview(state, "orders_alias") }).state.doc.toString(),
    ).toBe(
      "SELECT orders_alias.id FROM orders orders_alias WHERE EXISTS (SELECT o.id FROM customers o WHERE o.id > 0) AND EXISTS (SELECT x.id FROM customers x WHERE x.id = orders_alias.id)",
    );
    expect(() => target.preview(state, "x")).toThrow(/capture|conflict/i);
  });

  test("keeps CTE and non-lateral derived bodies outside the containing FROM scope", () => {
    const state = stateWithCaret(
      "WITH c AS (SELECT o.id FROM orders o) SELECT |o.id FROM c o JOIN (SELECT o.id FROM other o) d ON d.id = o.id",
    );
    const target = analyzeAliasRename(state, state.selection.main.head, "postgres");
    expect(
      state.update({ changes: target.preview(state, "outer_alias") }).state.doc.toString(),
    ).toBe(
      "WITH c AS (SELECT o.id FROM orders o) SELECT outer_alias.id FROM c outer_alias JOIN (SELECT o.id FROM other o) d ON d.id = outer_alias.id",
    );
  });

  test.each(["postgres", "mysql", "snowflake"] as const)(
    "%s follows outer references in earlier ON clauses before a later alias shadows them",
    (engine) => {
      const state = stateWithCaret(
        "SELECT |o.id FROM orders o WHERE EXISTS (SELECT o.id FROM customers x JOIN orders y ON y.id = o.id JOIN orders o ON o.id = x.id WHERE o.id = y.id)",
        engine,
      );
      const target = analyzeAliasRename(state, state.selection.main.head, engine);
      expect(state.update({ changes: target.preview(state, "renamed") }).state.doc.toString()).toBe(
        "SELECT renamed.id FROM orders renamed WHERE EXISTS (SELECT o.id FROM customers x JOIN orders y ON y.id = renamed.id JOIN orders o ON o.id = x.id WHERE o.id = y.id)".replaceAll(
          "renamed",
          engine === "snowflake" ? '"renamed"' : "renamed",
        ),
      );
      expect(target.occurrences).toHaveLength(3);
      expect(() => target.preview(state, engine === "snowflake" ? "Y" : "y")).toThrow(
        /capture|conflict/i,
      );
    },
  );

  test("renames a later local declaration without adopting earlier correlated references", () => {
    const state = stateWithCaret(
      "SELECT o.id FROM orders o WHERE EXISTS (SELECT o.id FROM customers x JOIN orders y ON y.id = o.id JOIN orders |o ON o.id = x.id WHERE o.id = y.id)",
    );
    const target = analyzeAliasRename(state, state.selection.main.head, "postgres");
    expect(
      state.update({ changes: target.preview(state, "local_rows") }).state.doc.toString(),
    ).toBe(
      "SELECT o.id FROM orders o WHERE EXISTS (SELECT local_rows.id FROM customers x JOIN orders y ON y.id = o.id JOIN orders local_rows ON local_rows.id = x.id WHERE local_rows.id = y.id)",
    );
    expect(target.occurrences).toHaveLength(4);
  });

  test.each(["postgres", "mysql", "snowflake"] as const)(
    "%s uses JOIN operands rather than preceding comma groups in ON subqueries",
    (engine) => {
      const state = stateWithCaret(
        "SELECT |o.id FROM orders o WHERE EXISTS (SELECT o.id FROM orders o, customers x JOIN orders y ON EXISTS (SELECT 1 WHERE y.id = o.id) WHERE o.id = x.id)",
        engine,
      );
      const target = analyzeAliasRename(state, state.selection.main.head, engine);
      expect(state.update({ changes: target.preview(state, "renamed") }).state.doc.toString()).toBe(
        "SELECT renamed.id FROM orders renamed WHERE EXISTS (SELECT o.id FROM orders o, customers x JOIN orders y ON EXISTS (SELECT 1 WHERE y.id = renamed.id) WHERE o.id = x.id)".replaceAll(
          "renamed",
          engine === "snowflake" ? '"renamed"' : "renamed",
        ),
      );
    },
  );

  test("keeps unresolved later qualifiers unresolved and rejects replacement capture in earlier ON", () => {
    const state = stateWithCaret(
      "SELECT |o.id FROM orders o JOIN customers x ON x.id = n.id JOIN orders n ON n.id = o.id",
    );
    const target = analyzeAliasRename(state, state.selection.main.head, "postgres");
    expect(target.occurrences).toHaveLength(3);
    expect(() => target.preview(state, "n")).toThrow(/capture|conflict/i);
    expect(state.update({ changes: target.preview(state, "renamed") }).state.doc.toString()).toBe(
      "SELECT renamed.id FROM orders renamed JOIN customers x ON x.id = n.id JOIN orders n ON n.id = renamed.id",
    );
    const unbound = stateWithCaret(
      "SELECT |o.id FROM orders o, customers x JOIN orders y ON y.id = n.id",
    );
    const separate = analyzeAliasRename(unbound, unbound.selection.main.head, "postgres");
    // The comma-group alias remains outside this ON, including after replacement.
    expect(separate.preview(unbound, "n")).toHaveLength(2);
  });

  test("preserves nested JOIN association when ON clauses close pending joins", () => {
    const state = stateWithCaret(
      "SELECT |o.id FROM orders o WHERE EXISTS (SELECT o.id FROM orders o JOIN customers x JOIN orders y ON y.id = o.id ON x.id = o.id)",
    );
    const target = analyzeAliasRename(state, state.selection.main.head, "postgres");
    expect(state.update({ changes: target.preview(state, "renamed") }).state.doc.toString()).toBe(
      "SELECT renamed.id FROM orders renamed WHERE EXISTS (SELECT o.id FROM orders o JOIN customers x JOIN orders y ON y.id = renamed.id ON x.id = o.id)",
    );
  });

  test("retains CROSS/NATURAL/USING operands for later joins and restores full scope after FROM", () => {
    for (const join of [
      "CROSS JOIN customers x",
      "NATURAL JOIN customers x",
      "JOIN customers x USING (id)",
    ]) {
      const state = stateWithCaret(
        `SELECT |o.id FROM orders o ${join} LEFT JOIN orders y ON y.id = o.id WHERE o.id > 0 ORDER BY o.id`,
      );
      const target = analyzeAliasRename(state, state.selection.main.head, "postgres");
      expect(target.occurrences).toHaveLength(5);
      expect(state.update({ changes: target.preview(state, "renamed") }).state.doc.toString()).toBe(
        `SELECT renamed.id FROM orders renamed ${join} LEFT JOIN orders y ON y.id = renamed.id WHERE renamed.id > 0 ORDER BY renamed.id`,
      );
    }
  });

  test("supports MySQL CROSS JOIN conditions and keeps preceding CROSS joins left-associated", () => {
    for (const join of [
      "CROSS JOIN customers x ON x.id = o.id",
      "CROSS JOIN customers x USING (id)",
      "CROSS JOIN customers x JOIN orders y ON y.id = o.id",
    ]) {
      const state = stateWithCaret(`SELECT |o.id FROM orders o ${join}`, "mysql");
      const target = analyzeAliasRename(state, state.selection.main.head, "mysql");
      expect(state.update({ changes: target.preview(state, "renamed") }).state.doc.toString()).toBe(
        `SELECT renamed.id FROM orders renamed ${join.replaceAll("o.id", "renamed.id")}`,
      );
    }
  });

  test.each(["postgres", "mysql"] as const)(
    "%s keeps a completed CROSS JOIN's local shadow in a later ON",
    (engine) => {
      const state = stateWithCaret(
        "SELECT |o.id FROM orders o WHERE EXISTS (SELECT o.id FROM orders o CROSS JOIN customers x JOIN orders y ON y.id = o.id WHERE o.id = x.id)",
        engine,
      );
      const target = analyzeAliasRename(state, state.selection.main.head, engine);
      expect(state.update({ changes: target.preview(state, "renamed") }).state.doc.toString()).toBe(
        "SELECT renamed.id FROM orders renamed WHERE EXISTS (SELECT o.id FROM orders o CROSS JOIN customers x JOIN orders y ON y.id = o.id WHERE o.id = x.id)",
      );
      expect(target.occurrences).toHaveLength(2);
    },
  );

  test("rejects same-scope collisions, captured outer references and previously unbound qualifiers", () => {
    for (const marked of [
      "SELECT |o.id FROM orders o JOIN customers n ON o.id = n.id",
      "SELECT n.id FROM orders n WHERE EXISTS (SELECT |o.id FROM customers o WHERE o.id = n.id)",
      "SELECT |o.id, n.unknown FROM orders o",
    ]) {
      const state = stateWithCaret(marked);
      const target = analyzeAliasRename(state, state.selection.main.head, "postgres");
      expect(() => target.preview(state, "n")).toThrow(/capture|conflict/i);
    }
  });

  test.each(["postgres", "mysql", "snowflake"] as const)(
    "preserves %s quoting and escapes replacement delimiters",
    (engine) => {
      const quote = engine === "mysql" ? "`" : '"';
      const state = stateWithCaret(
        `SELECT |${quote}Old${quote}.id FROM orders ${quote}Old${quote}`,
        engine,
      );
      const target = analyzeAliasRename(state, state.selection.main.head, engine);
      const name = `New${quote}Name`;
      const changes = target.preview(state, name);
      expect(changes.map((x) => x.insert)).toEqual([
        `${quote}New${quote}${quote}Name${quote}`,
        `${quote}New${quote}${quote}Name${quote}`,
      ]);
    },
  );

  test("rejects document changes even when text is restored, invalid names and no-op renames", () => {
    const state = stateWithCaret("SELECT |o.id FROM orders o");
    const target = analyzeAliasRename(state, state.selection.main.head, "postgres");
    const changed = state.update({ changes: { from: 0, insert: " " } }).state;
    expect(() => target.preview(changed, "n")).toThrow(/changed/i);
    expect(() =>
      target.preview(changed.update({ changes: { from: 0, to: 1 } }).state, "n"),
    ).toThrow(/changed/i);
    for (const name of ["", "o", "bad\nname", "x".repeat(129)])
      expect(() => target.preview(state, name)).toThrow();
    expect(target.preview(state, "select").map((x) => x.insert)).toEqual(['"select"', '"select"']);
  });

  test.each([
    "SELECT |o FROM orders o",
    "SELECT |o.id FROM orders o JOIN customers o ON true",
    "SELECT |o.id FROM orders o UNION SELECT x.id FROM orders x",
    "SELECT |o.id FROM orders o JOIN LATERAL (SELECT o.id) x ON true",
    "SELECT |o.id FROM generate_series(1, 2) o",
    "WITH removed AS (DELETE FROM orders o RETURNING o.id) SELECT |o.id FROM orders o",
    "SELECT |o.id FROM orders o JOIN (customers c JOIN orders x ON c.id = x.id) j ON true",
    "SELECT |o.id FROM orders o JOIN customers x USING (id) AS joined_columns",
    "UPDATE orders |o SET id = 1",
    "SELECT o.|id FROM orders o",
    "SELECT '|o.id' FROM orders o",
    "SELECT |o.id FROM orders o WHERE (",
  ])("withholds unsupported or ambiguous source: %s", (marked) => {
    const state = stateWithCaret(marked);
    expect(() => analyzeAliasRename(state, state.selection.main.head, "postgres")).toThrow();
  });

  test("never offers a truncated edit list", () => {
    const state = stateWithCaret(`SELECT |o.id${", o.id".repeat(501)} FROM orders o`);
    expect(() => analyzeAliasRename(state, state.selection.main.head, "postgres")).toThrow(
      /limit|large/i,
    );
  });

  test("rejects names the database would truncate and conservatively folds MySQL aliases", () => {
    const pg = stateWithCaret("SELECT |o.id FROM orders o");
    const target = analyzeAliasRename(pg, pg.selection.main.head, "postgres");
    expect(() => target.preview(pg, "あ".repeat(22))).toThrow(/length limit/);
    const mysql = stateWithCaret(
      "SELECT |`Old`.id FROM orders `Old` JOIN customers `Other` ON Old.id = Other.id",
      "mysql",
    );
    const alias = analyzeAliasRename(mysql, mysql.selection.main.head, "mysql");
    expect(() => alias.preview(mysql, "OTHER")).toThrow(/conflict/);
    expect(() => alias.preview(mysql, "x".repeat(65))).toThrow(/length limit/);
    expect(alias.preview(mysql, "new_name")).toHaveLength(3);
  });
});

function stateWithCaret(text: string, engine: EngineKind = "postgres") {
  const position = text.indexOf("|");
  const clean = position >= 0 ? text.slice(0, position) + text.slice(position + 1) : text;
  return EditorState.create({
    doc: clean,
    selection: { anchor: position < 0 ? clean.length : position },
    extensions: [sql({ dialect: sqlDialect(engine) })],
  });
}

describe("statementRanges and adjacentStatement", () => {
  test("ignores semicolons inside strings, quoted identifiers, dollar-quotes, and comments", () => {
    const doc = [
      "-- leading comment; not a statement",
      "SELECT 'a;b' AS \"col;1\", $$inner;stmt;$$ AS body;",
      ";;",
      "/* block; comment */",
      "SELECT 2 FROM public.orders;",
      "SELECT 3",
    ].join("\n");
    const state = stateWithCaret(doc, "postgres");
    const ranges = statementRanges(state);
    expect(ranges).toHaveLength(3);
    expect(state.sliceDoc(ranges[0].from, ranges[0].to)).toBe(
      "SELECT 'a;b' AS \"col;1\", $$inner;stmt;$$ AS body;",
    );
    expect(state.sliceDoc(ranges[1].from, ranges[1].to)).toBe("SELECT 2 FROM public.orders;");
    expect(state.sliceDoc(ranges[2].from, ranges[2].to)).toBe("SELECT 3");
  });

  test("navigates forward and backward across multi-statement scripts", () => {
    const first = "SELECT 1;\n";
    const second = "SELECT 2;\n";
    const third = "SELECT 3;";
    const doc = `${first}${second}${third}`;

    const atFirstMid = stateWithCaret(`SELECT |1;\n${second}${third}`);
    expect(adjacentStatement(atFirstMid, -1)).toBeNull();
    expect(adjacentStatement(atFirstMid, 1)?.from).toBe(first.length);

    const atSecondMid = stateWithCaret(`${first}SELECT |2;\n${third}`);
    expect(adjacentStatement(atSecondMid, -1)?.from).toBe(0);
    expect(adjacentStatement(atSecondMid, 1)?.from).toBe(first.length + second.length);

    const atBetween = EditorState.create({
      doc: "SELECT 1;\n\nSELECT 2;",
      selection: { anchor: 10 },
      extensions: [sql({ dialect: sqlDialect("postgres") })],
    });
    expect(adjacentStatement(atBetween, -1)?.from).toBe(0);
    expect(adjacentStatement(atBetween, 1)?.from).toBe(11);

    const atTrailing = EditorState.create({
      doc: `${doc}\n\n`,
      selection: { anchor: doc.length + 2 },
      extensions: [sql({ dialect: sqlDialect("postgres") })],
    });
    expect(adjacentStatement(atTrailing, 1)).toBeNull();
    expect(adjacentStatement(atTrailing, -1)?.from).toBe(first.length + second.length);
  });
});

describe("selectedStatementSlice", () => {
  test("returns exact UTF-16 start offset for current statement and explicit selection", () => {
    const doc = "SELECT 1;\nSELECT '🧪', bad_col FROM public.orders;";
    const secondStart = doc.indexOf("SELECT '🧪'");

    const byCaret = EditorState.create({
      doc,
      selection: { anchor: secondStart + 8 },
      extensions: [sql({ dialect: sqlDialect("postgres") })],
    });
    const stmtSlice = selectedStatementSlice(byCaret);
    expect(stmtSlice.fromUtf16).toBe(secondStart);
    expect(stmtSlice.sql).toBe("SELECT '🧪', bad_col FROM public.orders;");

    const bySelection = EditorState.create({
      doc,
      selection: { anchor: secondStart, head: doc.length - 1 },
      extensions: [sql({ dialect: sqlDialect("postgres") })],
    });
    const selSlice = selectedStatementSlice(bySelection);
    expect(selSlice.fromUtf16).toBe(secondStart);
    expect(selSlice.sql).toBe("SELECT '🧪', bad_col FROM public.orders");
  });
});

describe("resolveDocumentErrorRange and appErrorOf", () => {
  test("maps UTF-16 spans with emoji surrogate pairs, CRLF, and selection offsetUtf16", () => {
    const prefix = "SELECT 1;\r\n";
    const subSql = "SELECT '🧪🧪',日本語,\r\n  missing_col FROM orders";
    const doc = `${prefix}${subSql}`;
    const relStart = subSql.indexOf("missing_col");
    const relEnd = relStart + "missing_col".length;

    const location: ErrorLocation = {
      span: {
        startUtf16: relStart,
        endUtf16: relEnd,
        line: 2,
        column: 3,
      },
      sqlState: "42703",
    };

    const resolved = resolveDocumentErrorRange(doc, location, prefix.length);
    expect(doc.slice(resolved.from, resolved.to)).toBe("missing_col");
    expect(resolved.line).toBe(3);
    expect(resolved.column).toBe(3);
  });

  test("counts Unicode scalars rather than UTF-16 code units for column display and avoids splitting surrogate pairs", () => {
    const doc = "SELECT '🧪', bad";
    // '🧪' is 2 UTF-16 code units at indices 8..10, but 1 Unicode scalar column.
    const badStart = doc.indexOf("bad");
    const resolved = resolveDocumentErrorRange(doc, {
      span: {
        startUtf16: badStart,
        endUtf16: badStart + 3,
        line: 1,
        column: 13,
      },
    });
    expect(doc.slice(resolved.from, resolved.to)).toBe("bad");
    expect(resolved.line).toBe(1);
    expect(resolved.column).toBe(13);

    // Mid-surrogate offset (index 9 inside '🧪' at 8..10) clamps cleanly to surrogate boundaries.
    const clamped = resolveDocumentErrorRange(doc, {
      span: {
        startUtf16: 9,
        endUtf16: 9,
        line: 1,
        column: 9,
      },
    });
    expect(clamped.from).toBe(8);
    expect(clamped.to).toBe(10);
    expect(doc.slice(clamped.from, clamped.to)).toBe("🧪");
  });

  test("expands zero-width EOF spans to a non-empty mark range when document is non-empty", () => {
    const doc = "SELECT FROM";
    const resolved = resolveDocumentErrorRange(doc, {
      span: {
        startUtf16: doc.length,
        endUtf16: doc.length,
        line: 1,
        column: doc.length + 1,
      },
    });
    expect(resolved.to).toBeGreaterThan(resolved.from);
    expect(resolved.to).toBe(doc.length);
  });

  test("appErrorOf preserves structured AppError location and falls back safely", () => {
    const structured = {
      code: "policyBlocked" as const,
      message: "Expected expression",
      location: {
        span: { startUtf16: 7, endUtf16: 11, line: 1, column: 8 },
        sqlState: "42601",
      },
    };
    expect(appErrorOf(structured)).toEqual(structured);
    expect(appErrorOf(new Error("boom"))).toEqual({ code: "database", message: "boom" });
  });
});

describe("execution source identity", () => {
  const query = "SELECT :value, missing_column";
  const prefix = "SELECT 1;\r\n-- 🧪 日本語\r\n";
  const document = prefix + query;
  const origin = { document, query, fromUtf16: prefix.length };
  const startUtf16 = query.indexOf("missing_column");
  const error = {
    code: "database" as const,
    message: "missing_column",
    location: {
      span: { startUtf16, endUtf16: query.length, line: 1, column: startUtf16 + 1 },
    },
  };

  test("maps original placeholder coordinates without applying Explain prefixes twice", () => {
    const range = resolveExecutionError(document, error, origin)!;
    expect(editorDocument(document).slice(range.from, range.to)).toBe("missing_column");
    expect(range.line).toBe(3);
    expect(range.column).toBe(startUtf16 + 1);
  });
  test("maps CRLF and bare CR file coordinates into CodeMirror newline units", () => {
    const query = "SELECT 1;\r\nSELECT missing_column;\rSELECT 3;";
    const startUtf16 = query.indexOf("missing_column");
    const input = {
      ...error,
      location: { span: { startUtf16, endUtf16: startUtf16 + 14, line: 2, column: 8 } },
    };
    const origin = { document: query, query, fromUtf16: 0 };
    for (const document of [query, editorDocument(query)]) {
      const range = resolveExecutionError(document, input, origin)!;
      expect(editorDocument(document).slice(range.from, range.to)).toBe("missing_column");
      expect(range.line).toBe(2);
      expect(range.column).toBe(8);
    }
  });
  test("rejects edited documents, absent origins and mismatched slices", () => {
    expect(resolveExecutionError(document + " ", error, origin)).toBeNull();
    expect(resolveExecutionError(document, error)).toBeNull();
    expect(resolveExecutionError(document, error, { ...origin, fromUtf16: 0 })).toBeNull();
  });
  test("rejects malformed and out of slice spans instead of marking unrelated SQL", () => {
    for (const [startUtf16, endUtf16] of [
      [-1, 3],
      [NaN, 3],
      [Infinity, 4],
      [0, query.length + 1],
      [4, 2],
      [1.5, 2],
    ]) {
      expect(
        resolveExecutionError(
          document,
          {
            ...error,
            location: {
              span: {
                startUtf16,
                endUtf16,
                line: 1,
                column: 1,
              },
            },
          },
          origin,
        ),
      ).toBeNull();
    }
  });
  test("keeps EOF marker expansion inside the submitted selection", () => {
    const origin = { document: "SELECT 1; x\nSELECT 2", query: "x", fromUtf16: 10 };
    const eof = {
      ...error,
      location: { span: { startUtf16: 1, endUtf16: 1, line: 1, column: 2 } },
    };
    const range = resolveExecutionError(origin.document, eof, origin)!;
    expect(origin.document.slice(range.from, range.to)).toBe("x");
  });
});

function samplePgSymbolContext(): SqlSymbolContext {
  const tables: TableInfo[] = [
    {
      table: { schema: "public", name: "orders" },
      kind: "table",
      estimatedRows: 420,
      totalBytes: 16384,
    },
    {
      table: { schema: "public", name: "customers" },
      kind: "table",
      estimatedRows: 120,
      totalBytes: 8192,
    },
    {
      table: { schema: "public", name: "active_orders" },
      kind: "view",
      estimatedRows: null,
      totalBytes: null,
    },
  ];

  const ordersSchema: TableSchema = {
    table: { schema: "public", name: "orders" },
    columns: [
      {
        id: "id",
        name: "id",
        kind: "integer",
        databaseType: "int8",
        nullable: false,
        primaryKey: true,
        foreignKey: false,
        editable: false,
        insertable: true,
        hasDefault: true,
      },
      {
        id: "customer_id",
        name: "customer_id",
        kind: "integer",
        databaseType: "int8",
        nullable: false,
        primaryKey: false,
        foreignKey: true,
        editable: true,
        insertable: true,
        hasDefault: false,
      },
      {
        id: "status",
        name: "status",
        kind: "text",
        databaseType: "text",
        nullable: false,
        primaryKey: false,
        foreignKey: false,
        editable: true,
        insertable: true,
        hasDefault: true,
      },
    ],
    primaryKey: ["id"],
    foreignKeys: [
      {
        name: "orders_customer_id_fkey",
        columns: ["customer_id"],
        target: { schema: "public", name: "customers" },
        targetColumns: ["id"],
      },
    ],
    inboundForeignKeys: [],
    editable: true,
  };

  const customersSchema: TableSchema = {
    table: { schema: "public", name: "customers" },
    columns: [
      {
        id: "id",
        name: "id",
        kind: "integer",
        databaseType: "int8",
        nullable: false,
        primaryKey: true,
        foreignKey: false,
        editable: false,
        insertable: true,
        hasDefault: true,
      },
      {
        id: "name",
        name: "name",
        kind: "text",
        databaseType: "text",
        nullable: false,
        primaryKey: false,
        foreignKey: false,
        editable: true,
        insertable: true,
        hasDefault: false,
      },
    ],
    primaryKey: ["id"],
    foreignKeys: [],
    inboundForeignKeys: [],
    editable: true,
  };

  const schemas = new Map<string, TableSchema>([
    [schemaCacheKey({ schema: "public", name: "orders" }), ordersSchema],
    [schemaCacheKey({ schema: "public", name: "customers" }), customersSchema],
  ]);

  const databaseObjects: DatabaseObjectSummary[] = [
    {
      object: { schema: "public", name: "orders", kind: "table", identity: null },
      ownerTable: null,
      signature: null,
      estimatedRows: 420,
      totalBytes: 16384,
    },
    {
      object: { schema: "public", name: "active_orders", kind: "view", identity: null },
      ownerTable: null,
      signature: null,
      estimatedRows: null,
      totalBytes: null,
    },
    {
      object: {
        schema: "public",
        name: "calc_tax",
        kind: "function",
        identity: "numeric",
      },
      ownerTable: null,
      signature: "(amount numeric)",
      estimatedRows: null,
      totalBytes: null,
    },
    {
      object: { schema: "public", name: "order_seq", kind: "sequence", identity: null },
      ownerTable: null,
      signature: null,
      estimatedRows: null,
      totalBytes: null,
    },
  ];

  return {
    engine: "postgres",
    tables,
    databaseObjects,
    schemas,
    searchPath: ["public"],
  };
}

describe("resolveSymbolAt and tablesNeededForNavigationAt", () => {
  test("resolves named CTE outputs to their local declaration and proven source", () => {
    const state = stateWithCaret(
      "WITH recent AS (SELECT o.id AS order_id FROM orders o) SELECT r.|order_id FROM recent r WHERE r.order_id > 0;",
    );
    const symbol = resolveSymbolAt(state, state.selection.main.head, samplePgSymbolContext());
    expect(symbol).toMatchObject({
      kind: "projection",
      name: "order_id",
      relationName: "recent",
      sourceTable: { schema: "public", name: "orders" },
      sourceColumn: { name: "id", databaseType: "int8" },
      declarationSpan: { from: 31, to: 39 },
    });
    expect(symbol?.occurrences).toHaveLength(3);
    expect(symbol?.occurrences.filter((o) => o.isDeclaration)).toHaveLength(1);
    expect(
      tablesNeededForNavigationAt(state, state.selection.main.head, {
        ...samplePgSymbolContext(),
        schemas: new Map(),
      }),
    ).toEqual([{ schema: "public", name: "orders" }]);
  });

  test.each([
    ["WITH recent AS (SELECT id FROM orders) SELECT recent.|id FROM recent;", "id", "recent", "id"],
    [
      "WITH recent(order_id) AS (SELECT id FROM orders) SELECT |order_id FROM recent;",
      "order_id",
      "recent",
      "id",
    ],
    [
      "WITH recent(order_id) AS (SELECT id AS ignored FROM orders) SELECT r.|order_id FROM recent r;",
      "order_id",
      "recent",
      "id",
    ],
    ["SELECT d.|order_id FROM (SELECT id order_id FROM orders) d;", "order_id", "d", "id"],
    ["SELECT |order_id FROM (SELECT o.id AS order_id FROM orders o) d;", "order_id", "d", "id"],
    [
      "WITH a AS (SELECT id AS order_id FROM orders), b AS (SELECT order_id AS result_id FROM a) SELECT b.|result_id FROM b;",
      "result_id",
      "b",
      "id",
    ],
    [
      "WITH a AS (SELECT id FROM orders) SELECT d.|result_id FROM (SELECT a.id AS result_id FROM a) d;",
      "result_id",
      "d",
      "id",
    ],
    [
      'WITH "Recent"("OrderID") AS (SELECT id FROM orders) SELECT r.|"OrderID" FROM "Recent" r;',
      "OrderID",
      "Recent",
      "id",
    ],
    [
      "WITH a AS (SELECT status FROM orders) SELECT |status FROM a JOIN customers c ON true;",
      "status",
      "a",
      "status",
    ],
    [
      "WITH a AS (SELECT id AS |order_id FROM orders) SELECT a.order_id FROM a;",
      "order_id",
      "a",
      "id",
    ],
    ["SELECT d.order_id FROM (SELECT id AS |order_id FROM orders) d;", "order_id", "d", "id"],
    [
      "WITH a(|order_id) AS (SELECT id FROM orders) SELECT a.order_id FROM a;",
      "order_id",
      "a",
      "id",
    ],
  ])("resolves the unique named output in %s", (query, name, relationName, sourceName) => {
    const state = stateWithCaret(query);
    const symbol = resolveSymbolAt(state, state.selection.main.head, samplePgSymbolContext());
    expect(symbol).toMatchObject({
      kind: "projection",
      name,
      relationName,
      sourceTable: { schema: "public", name: "orders" },
      sourceColumn: { name: sourceName },
    });
    if (symbol?.kind !== "projection") throw new Error("Expected named output");
    expect(symbol.occurrences.some((o) => o.isDeclaration)).toBe(true);
    expect(
      state.sliceDoc(symbol.declarationSpan.from, symbol.declarationSpan.to).replaceAll('"', ""),
    ).toBe(name);
  });

  test.each([
    "WITH a AS (SELECT id AS x, customer_id AS x FROM orders) SELECT a.|x FROM a;",
    "SELECT |id FROM (SELECT id FROM orders) a JOIN customers c ON true;",
    "WITH a AS (SELECT id + 1 AS x FROM orders) SELECT a.|x FROM a;",
    "WITH a(x) AS (SELECT id + 1 FROM orders) SELECT a.|x FROM a;",
    "WITH a AS (SELECT * FROM orders) SELECT a.|id FROM a;",
    "WITH a AS (SELECT o.* FROM orders o) SELECT a.|id FROM a;",
    "WITH a AS (SELECT id FROM orders UNION SELECT id FROM customers) SELECT a.|id FROM a;",
    "WITH RECURSIVE a(id) AS (SELECT id FROM a) SELECT a.|id FROM a;",
    "WITH a AS (SELECT id FROM b), b AS (SELECT id FROM orders) SELECT a.|id FROM a;",
    "WITH a(x) AS (SELECT id, status FROM orders) SELECT a.|x FROM a;",
    "SELECT d.|id FROM (SELECT id FROM orders) d(renamed);",
    "SELECT d.|id FROM generate_series(1, 3) d;",
    'WITH a AS (SELECT id AS "OrderID" FROM orders) SELECT a.|orderid FROM a;',
    "WITH a AS (SELECT missing FROM orders) SELECT a.|missing FROM a;",
    "SELECT |id FROM orders o JOIN (SELECT id FROM unavailable) d ON true;",
  ])("withholds ambiguous or unsupported output in %s", (query) => {
    const state = stateWithCaret(query);
    expect(resolveSymbolAt(state, state.selection.main.head, samplePgSymbolContext())).toBeNull();
  });

  test("keeps nested output declarations distinct and does not leak a shadowed catalog name", () => {
    const state = stateWithCaret(
      "WITH a AS (SELECT id AS x FROM orders) SELECT a.|x FROM a WHERE EXISTS (WITH a AS (SELECT customer_id AS x FROM orders) SELECT a.x FROM a);",
    );
    const symbol = resolveSymbolAt(state, state.selection.main.head, samplePgSymbolContext());
    expect(symbol).toMatchObject({ kind: "projection", sourceColumn: { name: "id" } });
    expect(symbol?.occurrences).toHaveLength(2);
    const innerPos = state.doc.toString().lastIndexOf("a.x") + 2;
    const inner = resolveSymbolAt(state, innerPos, samplePgSymbolContext());
    expect(inner).toMatchObject({ kind: "projection", sourceColumn: { name: "customer_id" } });
    expect(inner?.occurrences).toHaveLength(2);

    const shadowed = stateWithCaret(
      'SELECT "public".|orders FROM (SELECT missing FROM customers) AS "public";',
    );
    expect(
      resolveSymbolAt(shadowed, shadowed.selection.main.head, samplePgSymbolContext()),
    ).toBeNull();
  });

  test("requires current source schemas and bounds nested projection inference", () => {
    const ctx = samplePgSymbolContext();
    const state = stateWithCaret("WITH a AS (SELECT id FROM orders) SELECT a.|id FROM a;");
    expect(
      resolveSymbolAt(state, state.selection.main.head, { ...ctx, schemas: new Map() }),
    ).toBeNull();
    const chain = Array.from(
      { length: 20 },
      (_, i) => `a${i} AS (SELECT id FROM ${i ? `a${i - 1}` : "orders"})`,
    ).join(", ");
    const deep = stateWithCaret(`WITH ${chain} SELECT a19.|id FROM a19;`);
    expect(resolveSymbolAt(deep, deep.selection.main.head, ctx)).toBeNull();
  });

  test("honors MySQL case-insensitive output names independently of table casing", () => {
    const ctx = {
      ...samplePgSymbolContext(),
      engine: "mysql" as const,
      caseSensitiveTableNames: true,
    };
    const state = stateWithCaret(
      "WITH recent AS (SELECT `ID` AS `OrderID` FROM public.orders) SELECT recent.|orderid FROM recent;",
      "mysql",
    );
    expect(resolveSymbolAt(state, state.selection.main.head, ctx)).toMatchObject({
      kind: "projection",
      name: "OrderID",
      sourceColumn: { name: "id" },
    });
  });

  test("resolves CTE declarations and references with declaration span and local occurrences", () => {
    const ctx = samplePgSymbolContext();
    const doc =
      "WITH recent_orders(order_id, status) AS (SELECT id, status FROM public.orders)\nSELECT r.order_id FROM recent_|orders AS r;";
    const state = stateWithCaret(doc, "postgres");
    const pos = state.selection.main.head;
    const symbol = resolveSymbolAt(state, pos, ctx);

    expect(symbol?.kind).toBe("cte");
    if (symbol?.kind === "cte") {
      expect(symbol.name).toBe("recent_orders");
      expect(symbol.columns).toEqual(["order_id", "status"]);
      expect(state.sliceDoc(symbol.declarationSpan.from, symbol.declarationSpan.to)).toBe(
        "recent_orders",
      );
      expect(symbol.occurrences).toHaveLength(2);
      expect(symbol.occurrences[0].isDeclaration).toBe(true);
      expect(symbol.occurrences[0].line).toBe(1);
      expect(symbol.occurrences[1].isDeclaration).toBe(false);
      expect(symbol.occurrences[1].line).toBe(2);
    }
  });

  test("resolves table aliases, qualified columns, and keyword-named bare columns", () => {
    const ctx = samplePgSymbolContext();

    const aliasState = stateWithCaret(
      "SELECT |o.customer_id, o.status FROM public.orders AS o WHERE o.status = 'paid';",
    );
    const aliasSym = resolveSymbolAt(aliasState, aliasState.selection.main.head, ctx);
    expect(aliasSym?.kind).toBe("alias");
    if (aliasSym?.kind === "alias") {
      expect(aliasSym.name).toBe("o");
      expect(aliasSym.table).toEqual({ schema: "public", name: "orders" });
      expect(aliasState.sliceDoc(aliasSym.declarationSpan.from, aliasSym.declarationSpan.to)).toBe(
        "o",
      );
      expect(aliasSym.occurrences).toHaveLength(4);
      expect(aliasSym.occurrences.filter((occ) => occ.isDeclaration)).toHaveLength(1);
    }

    const qualColState = stateWithCaret(
      "SELECT o.customer|_id, o.status FROM public.orders AS o WHERE customer_id > 10;",
    );
    const qualCol = resolveSymbolAt(qualColState, qualColState.selection.main.head, ctx);
    expect(qualCol?.kind).toBe("column");
    if (qualCol?.kind === "column") {
      expect(qualCol.name).toBe("customer_id");
      expect(qualCol.table).toEqual({ schema: "public", name: "orders" });
      expect(qualCol.column?.foreignKey).toBe(true);
      expect(qualCol.occurrences).toHaveLength(2);
    }

    // "status" lexes as Keyword in @codemirror/lang-sql; ensure bare column resolution still succeeds.
    const bareKeywordColState = stateWithCaret(
      "SELECT id, stat|us FROM orders WHERE status = 'paid';",
    );
    const bareCol = resolveSymbolAt(
      bareKeywordColState,
      bareKeywordColState.selection.main.head,
      ctx,
    );
    expect(bareCol?.kind).toBe("column");
    if (bareCol?.kind === "column") {
      expect(bareCol.name).toBe("status");
      expect(bareCol.column?.databaseType).toBe("text");
      expect(bareCol.occurrences).toHaveLength(2);
    }
  });

  test("resolves catalog tables across multi-statement scripts, views, routines, and sequences", () => {
    const ctx = samplePgSymbolContext();

    const multiTableState = stateWithCaret(
      "SELECT * FROM ord|ers;\nSELECT count(*) FROM public.orders;",
    );
    const needed = tablesNeededForNavigationAt(
      multiTableState,
      multiTableState.selection.main.head,
      ctx,
    );
    expect(needed).toContainEqual({ schema: "public", name: "orders" });

    const tableSym = resolveSymbolAt(multiTableState, multiTableState.selection.main.head, ctx);
    expect(tableSym?.kind).toBe("object");
    if (tableSym?.kind === "object") {
      expect(tableSym.object).toEqual({
        schema: "public",
        name: "orders",
        kind: "table",
        identity: null,
      });
      expect(tableSym.estimatedRows).toBe(420);
      expect(tableSym.occurrences).toHaveLength(2);
    }

    const viewState = stateWithCaret("SELECT * FROM active_|orders;");
    const viewSym = resolveSymbolAt(viewState, viewState.selection.main.head, ctx);
    expect(viewSym?.kind).toBe("object");
    if (viewSym?.kind === "object") {
      expect(viewSym.object.kind).toBe("view");
      expect(viewSym.object.name).toBe("active_orders");
    }

    const fnState = stateWithCaret("SELECT calc|_tax(12.50), public.calc_tax(99.00);");
    const fnSym = resolveSymbolAt(fnState, fnState.selection.main.head, ctx);
    expect(fnSym?.kind).toBe("object");
    if (fnSym?.kind === "object") {
      expect(fnSym.object.kind).toBe("function");
      expect(fnSym.object.name).toBe("calc_tax");
      expect(fnSym.summary?.signature).toBe("(amount numeric)");
      expect(fnSym.occurrences).toHaveLength(2);
    }

    const seqState = stateWithCaret("SELECT nextval('x'), order|_seq;");
    const seqSym = resolveSymbolAt(seqState, seqState.selection.main.head, ctx);
    expect(seqSym?.kind).toBe("object");
    if (seqSym?.kind === "object") {
      expect(seqSym.object.kind).toBe("sequence");
      expect(seqSym.object.name).toBe("order_seq");
    }
  });

  test("ignores comments, string literals, reserved SQL clauses, and ambiguous bare columns", () => {
    const ctx = samplePgSymbolContext();

    const inString = stateWithCaret("SELECT 'ord|ers' FROM orders;");
    expect(resolveSymbolAt(inString, inString.selection.main.head, ctx)).toBeNull();

    const inComment = stateWithCaret("-- ord|ers\nSELECT * FROM orders;");
    expect(resolveSymbolAt(inComment, inComment.selection.main.head, ctx)).toBeNull();

    const onSelect = stateWithCaret("SEL|ECT * FROM orders;");
    expect(resolveSymbolAt(onSelect, onSelect.selection.main.head, ctx)).toBeNull();

    // Both orders and customers define "id", so an unqualified "id" is ambiguous and must not guess.
    const ambiguousCol = stateWithCaret(
      "SELECT i|d FROM orders o JOIN customers c ON o.customer_id = c.id;",
    );
    expect(resolveSymbolAt(ambiguousCol, ambiguousCol.selection.main.head, ctx)).toBeNull();
  });
});

describe("SQL symbol scope and identity", () => {
  const resolve = (text: string, ctx = samplePgSymbolContext()) => {
    const state = stateWithCaret(text, ctx.engine);
    return { state, symbol: resolveSymbolAt(state, state.selection.main.head, ctx) };
  };
  test("keeps alias usages separate from nested aliases and same-named columns", () => {
    const { symbol, state } = resolve(
      "SELECT |o.id, (SELECT o.id FROM customers o) FROM orders o WHERE o.status = 'paid';",
    );
    expect(symbol?.kind).toBe("alias");
    expect(symbol?.occurrences.map((o) => state.sliceDoc(o.from, o.to))).toEqual(["o", "o", "o"]);
    expect(symbol?.occurrences.map((o) => o.from)).toEqual([
      7,
      state.doc.toString().indexOf("orders o") + 7,
      state.doc.toString().indexOf("WHERE o") + 6,
    ]);
    const bare = resolve("SELECT |o, o.id FROM orders o;");
    expect(bare.symbol).toBeNull();
  });
  test("keeps column usages in their resolved scopes including correlated references", () => {
    const { symbol, state } = resolve(
      "SELECT o.|id, (SELECT o.id FROM customers o WHERE o.id > 0), (SELECT c.id FROM customers c WHERE c.id = o.id) FROM orders o;",
    );
    expect(symbol?.kind).toBe("column");
    expect(symbol?.occurrences.map((o) => o.from)).toEqual([
      9,
      state.doc.toString().lastIndexOf("o.id") + 2,
    ]);
    const correlated = resolve(
      "SELECT (SELECT c.id FROM customers c WHERE c.id = o.|id) FROM orders o;",
    );
    expect(correlated.symbol?.kind).toBe("column");
    if (correlated.symbol?.kind === "column") expect(correlated.symbol.table.name).toBe("orders");
  });
  test("excludes CTE and alias shadows from catalog object usages", () => {
    const { symbol, state } = resolve(
      "SELECT * FROM |orders; WITH orders AS (SELECT * FROM customers) SELECT * FROM orders; SELECT orders.id FROM customers orders;",
    );
    expect(symbol?.kind).toBe("object");
    expect(symbol?.occurrences.map((o) => state.sliceDoc(o.from, o.to))).toEqual(["orders"]);
  });
  test("keeps nested CTE identities separate", () => {
    const { symbol } = resolve(
      "WITH c AS (SELECT * FROM orders) SELECT * FROM |c WHERE EXISTS (WITH c AS (SELECT * FROM customers) SELECT * FROM c);",
    );
    expect(symbol?.kind).toBe("cte");
    expect(symbol?.occurrences).toHaveLength(2);
  });
  test("does not invent columns or resolve ambiguous/incomplete bare bindings", () => {
    expect(resolve("SELECT o.|missing FROM orders o;").symbol).toBeNull();
    expect(resolve("SELECT |id FROM orders o JOIN orders p ON o.id = p.id;").symbol).toBeNull();
    expect(
      resolve("SELECT |status FROM orders o JOIN (SELECT status FROM customers) d ON true;").symbol,
    ).toBeNull();
    const ctx = samplePgSymbolContext();
    const schemas = new Map(ctx.schemas);
    schemas.delete(schemaCacheKey({ schema: "public", name: "customers" }));
    expect(
      resolve("SELECT |id FROM orders o JOIN customers c ON true;", { ...ctx, schemas }).symbol,
    ).toBeNull();
  });
  test("does not guess routine overloads or use routines outside the live search path", () => {
    const ctx = samplePgSymbolContext();
    const routine = ctx.databaseObjects!.find((o) => o.object.name === "calc_tax")!;
    const overload = { ...routine, object: { ...routine.object, identity: "integer" } };
    expect(
      resolve("SELECT public.calc_|tax(1);", { ...ctx, databaseObjects: [routine, overload] })
        .symbol,
    ).toBeNull();
    expect(resolve("SELECT calc_|tax(1);", { ...ctx, searchPath: ["private"] }).symbol).toBeNull();
  });
  test("honors MySQL table case mode but matches quoted columns without case sensitivity", () => {
    const pg = samplePgSymbolContext();
    const ctx = { ...pg, engine: "mysql" as const, caseSensitiveTableNames: true };
    expect(resolve("SELECT * FROM public.Ord|ers;", ctx).symbol).toBeNull();
    const { symbol } = resolve("SELECT o.`I|D` FROM public.orders o;", ctx);
    expect(symbol?.kind).toBe("column");
    if (symbol?.kind === "column") expect(symbol.name).toBe("id");
  });
});

test("does not conflate output aliases with base columns or catalog names", () => {
  const ctx = samplePgSymbolContext();
  for (const text of [
    "SELECT customer_id AS |id FROM orders;",
    "SELECT customer_id AS id FROM orders ORDER BY |id;",
    "SELECT |orders FROM customers;",
  ]) {
    const state = stateWithCaret(text);
    expect(resolveSymbolAt(state, state.selection.main.head, ctx)).toBeNull();
  }
});

test("bounds local usage results explicitly and preserves Unicode locations", () => {
  const text = "SELECT '🧪日本語', * FROM |orders;\n" + "SELECT * FROM orders;\n".repeat(504);
  const state = stateWithCaret(text);
  const symbol = resolveSymbolAt(state, state.selection.main.head, samplePgSymbolContext());
  expect(symbol?.occurrences).toHaveLength(500);
  expect(symbol?.usagesLimited).toBe(true);
  expect(symbol?.occurrences[0].column).toBe(23);
});
