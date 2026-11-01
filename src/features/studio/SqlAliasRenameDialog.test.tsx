import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { sql } from "@codemirror/lang-sql";
import { sqlDialect } from "./sql-identifiers";
import { analyzeAliasRename } from "./sql-navigation";
import { SqlAliasRenameDialog } from "./SqlAliasRenameDialog";

afterEach(cleanup);

test("previews exact edits, rejects collisions and requires explicit Apply", () => {
  const state = EditorState.create({
    doc: "SELECT o.id FROM orders o JOIN customers c ON o.id = c.id",
    extensions: [sql({ dialect: sqlDialect("postgres") })],
  });
  const target = analyzeAliasRename(state, 7, "postgres");
  const onApply = vi.fn(),
    onClose = vi.fn();
  render(
    <SqlAliasRenameDialog target={target} state={state} onApply={onApply} onClose={onClose} />,
  );
  const apply = screen.getByRole("button", { name: "Apply rename" }) as HTMLButtonElement;
  expect(apply.disabled).toBe(true);
  fireEvent.change(screen.getByLabelText("New alias"), { target: { value: "c" } });
  expect(screen.getByRole("alert").textContent).toMatch(/conflict/);
  expect(apply.disabled).toBe(true);
  fireEvent.change(screen.getByLabelText("New alias"), { target: { value: "order_rows" } });
  expect(apply.disabled).toBe(false);
  expect(screen.getAllByText("o → order_rows")).toHaveLength(3);
  expect(onApply).not.toHaveBeenCalled();
  expect(state.doc.toString()).toContain("orders o");
  fireEvent.click(apply);
  expect(onApply).toHaveBeenCalledWith("order_rows");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onClose).toHaveBeenCalledOnce();
});

test("Escape closes without applying and focus stays in the dialog", () => {
  const state = EditorState.create({
    doc: "SELECT o.id FROM orders o",
    extensions: [sql({ dialect: sqlDialect("postgres") })],
  });
  const onApply = vi.fn(),
    onClose = vi.fn();
  render(
    <SqlAliasRenameDialog
      target={analyzeAliasRename(state, 7, "postgres")}
      state={state}
      onApply={onApply}
      onClose={onClose}
    />,
  );
  expect(document.activeElement).toBe(screen.getByLabelText("New alias"));
  const globalKey = vi.fn();
  window.addEventListener("keydown", globalKey);
  fireEvent.keyDown(screen.getByLabelText("New alias"), {
    key: "Enter",
    ctrlKey: true,
    shiftKey: true,
  });
  window.removeEventListener("keydown", globalKey);
  expect(globalKey).not.toHaveBeenCalled();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(onClose).toHaveBeenCalledOnce();
  expect(onApply).not.toHaveBeenCalled();
});
