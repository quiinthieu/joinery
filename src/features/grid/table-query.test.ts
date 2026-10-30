import { expect, test } from "vitest";
import type { ColumnInfo, ForeignKey, InboundForeignKey } from "../../ipc/generated";
import {
  EMPTY_QUERY,
  allowedOperators,
  cycleSort,
  describeFilter,
  hasRawClauses,
  inboundConstraintsForCell,
  isEmptyQuery,
  normalizeQuery,
  referencedRowsQuery,
  referencingRowsQuery,
  sameQuery,
  sortIndicator,
  tableTabTitle,
} from "./table-query";

test("plain header clicks cycle one key; additive clicks keep the others", () => {
  let query = cycleSort(EMPTY_QUERY, "bucket", false);
  expect(query.sort).toEqual([{ column: "bucket", descending: false }]);
  query = cycleSort(query, "bucket", false);
  expect(query.sort).toEqual([{ column: "bucket", descending: true }]);
  query = cycleSort(query, "label", true);
  expect(query.sort).toEqual([
    { column: "bucket", descending: true },
    { column: "label", descending: false },
  ]);
  expect(sortIndicator(query, "bucket")).toBe("↓1");
  expect(sortIndicator(query, "label")).toBe("↑2");
  query = cycleSort(query, "bucket", true);
  expect(query.sort).toEqual([{ column: "label", descending: false }]);
  expect(sortIndicator(query, "label")).toBe("↑");
  expect(cycleSort(query, "id", false).sort).toEqual([{ column: "id", descending: false }]);
  expect(cycleSort(cycleSort(query, "label", false), "label", false).sort).toEqual([]);
});

test("filters describe themselves with quoted literals", () => {
  expect(describeFilter({ column: "label", op: "contains", value: "it's" })).toBe(
    "label contains 'it''s'",
  );
  expect(describeFilter({ column: "label", op: "isNull", value: null })).toBe("label is NULL");
});

test("binary and JSON columns only offer safe operators", () => {
  const column = (kind: ColumnInfo["kind"]) => ({ kind }) as ColumnInfo;
  expect(allowedOperators(column("binary"))).toEqual(["isNull", "isNotNull"]);
  expect(allowedOperators(column("json"))).toEqual([
    "contains",
    "startsWith",
    "isNull",
    "isNotNull",
  ]);
  expect(allowedOperators(column("integer"))).toContain("gte");
});

test("referenced rows pair composite columns by ordinal and refuse NULL keys", () => {
  const key: ForeignKey = {
    name: "line_notes_line_fk",
    columns: ["note_order", "note_line"],
    target: { schema: "public", name: "order_lines" },
    targetColumns: ["order_id", "line_no"],
  };
  const row: Record<string, string | null> = { note_order: "7", note_line: "2" };
  expect(referencedRowsQuery(key, (column) => row[column])).toEqual({
    sort: [],
    filters: [
      { column: "order_id", op: "eq", value: "7" },
      { column: "line_no", op: "eq", value: "2" },
    ],
    whereClause: null,
    orderClause: null,
  });
  row.note_line = null;
  expect(referencedRowsQuery(key, (column) => row[column])).toBeNull();
});

test("referencing rows map target columns to source filters and narrow by selected column", () => {
  const composite: InboundForeignKey = {
    name: "line_notes_line_fk",
    source: { schema: "public", name: "line_notes" },
    columns: ["note_order", "note_line"],
    targetColumns: ["order_id", "line_no"],
  };
  const slugKey: InboundForeignKey = {
    name: "line_tags_sku_fk",
    source: { schema: "public", name: "line_tags" },
    columns: ["line_sku"],
    targetColumns: ["sku"],
  };
  expect(inboundConstraintsForCell([composite, slugKey], "sku")).toEqual([slugKey]);
  expect(inboundConstraintsForCell([composite, slugKey], "created_at")).toEqual([
    composite,
    slugKey,
  ]);
  const row: Record<string, string | null> = { order_id: "7", line_no: "2", sku: "SKU-7-2" };
  expect(referencingRowsQuery(composite, (column) => row[column])).toEqual({
    sort: [],
    filters: [
      { column: "note_order", op: "eq", value: "7" },
      { column: "note_line", op: "eq", value: "2" },
    ],
    whereClause: null,
    orderClause: null,
  });
  row.line_no = null;
  expect(referencingRowsQuery(composite, (column) => row[column])).toBeNull();
});

test("table titles summarize at most two filters", () => {
  expect(tableTabTitle("orders", EMPTY_QUERY)).toBe("orders");
  expect(
    tableTabTitle("order_lines", {
      sort: [],
      filters: [
        { column: "order_id", op: "eq", value: "7" },
        { column: "line_no", op: "eq", value: "2" },
        { column: "sku", op: "isNotNull", value: null },
      ],
      whereClause: null,
      orderClause: null,
    }),
  ).toBe("order_lines · order_id = 7, line_no = 2…");
});

test("raw WHERE and ORDER BY clauses normalize whitespace and yield to header sort clicks", () => {
  const query = normalizeQuery({
    sort: [],
    filters: [],
    whereClause: "  bucket > 1  ",
    orderClause: "   ",
  });
  expect(query).toEqual({
    sort: [],
    filters: [],
    whereClause: "bucket > 1",
    orderClause: null,
  });
  expect(hasRawClauses(query)).toBe(true);
  expect(isEmptyQuery(query)).toBe(false);
  expect(sameQuery(query, { ...query, whereClause: "bucket > 1\n" })).toBe(true);
  const sorted = cycleSort({ ...query, orderClause: "id DESC" }, "bucket", false);
  expect(sorted.orderClause).toBeNull();
  expect(sorted.sort).toEqual([{ column: "bucket", descending: false }]);
});
