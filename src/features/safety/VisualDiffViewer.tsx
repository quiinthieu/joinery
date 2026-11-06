import { useEffect, useRef, type ReactNode } from "react";
import { Columns, FileText, RotateCcw } from "lucide-react";
import type { CellDiff, RowDiff } from "./mutation-diff";

export interface VisualDiffViewerProps {
  diffRows: RowDiff[];
  selectedIds: Set<string>;
  onToggle(id: string): void;
  onToggleAll(select: boolean): void;
  layout: "unified" | "split";
  onLayoutChange(layout: "unified" | "split"): void;
  changedColsOnly: boolean;
  onChangedColsOnlyChange(changedOnly: boolean): void;
  onDiscardRow?(row: RowDiff): void;
}

export function VisualDiffViewer({
  diffRows,
  selectedIds,
  onToggle,
  onToggleAll,
  layout,
  onLayoutChange,
  changedColsOnly,
  onChangedColsOnlyChange,
  onDiscardRow,
}: VisualDiffViewerProps) {
  const masterCheckRef = useRef<HTMLInputElement>(null);
  const totalCount = diffRows.length;
  const selectedCount = selectedIds.size;
  const isAllSelected = totalCount > 0 && selectedCount === totalCount;
  const isIndeterminate = selectedCount > 0 && selectedCount < totalCount;

  useEffect(() => {
    if (masterCheckRef.current) {
      masterCheckRef.current.indeterminate = isIndeterminate;
    }
  }, [isIndeterminate]);

  return (
    <div className="visual-diff-viewer flex flex-col gap-3">
      {/* Control Strip */}
      <div className="flex items-center justify-between gap-3 px-3 py-2 bg-surface border border-border rounded-md text-xs">
        <label className="flex items-center gap-2 cursor-pointer select-none font-medium">
          <input
            ref={masterCheckRef}
            type="checkbox"
            checked={isAllSelected}
            onChange={(e) => onToggleAll(e.target.checked)}
            className="rounded border-border accent-accent"
          />
          <span>
            {selectedCount} of {totalCount} change{totalCount === 1 ? "" : "s"} selected
          </span>
        </label>

        <div className="flex items-center gap-3">
          <label className="flex items-center gap-1.5 cursor-pointer select-none text-muted hover:text-text">
            <input
              type="checkbox"
              checked={changedColsOnly}
              onChange={(e) => onChangedColsOnlyChange(e.target.checked)}
              className="rounded border-border accent-accent"
            />
            <span>Modified columns only</span>
          </label>

          <div className="flex items-center rounded border border-border overflow-hidden bg-bg">
            <button
              type="button"
              className={`flex items-center gap-1 px-2.5 py-1 text-xs transition-colors ${
                layout === "unified"
                  ? "bg-accent text-white font-medium"
                  : "text-muted hover:text-text"
              }`}
              onClick={() => onLayoutChange("unified")}
            >
              <FileText size={12} />
              Unified
            </button>
            <button
              type="button"
              className={`flex items-center gap-1 px-2.5 py-1 text-xs transition-colors ${
                layout === "split"
                  ? "bg-accent text-white font-medium"
                  : "text-muted hover:text-text"
              }`}
              onClick={() => onLayoutChange("split")}
            >
              <Columns size={12} />
              Split
            </button>
          </div>
        </div>
      </div>

      {/* Diff List Container */}
      <div className="diff-list max-h-[380px] overflow-y-auto flex flex-col gap-2.5 pr-1">
        {diffRows.map((row) => {
          const isSelected = selectedIds.has(row.id);
          const visibleCells = changedColsOnly
            ? row.cells.filter((c) =>
                row.kind === "update"
                  ? c.changed || c.isPrimaryKey
                  : row.kind === "delete"
                    ? c.before !== null || c.isPrimaryKey
                    : c.after !== null || c.isPrimaryKey,
              )
            : row.cells;

          return (
            <div
              key={row.id}
              className={`diff-row-card border rounded-md p-3 transition-opacity ${
                isSelected ? "border-border bg-bg-elevated" : "border-border/60 bg-bg/40 opacity-60"
              }`}
            >
              {/* Row Header */}
              <div className="flex items-center justify-between gap-2 pb-2 mb-2 border-b border-border/50 text-xs">
                <div className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={isSelected}
                    onChange={() => onToggle(row.id)}
                    className="rounded border-border accent-accent cursor-pointer"
                    aria-label={`Include change for ${row.primaryKeySummary}`}
                  />
                  <MutationBadge kind={row.kind} />
                  <span className="font-semibold text-text">
                    {row.tableName ? `${row.tableName} · ` : ""}
                    {row.primaryKeySummary}
                  </span>
                  {!isSelected && (
                    <span className="text-[11px] px-1.5 py-0.5 rounded bg-surface text-muted border border-border">
                      Unstaged (excluded)
                    </span>
                  )}
                </div>

                {onDiscardRow && (
                  <button
                    type="button"
                    className="flex items-center gap-1 text-[11px] text-muted hover:text-danger px-1.5 py-0.5 rounded hover:bg-surface transition-colors"
                    onClick={() => onDiscardRow(row)}
                    title="Discard pending edits on this row"
                  >
                    <RotateCcw size={11} />
                    Discard
                  </button>
                )}
              </div>

              {/* Diff Content */}
              {layout === "unified" ? (
                <UnifiedRowContent cells={visibleCells} kind={row.kind} />
              ) : (
                <SplitRowContent cells={visibleCells} kind={row.kind} />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function MutationBadge({ kind }: { kind: "update" | "insert" | "delete" }) {
  if (kind === "update") {
    return (
      <span className="text-[10px] uppercase font-bold tracking-wider px-1.5 py-0.5 rounded bg-amber-950/40 text-amber-300 border border-amber-500/30">
        MODIFIED
      </span>
    );
  }
  if (kind === "insert") {
    return (
      <span className="text-[10px] uppercase font-bold tracking-wider px-1.5 py-0.5 rounded bg-emerald-950/40 text-emerald-400 border border-emerald-500/30">
        INSERTED
      </span>
    );
  }
  return (
    <span className="text-[10px] uppercase font-bold tracking-wider px-1.5 py-0.5 rounded bg-red-950/40 text-red-400 border border-red-500/30">
      DELETED
    </span>
  );
}

function formatValue(val: string | null): ReactNode {
  if (val === null) return <span className="italic text-muted/80">NULL</span>;
  if (val === "") return <span className="italic text-muted/80">""</span>;
  return val;
}

function UnifiedRowContent({
  cells,
  kind,
}: {
  cells: CellDiff[];
  kind: "update" | "insert" | "delete";
}) {
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-x-4 gap-y-1.5 font-mono text-xs">
      {cells.map((cell) => {
        return (
          <div
            key={cell.columnId}
            className={`flex items-center gap-2 py-0.5 px-1 rounded ${
              cell.changed ? "bg-surface/50" : ""
            }`}
          >
            <span
              className={`w-32 truncate shrink-0 ${
                cell.isPrimaryKey ? "font-semibold text-text" : "text-muted"
              }`}
              title={cell.columnName}
            >
              {cell.columnName}:
            </span>

            {kind === "update" ? (
              <div className="flex items-center gap-1.5 overflow-hidden truncate">
                {cell.changed ? (
                  <>
                    <span className="line-through text-red-400 bg-red-950/30 px-1 rounded truncate max-w-[140px]">
                      {formatValue(cell.before)}
                    </span>
                    <span className="text-muted shrink-0">→</span>
                    <span className="text-emerald-400 bg-emerald-950/30 font-medium px-1 rounded truncate max-w-[140px]">
                      {formatValue(cell.after)}
                    </span>
                  </>
                ) : (
                  <span className="text-muted truncate max-w-[200px]">
                    {formatValue(cell.after)}
                  </span>
                )}
              </div>
            ) : kind === "delete" ? (
              <span className="line-through text-red-400 bg-red-950/30 px-1 rounded truncate max-w-[240px]">
                {formatValue(cell.before)}
              </span>
            ) : (
              <span className="text-emerald-400 bg-emerald-950/30 font-medium px-1 rounded truncate max-w-[240px]">
                {formatValue(cell.after)}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}

function SplitRowContent({
  cells,
  kind,
}: {
  cells: CellDiff[];
  kind: "update" | "insert" | "delete";
}) {
  return (
    <div className="grid grid-cols-2 gap-4 font-mono text-xs">
      {/* Left: Original (Before) */}
      <div className="flex flex-col gap-1 border-r border-border/40 pr-3">
        <div className="text-[10px] uppercase font-bold text-muted pb-1 border-b border-border/30">
          Original (Before)
        </div>
        {cells.map((cell) => (
          <div key={cell.columnId} className="flex items-center gap-2 py-0.5">
            <span className="w-28 truncate shrink-0 text-muted" title={cell.columnName}>
              {cell.columnName}:
            </span>
            <span
              className={`truncate max-w-[160px] ${
                kind === "delete" || (kind === "update" && cell.changed)
                  ? "line-through text-red-400 bg-red-950/30 px-1 rounded"
                  : "text-muted"
              }`}
            >
              {kind === "insert" ? (
                <span className="italic text-muted/40">—</span>
              ) : (
                formatValue(cell.before)
              )}
            </span>
          </div>
        ))}
      </div>

      {/* Right: Staged (After) */}
      <div className="flex flex-col gap-1 pl-1">
        <div className="text-[10px] uppercase font-bold text-muted pb-1 border-b border-border/30">
          Staged (After)
        </div>
        {cells.map((cell) => (
          <div key={cell.columnId} className="flex items-center gap-2 py-0.5">
            <span className="w-28 truncate shrink-0 text-muted" title={cell.columnName}>
              {cell.columnName}:
            </span>
            <span
              className={`truncate max-w-[160px] ${
                kind === "insert" || (kind === "update" && cell.changed)
                  ? "text-emerald-400 bg-emerald-950/30 font-medium px-1 rounded"
                  : "text-muted"
              }`}
            >
              {kind === "delete" ? (
                <span className="italic text-muted/40">—</span>
              ) : (
                formatValue(cell.after)
              )}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
