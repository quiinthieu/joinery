import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import type { ResultInfo } from "../../ipc/generated";
import { resultCache } from "../../data/result-cache";
import { parseQueryPlan, type PlanNode, type QueryPlan } from "./query-plan";
import { planMismatch, planSnapshots, type PlanSource } from "./plan-snapshots";

export function isQueryPlanResult(result: ResultInfo) {
  return (
    result.retainedRows === 1 &&
    result.columns.length === 1 &&
    ["QUERY PLAN", "EXPLAIN", "ANALYZE"].includes(result.columns[0].name.toUpperCase())
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

function PlanTree({ plan }: { plan: QueryPlan }) {
  return (
    <>
      {plan.timings.length > 0 && (
        <dl className="plan-timings">
          {plan.timings.map((metric) => (
            <div key={metric.label}>
              <dt>{metric.label}</dt>
              <dd>{metric.value}</dd>
            </div>
          ))}
        </dl>
      )}
      <ul className="plan-tree">
        {plan.roots.map((node) => (
          <Node key={node.path} node={node} depth={0} />
        ))}
      </ul>
    </>
  );
}

export function QueryPlanView({
  result,
  ownerTabId,
  source,
  canCapture,
}: {
  result: ResultInfo;
  ownerTabId: string;
  source?: PlanSource;
  canCapture: boolean;
}) {
  const [raw, setRaw] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<"tree" | "raw">("tree");
  const snapshots = useSyncExternalStore(planSnapshots.subscribe, planSnapshots.snapshot);
  const [baselineId, setBaselineId] = useState("");
  const [notice, setNotice] = useState("");
  const baseline = snapshots.find((row) => String(row.id) === baselineId);
  const compatibleBaseline =
    baseline && source && !planMismatch(baseline.source, source) ? baseline : null;
  const baselinePlan = useMemo(
    () => (compatibleBaseline ? parseQueryPlan(compatibleBaseline.raw) : null),
    [compatibleBaseline],
  );
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
            disabled={
              !raw ||
              !parsed?.plan ||
              !source ||
              !canCapture ||
              snapshots.some((row) => row.resultId === result.id)
            }
            onClick={() => {
              if (!raw || !source || !canCapture) return;
              try {
                const { saved, evicted } = planSnapshots.capture(
                  ownerTabId,
                  result.id,
                  source,
                  raw,
                );
                setNotice(
                  `Kept snapshot ${saved.id}.${evicted ? ` Evicted ${evicted} oldest snapshot(s).` : ""}`,
                );
              } catch (error) {
                setNotice(error instanceof Error ? error.message : "Could not keep this plan");
              }
            }}
          >
            Keep snapshot
          </button>
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
      <div className="plan-snapshot-toolbar">
        <label>
          Compare with snapshot
          <select
            aria-label="Compare with snapshot"
            value={compatibleBaseline ? baselineId : ""}
            onChange={(event) => setBaselineId(event.target.value)}
          >
            <option value="">Current plan only</option>
            {snapshots
              .filter((row) => row.resultId !== result.id)
              .map((row) => {
                const mismatch = source
                  ? planMismatch(row.source, source)
                  : "Execution identity unavailable";
                return (
                  <option key={row.id} value={row.id} disabled={!!mismatch}>
                    Snapshot {row.id} · {new Date(row.capturedAt).toLocaleTimeString()}
                    {mismatch ? ` · ${mismatch}` : ""}
                  </option>
                );
              })}
          </select>
        </label>
        {compatibleBaseline && (
          <button
            className="button ghost small"
            onClick={() => planSnapshots.remove(compatibleBaseline.id)}
          >
            Remove selected snapshot
          </button>
        )}
        {snapshots.length > 0 && (
          <button className="button ghost small" onClick={planSnapshots.clear}>
            Clear all snapshots
          </button>
        )}
      </div>
      <p className="plan-note">
        {snapshots.length}/10 snapshots · {(planSnapshots.retainedBytes / 1024).toFixed(1)} / 2048
        KiB payload. Kept in memory until tab/session close; oldest evicted at either limit.
        Capturing and comparing never execute SQL.
      </p>
      {!source && (
        <p className="plan-note">
          Comparable execution identity is unavailable. Use Explain or Analyze with a known session
          context to keep a snapshot.
        </p>
      )}
      {notice && <p role="status">{notice}</p>}
      {source && (
        <details className="plan-source">
          <summary>Executed SQL and observed context</summary>
          <pre className="plan-source-text">{source.executedSql}</pre>
          <p>
            {source.engine} · {source.serverVersion} · Session {source.sessionId}
          </p>
          <pre className="plan-source-text">{source.context}</pre>
        </details>
      )}
      {compatibleBaseline && (
        <p className="plan-note">
          Same query text, engine/version, session and observed context. Database state and
          unobserved settings may differ. Trees are independent; nodes are not paired by position.
        </p>
      )}
      {error ? (
        <p role="alert">{error}</p>
      ) : raw === null ? (
        <p role="status">Loading plan…</p>
      ) : compatibleBaseline && baselinePlan && parsed?.plan ? (
        <section className="plan-comparison" aria-label="Plan comparison">
          <section aria-label="Snapshot plan">
            <h3>
              Snapshot {compatibleBaseline.id} · {baselinePlan.actual ? "Actual" : "Estimated"}
            </h3>
            {view === "raw" ? (
              <pre className="plan-raw">{compatibleBaseline.raw}</pre>
            ) : (
              <PlanTree plan={baselinePlan} />
            )}
          </section>
          <section aria-label="Current plan">
            <h3>Current plan · {parsed.plan.actual ? "Actual" : "Estimated"}</h3>
            {view === "raw" ? (
              <pre className="plan-raw">{raw}</pre>
            ) : (
              <PlanTree plan={parsed.plan} />
            )}
          </section>
        </section>
      ) : view === "raw" ? (
        <pre className="plan-raw">{raw}</pre>
      ) : parsed?.plan ? (
        <>
          <p className="plan-note">
            {parsed.plan.engine} · Costs use engine optimizer units, not milliseconds. Row estimates
            and actual measurements are shown separately.
          </p>
          <PlanTree plan={parsed.plan} />
        </>
      ) : (
        <p role="alert">{parsed?.error}</p>
      )}
    </section>
  );
}
