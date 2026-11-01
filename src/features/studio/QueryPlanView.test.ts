import { expect, it } from "vitest";
import type { ResultInfo } from "../../ipc/generated";
import type { PlanSource } from "./plan-snapshots";
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
