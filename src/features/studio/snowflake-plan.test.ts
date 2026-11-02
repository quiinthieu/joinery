import { describe, expect, it } from "vitest";
import { parseQueryPlan, type PlanNode } from "./query-plan";

const raw = (operations: unknown[][], stats: unknown = {}) =>
  JSON.stringify({ GlobalStats: stats, Operations: operations });
const flatten = (nodes: PlanNode[]): PlanNode[] =>
  nodes.flatMap((n) => [n, ...flatten(n.children)]);

describe("Snowflake logical plan display", () => {
  it("preserves steps, unordered edges and shared operators without double-counting metrics", () => {
    const plan = parseQueryPlan(
      raw(
        [
          [
            {
              id: 4,
              parentOperators: [2, 3],
              operation: "TableScan",
              objects: ["LAB.PUBLIC.EVENTS"],
              partitionsTotal: 5,
              partitionsAssigned: 0,
              bytesAssigned: 0,
            },
            { id: 0, operation: "Result", expressions: ["COUNT(*)"] },
            { id: 2, parentOperators: [0], operation: "Aggregate" },
            { id: 3, parentOperators: [0], operation: "Filter", expressions: ["ID > 10"] },
          ],
          [{ id: 0, operation: "Result" }],
        ],
        { partitionsTotal: 5, partitionsAssigned: 0, bytesAssigned: 0 },
      ),
    );
    expect(plan.engine).toBe("Snowflake");
    expect(plan.actual).toBe(false);
    expect(plan.timings).toEqual([]);
    expect(plan.roots.map((n) => n.label)).toEqual(["Step 1", "Step 2"]);
    const nodes = flatten(plan.roots);
    const scan = nodes.find((n) => n.label === "TableScan · operator 4")!;
    expect(scan.relation).toBe("LAB.PUBLIC.EVENTS");
    expect(scan.metrics).toContainEqual({
      label: "Assigned partitions (upper-bound estimate)",
      value: "0",
    });
    expect(nodes.filter((n) => n.label.includes("Shared operator 4"))).toHaveLength(1);
    expect(nodes.find((n) => n.label.includes("Shared operator 4"))?.metrics).toEqual([]);
    expect(nodes.filter((n) => n.label === "Result · operator 0")).toHaveLength(2);
  });

  it("keeps unknown fields raw and refuses to present rounded or malformed numeric estimates", () => {
    const input =
      '{"GlobalStats":{},"Operations":[[{"id":0,"operation":"Scan","bytesAssigned":9007199254740993123,"future":"retain"}]]}';
    const plan = parseQueryPlan(input);
    expect(flatten(plan.roots).find((n) => n.label.startsWith("Scan"))?.metrics).toContainEqual({
      label: "Assigned bytes (upper-bound estimate)",
      value: "See raw JSON (outside exact integer range)",
    });
    const node = (value: unknown) => raw([[{ id: 0, operation: "Scan", bytesAssigned: value }]]);
    for (const value of [-1, 1.5, "123", null]) expect(() => parseQueryPlan(node(value))).toThrow();
  });

  it("rejects ambiguous, cyclic, orphaned or malformed graphs and retains raw fallback", () => {
    for (const operations of [
      [],
      [[]],
      [
        [
          { id: 0, operation: "A" },
          { id: 0, operation: "B" },
        ],
      ],
      [[{ id: 0, operation: "A", parentOperators: [9] }]],
      [[{ id: 0, operation: "A", parentOperators: [0] }]],
      [
        [
          { id: 0, operation: "A", parentOperators: [1] },
          { id: 1, operation: "B", parentOperators: [0] },
        ],
      ],
      [
        [
          { id: 0, operation: "A" },
          { id: 1, operation: "B", parentOperators: [0, 0] },
        ],
      ],
      [[{ id: 0, operation: "A", expressions: [3] }]],
      [[{ id: 0, operation: "A", objects: "table" }]],
      [[{ id: 0, operation: "A", parentOperators: "0" }]],
      [[{ id: 9007199254740992, operation: "A" }]],
    ])
      expect(() => parseQueryPlan(raw(operations))).toThrow();
  });

  it("bounds input size, depth, nodes and shared-edge expansion", () => {
    expect(() => parseQueryPlan(" ".repeat(512 * 1024 + 1))).toThrow("512 KiB");
    const chain = Array.from({ length: 42 }, (_, id) => ({
      id,
      operation: "A",
      ...(id ? { parentOperators: [id - 1] } : {}),
    }));
    expect(() => parseQueryPlan(raw([chain]))).toThrow("complexity");
    expect(() =>
      parseQueryPlan(raw([Array.from({ length: 2001 }, (_, id) => ({ id, operation: "A" }))])),
    ).toThrow("complexity");
    const parents = Array.from({ length: 50 }, (_, id) => ({ id, operation: "A" }));
    const shared = Array.from({ length: 50 }, (_, i) => ({
      id: i + 50,
      operation: "B",
      parentOperators: parents.map((p) => p.id),
    }));
    expect(() => parseQueryPlan(raw([[...parents, ...shared]]))).toThrow("complexity");
  });
});
