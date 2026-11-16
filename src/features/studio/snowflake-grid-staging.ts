import type {
  ColumnInspection,
  SnowflakeGridCellInput,
  SnowflakeGridColumnValue,
  SnowflakeGridMutation,
} from "../../ipc/generated";

export const MAX_SNOWFLAKE_GRID_MUTATIONS = 500;
export const MAX_SNOWFLAKE_CSV_IMPORT_ROWS = 100_000;
export const MAX_SNOWFLAKE_SCALAR_BIND_BYTES = 262_144;
export const MAX_SNOWFLAKE_UNIT_SQL_BYTES = 786_432;
export const SAFE_SNOWFLAKE_BATCH_FRAME_BYTES = 900_000;
const SAFE_SNOWFLAKE_COMPILER_BATCH_BYTES = 7 * 1024 * 1024;
const BATCH_FRAME_ENVELOPE_BYTES = 512;
const utf8Encoder = new TextEncoder();
const SIMPLE_JSON_ASCII = /^[\x20-\x21\x23-\x5b\x5d-\x7e]*$/;

function utf8ByteLength(value: string): number {
  return SIMPLE_JSON_ASCII.test(value) ? value.length : utf8Encoder.encode(value).length;
}

function jsonStringUtf8Bytes(value: string): number {
  return SIMPLE_JSON_ASCII.test(value)
    ? value.length + 2
    : utf8Encoder.encode(JSON.stringify(value)).length;
}

function quoteSqlIdentifier(name: string): string {
  return `"${name.replaceAll('"', '""')}"`;
}

export interface CsvTargetRelationName {
  database?: string | null;
  schema: string;
  name: string;
}

export type CsvFieldDelimiter = "," | "\t" | ";" | "|";

export interface StagedInsertRow {
  id: string;
  cells: Record<number, SnowflakeGridCellInput>;
}

export type SnowflakeEditTypeFamily =
  | "fixed"
  | "real"
  | "text"
  | "boolean"
  | "date"
  | "time"
  | "timestamp_ntz"
  | "timestamp_ltz"
  | "timestamp_tz"
  | "binary"
  | "variant"
  | "object"
  | "array";

export function snowflakeEditTypeFamily(dataType: string): SnowflakeEditTypeFamily | null {
  const base = dataType.trim().toUpperCase().split(/[<( ]/, 1)[0];
  switch (base) {
    case "NUMBER":
    case "DECIMAL":
    case "NUMERIC":
    case "FIXED":
    case "INT":
    case "INTEGER":
    case "BIGINT":
    case "SMALLINT":
    case "TINYINT":
    case "BYTEINT":
      return "fixed";
    case "FLOAT":
    case "FLOAT4":
    case "FLOAT8":
    case "DOUBLE":
    case "REAL":
      return "real";
    case "VARCHAR":
    case "CHAR":
    case "CHARACTER":
    case "STRING":
    case "TEXT":
      return "text";
    case "BOOLEAN":
      return "boolean";
    case "DATE":
      return "date";
    case "TIME":
      return "time";
    case "TIMESTAMP_NTZ":
    case "DATETIME":
      return "timestamp_ntz";
    case "TIMESTAMP_LTZ":
      return "timestamp_ltz";
    case "TIMESTAMP_TZ":
      return "timestamp_tz";
    case "BINARY":
    case "VARBINARY":
      return "binary";
    case "VARIANT":
      return "variant";
    case "OBJECT":
      return "object";
    case "ARRAY":
      return "array";
    default:
      return null;
  }
}

export function isWritableSnowflakeColumn(column: ColumnInspection): boolean {
  return (
    column.kind === "column" &&
    column.expression === null &&
    column.identity.isIdentity === false &&
    snowflakeEditTypeFamily(column.dataType) !== null
  );
}

export function canColumnDefaultOnInsert(column: ColumnInspection): boolean {
  return column.nullable || column.defaultExpression !== null;
}

export function defaultInsertCellForColumn(column: ColumnInspection): SnowflakeGridCellInput {
  if (canColumnDefaultOnInsert(column)) {
    return { kind: "default" };
  }
  switch (snowflakeEditTypeFamily(column.dataType)) {
    case "fixed":
    case "real":
      return { kind: "text", value: "0" };
    case "boolean":
      return { kind: "text", value: "false" };
    case "date":
      return { kind: "text", value: "1970-01-01" };
    case "time":
      return { kind: "text", value: "00:00:00" };
    case "timestamp_ntz":
      return { kind: "text", value: "1970-01-01 00:00:00" };
    case "timestamp_ltz":
    case "timestamp_tz":
      return { kind: "text", value: "1970-01-01 00:00:00 +00:00" };
    case "variant":
      return { kind: "text", value: "null" };
    case "array":
      return { kind: "text", value: "[]" };
    case "object":
      return { kind: "text", value: "{}" };
    default:
      return { kind: "text", value: "" };
  }
}

export function createBlankInsertRow(columns: readonly ColumnInspection[]): StagedInsertRow {
  const cells: Record<number, SnowflakeGridCellInput> = {};
  columns.forEach((column, index) => {
    if (isWritableSnowflakeColumn(column)) {
      cells[index] = defaultInsertCellForColumn(column);
    }
  });
  return { id: crypto.randomUUID(), cells };
}

export function cloneSnowflakePreviewRow(
  columns: readonly ColumnInspection[],
  row: readonly (string | null)[],
  rowUpdates?: Readonly<Record<number, SnowflakeGridCellInput>>,
): StagedInsertRow {
  const cells: Record<number, SnowflakeGridCellInput> = {};
  columns.forEach((column, index) => {
    if (!isWritableSnowflakeColumn(column)) return;
    if (column.identity && canColumnDefaultOnInsert(column)) {
      cells[index] = { kind: "default" };
      return;
    }
    const updated = rowUpdates?.[index];
    if (updated) {
      cells[index] = updated;
      return;
    }
    const raw = row[index] ?? null;
    cells[index] = raw === null ? { kind: "null" } : { kind: "text", value: raw };
  });
  return { id: crypto.randomUUID(), cells };
}

export function buildSnowflakeGridMutations(args: {
  deletedRows: Readonly<Record<number, true>>;
  updatedCells: Readonly<Record<number, Readonly<Record<number, SnowflakeGridCellInput>>>>;
  insertedRows: readonly StagedInsertRow[];
  columns: readonly ColumnInspection[];
}): {
  mutations: SnowflakeGridMutation[];
  mixedUnknownInsertWarning: string | null;
} {
  const { deletedRows, updatedCells, insertedRows, columns } = args;
  const mutations: SnowflakeGridMutation[] = [];

  const deletedIndices = Object.keys(deletedRows)
    .map(Number)
    .sort((a, b) => a - b);
  for (const rowIndex of deletedIndices) {
    mutations.push({ kind: "delete", rowIndex });
  }

  const updatedIndices = Object.keys(updatedCells)
    .map(Number)
    .filter((rowIndex) => !deletedRows[rowIndex])
    .sort((a, b) => a - b);
  for (const rowIndex of updatedIndices) {
    const rowChanges = updatedCells[rowIndex];
    if (!rowChanges) continue;
    const changes: SnowflakeGridColumnValue[] = Object.keys(rowChanges)
      .map(Number)
      .sort((a, b) => a - b)
      .map((columnIndex) => ({
        columnIndex,
        value: rowChanges[columnIndex],
      }));
    if (changes.length > 0) {
      mutations.push({ kind: "update", rowIndex, changes });
    }
  }

  const hasOriginalMutations = mutations.length > 0;
  let hasUnknownInsertPostimage = false;

  for (const row of insertedRows) {
    const values: SnowflakeGridColumnValue[] = [];
    columns.forEach((column, columnIndex) => {
      if (!isWritableSnowflakeColumn(column)) {
        hasUnknownInsertPostimage = true;
        return;
      }
      const cell = row.cells[columnIndex] ?? defaultInsertCellForColumn(column);
      if (cell.kind === "default") {
        hasUnknownInsertPostimage = true;
      }
      values.push({ columnIndex, value: cell });
    });
    mutations.push({ kind: "insert", values });
  }

  const mixedUnknownInsertWarning =
    hasOriginalMutations && insertedRows.length > 0 && hasUnknownInsertPostimage
      ? "Inserts using DEFAULT or generated columns cannot be combined in the same batch with UPDATE/DELETE rows. Supply explicit insert values or submit inserts in a separate batch."
      : null;

  return { mutations, mixedUnknownInsertWarning };
}

/** RFC-4180 CSV parser supporting quoted fields, doubled quotes, and embedded newlines. */
export function parseCsvRows(raw: string, delimiter: CsvFieldDelimiter): string[][] {
  const text = raw.startsWith("\uFEFF") ? raw.slice(1) : raw;
  if (text.length === 0) return [];

  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let inQuotes = false;
  let fieldQuoted = false;

  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (inQuotes) {
      if (ch === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i += 1;
        } else {
          inQuotes = false;
        }
      } else {
        field += ch;
      }
      continue;
    }

    if (ch === '"') {
      if (field.length !== 0) {
        throw new Error("Unexpected quote inside unquoted CSV field");
      }
      inQuotes = true;
      fieldQuoted = true;
      continue;
    }

    if (ch === delimiter) {
      row.push(field);
      field = "";
      fieldQuoted = false;
      continue;
    }

    if (ch === "\r" || ch === "\n") {
      if (ch === "\r" && text[i + 1] === "\n") {
        i += 1;
      }
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
      fieldQuoted = false;
      continue;
    }

    if (fieldQuoted && ch.trim().length > 0) {
      throw new Error("Unexpected characters after closing quote in CSV field");
    }
    field += ch;
  }

  if (inQuotes) {
    throw new Error("Unclosed quoted field in CSV input");
  }

  if (field.length > 0 || fieldQuoted || row.length > 0) {
    row.push(field);
    rows.push(row);
  }

  return rows;
}

function mapCsvRowsToInsertRows(
  args: {
    raw: string;
    delimiter: CsvFieldDelimiter;
    hasHeader: boolean;
    nullToken: string;
    columns: readonly ColumnInspection[];
  },
  maxRows: number,
  maxRowsMessage: string,
): StagedInsertRow[] {
  const { raw, delimiter, hasHeader, nullToken, columns } = args;
  const parsed = parseCsvRows(raw, delimiter);
  if (parsed.length === 0) {
    throw new Error("CSV input is empty");
  }

  const writableIndices = columns
    .map((column, index) => ({ column, index }))
    .filter(({ column }) => isWritableSnowflakeColumn(column));
  if (writableIndices.length === 0) {
    throw new Error("This relation has no writable columns");
  }

  let dataRows: string[][];
  let columnMapping: number[];

  if (hasHeader) {
    const headers = parsed[0].map((header) => header.trim());
    if (headers.length === 0 || headers.every((header) => header.length === 0)) {
      throw new Error("CSV header row is empty");
    }
    const usedTargets = new Set<number>();
    columnMapping = headers.map((header) => {
      const exactMatches = writableIndices.filter(({ column }) => column.name === header);
      let resolved = exactMatches.length === 1 ? exactMatches[0].index : -1;
      if (resolved === -1) {
        const folded = header.toLocaleUpperCase();
        const ciMatches = writableIndices.filter(
          ({ column }) => column.name.toLocaleUpperCase() === folded,
        );
        if (ciMatches.length > 1) {
          throw new Error(`CSV header "${header}" matches multiple case-sensitive columns`);
        }
        if (ciMatches.length === 1) {
          resolved = ciMatches[0].index;
        }
      }
      if (resolved === -1) {
        throw new Error(`CSV header "${header}" does not match any writable column`);
      }
      if (usedTargets.has(resolved)) {
        throw new Error(`CSV header "${header}" is mapped more than once`);
      }
      usedTargets.add(resolved);
      return resolved;
    });
    dataRows = parsed.slice(1);
  } else {
    const width = parsed[0].length;
    if (width > writableIndices.length) {
      throw new Error(
        `CSV rows have ${width} columns, but this relation only has ${writableIndices.length} writable columns`,
      );
    }
    columnMapping = writableIndices.slice(0, width).map(({ index }) => index);
    dataRows = parsed;
  }

  if (dataRows.length === 0) {
    throw new Error("CSV input contains no data rows");
  }
  if (dataRows.length > maxRows) {
    throw new Error(maxRowsMessage);
  }

  const mappedSet = new Set(columnMapping);
  for (const { column, index } of writableIndices) {
    if (!mappedSet.has(index) && !canColumnDefaultOnInsert(column)) {
      throw new Error(
        `Required writable column "${column.name}" has no default and is missing from CSV`,
      );
    }
  }

  return dataRows.map((csvRow, rowOffset) => {
    if (csvRow.length !== columnMapping.length) {
      throw new Error(
        `CSV row ${hasHeader ? rowOffset + 2 : rowOffset + 1} has ${csvRow.length} fields; expected ${columnMapping.length}`,
      );
    }
    const cells: Record<number, SnowflakeGridCellInput> = {};
    for (const { column, index } of writableIndices) {
      if (!mappedSet.has(index)) {
        cells[index] = defaultInsertCellForColumn(column);
      }
    }
    csvRow.forEach((rawValue, csvIndex) => {
      const targetIndex = columnMapping[csvIndex];
      const column = columns[targetIndex];
      if (nullToken.length > 0 && rawValue === nullToken) {
        if (!column.nullable) {
          throw new Error(
            `CSV row ${hasHeader ? rowOffset + 2 : rowOffset + 1}: column "${column.name}" is NOT NULL`,
          );
        }
        cells[targetIndex] = { kind: "null" };
      } else if (
        rawValue === "" &&
        column.resultKind !== "text" &&
        !(
          snowflakeEditTypeFamily(column.dataType) === "binary" &&
          column.defaultExpression === null &&
          !column.nullable
        )
      ) {
        if (column.defaultExpression !== null) {
          cells[targetIndex] = { kind: "default" };
        } else if (column.nullable) {
          cells[targetIndex] = { kind: "null" };
        } else {
          throw new Error(
            `CSV row ${hasHeader ? rowOffset + 2 : rowOffset + 1}: non-text column "${column.name}" cannot be empty`,
          );
        }
      } else {
        if (utf8ByteLength(rawValue) > MAX_SNOWFLAKE_SCALAR_BIND_BYTES) {
          throw new Error(
            `CSV row ${hasHeader ? rowOffset + 2 : rowOffset + 1}: column "${column.name}" exceeds the 256 KiB scalar bind limit`,
          );
        }
        cells[targetIndex] = { kind: "text", value: rawValue };
      }
    });
    return { id: crypto.randomUUID(), cells };
  });
}

interface WritableInsertColumnCost {
  index: number;
  isSemiStructured: boolean;
  quotedSqlBytes: number;
  quotedJsonBytes: number;
  castSqlBytes: number;
  castJsonBytes: number;
}

function buildInsertColumnCosts(columns: readonly ColumnInspection[]): WritableInsertColumnCost[] {
  const costs: WritableInsertColumnCost[] = [];
  columns.forEach((column, index) => {
    if (!isWritableSnowflakeColumn(column)) return;
    const family = snowflakeEditTypeFamily(column.dataType);
    const quoted = quoteSqlIdentifier(column.name);
    const quotedSqlBytes = utf8ByteLength(quoted) + 2;
    const quotedJsonBytes = jsonStringUtf8Bytes(quoted) - 2 + 2;
    let castSqlBytes = 30;
    if (family === "timestamp_ntz" || family === "timestamp_ltz" || family === "timestamp_tz") {
      castSqlBytes = 100;
    } else if (family === "time") {
      castSqlBytes = 68;
    } else if (family === "date") {
      castSqlBytes = 52;
    } else if (family === "array" || family === "object") {
      castSqlBytes = 44;
    } else if (family === "variant") {
      castSqlBytes = 34;
    } else if (family === "binary") {
      castSqlBytes = 32;
    }
    costs.push({
      index,
      isSemiStructured: family === "variant" || family === "array" || family === "object",
      quotedSqlBytes,
      quotedJsonBytes,
      castSqlBytes,
      castJsonBytes: castSqlBytes + 4,
    });
  });
  return costs;
}

function qualifiedTableLengths(table?: CsvTargetRelationName): {
  sqlBytes: number;
  jsonBytes: number;
} {
  if (!table) {
    return { sqlBytes: 256, jsonBytes: 320 };
  }
  const qualified = `${quoteSqlIdentifier(table.database ?? "DB")}.${quoteSqlIdentifier(table.schema)}.${quoteSqlIdentifier(table.name)}`;
  return {
    sqlBytes: utf8ByteLength(qualified),
    jsonBytes: jsonStringUtf8Bytes(qualified) - 2,
  };
}

function estimateInsertRowCost(
  row: StagedInsertRow,
  columns: readonly ColumnInspection[],
  columnCosts: readonly WritableInsertColumnCost[],
  tableLengths: { sqlBytes: number; jsonBytes: number },
): { sqlBytes: number; frameBytes: number; compilerBytes: number } {
  let needsSelect = false;
  for (const col of columnCosts) {
    const cell = row.cells[col.index] ?? defaultInsertCellForColumn(columns[col.index]);
    if (col.isSemiStructured && cell.kind !== "default") {
      needsSelect = true;
      break;
    }
  }
  let sqlBytes = 28 + tableLengths.sqlBytes;
  let jsonSqlBytes = 28 + tableLengths.jsonBytes;
  let bindsJsonBytes = 2;
  let compilerCellBytes = 128;
  for (const col of columnCosts) {
    const cell = row.cells[col.index] ?? defaultInsertCellForColumn(columns[col.index]);
    if (needsSelect && cell.kind === "default") continue;
    sqlBytes += col.quotedSqlBytes;
    jsonSqlBytes += col.quotedJsonBytes;
    if (cell.kind === "default") {
      sqlBytes += 9;
      jsonSqlBytes += 9;
    } else if (cell.kind === "null") {
      sqlBytes += 32;
      jsonSqlBytes += 32;
      bindsJsonBytes += 5;
      compilerCellBytes += 64;
    } else {
      sqlBytes += col.castSqlBytes;
      jsonSqlBytes += col.castJsonBytes;
      bindsJsonBytes += jsonStringUtf8Bytes(cell.value) + 1;
      compilerCellBytes += 2 * utf8ByteLength(cell.value) + 64;
    }
  }
  return {
    sqlBytes,
    frameBytes: 48 + jsonSqlBytes + bindsJsonBytes,
    compilerBytes: sqlBytes + compilerCellBytes,
  };
}

export function estimateInsertRowsFrameBytes(
  rows: readonly StagedInsertRow[],
  columns: readonly ColumnInspection[],
  table?: CsvTargetRelationName,
): number {
  const columnCosts = buildInsertColumnCosts(columns);
  const tableLengths = qualifiedTableLengths(table);
  let total = BATCH_FRAME_ENVELOPE_BYTES;
  for (const row of rows) {
    total += estimateInsertRowCost(row, columns, columnCosts, tableLengths).frameBytes;
  }
  return total;
}

export function stageCsvInsertRows(args: {
  raw: string;
  delimiter: CsvFieldDelimiter;
  hasHeader: boolean;
  nullToken: string;
  columns: readonly ColumnInspection[];
  table?: CsvTargetRelationName;
}): StagedInsertRow[] {
  const rows = mapCsvRowsToInsertRows(
    args,
    MAX_SNOWFLAKE_GRID_MUTATIONS,
    `CSV batch exceeds the ${MAX_SNOWFLAKE_GRID_MUTATIONS}-row generated mutation limit`,
  );
  const columnCosts = buildInsertColumnCosts(args.columns);
  const tableLengths = qualifiedTableLengths(args.table);
  let frameBytes = BATCH_FRAME_ENVELOPE_BYTES;
  let compilerBytes = 0;
  rows.forEach((row, idx) => {
    const cost = estimateInsertRowCost(row, args.columns, columnCosts, tableLengths);
    if (
      cost.sqlBytes > MAX_SNOWFLAKE_UNIT_SQL_BYTES ||
      BATCH_FRAME_ENVELOPE_BYTES + cost.frameBytes > SAFE_SNOWFLAKE_BATCH_FRAME_BYTES ||
      cost.compilerBytes > SAFE_SNOWFLAKE_COMPILER_BATCH_BYTES
    ) {
      throw new Error(
        `CSV row ${args.hasHeader ? idx + 2 : idx + 1} exceeds the Snowflake generated batch frame limit; reduce row width or cell payload size`,
      );
    }
    frameBytes += cost.frameBytes;
    compilerBytes += cost.compilerBytes;
  });
  if (
    frameBytes > SAFE_SNOWFLAKE_BATCH_FRAME_BYTES ||
    compilerBytes > SAFE_SNOWFLAKE_COMPILER_BATCH_BYTES
  ) {
    throw new Error(
      "CSV batch exceeds the Snowflake helper frame limit; use multi-batch import or fewer rows",
    );
  }
  return rows;
}

/**
 * Validates up to 100,000 CSV data rows upfront and splits them into generated-mutation
 * batches bounded by row count (<= 500), 1 MiB helper wire frame, and 8 MiB compiler budget.
 */
export function stageCsvInsertBatches(
  args: {
    raw: string;
    delimiter: CsvFieldDelimiter;
    hasHeader: boolean;
    nullToken: string;
    columns: readonly ColumnInspection[];
    table?: CsvTargetRelationName;
  },
  batchSize: number = MAX_SNOWFLAKE_GRID_MUTATIONS,
): StagedInsertRow[][] {
  if (batchSize <= 0 || batchSize > MAX_SNOWFLAKE_GRID_MUTATIONS) {
    throw new Error(`CSV batch size must be between 1 and ${MAX_SNOWFLAKE_GRID_MUTATIONS} rows`);
  }
  const allRows = mapCsvRowsToInsertRows(
    args,
    MAX_SNOWFLAKE_CSV_IMPORT_ROWS,
    `CSV import exceeds the ${MAX_SNOWFLAKE_CSV_IMPORT_ROWS.toLocaleString()}-row multi-batch limit`,
  );
  const columnCosts = buildInsertColumnCosts(args.columns);
  const tableLengths = qualifiedTableLengths(args.table);
  const batches: StagedInsertRow[][] = [];
  let currentBatch: StagedInsertRow[] = [];
  let currentFrameBytes = BATCH_FRAME_ENVELOPE_BYTES;
  let currentCompilerBytes = 0;

  allRows.forEach((row, idx) => {
    const cost = estimateInsertRowCost(row, args.columns, columnCosts, tableLengths);
    if (
      cost.sqlBytes > MAX_SNOWFLAKE_UNIT_SQL_BYTES ||
      BATCH_FRAME_ENVELOPE_BYTES + cost.frameBytes > SAFE_SNOWFLAKE_BATCH_FRAME_BYTES ||
      cost.compilerBytes > SAFE_SNOWFLAKE_COMPILER_BATCH_BYTES
    ) {
      throw new Error(
        `CSV row ${args.hasHeader ? idx + 2 : idx + 1} exceeds the Snowflake generated batch frame limit; reduce row width or cell payload size`,
      );
    }
    if (
      currentBatch.length > 0 &&
      (currentBatch.length >= batchSize ||
        currentFrameBytes + cost.frameBytes > SAFE_SNOWFLAKE_BATCH_FRAME_BYTES ||
        currentCompilerBytes + cost.compilerBytes > SAFE_SNOWFLAKE_COMPILER_BATCH_BYTES)
    ) {
      batches.push(currentBatch);
      currentBatch = [];
      currentFrameBytes = BATCH_FRAME_ENVELOPE_BYTES;
      currentCompilerBytes = 0;
    }
    currentBatch.push(row);
    currentFrameBytes += cost.frameBytes;
    currentCompilerBytes += cost.compilerBytes;
  });

  if (currentBatch.length > 0) {
    batches.push(currentBatch);
  }
  return batches;
}
