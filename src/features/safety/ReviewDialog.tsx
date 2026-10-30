import { useState, type FormEvent } from "react";
import { AlertTriangle, LockKeyhole, Play, Loader2 } from "lucide-react";
import { Modal, InlineError } from "../../components/ui/Modal";
import { client, messageOf } from "../../ipc/client";
import type { ChangePreview, ConnectionProfile, PreparedPlan } from "../../ipc/generated";

export function ReviewDialog({
  plan,
  preview,
  profile,
  onExecute,
  onClose,
}: {
  plan: PreparedPlan;
  preview?: ChangePreview;
  profile?: ConnectionProfile;
  onExecute(approval: string): Promise<void>;
  onClose(): void;
}) {
  const [password, setPassword] = useState(""),
    [confirmation, setConfirmation] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null);
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const approval = await client.call<string>("authorize_plan", {
        planId: plan.id,
        passphrase: password || null,
        confirmation: confirmation || null,
      });
      setPassword("");
      await onExecute(approval);
      onClose();
    } catch (error) {
      setError(messageOf(error));
      setBusy(false);
      setPassword("");
    }
  }
  return (
    <Modal
      title={preview ? "Review pending changes" : "Review this operation"}
      subtitle={`${profile?.name ?? "Connection"} · ${profile?.database ?? ""}`}
      onClose={onClose}
      wide
      busy={busy}
    >
      <form onSubmit={(e) => void submit(e)}>
        <div className="modal-body">
          <div className="review-summary">
            <span className={`environment ${profile?.environment ?? "local"}`}>
              {profile?.environment ?? "local"}
            </span>
            <span>
              {preview
                ? `${preview.rowCount} row${preview.rowCount === 1 ? "" : "s"} · one transaction`
                : `${plan.statements.length} statement${plan.statements.length === 1 ? "" : "s"}`}
            </span>
            <span className="review-exact">Exact batch approval</span>
          </div>
          {plan.warnings.map((warning) => (
            <div className="safety-warning" key={warning}>
              <AlertTriangle size={15} />
              <span>{warning}</span>
            </div>
          ))}
          <pre className="sql-preview">
            {preview ? preview.sql.join(";\n\n") : plan.statements.map((s) => s.sql).join(";\n\n")}
          </pre>
          {preview && (
            <details className="bindings">
              <summary>Bound values</summary>
              {preview.bindings.map((values, i) => (
                <div key={i}>
                  <span>Row {i + 1}</span>
                  <code>
                    {values.length
                      ? values
                          .map(
                            (value, j) =>
                              `$${j + 1} = ${value === null ? "NULL" : JSON.stringify(value)}`,
                          )
                          .join("  ·  ")
                      : "(default values)"}
                  </code>
                </div>
              ))}
            </details>
          )}
          {plan.confirmationText && (
            <label className="review-field">
              Type <strong>{plan.confirmationText}</strong> to confirm
              <input
                value={confirmation}
                onChange={(e) => setConfirmation(e.target.value)}
                autoComplete="off"
                spellCheck={false}
                required
              />
            </label>
          )}
          {plan.requiresPassphrase && (
            <label className="review-field">
              <span>
                <LockKeyhole size={14} /> Application write passphrase
              </span>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="current-password"
                required
              />
            </label>
          )}
          <p className="form-note">
            Approval expires after 60 seconds. Changes to the query require another review.
          </p>
          <InlineError message={error} />
        </div>
        <footer className="modal-footer">
          <button type="button" className="button ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button
            className="button primary"
            type="submit"
            disabled={busy || (!!plan.confirmationText && confirmation !== plan.confirmationText)}
          >
            {busy ? <Loader2 size={14} className="spin" /> : <Play size={14} />}
            {preview ? "Apply changes" : "Authorize and run"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
