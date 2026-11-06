export type SnowflakeCastType =
  | "string"
  | "number"
  | "int"
  | "float"
  | "boolean"
  | "date"
  | "timestamp_ntz"
  | "timestamp_tz"
  | "variant"
  | "array"
  | "object"
  | "none";

export interface SnowflakePathOptions {
  columnName: string;
  path: (string | number)[];
  valueType?: string;
  cast?: SnowflakeCastType | "auto";
  style?: "colon" | "bracket";
}

export const SNOWFLAKE_CAST_TYPES: { id: SnowflakeCastType; label: string }[] = [
  { id: "string", label: "::string" },
  { id: "number", label: "::number" },
  { id: "int", label: "::int" },
  { id: "float", label: "::float" },
  { id: "boolean", label: "::boolean" },
  { id: "date", label: "::date" },
  { id: "timestamp_ntz", label: "::timestamp_ntz" },
  { id: "timestamp_tz", label: "::timestamp_tz" },
  { id: "variant", label: "::variant" },
  { id: "array", label: "::array" },
  { id: "object", label: "::object" },
  { id: "none", label: "(No cast)" },
];

export const SNOWFLAKE_RESERVED_KEYWORDS: ReadonlySet<string> = new Set([
  "ALL",
  "ALTER",
  "ANALYZE",
  "AND",
  "ANY",
  "AS",
  "ASC",
  "AT",
  "BETWEEN",
  "BY",
  "CASE",
  "CAST",
  "CHECK",
  "CLUSTER",
  "COLUMN",
  "CONNECT",
  "CONNECTION",
  "CONSTRAINT",
  "COPY",
  "CREATE",
  "CROSS",
  "CURRENT",
  "CURRENT_DATE",
  "CURRENT_ROLE",
  "CURRENT_SESSION",
  "CURRENT_STATEMENT",
  "CURRENT_TIME",
  "CURRENT_TIMESTAMP",
  "CURRENT_USER",
  "DATABASE",
  "DEFAULT",
  "DELETE",
  "DESC",
  "DISTINCT",
  "DROP",
  "ELSE",
  "END",
  "EXCEPT",
  "EXISTS",
  "EXPLAIN",
  "FALSE",
  "FETCH",
  "FOLLOWING",
  "FOR",
  "FROM",
  "FULL",
  "GRANT",
  "GROUP",
  "GSCLUSTER",
  "HAVING",
  "IDENTIFIER",
  "ILIKE",
  "IN",
  "INCREMENT",
  "INNER",
  "INSERT",
  "INTERSECT",
  "INTO",
  "IS",
  "ISSUE",
  "JOIN",
  "LATERAL",
  "LEFT",
  "LIKE",
  "LIMIT",
  "LOCALTIME",
  "LOCALTIMESTAMP",
  "MATCHED",
  "MATERIALIZED",
  "MERGE",
  "MINUS",
  "NATURAL",
  "NOT",
  "NULL",
  "OF",
  "OFFSET",
  "ON",
  "OR",
  "ORDER",
  "ORGANIZATION",
  "OUTER",
  "OVER",
  "PARTITION",
  "PRECEDING",
  "QUALIFY",
  "RANGE",
  "RECURSIVE",
  "REGEXP",
  "REPLACE",
  "RETURNING",
  "REVOKE",
  "RIGHT",
  "RLIKE",
  "ROW",
  "ROWS",
  "SAMPLE",
  "SCHEMA",
  "SELECT",
  "SET",
  "SOME",
  "START",
  "TABLE",
  "TABLESAMPLE",
  "THEN",
  "TO",
  "TOP",
  "TRIGGER",
  "TRUE",
  "TRUNCATE",
  "TRY_CAST",
  "UNION",
  "UNIQUE",
  "UNNEST",
  "UPDATE",
  "USING",
  "VALUES",
  "VIEW",
  "WHEN",
  "WHENEVER",
  "WHERE",
  "WINDOW",
  "WITH",
]);

export function isSnowflakeSimpleIdentifier(str: string): boolean {
  if (!str) return false;
  if (!/^[a-zA-Z_][a-zA-Z0-9_$]*$/.test(str)) {
    return false;
  }
  return !SNOWFLAKE_RESERVED_KEYWORDS.has(str.toUpperCase());
}

export function inferDefaultSnowflakeCast(
  valueType?: string,
  sampleValue?: unknown,
): SnowflakeCastType {
  if (!valueType) return "string";
  switch (valueType) {
    case "string":
      return "string";
    case "number":
      return Number.isInteger(sampleValue) ? "int" : "number";
    case "int":
    case "integer":
      return "int";
    case "float":
      return "float";
    case "boolean":
      return "boolean";
    case "date":
      return "date";
    case "timestamp":
    case "timestamp_ntz":
      return "timestamp_ntz";
    case "timestamp_tz":
      return "timestamp_tz";
    case "array":
      return "array";
    case "object":
      return "object";
    case "null":
      return "string";
    default:
      return "variant";
  }
}

/**
 * Generates Snowflake SQL path expression for variant and JSON navigation.
 * Examples:
 * - payload_json:store.book[0].price::number
 * - payload_json:store.book[0].title::string
 * - tags_array[0]::string
 * - col:"user name".id::int
 */
export function generateSnowflakePath(options: SnowflakePathOptions): string {
  const { columnName, path, valueType, cast = "auto", style = "colon" } = options;
  if (!path || path.length === 0) return columnName;

  let expr = columnName;

  if (style === "bracket") {
    for (const segment of path) {
      if (typeof segment === "number") {
        expr += `[${segment}]`;
      } else {
        const escaped = segment.replace(/\\/g, "\\\\").replace(/'/g, "''");
        expr += `['${escaped}']`;
      }
    }
  } else {
    path.forEach((segment, idx) => {
      if (typeof segment === "number") {
        expr += `[${segment}]`;
      } else {
        const needsQuotes = !isSnowflakeSimpleIdentifier(segment);
        const formatted = needsQuotes ? `"${segment.replace(/"/g, '""')}"` : segment;
        if (idx === 0) {
          expr += `:${formatted}`;
        } else {
          expr += `.${formatted}`;
        }
      }
    });
  }

  const effectiveCast = cast === "auto" ? inferDefaultSnowflakeCast(valueType) : cast;

  if (effectiveCast && effectiveCast !== "none") {
    expr += `::${effectiveCast}`;
  }

  return expr;
}
