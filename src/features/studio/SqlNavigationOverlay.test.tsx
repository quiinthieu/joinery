import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ObjectDetails, TableSchema } from "../../ipc/generated";
import { SqlNavigationOverlay } from "./SqlNavigationOverlay";
import { StructureView } from "./UtilityViews";
import type { ResolvedSqlSymbol } from "./sql-navigation";

const ordersSchema: TableSchema = {
  table: { schema: "public", name: "orders" },
  columns: [
    {
      id: "id",
      name: "id",
      kind: "integer",
      databaseType: "int8",
      nullable: false,
      primaryKey: true,
      foreignKey: false,
      editable: false,
      insertable: true,
      hasDefault: true,
    },
    {
      id: "customer_id",
      name: "customer_id",
      kind: "integer",
      databaseType: "int8",
      nullable: false,
      primaryKey: false,
      foreignKey: true,
      editable: true,
      insertable: true,
      hasDefault: false,
    },
  ],
  primaryKey: ["id"],
  foreignKeys: [
    {
      name: "orders_customer_id_fkey",
      columns: ["customer_id"],
      target: { schema: "public", name: "customers" },
      targetColumns: ["id"],
    },
  ],
  inboundForeignKeys: [],
  editable: true,
};

const sampleDetails: ObjectDetails = {
  object: { schema: "public", name: "orders", kind: "table", identity: null },
  ownerTable: null,
  comment: "Customer purchase orders",
  ddl: { status: "available", ddl: "CREATE TABLE public.orders (...);", truncated: false },
  constraints: [],
  indexes: [],
  triggers: [],
  routine: null,
  sequence: null,
  uses: [],
  usedBy: [
    {
      object: { schema: "public", name: "active_orders", kind: "view", identity: null },
      relation: "view dependency",
    },
  ],
};

describe("SqlNavigationOverlay and StructureView column highlight", () => {
  it("renders quick documentation for columns and switches to local + database usages", () => {
    const columnSymbol: ResolvedSqlSymbol = {
      kind: "column",
      name: "customer_id",
      tokenSpan: { from: 9, to: 20 },
      table: { schema: "public", name: "orders" },
      object: { schema: "public", name: "orders", kind: "table", identity: null },
      column: ordersSchema.columns[1],
      schema: ordersSchema,
      occurrences: [
        {
          from: 9,
          to: 20,
          line: 1,
          column: 10,
          lineText: "SELECT o.customer_id FROM public.orders o",
          isDeclaration: false,
        },
        {
          from: 52,
          to: 63,
          line: 2,
          column: 9,
          lineText: "WHERE o.customer_id = 42;",
          isDeclaration: false,
        },
      ],
    };

    const onJump = vi.fn();
    const onOpenDef = vi.fn();
    const onOpenObj = vi.fn();
    const onSwitchMode = vi.fn();
    const onClose = vi.fn();

    const { rerender } = render(
      <SqlNavigationOverlay
        mode="docs"
        symbol={columnSymbol}
        objectDetails={sampleDetails}
        objectDetailsLoading={false}
        objectDetailsError={null}
        objectExplorerSupported={true}
        onJumpInEditor={onJump}
        onOpenDefinition={onOpenDef}
        onOpenObject={onOpenObj}
        onSwitchMode={onSwitchMode}
        onClose={onClose}
      />,
    );

    expect(screen.getByRole("region", { name: "Quick SQL documentation" })).toBeTruthy();
    expect(screen.getByText("public.orders.customer_id")).toBeTruthy();
    expect(screen.getByText("Customer purchase orders")).toBeTruthy();
    expect(screen.getByText("int8")).toBeTruthy();
    expect(screen.getByText("No (NOT NULL)")).toBeTruthy();
    expect(screen.getByText("FOREIGN KEY")).toBeTruthy();
    expect(screen.getByText("public.customers.id (orders_customer_id_fkey)")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /Definition/i }));
    expect(onOpenDef).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: /Usages \(2\)/i }));
    expect(onSwitchMode).toHaveBeenCalledWith("usages");

    rerender(
      <SqlNavigationOverlay
        mode="usages"
        symbol={columnSymbol}
        objectDetails={sampleDetails}
        objectDetailsLoading={false}
        objectDetailsError={null}
        objectExplorerSupported={true}
        onJumpInEditor={onJump}
        onOpenDefinition={onOpenDef}
        onOpenObject={onOpenObj}
        onSwitchMode={onSwitchMode}
        onClose={onClose}
      />,
    );

    expect(screen.getByRole("region", { name: "SQL symbol usages" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Ln 2, Col 9/i }));
    expect(onJump).toHaveBeenCalledWith(52, 63);

    fireEvent.click(screen.getByRole("button", { name: /public\.active_orders/i }));
    expect(onOpenObj).toHaveBeenCalledWith({
      schema: "public",
      name: "active_orders",
      kind: "view",
      identity: null,
    });

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("marks the navigated column row as highlighted in StructureView", () => {
    const { container } = render(
      <StructureView schema={ordersSchema} highlightedColumn="customer_id" />,
    );
    const highlightedRows = container.querySelectorAll('tr[data-highlighted="true"]');
    expect(highlightedRows).toHaveLength(1);
    expect(highlightedRows[0].textContent).toContain("customer_id");
  });
});
