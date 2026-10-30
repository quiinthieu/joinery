import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import DataEditor, {
  GridCellKind,
  CompactSelection,
  type GridSelection,
  type DataEditorRef,
  type EditableGridCell,
  type GridCell,
  type GridColumn,
  type Item,
  type Rectangle,
  type Theme,
} from "@glideapps/glide-data-grid";
import "@glideapps/glide-data-grid/dist/index.css";
import type { ColumnInfo, ResultInfo } from "../../ipc/generated";
import { resultCache } from "../../data/result-cache";
import { dirtyStore } from "../../data/dirty-store";
import { useStudio, type StudioTab } from "../../state/studio";
import { parseTsvClipboard, validatePasteMatrix } from "./paste";
import {
  clampColumnWidth,
  defaultColumnWidth,
  findTableLayout,
  moveVisibleColumn,
  resizeTableColumn,
  resolveTableLayout,
} from "./table-layout";
import { canOrder, sortIndicator } from "./table-query";

export interface CellTarget {
  resultId: string;
  row: number;
  column: ColumnInfo;
  value: string | null;
  large: boolean;
}
interface Props {
  tab: StudioTab;
  result?: ResultInfo;
  theme: string;
  onSelect(target: CellTarget | null): void;
  onOpenValue(target: CellTarget): void;
  /** Server-side header sorting for table tabs; absent when unavailable. */
  onSort?(column: ColumnInfo, additive: boolean): void;
}

const benchmarkColumns: ColumnInfo[] = [
  ["id", "bigint", "integer"],
  ["workspace", "text", "text"],
  ["status", "text", "text"],
  ["owner", "text", "text"],
  ["email", "text", "text"],
  ["amount", "numeric", "decimal"],
  ["region", "text", "text"],
  ["created_at", "timestamptz", "temporal"],
].map(([name, databaseType, kind], i) => ({
  id: `c${i}`,
  name,
  databaseType,
  kind: kind as ColumnInfo["kind"],
  nullable: false,
  primaryKey: i === 0,
  foreignKey: false,
  editable: false,
}));
function benchmarkValue(col: number, row: number) {
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

export default function DatabaseGrid({
  tab,
  result,
  theme: themeName,
  onSelect,
  onOpenValue,
  onSort,
}: Props) {
  const grid = useRef<DataEditorRef>(null),
    host = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 800, height: 400 });
  const [widths, setWidths] = useState<Record<string, number>>({});
  const density = useStudio((state) => state.density);
  const [selection, setSelection] = useState<GridSelection>({
    rows: CompactSelection.empty(),
    columns: CompactSelection.empty(),
  });
  const connected = useStudio(
    (state) => !!(tab.connectionId && state.sessions[tab.connectionId]?.id === tab.dataSessionId),
  );
  const visible = useRef<Rectangle>({ x: 0, y: 0, width: 8, height: 20 });
  const model = useRef({ tab, result });
  model.current = { tab, result };
  const rawColumnsInfo =
    tab.kind === "benchmark"
      ? benchmarkColumns
      : tab.kind === "table"
        ? (tab.schema?.columns ?? [])
        : (result?.columns ?? []);
  const tableLayout = useStudio((state) =>
    tab.kind === "table" && tab.connectionId && tab.table
      ? findTableLayout(state.tableLayouts, tab.connectionId, tab.table)
      : null,
  );
  const resolvedLayout = useMemo(
    () => resolveTableLayout(rawColumnsInfo, tableLayout),
    [rawColumnsInfo, tableLayout],
  );
  const visibleColumns = resolvedLayout.visibleColumns;
  const visibleOrderKey = useMemo(
    () => visibleColumns.map((column) => column.id).join(","),
    [visibleColumns],
  );
  const freezeColumns = visibleColumns[0]?.primaryKey ? 1 : 0;
  const frozen = useRef(freezeColumns);
  frozen.current = freezeColumns;
  const sortQuery = tab.kind === "table" ? tab.query : null;
  const columns = useMemo<GridColumn[]>(
    () =>
      visibleColumns.map((column) => {
        const indicator = sortQuery ? sortIndicator(sortQuery, column.name) : "";
        return {
          id: column.id,
          title: `${column.primaryKey ? "⌑ " : column.foreignKey ? "↗ " : ""}${column.name}${
            indicator ? ` ${indicator}` : ""
          }`,
          width:
            tab.kind === "table"
              ? (resolvedLayout.widths[column.name] ?? defaultColumnWidth(column))
              : (widths[column.id] ?? defaultColumnWidth(column)),
          icon:
            column.kind === "integer" || column.kind === "decimal"
              ? "headerNumber"
              : column.kind === "boolean"
                ? "headerBoolean"
                : column.kind === "temporal"
                  ? "headerDate"
                  : column.kind === "json"
                    ? "headerCode"
                    : "headerString",
        };
      }),
    [visibleColumns, resolvedLayout.widths, tab.kind, widths, sortQuery],
  );
  const rowCount =
    tab.kind === "benchmark"
      ? 1_000_000
      : tab.kind === "table"
        ? tab.loadedRows + (tab.hasMore ? 1 : 0)
        : (result?.retainedRows ?? 0);
  const theme = useMemo<Partial<Theme>>(() => {
    const style = getComputedStyle(document.documentElement),
      color = (key: string) => style.getPropertyValue(key).trim();
    return {
      accentColor: color("--accent"),
      accentLight: color("--selection"),
      textDark: color("--text"),
      textMedium: color("--muted"),
      textLight: color("--faint"),
      textHeader: color("--muted"),
      bgCell: color("--surface"),
      bgCellMedium: color("--surface-hover"),
      bgHeader: color("--header"),
      bgHeaderHasFocus: color("--surface-hover"),
      bgHeaderHovered: color("--surface-hover"),
      borderColor: color("--border"),
      horizontalBorderColor: color("--row-border"),
      headerBottomBorderColor: color("--border"),
      baseFontStyle: "12px",
      headerFontStyle: "500 11px",
      fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
      cellHorizontalPadding: 16,
      editorFontSize: "13px",
    };
  }, [themeName]);

  useEffect(() => {
    const observer = new ResizeObserver(([entry]) =>
      setSize({
        width: Math.max(1, Math.floor(entry.contentRect.width)),
        height: Math.max(1, Math.floor(entry.contentRect.height)),
      }),
    );
    if (host.current) observer.observe(host.current);
    return () => observer.disconnect();
  }, []);

  const redraw = useCallback(() => {
    const range = visible.current,
      cells: { cell: Item }[] = [];
    // Glide's visible region starts after frozen columns, which also need
    // repainting once their page arrives.
    const columns = new Set<number>();
    for (let x = 0; x < frozen.current; x++) columns.add(x);
    for (let x = range.x; x < range.x + range.width; x++) columns.add(x);
    for (let y = range.y; y < range.y + range.height + 1; y++)
      for (const x of columns) cells.push({ cell: [x, y] });
    grid.current?.updateCells(cells);
  }, []);
  useEffect(() => resultCache.subscribe(redraw), [redraw]);
  useEffect(() => dirtyStore.subscribe(redraw), [redraw]);
  useEffect(() => {
    setSelection({ rows: CompactSelection.empty(), columns: CompactSelection.empty() });
    onSelect(null);
    redraw();
  }, [tab.id, tab.revision, result?.id, visibleOrderKey, redraw]);

  const locate = useCallback((row: number) => {
    const { tab, result } = model.current;
    if (tab.kind === "table") {
      const page = tab.pages.find((p) => row >= p.start && row < p.start + p.result.retainedRows);
      return page ? { result: page.result, row: row - page.start } : null;
    }
    return result ? { result, row } : null;
  }, []);
  const targetAt = useCallback(
    ([col, row]: Item, full = false): CellTarget | null => {
      const source = locate(row),
        column = visibleColumns[col];
      if (!source || !column) return null;
      const value = resultCache.read(source.result.id, column.id, source.row, full);
      if (!value) return null;
      const dirty = dirtyStore.get(source.result.id, source.row, column.id);
      const editable =
        connected &&
        !["running", "queued", "outcomeUnknown"].includes(model.current.tab.status) &&
        !!model.current.tab.schema?.editable;
      return {
        resultId: source.result.id,
        row: source.row,
        column: { ...column, editable: column.editable && editable },
        value: dirty ? dirty.value : value.value,
        large: value.large,
      };
    },
    [locate, visibleColumns, connected],
  );

  const getCellContent = useCallback(
    ([col, row]: Item): GridCell => {
      const { tab } = model.current,
        column = visibleColumns[col];
      if (!column) return { kind: GridCellKind.Loading, allowOverlay: false };
      if (tab.kind === "benchmark") {
        const value = benchmarkValue(col, row);
        return {
          kind: GridCellKind.Text,
          data: value,
          displayData: value,
          allowOverlay: true,
          readonly: true,
        };
      }
      const source = locate(row);
      if (!source) {
        if (tab.kind === "table" && row < tab.loadedRows)
          void useStudio.getState().ensureTablePage(tab.id, row);
        return { kind: GridCellKind.Loading, allowOverlay: false };
      }
      const cached = resultCache.read(source.result.id, column.id, source.row);
      if (!cached) {
        if (!resultCache.error(source.result.id, source.row))
          void resultCache.ensure(source.result.id, source.row, source.result.retainedRows);
        else
          return {
            kind: GridCellKind.Text,
            data: "",
            displayData: "Unable to load",
            allowOverlay: false,
            readonly: true,
            style: "faded",
          };
        return { kind: GridCellKind.Loading, allowOverlay: false };
      }
      const dirty = dirtyStore.get(source.result.id, source.row, column.id),
        value = dirty ? dirty.value : cached.value;
      const editable =
        connected &&
        tab.kind === "table" &&
        column.editable &&
        !!tab.schema?.editable &&
        !["outcomeUnknown", "queued", "running"].includes(tab.status) &&
        !cached.large;
      const override = dirty
        ? {
            bgCell: themeName === "light" ? "#fff3d9" : "#353021",
            textDark: themeName === "light" ? "#845715" : "#e7c384",
          }
        : undefined;
      if (column.kind === "boolean" && value !== null)
        return {
          kind: GridCellKind.Boolean,
          data: value === "true" || value === "t" || value === "1",
          allowOverlay: false,
          readonly: !editable,
          themeOverride: override,
        };
      return {
        kind: GridCellKind.Text,
        data: value ?? "",
        displayData: value === null ? "NULL" : value,
        allowOverlay: !cached.large,
        readonly: !editable,
        style: value === null ? "faded" : "normal",
        themeOverride: override,
      };
    },
    [visibleColumns, locate, themeName, connected],
  );

  const edit = useCallback(
    (cell: Item, edited: EditableGridCell) => {
      const target = targetAt(cell, true);
      if (!target || !target.column.editable || !model.current.tab.schema?.editable) return;
      const value =
        edited.kind === GridCellKind.Boolean
          ? edited.data === undefined || edited.data === null
            ? null
            : edited.data
              ? "true"
              : "false"
          : edited.kind === GridCellKind.Text
            ? edited.data
            : target.value;
      try {
        dirtyStore.set({
          resultId: target.resultId,
          row: target.row,
          column: target.column.id,
          original: resultCache.read(target.resultId, target.column.id, target.row, true)!.value,
          value,
        });
      } catch (error) {
        useStudio
          .getState()
          .notify(error instanceof Error ? error.message : "Could not edit this value");
      }
    },
    [targetAt],
  );

  const selectionRef = useRef(selection);
  selectionRef.current = selection;

  const applyPaste = useCallback(
    (anchor: Item, values: readonly (readonly string[])[]) => {
      const currentTab = model.current.tab;
      const tableEditable =
        connected &&
        currentTab.kind === "table" &&
        !!currentTab.schema?.editable &&
        !["outcomeUnknown", "queued", "running"].includes(currentTab.status);
      const outcome = validatePasteMatrix({
        anchor,
        values,
        visibleColumns,
        tableEditable,
        resolveRow: (visualRow) => {
          if (currentTab.kind !== "table" || visualRow < 0) return { kind: "outOfBounds" };
          const totalRows = currentTab.loadedRows + (currentTab.hasMore ? 1 : 0);
          if (visualRow >= totalRows) return { kind: "outOfBounds" };
          if (visualRow >= currentTab.loadedRows) return { kind: "unloaded" };
          const source = locate(visualRow);
          if (!source) return { kind: "unloaded" };
          return {
            kind: "existing",
            resultId: source.result.id,
            row: source.row,
            readOriginal: (columnId) =>
              resultCache.read(source.result.id, columnId, source.row, true)?.value,
          };
        },
      });
      if (!outcome.ok) {
        useStudio.getState().notify(outcome.error);
        return;
      }
      try {
        dirtyStore.setBatch(outcome.cells);
        const activeCell = selectionRef.current.current?.cell;
        if (activeCell) onSelect(targetAt(activeCell, true));
      } catch (error) {
        useStudio
          .getState()
          .notify(error instanceof Error ? error.message : "Could not paste these cells");
      }
    },
    [connected, locate, onSelect, targetAt, visibleColumns],
  );

  const applyPasteRef = useRef(applyPaste);
  applyPasteRef.current = applyPaste;

  useLayoutEffect(() => {
    const onWindowPaste = (event: ClipboardEvent) => {
      if (!host.current?.contains(document.activeElement)) return;
      if (
        document.activeElement instanceof HTMLInputElement ||
        document.activeElement instanceof HTMLTextAreaElement
      )
        return;
      const sel = selectionRef.current;
      const anchor: Item | null = sel.current
        ? [sel.current.range.x, sel.current.range.y]
        : sel.columns.length === 1
          ? [sel.columns.first() ?? 0, 0]
          : sel.rows.length === 1
            ? [0, sel.rows.first() ?? 0]
            : null;
      if (!anchor || !event.clipboardData) return;
      if (!event.clipboardData.types.includes("text/plain")) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      const raw = event.clipboardData.getData("text/plain");
      applyPasteRef.current(anchor, parseTsvClipboard(raw));
    };
    window.addEventListener("paste", onWindowPaste, true);
    return () => window.removeEventListener("paste", onWindowPaste, true);
  }, []);

  return (
    <div className="grid-host" ref={host}>
      {columns.length ? (
        <DataEditor
          ref={grid}
          width={size.width}
          height={size.height}
          columns={columns}
          rows={rowCount}
          getCellContent={getCellContent}
          rowMarkers="number"
          rowHeight={density === "compact" ? 28 : 36}
          headerHeight={37}
          smoothScrollX
          smoothScrollY
          theme={theme}
          freezeColumns={freezeColumns}
          onColumnResize={(column, width) => {
            if (tab.kind === "table" && tab.connectionId && tab.table && column.id) {
              const target = visibleColumns.find((entry) => entry.id === column.id);
              if (target) {
                useStudio
                  .getState()
                  .setTableLayout(
                    tab.connectionId,
                    tab.table,
                    resizeTableColumn(
                      rawColumnsInfo,
                      tableLayout,
                      tab.connectionId,
                      tab.table,
                      target.name,
                      width,
                    ),
                  );
              }
              return;
            }
            if (column.id) setWidths((old) => ({ ...old, [column.id!]: clampColumnWidth(width) }));
          }}
          onColumnMoved={
            tab.kind === "table" && tab.connectionId && tab.table
              ? (startIndex, endIndex) => {
                  useStudio
                    .getState()
                    .setTableLayout(
                      tab.connectionId!,
                      tab.table!,
                      moveVisibleColumn(
                        rawColumnsInfo,
                        tableLayout,
                        tab.connectionId!,
                        tab.table!,
                        startIndex,
                        endIndex,
                      ),
                    );
                }
              : undefined
          }
          onCellEdited={edit}
          onPaste={(target, values) => {
            applyPaste(target, values);
            return false;
          }}
          onHeaderClicked={(col, event) => {
            const column = visibleColumns[col];
            if (!onSort || !column || !canOrder(column)) return;
            event.preventDefault();
            onSort(column, event.shiftKey);
          }}
          onCellClicked={(cell) => onSelect(targetAt(cell, true))}
          gridSelection={selection}
          onGridSelectionChange={(selection) => {
            setSelection(selection);
            onSelect(selection.current ? targetAt(selection.current.cell, true) : null);
          }}
          onCellActivated={(cell) => {
            const target = targetAt(cell, true);
            if (
              target &&
              (target.large || target.column.kind === "json" || target.column.kind === "temporal")
            )
              onOpenValue(target);
          }}
          onVisibleRegionChanged={(range) => {
            visible.current = range;
            const current = model.current.tab;
            if (
              current.kind === "table" &&
              current.hasMore &&
              range.y + range.height >= current.loadedRows - 40
            )
              void useStudio.getState().loadNextPage(current.id);
          }}
          getCellsForSelection={(rectangle, signal) => {
            if (rectangle.width * rectangle.height > 10000) {
              useStudio.getState().notify("Select at most 10,000 cells to copy.");
              return [];
            }
            return async () => {
              const revision = model.current.tab.revision,
                tabId = model.current.tab.id;
              try {
                let bytes = 0;
                const cells: GridCell[][] = [];
                for (let row = rectangle.y; row < rectangle.y + rectangle.height; row++) {
                  if (
                    signal.aborted ||
                    model.current.tab.id !== tabId ||
                    model.current.tab.revision !== revision
                  )
                    return [];
                  if (model.current.tab.kind === "table" && !locate(row))
                    await useStudio.getState().ensureTablePage(tabId, row);
                  const source = locate(row);
                  if (source)
                    await resultCache.ensure(
                      source.result.id,
                      source.row,
                      source.result.retainedRows,
                    );
                  const line: GridCell[] = [];
                  for (let col = rectangle.x; col < rectangle.x + rectangle.width; col++) {
                    if (model.current.tab.kind === "benchmark") {
                      line.push(getCellContent([col, row]));
                      continue;
                    }
                    const target = targetAt([col, row], true);
                    if (!target)
                      throw new Error("Wait for the selected rows to load before copying.");
                    bytes += (target.value?.length ?? 0) * 2;
                    if (bytes > 8 * 1024 * 1024)
                      throw new Error("Copy is limited to 8 MiB. Select fewer cells.");
                    line.push({
                      kind: GridCellKind.Text,
                      data: target.value ?? "NULL",
                      displayData: target.value ?? "NULL",
                      allowOverlay: false,
                      readonly: true,
                    });
                  }
                  cells.push(line);
                }
                return cells;
              } catch (error) {
                useStudio
                  .getState()
                  .notify(error instanceof Error ? error.message : "Could not copy these cells");
                return [];
              }
            };
          }}
        />
      ) : (
        <div className="grid-empty">
          {tab.loadingPage || tab.status === "running"
            ? "Waiting for results…"
            : "No rows to display"}
        </div>
      )}
    </div>
  );
}
