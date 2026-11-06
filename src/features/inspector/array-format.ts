import { parsePostgresArray } from "../grid/value-format";

export type ArrayElementKind =
  "string" | "number" | "boolean" | "null" | "object" | "array" | "unknown";

export interface ArrayElementRow {
  index: number;
  indexLabel: string; // e.g. "[0]", "[1]" (JSON) or "[1]", "[1][2]" (PG)
  path: (string | number)[];
  raw: unknown;
  displayValue: string;
  kind: ArrayElementKind;
  lengthOrSize: string; // e.g. "12 chars", "int", "float", "3 items", "2 keys", "—"
  isNull: boolean;
  isNested: boolean;
  childCount?: number;
}

export interface UniversalArrayResult {
  ok: boolean;
  format: "postgres" | "json" | null;
  dimensions: number;
  totalElements: number;
  elements: ArrayElementRow[];
  truncated: boolean;
  error?: string;
}

export const MAX_ARRAY_ELEMENTS = 2_000;

/**
 * Universal array parser supporting PostgreSQL arrays ({...}) and JSON arrays ([...]).
 */
export function parseUniversalArray(
  raw: string | null | undefined,
  maxElements = MAX_ARRAY_ELEMENTS,
): UniversalArrayResult {
  if (raw === null || raw === undefined) {
    return {
      ok: false,
      format: null,
      dimensions: 0,
      totalElements: 0,
      elements: [],
      truncated: false,
      error: "Value is null",
    };
  }

  const str = raw.trim();
  if (!str) {
    return {
      ok: false,
      format: null,
      dimensions: 0,
      totalElements: 0,
      elements: [],
      truncated: false,
      error: "Value is empty",
    };
  }

  // 1. Try JSON Array first if starts with '['
  if (str.startsWith("[")) {
    try {
      const parsed = JSON.parse(str);
      if (Array.isArray(parsed)) {
        return parseJsonArray(parsed, maxElements);
      }
    } catch {
      // not valid JSON, check Postgres array below
    }
  }

  // 2. Try PostgreSQL Array if starts with '{' or '[...:...]='
  if (str.startsWith("{") || (str.startsWith("[") && str.includes("="))) {
    const pg = parsePostgresArray(str, maxElements);
    if (pg.ok) {
      return convertPostgresArray(pg);
    }
  }

  // 3. Fallback: try JSON.parse in case of leading whitespace or wrapped quotes
  try {
    const parsed = JSON.parse(str);
    if (Array.isArray(parsed)) {
      return parseJsonArray(parsed, maxElements);
    }
  } catch {
    // not valid
  }

  return {
    ok: false,
    format: null,
    dimensions: 0,
    totalElements: 0,
    elements: [],
    truncated: false,
    error: "Unrecognized array format (expected JSON array or PostgreSQL array literal)",
  };
}

function parseJsonArray(arr: unknown[], maxElements: number): UniversalArrayResult {
  const elements: ArrayElementRow[] = [];
  const totalElements = arr.length;
  const truncated = totalElements > maxElements;
  const count = Math.min(totalElements, maxElements);

  let maxDepth = 1;

  for (let i = 0; i < count; i++) {
    const item = arr[i];
    const row = classifyJsonElement(item, i, [i]);
    elements.push(row);
    if (Array.isArray(item)) maxDepth = Math.max(maxDepth, 2);
  }

  return {
    ok: true,
    format: "json",
    dimensions: maxDepth,
    totalElements,
    elements,
    truncated,
  };
}

function classifyJsonElement(
  item: unknown,
  index: number,
  path: (string | number)[],
): ArrayElementRow {
  const indexLabel = `[${index}]`;

  if (item === null || item === undefined) {
    return {
      index,
      indexLabel,
      path,
      raw: item,
      displayValue: "NULL",
      kind: "null",
      lengthOrSize: "—",
      isNull: true,
      isNested: false,
    };
  }

  if (typeof item === "string") {
    return {
      index,
      indexLabel,
      path,
      raw: item,
      displayValue: item,
      kind: "string",
      lengthOrSize: `${item.length} char${item.length === 1 ? "" : "s"}`,
      isNull: false,
      isNested: false,
    };
  }

  if (typeof item === "number") {
    return {
      index,
      indexLabel,
      path,
      raw: item,
      displayValue: String(item),
      kind: "number",
      lengthOrSize: Number.isInteger(item) ? "int" : "float",
      isNull: false,
      isNested: false,
    };
  }

  if (typeof item === "boolean") {
    return {
      index,
      indexLabel,
      path,
      raw: item,
      displayValue: String(item),
      kind: "boolean",
      lengthOrSize: "bool",
      isNull: false,
      isNested: false,
    };
  }

  if (Array.isArray(item)) {
    return {
      index,
      indexLabel,
      path,
      raw: item,
      displayValue: JSON.stringify(item),
      kind: "array",
      lengthOrSize: `${item.length} item${item.length === 1 ? "" : "s"}`,
      isNull: false,
      isNested: true,
      childCount: item.length,
    };
  }

  if (typeof item === "object") {
    const keys = Object.keys(item as Record<string, unknown>);
    return {
      index,
      indexLabel,
      path,
      raw: item,
      displayValue: JSON.stringify(item),
      kind: "object",
      lengthOrSize: `${keys.length} key${keys.length === 1 ? "" : "s"}`,
      isNull: false,
      isNested: true,
      childCount: keys.length,
    };
  }

  return {
    index,
    indexLabel,
    path,
    raw: item,
    displayValue: String(item),
    kind: "unknown",
    lengthOrSize: "—",
    isNull: false,
    isNested: false,
  };
}

function convertPostgresArray(
  pg: Extract<ReturnType<typeof parsePostgresArray>, { ok: true }>,
): UniversalArrayResult {
  const elements: ArrayElementRow[] = pg.elements.map((elem, idx) => {
    if (elem.isNull || elem.value === null) {
      return {
        index: idx,
        indexLabel: elem.indexLabel,
        path: elem.path,
        raw: null,
        displayValue: "NULL",
        kind: "null",
        lengthOrSize: "—",
        isNull: true,
        isNested: false,
      };
    }

    const val = elem.value;

    // Check if numeric
    if (/^[+-]?\d+$/.test(val)) {
      return {
        index: idx,
        indexLabel: elem.indexLabel,
        path: elem.path,
        raw: Number(val),
        displayValue: val,
        kind: "number",
        lengthOrSize: "int",
        isNull: false,
        isNested: false,
      };
    }

    if (/^[+-]?\d+\.\d+$/.test(val)) {
      return {
        index: idx,
        indexLabel: elem.indexLabel,
        path: elem.path,
        raw: Number(val),
        displayValue: val,
        kind: "number",
        lengthOrSize: "float",
        isNull: false,
        isNested: false,
      };
    }

    // Check if boolean
    if (/^(true|false|t|f)$/i.test(val)) {
      const b = /^t(rue)?$/i.test(val);
      return {
        index: idx,
        indexLabel: elem.indexLabel,
        path: elem.path,
        raw: b,
        displayValue: String(b),
        kind: "boolean",
        lengthOrSize: "bool",
        isNull: false,
        isNested: false,
      };
    }

    // String default
    return {
      index: idx,
      indexLabel: elem.indexLabel,
      path: elem.path,
      raw: val,
      displayValue: val,
      kind: "string",
      lengthOrSize: `${val.length} char${val.length === 1 ? "" : "s"}`,
      isNull: false,
      isNested: false,
    };
  });

  return {
    ok: true,
    format: "postgres",
    dimensions: pg.dimensions,
    totalElements: pg.totalElements,
    elements,
    truncated: pg.truncated,
  };
}

/**
 * Filters array element rows by search term matching index, value, or kind.
 */
export function filterArrayElements(elements: ArrayElementRow[], query: string): ArrayElementRow[] {
  const q = query.trim().toLowerCase();
  if (!q) return elements;

  return elements.filter((row) => {
    if (row.indexLabel.toLowerCase().includes(q)) return true;
    if (row.displayValue.toLowerCase().includes(q)) return true;
    if (row.kind.toLowerCase().includes(q)) return true;
    return false;
  });
}

/**
 * Exports elements to standard JSON array.
 */
export function exportArrayAsJson(elements: ArrayElementRow[]): string {
  const rawList = elements.map((e) => (e.isNull ? null : e.raw));
  return JSON.stringify(rawList, null, 2);
}

/**
 * Exports elements to RFC 4180 CSV string.
 */
export function exportArrayAsCsv(elements: ArrayElementRow[]): string {
  const rows: string[] = ['"Index","Type","Value","Size"'];
  for (const e of elements) {
    const idx = e.indexLabel;
    const kind = e.kind;
    const val = e.displayValue.replace(/"/g, '""');
    const size = e.lengthOrSize;
    rows.push(`"${idx}","${kind}","${val}","${size}"`);
  }
  return rows.join("\n");
}
