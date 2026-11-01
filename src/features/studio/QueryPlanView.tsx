import { useEffect, useMemo, useState } from "react";
import type { ResultInfo } from "../../ipc/generated";
import { resultCache } from "../../data/result-cache";
import { parseQueryPlan, type PlanNode } from "./query-plan";

export function isQueryPlanResult(result: ResultInfo) {
  return (
    result.retainedRows === 1 &&
    result.columns.length === 1 &&
    ["QUERY PLAN", "EXPLAIN"].includes(result.columns[0].name.toUpperCase())
  );
}

function Node({ node, depth }: { node: PlanNode; depth: number }) {
  return (
    <li>
      <details open={depth < 2}>
        <summary>
          <strong>{node.label}</strong>
          {node.relation && <code>{node.relation}</code>}
        </summary>
        <dl>
          {node.metrics.map((metric) => (
            <div key={metric.label}>
              <dt>{metric.label}</dt>
              <dd>{metric.value}</dd>
            </div>
          ))}
        </dl>
        {node.children.length > 0 && (
          <ul>
            {node.children.map((child) => (
              <Node key={child.path} node={child} depth={depth + 1} />
            ))}
          </ul>
        )}
      </details>
    </li>
  );
}

export function QueryPlanView({ result }: { result: ResultInfo }) {
  const [raw, setRaw] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<"tree" | "raw">("tree");
  const column = result.columns[0].id;
  useEffect(() => {
    let active = true;
    void resultCache.ensure(result.id, 0, result.retainedRows).then(() => {
      if (!active) return;
      const value = resultCache.read(result.id, column, 0, true);
      if (value?.value != null) setRaw(value.value);
      else
        setError(
          resultCache.error(result.id, 0) ??
            "Plan data is unavailable. Return to the data grid to retry.",
        );
    });
    return () => {
      active = false;
    };
  }, [result.id, result.retainedRows, column]);
  const parsed = useMemo(() => {
    if (raw === null) return null;
    try {
      return { plan: parseQueryPlan(raw), error: null };
    } catch (error) {
      return {
        plan: null,
        error: error instanceof Error ? error.message : "Could not read this plan",
      };
    }
  }, [raw]);
  return (
    <section className="query-plan-view" aria-label="Query execution plan">
      <div className="plan-view-toolbar">
        <strong>
          {parsed?.plan?.actual ? "Plan with actual measurements" : "Estimated query plan"}
        </strong>
        <div>
          <button
            className="button ghost small"
            aria-pressed={view === "tree"}
            onClick={() => setView("tree")}
          >
            Tree
          </button>
          <button
            className="button ghost small"
            aria-pressed={view === "raw"}
            onClick={() => setView("raw")}
          >
            Raw JSON
          </button>
        </div>
      </div>
      {error ? (
        <p role="alert">{error}</p>
      ) : raw === null ? (
        <p role="status">Loading plan…</p>
      ) : view === "raw" ? (
        <pre className="plan-raw">{raw}</pre>
      ) : parsed?.plan ? (
        <>
          <p className="plan-note">
            {parsed.plan.engine} · Costs use engine optimizer units, not milliseconds. Row estimates
            and actual measurements are shown separately.
          </p>
          {parsed.plan.timings.length > 0 && (
            <dl className="plan-timings">
              {parsed.plan.timings.map((metric) => (
                <div key={metric.label}>
                  <dt>{metric.label}</dt>
                  <dd>{metric.value}</dd>
                </div>
              ))}
            </dl>
          )}
          <ul className="plan-tree">
            {parsed.plan.roots.map((node) => (
              <Node key={node.path} node={node} depth={0} />
            ))}
          </ul>
        </>
      ) : (
        <p role="alert">{parsed?.error}</p>
      )}
    </section>
  );
}
