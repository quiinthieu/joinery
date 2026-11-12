import { useEffect, useId, useRef, useState } from "react";
import { useStudio } from "../../state/studio";
import { client, messageOf } from "../../ipc/client";
import type {
  SnowflakeConsoleReceiptSummary,
  SnowflakeConsoleReceiptFilter,
  SnowflakeConsoleReceiptPage,
  SnowflakeConsoleStorage,
} from "../../ipc/generated";
import {
  consoleReceiptHeld,
  consoleReceiptStorageProblem,
  rememberConsoleReceipts,
  validateConsoleSummary,
  validateConsoleStorage,
  type SnowflakeConsoleReceipt,
} from "./SnowflakeConsole";
import { startConsoleReceiptReconciler } from "./SnowflakeConsoleReconciler";

function Receipt({ receipt }: { receipt: SnowflakeConsoleReceipt }) {
  const state = useStudio();
  const [release, setRelease] = useState(false);
  const view = receipt.view;
  const counts = view?.run.terminal?.accounting;
  const held = consoleReceiptHeld(receipt);
  return (
    <article
      className="snowflake-console-receipt"
      aria-label={`Console receipt ${receipt.clientRequestId}`}
    >
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
      {receipt.decisionConflict && (
        <p role="alert">
          Conflicting native admission decisions were observed. New console runs remain held;
          reconciliation will not erase the original evidence or resend SQL.
        </p>
      )}
      {receipt.notAdmitted && (
        <p role="status">
          Durable pre-admission refusal ({receipt.notAdmitted}).{" "}
          {receipt.lookupPending
            ? "Reconciliation is unresolved; new console runs remain held. "
            : "No SQL was admitted. Review again to create a new request. "}
          This request will not be resent.
        </p>
      )}
      {!view && !receipt.notAdmitted && (
        <p role="status">
          {receipt.settledProof
            ? "Full receipt details are deferred to keep memory bounded. The original terminal identity is retained for native inventory reconciliation."
            : "Original outcome unavailable. SQL-free lookup continues; execution will not be resent."}
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
        <>
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
          <button
            type="button"
            className="button ghost small"
            disabled={held || !receipt.nativeSettled}
            onClick={() => setRelease(true)}
          >
            Release retained result rows
          </button>
          {release && (
            <div role="group" aria-label="Confirm result row release">
              <p>
                Release these {view.results.length} process-local results? Their rows will become
                unavailable. Original receipts and accounting stay intact. No SQL runs.
              </p>
              <button
                type="button"
                className="button ghost small"
                onClick={() =>
                  void state
                    .releaseSnowflakeConsoleResults(receipt.clientRequestId)
                    .then(() => setRelease(false))
                    .catch((error) => state.notify(messageOf(error)))
                }
              >
                Confirm row release
              </button>
              <button
                type="button"
                className="button ghost small"
                onClick={() => setRelease(false)}
              >
                Keep rows
              </button>
            </div>
          )}
        </>
      )}
      <button
        type="button"
        className="button ghost small"
        onClick={() => void state.refreshSnowflakeConsole(receipt.clientRequestId)}
      >
        Reconcile receipt (no SQL)
      </button>
      {held && !receipt.notAdmitted && (
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
  const profiles = useStudio((state) => state.bootstrap?.profiles);
  const [expanded, setExpanded] = useState(false);
  const [page, setPage] = useState<SnowflakeConsoleReceiptPage | null>(null);
  const [storage, setStorage] = useState<SnowflakeConsoleStorage | null>(null);
  const [storageReadProblem, setStorageReadProblem] = useState<string | null>(null);
  const [filter, setFilter] = useState<SnowflakeConsoleReceiptFilter>({
    visibility: "visible",
    kind: null,
    connectionId: null,
  });
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<Array<string | null>>([]);
  const [refresh, setRefresh] = useState(0);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [reconcileProblem, setReconcileProblem] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [lookup, setLookup] = useState("");
  const [pendingOffset, setPendingOffset] = useState(0);
  const historyId = useId();
  const refreshButton = useRef<HTMLButtonElement>(null);
  const entries = Object.values(receipts);
  useEffect(() => {
    let stopped = false;
    const stop = startConsoleReceiptReconciler({
      getReceipts: () => useStudio.getState().snowflakeConsoleReceipts,
      refresh: (id) => useStudio.getState().refreshSnowflakeConsole(id),
      handoff: (id) => useStudio.getState().handoffSnowflakeConsoleReceipt(id),
      onError: (message) => {
        if (!stopped) setReconcileProblem(message);
      },
      discoverPending: async () => {
        const pending = await client.call("snowflake_console_pending");
        if (stopped) return;
        if (
          !pending ||
          !Array.isArray(pending.clientRequestIds) ||
          pending.clientRequestIds.length > 128 ||
          typeof pending.recordingHold !== "boolean" ||
          pending.clientRequestIds.some(
            (id) => !/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i.test(id),
          )
        )
          throw new Error(
            "Native pending receipts are unavailable; absence has not been established.",
          );
        const current = useStudio.getState().snowflakeConsoleReceipts;
        const next = { ...current };
        for (const id of pending.clientRequestIds)
          if (!next[id]) next[id] = { clientRequestId: id, lookupPending: true };
        if (Object.keys(next).length !== Object.keys(current).length) {
          rememberConsoleReceipts(next);
          useStudio.setState({ snowflakeConsoleReceipts: next });
        }
        if (pending.recordingHold)
          setReconcileProblem(
            "Native recording is pending. Keep Joinery open and reconcile original evidence; do not resend SQL.",
          );
        else setReconcileProblem(null);
      },
    });
    return () => {
      stopped = true;
      stop();
    };
  }, []);
  useEffect(() => {
    let stopped = false;
    void client
      .call("snowflake_console_storage")
      .then((value) => {
        if (stopped) return;
        validateConsoleStorage(value);
        setStorage(value);
        setStorageReadProblem(null);
      })
      .catch((error) => {
        if (!stopped) {
          setStorage(null);
          setStorageReadProblem(messageOf(error));
        }
      });
    return () => {
      stopped = true;
    };
  }, [refresh]);
  useEffect(() => {
    if (!expanded) return;
    let stopped = false;
    setBusy(true);
    void client
      .call("snowflake_console_receipts", { request: { filter, cursor, limit: 25 } })
      .then((value) => {
        if (stopped) return;
        if (
          !value ||
          !/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i.test(
            value.inventoryEpoch,
          ) ||
          !Array.isArray(value.receipts) ||
          value.receipts.length > 25 ||
          (value.nextCursor !== null &&
            (typeof value.nextCursor !== "string" ||
              value.nextCursor.length === 0 ||
              value.nextCursor.length > 2048))
        )
          throw new Error("Native receipt inventory is unavailable.");
        value.receipts.forEach(validateConsoleSummary);
        if (value.receipts.some((receipt) => receipt.inventoryEpoch !== value.inventoryEpoch))
          throw new Error("Receipt inventory epoch changed; refresh from the first page.");
        setPage(value);
        setProblem(null);
      })
      .catch((error) => {
        if (!stopped) {
          setPage(null);
          setProblem(`${messageOf(error)} Refresh from the first page; SQL will not be resent.`);
        }
      })
      .finally(() => {
        if (!stopped) setBusy(false);
      });
    return () => {
      stopped = true;
    };
  }, [expanded, filter, cursor, refresh]);
  const pending = entries.filter(consoleReceiptHeld);
  const held = pending.length > 0;
  const storageProblem = consoleReceiptStorageProblem();
  const active = selected ? receipts[selected] : (pending[0] ?? entries.at(-1));
  const firstPage = () => {
    setCursor(null);
    setPrevious([]);
    setRefresh((value) => value + 1);
  };
  const choose = (summary: SnowflakeConsoleReceiptSummary) => {
    setSelected(summary.clientRequestId);
    void useStudio
      .getState()
      .discoverSnowflakeConsoleReceipt(summary)
      .catch((error) => setProblem(messageOf(error)));
  };
  const archive = async (summary: SnowflakeConsoleReceiptSummary) => {
    setBusy(true);
    const {
      clientRequestId,
      sessionId,
      planDigest,
      runId,
      kind,
      recordedAt,
      originalRevision,
      browsingRevision,
      refusalCode,
    } = summary;
    try {
      await client.call("set_snowflake_console_receipt_archive", {
        request: {
          inventoryEpoch: summary.inventoryEpoch,
          items: [
            {
              clientRequestId,
              sessionId,
              planDigest,
              runId,
              kind,
              recordedAt,
              originalRevision,
              browsingRevision,
              refusalCode,
            },
          ],
          archived: !summary.archived,
        },
      });
      firstPage();
    } catch (error) {
      // Recover only browsing state. Never retry a mutation or start SQL.
      try {
        const current = await client.call("snowflake_console_receipt_summary", { clientRequestId });
        if (current) validateConsoleSummary(current);
        if (
          !current ||
          current.clientRequestId !== clientRequestId ||
          current.sessionId !== sessionId ||
          current.planDigest !== planDigest ||
          current.kind !== kind ||
          current.runId !== runId ||
          current.recordedAt !== recordedAt ||
          current.originalRevision !== originalRevision ||
          current.refusalCode !== refusalCode
        )
          throw new Error("Current archive preference is unavailable.");
        setProblem(
          `Archive response was unavailable. Current local preference is ${current.archived ? "archived" : "visible"}; original evidence is preserved. No SQL was sent.`,
        );
      } catch {
        setProblem(
          `${messageOf(error)} Reconcile the archive preference by exact request ID; no SQL was sent.`,
        );
      }
    } finally {
      setBusy(false);
      // The archived row may disappear. Keep keyboard focus on a stable control.
      refreshButton.current?.focus();
    }
  };
  return (
    <section className="snowflake-console-receipts" aria-label="Snowflake console receipts">
      <div className="snowflake-console-receipts-header">
        <button
          type="button"
          className="button ghost small"
          aria-expanded={expanded}
          aria-controls={historyId}
          title={expanded ? "Hide settled receipt history" : "Show settled receipt history"}
          onClick={() => setExpanded(!expanded)}
        >
          Console receipts (
          {storage
            ? (BigInt(storage.positiveCount) + BigInt(storage.refusalCount)).toString()
            : entries.length}
          ){held ? " · reconciliation pending" : ""}
        </button>
        {held && (
          <p role="status">
            {pending.length} pending {pending.length === 1 ? "receipt remains" : "receipts remain"}{" "}
            visible until reconciled.
          </p>
        )}
      </div>
      {storageProblem && <p role="alert">{storageProblem}</p>}
      {storageReadProblem && <p role="alert">{storageReadProblem}</p>}
      {reconcileProblem && <p role="alert">{reconcileProblem}</p>}
      <div
        className="snowflake-console-receipts-details"
        role="region"
        aria-label="Console receipt details"
        tabIndex={0}
        hidden={!expanded && !held}
      >
        {pending.length > 0 && (
          <div aria-label="Pending console correlations">
            <p>
              Pending correlations are kept outside settled history. An unknown outcome may have
              executed.
            </p>
            {pending.slice(pendingOffset, pendingOffset + 25).map((receipt) => (
              <button
                type="button"
                className="button ghost small"
                key={receipt.clientRequestId}
                onClick={() => setSelected(receipt.clientRequestId)}
              >
                Pending request {receipt.clientRequestId}
              </button>
            ))}
            <button
              type="button"
              disabled={pendingOffset === 0}
              onClick={() => setPendingOffset(Math.max(0, pendingOffset - 25))}
            >
              Previous pending requests
            </button>
            <button
              type="button"
              disabled={pendingOffset + 25 >= pending.length}
              onClick={() => setPendingOffset(pendingOffset + 25)}
            >
              Next pending requests
            </button>
          </div>
        )}
        {active && (expanded || consoleReceiptHeld(active)) && (
          <Receipt key={active.clientRequestId} receipt={active} />
        )}
        <div id={historyId} hidden={!expanded}>
          {expanded && (
            <>
              <p>
                Archive keeps original evidence and recovery identity. It does not free journal disk
                space or run SQL. Result row release is separate.
              </p>
              {storage && (
                <p>
                  Journal: {storage.positiveCount} admitted runs · {storage.refusalCount} durable
                  refusals.{" "}
                  {storage.chargedBytes === null
                    ? "Charged storage unavailable."
                    : `${storage.chargedBytes} charged bytes.`}{" "}
                  {!storage.retentionEnabled &&
                    "Existing capacity limits remain; expanded retention is not enabled."}
                </p>
              )}
              <label>
                Receipt visibility
                <select
                  aria-label="Receipt visibility"
                  value={filter.visibility}
                  onChange={(event) => {
                    setFilter({
                      ...filter,
                      visibility: event.target.value as typeof filter.visibility,
                    });
                    firstPage();
                  }}
                >
                  <option value="visible">Visible</option>
                  <option value="archived">Archived</option>
                  <option value="all">All</option>
                </select>
              </label>
              <label>
                Receipt decision
                <select
                  aria-label="Receipt decision"
                  value={filter.kind ?? "all"}
                  onChange={(event) => {
                    setFilter({
                      ...filter,
                      kind:
                        event.target.value === "all"
                          ? null
                          : (event.target.value as "positive" | "refusal"),
                    });
                    firstPage();
                  }}
                >
                  <option value="all">All decisions</option>
                  <option value="positive">Admitted runs</option>
                  <option value="refusal">Durable refusals</option>
                </select>
              </label>
              <label>
                Receipt connection
                <select
                  aria-label="Receipt connection"
                  value={filter.connectionId ?? ""}
                  onChange={(event) => {
                    setFilter({ ...filter, connectionId: event.target.value || null });
                    firstPage();
                  }}
                >
                  <option value="">All connections, including unrecorded</option>
                  {profiles
                    ?.filter((profile) => profile.engine === "snowflake")
                    .map((profile) => (
                      <option key={profile.id} value={profile.id}>
                        {profile.name}
                      </option>
                    ))}
                </select>
              </label>
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  void client
                    .call("snowflake_console_receipt_summary", { clientRequestId: lookup })
                    .then((summary) => {
                      if (!summary)
                        throw new Error(
                          "No durable receipt is discoverable. This does not prove that no SQL ran.",
                        );
                      choose(summary);
                    })
                    .catch((error) => setProblem(messageOf(error)));
                }}
              >
                <label>
                  Exact client request ID
                  <input
                    value={lookup}
                    maxLength={36}
                    onChange={(event) => setLookup(event.target.value)}
                    required
                  />
                </label>
                <button type="submit">Look up receipt (no SQL)</button>
              </form>
              <button
                type="button"
                ref={refreshButton}
                aria-disabled={busy}
                onClick={() => {
                  if (!busy) firstPage();
                }}
              >
                Refresh receipts
              </button>
              {problem && <p role="alert">{problem}</p>}
              <div role="status" aria-live="polite">
                {busy
                  ? "Loading local receipts…"
                  : page
                    ? `${page.receipts.length} receipts on this page.`
                    : "Receipt inventory unavailable."}
              </div>
              <ul aria-label="Durable console receipt inventory">
                {page?.receipts.map((summary) => (
                  <li key={summary.clientRequestId}>
                    <button
                      type="button"
                      className="button ghost small"
                      aria-pressed={selected === summary.clientRequestId}
                      onClick={() => choose(summary)}
                    >
                      Receipt {summary.clientRequestId}
                    </button>
                    {summary.unknown && (
                      <strong> Original outcome unknown; may have executed.</strong>
                    )}
                    <span>
                      {" "}
                      {summary.kind === "refusal"
                        ? `Durable refusal (${summary.refusalCode})`
                        : `Run ${summary.state}${summary.stopReason ? `: ${summary.stopReason}` : ""}`}{" "}
                      ·{" "}
                      {summary.connectionId
                        ? (profiles?.find((profile) => profile.id === summary.connectionId)?.name ??
                          summary.connectionId)
                        : "Connection not recorded"}
                    </span>
                    <button
                      type="button"
                      className="button ghost small"
                      disabled={busy || !summary.archiveEligible}
                      title={summary.archiveBlockReason ?? undefined}
                      onClick={() => void archive(summary)}
                    >
                      {summary.archived ? "Unarchive receipt" : "Archive receipt"}
                    </button>
                    {!summary.archiveEligible && (
                      <span>
                        {" "}
                        {summary.archiveBlockReason ?? "Receipt is not settled for archiving."}
                      </span>
                    )}
                  </li>
                ))}
              </ul>
              <button
                type="button"
                disabled={busy || previous.length === 0}
                onClick={() => {
                  setCursor(previous.at(-1) ?? null);
                  setPrevious(previous.slice(0, -1));
                }}
              >
                Previous receipts
              </button>
              <button
                type="button"
                disabled={busy || !page?.nextCursor}
                onClick={() => {
                  setPrevious([...previous, cursor].slice(-8));
                  setCursor(page!.nextCursor);
                }}
              >
                Next receipts
              </button>
            </>
          )}
        </div>
      </div>
    </section>
  );
}
