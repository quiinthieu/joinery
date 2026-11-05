export const DEFAULT_GRID_PAGE_SIZE = 500 as const;

export const GRID_PAGE_SIZE_OPTIONS = [100, 200, 500, 1000, "all"] as const;

export type GridPageSize = (typeof GRID_PAGE_SIZE_OPTIONS)[number];

export interface GridPaginationInput {
  totalLoadedRows: number;
  pageSize: GridPageSize;
  pageIndex: number;
  hasMore?: boolean;
  loadingMore?: boolean;
}

export interface GridPaginationState {
  pageSize: GridPageSize;
  pageIndex: number;
  loadedPages: number;
  totalPages: number;
  pageOffset: number;
  pageRowLimit: number | null;
  visibleRowCount: number;
  startRowNumber: number;
  endRowNumber: number;
  canFirst: boolean;
  canPrevious: boolean;
  canNext: boolean;
  canLast: boolean;
  loadingMore: boolean;
  rangeLabel: string;
}

export function parseGridPageSize(raw: string): GridPageSize {
  if (raw === "all") return "all";
  const parsed = Number(raw);
  if (parsed === 100 || parsed === 200 || parsed === 500 || parsed === 1000) {
    return parsed;
  }
  return DEFAULT_GRID_PAGE_SIZE;
}

export function pageIndexForRow(row: number, pageSize: GridPageSize): number {
  if (pageSize === "all" || row <= 0) return 0;
  return Math.floor(row / pageSize);
}

export function computeGridPagination(input: GridPaginationInput): GridPaginationState {
  const totalLoadedRows = Math.max(0, Math.trunc(input.totalLoadedRows));
  const { pageSize } = input;
  const hasMore = Boolean(input.hasMore);
  const loadingMore = Boolean(input.loadingMore);

  const loadedPages =
    pageSize === "all" ? 1 : Math.max(1, Math.ceil(totalLoadedRows / Math.max(1, pageSize)));
  const pageIndex = Math.max(0, Math.min(Math.trunc(input.pageIndex), loadedPages - 1));
  const pageOffset = pageSize === "all" ? 0 : pageIndex * pageSize;
  const pageRowLimit = pageSize === "all" ? null : pageSize;
  const visibleRowCount =
    pageSize === "all"
      ? totalLoadedRows
      : Math.max(0, Math.min(pageSize, totalLoadedRows - pageOffset));

  const startRowNumber = totalLoadedRows === 0 || visibleRowCount === 0 ? 0 : pageOffset + 1;
  const endRowNumber =
    totalLoadedRows === 0 || visibleRowCount === 0 ? 0 : pageOffset + visibleRowCount;

  const canFirst = pageIndex > 0;
  const canPrevious = pageIndex > 0;
  const canNext = pageIndex + 1 < loadedPages || (pageSize !== "all" && hasMore);
  const canLast = pageIndex + 1 < loadedPages;

  const rangeLabel =
    totalLoadedRows === 0
      ? "0 rows"
      : `${startRowNumber.toLocaleString()}–${endRowNumber.toLocaleString()} of ${totalLoadedRows.toLocaleString()}${hasMore ? "+" : ""}`;

  return {
    pageSize,
    pageIndex,
    loadedPages,
    totalPages: loadedPages,
    pageOffset,
    pageRowLimit,
    visibleRowCount,
    startRowNumber,
    endRowNumber,
    canFirst,
    canPrevious,
    canNext,
    canLast,
    loadingMore,
    rangeLabel,
  };
}
