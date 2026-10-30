import { useState } from "react";
import { ArrowDown, ArrowUp, RotateCcw } from "lucide-react";
import type { ColumnInfo, SavedTableLayout, TableRef } from "../../ipc/generated";
import { Modal } from "../../components/ui/Modal";
import {
  MAX_COLUMN_WIDTH,
  MIN_COLUMN_WIDTH,
  moveOrderedColumn,
  resizeTableColumn,
  resolveTableLayout,
  toggleColumnVisibility,
} from "./table-layout";

interface Props {
  connectionId: string;
  table: TableRef;
  columns: ColumnInfo[];
  layout: SavedTableLayout | null;
  onChange(layout: SavedTableLayout | null): void;
  onClose(): void;
}

export function TableColumnsDialog({
  connectionId,
  table,
  columns,
  layout,
  onChange,
  onClose,
}: Props) {
  const [draftWidths, setDraftWidths] = useState<Record<string, string>>({});
  const resolved = resolveTableLayout(columns, layout);
  const soleVisible = resolved.visibleColumns.length <= 1;

  return (
    <Modal
      title="Table columns"
      subtitle={`${table.schema}.${table.name} · ${resolved.visibleColumns.length} of ${resolved.orderedColumns.length} visible`}
      onClose={onClose}
    >
      <div className="modal-body table-columns-dialog">
        <div className="table-columns-list" role="list" aria-label="Columns">
          {resolved.orderedColumns.map((column, index) => {
            const visible = !resolved.hiddenSet.has(column.name);
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
                </label>
                <div className="table-column-controls">
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
