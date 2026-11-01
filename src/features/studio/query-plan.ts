import { parseMysqlTreePlan } from "./mysql-tree-plan";

export interface PlanNode {
  path: string;
  label: string;
  relation?: string;
  metrics: { label: string; value: string }[];
  children: PlanNode[];
}
export interface QueryPlan {
  engine: "PostgreSQL" | "MySQL/MariaDB";
  format: "json" | "tree";
  actual: boolean;
  roots: PlanNode[];
  timings: { label: string; value: string }[];
}
type RecordValue = Record<string, unknown>;
const record = (value: unknown): value is RecordValue =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const number = (value: unknown) =>
  (typeof value === "number" && Number.isFinite(value)) ||
  (typeof value === "string" && /^-?\d+(\.\d+)?$/.test(value));

/** Bounded display adapter. Original JSON or TREE text stays available unchanged. */
export function parseQueryPlan(raw: string): QueryPlan {
  if (new TextEncoder().encode(raw).length > 512 * 1024)
    throw new Error("Plan exceeds the 512 KiB tree-view limit. Use the data grid or export.");
  if (raw.trimStart().startsWith("->")) return parseMysqlTreePlan(raw);
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    throw new Error("This result is not a supported query plan. Use the data grid or raw view.");
  }
  let visited = 0;
  const check = (depth: number) => {
    if (++visited > 2000 || depth > 40)
      throw new Error("Plan exceeds the tree-view complexity limit. Use the raw view.");
  };
  const result: QueryPlan = {
    engine: "PostgreSQL",
    format: "json",
    actual: false,
    roots: [],
    timings: [],
  };
  const metric = (source: RecordValue, key: string, label: string, suffix = "") =>
    number(source[key]) ? [{ label, value: `${source[key]}${suffix}` }] : [];
  if (Array.isArray(data) && data.length === 1 && record(data[0]) && record(data[0].Plan)) {
    const pg = (source: RecordValue, path: string, depth: number): PlanNode => {
      check(depth);
      if (typeof source["Node Type"] !== "string")
        throw new Error("Unrecognized PostgreSQL plan node");
      if (number(source["Actual Rows"])) result.actual = true;
      const relation = [source.Schema, source["Relation Name"]]
        .filter((part) => typeof part === "string")
        .join(".");
      const plans = source.Plans;
      if (plans !== undefined && (!Array.isArray(plans) || plans.some((child) => !record(child))))
        throw new Error("Unrecognized PostgreSQL child plans");
      return {
        path,
        label: source["Node Type"],
        relation: relation || undefined,
        metrics: [
          ...metric(source, "Plan Rows", "Estimated rows"),
          ...metric(source, "Startup Cost", "Startup cost (optimizer units)"),
          ...metric(source, "Total Cost", "Total cost (optimizer units)"),
          ...metric(source, "Actual Rows", "Actual rows per loop"),
          ...metric(source, "Actual Loops", "Actual loops"),
          ...metric(source, "Actual Startup Time", "Actual startup per loop", " ms"),
          ...metric(source, "Actual Total Time", "Actual total per loop", " ms"),
        ],
        children: ((plans ?? []) as RecordValue[]).map((child, index) =>
          pg(child, `${path}.Plans[${index}]`, depth + 1),
        ),
      };
    };
    result.roots = [pg(data[0].Plan, "$[0].Plan", 0)];
    result.timings = [
      ...metric(data[0], "Planning Time", "Server planning", " ms"),
      ...metric(data[0], "Execution Time", "Server execution", " ms"),
    ];
    return result;
  }
  if (record(data) && record(data.query_block)) {
    result.engine = "MySQL/MariaDB";
    const mysql = (value: unknown, path: string, label: string, depth: number): PlanNode[] => {
      check(depth);
      if (Array.isArray(value))
        return value.flatMap((child, index) => mysql(child, `${path}[${index}]`, label, depth + 1));
      if (!record(value)) return [];
      if (number(value.r_rows) || number(value.r_loops)) result.actual = true;
      const costs = record(value.cost_info) ? value.cost_info : {};
      const node: PlanNode = {
        path,
        label: label.replaceAll("_", " "),
        relation: typeof value.table_name === "string" ? value.table_name : undefined,
        metrics: [
          ...(typeof value.access_type === "string"
            ? [{ label: "Access", value: value.access_type }]
            : []),
          ...(typeof value.key === "string" ? [{ label: "Index", value: value.key }] : []),
          ...metric(value, "rows_examined_per_scan", "Estimated rows examined per scan"),
          ...metric(value, "rows_produced_per_join", "Estimated rows produced per join"),
          ...metric(value, "rows", "Estimated rows"),
          ...metric(value, "filtered", "Estimated filtered", "%"),
          ...metric(costs, "query_cost", "Query cost (optimizer units)"),
          ...metric(costs, "prefix_cost", "Prefix cost (optimizer units)"),
          ...metric(value, "r_rows", "Actual rows per loop"),
          ...metric(value, "r_loops", "Actual loops"),
          ...metric(value, "r_filtered", "Actual filtered", "%"),
          ...metric(value, "r_total_time_ms", "Actual total including children", " ms"),
        ],
        children: Object.entries(value)
          .filter(([key, child]) => key !== "cost_info" && (record(child) || Array.isArray(child)))
          .flatMap(([key, child]) => mysql(child, `${path}.${key}`, key, depth + 1)),
      };
      return [node];
    };
    result.roots = mysql(data.query_block, "$.query_block", "query_block", 0);
    if (record(data.query_optimization)) {
      result.timings = metric(
        data.query_optimization,
        "r_total_time_ms",
        "Server optimization",
        " ms",
      );
      if (result.timings.length) result.actual = true;
    }
    return result;
  }
  throw new Error("Unrecognized plan format. The raw result is still available.");
}
