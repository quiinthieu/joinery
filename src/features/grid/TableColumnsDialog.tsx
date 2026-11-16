import { useMemo, useState } from "react";
import { ArrowDown, ArrowUp, Maximize2, Pin, PinOff, RotateCcw, Search } from "lucide-react";
import type { ColumnInfo, SavedTableLayout, TableRef } from "../../ipc/generated";
import { Modal } from "../../components/ui/Modal";
import {
  MAX_COLUMN_WIDTH,
  MIN_COLUMN_WIDTH,
  fitColumnsToWindow,
  hideOptionalColumns,
  moveOrderedColumn,
  resizeTableColumn,
  resolveTableLayout,
  showAllColumns,
  toggleColumnPin,
  toggleColumnVisibility,
} from "./table-layout";

interface Props {
  connectionId: string;
  table: TableRef;
  subtitleLabel?: string;
  columns: ColumnInfo[];
  layout: SavedTableLayout | null;
  onChange(layout: SavedTableLayout | null): void;
  onClose(): void;
}

export function TableColumnsDialog({
  connectionId,
  table,
  subtitleLabel,
  columns,
  layout,
  onChange,
  onClose,
}: Props) {
  const [draftWidths, setDraftWidths] = useState<Record<string, string>>({});
  const [search, setSearch] = useState("");
  const resolved = resolveTableLayout(columns, layout);
  const soleVisible = resolved.visibleColumns.length <= 1;
  const normalizedQuery = search.trim().toLowerCase();
  const filteredEntries = useMemo(
    () =>
      resolved.orderedColumns
        .map((column, index) => ({ column, index }))
        .filter(
          ({ column }) =>
            !normalizedQuery ||
            column.name.toLowerCase().includes(normalizedQuery) ||
            column.databaseType.toLowerCase().includes(normalizedQuery),
        ),
    [normalizedQuery, resolved.orderedColumns],
  );
  const targetLabel = subtitleLabel ?? `${table.schema}.${table.name}`;

  return (
    <Modal
      title="Table columns"
      subtitle={`${targetLabel} · ${resolved.visibleColumns.length} of ${resolved.orderedColumns.length} visible${
        resolved.pinnedCount > 0 ? ` · ${resolved.pinnedCount} pinned` : ""
      }`}
      onClose={onClose}
    >
      <div className="modal-body table-columns-dialog">
        <div className="table-columns-toolbar">
          <label className="diagram-filter table-columns-search">
            <Search size={13} />
            <input
              aria-label="Search columns"
              placeholder="Search columns or types…"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </label>
          <div className="table-columns-bulk-actions">
            <button
              type="button"
              className="button ghost small"
              disabled={resolved.hiddenSet.size === 0}
              onClick={() => onChange(showAllColumns(columns, layout, connectionId, table))}
            >
              Show all
            </button>
            <button
              type="button"
              className="button ghost small"
              onClick={() => onChange(hideOptionalColumns(columns, layout, connectionId, table))}
            >
              Hide optional
            </button>
            <button
              type="button"
              className="button ghost small"
              aria-label="Fit columns to window"
              onClick={() => {
                const hostWidth =
                  document.querySelector(".grid-host")?.clientWidth ??
                  Math.max(480, window.innerWidth - 340);
                setDraftWidths({});
                onChange(fitColumnsToWindow(columns, layout, connectionId, table, hostWidth));
              }}
            >
              <Maximize2 size={12} />
              Fit to window
            </button>
          </div>
        </div>
        <div className="table-columns-list" role="list" aria-label="Columns">
          {filteredEntries.map(({ column, index }) => {
            const visible = !resolved.hiddenSet.has(column.name);
            const pinned = resolved.pinnedSet.has(column.name);
            const width = resolved.widths[column.name];
            return (
              <div className="table-column-row" role="listitem" key={column.id}>
                <label className="table-column-toggle">
                  <input
                    type="checkbox"
                    aria-label={`Show ${column.name}`}
                    checked={visible}
                    disabled={visible && soleVisible}
                    onChange={() =>
                      onChange(
                        toggleColumnVisibility(columns, layout, connectionId, table, column.name),
                      )
                    }
                  />
                  <span className="mono">{column.name}</span>
                  <span className="table-column-meta">{column.databaseType}</span>
                  {pinned && <span className="query-error-badge">Pinned</span>}
                </label>
                <div className="table-column-controls">
                  <button
                    type="button"
                    className={`icon-button ${pinned ? "active" : ""}`.trim()}
                    aria-label={pinned ? `Unpin ${column.name}` : `Pin ${column.name}`}
                    aria-pressed={pinned}
                    title={pinned ? "Unpin column" : "Pin column to left"}
                    onClick={() =>
                      onChange(toggleColumnPin(columns, layout, connectionId, table, column.name))
                    }
                  >
                    {pinned ? <PinOff size={13} /> : <Pin size={13} />}
                  </button>
                  <label className="table-column-width">
                    <span>px</span>
                    <input
                      type="number"
                      className="mono"
                      aria-label={`Width for ${column.name}`}
                      min={MIN_COLUMN_WIDTH}
                      max={MAX_COLUMN_WIDTH}
                      value={draftWidths[column.name] ?? String(width)}
                      onChange={(event) => {
                        const raw = event.target.value;
                        setDraftWidths((old) => ({ ...old, [column.name]: raw }));
                        const parsed = Number(raw);
                        if (
                          raw.trim() !== "" &&
                          Number.isFinite(parsed) &&
                          parsed >= MIN_COLUMN_WIDTH &&
                          parsed <= MAX_COLUMN_WIDTH
                        ) {
                          onChange(
                            resizeTableColumn(
                              columns,
                              layout,
                              connectionId,
                              table,
                              column.name,
                              parsed,
                            ),
                          );
                        }
                      }}
                      onBlur={(event) => {
                        const raw = event.target.value.trim();
                        const parsed = Number(raw);
                        setDraftWidths((old) => {
                          const next = { ...old };
                          delete next[column.name];
                          return next;
                        });
                        if (raw !== "" && Number.isFinite(parsed) && parsed > 0) {
                          onChange(
                            resizeTableColumn(
                              columns,
                              layout,
                              connectionId,
                              table,
                              column.name,
                              parsed,
                            ),
                          );
                        }
                      }}
                    />
                  </label>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={`Move ${column.name} up`}
                    disabled={index === 0}
                    onClick={() =>
                      onChange(
                        moveOrderedColumn(columns, layout, connectionId, table, index, index - 1),
                      )
                    }
                  >
                    <ArrowUp size={13} />
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={`Move ${column.name} down`}
                    disabled={index === resolved.orderedColumns.length - 1}
                    onClick={() =>
                      onChange(
                        moveOrderedColumn(columns, layout, connectionId, table, index, index + 1),
                      )
                    }
                  >
                    <ArrowDown size={13} />
                  </button>
                </div>
              </div>
            );
          })}
          {filteredEntries.length === 0 && (
            <div className="grid-empty">No columns match "{search.trim()}"</div>
          )}
        </div>
      </div>
      <footer className="modal-footer">
        <button
          type="button"
          className="button ghost"
          disabled={!resolved.customized}
          onClick={() => {
            setDraftWidths({});
            onChange(null);
          }}
        >
          <RotateCcw size={13} />
          Reset column layout
        </button>
        <button type="button" className="button primary" onClick={onClose}>
          Done
        </button>
      </footer>
    </Modal>
  );
}
