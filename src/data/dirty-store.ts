export interface DirtyCell {
  resultId: string;
  row: number;
  column: string;
  original: string | null;
  value: string | null;
}

export interface DraftInsertRow {
  id: string;
  values: Record<string, string | null>;
}

export interface DraftInsertCell {
  tabId: string;
  insertId: string;
  column: string;
  value: string | null | undefined;
}

const DELETE_ENTRY_BYTES = 64;
const MAX_STAGE_INSERTS = 500;
const EMPTY_INSERTS: readonly DraftInsertRow[] = Object.freeze([]);

export class DirtyStore {
  private cells = new Map<string, DirtyCell>();
  private deletes = new Map<string, Set<number>>();
  private inserts = new Map<string, DraftInsertRow[]>();
  private listeners = new Set<() => void>();
  private version = 0;
  private bytes = 0;
  constructor(private readonly budget = 8 * 1024 * 1024) {}
  private key(resultId: string, row: number, column: string) {
    return `${resultId}:${row}:${column}`;
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  snapshot = () => this.version;
  private changed() {
    this.version++;
    this.listeners.forEach((listener) => listener());
  }
  private size(cell: DirtyCell) {
    return 192 + 2 * ((cell.original?.length ?? 0) + (cell.value?.length ?? 0));
  }
  private insertRowSize(row: DraftInsertRow) {
    let total = 256;
    for (const [column, value] of Object.entries(row.values)) {
      total += 64 + 2 * (column.length + (value?.length ?? 0));
    }
    return total;
  }
  get(resultId: string, row: number, column: string) {
    return this.cells.get(this.key(resultId, row, column));
  }
  isDeleted(resultId: string, row: number) {
    return this.deletes.get(resultId)?.has(row) ?? false;
  }
  deletesForResult(resultId: string): number[] {
    const set = this.deletes.get(resultId);
    return set ? [...set].sort((a, b) => a - b) : [];
  }
  setDeleted(resultId: string, rows: Iterable<number>, deleted: boolean) {
    const unique = [...new Set(rows)].filter((row) => Number.isInteger(row) && row >= 0);
    if (!unique.length) return;
    const currentSet = this.deletes.get(resultId);
    const removedCellKeys: string[] = [];
    let delta = 0;

    if (deleted) {
      const targetRows = new Set(unique);
      for (const [key, cell] of this.cells) {
        if (cell.resultId === resultId && targetRows.has(cell.row)) {
          delta -= this.size(cell);
          removedCellKeys.push(key);
        }
      }
      for (const row of unique) {
        if (!currentSet?.has(row)) delta += DELETE_ENTRY_BYTES;
      }
    } else if (currentSet) {
      for (const row of unique) {
        if (currentSet.has(row)) delta -= DELETE_ENTRY_BYTES;
      }
    } else {
      return;
    }

    if (this.bytes + delta > this.budget)
      throw new Error("Pending edits exceed 8 MiB. Apply or discard some edits first.");

    let mutated = removedCellKeys.length > 0;
    for (const key of removedCellKeys) this.cells.delete(key);

    if (deleted) {
      const nextSet = currentSet ?? new Set<number>();
      for (const row of unique) {
        if (!nextSet.has(row)) {
          nextSet.add(row);
          mutated = true;
        }
      }
      if (nextSet.size) this.deletes.set(resultId, nextSet);
    } else if (currentSet) {
      for (const row of unique) {
        if (currentSet.delete(row)) mutated = true;
      }
      if (!currentSet.size) this.deletes.delete(resultId);
    }

    this.bytes += delta;
    if (mutated) this.changed();
  }
  insertsForTab(tabId: string): readonly DraftInsertRow[] {
    return this.inserts.get(tabId) ?? EMPTY_INSERTS;
  }
  addInsertRow(tabId: string): DraftInsertRow {
    const current = this.inserts.get(tabId) ?? [];
    if (current.length >= MAX_STAGE_INSERTS) {
      throw new Error(`Stage at most ${MAX_STAGE_INSERTS} inserted rows at a time.`);
    }
    const draft: DraftInsertRow = { id: crypto.randomUUID(), values: {} };
    const addedBytes = this.insertRowSize(draft);
    if (this.bytes + addedBytes > this.budget)
      throw new Error("Pending edits exceed 8 MiB. Apply or discard some edits first.");
    this.inserts.set(tabId, [...current, draft]);
    this.bytes += addedBytes;
    this.changed();
    return draft;
  }
  setInsertCell(tabId: string, insertId: string, column: string, value: string | null | undefined) {
    this.applyBatch([], [{ tabId, insertId, column, value }]);
  }
  removeInsertRows(tabId: string, insertIds: Iterable<string>) {
    const current = this.inserts.get(tabId);
    if (!current?.length) return;
    const targetIds = new Set(insertIds);
    if (!targetIds.size) return;
    let freed = 0;
    const remaining: DraftInsertRow[] = [];
    for (const row of current) {
      if (targetIds.has(row.id)) freed += this.insertRowSize(row);
      else remaining.push(row);
    }
    if (!freed) return;
    if (remaining.length) this.inserts.set(tabId, remaining);
    else this.inserts.delete(tabId);
    this.bytes -= freed;
    this.changed();
  }
  clearInserts(tabId?: string) {
    let freed = 0;
    let mutated = false;
    for (const [id, rows] of this.inserts) {
      if (!tabId || id === tabId) {
        for (const row of rows) freed += this.insertRowSize(row);
        this.inserts.delete(id);
        mutated = true;
      }
    }
    if (mutated) {
      this.bytes -= freed;
      this.changed();
    }
  }
  set(cell: DirtyCell) {
    if (this.isDeleted(cell.resultId, cell.row))
      throw new Error("Undo deletion on this row before editing its cells.");
    const key = this.key(cell.resultId, cell.row, cell.column),
      previous = this.cells.get(key);
    const next = { ...cell, original: previous ? previous.original : cell.original };
    const before = previous ? this.size(previous) : 0;
    const after = next.original === next.value ? 0 : this.size(next);
    if (this.bytes - before + after > this.budget)
      throw new Error("Pending edits exceed 8 MiB. Apply or discard some edits first.");
    if (after) this.cells.set(key, next);
    else this.cells.delete(key);
    this.bytes += after - before;
    this.changed();
  }
  setBatch(cells: DirtyCell[]) {
    this.applyBatch(cells, []);
  }
  applyBatch(cells: DirtyCell[], insertCells: DraftInsertCell[] = []) {
    if (!cells.length && !insertCells.length) return;
    for (const cell of cells) {
      if (this.isDeleted(cell.resultId, cell.row))
        throw new Error("Undo deletion on this row before editing its cells.");
    }
    const collapsed = new Map<string, DirtyCell>();
    const firstOriginals = new Map<string, string | null>();
    for (const cell of cells) {
      const key = this.key(cell.resultId, cell.row, cell.column);
      if (!firstOriginals.has(key)) firstOriginals.set(key, cell.original);
      collapsed.set(key, cell);
    }
    const staged = new Map<string, DirtyCell | null>();
    let delta = 0;
    for (const [key, cell] of collapsed) {
      const previous = this.cells.get(key);
      const original = previous ? previous.original : firstOriginals.get(key)!;
      const next: DirtyCell = { ...cell, original };
      const before = previous ? this.size(previous) : 0;
      const after = next.original === next.value ? 0 : this.size(next);
      delta += after - before;
      staged.set(key, after ? next : null);
    }

    const nextInsertsByTab = new Map<string, DraftInsertRow[]>();
    const insertMutatedTabs = new Set<string>();
    for (const edit of insertCells) {
      let tabRows = nextInsertsByTab.get(edit.tabId);
      if (!tabRows) {
        const current = this.inserts.get(edit.tabId);
        if (!current) throw new Error("Draft insert row no longer exists.");
        tabRows = current.map((row) => ({ id: row.id, values: { ...row.values } }));
        nextInsertsByTab.set(edit.tabId, tabRows);
      }
      const target = tabRows.find((row) => row.id === edit.insertId);
      if (!target) throw new Error("Draft insert row no longer exists.");
      const hadKey = edit.column in target.values;
      const prevVal = target.values[edit.column];
      if (edit.value === undefined) {
        if (hadKey) {
          delete target.values[edit.column];
          insertMutatedTabs.add(edit.tabId);
        }
      } else if (!hadKey || prevVal !== edit.value) {
        target.values[edit.column] = edit.value;
        insertMutatedTabs.add(edit.tabId);
      }
    }
    for (const tabId of insertMutatedTabs) {
      const beforeRows = this.inserts.get(tabId) ?? [];
      const afterRows = nextInsertsByTab.get(tabId) ?? [];
      for (const row of beforeRows) delta -= this.insertRowSize(row);
      for (const row of afterRows) delta += this.insertRowSize(row);
    }

    if (this.bytes + delta > this.budget)
      throw new Error("Pending edits exceed 8 MiB. Apply or discard some edits first.");

    let mutated = insertMutatedTabs.size > 0;
    for (const [key, next] of staged) {
      const previous = this.cells.get(key);
      if (next) {
        if (!previous || previous.value !== next.value) mutated = true;
        this.cells.set(key, next);
      } else if (previous) {
        mutated = true;
        this.cells.delete(key);
      }
    }
    for (const tabId of insertMutatedTabs) {
      this.inserts.set(tabId, nextInsertsByTab.get(tabId)!);
    }
    this.bytes += delta;
    if (mutated) this.changed();
  }
  forResult(id: string) {
    return [...this.cells.values()].filter((cell) => cell.resultId === id);
  }
  hasResult(id: string) {
    if ((this.deletes.get(id)?.size ?? 0) > 0) return true;
    for (const cell of this.cells.values()) if (cell.resultId === id) return true;
    return false;
  }
  pendingCountForTab(tabId: string, resultIds: Iterable<string>) {
    let total = this.inserts.get(tabId)?.length ?? 0;
    for (const id of resultIds) {
      total += this.deletes.get(id)?.size ?? 0;
      for (const cell of this.cells.values()) if (cell.resultId === id) total++;
    }
    return total;
  }
  clearTab(tabId: string, resultIds: Iterable<string>) {
    const targets = new Set(resultIds);
    let mutated = false;
    for (const [key, cell] of this.cells) {
      if (targets.has(cell.resultId)) {
        this.bytes -= this.size(cell);
        this.cells.delete(key);
        mutated = true;
      }
    }
    for (const id of targets) {
      const set = this.deletes.get(id);
      if (set) {
        this.bytes -= set.size * DELETE_ENTRY_BYTES;
        this.deletes.delete(id);
        mutated = true;
      }
    }
    const tabInserts = this.inserts.get(tabId);
    if (tabInserts) {
      for (const row of tabInserts) this.bytes -= this.insertRowSize(row);
      this.inserts.delete(tabId);
      mutated = true;
    }
    if (mutated) this.changed();
  }
  clear(id?: string) {
    if (!id) {
      this.cells.clear();
      this.deletes.clear();
      this.inserts.clear();
      this.bytes = 0;
      this.changed();
      return;
    }
    for (const [key, cell] of this.cells)
      if (cell.resultId === id) {
        this.bytes -= this.size(cell);
        this.cells.delete(key);
      }
    const deleted = this.deletes.get(id);
    if (deleted) {
      this.bytes -= deleted.size * DELETE_ENTRY_BYTES;
      this.deletes.delete(id);
    }
    this.changed();
  }
  get count() {
    let deletedCount = 0;
    for (const set of this.deletes.values()) deletedCount += set.size;
    let insertCount = 0;
    for (const list of this.inserts.values()) insertCount += list.length;
    return this.cells.size + deletedCount + insertCount;
  }
}

export const dirtyStore = new DirtyStore();
