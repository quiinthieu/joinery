import { expect, test } from "vitest";
import type { ColumnInfo, ForeignKey } from "../../ipc/generated";
import {
  EMPTY_QUERY,
  allowedOperators,
  cycleSort,
  describeFilter,
  referencedRowsQuery,
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
  });
  row.note_line = null;
  expect(referencedRowsQuery(key, (column) => row[column])).toBeNull();
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
    }),
  ).toBe("order_lines · order_id = 7, line_no = 2…");
});
