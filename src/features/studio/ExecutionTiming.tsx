import type { QueryEvent } from "../../ipc/generated";

type Metrics = Extract<QueryEvent, { type: "runMetrics" }>;
const milliseconds = (value: number | null | undefined) =>
  value == null ? "Unavailable" : `${Math.round(value)} ms`;

export function ExecutionTiming({
  metrics,
  clientElapsedMs,
}: {
  metrics?: Metrics;
  clientElapsedMs?: number;
}) {
  return (
    <details className="execution-timing">
      <summary>Execution timing</summary>
      <dl>
        <div>
          <dt>Queue wait</dt>
          <dd>{milliseconds(metrics?.queueMs)}</dd>
        </div>
        <div>
          <dt>Backend active</dt>
          <dd>{milliseconds(metrics?.activeMs)}</dd>
        </div>
        <div>
          <dt>Backend total</dt>
          <dd>{milliseconds(metrics?.totalMs)}</dd>
        </div>
        <div>
          <dt>First row batch</dt>
          <dd>{milliseconds(metrics?.firstRowMs)}</dd>
        </div>
        <div>
          <dt>Client elapsed</dt>
          <dd>{milliseconds(clientElapsedMs)}</dd>
        </div>
      </dl>
      <p>
        Queue wait precedes backend work. First row batch starts after the queue and is unavailable
        when no rows arrive. Drivers may buffer rows. Backend totals exclude history recording and
        delivery to this view; client elapsed includes delivery. Server plan timings are separate.
      </p>
    </details>
  );
}
