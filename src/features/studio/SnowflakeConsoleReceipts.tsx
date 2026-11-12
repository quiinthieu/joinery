import { useEffect, useState } from "react";
import { useStudio } from "../../state/studio";
import { consoleReceiptHeld, type SnowflakeConsoleReceipt } from "./SnowflakeConsole";

function Receipt({ receipt }: { receipt: SnowflakeConsoleReceipt }) {
  const state = useStudio();
  const view = receipt.view;
  const counts = view?.run.terminal?.accounting;
  const held = consoleReceiptHeld(receipt);
  return (
    <article aria-label={`Console receipt ${receipt.clientRequestId}`}>
      <p>
        <strong>Client request</strong> <code>{receipt.clientRequestId}</code>
        {receipt.runId && (
          <>
            {" "}
            · Run <code>{receipt.runId}</code>
          </>
        )}
      </p>
      {!receipt.sessionId && (
        <p role="status">
          Receipt scope is unresolved. New console runs are held until SQL-free reconciliation
          identifies its original session. A missing lookup is not proof that no SQL was submitted.
        </p>
      )}
      {receipt.error && <p role="alert">{receipt.error}</p>}
      {receipt.notAdmitted && (
        <p role="status">
          No SQL was admitted ({receipt.notAdmitted}). Review again to create a new request; this
          request will not be resent.
        </p>
      )}
      {!view && !receipt.notAdmitted && (
        <p role="status">
          Original outcome unavailable. SQL-free lookup continues; execution will not be resent.
        </p>
      )}
      {view && (
        <>
          <p role="status">
            {view.pendingRecording
              ? "Recording hold: original evidence is retained in memory; durable recording is pending. Do not rerun or close to recover it."
              : view.run.terminal
                ? `Run stopped: ${view.run.terminal.reason}.`
                : "Run is retained and still pending."}
          </p>
          {counts && (
            <p>
              Top-level units: {counts.planned} planned · {counts.intended} durable intents ·{" "}
              {counts.acknowledged} acknowledged · {counts.rejected} rejected · {counts.unknown}{" "}
              unknown · {counts.notDispatched} proven not dispatched · {counts.notIntended}{" "}
              untouched suffix · {counts.notAttempted} not attempted in total.
            </p>
          )}
          <ol>
            {Array.from({ length: view.run.metadata.stepCount }, (_, ordinal) => {
              const step = view.run.steps.find((entry) => entry.ordinal === ordinal);
              const outcome = step?.outcome;
              const text = !step
                ? "Not attempted (untouched suffix)"
                : !outcome
                  ? "Durable intent; original outcome pending"
                  : outcome.kind === "acknowledged"
                    ? "Original statement acknowledged"
                    : outcome.kind === "rejected"
                      ? "Original statement rejected"
                      : outcome.kind === "notDispatched"
                        ? `Proven not dispatched: ${outcome.evidence.reason}`
                        : "Original outcome unknown; may have executed";
              return (
                <li key={ordinal}>
                  Unit {ordinal + 1}: {text}
                  {outcome?.kind === "acknowledged" && (
                    <span>
                      {" "}
                      · Query ID {outcome.evidence.statement.queryId ?? "unavailable"}. Passive
                      response context: database {outcome.evidence.context.database ?? "unknown"},
                      schema {outcome.evidence.context.schema ?? "unknown"}, role{" "}
                      {outcome.evidence.context.role ?? "unknown"}, warehouse{" "}
                      {outcome.evidence.context.warehouse ?? "unknown"}. This does not establish
                      transaction state or committed effects.
                    </span>
                  )}
                  {step?.observations.truncated && (
                    <span> · Result retention limit reached; acknowledged SQL is preserved.</span>
                  )}
                  {step?.observations.deliveryFailed && <span> · Result delivery failed.</span>}
                  {step?.observations.recordingFailed && (
                    <span> · Recording failure observed.</span>
                  )}
                </li>
              );
            })}
          </ol>
          {view.run.observations.cancellationObserved && (
            <p>Cancellation was observed; it does not replace original outcomes.</p>
          )}
          {view.run.observations.retired && (
            <p>The physical session was retired. No reconnect or replay occurs automatically.</p>
          )}
          {view.run.observations.deliveryFailed && (
            <p>Result delivery failed; original statement evidence remains above.</p>
          )}
          {view.run.observations.recordingFailed && (
            <p>
              A local recording failure occurred. Current observations remain separate from the
              original terminal snapshot.
            </p>
          )}
          {view.results.length === 0 && (
            <p>
              Result rows are unavailable in this receipt, including after restart. This does not
              mean the statements returned zero rows.
            </p>
          )}
          <p>
            Statement acknowledgement and result row counts do not prove affected rows, transaction
            health or a successful commit.
          </p>
        </>
      )}
      {!!view?.results.length && (
        <button
          type="button"
          className="button ghost small"
          onClick={() =>
            void state
              .openSnowflakeConsoleResults(receipt.clientRequestId)
              .catch((error) => state.notify(String(error)))
          }
        >
          Open retained results
        </button>
      )}
      {!receipt.notAdmitted && (
        <button
          type="button"
          className="button ghost small"
          onClick={() => void state.refreshSnowflakeConsole(receipt.clientRequestId)}
        >
          Reconcile receipt (no SQL)
        </button>
      )}
      {held && (
        <button
          type="button"
          className="button ghost small"
          onClick={() =>
            void state
              .cancelSnowflakeConsole(receipt.clientRequestId)
              .catch((error) => state.notify(String(error)))
          }
        >
          {receipt.cancellationRequested
            ? "Cancellation requested; reconcile original outcome"
            : "Cancel retained run"}
        </button>
      )}
    </article>
  );
}

/** Mounted at app scope: closing/editing/rebinding a tab never drops ownership. */
export function SnowflakeConsoleReceipts() {
  const receipts = useStudio((state) => state.snowflakeConsoleReceipts);
  const [expanded, setExpanded] = useState(false);
  useEffect(() => {
    const reconcile = () => {
      const state = useStudio.getState();
      for (const receipt of Object.values(state.snowflakeConsoleReceipts))
        if (consoleReceiptHeld(receipt))
          void state.refreshSnowflakeConsole(receipt.clientRequestId);
    };
    reconcile();
    const timer = setInterval(reconcile, 1000);
    return () => clearInterval(timer);
  }, []);
  const entries = Object.values(receipts);
  if (!entries.length) return null;
  const held = entries.some(consoleReceiptHeld);
  return (
    <section aria-label="Snowflake console receipts">
      <button
        type="button"
        className="button ghost small"
        aria-expanded={expanded}
        onClick={() => setExpanded(!expanded)}
      >
        Console receipts ({entries.length}){held ? " · reconciliation pending" : ""}
      </button>
      {(expanded || held) && (
        <div>
          {entries.map((receipt) => (
            <Receipt key={receipt.clientRequestId} receipt={receipt} />
          ))}
        </div>
      )}
    </section>
  );
}
