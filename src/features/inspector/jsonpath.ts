export interface JsonPathMatch {
  path: string; // e.g. "$.store.book[0].title"
  normalizedPath: (string | number)[]; // e.g. ["store", "book", 0, "title"]
  value: unknown;
}

export type JsonPathResult =
  { ok: true; matches: JsonPathMatch[] } | { ok: false; error: string; matches: [] };

type Segment =
  | { kind: "prop"; key: string }
  | { kind: "wildcard" }
  | { kind: "index"; indices: number[] }
  | { kind: "slice"; start?: number; end?: number; step?: number }
  | { kind: "filter"; condition: (val: unknown) => boolean; raw: string }
  | { kind: "recursive"; key?: string; wildcard?: boolean };

/**
 * Parses and evaluates a JSONPath expression against a JS value.
 * Supports:
 * - Root: $
 * - Dot property: $.store.book, .store
 * - Wildcard: .* or [*]
 * - Bracket quotes: ['store'], ["book"]
 * - Array indices: [0], [-1], [0, 1]
 * - Array slice: [0:2], [:2], [1:], [-2:]
 * - Recursive descent: $..price, $..*
 * - Filter expressions: [?(@.price < 30)], [?(@.active == true)], [?(@.title)]
 */
export function evaluateJsonPath(query: string, rootValue: unknown): JsonPathResult {
  if (query === null || query === undefined || typeof query !== "string") {
    return {
      ok: false,
      error: "Invalid query: query must be a string",
      matches: [],
    };
  }

  const trimmed = query.trim();
  if (!trimmed || trimmed === "$") {
    return {
      ok: true,
      matches: [{ path: "$", normalizedPath: [], value: rootValue }],
    };
  }

  const parseOutcome = parseSegments(trimmed);
  if (!parseOutcome.ok) {
    return { ok: false, error: parseOutcome.error, matches: [] };
  }

  let current: JsonPathMatch[] = [{ path: "$", normalizedPath: [], value: rootValue }];

  for (const segment of parseOutcome.segments) {
    const next: JsonPathMatch[] = [];

    for (const item of current) {
      if (item.value === null || item.value === undefined) continue;

      if (segment.kind === "prop") {
        if (
          typeof item.value === "object" &&
          !Array.isArray(item.value) &&
          segment.key in (item.value as Record<string, unknown>)
        ) {
          const val = (item.value as Record<string, unknown>)[segment.key];
          next.push({
            path: `${item.path}.${segment.key}`,
            normalizedPath: [...item.normalizedPath, segment.key],
            value: val,
          });
        }
      } else if (segment.kind === "index") {
        if (Array.isArray(item.value)) {
          for (const rawIdx of segment.indices) {
            const idx = rawIdx < 0 ? item.value.length + rawIdx : rawIdx;
            if (idx >= 0 && idx < item.value.length) {
              next.push({
                path: `${item.path}[${idx}]`,
                normalizedPath: [...item.normalizedPath, idx],
                value: item.value[idx],
              });
            }
          }
        }
      } else if (segment.kind === "wildcard") {
        if (Array.isArray(item.value)) {
          item.value.forEach((element, i) => {
            next.push({
              path: `${item.path}[${i}]`,
              normalizedPath: [...item.normalizedPath, i],
              value: element,
            });
          });
        } else if (typeof item.value === "object") {
          for (const [key, element] of Object.entries(item.value as Record<string, unknown>)) {
            next.push({
              path: `${item.path}.${key}`,
              normalizedPath: [...item.normalizedPath, key],
              value: element,
            });
          }
        }
      } else if (segment.kind === "slice") {
        if (Array.isArray(item.value)) {
          const len = item.value.length;
          let start = segment.start ?? 0;
          let end = segment.end ?? len;
          const step = segment.step ?? 1;

          if (start < 0) start = Math.max(0, len + start);
          if (end < 0) end = Math.max(0, len + end);
          start = Math.min(start, len);
          end = Math.min(end, len);

          if (step > 0) {
            for (let i = start; i < end; i += step) {
              next.push({
                path: `${item.path}[${i}]`,
                normalizedPath: [...item.normalizedPath, i],
                value: item.value[i],
              });
            }
          }
        }
      } else if (segment.kind === "filter") {
        if (Array.isArray(item.value)) {
          item.value.forEach((element, i) => {
            if (segment.condition(element)) {
              next.push({
                path: `${item.path}[${i}]`,
                normalizedPath: [...item.normalizedPath, i],
                value: element,
              });
            }
          });
        }
      } else if (segment.kind === "recursive") {
        collectRecursive(
          item.value,
          item.path,
          item.normalizedPath,
          segment.key,
          segment.wildcard,
          next,
        );
      }
    }

    current = next;
  }

  return { ok: true, matches: current };
}

function collectRecursive(
  val: unknown,
  basePath: string,
  baseNormalized: (string | number)[],
  targetKey: string | undefined,
  wildcard: boolean | undefined,
  results: JsonPathMatch[],
  visited: WeakSet<object> = new WeakSet<object>(),
) {
  if (val === null || typeof val !== "object") return;
  if (visited.has(val as object)) return;
  visited.add(val as object);

  if (Array.isArray(val)) {
    val.forEach((item, index) => {
      const childPath = `${basePath}[${index}]`;
      const childNormalized = [...baseNormalized, index];
      if (wildcard) {
        results.push({ path: childPath, normalizedPath: childNormalized, value: item });
      }
      collectRecursive(item, childPath, childNormalized, targetKey, wildcard, results, visited);
    });
  } else {
    for (const [key, childVal] of Object.entries(val as Record<string, unknown>)) {
      const childPath = `${basePath}.${key}`;
      const childNormalized = [...baseNormalized, key];
      if (wildcard || (!targetKey && !wildcard) || key === targetKey) {
        results.push({ path: childPath, normalizedPath: childNormalized, value: childVal });
      }
      collectRecursive(childVal, childPath, childNormalized, targetKey, wildcard, results, visited);
    }
  }
}

function parseSegments(
  raw: string,
): { ok: true; segments: Segment[] } | { ok: false; error: string } {
  let str = raw.trim();
  if (str.startsWith("$")) str = str.slice(1);

  const segments: Segment[] = [];
  let i = 0;

  while (i < str.length) {
    if (str[i] === ".") {
      if (str[i + 1] === ".") {
        // Recursive descent ..
        i += 2;
        if (i >= str.length) {
          return { ok: false, error: "Trailing '..' in JSONPath expression." };
        }
        if (str[i] === "*") {
          segments.push({ kind: "recursive", wildcard: true });
          i++;
        } else if (str[i] === "[") {
          // e.g. $..[*] or $..['title']
          continue; // let loop parse bracket segment next
        } else {
          const match = /^([a-zA-Z0-9_$]+)/.exec(str.slice(i));
          if (!match)
            return { ok: false, error: `Invalid identifier after '..' at position ${i}.` };
          segments.push({ kind: "recursive", key: match[1] });
          i += match[1].length;
        }
        continue;
      }
      i++;
      if (i >= str.length) {
        return { ok: false, error: "Trailing '.' in JSONPath expression." };
      }
      if (str[i] === "*") {
        segments.push({ kind: "wildcard" });
        i++;
        continue;
      }
      const match = /^([a-zA-Z0-9_$]+)/.exec(str.slice(i));
      if (!match) return { ok: false, error: `Expected property name at position ${i}.` };
      segments.push({ kind: "prop", key: match[1] });
      i += match[1].length;
    } else if (str[i] === "[") {
      const close = findClosingBracket(str, i);
      if (close === -1) {
        return { ok: false, error: "Unclosed '[' in JSONPath expression." };
      }
      const inner = str.slice(i + 1, close).trim();
      i = close + 1;

      if (inner === "*") {
        segments.push({ kind: "wildcard" });
      } else if (inner.startsWith("?(") && inner.endsWith(")")) {
        // Filter expression [?(...)]
        const cond = compileFilterCondition(inner.slice(2, -1).trim());
        if (!cond.ok) {
          return { ok: false, error: cond.error };
        }
        segments.push({ kind: "filter", condition: cond.condition, raw: inner });
      } else if (inner.includes(":")) {
        // Array slice
        const parts = inner.split(":");
        const start = parts[0].trim() ? Number.parseInt(parts[0], 10) : undefined;
        const end = parts[1].trim() ? Number.parseInt(parts[1], 10) : undefined;
        const step = parts[2]?.trim() ? Number.parseInt(parts[2], 10) : undefined;
        if (
          (start !== undefined && Number.isNaN(start)) ||
          (end !== undefined && Number.isNaN(end)) ||
          (step !== undefined && Number.isNaN(step))
        ) {
          return { ok: false, error: `Invalid slice parameters in '[${inner}]'.` };
        }
        segments.push({ kind: "slice", start, end, step });
      } else if (inner.includes(",")) {
        // Multiple indices or multiple quoted props: [0, 1] or ['a', 'b']
        const items = inner.split(",").map((s) => s.trim());
        const allNumbers = items.every((s) => /^[+-]?\d+$/.test(s));
        if (allNumbers) {
          segments.push({ kind: "index", indices: items.map((s) => Number.parseInt(s, 10)) });
        } else {
          return { ok: false, error: `Unsupported comma-separated bracket token '[${inner}]'.` };
        }
      } else if (/^[+-]?\d+$/.test(inner)) {
        segments.push({ kind: "index", indices: [Number.parseInt(inner, 10)] });
      } else if (
        (inner.startsWith("'") && inner.endsWith("'")) ||
        (inner.startsWith('"') && inner.endsWith('"'))
      ) {
        segments.push({ kind: "prop", key: inner.slice(1, -1) });
      } else {
        return { ok: false, error: `Invalid bracket token '[${inner}]'.` };
      }
    } else {
      // Leading property without dot (e.g. "store.book")
      const match = /^([a-zA-Z0-9_$]+)/.exec(str.slice(i));
      if (match) {
        segments.push({ kind: "prop", key: match[1] });
        i += match[1].length;
      } else {
        return { ok: false, error: `Unexpected character '${str[i]}' at position ${i}.` };
      }
    }
  }

  return { ok: true, segments };
}

function findClosingBracket(str: string, openIndex: number): number {
  let depth = 0;
  let inQuote: "'" | '"' | null = null;
  for (let idx = openIndex; idx < str.length; idx++) {
    const ch = str[idx];
    if (inQuote) {
      if (ch === inQuote && str[idx - 1] !== "\\") inQuote = null;
    } else if (ch === "'" || ch === '"') {
      inQuote = ch;
    } else if (ch === "[") {
      depth++;
    } else if (ch === "]") {
      depth--;
      if (depth === 0) return idx;
    }
  }
  return -1;
}

function compileFilterCondition(
  expr: string,
): { ok: true; condition: (val: unknown) => boolean } | { ok: false; error: string } {
  // Supports:
  // @.prop == val
  // @.prop != val
  // @.prop < num
  // @.prop <= num
  // @.prop > num
  // @.prop >= num
  // @.prop (truthy check)
  const opMatch =
    /^@(?:\.([a-zA-Z0-9_$]+)|\[['"]([^'"]+)['"]\])\s*(===|==|!==|!=|<=|>=|<|>)\s*(.+)$/.exec(expr);
  if (opMatch) {
    const prop = opMatch[1] || opMatch[2];
    const op = opMatch[3];
    const rawExpected = opMatch[4].trim();

    let expected: unknown = rawExpected;
    if (rawExpected === "true") expected = true;
    else if (rawExpected === "false") expected = false;
    else if (rawExpected === "null") expected = null;
    else if (/^[+-]?\d+(?:\.\d+)?$/.test(rawExpected)) expected = Number(rawExpected);
    else if (
      (rawExpected.startsWith("'") && rawExpected.endsWith("'")) ||
      (rawExpected.startsWith('"') && rawExpected.endsWith('"'))
    ) {
      expected = rawExpected.slice(1, -1);
    }

    const condition = (item: unknown): boolean => {
      if (item === null || typeof item !== "object") return false;
      const actual = (item as Record<string, unknown>)[prop];
      switch (op) {
        case "==":
        case "===":
          return actual === expected;
        case "!=":
        case "!==":
          return actual !== expected;
        case "<":
          return typeof actual === "number" && typeof expected === "number" && actual < expected;
        case "<=":
          return typeof actual === "number" && typeof expected === "number" && actual <= expected;
        case ">":
          return typeof actual === "number" && typeof expected === "number" && actual > expected;
        case ">=":
          return typeof actual === "number" && typeof expected === "number" && actual >= expected;
        default:
          return false;
      }
    };

    return { ok: true, condition };
  }

  // Existence/truthy check: @.prop
  const propCheck = /^@(?:\.([a-zA-Z0-9_$]+)|\[['"]([^'"]+)['"]\])$/.exec(expr);
  if (propCheck) {
    const prop = propCheck[1] || propCheck[2];
    const condition = (item: unknown): boolean => {
      if (item === null || typeof item !== "object") return false;
      const actual = (item as Record<string, unknown>)[prop];
      return Boolean(actual);
    };
    return { ok: true, condition };
  }

  return { ok: false, error: `Invalid filter condition '[?(${expr})]'.` };
}
