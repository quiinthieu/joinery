import { describe, expect, it } from "vitest";
import { DirtyStore, type DirtyCell, type DraftInsertRow } from "./dirty-store";

function calculateExpectedBytes(store: DirtyStore): number {
  const storeAny = store as unknown as {
    cells: Map<string, DirtyCell>;
    deletes: Map<string, Set<number>>;
    inserts: Map<string, DraftInsertRow[]>;
    size(cell: DirtyCell): number;
    insertRowSize(row: DraftInsertRow): number;
  };

  let expected = 0;
  for (const cell of storeAny.cells.values()) {
    expected += storeAny.size(cell);
  }
  for (const set of storeAny.deletes.values()) {
    expected += set.size * 64; // DELETE_ENTRY_BYTES
  }
  for (const list of storeAny.inserts.values()) {
    for (const row of list) {
      expected += storeAny.insertRowSize(row);
    }
  }
  return expected;
}

function getActualBytes(store: DirtyStore): number {
  return (store as unknown as { bytes: number }).bytes;
}

describe("DirtyStore Adversarial Stress Tests", () => {
  it("rapid sequential edits to the same cell preserve pristine original and revert cleans up completely", () => {
    const store = new DirtyStore();
    const resultId = "res_seq";
    const row = 0;
    const col = "amount";
    const pristineOriginal = "100.00";

    // Perform 25 rapid sequential updates on the exact same cell
    for (let i = 1; i <= 25; i++) {
      store.set({
        resultId,
        row,
        column: col,
        original: i === 1 ? pristineOriginal : `intermediate_${i - 1}`,
        value: `intermediate_${i}`,
      });
      // The original must NEVER drift to an intermediate value
      expect(store.get(resultId, row, col)?.original).toBe(pristineOriginal);
      expect(store.getOriginal(resultId, row, col)).toBe(pristineOriginal);
      expect(store.hasCell(resultId, row, col)).toBe(true);
      expect(getActualBytes(store)).toBe(calculateExpectedBytes(store));
    }

    // Now revert the cell
    store.revertCell(resultId, row, col);

    // Pristine state restored: cell is no longer dirty
    expect(store.hasCell(resultId, row, col)).toBe(false);
    expect(store.get(resultId, row, col)).toBeUndefined();
    expect(store.getOriginal(resultId, row, col)).toBeUndefined();
    expect(store.count).toBe(0);
    expect(getActualBytes(store)).toBe(0);
  });

  it("editing back to original value automatically clears cell and reclaims exact budget", () => {
    const store = new DirtyStore();
    store.set({
      resultId: "res1",
      row: 0,
      column: "status",
      original: "ACTIVE",
      value: "PENDING",
    });
    expect(store.count).toBe(1);
    expect(getActualBytes(store)).toBeGreaterThan(0);

    // Edit back to "ACTIVE"
    store.set({
      resultId: "res1",
      row: 0,
      column: "status",
      original: "PENDING",
      value: "ACTIVE",
    });
    expect(store.count).toBe(0);
    expect(store.hasCell("res1", 0, "status")).toBe(false);
    expect(getActualBytes(store)).toBe(0);
  });

  it("multi-cell batch reverts accurately reclaim memory budget without underflow or orphans", () => {
    const store = new DirtyStore();
    const totalCells = 200;
    const cellsToStaged: DirtyCell[] = [];

    for (let r = 0; r < 20; r++) {
      for (let c = 0; c < 10; c++) {
        cellsToStaged.push({
          resultId: "res_batch",
          row: r,
          column: `col_${c}`,
          original: `orig_${r}_${c}`,
          value: `edited_${r}_${c}`,
        });
      }
    }

    store.setBatch(cellsToStaged);
    expect(store.count).toBe(totalCells);
    expect(getActualBytes(store)).toBe(calculateExpectedBytes(store));

    // Revert in 4 chunks of 50 cells
    const revertTargets = cellsToStaged.map((c) => ({
      resultId: c.resultId,
      row: c.row,
      column: c.column,
    }));

    for (let chunk = 0; chunk < 4; chunk++) {
      const slice = revertTargets.slice(chunk * 50, (chunk + 1) * 50);
      store.revertBatch(slice);
      expect(store.count).toBe(totalCells - (chunk + 1) * 50);
      expect(getActualBytes(store)).toBe(calculateExpectedBytes(store));
      expect(getActualBytes(store)).toBeGreaterThanOrEqual(0);
    }

    // All cells reverted, budget must be exactly 0 with zero orphans
    expect(store.count).toBe(0);
    expect(getActualBytes(store)).toBe(0);
  });

  it("reverting non-existent or un-staged cells safely no-ops without corruption or notifications", () => {
    const store = new DirtyStore();
    let notifications = 0;
    store.subscribe(() => {
      notifications++;
    });

    // Stage one real cell
    store.set({
      resultId: "real_res",
      row: 5,
      column: "name",
      original: "Alice",
      value: "Alicia",
    });
    const bytesBefore = getActualBytes(store);
    expect(notifications).toBe(1);

    // Attempt reverting non-existent cell on real result
    store.revertCell("real_res", 5, "non_existent_col");
    expect(notifications).toBe(1);
    expect(getActualBytes(store)).toBe(bytesBefore);

    // Attempt reverting cell on non-existent result
    store.revertCell("ghost_res", 10, "col");
    expect(notifications).toBe(1);
    expect(getActualBytes(store)).toBe(bytesBefore);

    // Attempt batch revert with all ghost cells + duplicate targets
    store.revertBatch([
      { resultId: "ghost_res", row: 10, column: "col" },
      { resultId: "ghost_res", row: 10, column: "col" },
      { resultId: "real_res", row: 999, column: "name" },
      { resultId: "insert:nonexistent-uuid", row: 0, column: "col" },
    ]);
    expect(notifications).toBe(1);
    expect(getActualBytes(store)).toBe(bytesBefore);

    // Attempt revertRow on non-existent row
    store.revertRow("real_res", 999);
    store.revertRow("ghost_res", 0);
    store.revertRow("insert:ghost-uuid", 0);
    expect(notifications).toBe(1);
    expect(getActualBytes(store)).toBe(bytesBefore);

    // Verify original staged cell was completely untouched
    expect(store.hasCell("real_res", 5, "name")).toBe(true);
    expect(store.getOriginal("real_res", 5, "name")).toBe("Alice");
    expect(store.get("real_res", 5, "name")?.value).toBe("Alicia");
  });

  it("mixed edits and deletions on the same row: revertRow restores cleanly", () => {
    const store = new DirtyStore();
    const resultId = "res_mixed";

    // Row 10: has 3 cell edits
    store.set({ resultId, row: 10, column: "c1", original: "o1", value: "v1" });
    store.set({ resultId, row: 10, column: "c2", original: "o2", value: "v2" });
    store.set({ resultId, row: 10, column: "c3", original: "o3", value: "v3" });

    // Now delete row 10 (which drops the cell edits and marks row as deleted)
    store.setDeleted(resultId, [10], true);
    expect(store.isDeleted(resultId, 10)).toBe(true);
    expect(store.hasCell(resultId, 10, "c1")).toBe(false);
    expect(getActualBytes(store)).toBe(calculateExpectedBytes(store));

    // Reverting row 10 removes the deletion
    store.revertRow(resultId, 10);
    expect(store.isDeleted(resultId, 10)).toBe(false);
    expect(store.hasResult(resultId)).toBe(false);
    expect(store.count).toBe(0);
    expect(getActualBytes(store)).toBe(0);

    // Row 20: has 3 cell edits, NOT deleted, revertRow clears all 3
    store.set({ resultId, row: 20, column: "c1", original: "o1", value: "v1" });
    store.set({ resultId, row: 20, column: "c2", original: "o2", value: "v2" });
    store.set({ resultId, row: 20, column: "c3", original: "o3", value: "v3" });
    expect(store.count).toBe(3);

    store.revertRow(resultId, 20);
    expect(store.hasCell(resultId, 20, "c1")).toBe(false);
    expect(store.hasCell(resultId, 20, "c2")).toBe(false);
    expect(store.hasCell(resultId, 20, "c3")).toBe(false);
    expect(store.count).toBe(0);
    expect(getActualBytes(store)).toBe(0);
  });

  it("draft insert rows: partial revert via revertBatch and full revert via revertRow", () => {
    const store = new DirtyStore();
    const tabId = "tab_draft";
    const draft = store.addInsertRow(tabId);
    store.setInsertCell(tabId, draft.id, "name", "Product A");
    store.setInsertCell(tabId, draft.id, "price", "49.99");
    store.setInsertCell(tabId, draft.id, "category", "Electronics");

    expect(store.hasCell(`insert:${draft.id}`, 0, "name")).toBe(true);
    expect(store.hasCell(`insert:${draft.id}`, 0, "price")).toBe(true);
    expect(store.hasCell(`insert:${draft.id}`, 0, "category")).toBe(true);
    expect(store.getOriginal(`insert:${draft.id}`, 0, "name")).toBeUndefined();
    expect(getActualBytes(store)).toBe(calculateExpectedBytes(store));

    // Revert 1 column via revertBatch
    store.revertBatch([{ resultId: `insert:${draft.id}`, row: 0, column: "price" }]);
    expect(store.hasCell(`insert:${draft.id}`, 0, "price")).toBe(false);
    expect(store.hasCell(`insert:${draft.id}`, 0, "name")).toBe(true);
    expect(store.hasCell(`insert:${draft.id}`, 0, "category")).toBe(true);
    expect(getActualBytes(store)).toBe(calculateExpectedBytes(store));

    // Revert entire draft row via revertRow
    store.revertRow(`insert:${draft.id}`, 0);
    expect(store.insertsForTab(tabId).length).toBe(0);
    expect(store.count).toBe(0);
    expect(getActualBytes(store)).toBe(0);
  });

  it("invariant fuzz test: 1,000 random operations never desynchronize byte budget", () => {
    const store = new DirtyStore(50 * 1024 * 1024); // Large budget to avoid early overflow
    const resultIds = ["res_fuzz_1", "res_fuzz_2"];
    const tabId = "tab_fuzz";
    const cols = ["id", "title", "description", "score", "flag"];
    let draftIds: string[] = [];

    for (let step = 0; step < 1000; step++) {
      const op = Math.floor(Math.random() * 8);
      const resId = resultIds[Math.floor(Math.random() * resultIds.length)];
      const row = Math.floor(Math.random() * 10);
      const col = cols[Math.floor(Math.random() * cols.length)];

      switch (op) {
        case 0: {
          // Set cell
          if (!store.isDeleted(resId, row)) {
            store.set({
              resultId: resId,
              row,
              column: col,
              original: `orig_${row}_${col}`,
              value: Math.random() > 0.1 ? `val_${step}` : null,
            });
          }
          break;
        }
        case 1: {
          // Revert cell
          store.revertCell(resId, row, col);
          break;
        }
        case 2: {
          // Revert row
          store.revertRow(resId, row);
          break;
        }
        case 3: {
          // Toggle delete
          const isDel = store.isDeleted(resId, row);
          store.setDeleted(resId, [row], !isDel);
          break;
        }
        case 4: {
          // Add insert row
          if (draftIds.length < 20) {
            const draft = store.addInsertRow(tabId);
            draftIds.push(draft.id);
          }
          break;
        }
        case 5: {
          // Edit insert cell
          if (draftIds.length > 0) {
            const draftId = draftIds[Math.floor(Math.random() * draftIds.length)];
            store.setInsertCell(tabId, draftId, col, `insert_val_${step}`);
          }
          break;
        }
        case 6: {
          // Revert insert cell or row
          if (draftIds.length > 0) {
            const draftId = draftIds[Math.floor(Math.random() * draftIds.length)];
            if (Math.random() > 0.5) {
              store.revertBatch([{ resultId: `insert:${draftId}`, row: 0, column: col }]);
            } else {
              store.revertRow(`insert:${draftId}`, 0);
              draftIds = draftIds.filter((id) => id !== draftId);
            }
          }
          break;
        }
        case 7: {
          // Revert batch of random cells
          const batch = Array.from({ length: 5 }, () => ({
            resultId: resultIds[Math.floor(Math.random() * resultIds.length)],
            row: Math.floor(Math.random() * 10),
            column: cols[Math.floor(Math.random() * cols.length)],
          }));
          store.revertBatch(batch);
          break;
        }
      }

      // Assert invariant on EVERY single step
      const actual = getActualBytes(store);
      const expected = calculateExpectedBytes(store);
      expect(actual).toBe(expected);
      expect(actual).toBeGreaterThanOrEqual(0);
    }

    // Clean everything and verify 0
    store.clear();
    expect(getActualBytes(store)).toBe(0);
    expect(store.count).toBe(0);
  });
});
