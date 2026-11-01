import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import type { SqlColumnInspection } from "./sql-inspections";
import { SqlColumnInspectionDialog } from "./SqlColumnInspectionDialog";

afterEach(cleanup);
const state = EditorState.create({ doc: "SELECT o.typo FROM orders o" });
const issue = { name: "typo", from: 9, to: 13, line: 1 };
const result: SqlColumnInspection = {
  table: { schema: "public", name: "orders" },
  checked: 1,
  issues: [issue],
  columns: ["id"],
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
