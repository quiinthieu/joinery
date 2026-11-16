import type { ColumnInfo, SavedTableLayout, TableRef } from "../../ipc/generated";

export const MIN_COLUMN_WIDTH = 60;
export const MAX_COLUMN_WIDTH = 1600;
export const MAX_TABLE_LAYOUTS = 128;
const MAX_LAYOUT_COLUMNS = 256;

export interface ResolvedTableLayout {
  orderedColumns: ColumnInfo[];
  visibleColumns: ColumnInfo[];
  hiddenSet: ReadonlySet<string>;
  pinnedSet: ReadonlySet<string>;
  pinnedCount: number;
  widths: Readonly<Record<string, number>>;
  customized: boolean;
}

export function clampColumnWidth(width: number): number {
  if (!Number.isFinite(width)) return 200;
  return Math.min(MAX_COLUMN_WIDTH, Math.max(MIN_COLUMN_WIDTH, Math.round(width)));
}

export function defaultColumnWidth(column: ColumnInfo): number {
  if (column.name.includes("email")) return 230;
  if (column.kind === "temporal") return 225;
  if (column.kind === "integer") return 110;
  if (column.name === "status") return 150;
  return 200;
}

export function sameTableLayoutTarget(
  layout: Pick<SavedTableLayout, "connectionId" | "table">,
  connectionId: string,
  table: TableRef,
): boolean {
  return (
    layout.connectionId === connectionId &&
    layout.table.schema === table.schema &&
    layout.table.name === table.name
  );
}

export function findTableLayout(
  layouts: SavedTableLayout[],
  connectionId: string | null,
  table: TableRef | null,
): SavedTableLayout | null {
  if (!connectionId || !table) return null;
  return layouts.find((layout) => sameTableLayoutTarget(layout, connectionId, table)) ?? null;
}

export function isDefaultTableLayout(layout: SavedTableLayout): boolean {
  return (
    layout.columnOrder.length === 0 &&
    layout.hiddenColumns.length === 0 &&
    (layout.pinnedColumns?.length ?? 0) === 0 &&
    Object.keys(layout.columnWidths).length === 0
  );
}

/**
 * Resolves a table's current schema columns against a saved layout:
 * - Preserves known columns in saved `columnOrder`, appending any new schema columns at the end
 * - Ignores dropped columns that no longer exist in `columns`
 * - Groups pinned visible columns on the left so `freezeColumns` matches `pinnedCount`
 * - Guarantees at least one column remains visible when `columns.length > 0`
 */
export function resolveTableLayout(
  columns: ColumnInfo[],
  layout: SavedTableLayout | null | undefined,
): ResolvedTableLayout {
  if (!columns.length) {
    return {
      orderedColumns: [],
      visibleColumns: [],
      hiddenSet: new Set(),
      pinnedSet: new Set(),
      pinnedCount: 0,
      widths: {},
      customized: false,
    };
  }
  const byName = new Map<string, ColumnInfo[]>();
  for (const column of columns) {
    const bucket = byName.get(column.name);
    if (bucket) bucket.push(column);
    else byName.set(column.name, [column]);
  }
  const orderedColumns: ColumnInfo[] = [];
  const placedNames = new Set<string>();
  for (const name of layout?.columnOrder ?? []) {
    if (placedNames.has(name)) continue;
    const bucket = byName.get(name);
    if (bucket) {
      orderedColumns.push(...bucket);
      placedNames.add(name);
    }
  }
  for (const column of columns) {
    if (!placedNames.has(column.name)) {
      orderedColumns.push(column);
    }
  }

  const requestedHidden = new Set((layout?.hiddenColumns ?? []).filter((name) => byName.has(name)));
  if (orderedColumns.length > 0 && orderedColumns.every((col) => requestedHidden.has(col.name))) {
    requestedHidden.delete(orderedColumns[0].name);
  }
  const requestedPinned = new Set(
    (layout?.pinnedColumns ?? []).filter((name) => byName.has(name) && !requestedHidden.has(name)),
  );
  const baseVisible = orderedColumns.filter((column) => !requestedHidden.has(column.name));
  const pinnedVisible = baseVisible.filter((column) => requestedPinned.has(column.name));
  const unpinnedVisible = baseVisible.filter((column) => !requestedPinned.has(column.name));
  const visibleColumns =
    pinnedVisible.length > 0 ? [...pinnedVisible, ...unpinnedVisible] : baseVisible;

  const widths: Record<string, number> = {};
  let hasCustomWidth = false;
  for (const column of orderedColumns) {
    const saved = layout?.columnWidths[column.name];
    if (typeof saved === "number") {
      const clamped = clampColumnWidth(saved);
      widths[column.name] = clamped;
      if (clamped !== defaultColumnWidth(column)) hasCustomWidth = true;
    } else {
      widths[column.name] = defaultColumnWidth(column);
    }
  }

  const orderChanged = orderedColumns.some((column, index) => column.name !== columns[index].name);
  return {
    orderedColumns,
    visibleColumns,
    hiddenSet: requestedHidden,
    pinnedSet: requestedPinned,
    pinnedCount: pinnedVisible.length,
    widths,
    customized:
      orderChanged || requestedHidden.size > 0 || requestedPinned.size > 0 || hasCustomWidth,
  };
}

function buildNormalizedLayout(
  columns: ColumnInfo[],
  connectionId: string,
  table: TableRef,
  orderedNames: string[],
  hiddenNames: Iterable<string>,
  rawWidths: Readonly<Record<string, number | undefined>>,
  pinnedNames: Iterable<string> = [],
): SavedTableLayout {
  const known = new Map(columns.slice(0, MAX_LAYOUT_COLUMNS).map((c) => [c.name, c]));
  const defaultOrder = [...known.keys()];
  const dedupOrder: string[] = [];
  const seen = new Set<string>();
  for (const name of orderedNames) {
    if (known.has(name) && !seen.has(name)) {
      dedupOrder.push(name);
      seen.add(name);
    }
  }
  for (const name of defaultOrder) {
    if (!seen.has(name)) {
      dedupOrder.push(name);
      seen.add(name);
    }
  }
  const orderMatchesDefault =
    dedupOrder.length === defaultOrder.length &&
    dedupOrder.every((name, i) => name === defaultOrder[i]);

  const hiddenSet = new Set<string>();
  for (const name of hiddenNames) {
    if (known.has(name)) hiddenSet.add(name);
  }
  if (dedupOrder.length > 0 && hiddenSet.size >= dedupOrder.length) {
    hiddenSet.delete(dedupOrder[0]);
  }

  const pinnedSet = new Set<string>();
  for (const name of pinnedNames) {
    if (known.has(name) && !hiddenSet.has(name)) pinnedSet.add(name);
  }

  const columnWidths: Record<string, number> = {};
  for (const [name, column] of known) {
    const width = rawWidths[name];
    if (typeof width === "number") {
      const clamped = clampColumnWidth(width);
      if (clamped !== defaultColumnWidth(column)) {
        columnWidths[name] = clamped;
      }
    }
  }

  return {
    connectionId,
    table,
    columnOrder: orderMatchesDefault ? [] : dedupOrder,
    hiddenColumns: dedupOrder.filter((name) => hiddenSet.has(name)),
    pinnedColumns: dedupOrder.filter((name) => pinnedSet.has(name)),
    columnWidths,
  };
}

export function toggleColumnVisibility(
  columns: ColumnInfo[],
  current: SavedTableLayout | null,
  connectionId: string,
  table: TableRef,
  columnName: string,
): SavedTableLayout | null {
  const resolved = resolveTableLayout(columns, current);
  const nextHidden = new Set(resolved.hiddenSet);
  const nextPinned = new Set(resolved.pinnedSet);
  if (nextHidden.has(columnName)) {
    nextHidden.delete(columnName);
  } else {
    if (resolved.visibleColumns.length <= 1) return current;
    nextHidden.add(columnName);
    nextPinned.delete(columnName);
  }
  return buildNormalizedLayout(
    columns,
    connectionId,
    table,
    resolved.orderedColumns.map((c) => c.name),
    nextHidden,
    current?.columnWidths ?? {},
    nextPinned,
  );
}

export function toggleColumnPin(
  columns: ColumnInfo[],
  current: SavedTableLayout | null,
  connectionId: string,
  table: TableRef,
  columnName: string,
): SavedTableLayout {
  const resolved = resolveTableLayout(columns, current);
  const nextHidden = new Set(resolved.hiddenSet);
  const nextPinned = new Set(resolved.pinnedSet);
  if (nextPinned.has(columnName)) {
    nextPinned.delete(columnName);
  } else {
    nextHidden.delete(columnName);
    nextPinned.add(columnName);
  }
  return buildNormalizedLayout(
    columns,
    connectionId,
    table,
    resolved.orderedColumns.map((c) => c.name),
    nextHidden,
    current?.columnWidths ?? {},
    nextPinned,
  );
}

export function showAllColumns(
  columns: ColumnInfo[],
  current: SavedTableLayout | null,
  connectionId: string,
  table: TableRef,
): SavedTableLayout {
  const resolved = resolveTableLayout(columns, current);
  return buildNormalizedLayout(
    columns,
    connectionId,
    table,
    resolved.orderedColumns.map((c) => c.name),
    [],
    current?.columnWidths ?? {},
    resolved.pinnedSet,
  );
}

export function hideOptionalColumns(
  columns: ColumnInfo[],
  current: SavedTableLayout | null,
  connectionId: string,
  table: TableRef,
): SavedTableLayout {
  const resolved = resolveTableLayout(columns, current);
  const nextHidden = new Set<string>();
  for (const col of resolved.orderedColumns) {
    if (!col.primaryKey && col.nullable && !resolved.pinnedSet.has(col.name)) {
      nextHidden.add(col.name);
    }
  }
  if (
    resolved.orderedColumns.length > 0 &&
    resolved.orderedColumns.every((col) => nextHidden.has(col.name))
  ) {
    nextHidden.delete(resolved.orderedColumns[0].name);
  }
  return buildNormalizedLayout(
    columns,
    connectionId,
    table,
    resolved.orderedColumns.map((c) => c.name),
    nextHidden,
    current?.columnWidths ?? {},
    resolved.pinnedSet,
  );
}

export function fitColumnsToWindow(
  columns: ColumnInfo[],
  current: SavedTableLayout | null,
  connectionId: string,
  table: TableRef,
  availableWidth: number,
): SavedTableLayout {
  const resolved = resolveTableLayout(columns, current);
  if (!resolved.visibleColumns.length) {
    return buildNormalizedLayout(
      columns,
      connectionId,
      table,
      resolved.orderedColumns.map((c) => c.name),
      resolved.hiddenSet,
      current?.columnWidths ?? {},
      resolved.pinnedSet,
    );
  }
  const usable = Math.max(
    MIN_COLUMN_WIDTH * resolved.visibleColumns.length,
    Math.floor((Number.isFinite(availableWidth) ? availableWidth : 960) - 48),
  );
  const naturalTotal = resolved.visibleColumns.reduce(
    (sum, col) => sum + defaultColumnWidth(col),
    0,
  );
  const nextWidths: Record<string, number | undefined> = { ...(current?.columnWidths ?? {}) };
  let allocated = 0;
  resolved.visibleColumns.forEach((col, index) => {
    const isLast = index === resolved.visibleColumns.length - 1;
    const share = isLast
      ? usable - allocated
      : Math.round((usable * defaultColumnWidth(col)) / Math.max(1, naturalTotal));
    const clamped = clampColumnWidth(share);
    nextWidths[col.name] = clamped;
    allocated += clamped;
  });
  return buildNormalizedLayout(
    columns,
    connectionId,
    table,
    resolved.orderedColumns.map((c) => c.name),
    resolved.hiddenSet,
    nextWidths,
    resolved.pinnedSet,
  );
}

export function moveOrderedColumn(
  columns: ColumnInfo[],
  current: SavedTableLayout | null,
  connectionId: string,
  table: TableRef,
  fromIndex: number,
  toIndex: number,
): SavedTableLayout {
  const resolved = resolveTableLayout(columns, current);
  const next = resolved.orderedColumns.map((c) => c.name);
  if (
    fromIndex < 0 ||
    toIndex < 0 ||
    fromIndex >= next.length ||
    toIndex >= next.length ||
    fromIndex === toIndex
  ) {
    return buildNormalizedLayout(
      columns,
      connectionId,
      table,
      next,
      resolved.hiddenSet,
      current?.columnWidths ?? {},
      resolved.pinnedSet,
    );
  }
  const [moved] = next.splice(fromIndex, 1);
  next.splice(toIndex, 0, moved);
  return buildNormalizedLayout(
    columns,
    connectionId,
    table,
    next,
    resolved.hiddenSet,
    current?.columnWidths ?? {},
    resolved.pinnedSet,
  );
}

export function moveVisibleColumn(
  columns: ColumnInfo[],
  current: SavedTableLayout | null,
  connectionId: string,
  table: TableRef,
  fromVisibleIndex: number,
  toVisibleIndex: number,
): SavedTableLayout {
  const resolved = resolveTableLayout(columns, current);
  const visible = resolved.visibleColumns.map((c) => c.name);
  if (
    fromVisibleIndex < 0 ||
    toVisibleIndex < 0 ||
    fromVisibleIndex >= visible.length ||
    toVisibleIndex >= visible.length ||
    fromVisibleIndex === toVisibleIndex
  ) {
    return buildNormalizedLayout(
      columns,
      connectionId,
      table,
      resolved.orderedColumns.map((c) => c.name),
      resolved.hiddenSet,
      current?.columnWidths ?? {},
      resolved.pinnedSet,
    );
  }
  const [moved] = visible.splice(fromVisibleIndex, 1);
  visible.splice(toVisibleIndex, 0, moved);
  let cursor = 0;
  const mergedOrder = resolved.orderedColumns.map((column) =>
    resolved.hiddenSet.has(column.name) ? column.name : visible[cursor++],
  );
  return buildNormalizedLayout(
    columns,
    connectionId,
    table,
    mergedOrder,
    resolved.hiddenSet,
    current?.columnWidths ?? {},
    resolved.pinnedSet,
  );
}

export function resizeTableColumn(
  columns: ColumnInfo[],
  current: SavedTableLayout | null,
  connectionId: string,
  table: TableRef,
  columnName: string,
  width: number,
): SavedTableLayout {
  const resolved = resolveTableLayout(columns, current);
  return buildNormalizedLayout(
    columns,
    connectionId,
    table,
    resolved.orderedColumns.map((c) => c.name),
    resolved.hiddenSet,
    {
      ...(current?.columnWidths ?? {}),
      [columnName]: clampColumnWidth(width),
    },
    resolved.pinnedSet,
  );
}
