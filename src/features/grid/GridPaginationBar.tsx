import { ChevronFirst, ChevronLast, ChevronLeft, ChevronRight } from "lucide-react";
import {
  GRID_PAGE_SIZE_OPTIONS,
  parseGridPageSize,
  type GridPageSize,
  type GridPaginationState,
} from "./pagination";

interface Props {
  pagination: GridPaginationState;
  disabled?: boolean;
  loading?: boolean;
  onFirstPage(): void;
  onPreviousPage(): void;
  onNextPage(): void;
  onLastPage(): void;
  onPageSizeChange(pageSize: GridPageSize): void;
}

export function GridPaginationBar({
  pagination,
  disabled = false,
  loading = false,
  onFirstPage,
  onPreviousPage,
  onNextPage,
  onLastPage,
  onPageSizeChange,
}: Props) {
  const isBusy = disabled || loading || pagination.loadingMore;
  return (
    <div className="grid-pagination-bar" role="group" aria-label="Result pagination">
      <button
        type="button"
        className="icon-button grid-page-btn"
        aria-label="First page"
        title="First page"
        disabled={isBusy || !pagination.canFirst}
        onClick={onFirstPage}
      >
        <ChevronFirst size={13} />
      </button>
      <button
        type="button"
        className="icon-button grid-page-btn"
        aria-label="Previous page"
        title="Previous page"
        disabled={isBusy || !pagination.canPrevious}
        onClick={onPreviousPage}
      >
        <ChevronLeft size={13} />
      </button>
      <span
        className="grid-pagination-range"
        data-testid="grid-pagination-range"
        title={`Page ${pagination.pageIndex + 1} of ${pagination.loadedPages}`}
      >
        {pagination.rangeLabel}
      </span>
      <button
        type="button"
        className="icon-button grid-page-btn"
        aria-label="Next page"
        title="Next page"
        disabled={isBusy || !pagination.canNext}
        onClick={onNextPage}
      >
        <ChevronRight size={13} />
      </button>
      <button
        type="button"
        className="icon-button grid-page-btn"
        aria-label="Last page"
        title="Last page"
        disabled={isBusy || !pagination.canLast}
        onClick={onLastPage}
      >
        <ChevronLast size={13} />
      </button>
      <select
        className="grid-page-size-select"
        aria-label="Page size"
        title="Rows per page"
        disabled={isBusy}
        value={String(pagination.pageSize)}
        onChange={(event) => onPageSizeChange(parseGridPageSize(event.target.value))}
      >
        {GRID_PAGE_SIZE_OPTIONS.map((opt) => (
          <option key={String(opt)} value={String(opt)}>
            {opt === "all" ? "All rows" : `${opt.toLocaleString()} / page`}
          </option>
        ))}
      </select>
    </div>
  );
}
