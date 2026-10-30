import type {
  BrowseQuery,
  ColumnFilter,
  ColumnInfo,
  FilterOp,
  ForeignKey,
  SortKey,
  TableSchema,
} from "../../ipc/generated";

export const EMPTY_QUERY: BrowseQuery = {
  sort: [],
  filters: [],
  whereClause: null,
  orderClause: null,
};

export const FILTER_OPERATORS: { op: FilterOp; label: string; needsValue: boolean }[] = [
  { op: "eq", label: "=", needsValue: true },
  { op: "notEq", label: "≠", needsValue: true },
  { op: "lt", label: "<", needsValue: true },
  { op: "lte", label: "≤", needsValue: true },
  { op: "gt", label: ">", needsValue: true },
  { op: "gte", label: "≥", needsValue: true },
  { op: "contains", label: "contains", needsValue: true },
  { op: "startsWith", label: "starts with", needsValue: true },
  { op: "isNull", label: "is NULL", needsValue: false },
  { op: "isNotNull", label: "is not NULL", needsValue: false },
];

export function needsValue(op: FilterOp) {
  return FILTER_OPERATORS.find((o) => o.op === op)?.needsValue ?? true;
}

/** Mirrors the backend's coarse rule; the backend remains authoritative. */
export function canOrder(column: ColumnInfo) {
  return column.kind !== "json" && column.kind !== "binary" && column.kind !== "unknown";
}

export function allowedOperators(column: ColumnInfo): FilterOp[] {
  if (column.kind === "binary") return ["isNull", "isNotNull"];
  return FILTER_OPERATORS.filter(
    (o) => canOrder(column) || ["contains", "startsWith", "isNull", "isNotNull"].includes(o.op),
  ).map((o) => o.op);
}

export function normalizeClause(clause: string | null | undefined): string | null {
  const trimmed = clause?.trim() ?? "";
  return trimmed ? trimmed : null;
}

export function normalizeQuery(query: BrowseQuery): BrowseQuery {
  return {
    sort: query.sort,
    filters: query.filters,
    whereClause: normalizeClause(query.whereClause),
    orderClause: normalizeClause(query.orderClause),
  };
}

export function hasRawClauses(query: BrowseQuery) {
  return normalizeClause(query.whereClause) !== null || normalizeClause(query.orderClause) !== null;
}

export function isEmptyQuery(query: BrowseQuery) {
  return query.sort.length === 0 && query.filters.length === 0 && !hasRawClauses(query);
}

export function sameQuery(a: BrowseQuery, b: BrowseQuery) {
  return JSON.stringify(normalizeQuery(a)) === JSON.stringify(normalizeQuery(b));
}

/**
 * Header-click sorting: ascending → descending → off. A plain click makes the
 * column the only key; an additive (Shift) click keeps the other keys.
 */
export function cycleSort(query: BrowseQuery, column: string, additive: boolean): BrowseQuery {
  const current = query.sort.find((key) => key.column === column);
  const next: SortKey | null = !current
    ? { column, descending: false }
    : !current.descending
      ? { column, descending: true }
      : null;
  let sort: SortKey[];
  if (additive) {
    sort = current
      ? query.sort.flatMap((key) => (key.column === column ? (next ? [next] : []) : [key]))
      : [...query.sort, next!];
  } else {
    sort = next ? [next] : [];
  }
  return { ...query, sort, orderClause: null };
}

export function sortIndicator(query: BrowseQuery, column: string) {
  const index = query.sort.findIndex((key) => key.column === column);
  if (index < 0) return "";
  const arrow = query.sort[index].descending ? "↓" : "↑";
  return query.sort.length > 1 ? `${arrow}${index + 1}` : arrow;
}

function literal(value: string) {
  return `'${value.replaceAll("'", "''")}'`;
}

export function describeFilter(filter: ColumnFilter) {
  const operator = FILTER_OPERATORS.find((o) => o.op === filter.op)?.label ?? filter.op;
  return filter.value === null
    ? `${filter.column} ${operator}`
    : `${filter.column} ${operator} ${literal(filter.value)}`;
}

export function describeSort(query: BrowseQuery) {
  return query.sort
    .map((key) => `${key.column} ${key.descending ? "descending" : "ascending"}`)
    .join(", ");
}

export function withFilter(query: BrowseQuery, filter: ColumnFilter): BrowseQuery {
  return { ...query, filters: [...query.filters, filter] };
}

export function withoutFilter(query: BrowseQuery, index: number): BrowseQuery {
  return { ...query, filters: query.filters.filter((_, i) => i !== index) };
}

/** Foreign-key constraints that include `column`, in schema order. */
export function constraintsForColumn(schema: TableSchema | null, column: string): ForeignKey[] {
  return schema?.foreignKeys.filter((key) => key.columns.includes(column)) ?? [];
}

/**
 * Equality filters selecting the row referenced by `key`, given the source row
 * values by column name. Returns null when any key column is NULL: SQL foreign
 * keys with a NULL component reference nothing.
 */
export function referencedRowsQuery(
  key: ForeignKey,
  value: (column: string) => string | null | undefined,
): BrowseQuery | null {
  const filters: ColumnFilter[] = [];
  for (let i = 0; i < key.columns.length; i++) {
    const current = value(key.columns[i]);
    if (current === null || current === undefined) return null;
    filters.push({ column: key.targetColumns[i], op: "eq", value: current });
  }
  return { sort: [], filters, whereClause: null, orderClause: null };
}

export function tableTabTitle(name: string, query: BrowseQuery) {
  if (!query.filters.length) {
    const raw = normalizeClause(query.whereClause);
    return raw ? `${name} · WHERE` : name;
  }
  const shown = query.filters
    .slice(0, 2)
    .map((filter) =>
      filter.op === "eq" ? `${filter.column} = ${filter.value}` : describeFilter(filter),
    )
    .join(", ");
  return `${name} · ${shown}${query.filters.length > 2 ? "…" : ""}`;
}
