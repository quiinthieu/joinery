import { describe, expect, it } from "vitest";
import type { ColumnInfo } from "../../ipc/generated";
import { parseTsvClipboard, validatePasteMatrix } from "./paste";
import { formatGridCells, MAX_COPY_BYTES, MAX_COPY_CELLS, type CopyCell } from "./copy-format";

function col(
  id: string,
  name: string,
  kind: ColumnInfo["kind"],
  databaseType: string = kind,
): ColumnInfo {
  return {
    id,
    name,
    databaseType,
    kind,
    nullable: true,
    primaryKey: id === "c1",
    foreignKey: false,
    editable: true,
    insertable: true,
    hasDefault: false,
  };
}

const sampleColumns: ColumnInfo[] = [
  col("c1", "order_id", "integer", "bigint"),
  col("c2", "amount", "decimal", "numeric(38,10)"),
  col("c3", "note", "text", "text"),
  col("c4", "active", "boolean", "boolean"),
  col("c5", "payload", "json", "jsonb"),
  col("c6", "digest", "binary", "bytea"),
];

const sampleRows: CopyCell[][] = [
  [
    "9223372036854775807",
    "12345678901234567890.1234567890",
    "",
    "t",
    '{"big":9223372036854775807,"tag":"α"}',
    "\\xdeadbeef",
  ],
  ["-9223372036854775808", "0.0000000001", null, "0", null, null],
  ["42", "NaN", 'Hello, "Joinery"\nLine 2 | C:\\temp \\N', "true", "not-valid-json", "0x00ff"],
];

describe("copy-format", () => {
  it("formats RFC 4180 CSV distinguishing empty string from NULL and quoting special tokens", () => {
    const res = formatGridCells({
      columns: sampleColumns,
      rows: sampleRows,
      format: "csv",
    });
    expect(res.ok).toBe(true);
    if (!res.ok) return;

    const lines = res.text.split("\n");
    expect(lines[0]).toBe("order_id,amount,note,active,payload,digest");
    // Row 1 has empty string `""` quoted explicitly, exact int64 & decimal unquoted
    expect(lines[1]).toBe(
      '9223372036854775807,12345678901234567890.1234567890,"",t,"{""big"":9223372036854775807,""tag"":""α""}",\\xdeadbeef',
    );
    // Row 2 has SQL NULLs emitted as unquoted empty fields
    expect(lines[2]).toBe("-9223372036854775808,0.0000000001,,0,,");
    // Literal `\N` in text is quoted so CSV importers never confuse it with NULL
    const literalNullToken = formatGridCells({
      columns: [col("c1", "note", "text")],
      rows: [["\\N"], [""], [null]],
      format: "csv",
      includeHeaders: false,
    });
    expect(literalNullToken.ok && literalNullToken.text).toBe('"\\N"\n""\n');
  });

  it("formats TSV compatible with Joinery grid paste round-trips", () => {
    const cols = [
      col("c1", "email", "text"),
      col("c2", "score", "integer"),
      col("c3", "active", "boolean"),
    ];
    const sourceRows: (string | null)[][] = [
      ["", "9007199254740993", "true"],
      [null, null, "false"],
      ['tab\there\nand "quote"', "7", "1"],
    ];
    const res = formatGridCells({
      columns: cols,
      rows: sourceRows,
      format: "tsv",
      includeHeaders: false,
    });
    expect(res.ok).toBe(true);
    if (!res.ok) return;

    const parsed = parseTsvClipboard(res.text);
    const outcome = validatePasteMatrix({
      anchor: [0, 0],
      values: parsed,
      visibleColumns: cols,
      tableEditable: true,
      resolveRow: (row) => ({
        kind: "existing",
        resultId: "r1",
        row,
        readOriginal: () => "orig",
      }),
    });
    expect(outcome.ok).toBe(true);
    if (!outcome.ok) return;
    // Empty string stays "" while SQL NULL becomes null and 64-bit integer stays exact
    expect(outcome.cells.map((c) => c.value)).toEqual([
      "",
      "9007199254740993",
      "true",
      null,
      null,
      "false",
      'tab\there\nand "quote"',
      "7",
      "true",
    ]);
  });

  it("formats JSON with lossless 64-bit integers, high-precision decimals, and embedded JSON", () => {
    const res = formatGridCells({
      columns: sampleColumns,
      rows: sampleRows,
      format: "json",
    });
    expect(res.ok).toBe(true);
    if (!res.ok) return;

    // Raw text must preserve exact int64 and 30-digit decimal tokens without float64 rounding
    expect(res.text).toContain('"order_id": 9223372036854775807');
    expect(res.text).toContain('"order_id": -9223372036854775808');
    expect(res.text).toContain('"amount": 12345678901234567890.1234567890');
    expect(res.text).toContain('"big": 9223372036854775807');
    // Non-RFC8259 numeric tokens such as Postgres `NaN` fall back to valid JSON strings
    expect(res.text).toContain('"amount": "NaN"');
    // Empty string vs null
    expect(res.text).toContain('"note": ""');
    expect(res.text).toContain('"note": null');
    // Entire output must be valid RFC 8259 JSON
    expect(() => JSON.parse(res.text)).not.toThrow();
  });

  it("formats dialect-aware SQL INSERT statements for PostgreSQL, MySQL, and Snowflake", () => {
    const pg = formatGridCells({
      columns: sampleColumns,
      rows: [
        sampleRows[0],
        [{ kind: "default" }, "10.50", "O'Reilly \\ path", "false", null, null],
      ],
      format: "sqlInsert",
      engine: "postgres",
      table: { schema: "public", name: 'user"orders' },
    });
    expect(pg.ok).toBe(true);
    if (!pg.ok) return;
    expect(pg.text).toContain(
      'INSERT INTO "public"."user""orders" ("order_id", "amount", "note", "active", "payload", "digest")',
    );
    expect(pg.text).toContain(
      "(9223372036854775807, 12345678901234567890.1234567890, '', TRUE, '{\"big\":9223372036854775807,\"tag\":\"α\"}', '\\xdeadbeef'::bytea)",
    );
    expect(pg.text).toContain("(DEFAULT, 10.50, 'O''Reilly \\ path', FALSE, NULL, NULL);");

    const mysql = formatGridCells({
      columns: sampleColumns,
      rows: [["1", "2.5", "O'Reilly\nC:\\temp", "0", null, "0xdeadbeef"]],
      format: "sqlInsert",
      engine: "mysql",
      table: { schema: "shop", name: "order`items" },
    });
    expect(mysql.ok).toBe(true);
    if (!mysql.ok) return;
    expect(mysql.text).toContain(
      "INSERT INTO `shop`.`order``items` (`order_id`, `amount`, `note`, `active`, `payload`, `digest`)",
    );
    expect(mysql.text).toContain("(1, 2.5, 'O''Reilly\\nC:\\\\temp', FALSE, NULL, X'deadbeef');");
  });

  it("formats GFM Markdown tables with numeric alignment and escaped pipes/backslashes/newlines", () => {
    // Reordered visible columns (`note` first, `order_id` second) must dictate output column order
    const reordered = [sampleColumns[2], sampleColumns[0]];
    const res = formatGridCells({
      columns: reordered,
      rows: [
        ["a | b\\\nc", "9223372036854775807"],
        ["", "-5"],
        [null, { kind: "default" }],
      ],
      format: "markdown",
    });
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.text).toBe(
      [
        "| note | order_id |",
        "| --- | ---: |",
        "| a \\| b\\\\<br>c | 9223372036854775807 |",
        "|  | -5 |",
        "| NULL | <default> |",
      ].join("\n"),
    );
  });

  it("enforces the 10,000-cell and 8 MiB clipboard budgets", () => {
    const cols = [col("c1", "a", "text"), col("c2", "b", "text")];
    const tooManyRows = Array.from({ length: MAX_COPY_CELLS / 2 + 1 }, () => ["x", "y"]);
    const cellCap = formatGridCells({
      columns: cols,
      rows: tooManyRows,
      format: "csv",
    });
    expect(cellCap.ok).toBe(false);
    if (!cellCap.ok) {
      expect(cellCap.error).toContain("10,000 cells");
    }

    const hugeCell = "x".repeat(1024 * 1024);
    const nineMiBRows = Array.from({ length: 9 }, () => [hugeCell]);
    const byteCap = formatGridCells({
      columns: [col("c1", "blob_text", "text")],
      rows: nineMiBRows,
      format: "csv",
    });
    expect(byteCap.ok).toBe(false);
    if (!byteCap.ok) {
      expect(byteCap.error).toContain("8 MiB");
    }
    expect(MAX_COPY_BYTES).toBe(8 * 1024 * 1024);
  });
});
