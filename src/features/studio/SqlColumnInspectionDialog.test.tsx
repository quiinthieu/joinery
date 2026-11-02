import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import type { SqlColumnInspection } from "./sql-inspections";
import { SqlColumnInspectionDialog } from "./SqlColumnInspectionDialog";

afterEach(cleanup);
const state = EditorState.create({ doc: "SELECT o.typo FROM orders o" });
const table = { schema: "public", name: "orders" };
const issue = { name: "typo", qualifier: "o", table, from: 9, to: 13, line: 1 };
const result: SqlColumnInspection = {
  tables: [table],
  checked: 1,
  issues: [issue],
  columnsFor: () => ["id"],
  preview: (_state, target, column) => ({
    from: target.from,
    to: target.to,
    insert: `"${column}"`,
  }),
};

test("requires an explicit catalog choice and Apply, and isolates keyboard shortcuts", () => {
  const onApply = vi.fn(),
    onClose = vi.fn();
  render(
    <SqlColumnInspectionDialog result={result} state={state} onApply={onApply} onClose={onClose} />,
  );
  const apply = screen.getByRole("button", {
    name: "Apply column correction",
  }) as HTMLButtonElement;
  expect(apply.disabled).toBe(true);
  expect(document.activeElement).toBe(screen.getByLabelText("Missing reference"));
  fireEvent.change(screen.getByLabelText("Replace with catalog column"), {
    target: { value: "id" },
  });
  expect(screen.getByLabelText("Column correction preview").textContent).toContain('typo → "id"');
  expect(apply.disabled).toBe(false);
  const globalKey = vi.fn();
  window.addEventListener("keydown", globalKey);
  fireEvent.keyDown(screen.getByLabelText("Replace with catalog column"), {
    key: "Enter",
    ctrlKey: true,
  });
  window.removeEventListener("keydown", globalKey);
  expect(globalKey).not.toHaveBeenCalled();
  expect(onApply).not.toHaveBeenCalled();
  expect(state.doc.toString()).toBe("SELECT o.typo FROM orders o");
  fireEvent.click(apply);
  expect(onApply).toHaveBeenCalledWith(issue, "id");
  fireEvent.keyDown(document, { key: "Escape" });
  expect(onClose).toHaveBeenCalledOnce();
});

test("describes a CTE miss as a projection gap and offers projected names", () => {
  render(
    <SqlColumnInspectionDialog
      result={{
        ...result,
        issues: [{ ...issue, outputOf: "c", projectedColumns: ["id"] }],
        columnsFor: () => ["id"],
      }}
      state={state}
      onApply={vi.fn()}
      onClose={vi.fn()}
    />,
  );
  expect(screen.getByText(/not projected by c/)).toBeTruthy();
  expect(screen.getByLabelText("Replace with projected column")).toBeTruthy();
  expect(screen.getByRole("option", { name: /projected by c/ })).toBeTruthy();
  expect(screen.queryByText(/complete selectable metadata/)).toBeNull();
});

test("labels a limited clean result without offering a correction", () => {
  render(
    <SqlColumnInspectionDialog
      result={{ ...result, issues: [] }}
      state={state}
      onApply={vi.fn()}
      onClose={vi.fn()}
    />,
  );
  expect(screen.getByText(/does not validate the full SQL/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Apply column correction" })).toBeNull();
});

test("switching JOIN issues resets the choice and offers only the owning table's columns", () => {
  const other = {
    ...issue,
    qualifier: "c",
    table: { schema: "public", name: "customers" },
    from: 17,
    to: 21,
  };
  const onApply = vi.fn();
  render(
    <SqlColumnInspectionDialog
      result={{
        ...result,
        tables: [table, other.table],
        issues: [issue, other],
        columnsFor: (target) => (target === issue ? ["order_only"] : ["customer_only"]),
      }}
      state={state}
      onApply={onApply}
      onClose={vi.fn()}
    />,
  );
  const choice = screen.getByLabelText("Replace with catalog column") as HTMLSelectElement;
  fireEvent.change(choice, { target: { value: "order_only" } });
  fireEvent.change(screen.getByLabelText("Missing reference"), { target: { value: "1" } });
  expect(choice.value).toBe("");
  expect(Array.from(choice.options, (o) => o.value)).toEqual(["", "customer_only"]);
  expect(
    (screen.getByRole("button", { name: "Apply column correction" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  fireEvent.change(choice, { target: { value: "customer_only" } });
  fireEvent.click(screen.getByRole("button", { name: "Apply column correction" }));
  expect(onApply).toHaveBeenCalledWith(other, "customer_only");
});
