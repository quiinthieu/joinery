import { useState } from "react";
import type { ChangeSpec, EditorState } from "@codemirror/state";
import { InlineError, Modal } from "../../components/ui/Modal";
import { messageOf } from "../../ipc/client";
import type { ExtractCtePlan } from "./sql-refactor";

export type SqlRefactorModalMode =
  | {
      kind: "extractCte";
      plan: ExtractCtePlan;
      state: EditorState;
    }
  | {
      kind: "expandWildcard";
      wildcardText: string;
      from: number;
      to: number;
      insert: string;
      columnCount: number;
      previewSql: string;
    };

export function SqlRefactorDialog({
  mode,
  onApplyChanges,
  onClose,
}: {
  mode: SqlRefactorModalMode;
  onApplyChanges(changes: ChangeSpec | readonly ChangeSpec[], userEvent: string): void;
  onClose(): void;
}) {
  const [cteName, setCteName] = useState(mode.kind === "extractCte" ? mode.plan.defaultName : "");

  if (mode.kind === "expandWildcard") {
    return (
      <div onKeyDown={(event) => event.stopPropagation()}>
        <Modal
          title="Expand SQL wildcard"
          subtitle={`Preview replacing ${mode.wildcardText} with ${mode.columnCount} explicit column${mode.columnCount === 1 ? "" : "s"}. No SQL is executed.`}
          onClose={onClose}
          wide
        >
          <div className="modal-body" style={{ display: "grid", gap: 10 }}>
            <div>
              <span className="eyebrow">EXPANDED PROJECTION ({mode.columnCount} COLUMNS)</span>
              <pre
                className="mono"
                aria-label="Expanded columns preview"
                style={{
                  whiteSpace: "pre-wrap",
                  overflowWrap: "anywhere",
                  maxHeight: 140,
                  overflow: "auto",
                  margin: "6px 0 0",
                }}
              >
                {mode.insert}
              </pre>
            </div>
            <div>
              <span className="eyebrow">STATEMENT PREVIEW</span>
              <pre
                className="mono"
                aria-label="Wildcard statement preview"
                style={{
                  whiteSpace: "pre-wrap",
                  overflowWrap: "anywhere",
                  maxHeight: 240,
                  overflow: "auto",
                  margin: "6px 0 0",
                }}
              >
                {mode.previewSql}
              </pre>
            </div>
          </div>
          <footer className="modal-footer">
            <button type="button" className="button ghost" onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              className="button primary"
              onClick={() =>
                onApplyChanges(
                  { from: mode.from, to: mode.to, insert: mode.insert },
                  "input.expand-wildcard",
                )
              }
            >
              Expand wildcard
            </button>
          </footer>
        </Modal>
      </div>
    );
  }

  let error: string | null = null;
  let previewSql = "";
  let changes: ChangeSpec[] = [];
  try {
    const res = mode.plan.preview(mode.state, cteName);
    previewSql = res.previewSql;
    changes = res.changes;
  } catch (cause) {
    error = messageOf(cause);
  }

  return (
    <div onKeyDown={(event) => event.stopPropagation()}>
      <Modal
        title="Extract SQL CTE"
        subtitle="Preview extracting the selected query into a WITH Common Table Expression. No SQL is executed."
        onClose={onClose}
        wide
      >
        <div className="modal-body" style={{ display: "grid", gap: 10 }}>
          <label>
            CTE name
            <input
              aria-label="CTE name"
              autoFocus
              value={cteName}
              spellCheck={false}
              autoComplete="off"
              onChange={(event) => setCteName(event.target.value)}
            />
          </label>
          <InlineError message={error} />
          {!error && (
            <div>
              <span className="eyebrow">EXTRACTED STATEMENT PREVIEW</span>
              <pre
                className="mono"
                aria-label="Extract CTE preview"
                style={{
                  whiteSpace: "pre-wrap",
                  overflowWrap: "anywhere",
                  maxHeight: 280,
                  overflow: "auto",
                  margin: "6px 0 0",
                }}
              >
                {previewSql}
              </pre>
            </div>
          )}
        </div>
        <footer className="modal-footer">
          <button type="button" className="button ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="button primary"
            disabled={Boolean(error)}
            onClick={() => onApplyChanges(changes, "input.extract-cte")}
          >
            Extract CTE
          </button>
        </footer>
      </Modal>
    </div>
  );
}
