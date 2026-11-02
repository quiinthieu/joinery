import type { PlanNode, QueryPlan } from "./query-plan";

// MySQL 8.4 TREE uses decimal or engineering tokens. Keep the server's text;
// never multiply averages by loops or infer totals/zeroes for skipped iterators.
const numeric = String.raw`(?:\d+(?:\.\d+)?(?:e[+-]?\d+)?)`;
const costs = new RegExp(
  String.raw`  \(cost=(${numeric})(?:\.\.(${numeric}))? rows=(${numeric})\)$`,
  "i",
);
const actuals = new RegExp(
  String.raw` \(actual time=(${numeric})\.\.(${numeric}) rows=(${numeric}) loops=(${numeric})\)$`,
  "i",
);
const unsupported = () =>
  new Error("Unrecognized MySQL TREE format. The original raw plan is still available.");

/** Called after the shared UTF-8 byte bound. Unknown iterator descriptions stay
 * intact. Unsupported continuations/indentation fail to raw rather than guessing. */
export function parseMysqlTreePlan(raw: string): QueryPlan {
  const plan: QueryPlan = {
    engine: "MySQL/MariaDB",
    format: "tree",
    actual: false,
    roots: [],
    timings: [],
  };
  const stack: PlanNode[] = [];
  let visited = 0,
    measured = false;
  for (const line of raw.split("\n")) {
    if (!line.trim()) continue;
    if (++visited > 2000)
      throw new Error("Plan exceeds the tree-view complexity limit. Use the raw view.");
    const match = /^( *)-> (.+?)\r?$/.exec(line);
    if (!match || match[1].length % 4 !== 0) throw unsupported();
    const depth = match[1].length / 4;
    if (depth > 40)
      throw new Error("Plan exceeds the tree-view complexity limit. Use the raw view.");
    if (depth > stack.length || (depth === 0 && plan.roots.length)) throw unsupported();
    let label = match[2];
    const metrics: PlanNode["metrics"] = [];
    const values = (tokens: string[]) => {
      if (tokens.some((token) => !Number.isFinite(Number(token)))) throw unsupported();
    };
    const actual = actuals.exec(label);
    if (actual) {
      values(actual.slice(1));
      metrics.push(
        { label: "Actual first row per loop", value: `${actual[1]} ms` },
        { label: "Actual total per loop including children", value: `${actual[2]} ms` },
        { label: "Actual rows per loop", value: actual[3] },
        { label: "Actual loops", value: actual[4] },
      );
      label = label.slice(0, actual.index);
      plan.actual = measured = true;
    } else if (label.endsWith(" (never executed)")) {
      metrics.push({ label: "Execution", value: "Never executed" });
      label = label.slice(0, -" (never executed)".length);
      plan.actual = measured = true;
    } else if (/ \(actual time=/.test(label)) throw unsupported();
    const cost = costs.exec(label);
    if (cost) {
      values(cost.slice(1).filter((value) => value !== undefined));
      metrics.unshift(
        ...(cost[2] ? [{ label: "Startup cost (optimizer units)", value: cost[1] }] : []),
        { label: "Total cost (optimizer units)", value: cost[2] ?? cost[1] },
        { label: "Estimated rows", value: cost[3] },
      );
      label = label.slice(0, cost.index);
      measured = true;
    }
    label = label.trimEnd();
    if (!label) throw unsupported();
    const node: PlanNode = { path: `$line[${visited}]`, label, metrics, children: [] };
    if (depth === 0) plan.roots.push(node);
    else stack[depth - 1].children.push(node);
    stack.length = depth;
    stack.push(node);
  }
  if (!plan.roots.length || !measured) throw unsupported();
  return plan;
}
