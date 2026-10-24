export interface DirtyCell { resultId: string; row: number; column: string; original: string | null; value: string | null }

export class DirtyStore {
  private cells = new Map<string, DirtyCell>();
  private listeners = new Set<() => void>();
  private version = 0;
  private bytes = 0;
  constructor(private readonly budget = 8 * 1024 * 1024) {}
  private key(resultId: string, row: number, column: string) { return `${resultId}:${row}:${column}`; }
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  snapshot = () => this.version;
  private changed() { this.version++; this.listeners.forEach((listener) => listener()); }
  private size(cell: DirtyCell) { return 192 + 2 * ((cell.original?.length ?? 0) + (cell.value?.length ?? 0)); }
  get(resultId: string, row: number, column: string) { return this.cells.get(this.key(resultId, row, column)); }
  set(cell: DirtyCell) {
    const key = this.key(cell.resultId, cell.row, cell.column), previous = this.cells.get(key);
    const next = { ...cell, original: previous ? previous.original : cell.original };
    const before = previous ? this.size(previous) : 0;
    const after = next.original === next.value ? 0 : this.size(next);
    if (this.bytes - before + after > this.budget) throw new Error("Pending edits exceed 8 MiB. Apply or discard some edits first.");
    if (after) this.cells.set(key, next); else this.cells.delete(key);
    this.bytes += after - before; this.changed();
  }
  forResult(id: string) { return [...this.cells.values()].filter((cell) => cell.resultId === id); }
  hasResult(id: string) { for (const cell of this.cells.values()) if (cell.resultId === id) return true; return false; }
  clear(id?: string) {
    for (const [key, cell] of this.cells) if (!id || cell.resultId === id) { this.bytes -= this.size(cell); this.cells.delete(key); }
    this.changed();
  }
  get count() { return this.cells.size; }
}

export const dirtyStore = new DirtyStore();
