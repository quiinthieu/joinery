import type { ColumnInfo, TableChanges } from "../../ipc/generated";
import type { DirtyCell, DirtyStore } from "../../data/dirty-store";
import type { ResultCache } from "../../data/result-cache";
import type { StudioTab } from "../../state/studio";

export type MutationKind = "update" | "insert" | "delete";

export interface CellDiff {
  columnId: string;
  columnName: string;
  isPrimaryKey: boolean;
  before: string | null;
  after: string | null;
  changed: boolean;
}

export interface RowDiff {
  id: string; // e.g. "update:res-1:0" | "delete:res-1:2" | "insert:tab-1:uuid"
  kind: MutationKind;
  resultId?: string;
  rowIndex?: number;
  insertId?: string;
  primaryKeySummary: string; // e.g. "id = 1" or "Row 3"
  tableName?: string;
  cells: CellDiff[];
  ref:
    | { kind: "update"; resultId: string; row: number }
    | { kind: "delete"; resultId: string; row: number }
    | { kind: "insert"; tabId: string; insertId: string };
}

export function buildMutationDiffs(
  tab: StudioTab,
  dirtyStore: DirtyStore,
  resultCache: ResultCache,
): RowDiff[] {
  const diffs: RowDiff[] = [];
  const activeResult =
    tab.kind === "query"
      ? (tab.results.find((r) => r.id === tab.activeResultId) ?? tab.results[0])
      : null;
  const schema = tab.kind === "table" ? tab.schema : null;
  const columns: ColumnInfo[] =
    tab.kind === "table" ? (schema?.columns ?? []) : (activeResult?.columns ?? []);
  const editability =
    activeResult?.editability?.status === "editable" ? activeResult.editability : null;
  const pkNames = new Set(
    tab.kind === "table" ? (schema?.primaryKey ?? []) : (editability?.primaryKey ?? []),
  );
  const tableName = tab.kind === "table" ? schema?.table.name : editability?.table.name;

  // 1. UPDATES
  const dirtyCells: DirtyCell[] =
    tab.kind === "query" && activeResult
      ? dirtyStore.forResult(activeResult.id)
      : tab.kind === "table"
        ? tab.pages.flatMap((p) => dirtyStore.forResult(p.result.id))
        : [];

  const updateMap = new Map<string, DirtyCell[]>();
  for (const cell of dirtyCells) {
    const key = `${cell.resultId}:${cell.row}`;
    const list = updateMap.get(key) ?? [];
    list.push(cell);
    updateMap.set(key, list);
  }

  for (const [key, cellList] of updateMap) {
    const { resultId, row } = cellList[0];
    const cellDict = new Map(cellList.map((c) => [c.column, c]));
    const pkParts: string[] = [];

    const cells: CellDiff[] = columns.map((col) => {
      const isPk = pkNames.has(col.name) || col.primaryKey;
      const dirty = cellDict.get(col.id);
      const original = dirty
        ? dirty.original
        : (resultCache.read(resultId, col.id, row, true)?.value ?? null);
      const current = dirty ? dirty.value : original;
      if (isPk) pkParts.push(`${col.name} = ${current ?? "NULL"}`);
      return {
        columnId: col.id,
        columnName: col.name,
        isPrimaryKey: isPk,
        before: original,
        after: current,
        changed: Boolean(dirty),
      };
    });

    diffs.push({
      id: `update:${key}`,
      kind: "update",
      resultId,
      rowIndex: row,
      primaryKeySummary: pkParts.length ? pkParts.join(", ") : `Row ${row + 1}`,
      tableName,
      cells,
      ref: { kind: "update", resultId, row },
    });
  }

  // 2. DELETES
  const resultIds =
    tab.kind === "table"
      ? tab.pages.map((p) => p.result.id)
      : activeResult
        ? [activeResult.id]
        : [];

  for (const resultId of resultIds) {
    const deletedRows = dirtyStore.deletesForResult(resultId);
    for (const row of deletedRows) {
      const pkParts: string[] = [];
      const cells: CellDiff[] = columns.map((col) => {
        const isPk = pkNames.has(col.name) || col.primaryKey;
        const val = resultCache.read(resultId, col.id, row, true)?.value ?? null;
        if (isPk) pkParts.push(`${col.name} = ${val ?? "NULL"}`);
        return {
          columnId: col.id,
          columnName: col.name,
          isPrimaryKey: isPk,
          before: val,
          after: null,
          changed: true,
        };
      });

      diffs.push({
        id: `delete:${resultId}:${row}`,
        kind: "delete",
        resultId,
        rowIndex: row,
        primaryKeySummary: pkParts.length ? pkParts.join(", ") : `Row ${row + 1}`,
        tableName,
        cells,
        ref: { kind: "delete", resultId, row },
      });
    }
  }

  // 3. INSERTS
  const scopeId = tab.kind === "table" ? tab.id : (activeResult?.id ?? tab.id);
  const inserts = dirtyStore.insertsForTab(scopeId);
  inserts.forEach((draft, idx) => {
    const cells: CellDiff[] = columns.map((col) => {
      const isPk = pkNames.has(col.name) || col.primaryKey;
      const hasVal = col.id in draft.values;
      const val = hasVal ? draft.values[col.id] : null;
      return {
        columnId: col.id,
        columnName: col.name,
        isPrimaryKey: isPk,
        before: null,
        after: val,
        changed: hasVal,
      };
    });

    diffs.push({
      id: `insert:${scopeId}:${draft.id}`,
      kind: "insert",
      insertId: draft.id,
      primaryKeySummary: `New Row #${idx + 1}`,
      tableName,
      cells,
      ref: { kind: "insert", tabId: scopeId, insertId: draft.id },
    });
  });

  return diffs;
}

export function filterTableChanges(
  changes: TableChanges,
  selectedIds: Set<string>,
  diffRows: RowDiff[],
): TableChanges {
  const selectedRowMap = new Map(
    diffRows.filter((r) => selectedIds.has(r.id)).map((r) => [r.id, r]),
  );

  return {
    resultId: changes.resultId,
    updates: changes.updates
      .map((u) => ({
        resultId: u.resultId,
        changes: u.changes.filter((c) => selectedRowMap.has(`update:${u.resultId}:${c.row}`)),
      }))
      .filter((u) => u.changes.length > 0),
    deletes: changes.deletes
      .map((d) => ({
        resultId: d.resultId,
        rows: d.rows.filter((r) => selectedRowMap.has(`delete:${d.resultId}:${r}`)),
      }))
      .filter((d) => d.rows.length > 0),
    inserts: changes.inserts.filter((_, idx) => {
      const insertDiffs = diffRows.filter((r) => r.kind === "insert");
      const targetDiff = insertDiffs[idx];
      return targetDiff ? selectedIds.has(targetDiff.id) : true;
    }),
  };
}
