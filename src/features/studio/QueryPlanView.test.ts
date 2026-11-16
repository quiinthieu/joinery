import { expect, it } from "vitest";
import type { ResultInfo } from "../../ipc/generated";
import type { PlanSource } from "./plan-snapshots";
import { parseQueryPlan } from "./query-plan";
import { isQueryPlanResult } from "./QueryPlanView";

it("offers a Snowflake CONTENT plan only for a recognized execution identity", () => {
  const result = { retainedRows: 1, columns: [{ name: "content" }] } as ResultInfo;
  const source = { engine: "snowflake" } as PlanSource;
  expect(isQueryPlanResult(result)).toBe(false);
  expect(isQueryPlanResult(result, { engine: "mysql" } as PlanSource)).toBe(false);
  expect(isQueryPlanResult(result, source)).toBe(true);
  expect(isQueryPlanResult({ ...result, retainedRows: 2 }, source)).toBe(false);
  expect(
    isQueryPlanResult({ ...result, columns: [...result.columns, ...result.columns] }, source),
  ).toBe(false);
});

it("parses SQLite EXPLAIN QUERY PLAN JSON steps into a nested PlanNode tree", () => {
  const raw = JSON.stringify({
    sqliteQueryPlan: [
      { id: 2, parent: 0, detail: "MATERIALIZE sub" },
      { id: 5, parent: 2, detail: "SCAN customers" },
      { id: 12, parent: 0, detail: "SEARCH orders USING INDEX idx_orders_status (status=?)" },
    ],
  });
  const plan = parseQueryPlan(raw);
  expect(plan.engine).toBe("SQLite");
  expect(plan.actual).toBe(false);
  expect(plan.roots).toHaveLength(2);
  expect(plan.roots[0]?.label).toBe("MATERIALIZE sub");
  expect(plan.roots[0]?.children[0]?.relation).toBe("customers");
  expect(plan.roots[1]?.relation).toBe("orders");
  expect(plan.roots[1]?.metrics).toEqual([{ label: "Index", value: "idx_orders_status" }]);
});
