import type { ColumnInfo, EngineKind, TableSchema } from "../../ipc/generated";
import { formatSqlTableName, quoteSqlIdentifier } from "../grid/copy-format";

export type SqlCodegenTemplate = "select" | "insert" | "update" | "delete";

export const SQL_CODEGEN_TEMPLATES: readonly {
  id: SqlCodegenTemplate;
  label: string;
}[] = [
  { id: "select", label: "SELECT" },
  { id: "insert", label: "INSERT" },
  { id: "update", label: "UPDATE" },
  { id: "delete", label: "DELETE" },
];

function allocateParamName(colName: string, index: number, used: Set<string>): string {
  let base = colName.replace(/[^A-Za-z0-9_]/g, "_");
  if (!base || /^_+$/.test(base)) {
    base = `p${index + 1}`;
  } else if (/^[0-9]/.test(base)) {
    base = `p_${base}`;
  }
  let candidate = base;
  let suffix = 2;
  while (used.has(candidate.toLowerCase())) {
    candidate = `${base}_${suffix++}`;
  }
  used.add(candidate.toLowerCase());
  return `:${candidate}`;
}

function buildWhereKeyClause(
  columns: readonly ColumnInfo[],
  primaryKey: readonly string[],
  engine: EngineKind,
  usedParams: Set<string>,
): string {
  const keyCols =
    primaryKey.length > 0
      ? primaryKey.map(
          (pkName) =>
            columns.find((c) => c.name === pkName) ?? {
              id: pkName,
              name: pkName,
              databaseType: "text",
              kind: "text" as const,
              nullable: false,
              primaryKey: true,
              foreignKey: false,
              editable: false,
              insertable: false,
              hasDefault: false,
            },
        )
      : columns.slice(0, 1);

  if (keyCols.length === 0) {
    return "1 = 0";
  }

  return keyCols
    .map(
      (col, idx) =>
        `${quoteSqlIdentifier(col.name, engine)} = ${allocateParamName(col.name, idx, usedParams)}`,
    )
    .join(" AND ");
}

/**
 * Generates dialect-quoted SQL statements (`SELECT`, `INSERT`, `UPDATE`, `DELETE`)
 * for a table or view schema without executing any SQL.
 */
export function generateTableSql(
  template: SqlCodegenTemplate,
  engine: EngineKind,
  schema: TableSchema,
): string {
  const targetTable = formatSqlTableName(schema.table, engine);
  const cols = schema.columns;

  switch (template) {
    case "select": {
      if (cols.length === 0) {
        return `SELECT *\nFROM ${targetTable}\nLIMIT 100;`;
      }
      const projection = cols.map((c) => `  ${quoteSqlIdentifier(c.name, engine)}`).join(",\n");
      const orderBy =
        schema.primaryKey.length > 0
          ? `\nORDER BY ${schema.primaryKey.map((pk) => quoteSqlIdentifier(pk, engine)).join(", ")}`
          : "";
      return `SELECT\n${projection}\nFROM ${targetTable}${orderBy}\nLIMIT 100;`;
    }
    case "insert": {
      const insertCols = cols.filter((c) => c.insertable);
      const effectiveCols =
        insertCols.length > 0 ? insertCols : cols.filter((c) => !(c.primaryKey && c.hasDefault));
      if (effectiveCols.length === 0) {
        return engine === "mysql"
          ? `INSERT INTO ${targetTable} () VALUES ();`
          : `INSERT INTO ${targetTable} DEFAULT VALUES;`;
      }
      const usedParams = new Set<string>();
      const colNames = effectiveCols.map((c) => quoteSqlIdentifier(c.name, engine)).join(", ");
      const values = effectiveCols
        .map((c, idx) => allocateParamName(c.name, idx, usedParams))
        .join(", ");
      return `INSERT INTO ${targetTable} (${colNames})\nVALUES (${values});`;
    }
    case "update": {
      const pkSet = new Set(schema.primaryKey);
      const mutableCols = cols.filter((c) => !pkSet.has(c.name) && c.editable);
      const fallbackCols =
        mutableCols.length > 0 ? mutableCols : cols.filter((c) => !pkSet.has(c.name));
      const setCols = fallbackCols.length > 0 ? fallbackCols : cols.slice(0, 1);
      const usedParams = new Set<string>();
      const setClause =
        setCols.length > 0
          ? setCols
              .map(
                (c, idx) =>
                  `  ${quoteSqlIdentifier(c.name, engine)} = ${allocateParamName(c.name, idx, usedParams)}`,
              )
              .join(",\n")
          : `  /* no mutable columns */`;
      const whereClause = buildWhereKeyClause(cols, schema.primaryKey, engine, usedParams);
      return `UPDATE ${targetTable}\nSET\n${setClause}\nWHERE ${whereClause};`;
    }
    case "delete": {
      const usedParams = new Set<string>();
      const whereClause = buildWhereKeyClause(cols, schema.primaryKey, engine, usedParams);
      return `DELETE FROM ${targetTable}\nWHERE ${whereClause};`;
    }
  }
}
