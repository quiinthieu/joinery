import { describe, expect, it } from "vitest";
import type { ColumnInfo, SavedTableLayout } from "../../ipc/generated";
import {
  isDefaultTableLayout,
  moveOrderedColumn,
  moveVisibleColumn,
  resizeTableColumn,
  resolveTableLayout,
  toggleColumnVisibility,
} from "./table-layout";

const table = { schema: "public", name: "customers" };
const connectionId = "11111111-1111-4111-8111-111111111111";

function column(name: string, index: number, kind: ColumnInfo["kind"] = "text"): ColumnInfo {
  return {
    id: `c${index}`,
    name,
    databaseType: kind,
    kind,
    nullable: true,
    primaryKey: index === 0,
    foreignKey: false,
    editable: index !== 0,
    insertable: true,
    hasDefault: false,
  };
}

const columns = [
  column("id", 0, "integer"),
  column("name", 1),
  column("email", 2),
  column("metadata", 3, "json"),
];

describe("table-layout", () => {
  it("resolves saved order, appends unseen schema columns, and ignores dropped ones", () => {
    const saved: SavedTableLayout = {
      connectionId,
      table,
      columnOrder: ["email", "dropped_col", "id"],
      hiddenColumns: ["metadata", "dropped_col"],
      columnWidths: { email: 340, id: 10 },
    };
    const resolved = resolveTableLayout(columns, saved);
    expect(resolved.orderedColumns.map((c) => c.name)).toEqual(["email", "id", "name", "metadata"]);
    expect(resolved.visibleColumns.map((c) => c.name)).toEqual(["email", "id", "name"]);
    expect(resolved.widths.email).toBe(340);
    expect(resolved.widths.id).toBe(60);
    expect(resolved.customized).toBe(true);
  });

  it("never hides every column, even if a saved layout marks all columns hidden", () => {
    const saved: SavedTableLayout = {
      connectionId,
      table,
      columnOrder: ["name", "id"],
      hiddenColumns: ["id", "name", "email", "metadata"],
      columnWidths: {},
    };
    const resolved = resolveTableLayout(columns, saved);
    expect(resolved.visibleColumns.map((c) => c.name)).toEqual(["name"]);
    expect(resolved.hiddenSet.has("name")).toBe(false);

    // Toggling the sole visible column off is refused.
    const next = toggleColumnVisibility(columns, saved, connectionId, table, "name");
    expect(resolveTableLayout(columns, next).visibleColumns.map((c) => c.name)).toEqual(["name"]);
  });

  it("reorders visible columns while preserving hidden column slots and normalizes back to default", () => {
    let layout = toggleColumnVisibility(columns, null, connectionId, table, "name");
    expect(resolveTableLayout(columns, layout).visibleColumns.map((c) => c.name)).toEqual([
      "id",
      "email",
      "metadata",
    ]);

    // Move "metadata" (visible index 2) before "id" (visible index 0).
    layout = moveVisibleColumn(columns, layout, connectionId, table, 2, 0);
    expect(resolveTableLayout(columns, layout).visibleColumns.map((c) => c.name)).toEqual([
      "metadata",
      "id",
      "email",
    ]);
    expect(resolveTableLayout(columns, layout).orderedColumns.map((c) => c.name)).toEqual([
      "metadata",
      "name",
      "id",
      "email",
    ]);

    // Restore default order, unhide, and reset width to default -> layout normalizes to empty.
    layout = toggleColumnVisibility(columns, layout, connectionId, table, "name");
    layout = moveOrderedColumn(columns, layout, connectionId, table, 0, 3);
    layout = moveOrderedColumn(columns, layout, connectionId, table, 0, 1);
    layout = resizeTableColumn(columns, layout, connectionId, table, "email", 230);
    expect(isDefaultTableLayout(layout)).toBe(true);
  });
});
