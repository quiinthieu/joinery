import { useRef, useState } from "react";
import {
  AlertTriangle,
  Check,
  FileSpreadsheet,
  FolderOpen,
  Loader2,
  Play,
  Upload,
} from "lucide-react";
import { InlineError, Modal } from "../../components/ui/Modal";
import { client, desktopAvailable, messageOf } from "../../ipc/client";
import type {
  ChangePreview,
  ConnectionProfile,
  CsvColumnMapping,
  CsvDelimiter,
  CsvImportOptions,
  CsvImportPreview,
  FileSelection,
  QueryEvent,
  RunStatus,
  SessionInfo,
  TableRef,
  TableSchema,
} from "../../ipc/generated";
import { ReviewDialog } from "../safety/ReviewDialog";

const DELIMITERS: readonly { id: CsvDelimiter; label: string }[] = [
  { id: "comma", label: "Comma (,)" },
  { id: "tab", label: "Tab (\\t)" },
  { id: "semicolon", label: "Semicolon (;)" },
  { id: "pipe", label: "Pipe (|)" },
];

interface PendingBatch {
  batchIndex: number;
  preview: ChangePreview;
}

interface BatchFailure {
  batchIndex: number;
  status: RunStatus;
  message: string;
}

function runApprovedBatch(
  planId: string,
  approval: string,
): Promise<{ status: RunStatus; errorMessage: string | null }> {
  return new Promise((resolve, reject) => {
    let errorMessage: string | null = null;
    client
      .execute(planId, approval, (event: QueryEvent) => {
        if (event.type === "error") {
          errorMessage = event.error.message;
        } else if (event.type === "finished") {
          resolve({ status: event.status, errorMessage });
        }
      })
      .catch(reject);
  });
}

export function ImportCsvDialog({
  table,
  schema,
  profile,
  session,
  onImported,
  onClose,
}: {
  table: TableRef;
  schema: TableSchema | null;
  profile?: ConnectionProfile;
  session?: SessionInfo;
  onImported(): void;
  onClose(): void;
}) {
  const cachedSchemaRef = useRef<TableSchema | null>(schema);
  if (schema) {
    cachedSchemaRef.current = schema;
  }
  const activeSchema = schema ?? cachedSchemaRef.current;
  const [selection, setSelection] = useState<FileSelection | null>(null);
  const [options, setOptions] = useState<CsvImportOptions>({
    delimiter: "comma",
    hasHeader: true,
    nullToken: "\\N",
  });
  const [mappings, setMappings] = useState<CsvColumnMapping[]>([]);
  const [preview, setPreview] = useState<CsvImportPreview | null>(null);
  const [busyMode, setBusyMode] = useState<"idle" | "picking" | "previewing" | "preparing">("idle");
  const [pendingBatch, setPendingBatch] = useState<PendingBatch | null>(null);
  const [completedBatches, setCompletedBatches] = useState(0);
  const [committedRows, setCommittedRows] = useState(0);
  const [batchFailure, setBatchFailure] = useState<BatchFailure | null>(null);
  const [allCompleted, setAllCompleted] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const previewSeqRef = useRef(0);

  async function loadPreview(
    chosen: FileSelection,
    nextOptions: CsvImportOptions,
    nextMappings: CsvColumnMapping[],
  ) {
    if (!session) {
      setError("Connect to the database before previewing a CSV import.");
      return;
    }
    const seq = ++previewSeqRef.current;
    setBusyMode("previewing");
    setError(null);
    setBatchFailure(null);
    setAllCompleted(false);
    setCompletedBatches(0);
    setCommittedRows(0);
    try {
      const loaded = await client.call<CsvImportPreview>("preview_csv_import", {
        sessionId: session.id,
        sourceToken: chosen.token,
        table,
        options: nextOptions,
        mappings: nextMappings,
      });
      if (seq !== previewSeqRef.current) return;
      setPreview(loaded);
      setMappings(loaded.mappings);
      setBusyMode("idle");
    } catch (err) {
      if (seq !== previewSeqRef.current) return;
      setPreview(null);
      setBusyMode("idle");
      setError(messageOf(err));
    }
  }

  async function handleChooseFile() {
    if (!desktopAvailable) {
      setError("CSV file import is available in the desktop app.");
      return;
    }
    setBusyMode("picking");
    setError(null);
    try {
      const picked = await client.call<FileSelection | null>("pick_import_source");
      setBusyMode("idle");
      if (!picked) return;
      const inferredDelimiter: CsvDelimiter = picked.fileName.toLowerCase().endsWith(".tsv")
        ? "tab"
        : options.delimiter;
      const nextOptions: CsvImportOptions = {
        ...options,
        delimiter: inferredDelimiter,
      };
      setSelection(picked);
      setOptions(nextOptions);
      setMappings([]);
      await loadPreview(picked, nextOptions, []);
    } catch (err) {
      setBusyMode("idle");
      setError(messageOf(err));
    }
  }

  async function handleOptionChange(nextOptions: CsvImportOptions, resetMappings: boolean) {
    setOptions(nextOptions);
    const nextMappings = resetMappings ? [] : mappings;
    if (resetMappings) {
      setMappings([]);
    }
    if (selection) {
      await loadPreview(selection, nextOptions, nextMappings);
    }
  }

  async function handleMappingChange(csvIndex: number, targetColumn: string) {
    const updated = mappings.map((entry) =>
      entry.csvIndex === csvIndex
        ? { ...entry, targetColumn: targetColumn ? targetColumn : null }
        : entry,
    );
    setMappings(updated);
    if (selection) {
      await loadPreview(selection, options, updated);
    }
  }

  async function prepareBatch(batchIndex: number) {
    if (!session || !selection || !preview?.canPrepare) return;
    setBusyMode("preparing");
    setError(null);
    setBatchFailure(null);
    try {
      const changePreview = await client.call<ChangePreview>("prepare_csv_import_batch", {
        sessionId: session.id,
        sourceToken: selection.token,
        table,
        options,
        mappings,
        batchIndex,
      });
      setBusyMode("idle");
      setPendingBatch({ batchIndex, preview: changePreview });
    } catch (err) {
      setBusyMode("idle");
      setError(messageOf(err));
    }
  }

  if (pendingBatch) {
    const activeBatch = pendingBatch;
    const totalBatches = preview?.batchCount ?? 1;
    return (
      <ReviewDialog
        key={activeBatch.preview.plan.id}
        plan={activeBatch.preview.plan}
        preview={activeBatch.preview}
        profile={profile}
        onExecute={async (approval) => {
          const outcome = await runApprovedBatch(activeBatch.preview.plan.id, approval);
          if (outcome.status === "completed") {
            const nextCompleted = activeBatch.batchIndex + 1;
            const nextCommittedRows = committedRows + activeBatch.preview.rowCount;
            setCompletedBatches(nextCompleted);
            setCommittedRows(nextCommittedRows);
            onImported();
            if (nextCompleted < totalBatches && session && selection) {
              try {
                const nextPreview = await client.call<ChangePreview>("prepare_csv_import_batch", {
                  sessionId: session.id,
                  sourceToken: selection.token,
                  table,
                  options,
                  mappings,
                  batchIndex: nextCompleted,
                });
                setPendingBatch({ batchIndex: nextCompleted, preview: nextPreview });
              } catch (err) {
                setError(messageOf(err));
              }
            } else {
              setAllCompleted(true);
            }
            return;
          }

          setBatchFailure({
            batchIndex: activeBatch.batchIndex,
            status: outcome.status,
            message:
              outcome.errorMessage ??
              (outcome.status === "outcomeUnknown"
                ? "The connection closed while committing. Inspect the table before retrying this batch."
                : outcome.status === "cancelled"
                  ? "This import batch was cancelled and rolled back."
                  : "This import batch failed and was rolled back atomically."),
          });
        }}
        onClose={() =>
          setPendingBatch((curr) => (curr?.batchIndex === activeBatch.batchIndex ? null : curr))
        }
      />
    );
  }

  const busy = busyMode !== "idle";
  const outcomeUnknownLocked = batchFailure?.status === "outcomeUnknown";
  const samplePreviewRows = preview ? preview.sampleRows.slice(0, 8) : [];

  return (
    <Modal
      title="Import CSV into table"
      subtitle={`${table.schema}.${table.name} · ${profile?.name ?? "Database"}`}
      onClose={onClose}
      wide
      busy={busy}
    >
      <div className="modal-body import-dialog-body">
        <div className="export-destination-row">
          <div>
            <span className="export-destination-label">CSV source file</span>
            <strong className="export-destination-name">
              {selection
                ? selection.displayLabel
                : "Choose a UTF-8 CSV or TSV file (up to 16 MiB / 5,000 rows)"}
            </strong>
          </div>
          <button
            type="button"
            className="button ghost small"
            disabled={busy || outcomeUnknownLocked}
            onClick={() => void handleChooseFile()}
          >
            <FolderOpen size={13} />
            {selection ? "Change file…" : "Choose CSV file…"}
          </button>
        </div>

        <div className="form-grid">
          <label>
            Delimiter
            <select
              aria-label="CSV delimiter"
              value={options.delimiter}
              disabled={busy || outcomeUnknownLocked}
              onChange={(event) =>
                void handleOptionChange(
                  { ...options, delimiter: event.target.value as CsvDelimiter },
                  true,
                )
              }
            >
              {DELIMITERS.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.label}
                </option>
              ))}
            </select>
          </label>

          <label>
            NULL token (unquoted)
            <input
              aria-label="NULL token"
              value={options.nullToken}
              disabled={busy || outcomeUnknownLocked}
              placeholder="\\N"
              onChange={(event) =>
                void handleOptionChange({ ...options, nullToken: event.target.value }, false)
              }
            />
          </label>

          <label className="checkbox-label span-2">
            <input
              type="checkbox"
              checked={options.hasHeader}
              disabled={busy || outcomeUnknownLocked}
              onChange={(event) =>
                void handleOptionChange({ ...options, hasHeader: event.target.checked }, true)
              }
            />
            <span>
              First row contains column headers · unquoted empty fields import as <code>NULL</code>{" "}
              while quoted <code>&quot;&quot;</code> stays an empty string
            </span>
          </label>
        </div>

        {busyMode === "previewing" ? (
          <div className="export-progress-card" role="status" aria-live="polite">
            <div className="export-progress-head">
              <Loader2 size={15} className="spin" />
              <strong>Validating CSV rows against {table.name}…</strong>
            </div>
          </div>
        ) : null}

        {preview ? (
          <>
            <div className="import-summary-bar" role="status" aria-live="polite">
              <span>
                <FileSpreadsheet size={13} />
                <strong>{preview.totalRows.toLocaleString()}</strong> row
                {preview.totalRows === 1 ? "" : "s"} · <strong>{preview.batchCount}</strong>{" "}
                reviewed batch
                {preview.batchCount === 1 ? "" : "es"} (up to {preview.batchSize} rows/transaction)
              </span>
              {preview.unmappedDefaults.length > 0 ? (
                <span className="import-pill">
                  &lt;default&gt;: {preview.unmappedDefaults.join(", ")}
                </span>
              ) : null}
              {preview.unmappedNulls.length > 0 ? (
                <span className="import-pill muted">
                  Unmapped → NULL: {preview.unmappedNulls.join(", ")}
                </span>
              ) : null}
            </div>

            {preview.headers.length > 0 ? (
              <div className="import-mapping-section">
                <div className="import-section-title">Column mapping</div>
                <div className="import-mapping-list">
                  {preview.headers.map((header, csvIndex) => {
                    const currentTarget =
                      mappings.find((m) => m.csvIndex === csvIndex)?.targetColumn ?? "";
                    const sampleCell = preview.sampleRows[0]?.[csvIndex];
                    return (
                      <div className="import-mapping-row" key={`${csvIndex}-${header}`}>
                        <div className="import-csv-col">
                          <strong>{header}</strong>
                          <span className="import-sample-chip">
                            {sampleCell === undefined
                              ? "—"
                              : sampleCell === null
                                ? "NULL"
                                : sampleCell === ""
                                  ? '"" (empty)'
                                  : sampleCell.slice(0, 36)}
                          </span>
                        </div>
                        <select
                          aria-label={`Target column for ${header}`}
                          value={currentTarget}
                          disabled={busy || outcomeUnknownLocked || completedBatches > 0}
                          onChange={(event) =>
                            void handleMappingChange(csvIndex, event.target.value)
                          }
                        >
                          <option value="">&lt;skip&gt;</option>
                          {(activeSchema?.columns ?? []).map((col) => (
                            <option key={col.id} value={col.name} disabled={!col.insertable}>
                              {col.name} ({col.databaseType})
                              {!col.insertable
                                ? " · read-only"
                                : !col.nullable && !col.hasDefault
                                  ? " · required"
                                  : col.hasDefault
                                    ? " · default"
                                    : ""}
                            </option>
                          ))}
                        </select>
                      </div>
                    );
                  })}
                </div>
              </div>
            ) : null}

            {preview.issues.length > 0 ? (
              <div className="import-issues-panel" role="alert">
                <div className="import-issues-header">
                  <AlertTriangle size={14} />
                  <strong>
                    {preview.totalIssues.toLocaleString()} validation issue
                    {preview.totalIssues === 1 ? "" : "s"} must be resolved before importing
                  </strong>
                </div>
                <ul className="import-issues-list">
                  {preview.issues.slice(0, 25).map((issue, idx) => (
                    <li key={`${issue.line}-${issue.column ?? "row"}-${idx}`}>
                      <code>
                        Line {issue.line}
                        {issue.column ? ` · ${issue.column}` : ""}
                      </code>
                      <span>{issue.message}</span>
                    </li>
                  ))}
                </ul>
                {preview.totalIssues > 25 ? (
                  <p className="import-issues-more">
                    Showing first 25 of {preview.totalIssues.toLocaleString()} issues.
                  </p>
                ) : null}
              </div>
            ) : null}

            {samplePreviewRows.length > 0 ? (
              <div className="import-sample-section">
                <div className="import-section-title">
                  Sample rows (showing {samplePreviewRows.length} of{" "}
                  {preview.totalRows.toLocaleString()})
                </div>
                <div className="import-sample-scroll">
                  <table className="import-sample-table">
                    <thead>
                      <tr>
                        {preview.headers.map((header, idx) => {
                          const target =
                            mappings.find((m) => m.csvIndex === idx)?.targetColumn ?? null;
                          return (
                            <th key={`${idx}-${header}`}>
                              <div>{header}</div>
                              <small>{target ? `→ ${target}` : "<skip>"}</small>
                            </th>
                          );
                        })}
                      </tr>
                    </thead>
                    <tbody>
                      {samplePreviewRows.map((row, rIdx) => (
                        <tr key={rIdx}>
                          {preview.headers.map((_, cIdx) => {
                            const val = row[cIdx];
                            return (
                              <td key={cIdx}>
                                {val === null || val === undefined ? (
                                  <em className="cell-token">NULL</em>
                                ) : val === "" ? (
                                  <em className="cell-token">&quot;&quot;</em>
                                ) : (
                                  val.slice(0, 80)
                                )}
                              </td>
                            );
                          })}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            ) : null}
          </>
        ) : null}

        {batchFailure ? (
          <div className="export-status-card incomplete" role="alert">
            <div className="export-progress-head">
              <AlertTriangle size={15} />
              <strong>
                Batch {batchFailure.batchIndex + 1} of {preview?.batchCount ?? 1}{" "}
                {batchFailure.status === "outcomeUnknown"
                  ? "commit outcome unknown — import halted"
                  : batchFailure.status === "cancelled"
                    ? "cancelled and rolled back"
                    : "failed and rolled back atomically"}
              </strong>
            </div>
            <p>
              {batchFailure.message}{" "}
              {committedRows > 0
                ? `${committedRows.toLocaleString()} rows from earlier batches remain committed.`
                : "No rows from this batch were committed."}
            </p>
          </div>
        ) : null}

        {allCompleted ? (
          <div className="export-status-card completed" role="status">
            <div className="export-progress-head">
              <Check size={15} />
              <strong>
                Imported {committedRows.toLocaleString()} row{committedRows === 1 ? "" : "s"} into{" "}
                {table.schema}.{table.name}
              </strong>
            </div>
            <p>
              All {completedBatches} reviewed transaction batch
              {completedBatches === 1 ? "" : "es"} committed cleanly.
            </p>
          </div>
        ) : completedBatches > 0 && preview && completedBatches < preview.batchCount ? (
          <div className="export-progress-card" role="status">
            <div className="export-progress-head">
              <Check size={15} />
              <strong>
                Committed {completedBatches} of {preview.batchCount} batches (
                {committedRows.toLocaleString()} rows)
              </strong>
            </div>
            <p>Review and authorize the next batch to continue importing.</p>
          </div>
        ) : null}

        <p className="form-note">
          Every 500-row batch is reviewed through Safe Mode and committed inside its own{" "}
          <code>BEGIN … COMMIT</code> transaction with zero automatic retries.
        </p>
        <InlineError message={error} />
      </div>

      <footer className="modal-footer">
        <button type="button" className="button ghost" disabled={busy} onClick={onClose}>
          {allCompleted ? "Done" : "Close"}
        </button>
        {!allCompleted ? (
          <button
            type="button"
            className="button primary"
            disabled={
              busy ||
              !selection ||
              !preview?.canPrepare ||
              outcomeUnknownLocked ||
              (preview !== null && completedBatches >= preview.batchCount)
            }
            onClick={() => void prepareBatch(completedBatches)}
          >
            {busyMode === "preparing" ? (
              <Loader2 size={14} className="spin" />
            ) : completedBatches > 0 ? (
              <Play size={14} />
            ) : (
              <Upload size={14} />
            )}
            {preview && preview.batchCount > 1
              ? `Review batch ${completedBatches + 1} of ${preview.batchCount}…`
              : preview
                ? `Review & import ${preview.totalRows.toLocaleString()} row${preview.totalRows === 1 ? "" : "s"}…`
                : "Review & import…"}
          </button>
        ) : null}
      </footer>
    </Modal>
  );
}
