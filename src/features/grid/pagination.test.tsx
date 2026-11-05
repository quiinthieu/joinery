import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { GridPaginationBar } from "./GridPaginationBar";
import {
  computeGridPagination,
  DEFAULT_GRID_PAGE_SIZE,
  pageIndexForRow,
  parseGridPageSize,
} from "./pagination";

describe("DataGrip-style grid pagination", () => {
  it("defaults to 500 rows per page and slices multi-page result sets", () => {
    expect(DEFAULT_GRID_PAGE_SIZE).toBe(500);

    const first = computeGridPagination({
      totalLoadedRows: 1250,
      pageSize: DEFAULT_GRID_PAGE_SIZE,
      pageIndex: 0,
    });
    expect(first).toMatchObject({
      pageIndex: 0,
      loadedPages: 3,
      pageOffset: 0,
      pageRowLimit: 500,
      visibleRowCount: 500,
      startRowNumber: 1,
      endRowNumber: 500,
      rangeLabel: "1–500 of 1,250",
      canFirst: false,
      canPrevious: false,
      canNext: true,
      canLast: true,
    });

    const second = computeGridPagination({
      totalLoadedRows: 1250,
      pageSize: DEFAULT_GRID_PAGE_SIZE,
      pageIndex: 1,
    });
    expect(second).toMatchObject({
      pageIndex: 1,
      pageOffset: 500,
      pageRowLimit: 500,
      visibleRowCount: 500,
      startRowNumber: 501,
      endRowNumber: 1000,
      rangeLabel: "501–1,000 of 1,250",
      canFirst: true,
      canPrevious: true,
      canNext: true,
      canLast: true,
    });

    const last = computeGridPagination({
      totalLoadedRows: 1250,
      pageSize: DEFAULT_GRID_PAGE_SIZE,
      pageIndex: 99,
    });
    expect(last).toMatchObject({
      pageIndex: 2,
      pageOffset: 1000,
      pageRowLimit: 500,
      visibleRowCount: 250,
      startRowNumber: 1001,
      endRowNumber: 1250,
      rangeLabel: "1,001–1,250 of 1,250",
      canFirst: true,
      canPrevious: true,
      canNext: false,
      canLast: false,
    });
  });

  it("supports open-ended totals (500+) when more server pages are available", () => {
    const state = computeGridPagination({
      totalLoadedRows: 500,
      pageSize: 500,
      pageIndex: 0,
      hasMore: true,
    });
    expect(state.rangeLabel).toBe("1–500 of 500+");
    expect(state.canNext).toBe(true);
    expect(state.canLast).toBe(false);
  });

  it("supports viewing all rows on a single page", () => {
    const all = computeGridPagination({
      totalLoadedRows: 1250,
      pageSize: "all",
      pageIndex: 2,
    });
    expect(all).toMatchObject({
      pageIndex: 0,
      loadedPages: 1,
      pageOffset: 0,
      pageRowLimit: null,
      visibleRowCount: 1250,
      rangeLabel: "1–1,250 of 1,250",
      canFirst: false,
      canPrevious: false,
      canNext: false,
      canLast: false,
    });
  });

  it("parses page size options and maps row indices to page indices", () => {
    expect(parseGridPageSize("100")).toBe(100);
    expect(parseGridPageSize("500")).toBe(500);
    expect(parseGridPageSize("1000")).toBe(1000);
    expect(parseGridPageSize("all")).toBe("all");
    expect(parseGridPageSize("invalid")).toBe(500);

    expect(pageIndexForRow(0, 500)).toBe(0);
    expect(pageIndexForRow(499, 500)).toBe(0);
    expect(pageIndexForRow(500, 500)).toBe(1);
    expect(pageIndexForRow(1249, "all")).toBe(0);
  });

  it("renders DataGrip pagination controls and dispatches navigation callbacks", () => {
    const onFirstPage = vi.fn();
    const onPreviousPage = vi.fn();
    const onNextPage = vi.fn();
    const onLastPage = vi.fn();
    const onPageSizeChange = vi.fn();
    const pagination = computeGridPagination({
      totalLoadedRows: 1500,
      pageSize: 500,
      pageIndex: 1,
    });

    render(
      <GridPaginationBar
        pagination={pagination}
        onFirstPage={onFirstPage}
        onPreviousPage={onPreviousPage}
        onNextPage={onNextPage}
        onLastPage={onLastPage}
        onPageSizeChange={onPageSizeChange}
      />,
    );

    expect(screen.getByTestId("grid-pagination-range").textContent).toBe("501–1,000 of 1,500");

    fireEvent.click(screen.getByRole("button", { name: "First page" }));
    expect(onFirstPage).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Previous page" }));
    expect(onPreviousPage).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    expect(onNextPage).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Last page" }));
    expect(onLastPage).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByRole("combobox", { name: "Page size" }), {
      target: { value: "100" },
    });
    expect(onPageSizeChange).toHaveBeenCalledWith(100);
  });
});
