import { expect, test } from "vitest";
import { EditorState } from "@codemirror/state";
import { sql, PostgreSQL } from "@codemirror/lang-sql";
import { referencedTables, selectedStatement } from "./sql-context";

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
