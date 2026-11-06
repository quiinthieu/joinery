import { describe, expect, it } from "vitest";
import { CompletionIndex } from "./completion-index";

describe("completion prefix index", () => {
  it("caps suggestions and preserves identifier spelling in a large catalog", () => {
    const index = new CompletionIndex();
    index.replace(
      Array.from({ length: 100000 }, (_, i) => ({
        label: `Order_${String(i).padStart(6, "0")}`,
        type: "type",
      })),
    );
    const suggestions = index.prefix("ORDER_000", 20);
    expect(suggestions).toHaveLength(20);
    expect(suggestions[0].label).toBe("Order_000000");
    expect(index.prefix("absent")).toHaveLength(0);
  });

  it("keeps schemas visible ahead of 3-part tables and supports leaf and subword table search", () => {
    const index = new CompletionIndex();
    const analyticsTables = Array.from({ length: 150 }, (_, i) => ({
      label: `DWH.ANALYTICS.TABLE_${String(i).padStart(3, "0")}`,
      path: ["DWH", "ANALYTICS", `TABLE_${String(i).padStart(3, "0")}`],
      type: "type",
    }));
    index.replace([
      { label: "DWH", path: ["DWH"], type: "namespace" },
      { label: "ANALYTICS", path: ["ANALYTICS"], type: "namespace" },
      { label: "PROD", path: ["PROD"], type: "namespace" },
      { label: "DWH.ANALYTICS", path: ["DWH", "ANALYTICS"], type: "namespace" },
      { label: "DWH.PROD", path: ["DWH", "PROD"], type: "namespace" },
      ...analyticsTables,
      { label: "PROD.FACT_PO_TRACKER_7", path: ["PROD", "FACT_PO_TRACKER_7"], type: "type" },
      {
        label: "DWH.PROD.FACT_PO_TRACKER_7",
        path: ["DWH", "PROD", "FACT_PO_TRACKER_7"],
        type: "type",
      },
      { label: "DELETE FROM", type: "keyword" },
      { label: "FROM", type: "keyword" },
    ]);

    // Typing `dwh.` returns 2-part schemas (`DWH.ANALYTICS`, `DWH.PROD`) before 150 3-part tables.
    const dwhDot = index.search(
      [
        { value: "dwh", quoted: false },
        { value: "", quoted: false },
      ],
      "snowflake",
      100,
    );
    expect(dwhDot.slice(0, 2).map((item) => item.label)).toEqual(["DWH.ANALYTICS", "DWH.PROD"]);

    // Typing `dwh.prod.` resolves `DWH.PROD.FACT_PO_TRACKER_7`.
    const dwhProdDot = index.search(
      [
        { value: "dwh", quoted: false },
        { value: "prod", quoted: false },
        { value: "", quoted: false },
      ],
      "snowflake",
      100,
    );
    expect(dwhProdDot.map((item) => item.label)).toEqual(["DWH.PROD.FACT_PO_TRACKER_7"]);

    // Typing a subword after `dwh.prod.` (`dwh.prod.po_tracker`) finds `DWH.PROD.FACT_PO_TRACKER_7`.
    const dwhProdSubword = index.search(
      [
        { value: "dwh", quoted: false },
        { value: "prod", quoted: false },
        { value: "po_tracker", quoted: false },
      ],
      "snowflake",
      100,
    );
    expect(dwhProdSubword.map((item) => item.label)).toEqual(["DWH.PROD.FACT_PO_TRACKER_7"]);

    // Typing unqualified `fact_po` or `po_tracker` finds qualified tables and ranks them before keywords.
    const byLeaf = index.search([{ value: "fact_po", quoted: false }], "snowflake", 100);
    expect(byLeaf.map((item) => item.label)).toEqual([
      "PROD.FACT_PO_TRACKER_7",
      "DWH.PROD.FACT_PO_TRACKER_7",
    ]);
    const bySubword = index.search([{ value: "po_tracker", quoted: false }], "snowflake", 100);
    expect(bySubword.map((item) => item.label)).toEqual([
      "PROD.FACT_PO_TRACKER_7",
      "DWH.PROD.FACT_PO_TRACKER_7",
    ]);

    // Data objects rank ahead of SQL keywords for `f` (`FACT_PO_TRACKER_7` before `FROM`).
    const fMatches = index.search([{ value: "f", quoted: false }], "snowflake", 100);
    expect(fMatches.map((item) => item.label)).toEqual([
      "PROD.FACT_PO_TRACKER_7",
      "DWH.PROD.FACT_PO_TRACKER_7",
      "FROM",
    ]);
  });

  it("prioritizes bare columns over multi-part table qualifier duplicates for 1-part column queries", () => {
    const index = new CompletionIndex();
    index.replace([
      { label: "DELIVERY_DATE", path: ["DELIVERY_DATE"], type: "property" },
      {
        label: "FACT_PO_TRACKER_7.DELIVERY_DATE",
        path: ["FACT_PO_TRACKER_7", "DELIVERY_DATE"],
        type: "property",
      },
      {
        label: "PROD.FACT_PO_TRACKER_7.DELIVERY_DATE",
        path: ["PROD", "FACT_PO_TRACKER_7", "DELIVERY_DATE"],
        type: "property",
      },
      {
        label: "DWH.PROD.FACT_PO_TRACKER_7.DELIVERY_DATE",
        path: ["DWH", "PROD", "FACT_PO_TRACKER_7", "DELIVERY_DATE"],
        type: "property",
      },
      { label: "PO_NUMBER", path: ["PO_NUMBER"], type: "property" },
      {
        label: "DWH.PROD.FACT_PO_TRACKER_7.PO_NUMBER",
        path: ["DWH", "PROD", "FACT_PO_TRACKER_7", "PO_NUMBER"],
        type: "property",
      },
    ]);

    // Typing `d` suggests bare `DELIVERY_DATE` and `_DATE` subword without flooding `DWH.PROD...`
    const dCols = index.search([{ value: "d", quoted: false }], "snowflake", 100);
    expect(dCols.map((item) => item.label)).toEqual(["DELIVERY_DATE"]);

    // Subword `number` matches `PO_NUMBER`
    const subCols = index.search([{ value: "number", quoted: false }], "snowflake", 100);
    expect(subCols.map((item) => item.label)).toEqual(["PO_NUMBER"]);
  });
});
