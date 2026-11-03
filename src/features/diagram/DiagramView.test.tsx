import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SchemaDiagram, SessionInfo } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";
import { DiagramView, neighborNode } from "./DiagramView";
import { layoutDiagram } from "./er-layout";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (e: unknown) => String(e instanceof Error ? e.message : e),
}));

const session: SessionInfo = {
  id: "session-1",
  connectionId: "profile",
  engine: "postgres",
  serverVersion: "fixture",
  latencyMs: 0,
  encrypted: false,
  capabilities: {
    sql: true,
    transactions: true,
    cancellation: true,
    tableBrowsing: true,
    tableEditing: true,
    tableFiltering: true,
    processes: true,
    objectExplorer: true,
  },
};
const diagram: SchemaDiagram = {
  schema: "public",
  tables: [
    {
      table: { schema: "public", name: "customers" },
      kind: "r",
      columns: [{ name: "id", databaseType: "bigint", nullable: false, primaryKey: true }],
    },
    {
      table: { schema: "public", name: "orders" },
      kind: "r",
      columns: [
        { name: "id", databaseType: "bigint", nullable: false, primaryKey: true },
        { name: "customer_id", databaseType: "bigint", nullable: false, primaryKey: false },
      ],
    },
  ],
  foreignKeys: [
    {
      name: "orders_customer_fk",
      source: { schema: "public", name: "orders" },
      columns: ["customer_id"],
      target: { schema: "public", name: "customers" },
      targetColumns: ["id"],
    },
  ],
  truncated: true,
  restricted: false,
};
const diagramTab = {
  id: "diagram-tab",
  kind: "diagram",
  title: "public diagram",
  connectionId: "profile",
  diagramSchema: "public",
} as unknown as StudioTab;

beforeEach(() => {
  useStudio.setState({ sessions: { profile: session } });
  vi.mocked(client.call).mockReset();
});
afterEach(() => cleanup());

it("loads one schema diagram, discloses truncation and opens tables by keyboard", async () => {
  vi.mocked(client.call).mockResolvedValue(diagram);
  const onOpenTable = vi.fn();
  render(<DiagramView tab={diagramTab} onOpenTable={onOpenTable} />);
  const customers = await screen.findByRole("button", { name: "customers, 1 columns" });
  expect(client.call).toHaveBeenCalledWith("schema_diagram", {
    sessionId: "session-1",
    schema: "public",
  });
  expect(screen.getByText(/exceeds the diagram limits/)).toBeTruthy();
  expect(screen.getByText("2 relations · 1 foreign keys")).toBeTruthy();
  expect(customers.getAttribute("tabindex")).toBe("0");
  const orders = screen.getByRole("button", { name: "orders, 2 columns" });
  expect(orders.getAttribute("tabindex")).toBe("-1");
  act(() => customers.focus());
  fireEvent.keyDown(customers, { key: "ArrowRight" });
  expect(document.activeElement).toBe(orders);
  fireEvent.keyDown(orders, { key: "Enter" });
  expect(onOpenTable).toHaveBeenCalledWith({ schema: "public", name: "orders" });
  expect(
    document.querySelector('[data-diagram-edge="orders_customer_fk"] title')?.textContent,
  ).toBe("orders_customer_fk: orders(customer_id) → public.customers(id)");
});

it("filters tables and reports unsupported engines without guessing", async () => {
  vi.mocked(client.call).mockResolvedValueOnce(diagram);
  render(<DiagramView tab={diagramTab} onOpenTable={vi.fn()} />);
  await screen.findByRole("button", { name: "orders, 2 columns" });
  fireEvent.change(screen.getByLabelText("Filter diagram tables"), {
    target: { value: "zzz" },
  });
  expect(screen.getByText("No matching tables")).toBeTruthy();
  vi.mocked(client.call).mockRejectedValueOnce(
    new Error("Schema diagrams are unavailable for this engine"),
  );
  fireEvent.click(screen.getByRole("button", { name: /Refresh/ }));
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toBe(
      "Schema diagrams are unavailable for this engine",
    ),
  );
  expect(screen.queryByRole("button", { name: "orders, 2 columns" })).toBeNull();
});

it("traverses layers and wraps at the edges", () => {
  const layout = layoutDiagram(diagram);
  const [customers, orders] = layout.layers.map((layer) => layer[0]);
  expect(neighborNode(layout, customers, "ArrowRight")).toBe(orders);
  expect(neighborNode(layout, orders, "ArrowRight")).toBe(orders);
  expect(neighborNode(layout, orders, "ArrowLeft")).toBe(customers);
  expect(neighborNode(layout, orders, "Home")).toBe(customers);
  expect(neighborNode(layout, customers, "End")).toBe(orders);
  expect(neighborNode(layout, customers, "ArrowUp")).toBe(customers);
});
