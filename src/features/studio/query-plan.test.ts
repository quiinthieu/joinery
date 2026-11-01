import { describe, expect, it } from "vitest";
import { parseQueryPlan } from "./query-plan";

describe("query plan display adapters", () => {
  it("reads MySQL TREE iterators with inclusive per-loop times and unexecuted children", () => {
    const raw =
      "-> Nested loop inner join  (cost=0.81 rows=1) (actual time=0.0414..0.0414 rows=0 loops=1)\n" +
      "    -> Filter: (a.id < 3)  (cost=0.46 rows=1) (actual time=0.0405..0.0405 rows=0 loops=1)\n" +
      "        -> Covering index range scan on a using PRIMARY over (id < 3)  (cost=0.46 rows=1) (actual time=0.00921..0.00921 rows=0 loops=1)\n" +
      "    -> Single-row covering index lookup on b using PRIMARY (id=a.id)  (cost=0.35 rows=1) (never executed)\n";
    const plan = parseQueryPlan(raw);
    expect(plan.actual).toBe(true);
    expect(plan.format).toBe("tree");
    expect(plan.timings).toEqual([]);
    expect(plan.roots[0].children).toHaveLength(2);
    expect(plan.roots[0].metrics).toContainEqual({
      label: "Actual total per loop including children",
      value: "0.0414 ms",
    });
    expect(plan.roots[0].children[0].children[0].metrics).toContainEqual({
      label: "Actual rows per loop",
      value: "0",
    });
    expect(plan.roots[0].children[1].metrics).toContainEqual({
      label: "Execution",
      value: "Never executed",
    });
    expect(plan.roots[0].children[1].metrics.some((x) => x.label === "Actual loops")).toBe(false);
  });

  it("preserves MySQL engineering tokens, cost ranges, missing costs and iterator labels", () => {
    const plan = parseQueryPlan(
      "-> Hash  (actual time=0..209e-6 rows=2.5 loops=4)\n    -> Rows fetched before execution  (cost=0..0 rows=1) (actual time=84e-6..1.23e+9 rows=1 loops=1)\n",
    );
    expect(plan.roots[0].label).toBe("Hash");
    expect(plan.roots[0].metrics).toEqual([
      { label: "Actual first row per loop", value: "0 ms" },
      { label: "Actual total per loop including children", value: "209e-6 ms" },
      { label: "Actual rows per loop", value: "2.5" },
      { label: "Actual loops", value: "4" },
    ]);
    expect(plan.roots[0].children[0].metrics).toContainEqual({
      label: "Startup cost (optimizer units)",
      value: "0",
    });
    expect(plan.roots[0].children[0].metrics).toContainEqual({
      label: "Total cost (optimizer units)",
      value: "0",
    });
  });

  it("keeps unsupported MySQL TREE text in raw view and bounds its hierarchy", () => {
    for (const raw of [
      "-> Unknown",
      "    -> Orphan  (cost=1 rows=1)",
      "-> Root  (cost=1 rows=1)\n        -> Skipped parent",
      "-> Root  (cost=1 rows=1)\nunrecognized continuation",
      "-> Root  (actual time=1e999..2 rows=1 loops=1)",
    ])
      expect(() => parseQueryPlan(raw)).toThrow();
    expect(() =>
      parseQueryPlan(
        Array.from({ length: 42 }, (_, i) => "    ".repeat(i) + "-> Hash  (cost=1 rows=1)").join(
          "\n",
        ),
      ),
    ).toThrow("complexity");
    expect(() =>
      parseQueryPlan("-> Root  (cost=1 rows=1)\n" + "    -> Child  (cost=1 rows=1)\n".repeat(2000)),
    ).toThrow("complexity");
  });

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
        query_optimization: { r_total_time_ms: 0.25 },
        query_block: {
          table: {
            table_name: "sample",
            rows: 20,
            r_rows: 3,
            r_loops: 2,
            r_filtered: 50,
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
      { label: "Actual filtered", value: "50%" },
      { label: "Actual total including children", value: "1.25 ms" },
    ]);
    expect(plan.timings).toEqual([{ label: "Server optimization", value: "0.25 ms" }]);
  });

  it("recognizes MariaDB optimization-only actuals without inventing execution metrics", () => {
    const plan = parseQueryPlan(
      JSON.stringify({
        query_optimization: { r_total_time_ms: 0 },
        query_block: { table: { message: "No tables used" } },
      }),
    );
    expect(plan.actual).toBe(true);
    expect(plan.timings).toEqual([{ label: "Server optimization", value: "0 ms" }]);
    expect(plan.roots[0].metrics).toEqual([]);
    expect(plan.roots[0].children[0].metrics).toEqual([]);
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
