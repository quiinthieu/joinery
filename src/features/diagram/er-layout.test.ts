import { describe, expect, test } from "vitest";
import type { DiagramForeignKey, SchemaDiagram } from "../../ipc/generated";
import {
  HEADER_HEIGHT,
  MAX_VISIBLE_COLUMNS,
  ROW_HEIGHT,
  columnAnchor,
  filterDiagram,
  layoutDiagram,
  tableKey,
} from "./er-layout";

const column = (name: string, primaryKey = false) => ({
  name,
  databaseType: "bigint",
  nullable: !primaryKey,
  primaryKey,
});
const ref = (name: string, schema = "public") => ({ schema, name });
const fk = (
  name: string,
  source: string,
  target: string,
  columns: string[],
  targetColumns: string[],
  targetSchema = "public",
): DiagramForeignKey => ({
  name,
  source: ref(source),
  columns,
  target: ref(target, targetSchema),
  targetColumns,
});

function sample(): SchemaDiagram {
  return {
    schema: "public",
    tables: [
      { table: ref("line_notes"), kind: "r", columns: [column("note_order"), column("note_line")] },
      {
        table: ref("order_lines"),
        kind: "r",
        columns: [column("order_id", true), column("line_no", true)],
      },
      { table: ref("orders"), kind: "r", columns: [column("id", true), column("customer_id")] },
      { table: ref("customers"), kind: "r", columns: [column("id", true)] },
      { table: ref("lonely"), kind: "v", columns: [column("x")] },
    ],
    foreignKeys: [
      fk(
        "line_notes_line_fk",
        "line_notes",
        "order_lines",
        ["note_order", "note_line"],
        ["order_id", "line_no"],
      ),
      fk("line_notes_order_fk", "line_notes", "orders", ["note_order"], ["id"]),
      fk("order_lines_order_fk", "order_lines", "orders", ["order_id"], ["id"]),
      fk("orders_customer_fk", "orders", "customers", ["customer_id"], ["id"]),
    ],
    truncated: false,
    restricted: false,
  };
}

describe("schema diagram layout", () => {
  test("places referenced tables left of referencing tables, deterministically", () => {
    const layout = layoutDiagram(sample());
    const layerOf = (name: string) => layout.nodes.find((node) => node.table.name === name)!.layer;
    expect(layerOf("customers")).toBe(0);
    expect(layerOf("lonely")).toBe(0);
    expect(layerOf("orders")).toBe(1);
    expect(layerOf("order_lines")).toBe(2);
    expect(layerOf("line_notes")).toBe(3);
    const x = (name: string) => layout.nodes.find((node) => node.table.name === name)!.x;
    expect(x("customers")).toBeLessThan(x("orders"));
    expect(x("orders")).toBeLessThan(x("line_notes"));
    expect(layout.edges).toHaveLength(4);
    expect(layout.layers.flat()).toHaveLength(5);

    const shuffled = sample();
    shuffled.tables.reverse();
    shuffled.foreignKeys.reverse();
    expect(layoutDiagram(shuffled)).toEqual(layout);
  });

  test("marks foreign-key columns and anchors edges to their rows", () => {
    const layout = layoutDiagram(sample());
    const notes = layout.nodes.find((node) => node.table.name === "line_notes")!;
    expect([...notes.foreignKeyColumns].sort()).toEqual(["note_line", "note_order"]);
    expect(columnAnchor(notes, "note_line")).toBe(
      notes.y + HEADER_HEIGHT + ROW_HEIGHT + ROW_HEIGHT / 2,
    );
    expect(columnAnchor(notes, "missing")).toBe(notes.y + HEADER_HEIGHT / 2);
    const edge = layout.edges.find((e) => e.foreignKey.name === "line_notes_line_fk")!;
    expect(edge.source).toBe(tableKey(ref("line_notes")));
    expect(edge.target).toBe(tableKey(ref("order_lines")));
    expect(edge.path.startsWith(`M ${notes.x} `)).toBe(true);
  });

  test("breaks cycles, loops self-references and adds external targets", () => {
    const diagram: SchemaDiagram = {
      schema: "public",
      tables: [
        { table: ref("a"), kind: "r", columns: [column("id", true), column("b_id")] },
        { table: ref("b"), kind: "r", columns: [column("id", true), column("a_id")] },
        { table: ref("tree"), kind: "r", columns: [column("id", true), column("parent_id")] },
      ],
      foreignKeys: [
        fk("a_b", "a", "b", ["b_id"], ["id"]),
        fk("b_a", "b", "a", ["a_id"], ["id"]),
        fk("tree_parent", "tree", "tree", ["parent_id"], ["id"]),
        fk("tree_audit", "tree", "events", ["id"], ["tree_id"], "audit"),
      ],
      truncated: false,
      restricted: false,
    };
    const layout = layoutDiagram(diagram);
    const a = layout.nodes.find((n) => n.table.name === "a")!;
    const b = layout.nodes.find((n) => n.table.name === "b")!;
    expect(new Set([a.layer, b.layer]).size).toBe(2);
    const self = layout.edges.find((e) => e.foreignKey.name === "tree_parent")!;
    expect(self.selfReference).toBe(true);
    const external = layout.nodes.find((n) => n.table.schema === "audit")!;
    expect(external.external).toBe(true);
    expect(external.columns).toEqual([]);
    const tree = layout.nodes.find((n) => n.table.name === "tree")!;
    expect(tree.layer).toBe(external.layer + 1);
  });

  test("caps visible columns and reports the remainder", () => {
    const wide: SchemaDiagram = {
      schema: "public",
      tables: [
        {
          table: ref("wide"),
          kind: "r",
          columns: Array.from({ length: MAX_VISIBLE_COLUMNS + 5 }, (_, i) => column(`c${i}`)),
        },
      ],
      foreignKeys: [],
      truncated: false,
      restricted: false,
    };
    const [node] = layoutDiagram(wide).nodes;
    expect(node.columns).toHaveLength(MAX_VISIBLE_COLUMNS);
    expect(node.hiddenColumns).toBe(5);
    expect(node.height).toBe(HEADER_HEIGHT + (MAX_VISIBLE_COLUMNS + 1) * ROW_HEIGHT + 6);
  });

  test("filters to matching tables and their direct neighbors", () => {
    const filtered = filterDiagram(sample(), "ORDER_LINES");
    expect(filtered.tables.map((t) => t.table.name).sort()).toEqual([
      "line_notes",
      "order_lines",
      "orders",
    ]);
    expect(filtered.foreignKeys.map((k) => k.name).sort()).toEqual([
      "line_notes_line_fk",
      "line_notes_order_fk",
      "order_lines_order_fk",
    ]);
    expect(filterDiagram(sample(), "  ")).toEqual(sample());
    expect(layoutDiagram(filterDiagram(sample(), "nothing")).nodes).toEqual([]);
  });
});
