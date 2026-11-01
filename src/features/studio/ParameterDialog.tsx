import { useState, type FormEvent } from "react";
import { Loader2, Play, SlidersHorizontal } from "lucide-react";
import { InlineError, Modal } from "../../components/ui/Modal";
import { messageOf } from "../../ipc/client";
import type {
  ConnectionProfile,
  QueryParamInput,
  QueryParamKind,
  QueryParamSlot,
} from "../../ipc/generated";

export interface QueryParamDraft {
  kind: QueryParamKind;
  value: string;
  isNull: boolean;
}

const transientTabDrafts = new Map<string, Record<string, QueryParamDraft>>();

export function getTabParamDrafts(tabId: string): Record<string, QueryParamDraft> {
  return transientTabDrafts.get(tabId) ?? {};
}

export function setTabParamDrafts(tabId: string, drafts: Record<string, QueryParamDraft>): void {
  transientTabDrafts.set(tabId, { ...drafts });
}

export function pruneTabParamDrafts(tabIds: readonly string[]): void {
  const live = new Set(tabIds);
  for (const tabId of transientTabDrafts.keys())
    if (!live.has(tabId)) transientTabDrafts.delete(tabId);
}

export function clearTabParamDrafts(tabId: string): void {
  transientTabDrafts.delete(tabId);
}

export const PARAM_KINDS: ReadonlyArray<{
  id: QueryParamKind;
  label: string;
  placeholder: string;
}> = [
  { id: "text", label: "Text", placeholder: "Text value" },
  { id: "integer", label: "Integer", placeholder: "e.g. 42" },
  { id: "decimal", label: "Decimal", placeholder: "e.g. 123.45" },
  { id: "boolean", label: "Boolean", placeholder: "true or false" },
  { id: "uuid", label: "UUID", placeholder: "550e8400-e29b-41d4-a716-446655440000" },
  { id: "date", label: "Date", placeholder: "YYYY-MM-DD" },
  { id: "timestamp", label: "Timestamp", placeholder: "YYYY-MM-DDTHH:MM:SSZ" },
  { id: "json", label: "JSON", placeholder: '{"key": "value"}' },
];

const MIN_I64 = -9223372036854775808n;
const MAX_I64 = 9223372036854775807n;

export function formatParamDisplayKey(key: string): string {
  return key.startsWith("$") || key.startsWith("?") ? key : `:${key}`;
}

export function validateParamDraft(key: string, draft: QueryParamDraft): string | null {
  const label = formatParamDisplayKey(key);
  if (draft.isNull) return null;
  if (draft.value.includes("\0")) return `Parameter ${label} cannot contain NUL bytes`;
  const trimmed = draft.value.trim();
  switch (draft.kind) {
    case "text":
      return null;
    case "integer": {
      if (!/^[+-]?\d+$/.test(trimmed)) {
        return `Parameter ${label} must be a 64-bit integer`;
      }
      try {
        const parsed = BigInt(trimmed);
        if (parsed < MIN_I64 || parsed > MAX_I64) {
          return `Parameter ${label} must fit within a signed 64-bit integer`;
        }
      } catch {
        return `Parameter ${label} must be a 64-bit integer`;
      }
      return null;
    }
    case "decimal":
      if (!/^[+-]?(?:\d+\.?\d*|\.\d+)$/.test(trimmed)) {
        return `Parameter ${label} must be a valid decimal number`;
      }
      return null;
    case "boolean":
      if (!["true", "false", "1", "0", "yes", "no"].includes(trimmed.toLowerCase())) {
        return `Parameter ${label} must be true or false`;
      }
      return null;
    case "uuid":
      if (
        !/^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/.test(
          trimmed,
        )
      ) {
        return `Parameter ${label} must be a valid UUID`;
      }
      return null;
    case "date":
      if (!/^\d{4}-\d{2}-\d{2}$/.test(trimmed)) {
        return `Parameter ${label} must use YYYY-MM-DD format`;
      }
      return null;
    case "timestamp":
      if (!/^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}/.test(trimmed)) {
        return `Parameter ${label} must use ISO-8601 YYYY-MM-DDTHH:MM:SS[Z|+HH:MM] format`;
      }
      return null;
    case "json":
      try {
        JSON.parse(trimmed);
        return null;
      } catch {
        return `Parameter ${label} must be valid JSON`;
      }
  }
}

export function buildQueryParamInputs(
  slots: QueryParamSlot[],
  drafts: Record<string, QueryParamDraft>,
): { inputs?: QueryParamInput[]; error?: string } {
  const inputs: QueryParamInput[] = [];
  for (const slot of slots) {
    const draft = (Object.hasOwn(drafts, slot.key) ? drafts[slot.key] : undefined) ?? {
      kind: "text",
      value: "",
      isNull: false,
    };
    const err = validateParamDraft(slot.key, draft);
    if (err) return { error: err };
    inputs.push({
      key: slot.key,
      kind: draft.kind,
      value: draft.isNull ? null : draft.value,
    });
  }
  return { inputs };
}

export function ParameterDialog({
  slots,
  initialDrafts,
  profile,
  canRun = true,
  allowSaveOnly = false,
  onSaveDrafts,
  onRun,
  onClose,
}: {
  slots: QueryParamSlot[];
  initialDrafts?: Record<string, QueryParamDraft>;
  profile?: ConnectionProfile;
  canRun?: boolean;
  allowSaveOnly?: boolean;
  onSaveDrafts(drafts: Record<string, QueryParamDraft>): void;
  onRun?(inputs: QueryParamInput[], drafts: Record<string, QueryParamDraft>): Promise<void>;
  onClose(): void;
}) {
  const [drafts, setDrafts] = useState<Record<string, QueryParamDraft>>(() => {
    const next: Record<string, QueryParamDraft> = Object.assign(
      Object.create(null),
      initialDrafts ?? {},
    );
    for (const slot of slots) {
      if (!Object.hasOwn(next, slot.key)) {
        next[slot.key] = { kind: "text", value: "", isNull: false };
      }
    }
    return next;
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function updateSlot(key: string, patch: Partial<QueryParamDraft>) {
    setError(null);
    setDrafts((current) => {
      const existing = current[key] ?? { kind: "text", value: "", isNull: false };
      const updated: QueryParamDraft = { ...existing, ...patch };
      if (
        patch.kind === "boolean" &&
        !updated.isNull &&
        !["true", "false"].includes(updated.value.trim().toLowerCase())
      ) {
        updated.value = "true";
      }
      return { ...current, [key]: updated };
    });
  }

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    const built = buildQueryParamInputs(slots, drafts);
    if (built.error || !built.inputs) {
      setError(built.error ?? "Invalid parameter values");
      return;
    }
    onSaveDrafts(drafts);
    if (!canRun || !onRun) {
      onClose();
      return;
    }
    setBusy(true);
    try {
      await onRun(built.inputs, drafts);
    } catch (err) {
      setError(messageOf(err));
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Query parameters"
      subtitle={`${profile?.name ?? "SQL query"} · ${slots.length} parameter${slots.length === 1 ? "" : "s"}`}
      onClose={onClose}
      wide
      busy={busy}
    >
      <form onSubmit={(e) => void handleSubmit(e)}>
        <div className="modal-body">
          <p className="form-note" style={{ marginTop: 0 }}>
            <SlidersHorizontal size={13} style={{ verticalAlign: "text-bottom", marginRight: 6 }} />
            Bound values are included in the execution plan digest and stay in memory for this tab
            only—never saved to query history or workspace files.
          </p>
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            {slots.map((slot) => {
              const draft = (Object.hasOwn(drafts, slot.key) ? drafts[slot.key] : undefined) ?? {
                kind: "text",
                value: "",
                isNull: false,
              };
              const kindMeta = PARAM_KINDS.find((k) => k.id === draft.kind) ?? PARAM_KINDS[0]!;
              const label = formatParamDisplayKey(slot.key);
              return (
                <div
                  key={slot.key}
                  className="param-row"
                  style={{
                    display: "grid",
                    gridTemplateColumns: "minmax(120px, 1.1fr) 136px minmax(180px, 2fr) auto",
                    gap: 8,
                    alignItems: "center",
                  }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 6, minWidth: 0 }}>
                    <code
                      style={{
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                        fontWeight: 600,
                      }}
                      title={label}
                    >
                      {label}
                    </code>
                    {slot.occurrences > 1 && (
                      <span className="result-caption" title={`Used ${slot.occurrences} times`}>
                        ×{slot.occurrences}
                      </span>
                    )}
                  </div>
                  <select
                    aria-label={`Type for parameter ${slot.key}`}
                    value={draft.kind}
                    disabled={busy}
                    onChange={(e) =>
                      updateSlot(slot.key, { kind: e.target.value as QueryParamKind })
                    }
                  >
                    {PARAM_KINDS.map((kind) => (
                      <option key={kind.id} value={kind.id}>
                        {kind.label}
                      </option>
                    ))}
                  </select>
                  {draft.kind === "boolean" && !draft.isNull ? (
                    <select
                      aria-label={`Value for parameter ${slot.key}`}
                      value={draft.value.trim().toLowerCase() === "false" ? "false" : "true"}
                      disabled={busy}
                      onChange={(e) => updateSlot(slot.key, { value: e.target.value })}
                    >
                      <option value="true">true</option>
                      <option value="false">false</option>
                    </select>
                  ) : (
                    <input
                      type="text"
                      aria-label={`Value for parameter ${slot.key}`}
                      value={draft.isNull ? "" : draft.value}
                      placeholder={draft.isNull ? "NULL" : kindMeta.placeholder}
                      disabled={busy || draft.isNull}
                      spellCheck={false}
                      autoComplete="off"
                      onChange={(e) => updateSlot(slot.key, { value: e.target.value })}
                    />
                  )}
                  <label
                    style={{
                      display: "inline-flex",
                      alignItems: "center",
                      gap: 5,
                      fontSize: 12,
                      cursor: "pointer",
                      userSelect: "none",
                    }}
                  >
                    <input
                      type="checkbox"
                      aria-label={`NULL for parameter ${slot.key}`}
                      checked={draft.isNull}
                      disabled={busy}
                      onChange={(e) => updateSlot(slot.key, { isNull: e.target.checked })}
                    />
                    NULL
                  </label>
                </div>
              );
            })}
          </div>
          <InlineError message={error} />
        </div>
        <footer className="modal-footer">
          <button type="button" className="button ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          {allowSaveOnly && canRun && (
            <button
              type="button"
              className="button ghost"
              disabled={busy}
              onClick={() => {
                const built = buildQueryParamInputs(slots, drafts);
                if (built.error) {
                  setError(built.error);
                  return;
                }
                onSaveDrafts(drafts);
                onClose();
              }}
            >
              Save values
            </button>
          )}
          <button className="button primary" type="submit" disabled={busy}>
            {busy ? <Loader2 size={14} className="spin" /> : canRun ? <Play size={14} /> : null}
            {canRun ? "Run query" : "Save values"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
