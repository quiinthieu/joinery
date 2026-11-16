import { useId, useMemo, useState } from "react";
import { Link2, Plus, Trash2 } from "lucide-react";
import type { TableRef, VirtualForeignKey } from "../../ipc/generated";
import { messageOf } from "../../ipc/client";
import { InlineError, Modal } from "../../components/ui/Modal";
import { useStudio } from "../../state/studio";

export interface VirtualForeignKeyDialogProps {
  connectionId: string;
  initialSource?: TableRef | null;
  sourceColumns?: readonly string[];
  knownTables?: readonly TableRef[];
  existing?: VirtualForeignKey | null;
  onSaved?(saved: VirtualForeignKey): void;
  onClose(): void;
}

interface ColumnPairDraft {
  id: string;
  sourceColumn: string;
  targetColumn: string;
}

function defaultRelationshipName(sourceName: string, targetName: string): string {
  const src = sourceName.trim() || "source";
  const dst = targetName.trim() || "target";
  return `vfk_${src}_${dst}`.slice(0, 120);
}

export function VirtualForeignKeyDialog({
  connectionId,
  initialSource = null,
  sourceColumns = [],
  knownTables = [],
  existing = null,
  onSaved,
  onClose,
}: VirtualForeignKeyDialogProps) {
  const state = useStudio();
  const datalistBaseId = useId();
  const profile = state.bootstrap?.profiles.find((p) => p.id === connectionId) ?? null;

  const defaultSourceSchema = existing?.source.schema ?? initialSource?.schema ?? "public";
  const defaultSourceName = existing?.source.name ?? initialSource?.name ?? "";
  const fallbackTarget = useMemo(() => {
    if (existing) return existing.target;
    const candidate = knownTables.find(
      (t) => !(t.schema === defaultSourceSchema && t.name === defaultSourceName),
    );
    return candidate ?? { schema: defaultSourceSchema, name: "" };
  }, [defaultSourceName, defaultSourceSchema, existing, knownTables]);

  const [sourceSchema, setSourceSchema] = useState(defaultSourceSchema);
  const [sourceName, setSourceName] = useState(defaultSourceName);
  const [targetSchema, setTargetSchema] = useState(fallbackTarget.schema);
  const [targetName, setTargetName] = useState(fallbackTarget.name);
  const [name, setName] = useState(
    existing?.name ?? defaultRelationshipName(defaultSourceName, fallbackTarget.name),
  );
  const [nameEdited, setNameEdited] = useState(Boolean(existing));
  const [pairs, setPairs] = useState<ColumnPairDraft[]>(() => {
    if (existing && existing.columns.length > 0) {
      return existing.columns.map((col, idx) => ({
        id: `pair-${idx}`,
        sourceColumn: col,
        targetColumn: existing.targetColumns[idx] ?? "id",
      }));
    }
    const guessedFkCol = sourceColumns.find((col) => /_id$/i.test(col)) ?? sourceColumns[0] ?? "";
    return [{ id: "pair-0", sourceColumn: guessedFkCol, targetColumn: "id" }];
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const sourceMatchesInitial =
    Boolean(initialSource) &&
    sourceSchema.trim() === initialSource?.schema &&
    sourceName.trim() === initialSource?.name;

  function syncAutoName(nextSrc: string, nextDst: string) {
    if (!nameEdited) {
      setName(defaultRelationshipName(nextSrc, nextDst));
    }
  }

  return (
    <Modal
      title={existing ? "Edit virtual foreign key" : "Add virtual foreign key"}
      subtitle={
        profile
          ? `Local Joinery relationship on ${profile.name} (never alters database DDL)`
          : "Local Joinery relationship for diagrams, JOIN completion, and navigation"
      }
      onClose={onClose}
      busy={busy}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          const cleanName = name.trim();
          const cleanSource = { schema: sourceSchema.trim(), name: sourceName.trim() };
          const cleanTarget = { schema: targetSchema.trim(), name: targetName.trim() };
          const columns = pairs.map((p) => p.sourceColumn.trim());
          const targetColumns = pairs.map((p) => p.targetColumn.trim());
          if (!cleanName) {
            setError("Provide a relationship name");
            return;
          }
          if (
            !cleanSource.schema ||
            !cleanSource.name ||
            !cleanTarget.schema ||
            !cleanTarget.name
          ) {
            setError("Provide both source and referenced schema and table names");
            return;
          }
          if (columns.some((c) => !c) || targetColumns.some((c) => !c)) {
            setError("Every column mapping pair must specify source and referenced columns");
            return;
          }
          setBusy(true);
          setError(null);
          void state
            .saveVirtualForeignKey({
              id: existing?.id ?? null,
              connectionId,
              name: cleanName,
              source: cleanSource,
              columns,
              target: cleanTarget,
              targetColumns,
            })
            .then((saved) => {
              onSaved?.(saved);
              onClose();
            })
            .catch((err) => {
              setError(messageOf(err));
              setBusy(false);
            });
        }}
      >
        <div className="modal-body form-grid">
          <label className="field span-2">
            <span>Relationship name</span>
            <input
              aria-label="Relationship name"
              maxLength={120}
              value={name}
              disabled={busy}
              onChange={(event) => {
                setNameEdited(true);
                setName(event.target.value);
              }}
            />
          </label>

          <label className="field">
            <span>Source schema</span>
            <input
              aria-label="Source schema"
              value={sourceSchema}
              disabled={busy}
              onChange={(event) => setSourceSchema(event.target.value)}
            />
          </label>
          <label className="field">
            <span>Source table</span>
            <input
              aria-label="Source table"
              list={`${datalistBaseId}-tables`}
              value={sourceName}
              disabled={busy}
              onChange={(event) => {
                const next = event.target.value;
                setSourceName(next);
                syncAutoName(next, targetName);
              }}
            />
          </label>

          {knownTables.length > 0 && (
            <label className="field span-2">
              <span>Quick-fill referenced table</span>
              <select
                aria-label="Quick-fill referenced table"
                value={
                  knownTables.some(
                    (t) => t.schema === targetSchema.trim() && t.name === targetName.trim(),
                  )
                    ? `${targetSchema.trim()}.${targetName.trim()}`
                    : ""
                }
                disabled={busy}
                onChange={(event) => {
                  const chosen = knownTables.find(
                    (t) => `${t.schema}.${t.name}` === event.target.value,
                  );
                  if (chosen) {
                    setTargetSchema(chosen.schema);
                    setTargetName(chosen.name);
                    syncAutoName(sourceName, chosen.name);
                  }
                }}
              >
                <option value="">Custom schema / table…</option>
                {knownTables.map((t) => (
                  <option key={`${t.schema}.${t.name}`} value={`${t.schema}.${t.name}`}>
                    {t.schema}.{t.name}
                  </option>
                ))}
              </select>
            </label>
          )}

          <label className="field">
            <span>Referenced schema</span>
            <input
              aria-label="Referenced schema"
              value={targetSchema}
              disabled={busy}
              onChange={(event) => setTargetSchema(event.target.value)}
            />
          </label>
          <label className="field">
            <span>Referenced table</span>
            <input
              aria-label="Referenced table"
              list={`${datalistBaseId}-tables`}
              value={targetName}
              disabled={busy}
              onChange={(event) => {
                const next = event.target.value;
                setTargetName(next);
                syncAutoName(sourceName, next);
              }}
            />
          </label>

          <div className="span-2" style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
              <strong style={{ fontSize: 12 }}>Column pairs</strong>
              <button
                type="button"
                className="button ghost small"
                disabled={busy || pairs.length >= 16}
                onClick={() =>
                  setPairs((prev) => [
                    ...prev,
                    {
                      id: `pair-${Date.now()}-${prev.length}`,
                      sourceColumn: "",
                      targetColumn: "",
                    },
                  ])
                }
              >
                <Plus size={12} />
                Add column pair
              </button>
            </div>
            {pairs.map((pair, idx) => (
              <div
                key={pair.id}
                style={{
                  display: "grid",
                  gridTemplateColumns: "1fr auto 1fr auto",
                  gap: 8,
                  alignItems: "center",
                }}
              >
                <input
                  aria-label={`Source column ${idx + 1}`}
                  placeholder="Source column"
                  list={sourceMatchesInitial ? `${datalistBaseId}-src-cols` : undefined}
                  value={pair.sourceColumn}
                  disabled={busy}
                  onChange={(event) =>
                    setPairs((prev) =>
                      prev.map((entry) =>
                        entry.id === pair.id
                          ? { ...entry, sourceColumn: event.target.value }
                          : entry,
                      ),
                    )
                  }
                />
                <Link2 size={13} className="muted" />
                <input
                  aria-label={`Referenced column ${idx + 1}`}
                  placeholder="Referenced column"
                  value={pair.targetColumn}
                  disabled={busy}
                  onChange={(event) =>
                    setPairs((prev) =>
                      prev.map((entry) =>
                        entry.id === pair.id
                          ? { ...entry, targetColumn: event.target.value }
                          : entry,
                      ),
                    )
                  }
                />
                {pairs.length > 1 ? (
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={`Remove column pair ${idx + 1}`}
                    disabled={busy}
                    onClick={() => setPairs((prev) => prev.filter((e) => e.id !== pair.id))}
                  >
                    <Trash2 size={13} />
                  </button>
                ) : (
                  <span style={{ width: 26 }} />
                )}
              </div>
            ))}
          </div>

          <datalist id={`${datalistBaseId}-tables`}>
            {knownTables.map((t) => (
              <option key={`${t.schema}.${t.name}`} value={t.name}>
                {t.schema}.{t.name}
              </option>
            ))}
          </datalist>
          {sourceMatchesInitial && sourceColumns.length > 0 && (
            <datalist id={`${datalistBaseId}-src-cols`}>
              {sourceColumns.map((col) => (
                <option key={col} value={col} />
              ))}
            </datalist>
          )}

          <p className="field-hint span-2">
            Virtual foreign keys stay local in Joinery and never modify the remote database. They
            appear as dashed edges in schema diagrams, power SQL JOIN completion, and enable
            related-row navigation on unindexed analytics tables and views.
          </p>
          <div className="span-2">
            <InlineError message={error} />
          </div>
        </div>
        <footer className="modal-footer">
          <button type="button" className="button ghost" disabled={busy} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="button primary" disabled={busy}>
            {existing ? "Update virtual FK" : "Save virtual FK"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
