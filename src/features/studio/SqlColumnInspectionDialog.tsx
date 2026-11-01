import { useState } from "react";
import type { EditorState } from "@codemirror/state";
import { InlineError, Modal } from "../../components/ui/Modal";
import { messageOf } from "../../ipc/client";
import type { MissingSqlColumn, SqlColumnInspection } from "./sql-inspections";

export function SqlColumnInspectionDialog({
  result,
  state,
  onApply,
  onClose,
}: {
  result: SqlColumnInspection;
  state: EditorState;
  onApply(issue: MissingSqlColumn, column: string): void;
  onClose(): void;
}) {
  const [index, setIndex] = useState(0);
  const [column, setColumn] = useState("");
  const issue = result.issues[index];
  let preview: ReturnType<SqlColumnInspection["preview"]> | null = null;
  let error: string | null = null;
  if (issue && column) {
    try {
      preview = result.preview(state, issue, column);
    } catch (cause) {
      error = messageOf(cause);
    }
  }
  return (
    <div onKeyDown={(event) => event.stopPropagation()}>
      <Modal
        title="SQL column inspection"
        subtitle="Checks qualified columns in this direct-table SELECT. Choose a correction, preview it, then Apply. Your SQL is not run."
        onClose={onClose}
        wide
      >
        <div className="modal-body">
          <p>
            {result.tables.length} {result.tables.length === 1 ? "table" : "tables"} ·{" "}
            {result.checked} qualified column references checked
          </p>
          {issue ? (
            <>
              <p>
                {result.issues.length} missing column{" "}
                {result.issues.length === 1 ? "reference" : "references"} in the inspected metadata.
              </p>
              <label>
                Missing reference
                <select
                  aria-label="Missing reference"
                  autoFocus
                  value={index}
                  onChange={(event) => {
                    setIndex(Number(event.target.value));
                    setColumn("");
                  }}
                >
                  {result.issues.map((item, i) => (
                    <option key={item.from} value={i}>
                      Line {item.line}: {item.qualifier}.{item.name} ({item.table.schema}.
                      {item.table.name})
                    </option>
                  ))}
                </select>
              </label>
              <p>
                Column “{issue.name}” was not found in the complete selectable metadata for{" "}
                {issue.table.schema}.{issue.table.name}.
              </p>
              <label>
                Replace with catalog column
                <select
                  aria-label="Replace with catalog column"
                  value={column}
                  onChange={(event) => setColumn(event.target.value)}
                >
                  <option value="">Choose a column…</option>
                  {result.columnsFor(issue).map((name) => (
                    <option key={name} value={name}>
                      {name}
                    </option>
                  ))}
                </select>
              </label>
              <InlineError message={error} />
              <div
                aria-label="Column correction preview"
                style={{ marginTop: 12, overflowWrap: "anywhere" }}
              >
                <code>
                  {state.sliceDoc(issue.from, issue.to)} → {preview?.insert ?? "Choose a column"}
                </code>
                {preview && (
                  <pre style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>
                    {state.doc
                      .lineAt(issue.from)
                      .text.slice(0, issue.from - state.doc.lineAt(issue.from).from)}
                    {preview.insert}
                    {state.doc
                      .lineAt(issue.to)
                      .text.slice(issue.to - state.doc.lineAt(issue.to).from)}
                  </pre>
                )}
              </div>
              <p>Applies to this occurrence. Run the inspection again after editing.</p>
            </>
          ) : (
            <p>
              No missing columns among the qualified references checked. This does not validate the
              full SQL statement.
            </p>
          )}
        </div>
        <footer className="modal-footer">
          <button type="button" className="button ghost" onClick={onClose}>
            {issue ? "Cancel" : "Close"}
          </button>
          {issue && (
            <button
              type="button"
              className="button primary"
              disabled={!preview || !!error}
              onClick={() => onApply(issue, column)}
            >
              Apply column correction
            </button>
          )}
        </footer>
      </Modal>
    </div>
  );
}
