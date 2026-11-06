import { describe, expect, it, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { VisualDiffViewer } from "./VisualDiffViewer";
import type { RowDiff } from "./mutation-diff";

afterEach(cleanup);

describe("VisualDiffViewer", () => {
  const mockDiffRows: RowDiff[] = [
    {
      id: "update:res-1:0",
      kind: "update",
      resultId: "res-1",
      rowIndex: 0,
      primaryKeySummary: "id = 1",
      tableName: "CUSTOMERS",
      cells: [
        {
          columnId: "c0",
          columnName: "id",
          isPrimaryKey: true,
          before: "1",
          after: "1",
          changed: false,
        },
        {
          columnId: "c1",
          columnName: "customer_name",
          isPrimaryKey: false,
          before: "Acme Corp",
          after: "Acme Industries",
          changed: true,
        },
        {
          columnId: "c2",
          columnName: "balance",
          isPrimaryKey: false,
          before: "100.50",
          after: "150.00",
          changed: true,
        },
      ],
      ref: { kind: "update", resultId: "res-1", row: 0 },
    },
    {
      id: "delete:res-1:1",
      kind: "delete",
      resultId: "res-1",
      rowIndex: 1,
      primaryKeySummary: "id = 2",
      tableName: "CUSTOMERS",
      cells: [
        {
          columnId: "c0",
          columnName: "id",
          isPrimaryKey: true,
          before: "2",
          after: null,
          changed: true,
        },
        {
          columnId: "c1",
          columnName: "customer_name",
          isPrimaryKey: false,
          before: "Globex Inc",
          after: null,
          changed: true,
        },
      ],
      ref: { kind: "delete", resultId: "res-1", row: 1 },
    },
    {
      id: "insert:tab-1:inst-1",
      kind: "insert",
      insertId: "inst-1",
      primaryKeySummary: "New Row #1",
      tableName: "CUSTOMERS",
      cells: [
        {
          columnId: "c0",
          columnName: "id",
          isPrimaryKey: true,
          before: null,
          after: "3",
          changed: true,
        },
        {
          columnId: "c1",
          columnName: "customer_name",
          isPrimaryKey: false,
          before: null,
          after: "Initech",
          changed: true,
        },
      ],
      ref: { kind: "insert", tabId: "tab-1", insertId: "inst-1" },
    },
  ];

  it("renders row diffs with mutation badges and PK summaries", () => {
    const selected = new Set(["update:res-1:0", "delete:res-1:1", "insert:tab-1:inst-1"]);
    render(
      <VisualDiffViewer
        diffRows={mockDiffRows}
        selectedIds={selected}
        onToggle={vi.fn()}
        onToggleAll={vi.fn()}
        layout="unified"
        onLayoutChange={vi.fn()}
        changedColsOnly={false}
        onChangedColsOnlyChange={vi.fn()}
      />,
    );

    expect(screen.getByText("MODIFIED")).toBeDefined();
    expect(screen.getByText("DELETED")).toBeDefined();
    expect(screen.getByText("INSERTED")).toBeDefined();
    expect(screen.getByText(/CUSTOMERS · id = 1/)).toBeDefined();
    expect(screen.getByText(/CUSTOMERS · id = 2/)).toBeDefined();
    expect(screen.getByText(/CUSTOMERS · New Row #1/)).toBeDefined();
    expect(screen.getByText("3 of 3 changes selected")).toBeDefined();
  });

  it("toggles single row checkboxes and master select-all checkbox", () => {
    const onToggle = vi.fn();
    const onToggleAll = vi.fn();
    const selected = new Set(["update:res-1:0"]);

    render(
      <VisualDiffViewer
        diffRows={mockDiffRows}
        selectedIds={selected}
        onToggle={onToggle}
        onToggleAll={onToggleAll}
        layout="unified"
        onLayoutChange={vi.fn()}
        changedColsOnly={false}
        onChangedColsOnlyChange={vi.fn()}
      />,
    );

    expect(screen.getByText("1 of 3 changes selected")).toBeDefined();

    // Toggle row checkbox by label
    const rowCheck = screen.getByLabelText("Include change for id = 1");
    fireEvent.click(rowCheck);
    expect(onToggle).toHaveBeenCalledWith("update:res-1:0");

    // Click master checkbox
    const masterLabel = screen.getByText("1 of 3 changes selected");
    fireEvent.click(masterLabel);
    expect(onToggleAll).toHaveBeenCalled();
  });

  it("supports switching between unified and split diff layouts", () => {
    const onLayoutChange = vi.fn();
    const selected = new Set(["update:res-1:0"]);

    const { rerender } = render(
      <VisualDiffViewer
        diffRows={[mockDiffRows[0]]}
        selectedIds={selected}
        onToggle={vi.fn()}
        onToggleAll={vi.fn()}
        layout="unified"
        onLayoutChange={onLayoutChange}
        changedColsOnly={false}
        onChangedColsOnlyChange={vi.fn()}
      />,
    );

    // Click Split layout button
    fireEvent.click(screen.getByRole("button", { name: /Split/ }));
    expect(onLayoutChange).toHaveBeenCalledWith("split");

    // Rerender with split layout
    rerender(
      <VisualDiffViewer
        diffRows={[mockDiffRows[0]]}
        selectedIds={selected}
        onToggle={vi.fn()}
        onToggleAll={vi.fn()}
        layout="split"
        onLayoutChange={onLayoutChange}
        changedColsOnly={false}
        onChangedColsOnlyChange={vi.fn()}
      />,
    );

    expect(screen.getByText("Original (Before)")).toBeDefined();
    expect(screen.getByText("Staged (After)")).toBeDefined();
  });

  it("supports toggling modified columns only", () => {
    const onChangedColsOnlyChange = vi.fn();
    const selected = new Set(["update:res-1:0"]);

    render(
      <VisualDiffViewer
        diffRows={[mockDiffRows[0]]}
        selectedIds={selected}
        onToggle={vi.fn()}
        onToggleAll={vi.fn()}
        layout="unified"
        onLayoutChange={vi.fn()}
        changedColsOnly={true}
        onChangedColsOnlyChange={onChangedColsOnlyChange}
      />,
    );

    const filterCheckbox = screen.getByLabelText("Modified columns only");
    fireEvent.click(filterCheckbox);
    expect(onChangedColsOnlyChange).toHaveBeenCalledWith(false);
  });
});
