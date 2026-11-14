import { describe, expect, it } from "vitest";
import type { ColumnInspection } from "../../ipc/generated";
import {
  buildSnowflakeGridMutations,
  parseCsvRows,
  stageCsvInsertRows,
} from "./snowflake-grid-staging";

function makeColumn(overrides: Partial<ColumnInspection> & { name: string }): ColumnInspection {
  return {
    ordinal: 1,
    fullType: "TEXT(100)",
    dataType: "TEXT",
    dataTypeAlias: null,
    kind: "column",
    resultKind: "text",
    nullable: true,
    defaultExpression: null,
    descriptorDefaultExpression: null,
    writeDefaultExpression: null,
    writeDefaultStatus: "notReported",
    comment: null,
    characterMaximumLength: "100",
    characterOctetLength: "400",
    numericPrecision: null,
    numericPrecisionRadix: null,
    numericScale: null,
    identity: {
      isIdentity: false,
      generation: null,
      start: null,
      increment: null,
      ordered: null,
    },
    expression: null,
    ...overrides,
  };
}

describe("snowflake-grid-staging", () => {
  it("parses RFC-4180 CSV with BOM, quotes, doubled quotes, and embedded newlines", () => {
    const parsed = parseCsvRows('\uFEFF"ID","NOTE"\r\n10,"hello, ""world""\nline2"\r\n', ",");
    expect(parsed).toEqual([
      ["ID", "NOTE"],
      ["10", 'hello, "world"\nline2'],
    ]);
    expect(() => parseCsvRows('"unclosed', ",")).toThrow(/Unclosed quoted field/);
  });

  it("stages CSV insert rows with header mapping, defaults, and null tokens", () => {
    const columns: ColumnInspection[] = [
      makeColumn({
        name: "ID",
        ordinal: 1,
        fullType: "NUMBER(38,0)",
        dataType: "NUMBER",
        resultKind: "integer",
        nullable: false,
        defaultExpression: "42",
        descriptorDefaultExpression: "42",
        characterMaximumLength: null,
        characterOctetLength: null,
        numericPrecision: "38",
        numericPrecisionRadix: "10",
        numericScale: "0",
      }),
      makeColumn({ name: "NOTES", ordinal: 2, nullable: true }),
    ];

    const staged = stageCsvInsertRows({
      raw: "id,notes\n,alpha\n7,\\N\n",
      delimiter: ",",
      hasHeader: true,
      nullToken: "\\N",
      columns,
    });
    expect(staged).toHaveLength(2);
    expect(staged[0].cells).toEqual({
      0: { kind: "default" },
      1: { kind: "text", value: "alpha" },
    });
    expect(staged[1].cells).toEqual({
      0: { kind: "text", value: "7" },
      1: { kind: "null" },
    });
  });

  it("flags mixed batches combining UPDATE/DELETE with DEFAULT insert postimages", () => {
    const columns: ColumnInspection[] = [
      makeColumn({
        name: "ID",
        ordinal: 1,
        nullable: false,
        defaultExpression: "1",
        descriptorDefaultExpression: "1",
      }),
      makeColumn({ name: "NOTES", ordinal: 2, nullable: true }),
    ];

    const mixedWithDefault = buildSnowflakeGridMutations({
      deletedRows: { 2: true },
      updatedCells: {
        0: { 1: { kind: "text", value: "updated" } },
        2: { 1: { kind: "text", value: "ignored-because-deleted" } },
      },
      insertedRows: [
        {
          id: "ins-1",
          cells: {
            0: { kind: "default" },
            1: { kind: "text", value: "new" },
          },
        },
      ],
      columns,
    });
    expect(mixedWithDefault.mixedUnknownInsertWarning).toMatch(/Inserts using DEFAULT/);
    expect(mixedWithDefault.mutations).toEqual([
      { kind: "delete", rowIndex: 2 },
      {
        kind: "update",
        rowIndex: 0,
        changes: [{ columnIndex: 1, value: { kind: "text", value: "updated" } }],
      },
      {
        kind: "insert",
        values: [
          { columnIndex: 0, value: { kind: "default" } },
          { columnIndex: 1, value: { kind: "text", value: "new" } },
        ],
      },
    ]);

    const mixedWithExplicit = buildSnowflakeGridMutations({
      deletedRows: { 2: true },
      updatedCells: {},
      insertedRows: [
        {
          id: "ins-2",
          cells: {
            0: { kind: "text", value: "99" },
            1: { kind: "null" },
          },
        },
      ],
      columns,
    });
    expect(mixedWithExplicit.mixedUnknownInsertWarning).toBeNull();
  });
});
