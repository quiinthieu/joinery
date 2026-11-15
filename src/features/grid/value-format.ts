import type { ColumnInfo } from "../../ipc/generated";

export const MAX_HEX_PREVIEW_BYTES = 16 * 1024;
export const MAX_ARRAY_PREVIEW_ELEMENTS = 2_000;
export const MAX_JSON_TREE_NODES = 2_000;

export function isRawHexBinaryType(databaseType: string): boolean {
  return /^(binary|varbinary)\b/i.test(databaseType.trim());
}

export function isBinaryColumn(column: Pick<ColumnInfo, "kind" | "databaseType">): boolean {
  return (
    column.kind === "binary" ||
    /^(binary|varbinary|bytea|blob|tinyblob|mediumblob|longblob)\b/i.test(
      column.databaseType.trim(),
    )
  );
}

export function isPostgresArrayColumn(column: Pick<ColumnInfo, "kind" | "databaseType">): boolean {
  const dbType = column.databaseType.trim();
  if (dbType.endsWith("[]")) return true;
  return column.kind === "unknown" && /^_[a-z][a-z0-9_]*$/i.test(dbType);
}

export function isJsonColumn(column: Pick<ColumnInfo, "kind" | "databaseType">): boolean {
  if (isPostgresArrayColumn(column)) return false;
  return (
    column.kind === "json" ||
    /^(json|jsonb|variant|object|array)\b/i.test(column.databaseType.trim())
  );
}

export function isSpatialColumn(column: Pick<ColumnInfo, "kind" | "databaseType">): boolean {
  return /^(geography|geometry|point|linestring|polygon|multipoint|multilinestring|multipolygon|geometrycollection)\b/i.test(
    column.databaseType.trim(),
  );
}

export function opensInValueDialog(column: Pick<ColumnInfo, "kind" | "databaseType">): boolean {
  return (
    isJsonColumn(column) ||
    column.kind === "temporal" ||
    isBinaryColumn(column) ||
    isPostgresArrayColumn(column) ||
    isSpatialColumn(column)
  );
}

export interface BinaryPreview {
  bytes: Uint8Array;
  totalBytes: number;
  truncated: boolean;
  encoding: "hex" | "escaped";
}

function decodeHexPairs(body: string, maxBytes: number): BinaryPreview {
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

export function decodeBinaryPreview(
  raw: string,
  maxBytes = MAX_HEX_PREVIEW_BYTES,
  allowRawHex = false,
): BinaryPreview {
  if (
    raw.startsWith("\\x") ||
    raw.startsWith("\\X") ||
    raw.startsWith("0x") ||
    raw.startsWith("0X")
  ) {
    const body = raw.slice(2);
    if (body.length % 2 === 0 && /^[0-9a-fA-F]*$/.test(body)) {
      return decodeHexPairs(body, maxBytes);
    }
  }

  if (allowRawHex) {
    const trimmed = raw.trim();
    if (trimmed.length % 2 === 0 && /^[0-9a-fA-F]*$/.test(trimmed)) {
      return decodeHexPairs(trimmed, maxBytes);
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

export interface JsonTreeNode {
  path: string;
  keyLabel: string;
  depth: number;
  kind: "object" | "array" | "string" | "number" | "boolean" | "null";
  summary: string;
}

export type JsonTreeResult =
  | {
      ok: true;
      nodes: JsonTreeNode[];
      totalNodes: number;
      truncated: boolean;
    }
  | {
      ok: false;
      error: string;
    };

function formatRootColumnIdentifier(columnName: string): string {
  const clean = columnName.trim() || "VALUE";
  return /^[A-Za-z_][A-Za-z0-9_$]*$/.test(clean) ? clean : `"${clean.replaceAll('"', '""')}"`;
}

function appendSnowflakeObjectSegment(
  parentPath: string,
  key: string,
  usedSemiAccessor: boolean,
): { path: string; usedSemiAccessor: boolean } {
  if (/^[A-Za-z_][A-Za-z0-9_$]*$/.test(key)) {
    return {
      path: usedSemiAccessor ? `${parentPath}.${key}` : `${parentPath}:${key}`,
      usedSemiAccessor: true,
    };
  }
  return {
    path: `${parentPath}['${key.replaceAll("'", "''")}']`,
    usedSemiAccessor: true,
  };
}

/**
 * Lossless bounded semi-structured / JSON tree builder with Snowflake `:path`
 * expressions (`COL:customer.orders[0].id` and `COL['odd.key']`).
 * Preserves exact 64-bit integer and decimal text and object key order.
 */
export function buildJsonTree(
  raw: string,
  columnName = "VALUE",
  maxNodes = MAX_JSON_TREE_NODES,
): JsonTreeResult {
  const trimmed = raw.trim();
  try {
    JSON.parse(trimmed);
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : "Invalid JSON value.",
    };
  }

  const nodes: JsonTreeNode[] = [];
  let totalNodes = 0;
  let truncated = false;
  let pos = 0;

  const skipWs = () => {
    while (pos < trimmed.length && isAsciiSpace(trimmed[pos])) pos++;
  };

  const readStringLiteral = (): { decoded: string; rawLiteral: string } => {
    const start = pos;
    pos++; // '"'
    while (pos < trimmed.length) {
      if (trimmed[pos] === "\\") {
        pos += 2;
        continue;
      }
      if (trimmed[pos] === '"') {
        pos++;
        break;
      }
      pos++;
    }
    const rawLiteral = trimmed.slice(start, pos);
    const decoded = JSON.parse(rawLiteral) as string;
    return { decoded, rawLiteral };
  };

  const pushNode = (node: JsonTreeNode): number | null => {
    totalNodes++;
    if (nodes.length < maxNodes) {
      nodes.push(node);
      return nodes.length - 1;
    }
    truncated = true;
    return null;
  };

  const walkValue = (path: string, keyLabel: string, depth: number, usedSemiAccessor: boolean) => {
    skipWs();
    const ch = trimmed[pos];
    if (ch === "{") {
      pos++;
      const slot = pushNode({
        path,
        keyLabel,
        depth,
        kind: "object",
        summary: "{}",
      });
      skipWs();
      let count = 0;
      if (trimmed[pos] === "}") {
        pos++;
      } else {
        while (pos < trimmed.length) {
          skipWs();
          const { decoded: key } = readStringLiteral();
          skipWs();
          if (trimmed[pos] === ":") pos++;
          const child = appendSnowflakeObjectSegment(path, key, usedSemiAccessor);
          walkValue(child.path, key, depth + 1, child.usedSemiAccessor);
          count++;
          skipWs();
          if (trimmed[pos] === ",") {
            pos++;
            continue;
          }
          if (trimmed[pos] === "}") {
            pos++;
            break;
          }
          break;
        }
      }
      if (slot !== null) {
        nodes[slot].summary = count === 0 ? "{}" : `{${count} ${count === 1 ? "key" : "keys"}}`;
      }
      return;
    }

    if (ch === "[") {
      pos++;
      const slot = pushNode({
        path,
        keyLabel,
        depth,
        kind: "array",
        summary: "[]",
      });
      skipWs();
      let idx = 0;
      if (trimmed[pos] === "]") {
        pos++;
      } else {
        while (pos < trimmed.length) {
          walkValue(`${path}[${idx}]`, `[${idx}]`, depth + 1, usedSemiAccessor);
          idx++;
          skipWs();
          if (trimmed[pos] === ",") {
            pos++;
            continue;
          }
          if (trimmed[pos] === "]") {
            pos++;
            break;
          }
          break;
        }
      }
      if (slot !== null) {
        nodes[slot].summary = idx === 0 ? "[]" : `[${idx} ${idx === 1 ? "item" : "items"}]`;
      }
      return;
    }

    if (ch === '"') {
      const { decoded } = readStringLiteral();
      const clipped = decoded.length > 120 ? `${decoded.slice(0, 120)}…` : decoded;
      pushNode({
        path,
        keyLabel,
        depth,
        kind: "string",
        summary: JSON.stringify(clipped),
      });
      return;
    }

    const start = pos;
    while (
      pos < trimmed.length &&
      !isAsciiSpace(trimmed[pos]) &&
      trimmed[pos] !== "," &&
      trimmed[pos] !== "}" &&
      trimmed[pos] !== "]"
    ) {
      pos++;
    }
    const token = trimmed.slice(start, pos);
    const kind: JsonTreeNode["kind"] =
      token === "true" || token === "false" ? "boolean" : token === "null" ? "null" : "number";
    pushNode({
      path,
      keyLabel,
      depth,
      kind,
      summary: token,
    });
  };

  const rootPath = formatRootColumnIdentifier(columnName);
  walkValue(rootPath, rootPath, 0, false);

  return {
    ok: true,
    nodes,
    totalNodes,
    truncated,
  };
}

export interface SpatialSvgShape {
  kind: "point" | "polyline" | "polygon";
  cx?: number;
  cy?: number;
  d?: string;
}

export type SpatialPreviewResult =
  | {
      ok: true;
      format: "GeoJSON" | "WKT" | "EWKT";
      geometryType: string;
      srid: number | null;
      coordinateCount: number;
      bbox: [number, number, number, number] | null;
      shapes: SpatialSvgShape[];
      formatted: string;
    }
  | {
      ok: false;
      error: string;
    };

type RawSpatialPrimitive =
  | { kind: "point"; coord: [number, number] }
  | { kind: "polyline"; coords: [number, number][] }
  | { kind: "polygon"; rings: [number, number][][] };

function extractCoordPair(value: unknown): [number, number] {
  if (!Array.isArray(value) || value.length < 2) {
    throw new Error("Expected [x, y] coordinate pair.");
  }
  const x = Number(value[0]);
  const y = Number(value[1]);
  if (!Number.isFinite(x) || !Number.isFinite(y)) {
    throw new Error("Non-finite coordinate in spatial value.");
  }
  return [x, y];
}

function collectGeoJsonPrimitives(node: unknown, out: RawSpatialPrimitive[]): string {
  if (!node || typeof node !== "object" || Array.isArray(node)) {
    throw new Error("Invalid GeoJSON object.");
  }
  const obj = node as Record<string, unknown>;
  const type = typeof obj.type === "string" ? obj.type : "";
  switch (type) {
    case "Point": {
      if (Array.isArray(obj.coordinates) && obj.coordinates.length > 0) {
        out.push({ kind: "point", coord: extractCoordPair(obj.coordinates) });
      }
      return "Point";
    }
    case "MultiPoint": {
      if (!Array.isArray(obj.coordinates)) throw new Error("Invalid MultiPoint coordinates.");
      for (const c of obj.coordinates) {
        out.push({ kind: "point", coord: extractCoordPair(c) });
      }
      return "MultiPoint";
    }
    case "LineString": {
      if (!Array.isArray(obj.coordinates)) throw new Error("Invalid LineString coordinates.");
      const coords = obj.coordinates.map(extractCoordPair);
      if (coords.length > 0) out.push({ kind: "polyline", coords });
      return "LineString";
    }
    case "MultiLineString": {
      if (!Array.isArray(obj.coordinates)) throw new Error("Invalid MultiLineString coordinates.");
      for (const line of obj.coordinates) {
        if (!Array.isArray(line)) throw new Error("Invalid MultiLineString member.");
        const coords = line.map(extractCoordPair);
        if (coords.length > 0) out.push({ kind: "polyline", coords });
      }
      return "MultiLineString";
    }
    case "Polygon": {
      if (!Array.isArray(obj.coordinates)) throw new Error("Invalid Polygon coordinates.");
      const rings = obj.coordinates.map((ring) => {
        if (!Array.isArray(ring)) throw new Error("Invalid Polygon ring.");
        return ring.map(extractCoordPair);
      });
      if (rings.some((r) => r.length > 0)) out.push({ kind: "polygon", rings });
      return "Polygon";
    }
    case "MultiPolygon": {
      if (!Array.isArray(obj.coordinates)) throw new Error("Invalid MultiPolygon coordinates.");
      for (const poly of obj.coordinates) {
        if (!Array.isArray(poly)) throw new Error("Invalid MultiPolygon member.");
        const rings = poly.map((ring) => {
          if (!Array.isArray(ring)) throw new Error("Invalid Polygon ring.");
          return ring.map(extractCoordPair);
        });
        if (rings.some((r) => r.length > 0)) out.push({ kind: "polygon", rings });
      }
      return "MultiPolygon";
    }
    case "GeometryCollection": {
      if (!Array.isArray(obj.geometries)) throw new Error("Invalid GeometryCollection.");
      for (const child of obj.geometries) collectGeoJsonPrimitives(child, out);
      return "GeometryCollection";
    }
    case "Feature": {
      if (obj.geometry !== null && obj.geometry !== undefined) {
        const innerType = collectGeoJsonPrimitives(obj.geometry, out);
        return `Feature (${innerType})`;
      }
      return "Feature (null geometry)";
    }
    case "FeatureCollection": {
      if (!Array.isArray(obj.features)) throw new Error("Invalid FeatureCollection.");
      for (const feature of obj.features) collectGeoJsonPrimitives(feature, out);
      return "FeatureCollection";
    }
    default:
      throw new Error(`Unsupported GeoJSON type '${type || "unknown"}'.`);
  }
}

function parseWktGeometry(input: string, out: RawSpatialPrimitive[]): string {
  let pos = 0;
  const skipWs = () => {
    while (pos < input.length && isAsciiSpace(input[pos])) pos++;
  };
  const readWord = (): string => {
    skipWs();
    const start = pos;
    while (pos < input.length && /[A-Za-z]/.test(input[pos])) pos++;
    return input.slice(start, pos);
  };
  const consumeEmptyOrOpenParen = (): boolean => {
    skipWs();
    if (input[pos] === "(") {
      pos++;
      return false;
    }
    const word = readWord().toUpperCase();
    if (word === "EMPTY") return true;
    throw new Error("Expected '(' or 'EMPTY' in WKT geometry.");
  };
  const expectCloseParen = () => {
    skipWs();
    if (input[pos] !== ")") throw new Error("Expected ')' in WKT geometry.");
    pos++;
  };
  const readNumber = (): number => {
    skipWs();
    const match = /^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?/.exec(input.slice(pos));
    if (!match) throw new Error("Expected numeric coordinate in WKT.");
    pos += match[0].length;
    const val = Number(match[0]);
    if (!Number.isFinite(val)) throw new Error("Non-finite coordinate in WKT.");
    return val;
  };
  const readCoordTuple = (): [number, number] => {
    const x = readNumber();
    const y = readNumber();
    // Consume optional Z / M coordinates on the same vertex before ',' or ')'.
    while (pos < input.length) {
      skipWs();
      if (input[pos] === "," || input[pos] === ")" || pos >= input.length) break;
      readNumber();
    }
    return [x, y];
  };
  const readCoordSequence = (): [number, number][] => {
    const seq: [number, number][] = [];
    while (true) {
      skipWs();
      if (input[pos] === "(") {
        pos++;
        seq.push(readCoordTuple());
        expectCloseParen();
      } else {
        seq.push(readCoordTuple());
      }
      skipWs();
      if (input[pos] === ",") {
        pos++;
        continue;
      }
      break;
    }
    return seq;
  };

  const parseGeometry = (): string => {
    const rawType = readWord();
    const upper = rawType.toUpperCase();
    if (!upper) throw new Error("Expected WKT geometry type keyword.");
    // Optional Z / M / ZM dimension modifier (e.g. `POINT Z (...)`)
    const saved = pos;
    const dim = readWord().toUpperCase();
    if (dim !== "Z" && dim !== "M" && dim !== "ZM") {
      pos = saved;
    }

    switch (upper) {
      case "POINT": {
        if (consumeEmptyOrOpenParen()) return "POINT";
        const coord = readCoordTuple();
        expectCloseParen();
        out.push({ kind: "point", coord });
        return "POINT";
      }
      case "MULTIPOINT": {
        if (consumeEmptyOrOpenParen()) return "MULTIPOINT";
        const coords = readCoordSequence();
        expectCloseParen();
        for (const coord of coords) out.push({ kind: "point", coord });
        return "MULTIPOINT";
      }
      case "LINESTRING": {
        if (consumeEmptyOrOpenParen()) return "LINESTRING";
        const coords = readCoordSequence();
        expectCloseParen();
        if (coords.length > 0) out.push({ kind: "polyline", coords });
        return "LINESTRING";
      }
      case "MULTILINESTRING": {
        if (consumeEmptyOrOpenParen()) return "MULTILINESTRING";
        while (true) {
          if (!consumeEmptyOrOpenParen()) {
            const coords = readCoordSequence();
            expectCloseParen();
            if (coords.length > 0) out.push({ kind: "polyline", coords });
          }
          skipWs();
          if (input[pos] === ",") {
            pos++;
            continue;
          }
          break;
        }
        expectCloseParen();
        return "MULTILINESTRING";
      }
      case "POLYGON": {
        if (consumeEmptyOrOpenParen()) return "POLYGON";
        const rings: [number, number][][] = [];
        while (true) {
          if (!consumeEmptyOrOpenParen()) {
            rings.push(readCoordSequence());
            expectCloseParen();
          }
          skipWs();
          if (input[pos] === ",") {
            pos++;
            continue;
          }
          break;
        }
        expectCloseParen();
        if (rings.some((r) => r.length > 0)) out.push({ kind: "polygon", rings });
        return "POLYGON";
      }
      case "MULTIPOLYGON": {
        if (consumeEmptyOrOpenParen()) return "MULTIPOLYGON";
        while (true) {
          if (!consumeEmptyOrOpenParen()) {
            const rings: [number, number][][] = [];
            while (true) {
              if (!consumeEmptyOrOpenParen()) {
                rings.push(readCoordSequence());
                expectCloseParen();
              }
              skipWs();
              if (input[pos] === ",") {
                pos++;
                continue;
              }
              break;
            }
            expectCloseParen();
            if (rings.some((r) => r.length > 0)) out.push({ kind: "polygon", rings });
          }
          skipWs();
          if (input[pos] === ",") {
            pos++;
            continue;
          }
          break;
        }
        expectCloseParen();
        return "MULTIPOLYGON";
      }
      case "GEOMETRYCOLLECTION": {
        if (consumeEmptyOrOpenParen()) return "GEOMETRYCOLLECTION";
        while (true) {
          parseGeometry();
          skipWs();
          if (input[pos] === ",") {
            pos++;
            continue;
          }
          break;
        }
        expectCloseParen();
        return "GEOMETRYCOLLECTION";
      }
      default:
        throw new Error(`Unsupported WKT geometry type '${rawType}'.`);
    }
  };

  const topType = parseGeometry();
  skipWs();
  if (pos !== input.length) {
    throw new Error("Unexpected trailing characters after WKT geometry.");
  }
  return topType;
}

export function parseSpatialPreview(raw: string): SpatialPreviewResult {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: false, error: "Empty spatial value." };
  }

  const primitives: RawSpatialPrimitive[] = [];
  let format: "GeoJSON" | "WKT" | "EWKT" = "WKT";
  let srid: number | null = null;
  let geometryType = "";
  let formatted = trimmed;

  try {
    if (trimmed.startsWith("{")) {
      format = "GeoJSON";
      const parsed = JSON.parse(trimmed) as Record<string, unknown>;
      const crsName = (parsed.crs as { properties?: { name?: unknown } } | undefined)?.properties
        ?.name;
      if (typeof crsName === "string") {
        const epsgMatch = /(?:EPSG[:]+)(\d+)/i.exec(crsName);
        if (epsgMatch) srid = Number.parseInt(epsgMatch[1], 10);
      }
      if (typeof parsed.srid === "number" && Number.isInteger(parsed.srid)) {
        srid = parsed.srid;
      }
      geometryType = collectGeoJsonPrimitives(parsed, primitives);
      const lossless = formatJsonLossless(trimmed);
      formatted = lossless.ok ? lossless.formatted : JSON.stringify(parsed, null, 2);
    } else {
      let body = trimmed;
      const ewkt = /^SRID\s*=\s*(\d+)\s*;\s*/i.exec(body);
      if (ewkt) {
        format = "EWKT";
        srid = Number.parseInt(ewkt[1], 10);
        body = body.slice(ewkt[0].length);
      }
      geometryType = parseWktGeometry(body, primitives);
    }
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : "Invalid GeoJSON or WKT spatial value.",
    };
  }

  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  let coordinateCount = 0;

  const visit = ([x, y]: [number, number]) => {
    coordinateCount++;
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (x > maxX) maxX = x;
    if (y > maxY) maxY = y;
  };

  for (const prim of primitives) {
    if (prim.kind === "point") visit(prim.coord);
    else if (prim.kind === "polyline") prim.coords.forEach(visit);
    else prim.rings.forEach((ring) => ring.forEach(visit));
  }

  const bbox: [number, number, number, number] | null =
    coordinateCount > 0 ? [minX, minY, maxX, maxY] : null;

  const VIEW_W = 240;
  const VIEW_H = 160;
  const PAD = 18;
  const innerW = VIEW_W - PAD * 2;
  const innerH = VIEW_H - PAD * 2;
  const spanX = bbox ? maxX - minX : 0;
  const spanY = bbox ? maxY - minY : 0;
  const scale =
    spanX === 0 && spanY === 0
      ? 1
      : Math.min(spanX > 0 ? innerW / spanX : innerW, spanY > 0 ? innerH / spanY : innerH);
  const usedW = spanX * scale;
  const usedH = spanY * scale;
  const offsetX = PAD + (innerW - usedW) / 2;
  const offsetY = PAD + (innerH - usedH) / 2;

  const project = ([x, y]: [number, number]): [number, number] => {
    if (!bbox || (spanX === 0 && spanY === 0)) {
      return [VIEW_W / 2, VIEW_H / 2];
    }
    const px = spanX === 0 ? VIEW_W / 2 : offsetX + (x - minX) * scale;
    const py = spanY === 0 ? VIEW_H / 2 : VIEW_H - (offsetY + (y - minY) * scale);
    return [Number(px.toFixed(2)), Number(py.toFixed(2))];
  };

  const shapes: SpatialSvgShape[] = [];
  for (const prim of primitives) {
    if (prim.kind === "point") {
      const [cx, cy] = project(prim.coord);
      shapes.push({ kind: "point", cx, cy });
    } else if (prim.kind === "polyline") {
      if (prim.coords.length === 1) {
        const [cx, cy] = project(prim.coords[0]);
        shapes.push({ kind: "point", cx, cy });
      } else if (prim.coords.length > 1) {
        const d = prim.coords
          .map((c, idx) => {
            const [px, py] = project(c);
            return `${idx === 0 ? "M" : "L"}${px} ${py}`;
          })
          .join(" ");
        shapes.push({ kind: "polyline", d });
      }
    } else {
      const ringPaths = prim.rings
        .filter((ring) => ring.length > 0)
        .map((ring) => {
          const cmds = ring.map((c, idx) => {
            const [px, py] = project(c);
            return `${idx === 0 ? "M" : "L"}${px} ${py}`;
          });
          return `${cmds.join(" ")} Z`;
        });
      if (ringPaths.length > 0) {
        shapes.push({ kind: "polygon", d: ringPaths.join(" ") });
      }
    }
  }

  return {
    ok: true,
    format,
    geometryType,
    srid,
    coordinateCount,
    bbox,
    shapes,
    formatted,
  };
}
