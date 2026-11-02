import { useState } from "react";
import { InlineError, Modal } from "../../components/ui/Modal";
import { messageOf } from "../../ipc/client";
import type { EffectiveSqlSnippet } from "./sql-snippets";

export function SqlTemplateDialog({
  templates,
  onInsert,
  onClose,
}: {
  templates: EffectiveSqlSnippet[];
  onInsert(body: string): Promise<void>;
  onClose(): void;
}) {
  const [trigger, setTrigger] = useState(templates[0]?.trigger ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selected = templates.find((template) => template.trigger === trigger);
  return (
    <div onKeyDown={(event) => event.stopPropagation()}>
      <Modal
        title="Insert SQL template"
        subtitle="Replace the selection or insert at the cursor. Tab moves between editable fields."
        onClose={onClose}
        wide
      >
        <div className="modal-body">
          <label>
            Template
            <select
              aria-label="SQL template"
              autoFocus
              value={trigger}
              disabled={busy}
              onChange={(event) => {
                setTrigger(event.target.value);
                setError(null);
              }}
            >
              {templates.map((template) => (
                <option key={template.trigger} value={template.trigger}>
                  {template.trigger} · {template.title}
                </option>
              ))}
            </select>
          </label>
          <pre
            aria-label="SQL template body"
            style={{
              whiteSpace: "pre-wrap",
              overflowWrap: "anywhere",
              maxHeight: 320,
              overflow: "auto",
            }}
          >
            {selected?.body}
          </pre>
          <p className="form-note">
            Database and schema variables use the connected console. Selection variables keep your
            selected SQL text.
          </p>
          <InlineError message={error} />
        </div>
        <footer className="modal-footer">
          <button type="button" className="button ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="button primary"
            disabled={busy || !selected}
            onClick={() => {
              if (!selected) return;
              setBusy(true);
              setError(null);
              void onInsert(selected.body).catch((cause) => {
                setError(messageOf(cause));
                setBusy(false);
              });
            }}
          >
            {busy ? "Inserting…" : "Insert template"}
          </button>
        </footer>
      </Modal>
    </div>
  );
}
