import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
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
import {
  copyFormatLabel,
  formatGridCells,
  MAX_COPY_CELLS,
  writeClipboardText,
  type CopyCell,
  type CopyFormat,
} from "./copy-format";
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
import { opensInValueDialog } from "./value-format";

export type CopyScope = "auto" | "selection" | "row" | "cell" | "loaded";

export interface CopySelectionSummary {
  hasSelection: boolean;
  hasRangeSelection: boolean;
  hasSingleCell: boolean;
  selectedCellCount: number;
  selectedRowCount: number;
  loadedRowCount: number;
  visibleColumnCount: number;
}

export interface DatabaseGridHandle {
  copyAs(
    format: CopyFormat,
    options?: { scope?: CopyScope; includeHeaders?: boolean },
  ): Promise<boolean>;
  selectionSummary(): CopySelectionSummary;
}

export interface CellTarget {
  resultId: string;
  row: number;
  column: ColumnInfo;
  value: string | null;
  large: boolean;
  insert?: {
    tabId: string;
    insertId: string;
    usingDefault: boolean;
    explicitNull: boolean;
    unfilledRequired: boolean;
  };
  deleted?: boolean;
}
interface Props {
  tab: StudioTab;
  result?: ResultInfo;
  theme: string;
  onSelect(target: CellTarget | null): void;
  onOpenValue(target: CellTarget): void;
  onSelectedRowsChange?(rows: number[]): void;
  focusRowRequest?: { seq: number; row: number } | null;
  /** Server-side header sorting for table tabs; absent when unavailable. */
  onSort?(column: ColumnInfo, additive: boolean): void;
}

function selectedVisualRows(selection: GridSelection): number[] {
  const rows = new Set<number>(selection.rows.toArray());
  if (selection.current) {
    const { range, rangeStack } = selection.current;
    for (let y = range.y; y < range.y + range.height; y++) rows.add(y);
    for (const rect of rangeStack) {
      for (let y = rect.y; y < rect.y + rect.height; y++) rows.add(y);
    }
  }
  return [...rows].sort((a, b) => a - b);
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
  insertable: false,
  hasDefault: false,
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

const DatabaseGrid = forwardRef<DatabaseGridHandle, Props>(function DatabaseGrid(
  {
    tab,
    result,
    theme: themeName,
    onSelect,
    onOpenValue,
    onSelectedRowsChange,
    focusRowRequest,
    onSort,
  }: Props,
  ref,
) {
  const grid = useRef<DataEditorRef>(null),
    host = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 800, height: 400 });
  const [widths, setWidths] = useState<Record<string, number>>({});
  const density = useStudio((state) => state.density);
  const dirtyVersion = useSyncExternalStore(dirtyStore.subscribe, dirtyStore.snapshot);
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
  const draftInserts = tab.kind === "table" ? dirtyStore.insertsForTab(tab.id) : [];
  const rowCount =
    tab.kind === "benchmark"
      ? 1_000_000
      : tab.kind === "table"
        ? draftInserts.length > 0
          ? tab.loadedRows + draftInserts.length
          : tab.loadedRows + (tab.hasMore ? 1 : 0)
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
    const empty = { rows: CompactSelection.empty(), columns: CompactSelection.empty() };
    selectionRef.current = empty;
    setSelection(empty);
    onSelectedRowsChange?.([]);
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

  const locateDraft = useCallback((row: number) => {
    const { tab } = model.current;
    if (tab.kind !== "table" || row < tab.loadedRows) return null;
    const inserts = dirtyStore.insertsForTab(tab.id);
    const index = row - tab.loadedRows;
    const draft = inserts[index];
    return draft ? { tabId: tab.id, draft, index } : null;
  }, []);

  const targetAt = useCallback(
    ([col, row]: Item, full = false): CellTarget | null => {
      const column = visibleColumns[col];
      if (!column) return null;
      const tableEditable =
        connected &&
        !["running", "queued", "outcomeUnknown"].includes(model.current.tab.status) &&
        !!model.current.tab.schema?.editable;
      const draftSource = locateDraft(row);
      if (draftSource) {
        const hasExplicit = column.id in draftSource.draft.values;
        const explicitValue = hasExplicit ? draftSource.draft.values[column.id] : null;
        return {
          resultId: `insert:${draftSource.draft.id}`,
          row: draftSource.index,
          column: { ...column, editable: column.insertable && tableEditable },
          value: hasExplicit ? explicitValue : null,
          large: false,
          insert: {
            tabId: draftSource.tabId,
            insertId: draftSource.draft.id,
            usingDefault: !hasExplicit && column.hasDefault,
            explicitNull: hasExplicit && explicitValue === null,
            unfilledRequired: !hasExplicit && !column.hasDefault && !column.nullable,
          },
        };
      }
      const source = locate(row);
      if (!source) return null;
      const value = resultCache.read(source.result.id, column.id, source.row, full);
      if (!value) return null;
      const deleted = dirtyStore.isDeleted(source.result.id, source.row);
      const dirty = dirtyStore.get(source.result.id, source.row, column.id);
      return {
        resultId: source.result.id,
        row: source.row,
        column: { ...column, editable: column.editable && tableEditable && !deleted },
        value: dirty ? dirty.value : value.value,
        large: value.large,
        deleted,
      };
    },
    [locate, locateDraft, visibleColumns, connected],
  );

  const pendingCanvasFocus = useRef(false);

  useLayoutEffect(() => {
    if (!focusRowRequest || !columns.length) return;
    const targetRow = focusRowRequest.row;
    if (targetRow < 0 || targetRow >= rowCount) return;
    const isDraft = model.current.tab.kind === "table" && targetRow >= model.current.tab.loadedRows;
    const targetCol = isDraft
      ? Math.max(
          0,
          visibleColumns.findIndex((column) => column.insertable),
        )
      : 0;
    const nextSelection: GridSelection = {
      rows: CompactSelection.empty(),
      columns: CompactSelection.empty(),
      current: {
        cell: [targetCol, targetRow],
        range: { x: targetCol, y: targetRow, width: 1, height: 1 },
        rangeStack: [],
      },
    };
    selectionRef.current = nextSelection;
    pendingCanvasFocus.current = true;
    setSelection(nextSelection);
    onSelectedRowsChange?.([targetRow]);
    onSelect(targetAt([targetCol, targetRow], true));
  }, [focusRowRequest?.seq]);

  useLayoutEffect(() => {
    if (!pendingCanvasFocus.current) return;
    pendingCanvasFocus.current = false;
    const cell = selection.current?.cell;
    if (cell) grid.current?.scrollTo?.(cell[0], cell[1]);
    host.current
      ?.querySelector<HTMLElement>('[data-testid="data-grid-canvas"]')
      ?.focus({ preventScroll: true });
  }, [selection]);

  useEffect(() => {
    const rows = selectedVisualRows(selectionRef.current);
    if (rows.some((row) => row >= rowCount)) {
      const empty = { rows: CompactSelection.empty(), columns: CompactSelection.empty() };
      selectionRef.current = empty;
      setSelection(empty);
      onSelectedRowsChange?.([]);
      onSelect(null);
      return;
    }
    const activeCell = selectionRef.current.current?.cell;
    if (activeCell) onSelect(targetAt(activeCell, true));
  }, [dirtyVersion, rowCount, targetAt]);

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
      const draftSource = locateDraft(row);
      if (draftSource) {
        const tableEditable =
          connected &&
          tab.kind === "table" &&
          !!tab.schema?.editable &&
          !["outcomeUnknown", "queued", "running"].includes(tab.status);
        const editable = tableEditable && column.insertable;
        const hasExplicit = column.id in draftSource.draft.values;
        const explicitValue = hasExplicit ? draftSource.draft.values[column.id] : null;
        const override = {
          bgCell: themeName === "light" ? "#e8f8ef" : "#1d3528",
          textDark: themeName === "light" ? "#166534" : "#9fe3b8",
        };
        if (column.kind === "boolean" && hasExplicit && explicitValue !== null) {
          return {
            kind: GridCellKind.Boolean,
            data: explicitValue === "true" || explicitValue === "t" || explicitValue === "1",
            allowOverlay: false,
            readonly: !editable,
            themeOverride: override,
          };
        }
        const displayData = hasExplicit
          ? explicitValue === null
            ? "NULL"
            : explicitValue
          : column.hasDefault
            ? "<default>"
            : column.nullable
              ? "NULL"
              : "<required>";
        return {
          kind: GridCellKind.Text,
          data: hasExplicit && explicitValue !== null ? explicitValue : "",
          displayData,
          allowOverlay: editable && !opensInValueDialog(column),
          readonly: !editable,
          style: !hasExplicit || explicitValue === null ? "faded" : "normal",
          themeOverride: override,
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
      const deleted = dirtyStore.isDeleted(source.result.id, source.row);
      const dirty = dirtyStore.get(source.result.id, source.row, column.id),
        value = dirty ? dirty.value : cached.value;
      const editable =
        !deleted &&
        connected &&
        tab.kind === "table" &&
        column.editable &&
        !!tab.schema?.editable &&
        !["outcomeUnknown", "queued", "running"].includes(tab.status) &&
        !cached.large;
      const override = deleted
        ? {
            bgCell: themeName === "light" ? "#fdecec" : "#3a2024",
            textDark: themeName === "light" ? "#9f2d2d" : "#f29b9b",
          }
        : dirty
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
        allowOverlay: !cached.large && !deleted && !opensInValueDialog(column),
        readonly: !editable,
        style: deleted || value === null ? "faded" : "normal",
        themeOverride: override,
      };
    },
    [visibleColumns, locate, locateDraft, themeName, connected],
  );

  const edit = useCallback(
    (cell: Item, edited: EditableGridCell) => {
      const target = targetAt(cell, true);
      if (!target || !target.column.editable || !model.current.tab.schema?.editable) return;
      try {
        if (target.insert) {
          if (edited.kind === GridCellKind.Boolean) {
            const next =
              edited.data === undefined || edited.data === null
                ? null
                : edited.data
                  ? "true"
                  : "false";
            dirtyStore.setInsertCell(
              target.insert.tabId,
              target.insert.insertId,
              target.column.id,
              next,
            );
          } else if (edited.kind === GridCellKind.Text) {
            const raw = edited.data;
            if (raw === "\\N") {
              if (!target.column.nullable)
                throw new Error(`Column "${target.column.name}" does not allow NULL.`);
              dirtyStore.setInsertCell(
                target.insert.tabId,
                target.insert.insertId,
                target.column.id,
                null,
              );
            } else if (
              raw === "" &&
              target.column.kind !== "text" &&
              target.column.kind !== "unknown"
            ) {
              const resetVal =
                target.column.hasDefault || !target.column.nullable ? undefined : null;
              dirtyStore.setInsertCell(
                target.insert.tabId,
                target.insert.insertId,
                target.column.id,
                resetVal,
              );
            } else if (target.column.kind === "boolean") {
              const lower = raw.trim().toLowerCase();
              if (lower === "true" || lower === "t" || lower === "1") {
                dirtyStore.setInsertCell(
                  target.insert.tabId,
                  target.insert.insertId,
                  target.column.id,
                  "true",
                );
              } else if (lower === "false" || lower === "f" || lower === "0") {
                dirtyStore.setInsertCell(
                  target.insert.tabId,
                  target.insert.insertId,
                  target.column.id,
                  "false",
                );
              } else {
                throw new Error(
                  `Column "${target.column.name}" expects a boolean (true/false, t/f, 1/0).`,
                );
              }
            } else {
              dirtyStore.setInsertCell(
                target.insert.tabId,
                target.insert.insertId,
                target.column.id,
                raw,
              );
            }
          }
          onSelect(targetAt(cell, true));
          return;
        }
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
        dirtyStore.set({
          resultId: target.resultId,
          row: target.row,
          column: target.column.id,
          original: resultCache.read(target.resultId, target.column.id, target.row, true)!.value,
          value,
        });
        onSelect(targetAt(cell, true));
      } catch (error) {
        useStudio
          .getState()
          .notify(error instanceof Error ? error.message : "Could not edit this value");
      }
    },
    [onSelect, targetAt],
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
      const inserts = currentTab.kind === "table" ? dirtyStore.insertsForTab(currentTab.id) : [];
      const outcome = validatePasteMatrix({
        anchor,
        values,
        visibleColumns,
        tableEditable,
        resolveRow: (visualRow) => {
          if (currentTab.kind !== "table" || visualRow < 0) return { kind: "outOfBounds" };
          if (visualRow >= currentTab.loadedRows) {
            if (inserts.length > 0) {
              const draft = inserts[visualRow - currentTab.loadedRows];
              return draft
                ? { kind: "draftInsert", tabId: currentTab.id, insertId: draft.id }
                : { kind: "outOfBounds" };
            }
            const totalRows = currentTab.loadedRows + (currentTab.hasMore ? 1 : 0);
            return visualRow < totalRows ? { kind: "unloaded" } : { kind: "outOfBounds" };
          }
          const source = locate(visualRow);
          if (!source) return { kind: "unloaded" };
          return {
            kind: "existing",
            resultId: source.result.id,
            row: source.row,
            deleted: dirtyStore.isDeleted(source.result.id, source.row),
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
        dirtyStore.applyBatch(outcome.cells, outcome.insertCells);
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

  const resolveCopyMatrix = useCallback(
    (scope: CopyScope) => {
      const { tab: currentTab, result: currentResult } = model.current;
      const draftCount =
        currentTab.kind === "table" ? dirtyStore.insertsForTab(currentTab.id).length : 0;
      const maxCopyableRows =
        currentTab.kind === "benchmark"
          ? 1_000_000
          : currentTab.kind === "table"
            ? currentTab.loadedRows + draftCount
            : (currentResult?.retainedRows ?? 0);
      const loadedDefaultRows =
        currentTab.kind === "benchmark"
          ? 1_000_000
          : currentTab.kind === "table"
            ? currentTab.loadedRows > 0
              ? currentTab.loadedRows
              : draftCount
            : (currentResult?.retainedRows ?? 0);

      const sel = selectionRef.current;
      const rowList = sel.rows.toArray().filter((r) => r >= 0 && r < maxCopyableRows);
      const colList = sel.columns.toArray().filter((c) => c >= 0 && c < visibleColumns.length);
      const rects = sel.current ? [sel.current.range, ...sel.current.rangeStack] : [];
      const hasMultiCellRect =
        rects.length > 1 || (rects.length === 1 && rects[0].width * rects[0].height > 1);
      const hasRangeSelection = rowList.length > 0 || colList.length > 0 || hasMultiCellRect;
      const hasSingleCell = !!sel.current && !hasRangeSelection;

      const selectionCols = new Set<number>(colList);
      const selectionRows = new Set<number>(rowList);
      for (const rect of rects) {
        for (let x = rect.x; x < rect.x + rect.width; x++) {
          if (x >= 0 && x < visibleColumns.length) selectionCols.add(x);
        }
        for (let y = rect.y; y < rect.y + rect.height; y++) {
          if (y >= 0 && y < maxCopyableRows) selectionRows.add(y);
        }
      }
      if (selectionCols.size === 0 && selectionRows.size > 0) {
        for (let c = 0; c < visibleColumns.length; c++) selectionCols.add(c);
      }
      if (selectionRows.size === 0 && selectionCols.size > 0) {
        for (let r = 0; r < loadedDefaultRows; r++) selectionRows.add(r);
      }

      const summary: CopySelectionSummary = {
        hasSelection: hasRangeSelection || hasSingleCell,
        hasRangeSelection,
        hasSingleCell,
        selectedCellCount: selectionCols.size * selectionRows.size,
        selectedRowCount: selectedVisualRows(sel).filter((r) => r >= 0 && r < maxCopyableRows)
          .length,
        loadedRowCount: loadedDefaultRows,
        visibleColumnCount: visibleColumns.length,
      };

      const effectiveScope: Exclude<CopyScope, "auto"> =
        scope === "auto" ? (hasRangeSelection ? "selection" : "loaded") : scope;

      let colIndices: number[];
      let rowIndices: number[];
      if (effectiveScope === "loaded") {
        colIndices = visibleColumns.map((_, i) => i);
        rowIndices = Array.from({ length: loadedDefaultRows }, (_, i) => i);
      } else if (effectiveScope === "row") {
        colIndices = visibleColumns.map((_, i) => i);
        rowIndices = selectedVisualRows(sel).filter((r) => r >= 0 && r < maxCopyableRows);
      } else if (effectiveScope === "cell") {
        const active = sel.current?.cell;
        colIndices =
          active && active[0] >= 0 && active[0] < visibleColumns.length ? [active[0]] : [];
        rowIndices = active && active[1] >= 0 && active[1] < maxCopyableRows ? [active[1]] : [];
      } else if (selectionCols.size > 0 && selectionRows.size > 0) {
        colIndices = [...selectionCols].sort((a, b) => a - b);
        rowIndices = [...selectionRows].sort((a, b) => a - b);
      } else {
        colIndices = visibleColumns.map((_, i) => i);
        rowIndices = Array.from({ length: loadedDefaultRows }, (_, i) => i);
      }

      return { colIndices, rowIndices, summary };
    },
    [visibleColumns],
  );

  const copyAs = useCallback(
    async (
      format: CopyFormat,
      options?: { scope?: CopyScope; includeHeaders?: boolean },
    ): Promise<boolean> => {
      const { colIndices, rowIndices } = resolveCopyMatrix(options?.scope ?? "auto");
      if (colIndices.length === 0) {
        useStudio.getState().notify("No visible columns to copy.");
        return false;
      }
      if (rowIndices.length === 0) {
        useStudio.getState().notify("No rows to copy.");
        return false;
      }
      if (colIndices.length * rowIndices.length > MAX_COPY_CELLS) {
        useStudio
          .getState()
          .notify(
            `Select at most ${MAX_COPY_CELLS.toLocaleString()} cells to copy, or use Export for larger datasets.`,
          );
        return false;
      }

      const tabId = model.current.tab.id;
      const revision = model.current.tab.revision;
      try {
        const rows: CopyCell[][] = [];
        for (const row of rowIndices) {
          if (model.current.tab.id !== tabId || model.current.tab.revision !== revision) {
            return false;
          }
          if (model.current.tab.kind === "benchmark") {
            rows.push(colIndices.map((col) => benchmarkValue(col, row)));
            continue;
          }
          if (
            model.current.tab.kind === "table" &&
            row < model.current.tab.loadedRows &&
            !locate(row)
          ) {
            await useStudio.getState().ensureTablePage(tabId, row);
          }
          const source =
            model.current.tab.kind !== "table" || row < model.current.tab.loadedRows
              ? locate(row)
              : null;
          if (source) {
            await resultCache.ensure(source.result.id, source.row, source.result.retainedRows);
            const pageError = resultCache.error(source.result.id, source.row);
            if (pageError) throw new Error(pageError);
          }
          const line: CopyCell[] = [];
          for (const col of colIndices) {
            const target = targetAt([col, row], true);
            if (!target) {
              throw new Error("Wait for the selected rows to load before copying.");
            }
            line.push(target.insert?.usingDefault ? { kind: "default" } : target.value);
          }
          rows.push(line);
        }

        const currentTab = model.current.tab;
        const profile = useStudio
          .getState()
          .bootstrap?.profiles.find((p) => p.id === currentTab.connectionId);
        const formatted = formatGridCells({
          columns: colIndices.map((col) => visibleColumns[col]),
          rows,
          format,
          engine: profile?.engine,
          table: currentTab.kind === "table" ? currentTab.table : null,
          includeHeaders: options?.includeHeaders,
        });
        if (!formatted.ok) {
          useStudio.getState().notify(formatted.error);
          return false;
        }
        await writeClipboardText(formatted.text);
        useStudio
          .getState()
          .notify(
            `Copied ${formatted.rowCount.toLocaleString()} row${formatted.rowCount === 1 ? "" : "s"} as ${copyFormatLabel(format)}`,
          );
        return true;
      } catch (error) {
        useStudio
          .getState()
          .notify(error instanceof Error ? error.message : "Could not copy these cells");
        return false;
      }
    },
    [locate, resolveCopyMatrix, targetAt, visibleColumns],
  );

  useImperativeHandle(
    ref,
    () => ({
      copyAs,
      selectionSummary: () => resolveCopyMatrix("auto").summary,
    }),
    [copyAs, resolveCopyMatrix],
  );

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
          onCellClicked={(cell) => {
            if (
              !host.current?.contains(document.activeElement) &&
              !document.activeElement?.closest("#portal")
            ) {
              host.current
                ?.querySelector<HTMLElement>('[data-testid="data-grid-canvas"]')
                ?.focus({ preventScroll: true });
            }
            onSelect(targetAt(cell, true));
          }}
          gridSelection={selection}
          onGridSelectionChange={(nextSelection) => {
            selectionRef.current = nextSelection;
            setSelection(nextSelection);
            onSelectedRowsChange?.(selectedVisualRows(nextSelection));
            onSelect(nextSelection.current ? targetAt(nextSelection.current.cell, true) : null);
          }}
          onCellActivated={(cell) => {
            const target = targetAt(cell, true);
            if (target && (target.large || opensInValueDialog(target.column))) onOpenValue(target);
          }}
          onVisibleRegionChanged={(range) => {
            visible.current = range;
            const current = model.current.tab;
            if (
              current.kind === "table" &&
              current.hasMore &&
              dirtyStore.insertsForTab(current.id).length === 0 &&
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
                  if (
                    model.current.tab.kind === "table" &&
                    row < model.current.tab.loadedRows &&
                    !locate(row)
                  )
                    await useStudio.getState().ensureTablePage(tabId, row);
                  const source = row < model.current.tab.loadedRows ? locate(row) : null;
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
                    const copyText = target.insert?.usingDefault
                      ? "<default>"
                      : (target.value ?? "NULL");
                    bytes += copyText.length * 2;
                    if (bytes > 8 * 1024 * 1024)
                      throw new Error("Copy is limited to 8 MiB. Select fewer cells.");
                    line.push({
                      kind: GridCellKind.Text,
                      data: copyText,
                      displayData: copyText,
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
});

export default DatabaseGrid;
