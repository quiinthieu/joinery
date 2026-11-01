import type { PlanNode, QueryPlan } from "./query-plan";

type Row = Record<string, unknown>;
const record = (value: unknown): value is Row =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const invalid = (): never => {
  throw new Error("Unrecognized Snowflake logical plan. The raw result is still available.");
};
const complexity = (): never => {
  throw new Error("Plan exceeds the tree-view complexity limit. Use the raw view.");
};
const idValue = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
function strings(value: unknown): string[] {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.some((part) => typeof part !== "string")) return invalid();
  return value;
}
function estimates(source: Row): PlanNode["metrics"] {
  return [
    ["partitionsTotal", "Total partitions"],
    ["partitionsAssigned", "Assigned partitions (upper-bound estimate)"],
    ["bytesAssigned", "Assigned bytes (upper-bound estimate)"],
  ].flatMap(([key, label]) => {
    if (!(key in source)) return [];
    const value = source[key];
    if (
      typeof value !== "number" ||
      !Number.isFinite(value) ||
      !Number.isInteger(value) ||
      value < 0
    )
      return invalid();
    return [
      {
        label,
        value: Number.isSafeInteger(value)
          ? String(value)
          : "See raw JSON (outside exact integer range)",
      },
    ];
  });
}

/** Snowflake emits a DAG per step. Expand each operator once, with explicit
 * metric-free references for every additional parent edge. Never infer timing
 * or execution order from this logical structure. Raw JSON remains untouched. */
export function parseSnowflakePlan(data: Row): QueryPlan {
  if (!Array.isArray(data.Operations) || !data.Operations.length || !record(data.GlobalStats))
    return invalid();
  let budget = data.Operations.length;
  if (budget > 2000) return complexity();
  const roots = data.Operations.map((step, stepIndex): PlanNode => {
    if (!Array.isArray(step) || !step.length) return invalid();
    budget += step.length;
    if (budget > 2000) return complexity();
    const nodes = new Map<number, { source: Row; children: number[]; parents: number[] }>();
    for (const source of step) {
      if (
        !record(source) ||
        !idValue(source.id) ||
        nodes.has(source.id) ||
        typeof source.operation !== "string" ||
        !source.operation.trim()
      )
        return invalid();
      const parents = source.parentOperators === undefined ? [] : source.parentOperators;
      if (
        !Array.isArray(parents) ||
        parents.some((parent) => !idValue(parent)) ||
        new Set(parents).size !== parents.length
      )
        return invalid();
      budget += parents.length;
      if (budget > 2000) return complexity();
      strings(source.objects);
      strings(source.expressions);
      if (source.alias !== undefined && typeof source.alias !== "string") return invalid();
      estimates(source);
      nodes.set(source.id, { source, parents, children: [] });
    }
    for (const [id, node] of nodes) {
      for (const parent of node.parents) {
        const owner = nodes.get(parent);
        if (!owner) return invalid();
        owner.children.push(id);
      }
    }
    const rootIds = [...nodes].filter(([, node]) => !node.parents.length).map(([id]) => id);
    const pending = new Map([...nodes].map(([id, node]) => [id, node.parents.length]));
    const depths = new Map(rootIds.map((id) => [id, 1]));
    const queue = [...rootIds];
    for (let i = 0; i < queue.length; i++) {
      const id = queue[i],
        depth = depths.get(id)!;
      if (depth > 40) return complexity();
      for (const child of nodes.get(id)!.children) {
        depths.set(child, Math.max(depths.get(child) ?? 0, depth + 1));
        const remaining = pending.get(child)! - 1;
        pending.set(child, remaining);
        if (!remaining) queue.push(child);
      }
    }
    if (queue.length !== nodes.size) return invalid();
    const expanded = new Set<number>();
    const display = (id: number, path: string): PlanNode => {
      const node = nodes.get(id)!,
        source = node.source;
      if (expanded.has(id))
        return {
          path,
          label: `Shared operator ${id} · see its expanded occurrence in Step ${stepIndex + 1}`,
          metrics: [],
          children: [],
        };
      expanded.add(id);
      const expressions = strings(source.expressions);
      return {
        path,
        label: `${source.operation} · operator ${id}`,
        relation: strings(source.objects).join(", ") || undefined,
        metrics: [
          ...estimates(source),
          ...(typeof source.alias === "string" ? [{ label: "Alias", value: source.alias }] : []),
          ...(expressions.length ? [{ label: "Expressions", value: expressions.join("; ") }] : []),
        ],
        children: node.children.map((child) => display(child, `${path}/${child}`)),
      };
    };
    return {
      path: `$.Operations[${stepIndex}]`,
      label: `Step ${stepIndex + 1}`,
      metrics: [],
      children: rootIds.map((id) => display(id, `$.Operations[${stepIndex}]/${id}`)),
    };
  });
  return {
    engine: "Snowflake",
    format: "json",
    actual: false,
    roots,
    timings: [],
    estimates: estimates(data.GlobalStats),
  };
}
