import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import {
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  RotateCcw,
  Search,
} from "lucide-react";
import type { ColumnInfo, ResultInfo } from "../../ipc/generated";
import { resultCache } from "../../data/result-cache";
import { dirtyStore } from "../../data/dirty-store";
import {
  isSnowflakeConsoleSession,
  querySessionForTab,
  useStudio,
  type StudioTab,
} from "../../state/studio";
import type { CellTarget } from "./DatabaseGrid";

export type GridDisplayMode = "grid" | "transpose" | "record";

const TRANSPOSE_ROW_PAGE = 25;

interface AltViewProps {
  tab: StudioTab;
  result?: ResultInfo;
  visibleColumns: ColumnInfo[];
  focusRowRequest?: { seq: number; row: number } | null;
  onSelect(target: CellTarget | null): void;
  onOpenValue(target: CellTarget): void;
}

function benchmarkCellText(col: number, row: number): string {
  const statuses = ["completed", "processing", "pending", "completed", "refunded"];
  switch (col) {
    case 0:
      return String(100000 + row);
    case 1:
      return `Workspace ${String(row + 1).padStart(5, "0")}`;
    case 2:
      return statuses[row % 5];
    case 3:
      return ["Alex Morgan", "Jamie Chen", "Sam Rivera", "Taylor Kim"][row % 4];
    case 4:
      return `team${row + 1}@example.test`;
    case 5:
      return `${((row * 137) % 90000) + 100}.00`;
    case 6:
      return ["us-east-1", "eu-west-1", "ap-southeast-1"][row % 3];
    default:
      return `2026-10-${String(1 + (row % 24)).padStart(2, "0")} 09:42:18+00`;
  }
}

function resolveCellTarget(
  tab: StudioTab,
  result: ResultInfo | undefined,
  column: ColumnInfo,
  visualColIndex: number,
  visualRow: number,
  isGridEditable: boolean,
): { target: CellTarget | null; dirty: boolean; loading: boolean } {
  const loaded = tab.kind === "table" ? tab.loadedRows : (result?.retainedRows ?? 0);
  const scopeId = tab.kind === "table" ? tab.id : (result?.id ?? tab.id);
  if (tab.kind === "benchmark") {
    const val = benchmarkCellText(visualColIndex, visualRow);
    return {
      target: {
        resultId: "benchmark",
        row: visualRow,
        column,
        value: val,
        large: false,
      },
      dirty: false,
      loading: false,
    };
  }
  if (isGridEditable && visualRow >= loaded) {
    const inserts = dirtyStore.insertsForTab(scopeId);
    const draft = inserts[visualRow - loaded];
    if (!draft) return { target: null, dirty: false, loading: false };
    const hasKey = column.id in draft.values;
    const raw = draft.values[column.id];
    const usingDefault = !hasKey && (column.hasDefault || !column.insertable);
    const explicitNull = hasKey && raw === null;
    const unfilledRequired = !hasKey && !usingDefault && !column.nullable;
    return {
      target: {
        resultId: scopeId,
        row: visualRow,
        column: { ...column, editable: column.insertable },
        value: hasKey ? (raw ?? null) : null,
        large: Boolean(raw && (raw.length > 160 || raw.includes("\n"))),
        insert: {
          tabId: scopeId,
          insertId: draft.id,
          usingDefault,
          explicitNull,
          unfilledRequired,
        },
      },
      dirty: true,
      loading: false,
    };
  }
  const source =
    tab.kind === "table"
      ? (() => {
          const page = tab.pages.find(
            (p) => visualRow >= p.start && visualRow < p.start + p.result.retainedRows,
          );
          return page ? { result: page.result, row: visualRow - page.start } : null;
        })()
      : result
        ? { result, row: visualRow }
        : null;
  if (!source) return { target: null, dirty: false, loading: true };
  const cached = resultCache.read(source.result.id, column.id, source.row, true);
  if (!cached) return { target: null, dirty: false, loading: true };
  const deleted = dirtyStore.isDeleted(source.result.id, source.row);
  const dirtyCell = deleted ? undefined : dirtyStore.get(source.result.id, source.row, column.id);
  const value = dirtyCell ? dirtyCell.value : cached.value;
  return {
    target: {
      resultId: source.result.id,
      row: source.row,
      column: { ...column, editable: isGridEditable && column.editable && !deleted },
      value,
      large: cached.large || Boolean(value && (value.length > 160 || value.includes("\n"))),
      deleted,
    },
    dirty: Boolean(dirtyCell) || deleted,
    loading: false,
  };
}

export function TransposeGridView({
  tab,
  result,
  visibleColumns,
  focusRowRequest,
  onSelect,
  onOpenValue,
}: AltViewProps) {
  useSyncExternalStore(dirtyStore.subscribe, dirtyStore.snapshot);
  const [, setCacheTick] = useState(0);
  useEffect(() => resultCache.subscribe(() => setCacheTick((n) => n + 1)), []);

  const connected = useStudio((state) => {
    if (!tab.connectionId) return false;
    const activeSession =
      tab.kind === "query"
        ? querySessionForTab(tab, state.sessions)
        : state.sessions[tab.connectionId];
    return Boolean(activeSession && activeSession.id === tab.dataSessionId);
  });
  const readOnlyProfile = useStudio((state) => {
    const profile = state.bootstrap?.profiles.find((p) => p.id === tab.connectionId);
    return profile?.safeMode === "readOnly";
  });
  const consoleMode = useStudio((state) => {
    if (!tab.connectionId) return false;
    const activeSession =
      tab.kind === "query"
        ? querySessionForTab(tab, state.sessions)
        : state.sessions[tab.connectionId];
    return (
      isSnowflakeConsoleSession(state, activeSession?.id) ||
      isSnowflakeConsoleSession(state, tab.dataSessionId)
    );
  });
  const isGridEditable =
    connected &&
    !["running", "queued", "outcomeUnknown"].includes(tab.status) &&
    !readOnlyProfile &&
    !consoleMode &&
    (tab.kind === "table"
      ? Boolean(tab.schema?.editable)
      : tab.kind === "query" && result?.editability?.status === "editable");

  const scopeId = tab.kind === "table" ? tab.id : (result?.id ?? tab.id);
  const draftInserts = isGridEditable ? dirtyStore.insertsForTab(scopeId) : [];
  const totalRows =
    tab.kind === "benchmark"
      ? 1_000_000
      : tab.kind === "table"
        ? tab.loadedRows + draftInserts.length
        : (result?.retainedRows ?? 0) + draftInserts.length;

  const [rowPage, setRowPage] = useState(0);
  const [columnFilter, setColumnFilter] = useState("");
  const [activeCoord, setActiveCoord] = useState<{ colId: string; row: number } | null>(null);

  const maxPage = Math.max(0, Math.ceil(totalRows / TRANSPOSE_ROW_PAGE) - 1);
  const clampedPage = Math.min(rowPage, maxPage);
  const startRow = clampedPage * TRANSPOSE_ROW_PAGE;
  const endRow = Math.min(totalRows, startRow + TRANSPOSE_ROW_PAGE);
  const rowIndices = useMemo(
    () => Array.from({ length: Math.max(0, endRow - startRow) }, (_, i) => startRow + i),
    [endRow, startRow],
  );

  useEffect(() => {
    setRowPage(0);
  }, [tab.id, tab.revision, result?.id]);

  useEffect(() => {
    if (!focusRowRequest) return;
    setRowPage(Math.max(0, Math.floor(focusRowRequest.row / TRANSPOSE_ROW_PAGE)));
  }, [focusRowRequest]);

  useEffect(() => {
    if (tab.kind === "benchmark") return;
    const loaded = tab.kind === "table" ? tab.loadedRows : (result?.retainedRows ?? 0);
    for (const r of rowIndices) {
      if (r >= loaded) continue;
      if (tab.kind === "table") {
        const page = tab.pages.find((p) => r >= p.start && r < p.start + p.result.retainedRows);
        if (!page) {
          void useStudio.getState().ensureTablePage(tab.id, r);
        } else {
          void resultCache.ensure(page.result.id, r - page.start, page.result.retainedRows);
        }
      } else if (result) {
        void resultCache.ensure(result.id, r, result.retainedRows);
      }
    }
  }, [result, rowIndices, tab.id, tab.kind, tab.loadedRows, tab.pages]);

  const normalizedFilter = columnFilter.trim().toLowerCase();
  const filteredColumns = useMemo(
    () =>
      visibleColumns
        .map((col, idx) => ({ col, idx }))
        .filter(
          ({ col }) =>
            !normalizedFilter ||
            col.name.toLowerCase().includes(normalizedFilter) ||
            col.databaseType.toLowerCase().includes(normalizedFilter),
        ),
    [normalizedFilter, visibleColumns],
  );

  if (!visibleColumns.length || totalRows === 0) {
    return (
      <div className="grid-empty">
        {tab.loadingPage || tab.status === "running"
          ? "Waiting for results…"
          : "No rows to display"}
      </div>
    );
  }

  return (
    <div className="alt-grid-view" data-testid="transpose-grid-view">
      <div className="alt-grid-toolbar">
        <label className="diagram-filter alt-grid-filter">
          <Search size={13} />
          <input
            aria-label="Filter transposed columns"
            placeholder="Filter columns or types…"
            value={columnFilter}
            onChange={(event) => setColumnFilter(event.target.value)}
          />
        </label>
        <div className="alt-grid-pager">
          <span className="muted">
            Rows {(startRow + 1).toLocaleString()}–{endRow.toLocaleString()} of{" "}
            {totalRows.toLocaleString()}
          </span>
          <button
            type="button"
            className="icon-button"
            aria-label="Previous transposed rows"
            disabled={clampedPage === 0}
            onClick={() => setRowPage((p) => Math.max(0, p - 1))}
          >
            <ChevronLeft size={14} />
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label="Next transposed rows"
            disabled={clampedPage >= maxPage}
            onClick={() => setRowPage((p) => Math.min(maxPage, p + 1))}
          >
            <ChevronRight size={14} />
          </button>
          {tab.kind === "table" && tab.hasMore && clampedPage >= maxPage && (
            <button
              type="button"
              className="button ghost small"
              disabled={tab.loadingPage}
              onClick={() => void useStudio.getState().loadNextPage(tab.id)}
            >
              Load more rows
            </button>
          )}
        </div>
      </div>
      <div className="alt-grid-scroll">
        <table className="alt-grid-table" aria-label="Transposed data grid">
          <thead>
            <tr>
              <th scope="col" className="sticky-col">
                Column
              </th>
              <th scope="col" className="sticky-type-col">
                Type
              </th>
              {rowIndices.map((r) => {
                const loaded = tab.kind === "table" ? tab.loadedRows : (result?.retainedRows ?? 0);
                const isInsert = tab.kind !== "benchmark" && r >= loaded;
                return (
                  <th scope="col" key={r}>
                    {isInsert ? `Insert +${r - loaded + 1}` : `Row ${r + 1}`}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {filteredColumns.map(({ col, idx }) => (
              <tr key={col.id}>
                <th scope="row" className="sticky-col mono">
                  {col.primaryKey ? "⌑ " : col.foreignKey ? "↗ " : ""}
                  {col.name}
                </th>
                <td className="sticky-type-col muted mono">{col.databaseType}</td>
                {rowIndices.map((r) => {
                  const resolved = resolveCellTarget(tab, result, col, idx, r, isGridEditable);
                  const isActive = activeCoord?.colId === col.id && activeCoord.row === r;
                  const cellTarget = resolved.target;
                  return (
                    <td
                      key={r}
                      tabIndex={0}
                      className={`mono alt-grid-cell ${resolved.dirty ? "dirty" : ""} ${
                        cellTarget?.deleted ? "deleted" : ""
                      } ${isActive ? "selected" : ""}`.trim()}
                      onClick={() => {
                        setActiveCoord({ colId: col.id, row: r });
                        onSelect(cellTarget);
                      }}
                      onDoubleClick={() => {
                        if (cellTarget) onOpenValue(cellTarget);
                      }}
                      onKeyDown={(event) => {
                        if ((event.key === "Enter" || event.key === " ") && cellTarget) {
                          event.preventDefault();
                          onSelect(cellTarget);
                          if (event.key === "Enter") onOpenValue(cellTarget);
                        }
                      }}
                    >
                      {resolved.loading ? (
                        <span className="cell-token">…</span>
                      ) : cellTarget?.insert?.usingDefault ? (
                        <span className="cell-token">&lt;default&gt;</span>
                      ) : cellTarget?.insert?.unfilledRequired ? (
                        <span className="cell-token">&lt;required&gt;</span>
                      ) : cellTarget?.value === null || cellTarget?.value === undefined ? (
                        <span className="cell-token">NULL</span>
                      ) : cellTarget.value === "" ? (
                        <span className="cell-token">empty</span>
                      ) : cellTarget.value.length > 96 ? (
                        `${cellTarget.value.slice(0, 96)}…`
                      ) : (
                        cellTarget.value
                      )}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export function RecordCardView({
  tab,
  result,
  visibleColumns,
  focusRowRequest,
  onSelect,
  onOpenValue,
}: AltViewProps) {
  useSyncExternalStore(dirtyStore.subscribe, dirtyStore.snapshot);
  const [, setCacheTick] = useState(0);
  useEffect(() => resultCache.subscribe(() => setCacheTick((n) => n + 1)), []);

  const connected = useStudio((state) => {
    if (!tab.connectionId) return false;
    const activeSession =
      tab.kind === "query"
        ? querySessionForTab(tab, state.sessions)
        : state.sessions[tab.connectionId];
    return Boolean(activeSession && activeSession.id === tab.dataSessionId);
  });
  const readOnlyProfile = useStudio((state) => {
    const profile = state.bootstrap?.profiles.find((p) => p.id === tab.connectionId);
    return profile?.safeMode === "readOnly";
  });
  const consoleMode = useStudio((state) => {
    if (!tab.connectionId) return false;
    const activeSession =
      tab.kind === "query"
        ? querySessionForTab(tab, state.sessions)
        : state.sessions[tab.connectionId];
    return (
      isSnowflakeConsoleSession(state, activeSession?.id) ||
      isSnowflakeConsoleSession(state, tab.dataSessionId)
    );
  });
  const isGridEditable =
    connected &&
    !["running", "queued", "outcomeUnknown"].includes(tab.status) &&
    !readOnlyProfile &&
    !consoleMode &&
    (tab.kind === "table"
      ? Boolean(tab.schema?.editable)
      : tab.kind === "query" && result?.editability?.status === "editable");

  const scopeId = tab.kind === "table" ? tab.id : (result?.id ?? tab.id);
  const draftInserts = isGridEditable ? dirtyStore.insertsForTab(scopeId) : [];
  const totalRows =
    tab.kind === "benchmark"
      ? 1_000_000
      : tab.kind === "table"
        ? tab.loadedRows + draftInserts.length
        : (result?.retainedRows ?? 0) + draftInserts.length;

  const [rowIndex, setRowIndex] = useState(0);
  const [fieldFilter, setFieldFilter] = useState("");
  const clampedRow = totalRows > 0 ? Math.min(rowIndex, totalRows - 1) : 0;

  useEffect(() => {
    setRowIndex(0);
  }, [tab.id, tab.revision, result?.id]);

  useEffect(() => {
    if (!focusRowRequest) return;
    setRowIndex(Math.max(0, focusRowRequest.row));
  }, [focusRowRequest]);

  useEffect(() => {
    if (tab.kind === "benchmark" || totalRows === 0) return;
    const loaded = tab.kind === "table" ? tab.loadedRows : (result?.retainedRows ?? 0);
    if (clampedRow >= loaded) return;
    if (tab.kind === "table") {
      const page = tab.pages.find(
        (p) => clampedRow >= p.start && clampedRow < p.start + p.result.retainedRows,
      );
      if (!page) {
        void useStudio.getState().ensureTablePage(tab.id, clampedRow);
      } else {
        void resultCache.ensure(page.result.id, clampedRow - page.start, page.result.retainedRows);
      }
    } else if (result) {
      void resultCache.ensure(result.id, clampedRow, result.retainedRows);
    }
  }, [clampedRow, result, tab.id, tab.kind, tab.loadedRows, tab.pages, totalRows]);

  const normalizedFilter = fieldFilter.trim().toLowerCase();
  const filteredFields = useMemo(
    () =>
      visibleColumns
        .map((col, idx) => ({ col, idx }))
        .filter(
          ({ col }) =>
            !normalizedFilter ||
            col.name.toLowerCase().includes(normalizedFilter) ||
            col.databaseType.toLowerCase().includes(normalizedFilter),
        ),
    [normalizedFilter, visibleColumns],
  );

  if (!visibleColumns.length || totalRows === 0) {
    return (
      <div className="grid-empty">
        {tab.loadingPage || tab.status === "running"
          ? "Waiting for results…"
          : "No rows to display"}
      </div>
    );
  }

  const loaded = tab.kind === "table" ? tab.loadedRows : (result?.retainedRows ?? 0);
  const isInsertRow = tab.kind !== "benchmark" && clampedRow >= loaded;

  return (
    <div className="alt-grid-view" data-testid="record-card-view">
      <div className="alt-grid-toolbar">
        <label className="diagram-filter alt-grid-filter">
          <Search size={13} />
          <input
            aria-label="Filter record fields"
            placeholder="Filter fields…"
            value={fieldFilter}
            onChange={(event) => setFieldFilter(event.target.value)}
          />
        </label>
        <div className="alt-grid-pager">
          <button
            type="button"
            className="icon-button"
            aria-label="First record"
            disabled={clampedRow === 0}
            onClick={() => setRowIndex(0)}
          >
            <ChevronsLeft size={14} />
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label="Previous record"
            disabled={clampedRow === 0}
            onClick={() => setRowIndex((r) => Math.max(0, r - 1))}
          >
            <ChevronLeft size={14} />
          </button>
          <span className="muted">
            {isInsertRow
              ? `Draft insert +${clampedRow - loaded + 1}`
              : `Record ${(clampedRow + 1).toLocaleString()} of ${totalRows.toLocaleString()}`}
          </span>
          <button
            type="button"
            className="icon-button"
            aria-label="Next record"
            disabled={clampedRow >= totalRows - 1}
            onClick={() => setRowIndex((r) => Math.min(totalRows - 1, r + 1))}
          >
            <ChevronRight size={14} />
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label="Last record"
            disabled={clampedRow >= totalRows - 1}
            onClick={() => setRowIndex(totalRows - 1)}
          >
            <ChevronsRight size={14} />
          </button>
          {tab.kind === "table" && tab.hasMore && clampedRow >= totalRows - 1 && (
            <button
              type="button"
              className="button ghost small"
              disabled={tab.loadingPage}
              onClick={() => void useStudio.getState().loadNextPage(tab.id)}
            >
              Load more rows
            </button>
          )}
        </div>
      </div>
      <div className="alt-grid-scroll record-fields-list">
        {filteredFields.map(({ col, idx }) => {
          const resolved = resolveCellTarget(tab, result, col, idx, clampedRow, isGridEditable);
          const cellTarget = resolved.target;
          const canRevertCell =
            Boolean(cellTarget) &&
            !cellTarget!.insert &&
            !cellTarget!.deleted &&
            Boolean(dirtyStore.get(cellTarget!.resultId, cellTarget!.row, col.id));
          return (
            <div
              key={col.id}
              className={`record-field-row ${resolved.dirty ? "dirty" : ""}`.trim()}
              onClick={() => onSelect(cellTarget)}
            >
              <div className="record-field-label">
                <strong className="mono">
                  {col.primaryKey ? "⌑ " : col.foreignKey ? "↗ " : ""}
                  {col.name}
                </strong>
                <span className="muted mono">{col.databaseType}</span>
              </div>
              <div className="record-field-value mono">
                {resolved.loading ? (
                  <span className="cell-token">Loading…</span>
                ) : cellTarget?.insert?.usingDefault ? (
                  <span className="cell-token">&lt;default&gt;</span>
                ) : cellTarget?.insert?.unfilledRequired ? (
                  <span className="cell-token">&lt;required&gt;</span>
                ) : cellTarget?.value === null || cellTarget?.value === undefined ? (
                  <span className="cell-token">NULL</span>
                ) : cellTarget.value === "" ? (
                  <span className="cell-token">empty string</span>
                ) : (
                  cellTarget.value
                )}
              </div>
              <div className="record-field-actions">
                {canRevertCell && cellTarget && (
                  <button
                    type="button"
                    className="button ghost small"
                    aria-label={`Revert ${col.name}`}
                    onClick={(event) => {
                      event.stopPropagation();
                      dirtyStore.revertCell(cellTarget.resultId, cellTarget.row, col.id);
                    }}
                  >
                    <RotateCcw size={12} />
                    Revert
                  </button>
                )}
                {cellTarget && (
                  <button
                    type="button"
                    className="button ghost small"
                    aria-label={
                      cellTarget.column.editable ? `Edit ${col.name}` : `Inspect ${col.name}`
                    }
                    onClick={(event) => {
                      event.stopPropagation();
                      onSelect(cellTarget);
                      onOpenValue(cellTarget);
                    }}
                  >
                    {cellTarget.column.editable ? "Edit…" : "Inspect…"}
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
