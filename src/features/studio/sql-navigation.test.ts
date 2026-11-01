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
  type SqlSymbolContext,
} from "./sql-navigation";

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
