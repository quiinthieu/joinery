import type {
  ColumnInfo,
  ColumnKind,
  DataCompareReport,
  EngineKind,
  TableRef,
} from "../../ipc/generated";
import { formatSqlCell, formatSqlTableName, quoteSqlIdentifier } from "./copy-format";

export type DataSyncDirection = "leftToRight" | "rightToLeft";

export const MAX_SYNC_SQL_BYTES = 8 * 1024 * 1024;

export interface GenerateDataSyncSqlOptions {
  report: DataCompareReport;
  direction: DataSyncDirection;
  engine?: EngineKind;
  targetTable?: TableRef | null;
  targetColumns?: readonly ColumnInfo[];
  includeInsert?: boolean;
  includeUpdate?: boolean;
  includeDelete?: boolean;
}

export type GenerateDataSyncSqlResult =
  | {
      ok: true;
      sql: string;
      insertCount: number;
      updateCount: number;
      deleteCount: number;
      statementCount: number;
      warnings: string[];
    }
  | {
      ok: false;
      error: string;
    };

export function inferColumnKindFromType(rawType: string): ColumnKind {
  const lower = rawType.trim().toLowerCase();
  if (!lower) return "text";
  if (lower.includes("bool")) return "boolean";
  if (
    lower.includes("bytea") ||
    lower.includes("blob") ||
    lower.includes("varbinary") ||
    lower === "binary" ||
    lower.startsWith("binary(")
  ) {
    return "binary";
  }
  if (
    /^(?:big|small|tiny|medium)?int(?:eger|2|4|8)?\b/.test(lower) ||
    /^(?:big|small)?serial\b/.test(lower) ||
    /^number\s*\(\s*\d+\s*,\s*0\s*\)$/.test(lower)
  ) {
    return "integer";
  }
  if (
    lower.startsWith("numeric") ||
    lower.startsWith("decimal") ||
    lower.startsWith("number") ||
    lower.includes("float") ||
    lower.includes("double") ||
    lower === "real"
  ) {
    return "decimal";
  }
  if (lower.includes("json") || lower === "variant" || lower === "object" || lower === "array") {
    return "json";
  }
  if (lower.includes("date") || lower.includes("time")) {
    return "temporal";
  }
  return "text";
}

function makeSyntheticColumnInfo(name: string, dbType: string, isKey: boolean): ColumnInfo {
  return {
    id: name,
    name,
    databaseType: dbType,
    kind: inferColumnKindFromType(dbType),
    nullable: !isKey,
    primaryKey: isKey,
    foreignKey: false,
    editable: !isKey,
    insertable: true,
    hasDefault: false,
  };
}

/**
 * Generates a dialect-quoted synchronization SQL script (`INSERT`, `UPDATE`, `DELETE`)
 * from a `DataCompareReport` without executing any SQL.
 */
export function generateDataSyncSql(
  options: GenerateDataSyncSqlOptions,
): GenerateDataSyncSqlResult {
  const {
    report,
    direction,
    engine,
    targetTable,
    targetColumns = [],
    includeInsert = true,
    includeUpdate = true,
    includeDelete = false,
  } = options;

  if (report.alignedColumns.length === 0) {
    return { ok: false, error: "No aligned columns available to generate synchronization SQL." };
  }
  if (!includeInsert && !includeUpdate && !includeDelete) {
    return {
      ok: false,
      error: "Enable at least one statement category (INSERT, UPDATE, or DELETE).",
    };
  }

  const keyIndices: number[] = [];
  report.alignedColumns.forEach((col, idx) => {
    if (col.isKey) keyIndices.push(idx);
  });
  if ((includeUpdate || includeDelete) && keyIndices.length === 0) {
    return {
      ok: false,
      error: "At least one key column is required to generate UPDATE or DELETE statements.",
    };
  }

  const targetColMap = new Map<string, ColumnInfo>();
  const targetColMapLower = new Map<string, ColumnInfo>();
  for (const col of targetColumns) {
    targetColMap.set(col.name, col);
    targetColMapLower.set(col.name.toLowerCase(), col);
  }

  const resolvedCols = report.alignedColumns.map((aligned) => {
    const targetColName = direction === "leftToRight" ? aligned.rightName : aligned.leftName;
    const targetDbType = direction === "leftToRight" ? aligned.rightType : aligned.leftType;
    const known =
      targetColMap.get(targetColName) ?? targetColMapLower.get(targetColName.toLowerCase());
    return {
      alignedName: aligned.name,
      targetName: targetColName,
      quotedName: quoteSqlIdentifier(targetColName, engine),
      isKey: aligned.isKey,
      info: known ?? makeSyntheticColumnInfo(targetColName, targetDbType, aligned.isKey),
    };
  });

  const warnings: string[] = [];
  if (report.leftTruncated || report.rightTruncated) {
    warnings.push(
      "One or both compared snapshots were truncated; review missing/extra row statements carefully before running.",
    );
  }
  if (report.diffsTruncated) {
    warnings.push(
      `Comparison output stopped at ${report.diffs.length.toLocaleString()} row differences; additional rows are omitted.`,
    );
  }
  const unalignedTargetCols =
    direction === "leftToRight" ? report.unalignedRightColumns : report.unalignedLeftColumns;
  if (unalignedTargetCols.length > 0) {
    warnings.push(
      `Target-only columns (${unalignedTargetCols.join(", ")}) rely on table defaults during INSERT.`,
    );
  }
  const anomalyTotal =
    report.leftAnomalies.duplicateKeyRows +
    report.rightAnomalies.duplicateKeyRows +
    report.leftAnomalies.nullKeyCount +
    report.rightAnomalies.nullKeyCount;
  if (anomalyTotal > 0) {
    warnings.push(
      `${anomalyTotal.toLocaleString()} duplicate-key or NULL-key rows were excluded from 1:1 synchronization.`,
    );
  }

  const sourceLabel = direction === "leftToRight" ? report.leftLabel : report.rightLabel;
  const destLabel = direction === "leftToRight" ? report.rightLabel : report.leftLabel;
  const tableSql = formatSqlTableName(targetTable ?? { schema: "", name: "target_table" }, engine);

  const chunks: string[] = [];
  let totalBytes = 0;
  const encoder = new TextEncoder();
  const pushChunk = (text: string): boolean => {
    const bytes = encoder.encode(text).byteLength;
    if (totalBytes + bytes > MAX_SYNC_SQL_BYTES) return false;
    chunks.push(text);
    totalBytes += bytes;
    return true;
  };

  let insertCount = 0;
  let updateCount = 0;
  let deleteCount = 0;

  const insertStatements: string[] = [];
  const updateStatements: string[] = [];
  const deleteStatements: string[] = [];

  const insertColList = resolvedCols.map((c) => c.quotedName).join(", ");

  for (const diff of report.diffs) {
    const isMissingInTarget =
      (direction === "leftToRight" && diff.kind === "onlyLeft") ||
      (direction === "rightToLeft" && diff.kind === "onlyRight");
    const isExtraInTarget =
      (direction === "leftToRight" && diff.kind === "onlyRight") ||
      (direction === "rightToLeft" && diff.kind === "onlyLeft");

    if (isMissingInTarget && includeInsert) {
      const sourceRow = direction === "leftToRight" ? diff.leftRow : diff.rightRow;
      if (!sourceRow) continue;
      const values = resolvedCols
        .map((col, idx) => formatSqlCell(sourceRow[idx] ?? null, col.info, engine))
        .join(", ");
      insertStatements.push(`INSERT INTO ${tableSql} (${insertColList}) VALUES (${values});`);
      insertCount++;
    } else if (diff.kind === "different" && includeUpdate) {
      const sourceRow = direction === "leftToRight" ? diff.leftRow : diff.rightRow;
      const targetRow = direction === "leftToRight" ? diff.rightRow : diff.leftRow;
      const changedAlignedNames = new Set(diff.cells.map((c) => c.column));
      const setParts: string[] = [];
      resolvedCols.forEach((col, idx) => {
        if (col.isKey || !changedAlignedNames.has(col.alignedName)) return;
        const nextVal = sourceRow
          ? (sourceRow[idx] ?? null)
          : (diff.cells.find((c) => c.column === col.alignedName)?.[
              direction === "leftToRight" ? "left" : "right"
            ] ?? null);
        setParts.push(`${col.quotedName} = ${formatSqlCell(nextVal, col.info, engine)}`);
      });
      if (setParts.length === 0) continue;
      const whereParts = keyIndices.map((kIdx) => {
        const keyCol = resolvedCols[kIdx];
        const keyVal = targetRow ? (targetRow[kIdx] ?? null) : (sourceRow?.[kIdx] ?? null);
        return keyVal === null
          ? `${keyCol.quotedName} IS NULL`
          : `${keyCol.quotedName} = ${formatSqlCell(keyVal, keyCol.info, engine)}`;
      });
      updateStatements.push(
        `UPDATE ${tableSql} SET ${setParts.join(", ")} WHERE ${whereParts.join(" AND ")};`,
      );
      updateCount++;
    } else if (isExtraInTarget && includeDelete) {
      const targetRow = direction === "leftToRight" ? diff.rightRow : diff.leftRow;
      if (!targetRow) continue;
      const whereParts = keyIndices.map((kIdx) => {
        const keyCol = resolvedCols[kIdx];
        const keyVal = targetRow[kIdx] ?? null;
        return keyVal === null
          ? `${keyCol.quotedName} IS NULL`
          : `${keyCol.quotedName} = ${formatSqlCell(keyVal, keyCol.info, engine)}`;
      });
      deleteStatements.push(`DELETE FROM ${tableSql} WHERE ${whereParts.join(" AND ")};`);
      deleteCount++;
    }
  }

  const statementCount = insertCount + updateCount + deleteCount;
  const headerLines = [
    `-- Joinery Data Synchronization Script`,
    `-- Source: ${sourceLabel}`,
    `-- Target: ${destLabel} (${tableSql})`,
    `-- Key columns: ${report.keyColumns.join(", ")}`,
    `-- Statements: ${insertCount} INSERT, ${updateCount} UPDATE, ${deleteCount} DELETE`,
    ...warnings.map((w) => `-- WARNING: ${w}`),
  ];

  if (!pushChunk(`${headerLines.join("\n")}\n`)) {
    return { ok: false, error: "Synchronization script exceeds the 8 MiB editor limit." };
  }

  if (statementCount === 0) {
    pushChunk("\n-- No matching row differences for the selected statement categories.\n");
  } else {
    for (const group of [insertStatements, updateStatements, deleteStatements]) {
      if (group.length === 0) continue;
      if (!pushChunk(`\n${group.join("\n")}\n`)) {
        return {
          ok: false,
          error: "Synchronization script exceeds the 8 MiB limit. Filter the compared slice first.",
        };
      }
    }
  }

  return {
    ok: true,
    sql: chunks.join(""),
    insertCount,
    updateCount,
    deleteCount,
    statementCount,
    warnings,
  };
}
