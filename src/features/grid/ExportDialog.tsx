import { useRef, useState } from "react";
import {
  AlertTriangle,
  Check,
  Download,
  FileOutput,
  FolderOpen,
  Loader2,
  Square,
} from "lucide-react";
import { InlineError, Modal } from "../../components/ui/Modal";
import { client, desktopAvailable, messageOf } from "../../ipc/client";
import type {
  BrowseApproval,
  ColumnInfo,
  ConnectionProfile,
  ExportEvent,
  ExportFormat,
  FileSelection,
  PreparedPlan,
  ResultInfo,
  RunStatus,
  SessionInfo,
} from "../../ipc/generated";
import type { StudioTab } from "../../state/studio";
import { ReviewDialog } from "../safety/ReviewDialog";
import { hasRawClauses } from "./table-query";

export const EXPORT_FORMATS: readonly {
  id: ExportFormat;
  label: string;
  extension: string;
  hint: string;
}[] = [
  { id: "csv", label: "CSV", extension: "csv", hint: "RFC 4180 · comma-separated" },
  { id: "tsv", label: "TSV", extension: "tsv", hint: "Tab-separated · \\N nulls" },
  { id: "json", label: "JSON", extension: "json", hint: "Pretty array · exact numbers" },
  { id: "ndjson", label: "NDJSON", extension: "ndjson", hint: "One JSON object per line" },
  { id: "sqlInsert", label: "SQL INSERT", extension: "sql", hint: "Dialect-quoted batches" },
  { id: "markdown", label: "Markdown", extension: "md", hint: "GFM aligned table" },
];

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MiB`;
}

function defaultBaseName(tab: StudioTab): string {
  const raw =
    tab.kind === "table" && tab.table
      ? `${tab.table.schema}-${tab.table.name}`
      : tab.title || "query-result";
  const cleaned = raw
    .trim()
    .replace(/[^a-zA-Z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return cleaned || "export";
}

type ExportSourceMode = "table" | "result";

interface FinishedSummary {
  status: RunStatus;
  rowsWritten: number;
  bytesWritten: number;
  outputFileName: string | null;
  partialFileName: string | null;
  elapsedMs: number;
  errorMessage: string | null;
}

export function ExportDialog({
  tab,
  profile,
  session,
  visibleColumns,
  allColumns,
  result,
  onClose,
}: {
  tab: StudioTab;
  profile?: ConnectionProfile;
  session?: SessionInfo;
  visibleColumns: ColumnInfo[];
  allColumns: ColumnInfo[];
  result?: ResultInfo;
  onClose(): void;
}) {
  const canStreamTable =
    tab.kind === "table" &&
    !!tab.table &&
    !!session &&
    profile?.engine !== "snowflake" &&
    profile?.engine !== "redis";
  const snapshotResult = tab.kind === "table" ? tab.pages[0]?.result : result;

  const [format, setFormat] = useState<ExportFormat>("csv");
  const [sourceMode, setSourceMode] = useState<ExportSourceMode>(() =>
    canStreamTable ? "table" : "result",
  );
  const [columnScope, setColumnScope] = useState<"visible" | "all">("visible");
  const [includeHeaders, setIncludeHeaders] = useState(true);
  const [selection, setSelection] = useState<FileSelection | null>(null);
  const [phase, setPhase] = useState<"configure" | "picking" | "running" | "finished">("configure");
  const [exportId, setExportId] = useState<string | null>(null);
  const [rowsWritten, setRowsWritten] = useState(0);
  const [bytesWritten, setBytesWritten] = useState(0);
  const [sourceTruncated, setSourceTruncated] = useState(false);
  const [finished, setFinished] = useState<FinishedSummary | null>(null);
  const [pendingPlan, setPendingPlan] = useState<PreparedPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const activeExportRef = useRef<string | null>(null);

  const formatMeta = EXPORT_FORMATS.find((entry) => entry.id === format) ?? EXPORT_FORMATS[0];
  const suggestedFileName = `${defaultBaseName(tab)}.${formatMeta.extension}`;
  const selectedColumns = columnScope === "visible" ? visibleColumns : allColumns;
  const hasCustomLayout =
    visibleColumns.length !== allColumns.length ||
    visibleColumns.some((col, index) => col.id !== allColumns[index]?.id);
  const snapshotIsTruncated = sourceMode === "result" && snapshotResult?.status === "truncated";
  const keylessTableWarning =
    sourceMode === "table" &&
    tab.kind === "table" &&
    tab.schema !== null &&
    tab.schema.primaryKey.length === 0;

  async function pickDestination(): Promise<FileSelection | null> {
    if (!desktopAvailable) {
      setError("File export is available in the desktop app.");
      return null;
    }
    setPhase("picking");
    setError(null);
    try {
      const picked = await client.call<FileSelection | null>("pick_export_destination", {
        suggestedName: suggestedFileName,
        format,
      });
      setPhase("configure");
      if (picked) {
        setSelection(picked);
      }
      return picked;
    } catch (err) {
      setPhase("configure");
      setError(messageOf(err));
      return null;
    }
  }

  async function beginStream(chosen: FileSelection, approval: BrowseApproval | null) {
    if (selectedColumns.length === 0) {
      setError("No columns are selected for export.");
      return;
    }
    const source =
      sourceMode === "table"
        ? (() => {
            if (!session || !tab.table) {
              throw new Error("Connect to the database before streaming a table export.");
            }
            return {
              kind: "table" as const,
              sessionId: session.id,
              table: tab.table,
              query: tab.query,
              approval,
              columnIds: selectedColumns.map((col) => col.id),
            };
          })()
        : (() => {
            if (!snapshotResult) {
              throw new Error("Run a query or load a table page before exporting.");
            }
            return {
              kind: "result" as const,
              resultId: snapshotResult.id,
              columnIds: selectedColumns.map((col) => col.id),
            };
          })();

    setError(null);
    setFinished(null);
    setRowsWritten(0);
    setBytesWritten(0);
    setSourceTruncated(snapshotIsTruncated);
    setPhase("running");

    try {
      const startedId = await client.exportData(
        {
          destinationToken: chosen.token,
          format,
          source,
          includeHeaders,
          engine: profile?.engine ?? null,
          targetTable: tab.kind === "table" ? (tab.table ?? null) : null,
        },
        (event: ExportEvent) => {
          if (event.type === "started") {
            activeExportRef.current = event.exportId;
            setExportId(event.exportId);
            setSourceTruncated(event.sourceTruncated);
          } else if (event.type === "progress") {
            setRowsWritten(event.rowsWritten);
            setBytesWritten(event.bytesWritten);
          } else if (event.type === "finished") {
            activeExportRef.current = null;
            setExportId(null);
            setRowsWritten(event.rowsWritten);
            setBytesWritten(event.bytesWritten);
            setSelection(null);
            setFinished({
              status: event.status,
              rowsWritten: event.rowsWritten,
              bytesWritten: event.bytesWritten,
              outputFileName: event.outputFileName,
              partialFileName: event.partialFileName,
              elapsedMs: event.elapsedMs,
              errorMessage: event.error?.message ?? null,
            });
            setPhase("finished");
          }
        },
      );
      activeExportRef.current = startedId;
      setExportId(startedId);
    } catch (err) {
      activeExportRef.current = null;
      setExportId(null);
      setSelection(null);
      setPhase("configure");
      setError(messageOf(err));
    }
  }

  async function handleStartExport() {
    setError(null);
    if (selectedColumns.length === 0) {
      setError("Select at least one visible column before exporting.");
      return;
    }

    if (sourceMode === "table") {
      if (!session || !tab.table) {
        setError("Reconnect to the database before streaming all table rows.");
        return;
      }
      if (hasRawClauses(tab.query)) {
        try {
          const plan = await client.call<PreparedPlan | null>("prepare_browse_table", {
            sessionId: session.id,
            table: tab.table,
            query: tab.query,
          });
          if (plan) {
            const chosen = selection ?? (await pickDestination());
            if (!chosen) return;
            setPendingPlan(plan);
            return;
          }
        } catch (err) {
          setError(messageOf(err));
          return;
        }
      }
    } else if (!snapshotResult) {
      setError("No retained result rows are loaded to export.");
      return;
    }

    const chosen = selection ?? (await pickDestination());
    if (!chosen) return;
    await beginStream(chosen, null);
  }

  async function handleCancelExport() {
    const id = activeExportRef.current ?? exportId;
    if (!id) return;
    try {
      await client.call("cancel_export", { exportId: id });
    } catch (err) {
      setError(messageOf(err));
    }
  }

  if (pendingPlan && selection) {
    const chosen = selection;
    const plan = pendingPlan;
    return (
      <ReviewDialog
        plan={plan}
        profile={profile}
        onExecute={async (approval) => {
          setPendingPlan(null);
          await beginStream(chosen, { planId: plan.id, approval });
        }}
        onClose={() => setPendingPlan(null)}
      />
    );
  }

  const busy = phase === "running" || phase === "picking";

  return (
    <Modal
      title="Export data"
      subtitle={
        tab.kind === "table" && tab.table
          ? `${tab.table.schema}.${tab.table.name} · ${profile?.name ?? "Database"}`
          : `${tab.title} · ${profile?.name ?? "Query result"}`
      }
      onClose={onClose}
      wide
      busy={busy}
    >
      <div className="modal-body export-dialog-body">
        <div className="form-grid">
          <label>
            Format
            <select
              aria-label="Export format"
              value={format}
              disabled={busy}
              onChange={(event) => {
                setFormat(event.target.value as ExportFormat);
                setSelection(null);
                setFinished(null);
              }}
            >
              {EXPORT_FORMATS.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.label} (.{item.extension}) — {item.hint}
                </option>
              ))}
            </select>
          </label>

          <label>
            Source
            <select
              aria-label="Export source"
              value={sourceMode}
              disabled={busy || (!canStreamTable && !snapshotResult)}
              onChange={(event) => {
                setSourceMode(event.target.value as ExportSourceMode);
                setFinished(null);
              }}
            >
              {canStreamTable ? (
                <option value="table">
                  All matching table rows (streamed without retaining in memory)
                </option>
              ) : null}
              {snapshotResult ? (
                <option value="result">
                  {tab.kind === "table"
                    ? `Loaded page snapshot (${snapshotResult.retainedRows.toLocaleString()} rows)`
                    : `Retained query result (${snapshotResult.retainedRows.toLocaleString()} rows)`}
                </option>
              ) : null}
            </select>
          </label>

          {hasCustomLayout ? (
            <label className="span-2">
              Columns
              <select
                aria-label="Export columns"
                value={columnScope}
                disabled={busy}
                onChange={(event) => setColumnScope(event.target.value as "visible" | "all")}
              >
                <option value="visible">
                  Visible columns in current order ({visibleColumns.length} of {allColumns.length})
                </option>
                <option value="all">All columns in schema order ({allColumns.length})</option>
              </select>
            </label>
          ) : null}

          {format === "csv" || format === "tsv" ? (
            <label className="checkbox-label span-2">
              <input
                type="checkbox"
                checked={includeHeaders}
                disabled={busy}
                onChange={(event) => setIncludeHeaders(event.target.checked)}
              />
              <span>
                Include column header row · empty strings stay quoted (<code>&quot;&quot;</code>) so{" "}
                <code>NULL</code> round-trips cleanly
              </span>
            </label>
          ) : null}

          <div className="span-2 export-destination-row">
            <div>
              <span className="export-destination-label">Destination file</span>
              <strong className="export-destination-name">
                {selection ? selection.displayLabel : `${suggestedFileName} (choose on export)`}
              </strong>
            </div>
            <button
              type="button"
              className="button ghost small"
              disabled={busy}
              onClick={() => void pickDestination()}
            >
              <FolderOpen size={13} />
              {selection ? "Change file…" : "Choose file…"}
            </button>
          </div>
        </div>

        {snapshotIsTruncated || sourceTruncated ? (
          <div className="safety-warning" role="status">
            <AlertTriangle size={15} />
            <span>
              This result hit Joinery&apos;s in-memory retention limit (
              {snapshotResult?.retainedRows.toLocaleString() ?? 0} rows retained). Only retained
              rows are written; narrow the query or stream from a table tab with a primary key for
              full datasets.
            </span>
          </div>
        ) : null}

        {keylessTableWarning ? (
          <div className="safety-warning" role="status">
            <AlertTriangle size={15} />
            <span>
              This table has no primary key, so keyset streaming can export up to 500 rows. Tables
              exceeding 500 rows abort cleanly without publishing a truncated file.
            </span>
          </div>
        ) : null}

        {phase === "running" ? (
          <div className="export-progress-card" role="status" aria-live="polite">
            <div className="export-progress-head">
              <Loader2 size={15} className="spin" />
              <strong>Streaming to {selection?.fileName ?? suggestedFileName}.partial…</strong>
            </div>
            <p>
              {rowsWritten.toLocaleString()} rows · {formatBytes(bytesWritten)} written. Final file
              is published atomically only after every row flushes.
            </p>
          </div>
        ) : null}

        {phase === "finished" && finished ? (
          <div
            className={`export-status-card ${finished.status === "completed" ? "completed" : "incomplete"}`}
            role={finished.status === "failed" ? "alert" : "status"}
          >
            {finished.status === "completed" ? (
              <>
                <div className="export-progress-head">
                  <Check size={15} />
                  <strong>Exported {finished.outputFileName}</strong>
                </div>
                <p>
                  {finished.rowsWritten.toLocaleString()} rows ·{" "}
                  {formatBytes(finished.bytesWritten)} · {finished.elapsedMs} ms
                </p>
              </>
            ) : (
              <>
                <div className="export-progress-head">
                  <AlertTriangle size={15} />
                  <strong>
                    {finished.status === "cancelled" ? "Export cancelled" : "Export failed"}
                  </strong>
                </div>
                <p>
                  {finished.errorMessage ? `${finished.errorMessage} ` : ""}
                  {finished.partialFileName
                    ? `Final file was not created; partial output was marked incomplete in ${finished.partialFileName} (${finished.rowsWritten.toLocaleString()} rows).`
                    : "Final file was not created."}
                </p>
              </>
            )}
          </div>
        ) : null}

        <p className="form-note">
          64-bit integers and high-scale decimals stream as exact text (up to 1,000,000 rows / 512
          MiB). Neither exported rows nor file paths are stored in local app history.
        </p>
        <InlineError message={error} />
      </div>

      <footer className="modal-footer">
        {phase === "running" ? (
          <button type="button" className="button danger" onClick={() => void handleCancelExport()}>
            <Square size={12} />
            Cancel export
          </button>
        ) : (
          <>
            <button type="button" className="button ghost" disabled={busy} onClick={onClose}>
              {phase === "finished" && finished?.status === "completed" ? "Done" : "Close"}
            </button>
            <button
              type="button"
              className="button primary"
              disabled={busy || selectedColumns.length === 0}
              onClick={() => void handleStartExport()}
            >
              {phase === "picking" ? (
                <Loader2 size={14} className="spin" />
              ) : finished ? (
                <FileOutput size={14} />
              ) : (
                <Download size={14} />
              )}
              {selection ? "Start export" : "Choose destination & export"}
            </button>
          </>
        )}
      </footer>
    </Modal>
  );
}
