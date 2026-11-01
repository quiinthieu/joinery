import { useState } from "react";
import type { EditorState } from "@codemirror/state";
import { InlineError, Modal } from "../../components/ui/Modal";
import { messageOf } from "../../ipc/client";
import type { AliasRenameTarget } from "./sql-navigation";

export function SqlAliasRenameDialog({
  target,
  state,
  onApply,
  onClose,
}: {
  target: AliasRenameTarget;
  state: EditorState;
  onApply(name: string): void;
  onClose(): void;
}) {
  const [name, setName] = useState(target.name);
  let error: string | null = null;
  let changes: ReturnType<AliasRenameTarget["preview"]> = [];
  try {
    changes = target.preview(state, name);
  } catch (cause) {
    error = messageOf(cause);
  }
  return (
    <div onKeyDown={(event) => event.stopPropagation()}>
      <Modal
        title="Rename SQL alias"
        subtitle="Preview local SQL edits, then Apply. No SQL is executed."
        onClose={onClose}
        wide
      >
        <div className="modal-body">
          <label>
            New alias
            <input
              aria-label="New alias"
              autoFocus
              value={name}
              spellCheck={false}
              autoComplete="off"
              onChange={(event) => setName(event.target.value)}
            />
          </label>
          <InlineError message={error} />
          <p>{target.occurrences.length} local occurrences</p>
          <div style={{ maxHeight: 320, overflow: "auto" }} aria-label="Alias rename preview">
            {target.occurrences.map((occurrence, index) => (
              <div key={occurrence.from} style={{ marginBottom: 12 }}>
                <div>
                  Line {occurrence.line}, column {occurrence.column}
                  {occurrence.isDeclaration ? " · declaration" : ""}
                </div>
                <code>
                  {state.sliceDoc(occurrence.from, occurrence.to)} → {changes[index]?.insert ?? "…"}
                </code>
                <pre style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere", margin: "4px 0" }}>
                  {occurrence.lineText}
                </pre>
              </div>
            ))}
          </div>
        </div>
        <footer className="modal-footer">
          <button type="button" className="button ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="button primary"
            disabled={!!error}
            onClick={() => onApply(name)}
          >
            Apply rename
          </button>
        </footer>
      </Modal>
    </div>
  );
}
