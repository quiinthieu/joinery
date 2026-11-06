import { describe, expect, it, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { VisualDiffViewer } from "./VisualDiffViewer";
import { ReviewDialog } from "./ReviewDialog";
import { filterTableChanges, type RowDiff } from "./mutation-diff";
import { DirtyStore } from "../../data/dirty-store";
import type { ChangePreview, PreparedPlan, TableChanges } from "../../ipc/generated";

afterEach(cleanup);

describe("Selective Unstaging & Partial Reconciliation Adversarial Tests", () => {
  const create5RowsFixture = (): {
    tableChanges: TableChanges;
    diffRows: RowDiff[];
  } => {
    const diffRows: RowDiff[] = [
      {
        id: "update:res-1:0",
        kind: "update",
        resultId: "res-1",
        rowIndex: 0,
        primaryKeySummary: "id = 101",
        tableName: "ORDERS",
        cells: [
          {
            columnId: "c0",
            columnName: "id",
            isPrimaryKey: true,
            before: "101",
            after: "101",
            changed: false,
          },
          {
            columnId: "c1",
            columnName: "status",
            isPrimaryKey: false,
            before: "OPEN",
            after: "SHIPPED",
            changed: true,
          },
        ],
        ref: { kind: "update", resultId: "res-1", row: 0 },
      },
      {
        id: "update:res-1:1",
        kind: "update",
        resultId: "res-1",
        rowIndex: 1,
        primaryKeySummary: "id = 102",
        tableName: "ORDERS",
        cells: [
          {
            columnId: "c0",
            columnName: "id",
            isPrimaryKey: true,
            before: "102",
            after: "102",
            changed: false,
          },
          {
            columnId: "c1",
            columnName: "status",
            isPrimaryKey: false,
            before: "PENDING",
            after: "CANCELLED",
            changed: true,
          },
        ],
        ref: { kind: "update", resultId: "res-1", row: 1 },
      },
      {
        id: "update:res-1:2",
        kind: "update",
        resultId: "res-1",
        rowIndex: 2,
        primaryKeySummary: "id = 103",
        tableName: "ORDERS",
        cells: [
          {
            columnId: "c0",
            columnName: "id",
            isPrimaryKey: true,
            before: "103",
            after: "103",
            changed: false,
          },
          {
            columnId: "c1",
            columnName: "status",
            isPrimaryKey: false,
            before: "HOLD",
            after: "OPEN",
            changed: true,
          },
        ],
        ref: { kind: "update", resultId: "res-1", row: 2 },
      },
      {
        id: "delete:res-1:3",
        kind: "delete",
        resultId: "res-1",
        rowIndex: 3,
        primaryKeySummary: "id = 104",
        tableName: "ORDERS",
        cells: [
          {
            columnId: "c0",
            columnName: "id",
            isPrimaryKey: true,
            before: "104",
            after: null,
            changed: true,
          },
          {
            columnId: "c1",
            columnName: "status",
            isPrimaryKey: false,
            before: "REJECTED",
            after: null,
            changed: true,
          },
        ],
        ref: { kind: "delete", resultId: "res-1", row: 3 },
      },
      {
        id: "delete:res-1:4",
        kind: "delete",
        resultId: "res-1",
        rowIndex: 4,
        primaryKeySummary: "id = 105",
        tableName: "ORDERS",
        cells: [
          {
            columnId: "c0",
            columnName: "id",
            isPrimaryKey: true,
            before: "105",
            after: null,
            changed: true,
          },
          {
            columnId: "c1",
            columnName: "status",
            isPrimaryKey: false,
            before: "ARCHIVED",
            after: null,
            changed: true,
          },
        ],
        ref: { kind: "delete", resultId: "res-1", row: 4 },
      },
    ];

    const tableChanges: TableChanges = {
      resultId: "res-1",
      updates: [
        {
          resultId: "res-1",
          changes: [
            { row: 0, column: "c1", value: "SHIPPED" },
            { row: 1, column: "c1", value: "CANCELLED" },
            { row: 2, column: "c1", value: "OPEN" },
          ],
        },
      ],
      deletes: [
        {
          resultId: "res-1",
          rows: [3, 4],
        },
      ],
      inserts: [],
    };

    return { tableChanges, diffRows };
  };

  it("selective unstaging filters execution plan to exactly the 3 checked rows", () => {
    const { tableChanges, diffRows } = create5RowsFixture();

    // Select rows 0, 2 (updates) and 3 (delete) — unchecking row 1 (update) and row 4 (delete)
    const selectedIds = new Set(["update:res-1:0", "update:res-1:2", "delete:res-1:3"]);
    const filtered = filterTableChanges(tableChanges, selectedIds, diffRows);

    // Updates should only contain rows 0 and 2
    expect(filtered.updates.length).toBe(1);
    expect(filtered.updates[0].changes.map((c) => c.row)).toEqual([0, 2]);

    // Deletes should only contain row 3
    expect(filtered.deletes.length).toBe(1);
    expect(filtered.deletes[0].rows).toEqual([3]);

    // Inserts are empty
    expect(filtered.inserts.length).toBe(0);
  });

  it("partial commit reconciliation preserves unchecked rows in dirtyStore with untouched original baselines", () => {
    const { diffRows } = create5RowsFixture();
    const store = new DirtyStore();

    // Populate dirtyStore to mirror the 5 staged rows
    store.set({ resultId: "res-1", row: 0, column: "c1", original: "OPEN", value: "SHIPPED" });
    store.set({ resultId: "res-1", row: 1, column: "c1", original: "PENDING", value: "CANCELLED" });
    store.set({ resultId: "res-1", row: 2, column: "c1", original: "HOLD", value: "OPEN" });
    store.setDeleted("res-1", [3, 4], true);

    expect(store.count).toBe(5);

    // Simulate committing only the 3 checked rows: row 0, row 2, and row 3
    const selectedIds = new Set(["update:res-1:0", "update:res-1:2", "delete:res-1:3"]);
    const committedDiffs = diffRows.filter((r) => selectedIds.has(r.id));
    expect(committedDiffs.length).toBe(3);

    // Run reconciliation logic identical to App.tsx onSuccess handler
    const committedUpdates: { resultId: string; row: number; column: string }[] = [];
    for (const diff of committedDiffs) {
      if (diff.ref.kind === "update") {
        for (const c of diff.cells) {
          if (c.changed) {
            committedUpdates.push({
              resultId: diff.ref.resultId,
              row: diff.ref.row,
              column: c.columnId,
            });
          }
        }
      } else if (diff.ref.kind === "delete") {
        store.setDeleted(diff.ref.resultId, [diff.ref.row], false);
      }
    }
    if (committedUpdates.length > 0) {
      store.revertBatch(committedUpdates);
    }

    // VERIFY RECONCILIATION RESULT:
    // Exactly 2 changes remain in dirtyStore
    expect(store.count).toBe(2);

    // Row 0 and Row 2 updates were cleared
    expect(store.hasCell("res-1", 0, "c1")).toBe(false);
    expect(store.hasCell("res-1", 2, "c1")).toBe(false);

    // Row 3 deletion was cleared
    expect(store.isDeleted("res-1", 3)).toBe(false);

    // Row 1 (unchecked update) MUST be preserved with its pristine original baseline
    expect(store.hasCell("res-1", 1, "c1")).toBe(true);
    expect(store.getOriginal("res-1", 1, "c1")).toBe("PENDING");
    expect(store.get("res-1", 1, "c1")?.value).toBe("CANCELLED");

    // Row 4 (unchecked delete) MUST be preserved as deleted
    expect(store.isDeleted("res-1", 4)).toBe(true);
  });

  it("unchecking all rows produces empty plan and disables execution button", () => {
    const { tableChanges, diffRows } = create5RowsFixture();

    const emptySelection = new Set<string>();
    const filtered = filterTableChanges(tableChanges, emptySelection, diffRows);

    expect(filtered.updates).toEqual([]);
    expect(filtered.deletes).toEqual([]);
    expect(filtered.inserts).toEqual([]);

    const mockPlan: PreparedPlan = {
      id: "plan-test-1",
      sessionId: "session-test-1",
      statements: [
        {
          sql: "UPDATE ORDERS SET status = 'SHIPPED' WHERE id = 101",
          kind: "write",
          missingWhere: false,
        },
      ],
      warnings: [],
      requiresApproval: true,
      requiresPassphrase: false,
      confirmationText: null,
      expiresAt: Date.now() + 60000,
      boundParameters: [],
    };

    const mockPreview: ChangePreview = {
      rowCount: 5,
      sql: ["UPDATE ORDERS ..."],
      bindings: [],
      plan: mockPlan,
    };

    // Render ReviewDialog with diffRows
    render(
      <ReviewDialog
        plan={mockPlan}
        preview={mockPreview}
        diffRows={diffRows}
        onExecute={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    // Initially all 5 are selected -> button says "Apply 5 changes" and is NOT disabled
    const submitBtn = screen.getByRole("button", { name: /Apply 5 changes/i });
    expect(submitBtn).toBeDefined();
    expect((submitBtn as HTMLButtonElement).disabled).toBe(false);

    // Click master select-all checkbox to uncheck all
    const masterLabel = screen.getByText("5 of 5 changes selected");
    fireEvent.click(masterLabel);

    // Now 0 changes are selected -> button should display "Select changes to apply" and be disabled
    const disabledBtn = screen.getByRole("button", { name: /Select changes to apply/i });
    expect(disabledBtn).toBeDefined();
    expect((disabledBtn as HTMLButtonElement).disabled).toBe(true);
  });

  it("master checkbox accurately reflects indeterminate state", () => {
    const { diffRows } = create5RowsFixture();

    // Render with 2 of 5 selected
    const selected = new Set(["update:res-1:0", "update:res-1:1"]);
    const { container } = render(
      <VisualDiffViewer
        diffRows={diffRows}
        selectedIds={selected}
        onToggle={vi.fn()}
        onToggleAll={vi.fn()}
        layout="unified"
        onLayoutChange={vi.fn()}
        changedColsOnly={false}
        onChangedColsOnlyChange={vi.fn()}
      />,
    );

    const masterCheckbox = container.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(masterCheckbox).not.toBeNull();
    // In React/DOM, indeterminate property is set
    expect(masterCheckbox.indeterminate).toBe(true);
    expect(masterCheckbox.checked).toBe(false);
  });

  it("null handling in diffs: null to empty string and empty string to null render visibly distinct", () => {
    const nullToEmptyRow: RowDiff = {
      id: "update:res-null:0",
      kind: "update",
      resultId: "res-null",
      rowIndex: 0,
      primaryKeySummary: "id = 1",
      tableName: "NULL_TEST",
      cells: [
        {
          columnId: "c0",
          columnName: "nullable_col",
          isPrimaryKey: false,
          before: null,
          after: "",
          changed: true,
        },
      ],
      ref: { kind: "update", resultId: "res-null", row: 0 },
    };

    const emptyToNullRow: RowDiff = {
      id: "update:res-null:1",
      kind: "update",
      resultId: "res-null",
      rowIndex: 1,
      primaryKeySummary: "id = 2",
      tableName: "NULL_TEST",
      cells: [
        {
          columnId: "c0",
          columnName: "nullable_col",
          isPrimaryKey: false,
          before: "",
          after: null,
          changed: true,
        },
      ],
      ref: { kind: "update", resultId: "res-null", row: 1 },
    };

    const selected = new Set([nullToEmptyRow.id, emptyToNullRow.id]);

    // 1. Unified layout
    const { container, rerender } = render(
      <VisualDiffViewer
        diffRows={[nullToEmptyRow, emptyToNullRow]}
        selectedIds={selected}
        onToggle={vi.fn()}
        onToggleAll={vi.fn()}
        layout="unified"
        onLayoutChange={vi.fn()}
        changedColsOnly={false}
        onChangedColsOnlyChange={vi.fn()}
      />,
    );

    // Verify row 1: NULL (line-through) -> "" (emerald)
    const rowCards = container.querySelectorAll(".diff-row-card");
    expect(rowCards.length).toBe(2);

    const row0 = rowCards[0];
    const beforeText0 = row0.querySelector(".line-through")?.textContent;
    const afterText0 = row0.querySelector(".text-emerald-400")?.textContent;
    expect(beforeText0).toBe("NULL");
    expect(afterText0).toBe('""');

    // Verify row 2: "" (line-through) -> NULL (emerald)
    const row1 = rowCards[1];
    const beforeText1 = row1.querySelector(".line-through")?.textContent;
    const afterText1 = row1.querySelector(".text-emerald-400")?.textContent;
    expect(beforeText1).toBe('""');
    expect(afterText1).toBe("NULL");

    // 2. Split layout
    rerender(
      <VisualDiffViewer
        diffRows={[nullToEmptyRow]}
        selectedIds={new Set([nullToEmptyRow.id])}
        onToggle={vi.fn()}
        onToggleAll={vi.fn()}
        layout="split"
        onLayoutChange={vi.fn()}
        changedColsOnly={false}
        onChangedColsOnlyChange={vi.fn()}
      />,
    );

    expect(screen.getByText("Original (Before)")).toBeDefined();
    expect(screen.getByText("Staged (After)")).toBeDefined();

    // In split layout: original column has NULL, staged has ""
    const splitBefore = container.querySelector(".line-through")?.textContent;
    const splitAfter = container.querySelector(".text-emerald-400")?.textContent;
    expect(splitBefore).toBe("NULL");
    expect(splitAfter).toBe('""');
  });

  it("clicking discard on a row card invokes onDiscardRow callback", () => {
    const { diffRows } = create5RowsFixture();
    const onDiscardRow = vi.fn();

    render(
      <VisualDiffViewer
        diffRows={[diffRows[0]]}
        selectedIds={new Set([diffRows[0].id])}
        onToggle={vi.fn()}
        onToggleAll={vi.fn()}
        layout="unified"
        onLayoutChange={vi.fn()}
        changedColsOnly={false}
        onChangedColsOnlyChange={vi.fn()}
        onDiscardRow={onDiscardRow}
      />,
    );

    const discardBtn = screen.getByRole("button", { name: /Discard/i });
    expect(discardBtn).toBeDefined();
    fireEvent.click(discardBtn);
    expect(onDiscardRow).toHaveBeenCalledWith(diffRows[0]);
  });
});
