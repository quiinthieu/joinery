import { describe, expect, test } from "vitest";
import type { DatabaseObjectSummary, TableInfo } from "../../ipc/generated";
import {
  buildExplorerSchemas,
  flattenVisibleExplorerNodes,
  objectTabTitle,
  resolveTreeKeyAction,
} from "./explorer-tree";

const SAMPLE_TABLES: TableInfo[] = [
  {
    table: { schema: "public", name: "orders" },
    kind: "r",
    estimatedRows: 500,
    totalBytes: 32768,
  },
  {
    table: { schema: "public", name: "events_part" },
    kind: "p",
    estimatedRows: 1000,
    totalBytes: 65536,
  },
  {
    table: { schema: "public", name: "active_orders" },
    kind: "v",
    estimatedRows: null,
    totalBytes: null,
  },
  {
    table: { schema: "public", name: "daily_rollup" },
    kind: "m",
    estimatedRows: 30,
    totalBytes: 8192,
  },
  {
    table: { schema: "ANALYTICS", name: "CUSTOMER_SUMMARY" },
    kind: "VIEW",
    estimatedRows: 9000,
    totalBytes: 4096,
  },
];

const SAMPLE_OBJECTS: DatabaseObjectSummary[] = [
  {
    object: {
      schema: "public",
      name: "r3_calc",
      kind: "function",
      identity: "integer",
    },
    ownerTable: null,
    signature: "(a integer) -> integer",
    estimatedRows: null,
    totalBytes: null,
  },
  {
    object: {
      schema: "public",
      name: "r3_calc",
      kind: "function",
      identity: "numeric, integer",
    },
    ownerTable: null,
    signature: "(a numeric, b integer) -> numeric",
    estimatedRows: null,
    totalBytes: null,
  },
  {
    object: {
      schema: "public",
      name: "r3_noop_proc",
      kind: "procedure",
      identity: "text",
    },
    ownerTable: null,
    signature: "(IN label text)",
    estimatedRows: null,
    totalBytes: null,
  },
  {
    object: {
      schema: "public",
      name: "order_seq",
      kind: "sequence",
      identity: null,
    },
    ownerTable: null,
    signature: null,
    estimatedRows: null,
    totalBytes: 8192,
  },
  {
    object: {
      schema: "public",
      name: "orders_audit_trg",
      kind: "trigger",
      identity: "orders",
    },
    ownerTable: "orders",
    signature: "BEFORE INSERT",
    estimatedRows: null,
    totalBytes: null,
  },
  {
    object: {
      schema: "public",
      name: "PRIMARY",
      kind: "index",
      identity: "orders",
    },
    ownerTable: "orders",
    signature: "btree",
    estimatedRows: null,
    totalBytes: null,
  },
  {
    object: {
      schema: "public",
      name: "PRIMARY",
      kind: "index",
      identity: "events_part",
    },
    ownerTable: "events_part",
    signature: "btree",
    estimatedRows: null,
    totalBytes: null,
  },
];

describe("explorer-tree", () => {
  test("groups catalog relations and extended objects into ordered categories with exact labels", () => {
    const groups = buildExplorerSchemas(SAMPLE_TABLES, SAMPLE_OBJECTS, "all", "");
    expect(groups.map((g) => g.schema)).toEqual(["public", "ANALYTICS"]);

    const pub = groups[0];
    expect(pub.totalCount).toBe(11);
    expect(pub.categories.map((c) => c.id)).toEqual([
      "tables",
      "views",
      "materializedViews",
      "routines",
      "sequences",
      "triggers",
      "indexes",
    ]);

    // Tables and views keep exact item.name as primaryLabel
    expect(pub.categories[0].items.map((i) => i.primaryLabel)).toEqual(["orders", "events_part"]);
    expect(pub.categories[0].items[1].secondaryLabel).toBe("part");
    expect(pub.categories[1].items.map((i) => i.primaryLabel)).toEqual(["active_orders"]);
    expect(pub.categories[2].items.map((i) => i.primaryLabel)).toEqual(["daily_rollup"]);

    // Overloaded routines include identity argument types
    expect(pub.categories[3].items.map((i) => i.primaryLabel)).toEqual([
      "r3_calc(integer)",
      "r3_calc(numeric, integer)",
      "r3_noop_proc(text)",
    ]);
    expect(objectTabTitle(SAMPLE_OBJECTS[1].object)).toBe("r3_calc(numeric, integer)");

    // Unique triggers keep exact trigger name + secondary ownerTable; duplicate PRIMARY indexes disambiguate
    expect(pub.categories[5].items[0].primaryLabel).toBe("orders_audit_trg");
    expect(pub.categories[5].items[0].secondaryLabel).toBe("orders");
    expect(pub.categories[6].items.map((i) => i.primaryLabel)).toEqual([
      "PRIMARY (orders)",
      "PRIMARY (events_part)",
    ]);
  });

  test("filters by object-kind chip and substring across schema, signature, and owner table", () => {
    const viewsOnly = buildExplorerSchemas(SAMPLE_TABLES, SAMPLE_OBJECTS, "views", "");
    expect(viewsOnly.map((g) => [g.schema, g.categories.map((c) => c.id)])).toEqual([
      ["public", ["views", "materializedViews"]],
      ["ANALYTICS", ["views"]],
    ]);

    const routinesNumeric = buildExplorerSchemas(
      SAMPLE_TABLES,
      SAMPLE_OBJECTS,
      "routines",
      "numeric",
    );
    expect(routinesNumeric).toHaveLength(1);
    expect(routinesNumeric[0].categories[0].items.map((i) => i.primaryLabel)).toEqual([
      "r3_calc(numeric, integer)",
    ]);

    const otherOnOrders = buildExplorerSchemas(SAMPLE_TABLES, SAMPLE_OBJECTS, "other", "orders");
    expect(
      otherOnOrders[0].categories.map((c) => [c.id, c.items.map((i) => i.primaryLabel)]),
    ).toEqual([
      ["triggers", ["orders_audit_trg"]],
      ["indexes", ["PRIMARY (orders)"]],
    ]);
  });

  test("resolves directional keyboard tree navigation across schemas, categories, and items", () => {
    const groups = buildExplorerSchemas(SAMPLE_TABLES, SAMPLE_OBJECTS, "tables", "");
    const collapsedNodes = flattenVisibleExplorerNodes(
      groups,
      () => false,
      () => true,
    );
    expect(collapsedNodes.map((n) => n.id)).toEqual(["schema:public"]);
    expect(resolveTreeKeyAction(collapsedNodes, "schema:public", "ArrowRight")).toEqual({
      type: "expandSchema",
      schema: "public",
    });

    const expandedNodes = flattenVisibleExplorerNodes(
      groups,
      (s) => s === "public",
      () => true,
    );
    expect(expandedNodes.map((n) => n.id)).toEqual([
      "schema:public",
      "cat:public:tables",
      "item:public:table:orders:",
      "item:public:partitionedTable:events_part:",
    ]);

    // ArrowRight on expanded schema steps into first category; ArrowDown walks items; ArrowLeft walks back up & collapses
    expect(resolveTreeKeyAction(expandedNodes, "schema:public", "ArrowRight")).toEqual({
      type: "focus",
      nodeId: "cat:public:tables",
    });
    expect(resolveTreeKeyAction(expandedNodes, "cat:public:tables", "ArrowDown")).toEqual({
      type: "focus",
      nodeId: "item:public:table:orders:",
    });
    expect(resolveTreeKeyAction(expandedNodes, "schema:public", "End")).toEqual({
      type: "focus",
      nodeId: "item:public:partitionedTable:events_part:",
    });
    expect(
      resolveTreeKeyAction(expandedNodes, "item:public:partitionedTable:events_part:", "Home"),
    ).toEqual({
      type: "focus",
      nodeId: "schema:public",
    });
    expect(resolveTreeKeyAction(expandedNodes, "item:public:table:orders:", "ArrowLeft")).toEqual({
      type: "focus",
      nodeId: "cat:public:tables",
    });
    expect(resolveTreeKeyAction(expandedNodes, "cat:public:tables", "ArrowLeft")).toEqual({
      type: "collapseCategory",
      categoryKey: "public:tables",
    });

    const catCollapsedNodes = flattenVisibleExplorerNodes(
      groups,
      (s) => s === "public",
      () => false,
    );
    expect(resolveTreeKeyAction(catCollapsedNodes, "cat:public:tables", "ArrowLeft")).toEqual({
      type: "focus",
      nodeId: "schema:public",
    });
    expect(resolveTreeKeyAction(catCollapsedNodes, "schema:public", "ArrowLeft")).toEqual({
      type: "collapseSchema",
      schema: "public",
    });
  });
});
