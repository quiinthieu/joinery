import { describe, expect, it } from "vitest";
import { parseQueryPlan } from "./query-plan";

describe("query plan display adapters", () => {
  it("keeps PostgreSQL estimates, actual per-loop measurements and server timings distinct", () => {
    const plan = parseQueryPlan(
      JSON.stringify([
        {
          Plan: {
            "Node Type": "Nested Loop",
            "Total Cost": 42,
            "Plan Rows": 50,
            "Actual Rows": 2,
            "Actual Loops": 3,
            "Actual Total Time": 1.5,
            Plans: [
              {
                "Node Type": "Index Scan",
                Schema: "public",
                "Relation Name": "orders",
                "Plan Rows": 0,
              },
            ],
          },
          "Planning Time": 0.2,
          "Execution Time": 4.7,
        },
      ]),
    );
    expect(plan.actual).toBe(true);
    expect(plan.roots[0].metrics).toContainEqual({
      label: "Total cost (optimizer units)",
      value: "42",
    });
    expect(plan.roots[0].metrics).toContainEqual({ label: "Actual rows per loop", value: "2" });
    expect(plan.roots[0].children[0].relation).toBe("public.orders");
    expect(plan.roots[0].children[0].metrics).toEqual([{ label: "Estimated rows", value: "0" }]);
    expect(plan.timings).toEqual([
      { label: "Server planning", value: "0.2 ms" },
      { label: "Server execution", value: "4.7 ms" },
    ]);
  });

  it("adapts MySQL nested loops and string costs without inventing actuals", () => {
    const plan = parseQueryPlan(
      JSON.stringify({
        query_block: {
          cost_info: { query_cost: "2.75" },
          nested_loop: [
            {
              table: {
                table_name: "sample",
                access_type: "ref",
                key: "PRIMARY",
                rows_examined_per_scan: 12,
                filtered: "50.00",
              },
            },
          ],
        },
      }),
    );
    expect(plan.engine).toBe("MySQL/MariaDB");
    expect(plan.actual).toBe(false);
    expect(plan.roots[0].metrics).toEqual([
      { label: "Query cost (optimizer units)", value: "2.75" },
    ]);
    const table = plan.roots[0].children[0].children[0];
    expect(table.relation).toBe("sample");
    expect(table.metrics).toContainEqual({
      label: "Estimated rows examined per scan",
      value: "12",
    });
    expect(table.metrics).toContainEqual({ label: "Estimated filtered", value: "50.00%" });
  });

  it("handles MariaDB's different row fields and does not treat unknown metrics as times", () => {
    const plan = parseQueryPlan(
      JSON.stringify({
        query_block: {
          table: {
            table_name: "sample",
            rows: 20,
            r_rows: 3,
            r_loops: 2,
            r_total_time_ms: 1.25,
            future_metric: 99,
            cost_info: { query_cost: "not numeric" },
          },
        },
      }),
    );
    expect(plan.actual).toBe(true);
    expect(plan.roots[0].children[0].metrics).toEqual([
      { label: "Estimated rows", value: "20" },
      { label: "Actual rows per loop", value: "3" },
      { label: "Actual loops", value: "2" },
      { label: "Actual total", value: "1.25 ms" },
    ]);
  });

  it("fails closed on malformed or unsupported shapes and bounded resource violations", () => {
    for (const raw of [
      "not json",
      "{}",
      '[{"Plan":{"Node Type":"Scan","Plans":[null]}}]',
      '{"query_block":null}',
    ])
      expect(() => parseQueryPlan(raw)).toThrow();
    expect(() => parseQueryPlan(" ".repeat(512 * 1024 + 1))).toThrow("512 KiB");
    let deep: object = { "Node Type": "Scan" };
    for (let i = 0; i < 42; i++) deep = { "Node Type": "Loop", Plans: [deep] };
    expect(() => parseQueryPlan(JSON.stringify([{ Plan: deep }]))).toThrow("complexity");
    expect(() =>
      parseQueryPlan(
        JSON.stringify({
          query_block: {
            nested_loop: Array.from({ length: 2001 }, () => ({ table: { table_name: "t" } })),
          },
        }),
      ),
    ).toThrow("complexity");
  });
});
