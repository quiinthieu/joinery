import { describe, expect, test } from "vitest";
import { EditorState } from "@codemirror/state";
import { sql } from "@codemirror/lang-sql";
import type { EngineKind, ErrorLocation } from "../../ipc/generated";
import { appErrorOf } from "../../ipc/client";
import { selectedStatementSlice } from "./sql-context";
import { sqlDialect } from "./sql-identifiers";
import {
  editorDocument,
  adjacentStatement,
  resolveDocumentErrorRange,
  resolveExecutionError,
  statementRanges,
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
