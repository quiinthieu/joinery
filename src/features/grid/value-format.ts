import type { ColumnInfo } from "../../ipc/generated";

export const MAX_HEX_PREVIEW_BYTES = 16 * 1024;
export const MAX_ARRAY_PREVIEW_ELEMENTS = 2_000;

export function isBinaryColumn(column: Pick<ColumnInfo, "kind" | "databaseType">): boolean {
  return column.kind === "binary";
}

export function isPostgresArrayColumn(column: Pick<ColumnInfo, "kind" | "databaseType">): boolean {
  const dbType = column.databaseType.trim();
  if (dbType.endsWith("[]")) return true;
  return column.kind === "unknown" && /^_[a-z][a-z0-9_]*$/i.test(dbType);
}

export function isJsonColumn(column: Pick<ColumnInfo, "kind" | "databaseType">): boolean {
  return column.kind === "json" && !isPostgresArrayColumn(column);
}

export function opensInValueDialog(column: Pick<ColumnInfo, "kind" | "databaseType">): boolean {
  return (
    column.kind === "json" ||
    column.kind === "temporal" ||
    column.kind === "binary" ||
    isPostgresArrayColumn(column)
  );
}

export interface BinaryPreview {
  bytes: Uint8Array;
  totalBytes: number;
  truncated: boolean;
  encoding: "hex" | "escaped";
}

export function decodeBinaryPreview(raw: string, maxBytes = MAX_HEX_PREVIEW_BYTES): BinaryPreview {
  if (
    raw.startsWith("\\x") ||
    raw.startsWith("\\X") ||
    raw.startsWith("0x") ||
    raw.startsWith("0X")
  ) {
    const body = raw.slice(2);
    if (body.length % 2 === 0 && /^[0-9a-fA-F]*$/.test(body)) {
      const totalBytes = body.length / 2;
      const previewLength = Math.min(totalBytes, maxBytes);
      const bytes = new Uint8Array(previewLength);
      for (let i = 0; i < previewLength; i++) {
        bytes[i] = Number.parseInt(body.slice(i * 2, i * 2 + 2), 16);
      }
      return {
        bytes,
        totalBytes,
        truncated: totalBytes > maxBytes,
        encoding: "hex",
      };
    }
  }

  const encoder = new TextEncoder();
  const preview: number[] = [];
  let totalBytes = 0;
  let i = 0;
  while (i < raw.length) {
    if (raw[i] === "\\") {
      if (i + 1 < raw.length && raw[i + 1] === "\\") {
        if (preview.length < maxBytes) preview.push(0x5c);
        totalBytes++;
        i += 2;
        continue;
      }
      if (
        i + 3 < raw.length &&
        raw[i + 1] >= "0" &&
        raw[i + 1] <= "3" &&
        raw[i + 2] >= "0" &&
        raw[i + 2] <= "7" &&
        raw[i + 3] >= "0" &&
        raw[i + 3] <= "7"
      ) {
        const byte = Number.parseInt(raw.slice(i + 1, i + 4), 8);
        if (preview.length < maxBytes) preview.push(byte);
        totalBytes++;
        i += 4;
        continue;
      }
    }
    const codePoint = raw.codePointAt(i)!;
    const ch = String.fromCodePoint(codePoint);
    const encoded = encoder.encode(ch);
    for (const byte of encoded) {
      if (preview.length < maxBytes) preview.push(byte);
      totalBytes++;
    }
    i += ch.length;
  }

  return {
    bytes: Uint8Array.from(preview),
    totalBytes,
    truncated: totalBytes > maxBytes,
    encoding: "escaped",
  };
}

export function formatHexDump(bytes: Uint8Array): string {
  if (bytes.length === 0) return "";
  const lines: string[] = [];
  for (let offset = 0; offset < bytes.length; offset += 16) {
    const rowEnd = Math.min(offset + 16, bytes.length);
    const address = offset.toString(16).padStart(8, "0");
    const hexCells: string[] = [];
    for (let i = 0; i < 16; i++) {
      hexCells.push(offset + i < rowEnd ? bytes[offset + i].toString(16).padStart(2, "0") : "  ");
    }
    const hexPart = `${hexCells.slice(0, 8).join(" ")}  ${hexCells.slice(8, 16).join(" ")}`;
    let ascii = "";
    for (let i = offset; i < rowEnd; i++) {
      const byte = bytes[i];
      ascii += byte >= 0x20 && byte <= 0x7e ? String.fromCharCode(byte) : ".";
    }
    lines.push(`${address}  ${hexPart}  |${ascii}|`);
  }
  return lines.join("\n");
}

export interface PostgresArrayElement {
  path: number[];
  indexLabel: string;
  value: string | null;
  isNull: boolean;
}

export type PostgresArrayParseResult =
  | {
      ok: true;
      elements: PostgresArrayElement[];
      totalElements: number;
      dimensions: number;
      truncated: boolean;
    }
  | {
      ok: false;
      error: string;
    };

function isAsciiSpace(ch: string): boolean {
  return ch === " " || ch === "\t" || ch === "\r" || ch === "\n";
}

export function parsePostgresArray(
  raw: string,
  maxElements = MAX_ARRAY_PREVIEW_ELEMENTS,
): PostgresArrayParseResult {
  const input = raw.trim();
  let pos = 0;
  const lowerBounds: number[] = [];

  while (input.startsWith("[", pos)) {
    const match = /^\[([+-]?\d+):([+-]?\d+)\]/.exec(input.slice(pos));
    if (!match) {
      return { ok: false, error: "Invalid PostgreSQL array dimension bounds." };
    }
    lowerBounds.push(Number.parseInt(match[1], 10));
    pos += match[0].length;
  }

  if (lowerBounds.length > 0) {
    if (input[pos] !== "=") {
      return { ok: false, error: "Expected '=' after PostgreSQL array bounds." };
    }
    pos++;
  }

  if (input[pos] !== "{") {
    return { ok: false, error: "Expected a PostgreSQL array literal starting with '{'." };
  }

  // Empty array `{}`
  let probe = pos + 1;
  while (probe < input.length && isAsciiSpace(input[probe])) probe++;
  if (probe < input.length && input[probe] === "}") {
    probe++;
    while (probe < input.length && isAsciiSpace(input[probe])) probe++;
    if (probe !== input.length) {
      return { ok: false, error: "Unexpected trailing characters after array literal." };
    }
    return {
      ok: true,
      elements: [],
      totalElements: 0,
      dimensions: 0,
      truncated: false,
    };
  }

  const elements: PostgresArrayElement[] = [];
  let totalElements = 0;
  let maxDepth = 0;
  let truncated = false;

  const recordElement = (path: number[], value: string | null, isNull: boolean) => {
    totalElements++;
    if (elements.length < maxElements) {
      elements.push({
        path,
        indexLabel: path.map((idx) => `[${idx}]`).join(""),
        value,
        isNull,
      });
    } else {
      truncated = true;
    }
  };

  const skipSpaces = () => {
    while (pos < input.length && isAsciiSpace(input[pos])) pos++;
  };

  const parseGroup = (depth: number, prefix: number[]) => {
    if (input[pos] !== "{") {
      throw new Error("Expected '{' at start of array dimension.");
    }
    pos++;
    const lower = lowerBounds[depth] ?? 1;
    let offset = 0;
    let expectSubArray: boolean | null = null;

    while (true) {
      skipSpaces();
      if (pos >= input.length) {
        throw new Error("Unterminated PostgreSQL array literal.");
      }
      const ch = input[pos];
      if (ch === "}") {
        throw new Error(
          offset === 0
            ? "Empty nested sub-array is not valid in PostgreSQL."
            : "Unexpected '}' after ','.",
        );
      }

      const itemPath = [...prefix, lower + offset];
      if (ch === "{") {
        if (expectSubArray === false) {
          throw new Error("Mixed scalar and nested array elements at the same level.");
        }
        expectSubArray = true;
        parseGroup(depth + 1, itemPath);
      } else if (ch === '"') {
        if (expectSubArray === true) {
          throw new Error("Mixed nested array and scalar elements at the same level.");
        }
        expectSubArray = false;
        maxDepth = Math.max(maxDepth, depth + 1);
        pos++;
        let value = "";
        while (pos < input.length && input[pos] !== '"') {
          if (input[pos] === "\\") {
            pos++;
            if (pos >= input.length) {
              throw new Error("Unterminated escape in quoted array element.");
            }
            value += input[pos++];
          } else {
            value += input[pos++];
          }
        }
        if (input[pos] !== '"') {
          throw new Error("Unterminated quoted array element.");
        }
        pos++;
        recordElement(itemPath, value, false);
      } else {
        if (expectSubArray === true) {
          throw new Error("Mixed nested array and scalar elements at the same level.");
        }
        expectSubArray = false;
        maxDepth = Math.max(maxDepth, depth + 1);
        let rawToken = "";
        let hadEscape = false;
        while (pos < input.length && input[pos] !== "," && input[pos] !== "}") {
          if (input[pos] === "{" || input[pos] === '"') {
            throw new Error(`Unexpected '${input[pos]}' in unquoted array element.`);
          }
          if (input[pos] === "\\") {
            hadEscape = true;
            pos++;
            if (pos >= input.length) {
              throw new Error("Unterminated escape in array element.");
            }
            rawToken += input[pos++];
          } else {
            rawToken += input[pos++];
          }
        }
        const token = rawToken.trimEnd();
        if (token.length === 0) {
          throw new Error("Empty unquoted array element.");
        }
        const isNull = !hadEscape && /^null$/i.test(token);
        recordElement(itemPath, isNull ? null : token, isNull);
      }

      offset++;
      skipSpaces();
      if (pos >= input.length) {
        throw new Error("Unterminated PostgreSQL array literal.");
      }
      if (input[pos] === ",") {
        pos++;
        continue;
      }
      if (input[pos] === "}") {
        pos++;
        break;
      }
      throw new Error(`Expected ',' or '}' in array literal, found '${input[pos]}'.`);
    }
  };

  try {
    parseGroup(0, []);
    skipSpaces();
    if (pos !== input.length) {
      return { ok: false, error: "Unexpected trailing characters after array literal." };
    }
    return {
      ok: true,
      elements,
      totalElements,
      dimensions: maxDepth,
      truncated,
    };
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : "Invalid PostgreSQL array literal.",
    };
  }
}

export type JsonFormatResult =
  | {
      ok: true;
      formatted: string;
    }
  | {
      ok: false;
      error: string;
    };

/**
 * Validates JSON syntax with native `JSON.parse`, then pretty-prints tokens
 * directly so 64-bit integers, high-precision decimals, and object key order
 * are preserved without IEEE-754 float64 coercion or V8 integer-key sorting.
 */
export function formatJsonLossless(raw: string): JsonFormatResult {
  const trimmed = raw.trim();
  try {
    JSON.parse(trimmed);
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : "Invalid JSON value.",
    };
  }

  let out = "";
  let depth = 0;
  let i = 0;

  const nextNonSpace = (from: number): number => {
    let idx = from;
    while (idx < trimmed.length && isAsciiSpace(trimmed[idx])) idx++;
    return idx;
  };

  while (i < trimmed.length) {
    const ch = trimmed[i];
    if (isAsciiSpace(ch)) {
      i++;
      continue;
    }
    if (ch === '"') {
      const start = i;
      i++;
      while (i < trimmed.length) {
        if (trimmed[i] === "\\") {
          i += 2;
          continue;
        }
        if (trimmed[i] === '"') {
          i++;
          break;
        }
        i++;
      }
      out += trimmed.slice(start, i);
      continue;
    }
    if (ch === "{" || ch === "[") {
      const closeChar = ch === "{" ? "}" : "]";
      const peek = nextNonSpace(i + 1);
      if (trimmed[peek] === closeChar) {
        out += `${ch}${closeChar}`;
        i = peek + 1;
        continue;
      }
      depth++;
      out += `${ch}\n${"  ".repeat(depth)}`;
      i++;
      continue;
    }
    if (ch === "}" || ch === "]") {
      depth = Math.max(0, depth - 1);
      out += `\n${"  ".repeat(depth)}${ch}`;
      i++;
      continue;
    }
    if (ch === ",") {
      out += `,\n${"  ".repeat(depth)}`;
      i++;
      continue;
    }
    if (ch === ":") {
      out += ": ";
      i++;
      continue;
    }
    const start = i;
    while (
      i < trimmed.length &&
      !isAsciiSpace(trimmed[i]) &&
      trimmed[i] !== "," &&
      trimmed[i] !== "}" &&
      trimmed[i] !== "]" &&
      trimmed[i] !== ":"
    ) {
      i++;
    }
    out += trimmed.slice(start, i);
  }

  return { ok: true, formatted: out };
}
