import type { ColumnInfo, EngineKind, TableRef } from "../../ipc/generated";
import { formatJsonLossless, isJsonColumn } from "./value-format";

export type CopyFormat = "csv" | "tsv" | "json" | "sqlInsert" | "markdown";

export type CopyCell = string | null | { readonly kind: "default" };

export const MAX_COPY_CELLS = 10_000;
export const MAX_COPY_BYTES = 8 * 1024 * 1024;
const SQL_INSERT_BATCH_ROWS = 500;

export const COPY_FORMATS: readonly {
  id: CopyFormat;
  label: string;
  hint: string;
}[] = [
  { id: "csv", label: "CSV", hint: "RFC 4180" },
  { id: "tsv", label: "TSV", hint: "Tab-separated" },
  { id: "json", label: "JSON", hint: "Exact numbers" },
  { id: "sqlInsert", label: "SQL INSERT", hint: "Dialect-quoted" },
  { id: "markdown", label: "Markdown", hint: "GFM table" },
];

export function copyFormatLabel(format: CopyFormat): string {
  return COPY_FORMATS.find((entry) => entry.id === format)?.label ?? format;
}

export interface FormatGridOptions {
  columns: readonly ColumnInfo[];
  rows: readonly (readonly CopyCell[])[];
  format: CopyFormat;
  engine?: EngineKind;
  table?: TableRef | null;
  includeHeaders?: boolean;
  nullToken?: string;
}

export type FormatGridResult =
  | {
      ok: true;
      text: string;
      byteLength: number;
      rowCount: number;
      columnCount: number;
    }
  | {
      ok: false;
      error: string;
    };

const RFC8259_NUMBER_PATTERN = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/;
const SQL_INTEGER_PATTERN = /^-?(?:0|[1-9]\d*)$/;
const HEX_BINARY_PATTERN = /^(?:\\[xX]|0[xX])([0-9a-fA-F]*)$/;

const utf8Encoder = new TextEncoder();

function utf8ByteLength(text: string): number {
  for (let i = 0; i < text.length; i++) {
    if (text.charCodeAt(i) > 0x7f) {
      return utf8Encoder.encode(text).byteLength;
    }
  }
  return text.length;
}

class BoundedTextBuilder {
  private chunks: string[] = [];
  private bytes = 0;

  constructor(private readonly maxBytes: number) {}

  append(chunk: string): boolean {
    if (!chunk) return true;
    const chunkBytes = utf8ByteLength(chunk);
    if (this.bytes + chunkBytes > this.maxBytes) {
      return false;
    }
    this.chunks.push(chunk);
    this.bytes += chunkBytes;
    return true;
  }

  get byteLength(): number {
    return this.bytes;
  }

  finish(): string {
    return this.chunks.join("");
  }
}

function isDefaultCell(cell: CopyCell): cell is { readonly kind: "default" } {
  return typeof cell === "object" && cell !== null && cell.kind === "default";
}

function quoteDelimited(raw: string): string {
  return `"${raw.replace(/"/g, '""')}"`;
}

function needsDelimitedQuotes(raw: string, delimiter: string, nullToken: string): boolean {
  return (
    raw === "\\N" ||
    (nullToken !== "" && raw === nullToken) ||
    raw.includes(delimiter) ||
    raw.includes('"') ||
    raw.includes("\n") ||
    raw.includes("\r")
  );
}

function formatDelimitedHeader(name: string, delimiter: string, nullToken: string): string {
  if (name === "" || needsDelimitedQuotes(name, delimiter, nullToken)) {
    return quoteDelimited(name);
  }
  return name;
}

function formatDelimitedCell(cell: CopyCell, delimiter: string, nullToken: string): string {
  if (cell === null) return nullToken;
  if (isDefaultCell(cell)) return "<default>";
  if (cell === "") return '""';
  if (needsDelimitedQuotes(cell, delimiter, nullToken)) {
    return quoteDelimited(cell);
  }
  return cell;
}

function formatJsonCell(cell: CopyCell, column: ColumnInfo): string {
  if (cell === null || isDefaultCell(cell)) return "null";
  if (column.kind === "integer" || column.kind === "decimal") {
    const trimmed = cell.trim();
    return RFC8259_NUMBER_PATTERN.test(trimmed) ? trimmed : JSON.stringify(cell);
  }
  if (column.kind === "boolean") {
    const lower = cell.trim().toLowerCase();
    if (lower === "true" || lower === "t" || lower === "1") return "true";
    if (lower === "false" || lower === "f" || lower === "0") return "false";
    return JSON.stringify(cell);
  }
  if (isJsonColumn(column)) {
    const formatted = formatJsonLossless(cell);
    if (formatted.ok) {
      return formatted.formatted.replace(/\n/g, "\n    ");
    }
    return JSON.stringify(cell);
  }
  return JSON.stringify(cell);
}

export function quoteSqlIdentifier(name: string, engine?: EngineKind): string {
  if (engine === "mysql") {
    return `\`${name.replace(/`/g, "``")}\``;
  }
  return `"${name.replace(/"/g, '""')}"`;
}

export function formatSqlTableName(table?: TableRef | null, engine?: EngineKind): string {
  const tableName = table?.name?.trim();
  if (!tableName) {
    return quoteSqlIdentifier("result_rows", engine);
  }
  const schemaName = table?.schema?.trim();
  if (schemaName) {
    return `${quoteSqlIdentifier(schemaName, engine)}.${quoteSqlIdentifier(tableName, engine)}`;
  }
  return quoteSqlIdentifier(tableName, engine);
}

function quoteSqlString(raw: string, engine?: EngineKind): string {
  if (engine === "mysql" || engine === "snowflake") {
    let escaped = "";
    for (let i = 0; i < raw.length; i++) {
      const ch = raw[i];
      if (ch === "\\") escaped += "\\\\";
      else if (ch === "'") escaped += "''";
      else if (ch === "\0") escaped += "\\0";
      else if (ch === "\r") escaped += "\\r";
      else if (ch === "\n") escaped += "\\n";
      else escaped += ch;
    }
    return `'${escaped}'`;
  }
  return `'${raw.replace(/'/g, "''")}'`;
}

function formatSqlCell(cell: CopyCell, column: ColumnInfo, engine?: EngineKind): string {
  if (isDefaultCell(cell)) return "DEFAULT";
  if (cell === null) return "NULL";
  if (column.kind === "integer") {
    const trimmed = cell.trim();
    return SQL_INTEGER_PATTERN.test(trimmed) ? trimmed : quoteSqlString(cell, engine);
  }
  if (column.kind === "decimal") {
    const trimmed = cell.trim();
    return RFC8259_NUMBER_PATTERN.test(trimmed) ? trimmed : quoteSqlString(cell, engine);
  }
  if (column.kind === "boolean") {
    const lower = cell.trim().toLowerCase();
    if (lower === "true" || lower === "t" || lower === "1") return "TRUE";
    if (lower === "false" || lower === "f" || lower === "0") return "FALSE";
    return quoteSqlString(cell, engine);
  }
  if (column.kind === "binary") {
    const hexMatch = HEX_BINARY_PATTERN.exec(cell.trim());
    if (hexMatch && hexMatch[1].length % 2 === 0) {
      const hex = hexMatch[1];
      if (engine === "mysql" || engine === "snowflake") {
        return `X'${hex}'`;
      }
      return `'\\x${hex}'::bytea`;
    }
    return quoteSqlString(cell, engine);
  }
  return quoteSqlString(cell, engine);
}

function escapeMarkdownText(raw: string): string {
  return raw
    .replace(/\\/g, "\\\\")
    .replace(/\|/g, "\\|")
    .replace(/\r\n|\r|\n/g, "<br>");
}

function formatMarkdownCell(cell: CopyCell): string {
  if (cell === null) return "NULL";
  if (isDefaultCell(cell)) return "<default>";
  return escapeMarkdownText(cell);
}

const BYTE_LIMIT_ERROR = "Copy is limited to 8 MiB. Select fewer cells or use Export.";

export function formatGridCells(options: FormatGridOptions): FormatGridResult {
  const { columns, rows, format, engine, table } = options;
  const includeHeaders = options.includeHeaders ?? true;

  if (columns.length === 0) {
    return { ok: false, error: "No visible columns to copy." };
  }

  for (let r = 0; r < rows.length; r++) {
    if (rows[r].length !== columns.length) {
      return {
        ok: false,
        error: `Row ${r + 1} has ${rows[r].length} values, expected ${columns.length}.`,
      };
    }
  }

  const totalCells = rows.length * columns.length;
  if (totalCells > MAX_COPY_CELLS) {
    return {
      ok: false,
      error: `Select at most ${MAX_COPY_CELLS.toLocaleString()} cells to copy, or use Export for larger datasets.`,
    };
  }

  const builder = new BoundedTextBuilder(MAX_COPY_BYTES);

  if (format === "csv" || format === "tsv") {
    const delimiter = format === "csv" ? "," : "\t";
    const nullToken = options.nullToken ?? (format === "csv" ? "" : "\\N");
    let hasLine = false;

    if (includeHeaders) {
      const header = columns
        .map((col) => formatDelimitedHeader(col.name, delimiter, nullToken))
        .join(delimiter);
      if (!builder.append(header)) return { ok: false, error: BYTE_LIMIT_ERROR };
      hasLine = true;
    }

    for (const row of rows) {
      if (hasLine && !builder.append("\n")) return { ok: false, error: BYTE_LIMIT_ERROR };
      for (let c = 0; c < columns.length; c++) {
        if (c > 0 && !builder.append(delimiter)) return { ok: false, error: BYTE_LIMIT_ERROR };
        if (!builder.append(formatDelimitedCell(row[c], delimiter, nullToken))) {
          return { ok: false, error: BYTE_LIMIT_ERROR };
        }
      }
      hasLine = true;
    }
  } else if (format === "json") {
    if (rows.length === 0) {
      builder.append("[]");
    } else {
      if (!builder.append("[\n")) return { ok: false, error: BYTE_LIMIT_ERROR };
      const encodedKeys = columns.map((col) => JSON.stringify(col.name));
      for (let r = 0; r < rows.length; r++) {
        if (!builder.append("  {\n")) return { ok: false, error: BYTE_LIMIT_ERROR };
        const row = rows[r];
        for (let c = 0; c < columns.length; c++) {
          const suffix = c + 1 < columns.length ? ",\n" : "\n";
          const entry = `    ${encodedKeys[c]}: ${formatJsonCell(row[c], columns[c])}${suffix}`;
          if (!builder.append(entry)) return { ok: false, error: BYTE_LIMIT_ERROR };
        }
        const rowSuffix = r + 1 < rows.length ? "  },\n" : "  }\n";
        if (!builder.append(rowSuffix)) return { ok: false, error: BYTE_LIMIT_ERROR };
      }
      if (!builder.append("]")) return { ok: false, error: BYTE_LIMIT_ERROR };
    }
  } else if (format === "sqlInsert") {
    if (rows.length > 0) {
      const targetTable = formatSqlTableName(table, engine);
      const columnList = columns.map((col) => quoteSqlIdentifier(col.name, engine)).join(", ");
      const statementPrefix = `INSERT INTO ${targetTable} (${columnList})\nVALUES\n`;

      for (let start = 0; start < rows.length; start += SQL_INSERT_BATCH_ROWS) {
        if (start > 0 && !builder.append("\n\n")) return { ok: false, error: BYTE_LIMIT_ERROR };
        if (!builder.append(statementPrefix)) return { ok: false, error: BYTE_LIMIT_ERROR };
        const end = Math.min(rows.length, start + SQL_INSERT_BATCH_ROWS);
        for (let r = start; r < end; r++) {
          const row = rows[r];
          if (!builder.append("  (")) return { ok: false, error: BYTE_LIMIT_ERROR };
          for (let c = 0; c < columns.length; c++) {
            if (c > 0 && !builder.append(", ")) return { ok: false, error: BYTE_LIMIT_ERROR };
            if (!builder.append(formatSqlCell(row[c], columns[c], engine))) {
              return { ok: false, error: BYTE_LIMIT_ERROR };
            }
          }
          const tupleSuffix = r + 1 < end ? "),\n" : ");";
          if (!builder.append(tupleSuffix)) return { ok: false, error: BYTE_LIMIT_ERROR };
        }
      }
    }
  } else if (format === "markdown") {
    const header = `| ${columns.map((col) => escapeMarkdownText(col.name)).join(" | ")} |`;
    const separator = `| ${columns
      .map((col) => (col.kind === "integer" || col.kind === "decimal" ? "---:" : "---"))
      .join(" | ")} |`;
    if (!builder.append(`${header}\n${separator}`)) {
      return { ok: false, error: BYTE_LIMIT_ERROR };
    }
    for (const row of rows) {
      if (!builder.append("\n| ")) return { ok: false, error: BYTE_LIMIT_ERROR };
      for (let c = 0; c < columns.length; c++) {
        if (c > 0 && !builder.append(" | ")) return { ok: false, error: BYTE_LIMIT_ERROR };
        if (!builder.append(formatMarkdownCell(row[c]))) {
          return { ok: false, error: BYTE_LIMIT_ERROR };
        }
      }
      if (!builder.append(" |")) return { ok: false, error: BYTE_LIMIT_ERROR };
    }
  }

  return {
    ok: true,
    text: builder.finish(),
    byteLength: builder.byteLength,
    rowCount: rows.length,
    columnCount: columns.length,
  };
}

export async function writeClipboardText(text: string): Promise<void> {
  if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
    throw new Error("System clipboard is unavailable in this environment.");
  }
  await navigator.clipboard.writeText(text);
}
