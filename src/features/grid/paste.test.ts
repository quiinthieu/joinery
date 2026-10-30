import { describe, expect, it } from "vitest";
import type { ColumnInfo } from "../../ipc/generated";
import { parseTsvClipboard, validatePasteMatrix } from "./paste";

function col(
  id: string,
  name: string,
  kind: ColumnInfo["kind"],
  options?: {
    nullable?: boolean;
    editable?: boolean;
    insertable?: boolean;
    hasDefault?: boolean;
  },
): ColumnInfo {
  return {
    id,
    name,
    databaseType: kind,
    kind,
    nullable: options?.nullable ?? true,
    primaryKey: false,
    foreignKey: false,
    editable: options?.editable ?? true,
    insertable: options?.insertable ?? options?.editable ?? true,
    hasDefault: options?.hasDefault ?? false,
  };
}

const visibleColumns: ColumnInfo[] = [
  col("c2", "email", "text", { nullable: true }),
  col("c1", "name", "text", { nullable: false }),
  col("c3", "active", "boolean", { nullable: false }),
  col("c4", "score", "integer", { nullable: true }),
  col("c5", "metadata", "json", { nullable: true }),
];

const fixtureRows: Record<string, string | null>[] = [
  { c1: "Customer 1", c2: "c1@example.test", c3: "true", c4: "10", c5: '{"tier":"starter"}' },
  { c1: "Customer 2", c2: "c2@example.test", c3: "false", c4: null, c5: null },
];

describe("paste", () => {
  it("parses spreadsheet TSV with trailing newline and quoted multiline cells", () => {
    expect(parseTsvClipboard("a\tb\r\nc\td\r\n")).toEqual([
      ["a", "b"],
      ["c", "d"],
    ]);
    expect(parseTsvClipboard('"hello\nworld"\t"say ""hi"""\t\\N\n')).toEqual([
      ["hello\nworld", 'say "hi"', "\\N"],
    ]);
    expect(parseTsvClipboard("")).toEqual([[""]]);
  });

  it("maps pasted rectangles across visible reordered columns and coerces NULL/types", () => {
    const outcome = validatePasteMatrix({
      anchor: [0, 0],
      values: [
        ["aria@example.test", "Aria", "0", "42", '{"tier":"pro"}'],
        ["\\N", "Bram", "t", "", ""],
      ],
      visibleColumns,
      tableEditable: true,
      resolveRow: (row) =>
        row < fixtureRows.length
          ? {
              kind: "existing",
              resultId: "res-1",
              row,
              readOriginal: (id) => fixtureRows[row][id],
            }
          : { kind: "outOfBounds" },
    });
    expect(outcome.ok).toBe(true);
    if (!outcome.ok) return;
    expect(outcome.cells).toHaveLength(10);
    // Visual col 0 maps to schema column c2 ("email"), visual col 1 maps to c1 ("name").
    expect(outcome.cells[0]).toMatchObject({ row: 0, column: "c2", value: "aria@example.test" });
    expect(outcome.cells[1]).toMatchObject({ row: 0, column: "c1", value: "Aria" });
    expect(outcome.cells[2]).toMatchObject({ row: 0, column: "c3", value: "false" });
    expect(outcome.cells[5]).toMatchObject({ row: 1, column: "c2", value: null });
    expect(outcome.cells[8]).toMatchObject({ row: 1, column: "c4", value: null });
    expect(outcome.cells[9]).toMatchObject({ row: 1, column: "c5", value: null });
  });

  it("rejects mixed valid/invalid paste batches with exact row and column diagnostics", () => {
    const resolveRow = (row: number) =>
      row < fixtureRows.length
        ? {
            kind: "existing" as const,
            resultId: "res-1",
            row,
            readOriginal: (id: string) => fixtureRows[row][id],
          }
        : { kind: "outOfBounds" as const };

    // NULL into NOT NULL column on row 2
    expect(
      validatePasteMatrix({
        anchor: [1, 0],
        values: [["Valid Name"], ["\\N"]],
        visibleColumns,
        tableEditable: true,
        resolveRow,
      }),
    ).toEqual({
      ok: false,
      error: 'Row 2, column "name" does not allow NULL.',
    });

    // Invalid integer
    expect(
      validatePasteMatrix({
        anchor: [3, 0],
        values: [["100"], ["12.5"]],
        visibleColumns,
        tableEditable: true,
        resolveRow,
      }),
    ).toEqual({
      ok: false,
      error: 'Row 2, column "score" expects an exact integer.',
    });

    // Invalid JSON
    expect(
      validatePasteMatrix({
        anchor: [4, 0],
        values: [['{"ok":true}'], ["{broken"]],
        visibleColumns,
        tableEditable: true,
        resolveRow,
      }),
    ).toEqual({
      ok: false,
      error: 'Row 2, column "metadata" expects valid JSON.',
    });

    // Read-only column
    const withReadonly = [{ ...visibleColumns[0], editable: false }, ...visibleColumns.slice(1)];
    expect(
      validatePasteMatrix({
        anchor: [0, 0],
        values: [["blocked"]],
        visibleColumns: withReadonly,
        tableEditable: true,
        resolveRow,
      }),
    ).toEqual({
      ok: false,
      error: 'Row 1, column "email" is read-only.',
    });

    // Exceeding 5,000 cells
    const huge = Array.from({ length: 1001 }, () => ["a", "b", "c", "d", "e"]);
    expect(
      validatePasteMatrix({
        anchor: [0, 0],
        values: huge,
        visibleColumns,
        tableEditable: true,
        resolveRow,
      }),
    ).toEqual({
      ok: false,
      error: "Paste is limited to 5,000 cells at once.",
    });
  });

  it("allows pasting insertable primary keys and default placeholders onto draft insert rows", () => {
    const insertCols: ColumnInfo[] = [
      col("c0", "id", "integer", {
        nullable: false,
        editable: false,
        insertable: true,
        hasDefault: false,
      }),
      col("c1", "name", "text", { nullable: false }),
      col("c3", "active", "boolean", { nullable: false, hasDefault: true }),
    ];
    const outcome = validatePasteMatrix({
      anchor: [0, 0],
      values: [["1001", "Ada Lovelace", ""]],
      visibleColumns: insertCols,
      tableEditable: true,
      resolveRow: () => ({ kind: "draftInsert", tabId: "tab-1", insertId: "ins-1" }),
    });
    expect(outcome).toEqual({
      ok: true,
      cells: [],
      insertCells: [
        { tabId: "tab-1", insertId: "ins-1", column: "c0", value: "1001" },
        { tabId: "tab-1", insertId: "ins-1", column: "c1", value: "Ada Lovelace" },
        { tabId: "tab-1", insertId: "ins-1", column: "c3", value: undefined },
      ],
    });
  });
});
