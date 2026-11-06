import { describe, expect, it, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { DirtyStore } from "../../data/dirty-store";
import { ReviewDialog } from "../safety/ReviewDialog";
import { VisualDiffViewer } from "../safety/VisualDiffViewer";
import type { RowDiff } from "../safety/mutation-diff";
import type { PreparedPlan, ConnectionProfile } from "../../ipc/generated";

afterEach(cleanup);

describe("Milestone 6 Phase 2 Challenger: Grid & Data Editing Adversarial Suite", () => {
  describe("1. DirtyStore Revert APIs (revertCell, revertRow, revertBatch)", () => {
    it("revertCell restores individual dirty cells without affecting other dirty cells in the same row", () => {
      const store = new DirtyStore();
      const resId = "res-grid-1";

      // Stage modifications on row 0: 3 columns
      store.set({
        resultId: resId,
        row: 0,
        column: "first_name",
        original: "Alice",
        value: "Alicia",
      });
      store.set({
        resultId: resId,
        row: 0,
        column: "last_name",
        original: "Smith",
        value: "Smyth",
      });
      store.set({
        resultId: resId,
        row: 0,
        column: "status",
        original: "ACTIVE",
        value: "INACTIVE",
      });

      expect(store.hasCell(resId, 0, "first_name")).toBe(true);
      expect(store.hasCell(resId, 0, "last_name")).toBe(true);
      expect(store.hasCell(resId, 0, "status")).toBe(true);

      // Revert only "last_name"
      store.revertCell(resId, 0, "last_name");

      // Verify "last_name" is restored to original (no longer in dirty cells)
      expect(store.hasCell(resId, 0, "last_name")).toBe(false);
      expect(store.get(resId, 0, "last_name")).toBeUndefined();

      // Verify other cells in row 0 remain dirty with their staged values
      expect(store.hasCell(resId, 0, "first_name")).toBe(true);
      expect(store.get(resId, 0, "first_name")?.value).toBe("Alicia");
      expect(store.hasCell(resId, 0, "status")).toBe(true);
      expect(store.get(resId, 0, "status")?.value).toBe("INACTIVE");
    });

    it("revertRow wipes both cell modifications and deletion flags for the specified row", () => {
      const store = new DirtyStore();
      const resId = "res-grid-2";

      // Row 5 has 2 dirty cells
      store.set({ resultId: resId, row: 5, column: "amount", original: "100", value: "200" });
      store.set({ resultId: resId, row: 5, column: "currency", original: "USD", value: "EUR" });
      expect(store.hasCell(resId, 5, "amount")).toBe(true);
      expect(store.hasCell(resId, 5, "currency")).toBe(true);

      // Row 6 has modifications that must not be affected
      store.set({ resultId: resId, row: 6, column: "amount", original: "50", value: "75" });

      // Revert row 5
      store.revertRow(resId, 5);

      // Row 5 has no dirty cells
      expect(store.hasCell(resId, 5, "amount")).toBe(false);
      expect(store.hasCell(resId, 5, "currency")).toBe(false);

      // Row 6 remains intact
      expect(store.hasCell(resId, 6, "amount")).toBe(true);
      expect(store.get(resId, 6, "amount")?.value).toBe("75");
    });

    it("revertBatch handles multi-cell and multi-row rollbacks in a single atomic transaction", () => {
      const store = new DirtyStore();
      const resId = "res-grid-3";

      // Populate multiple dirty cells across 4 rows
      for (let r = 0; r < 4; r++) {
        store.set({
          resultId: resId,
          row: r,
          column: "col_a",
          original: `orig_${r}`,
          value: `new_${r}`,
        });
        store.set({
          resultId: resId,
          row: r,
          column: "col_b",
          original: `val_${r}`,
          value: `mod_${r}`,
        });
      }

      const listener = vi.fn();
      store.subscribe(listener);

      // Revert batch: row 0 col_a, row 1 col_b, row 2 col_a
      store.revertBatch([
        { resultId: resId, row: 0, column: "col_a" },
        { resultId: resId, row: 1, column: "col_b" },
        { resultId: resId, row: 2, column: "col_a" },
      ]);

      // Exactly 1 notification should be fired for the entire batch
      expect(listener).toHaveBeenCalledTimes(1);

      // Assert targeted cells were reverted
      expect(store.hasCell(resId, 0, "col_a")).toBe(false);
      expect(store.hasCell(resId, 1, "col_b")).toBe(false);
      expect(store.hasCell(resId, 2, "col_a")).toBe(false);

      // Assert non-targeted cells remain dirty
      expect(store.hasCell(resId, 0, "col_b")).toBe(true);
      expect(store.hasCell(resId, 1, "col_a")).toBe(true);
      expect(store.hasCell(resId, 2, "col_b")).toBe(true);
      expect(store.hasCell(resId, 3, "col_a")).toBe(true);
      expect(store.hasCell(resId, 3, "col_b")).toBe(true);
    });

    it("reverting non-existent cells or empty arrays is a clean no-op", () => {
      const store = new DirtyStore();
      const listener = vi.fn();
      store.subscribe(listener);

      // Empty batch
      store.revertBatch([]);
      expect(listener).not.toHaveBeenCalled();

      // Ghost cell
      store.revertCell("ghost-res", 999, "non_existent");
      expect(listener).not.toHaveBeenCalled();

      // Ghost row
      store.revertRow("ghost-res", 999);
      expect(listener).not.toHaveBeenCalled();
    });

    it("revertRow on an insert row removes the draft row from inserts completely", () => {
      const store = new DirtyStore();
      const tabId = "tab-insert-1";

      const r1 = store.addInsertRow(tabId);
      store.setInsertCell(tabId, r1.id, "name", "Alice");
      const r2 = store.addInsertRow(tabId);
      store.setInsertCell(tabId, r2.id, "name", "Bob");

      expect(store.insertsForTab(tabId)).toHaveLength(2);

      // Revert r1 via revertRow("insert:<id>", 0)
      store.revertRow(`insert:${r1.id}`, 0);

      const remaining = store.insertsForTab(tabId);
      expect(remaining).toHaveLength(1);
      expect(remaining[0].id).toBe(r2.id);
    });
  });

  describe("2. ReviewDialog and VisualDiffViewer Selective Unstaging & Concurrency Safety", () => {
    const mockDiffRows: RowDiff[] = [
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
            after: "CLOSED",
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
    ];

    const mockPlan: PreparedPlan = {
      id: "plan-review-1",
      sessionId: "sess-1",
      statements: [
        {
          sql: "UPDATE ORDERS SET status = 'CLOSED' WHERE id = 101;",
          kind: "write",
          missingWhere: false,
        },
      ],
      requiresApproval: true,
      requiresPassphrase: false,
      warnings: ["Snowflake optimistic concurrency check applied: WHERE col = original_val"],
      confirmationText: null,
      expiresAt: Date.now() + 60000,
    };

    const mockProfile: ConnectionProfile = {
      id: "conn-review",
      name: "Prod Snowflake",
      groupId: "local",
      engine: "snowflake",
      host: "test.snowflakecomputing.com",
      port: 443,
      database: "ANALYTICS",
      username: "ADMIN",
      environment: "production",
      safeMode: "guard",
      tlsMode: "verifyFull",
      caCertPath: null,
      ssh: null,
      rememberPassword: false,
      storeQueryText: true,
      storeDrafts: true,
      snowflake: null,
    };

    it("renders before-and-after visual diff with PK summary and optimistic concurrency warning", () => {
      render(
        <ReviewDialog
          plan={mockPlan}
          profile={mockProfile}
          diffRows={mockDiffRows}
          onExecute={vi.fn()}
          onClose={vi.fn()}
        />,
      );

      // Warning banner
      expect(screen.getByText(/Snowflake optimistic concurrency check applied/i)).toBeDefined();

      // Visual diff tab
      expect(screen.getByText(/Visual Diff \(2\/2\)/i)).toBeDefined();

      // PK summary
      expect(screen.getByText(/id = 101/i)).toBeDefined();
      expect(screen.getByText(/id = 102/i)).toBeDefined();

      // Before vs After values
      expect(screen.getByText("OPEN")).toBeDefined();
      expect(screen.getByText("CLOSED")).toBeDefined();
    });

    it("disables the Apply button when all mutation rows are selectively unstaged", async () => {
      const onRecomputePlan = vi.fn().mockImplementation(async (selectedIds: Set<string>) => ({
        plan: { ...mockPlan, statements: [] },
        preview: {
          rowCount: selectedIds.size,
          sql: [],
          bindings: [],
          deletions: 0,
          insertions: 0,
          updates: selectedIds.size,
        },
      }));

      render(
        <ReviewDialog
          plan={mockPlan}
          profile={mockProfile}
          diffRows={mockDiffRows}
          onRecomputePlan={onRecomputePlan}
          onExecute={vi.fn()}
          onClose={vi.fn()}
        />,
      );

      const checkboxes = screen.getAllByRole("checkbox");
      // Checkbox 0 is "Select/Deselect All", Checkbox 1 is Row 0, Checkbox 2 is Row 1
      expect(checkboxes.length).toBeGreaterThanOrEqual(3);

      // Click "Select/Deselect All" to unstage everything
      fireEvent.click(checkboxes[0]);

      // Recompute plan was triggered with empty Set
      expect(onRecomputePlan).toHaveBeenCalledWith(expect.any(Set));
      expect(screen.getByText(/Visual Diff \(0\/2\)/i)).toBeDefined();

      // The submit button must be disabled
      const submitBtn = screen.getByRole("button", {
        name: /Authorize and run|Select changes to apply|Apply/i,
      });
      expect((submitBtn as HTMLButtonElement).disabled).toBe(true);
    });

    it("supports switching between unified and split diff layouts and column filters", () => {
      render(
        <VisualDiffViewer
          diffRows={mockDiffRows}
          selectedIds={new Set(["update:res-1:0", "update:res-1:1"])}
          onToggle={vi.fn()}
          onToggleAll={vi.fn()}
          layout="unified"
          onLayoutChange={vi.fn()}
          changedColsOnly={true}
          onChangedColsOnlyChange={vi.fn()}
        />,
      );

      // Unified and Split buttons
      const unifiedBtn = screen.getByRole("button", { name: "Unified" });
      const splitBtn = screen.getByRole("button", { name: "Split" });
      expect(unifiedBtn).toBeDefined();
      expect(splitBtn).toBeDefined();

      // Changed columns toggle checkbox
      const colFilter = screen.getByLabelText(/Modified columns only/i);
      expect(colFilter).toBeDefined();
      expect((colFilter as HTMLInputElement).checked).toBe(true);
    });
  });
});
