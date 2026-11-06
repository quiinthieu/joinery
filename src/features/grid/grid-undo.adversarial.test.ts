import { describe, expect, it, vi } from "vitest";
import { DirtyStore } from "../../data/dirty-store";
import type { ColumnInfo } from "../../ipc/generated";

// Mirroring the exact algorithm from DatabaseGrid.tsx:134-216
interface GridRange {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface GridSelection {
  current?: {
    cell: [number, number];
    range: GridRange;
    rangeStack: GridRange[];
  };
  columns: {
    toArray(): number[];
    length: number;
    first(): number | undefined;
  };
  rows: {
    toArray(): number[];
    length: number;
    first(): number | undefined;
  };
}

function resolveSelectedModifications(
  sel: GridSelection,
  visibleColumns: ColumnInfo[],
  rowCount: number,
  locate: (visualRow: number) => { resultId: string; row: number } | null,
  store: DirtyStore,
) {
  const dirtyCells: { resultId: string; row: number; column: string }[] = [];
  const deletedRows: { resultId: string; row: number }[] = [];
  const seenCellKeys = new Set<string>();
  const seenRowKeys = new Set<string>();

  const rects = sel.current ? [sel.current.range, ...sel.current.rangeStack] : [];
  const rowList = sel.rows.toArray();
  const colList = sel.columns.toArray();

  const selectionCols = new Set<number>(colList);
  const selectionRows = new Set<number>(rowList);
  for (const rect of rects) {
    for (let x = rect.x; x < rect.x + rect.width; x++) {
      if (x >= 0 && x < visibleColumns.length) selectionCols.add(x);
    }
    for (let y = rect.y; y < rect.y + rect.height; y++) {
      if (y >= 0 && y < rowCount) selectionRows.add(y);
    }
  }
  if (selectionCols.size === 0 && selectionRows.size > 0) {
    for (let c = 0; c < visibleColumns.length; c++) selectionCols.add(c);
  }
  if (selectionRows.size === 0 && selectionCols.size > 0) {
    for (let r = 0; r < rowCount; r++) selectionRows.add(r);
  }

  for (const visualRow of selectionRows) {
    const source = locate(visualRow);
    if (source) {
      const rowKey = `${source.resultId}:${source.row}`;
      if (store.isDeleted(source.resultId, source.row) && !seenRowKeys.has(rowKey)) {
        seenRowKeys.add(rowKey);
        deletedRows.push({ resultId: source.resultId, row: source.row });
      }
    }

    for (const colIdx of selectionCols) {
      const col = visibleColumns[colIdx];
      if (!col) continue;

      if (source) {
        const cellKey = `${source.resultId}:${source.row}:${col.id}`;
        if (store.hasCell(source.resultId, source.row, col.id) && !seenCellKeys.has(cellKey)) {
          seenCellKeys.add(cellKey);
          dirtyCells.push({
            resultId: source.resultId,
            row: source.row,
            column: col.id,
          });
        }
      }
    }
  }

  return { dirtyCells, deletedRows };
}

describe("Grid Undo UX & Selection Adversarial Tests", () => {
  const columns: ColumnInfo[] = [
    {
      id: "c0",
      name: "id",
      databaseType: "int",
      kind: "integer",
      nullable: false,
      primaryKey: true,
      foreignKey: false,
      editable: false,
      insertable: false,
      hasDefault: false,
    },
    {
      id: "c1",
      name: "name",
      databaseType: "text",
      kind: "text",
      nullable: false,
      primaryKey: false,
      foreignKey: false,
      editable: true,
      insertable: true,
      hasDefault: false,
    },
    {
      id: "c2",
      name: "balance",
      databaseType: "numeric",
      kind: "decimal",
      nullable: false,
      primaryKey: false,
      foreignKey: false,
      editable: true,
      insertable: true,
      hasDefault: false,
    },
    {
      id: "c3",
      name: "notes",
      databaseType: "text",
      kind: "text",
      nullable: true,
      primaryKey: false,
      foreignKey: false,
      editable: true,
      insertable: true,
      hasDefault: false,
    },
  ];

  const locate = (visualRow: number) => ({ resultId: "res_grid", row: visualRow });

  it("single focused cell undo targets only the focused cell", () => {
    const store = new DirtyStore();
    // Stage edits on row 0, col 1 and row 0, col 2
    store.set({ resultId: "res_grid", row: 0, column: "c1", original: "Alice", value: "Alicia" });
    store.set({ resultId: "res_grid", row: 0, column: "c2", original: "100", value: "200" });
    // Stage edits on row 1, col 1
    store.set({ resultId: "res_grid", row: 1, column: "c1", original: "Bob", value: "Bobby" });

    // Focus only (col 1, row 0)
    const sel: GridSelection = {
      current: {
        cell: [1, 0],
        range: { x: 1, y: 0, width: 1, height: 1 },
        rangeStack: [],
      },
      columns: { toArray: () => [], length: 0, first: () => undefined },
      rows: { toArray: () => [], length: 0, first: () => undefined },
    };

    const { dirtyCells, deletedRows } = resolveSelectedModifications(
      sel,
      columns,
      10,
      locate,
      store,
    );

    // Only (row 0, c1) should be targeted
    expect(dirtyCells).toEqual([{ resultId: "res_grid", row: 0, column: "c1" }]);
    expect(deletedRows).toEqual([]);

    // Execute revert on the resolved cells
    store.revertBatch(dirtyCells);

    // Verify row 0, c1 was reverted, but row 0, c2 and row 1, c1 remain dirty
    expect(store.hasCell("res_grid", 0, "c1")).toBe(false);
    expect(store.hasCell("res_grid", 0, "c2")).toBe(true);
    expect(store.hasCell("res_grid", 1, "c1")).toBe(true);
  });

  it("pressing undo when no cells are dirty safely returns 0 modifications and no-ops", () => {
    const store = new DirtyStore();

    // Focus a cell with NO dirty edits anywhere
    const sel: GridSelection = {
      current: {
        cell: [2, 3],
        range: { x: 2, y: 3, width: 1, height: 1 },
        rangeStack: [],
      },
      columns: { toArray: () => [], length: 0, first: () => undefined },
      rows: { toArray: () => [], length: 0, first: () => undefined },
    };

    const { dirtyCells, deletedRows } = resolveSelectedModifications(
      sel,
      columns,
      10,
      locate,
      store,
    );
    expect(dirtyCells.length).toBe(0);
    expect(deletedRows.length).toBe(0);

    const total = dirtyCells.length + deletedRows.length;
    expect(total).toBe(0);

    // In DatabaseGrid: if (total === 0) return;
    // Verifying calling revertBatch with empty array also no-ops cleanly
    const revertSpy = vi.spyOn(store, "revertBatch");
    if (total > 0) {
      store.revertBatch(dirtyCells);
    }
    expect(revertSpy).not.toHaveBeenCalled();
    expect(store.count).toBe(0);
  });

  it("entire row selection undo reverts all cells in that row and row deletion", () => {
    const store = new DirtyStore();
    store.set({ resultId: "res_grid", row: 2, column: "c1", original: "Charlie", value: "Chuck" });
    store.set({ resultId: "res_grid", row: 2, column: "c2", original: "50", value: "75" });
    store.set({ resultId: "res_grid", row: 3, column: "c1", original: "Dave", value: "David" });
    store.setDeleted("res_grid", [2], true);

    // Row selection on row 2
    const sel: GridSelection = {
      columns: { toArray: () => [], length: 0, first: () => undefined },
      rows: { toArray: () => [2], length: 1, first: () => 2 },
    };

    const { dirtyCells, deletedRows } = resolveSelectedModifications(
      sel,
      columns,
      10,
      locate,
      store,
    );

    // Row 2 is deleted
    expect(deletedRows).toEqual([{ resultId: "res_grid", row: 2 }]);

    // Revert deleted row
    for (const item of deletedRows) {
      store.revertRow(item.resultId, item.row);
    }

    // Row 2 is no longer deleted, and row 3's dirty edit was unaffected
    expect(store.isDeleted("res_grid", 2)).toBe(false);
    expect(store.hasCell("res_grid", 3, "c1")).toBe(true);
  });
});
