import { expect, test } from "vitest";
import { EditorState } from "@codemirror/state";
import { sql, PostgreSQL } from "@codemirror/lang-sql";
import {
  catalogCompletions,
  referencedTables,
  selectedStatement,
  tableLookup,
} from "./sql-context";

test("runs only the statement under the cursor and respects SQL quoted semicolons", () => {
  const doc = "-- context\nSELECT $$héllo; there$$;\nSELECT 'it''s; safe';\nSELECT 3;";
  const state = (position: number, end?: number) =>
    EditorState.create({
      doc,
      selection: { anchor: position, head: end ?? position },
      extensions: [sql({ dialect: PostgreSQL })],
    });
  expect(selectedStatement(state(doc.indexOf("héllo")))).toBe("SELECT $$héllo; there$$;");
  expect(selectedStatement(state(doc.indexOf("safe")))).toBe("SELECT 'it''s; safe';");
  expect(selectedStatement(state(doc.length))).toBe("SELECT 3;");
  expect(selectedStatement(state(0))).toBe("SELECT $$héllo; there$$;");
  expect(selectedStatement(state(doc.indexOf("SELECT 3"), doc.length))).toBe("SELECT 3;");
});

test("schema lookups remain stable while typing expressions and resolve table aliases", () => {
  const table = { schema: "public", name: "orders" };
  const lookup = new Map([
    ["orders", table],
    ["public.orders", table],
  ]);
  expect(referencedTables('SELECT o.st FROM "public"."orders" AS o WHERE id=1', lookup)).toEqual([
    { table, aliases: ["o"] },
  ]);
  expect(referencedTables("SELECT status FROM orders WHERE id=1", lookup)).toEqual(
    referencedTables("SELECT status, notes FROM orders WHERE id=12", lookup),
  );
});

test("resolves MySQL backtick identifiers and escaped aliases for lazy completion", () => {
  const table = { schema: "joinery", name: "odd`table" };
  const lookup = new Map([["joinery.odd`table", table]]);
  expect(
    referencedTables("SELECT `o``x`.note FROM `joinery`.`odd``table` AS `o``x`", lookup),
  ).toEqual([{ table, aliases: ["o`x"] }]);
});

test("suggests schemas and qualified Snowflake tables without duplicating schemas", () => {
  const tables = [
    {
      table: { schema: "PROD", name: "FACT_PO_TRACKER_7" },
      kind: "BASE TABLE",
      estimatedRows: null,
      totalBytes: null,
    },
    {
      table: { schema: "PROD", name: "ORDERS" },
      kind: "BASE TABLE",
      estimatedRows: null,
      totalBytes: null,
    },
  ];
  const items = catalogCompletions(tables, "DWH");
  expect(items.filter((item) => item.label === "PROD")).toEqual([
    { label: "PROD", type: "namespace", detail: "schema", apply: "PROD." },
  ]);
  expect(items).toContainEqual({
    label: "DWH.PROD.FACT_PO_TRACKER_7",
    type: "type",
    detail: "table",
  });
  expect(items).toContainEqual({
    label: "DWH.PROD",
    type: "namespace",
    detail: "schema",
    apply: "DWH.PROD.",
  });
  const lookup = tableLookup(tables, "DWH");
  expect(referencedTables("SELECT f.PO FROM DWH.PROD.FACT_PO_TRACKER_7 AS f", lookup)).toEqual([
    { table: tables[0].table, aliases: ["f"] },
  ]);
  const duplicate = {
    table: { schema: "STAGING", name: "ORDERS" },
    kind: "BASE TABLE",
    estimatedRows: null,
    totalBytes: null,
  };
  expect(tableLookup([duplicate, ...tables], "DWH", "PROD").get("orders")).toEqual(tables[1].table);
});
