import { useState, type FormEvent } from "react";
import { AlertTriangle, LockKeyhole, Play, Loader2, Columns, FileCode } from "lucide-react";
import { Modal, InlineError } from "../../components/ui/Modal";
import { client, messageOf } from "../../ipc/client";
import type { ChangePreview, ConnectionProfile, PreparedPlan } from "../../ipc/generated";
import { VisualDiffViewer } from "./VisualDiffViewer";
import type { RowDiff } from "./mutation-diff";

export interface ReviewDialogProps {
  plan: PreparedPlan;
  preview?: ChangePreview;
  profile?: ConnectionProfile;
  diffRows?: RowDiff[];
  onRecomputePlan?(
    selectedIds: Set<string>,
  ): Promise<{ plan: PreparedPlan; preview: ChangePreview }>;
  onDiscardRow?(row: RowDiff): void;
  onExecute(approval: string): Promise<void>;
  onClose(): void;
}

export function ReviewDialog({
  plan,
  preview,
  profile,
  diffRows,
  onRecomputePlan,
  onDiscardRow,
  onExecute,
  onClose,
}: ReviewDialogProps) {
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [recomputing, setRecomputing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [activeTab, setActiveTab] = useState<"diff" | "sql">(
    diffRows && diffRows.length > 0 ? "diff" : "sql",
  );
  const [selectedIds, setSelectedIds] = useState<Set<string>>(
    () => new Set(diffRows?.map((r) => r.id) ?? []),
  );
  const [currentPlan, setCurrentPlan] = useState<PreparedPlan>(plan);
  const [currentPreview, setCurrentPreview] = useState<ChangePreview | undefined>(preview);
  const [diffLayout, setDiffLayout] = useState<"unified" | "split">("unified");
  const [changedColsOnly, setChangedColsOnly] = useState(true);

  const totalDiffCount = diffRows?.length ?? 0;
  const selectedDiffCount = selectedIds.size;

  const handleToggle = async (rowId: string) => {
    const next = new Set(selectedIds);
    if (next.has(rowId)) next.delete(rowId);
    else next.add(rowId);
    setSelectedIds(next);

    if (onRecomputePlan) {
      setRecomputing(true);
      try {
        const updated = await onRecomputePlan(next);
        setCurrentPlan(updated.plan);
        setCurrentPreview(updated.preview);
      } catch (err) {
        setError(messageOf(err));
      } finally {
        setRecomputing(false);
      }
    }
  };

  const handleToggleAll = async (select: boolean) => {
    const next = new Set(select ? (diffRows?.map((r) => r.id) ?? []) : []);
    setSelectedIds(next);

    if (onRecomputePlan) {
      setRecomputing(true);
      try {
        const updated = await onRecomputePlan(next);
        setCurrentPlan(updated.plan);
        setCurrentPreview(updated.preview);
      } catch (err) {
        setError(messageOf(err));
      } finally {
        setRecomputing(false);
      }
    }
  };

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const approval = await client.call<string>("authorize_plan", {
        planId: currentPlan.id,
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

  const isApplyDisabled =
    busy ||
    recomputing ||
    (diffRows && diffRows.length > 0 && selectedDiffCount === 0) ||
    (!!currentPlan.confirmationText && confirmation !== currentPlan.confirmationText);

  return (
    <Modal
      title={currentPreview ? "Review pending changes" : "Review this operation"}
      subtitle={`${profile?.name ?? "Connection"} · ${profile?.database ?? ""}`}
      onClose={onClose}
      wide
      busy={busy}
    >
      <form onSubmit={(e) => void submit(e)}>
        <div className="modal-body flex flex-col gap-3">
          {/* Summary pill banner */}
          <div className="review-summary">
            <span className={`environment ${profile?.environment ?? "local"}`}>
              {profile?.environment ?? "local"}
            </span>
            <span>
              {currentPreview
                ? `${currentPreview.rowCount} row${currentPreview.rowCount === 1 ? "" : "s"} · one transaction`
                : `${currentPlan.statements.length} statement${currentPlan.statements.length === 1 ? "" : "s"}`}
            </span>
            <span className="review-exact">Exact batch approval</span>
          </div>

          {currentPlan.warnings.map((warning) => (
            <div className="safety-warning" key={warning}>
              <AlertTriangle size={15} />
              <span>{warning}</span>
            </div>
          ))}

          {/* Dual view tabs if diffRows are provided */}
          {diffRows && diffRows.length > 0 && (
            <div className="flex items-center gap-1 border-b border-border pb-1">
              <button
                type="button"
                className={`flex items-center gap-1.5 px-3 py-1.5 rounded-t text-xs font-medium transition-colors ${
                  activeTab === "diff"
                    ? "bg-bg-elevated text-text border-b-2 border-accent"
                    : "text-muted hover:text-text"
                }`}
                onClick={() => setActiveTab("diff")}
              >
                <Columns size={13} />
                Visual Diff ({selectedDiffCount}/{totalDiffCount})
              </button>
              <button
                type="button"
                className={`flex items-center gap-1.5 px-3 py-1.5 rounded-t text-xs font-medium transition-colors ${
                  activeTab === "sql"
                    ? "bg-bg-elevated text-text border-b-2 border-accent"
                    : "text-muted hover:text-text"
                }`}
                onClick={() => setActiveTab("sql")}
              >
                <FileCode size={13} />
                SQL Script ({currentPreview?.rowCount ?? currentPlan.statements.length})
              </button>
            </div>
          )}

          {/* Tab content */}
          {activeTab === "diff" && diffRows && diffRows.length > 0 ? (
            <VisualDiffViewer
              diffRows={diffRows}
              selectedIds={selectedIds}
              onToggle={handleToggle}
              onToggleAll={handleToggleAll}
              layout={diffLayout}
              onLayoutChange={setDiffLayout}
              changedColsOnly={changedColsOnly}
              onChangedColsOnlyChange={setChangedColsOnly}
              onDiscardRow={onDiscardRow}
            />
          ) : (
            <div className="flex flex-col gap-2">
              <pre className="sql-preview">
                {currentPreview
                  ? currentPreview.sql.join(";\n\n")
                  : currentPlan.statements.map((s) => s.sql).join(";\n\n")}
              </pre>

              {(currentPlan.boundParameters?.length ?? 0) > 0 && (
                <details className="bindings" open aria-label="Bound query parameters">
                  <summary>Bound parameters ({currentPlan.boundParameters!.length})</summary>
                  {currentPlan.boundParameters!.map((param) => (
                    <div key={param.key}>
                      <span>
                        {param.key.startsWith("$") || param.key.startsWith("?")
                          ? param.key
                          : `:${param.key}`}{" "}
                        <small>({param.kind})</small>
                      </span>
                      <code>{param.value === null ? "NULL" : JSON.stringify(param.value)}</code>
                    </div>
                  ))}
                </details>
              )}

              {currentPreview && (
                <details className="bindings">
                  <summary>Bound values</summary>
                  {currentPreview.bindings.map((values, i) => (
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
            </div>
          )}

          {currentPlan.confirmationText && (
            <label className="review-field">
              Type <strong>{currentPlan.confirmationText}</strong> to confirm
              <input
                value={confirmation}
                onChange={(e) => setConfirmation(e.target.value)}
                autoComplete="off"
                spellCheck={false}
                required
              />
            </label>
          )}

          {currentPlan.requiresPassphrase && (
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
          <button className="button primary" type="submit" disabled={isApplyDisabled}>
            {busy || recomputing ? <Loader2 size={14} className="spin" /> : <Play size={14} />}
            {currentPreview
              ? diffRows && diffRows.length > 0
                ? selectedDiffCount === 0
                  ? "Select changes to apply"
                  : `Apply ${selectedDiffCount} change${selectedDiffCount === 1 ? "" : "s"}`
                : "Apply changes"
              : "Authorize and run"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
