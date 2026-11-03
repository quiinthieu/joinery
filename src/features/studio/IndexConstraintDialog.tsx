import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  Copy,
  FileCode2,
  Loader2,
  Play,
  Plus,
  Trash2,
} from "lucide-react";
import { InlineError, Modal } from "../../components/ui/Modal";
import { client, messageOf } from "../../ipc/client";
import type {
  ColumnInfo,
  ConnectionProfile,
  ConstraintInfo,
  ConstraintKind,
  IndexConstraintAction,
  IndexConstraintPreview,
  IndexInfo,
  ObjectDetails,
  PreparedPlan,
  QueryEvent,
  RunStatus,
  SessionInfo,
  TableInfo,
  TableRef,
  TableSchema,
} from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import { tableInfoKindToObjectKind } from "../connections/explorer-tree";
import { ReviewDialog } from "../safety/ReviewDialog";

export type IndexConstraintDialogMode =
  | { kind: "createIndex"; table: TableRef }
  | { kind: "dropIndex"; table: TableRef; index: IndexInfo }
  | { kind: "addConstraint"; table: TableRef }
  | { kind: "dropConstraint"; table: TableRef; constraint: ConstraintInfo };

const PG_INDEX_METHODS = ["btree", "hash", "gin", "gist", "spgist", "brin"] as const;
const MYSQL_INDEX_METHODS = ["btree", "hash", "fulltext", "spatial"] as const;
const REFERENTIAL_ACTIONS = [
  "NO ACTION",
  "RESTRICT",
  "CASCADE",
  "SET NULL",
  "SET DEFAULT",
] as const;

export function isConstraintBackedIndex(
  index: IndexInfo,
  constraints: readonly ConstraintInfo[],
): boolean {
  if (index.primary) return true;
  return constraints.some(
    (c) =>
      (c.kind === "primaryKey" || c.kind === "unique" || c.kind === "exclusion") &&
      c.name === index.name,
  );
}

function slugToken(value: string): string {
  const cleaned = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return cleaned || "obj";
}

function runApprovedPlan(
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

export function IndexConstraintDialog({
  mode,
  schema,
  details,
  profile,
  session,
  catalogTables,
  onSuccess,
  onClose,
}: {
  mode: IndexConstraintDialogMode;
  schema: TableSchema | null;
  details: ObjectDetails | null;
  profile: ConnectionProfile;
  session: SessionInfo;
  catalogTables: readonly TableInfo[];
  onSuccess(message: string): void;
  onClose(): void;
}) {
  const isPostgres = profile.engine === "postgres";
  const availableColumns = schema?.columns ?? [];
  const defaultColumnName = availableColumns[0]?.name ?? "id";
  const hasExistingPrimaryKey = Boolean(
    (schema?.primaryKey.length ?? 0) > 0 ||
    details?.constraints.some((c) => c.kind === "primaryKey"),
  );

  // Create Index state
  const [indexName, setIndexName] = useState(
    () => `${slugToken(mode.table.name)}_${slugToken(defaultColumnName)}_idx`,
  );
  const [indexNameTouched, setIndexNameTouched] = useState(false);
  const [indexMethod, setIndexMethod] = useState("btree");
  const [indexUnique, setIndexUnique] = useState(false);
  const [indexConcurrently, setIndexConcurrently] = useState(false);
  const [rawExpressions, setRawExpressions] = useState(false);
  const [indexColumns, setIndexColumns] = useState<string[]>(() => [defaultColumnName]);
  const [indexExpressions, setIndexExpressions] = useState<string[]>(() => [
    isPostgres ? `lower("${defaultColumnName}")` : `\`${defaultColumnName}\`(32)`,
  ]);
  const [indexPredicate, setIndexPredicate] = useState("");

  // Drop Index state
  const [dropConcurrently, setDropConcurrently] = useState(false);

  // Add Constraint state
  const [constraintKind, setConstraintKind] = useState<ConstraintKind>("unique");
  const [constraintName, setConstraintName] = useState(
    () => `${slugToken(mode.table.name)}_${slugToken(defaultColumnName)}_key`,
  );
  const [constraintNameTouched, setConstraintNameTouched] = useState(false);
  const [constraintColumns, setConstraintColumns] = useState<string[]>(() => [defaultColumnName]);
  const [checkExpression, setCheckExpression] = useState(() =>
    isPostgres ? `"${defaultColumnName}" IS NOT NULL` : `\`${defaultColumnName}\` IS NOT NULL`,
  );
  const baseTables = useMemo(
    () =>
      catalogTables.filter((entry) => {
        const objKind = tableInfoKindToObjectKind(entry.kind ?? "");
        return objKind === "table" || objKind === "partitionedTable";
      }),
    [catalogTables],
  );
  const initialTarget = baseTables[0]?.table ?? mode.table;
  const [targetSchema, setTargetSchema] = useState(initialTarget.schema);
  const [targetTable, setTargetTable] = useState(initialTarget.name);
  const [targetColumns, setTargetColumns] = useState<string[]>(["id"]);
  const [targetTableColumns, setTargetTableColumns] = useState<ColumnInfo[]>([]);
  const [onDelete, setOnDelete] = useState("NO ACTION");
  const [onUpdate, setOnUpdate] = useState("NO ACTION");
  const [notValid, setNotValid] = useState(false);

  // Drop Constraint state
  const [dropCascade, setDropCascade] = useState(false);

  // Preview and execution state
  const [preview, setPreview] = useState<IndexConstraintPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [executionError, setExecutionError] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [preparedPlan, setPreparedPlan] = useState<PreparedPlan | null>(null);
  const previewSeqRef = useRef(0);

  // Update auto-generated index name when columns change if untouched
  useEffect(() => {
    if (mode.kind !== "createIndex" || indexNameTouched) return;
    const keys = rawExpressions ? indexExpressions : indexColumns;
    const suffix = keys.map(slugToken).filter(Boolean).slice(0, 2).join("_") || "key";
    setIndexName(`${slugToken(mode.table.name)}_${suffix}_idx`);
  }, [indexColumns, indexExpressions, indexNameTouched, mode, rawExpressions]);

  // Update auto-generated constraint name when kind or columns change if untouched
  useEffect(() => {
    if (mode.kind !== "addConstraint" || constraintNameTouched) return;
    const tbl = slugToken(mode.table.name);
    const colPart = constraintColumns.map(slugToken).filter(Boolean).slice(0, 2).join("_") || "col";
    if (constraintKind === "primaryKey") {
      setConstraintName(!isPostgres ? "PRIMARY" : `${tbl}_pkey`);
    } else if (constraintKind === "unique") {
      setConstraintName(`${tbl}_${colPart}_key`);
    } else if (constraintKind === "check") {
      setConstraintName(`${tbl}_check`);
    } else if (constraintKind === "foreignKey") {
      setConstraintName(`${tbl}_${colPart}_fkey`);
    }
  }, [constraintColumns, constraintKind, constraintNameTouched, isPostgres, mode]);

  // Load target table schema when adding a foreign key
  useEffect(() => {
    if (mode.kind !== "addConstraint" || constraintKind !== "foreignKey") return;
    const s = targetSchema.trim();
    const t = targetTable.trim();
    if (!s || !t) {
      setTargetTableColumns([]);
      return;
    }
    let active = true;
    void client
      .call<TableSchema>("table_schema", {
        sessionId: session.id,
        table: { schema: s, name: t },
      })
      .then((res) => {
        if (!active) return;
        setTargetTableColumns(res.columns);
        if (res.columns.length > 0) {
          const pkDefaults = res.primaryKey.length ? res.primaryKey : [res.columns[0]!.name];
          setTargetColumns((prev) =>
            constraintColumns.map((_, idx) => prev[idx] || pkDefaults[idx] || res.columns[0]!.name),
          );
        }
      })
      .catch(() => {
        if (active) setTargetTableColumns([]);
      });
    return () => {
      active = false;
    };
  }, [constraintColumns.length, constraintKind, mode.kind, session.id, targetSchema, targetTable]);

  // Keep targetColumns length aligned with constraintColumns length for foreign keys
  useEffect(() => {
    if (mode.kind !== "addConstraint" || constraintKind !== "foreignKey") return;
    setTargetColumns((prev) => {
      if (prev.length === constraintColumns.length) return prev;
      const fallback = targetTableColumns[0]?.name ?? "id";
      return constraintColumns.map((_, i) => prev[i] ?? fallback);
    });
  }, [constraintColumns, constraintKind, mode.kind, targetTableColumns]);

  const action: IndexConstraintAction = useMemo(() => {
    switch (mode.kind) {
      case "createIndex":
        return {
          type: "createIndex",
          table: mode.table,
          name: indexName,
          unique: indexUnique,
          method: indexMethod,
          concurrently: isPostgres ? indexConcurrently : false,
          expressions: rawExpressions ? indexExpressions : indexColumns,
          rawExpressions,
          predicate: isPostgres && indexPredicate.trim() ? indexPredicate.trim() : null,
        };
      case "dropIndex":
        return {
          type: "dropIndex",
          table: mode.table,
          name: mode.index.name,
          concurrently: isPostgres ? dropConcurrently : false,
        };
      case "addConstraint":
        return {
          type: "addConstraint",
          table: mode.table,
          name: constraintName,
          kind: constraintKind,
          columns: constraintKind === "check" ? [] : constraintColumns,
          checkExpression: constraintKind === "check" ? checkExpression : null,
          target:
            constraintKind === "foreignKey"
              ? { schema: targetSchema.trim(), name: targetTable.trim() }
              : null,
          targetColumns: constraintKind === "foreignKey" ? targetColumns : [],
          onDelete: constraintKind === "foreignKey" ? onDelete : null,
          onUpdate: constraintKind === "foreignKey" ? onUpdate : null,
          notValid:
            isPostgres && (constraintKind === "check" || constraintKind === "foreignKey")
              ? notValid
              : false,
        };
      case "dropConstraint":
        return {
          type: "dropConstraint",
          table: mode.table,
          name: mode.constraint.name,
          kind: mode.constraint.kind,
          cascade: isPostgres ? dropCascade : false,
        };
    }
  }, [
    checkExpression,
    constraintColumns,
    constraintKind,
    constraintName,
    dropCascade,
    dropConcurrently,
    indexColumns,
    indexConcurrently,
    indexExpressions,
    indexMethod,
    indexName,
    indexPredicate,
    indexUnique,
    isPostgres,
    mode,
    notValid,
    onDelete,
    onUpdate,
    rawExpressions,
    targetColumns,
    targetSchema,
    targetTable,
  ]);

  useEffect(() => {
    const seq = ++previewSeqRef.current;
    void client
      .call<IndexConstraintPreview>("preview_index_constraint_ddl", {
        engine: profile.engine,
        action,
      })
      .then((res) => {
        if (seq !== previewSeqRef.current) return;
        setPreview(res);
        setPreviewError(null);
      })
      .catch((err) => {
        if (seq !== previewSeqRef.current) return;
        setPreview(null);
        setPreviewError(messageOf(err));
      });
  }, [action, profile.engine]);

  if (preparedPlan) {
    return (
      <ReviewDialog
        key={preparedPlan.id}
        plan={preparedPlan}
        profile={profile}
        onExecute={async (approval) => {
          const outcome = await runApprovedPlan(preparedPlan.id, approval);
          if (outcome.status === "completed") {
            onSuccess(preview?.statement.description ?? "Schema updated");
            onClose();
            return;
          }
          setPreparedPlan(null);
          setExecutionError(
            outcome.errorMessage ??
              (outcome.status === "outcomeUnknown"
                ? "The connection closed while applying DDL. Refresh the table structure to verify the outcome."
                : "The DDL operation failed."),
          );
        }}
        onClose={() => setPreparedPlan(null)}
      />
    );
  }

  async function handlePrepareReview() {
    if (!preview) return;
    setPreparing(true);
    setExecutionError(null);
    try {
      const plan = await client.call<PreparedPlan>("prepare_index_constraint_ddl", {
        sessionId: session.id,
        action,
      });
      setPreparing(false);
      setPreparedPlan(plan);
    } catch (err) {
      setPreparing(false);
      setExecutionError(messageOf(err));
    }
  }

  const dialogTitle =
    mode.kind === "createIndex"
      ? "Create index"
      : mode.kind === "dropIndex"
        ? `Drop index ${mode.index.name}`
        : mode.kind === "addConstraint"
          ? "Add table constraint"
          : `Drop constraint ${mode.constraint.name}`;

  const uniqueSupported = isPostgres
    ? indexMethod === "btree"
    : indexMethod !== "fulltext" && indexMethod !== "spatial";

  return (
    <Modal
      title={dialogTitle}
      subtitle={`${mode.table.schema}.${mode.table.name} · ${profile.name}`}
      onClose={onClose}
      wide
      busy={preparing}
    >
      <div className="modal-body" data-testid="index-constraint-dialog">
        {mode.kind === "createIndex" && (
          <div className="form-grid">
            <label>
              Index name
              <input
                aria-label="Index name"
                value={indexName}
                onChange={(e) => {
                  setIndexNameTouched(true);
                  setIndexName(e.target.value);
                }}
                placeholder="orders_customer_id_idx"
              />
            </label>
            <label>
              Access method
              <select
                aria-label="Index access method"
                value={indexMethod}
                onChange={(e) => {
                  const next = e.target.value;
                  setIndexMethod(next);
                  const nextUniqueOk = isPostgres
                    ? next === "btree"
                    : next !== "fulltext" && next !== "spatial";
                  if (!nextUniqueOk) setIndexUnique(false);
                }}
              >
                {(isPostgres ? PG_INDEX_METHODS : MYSQL_INDEX_METHODS).map((m) => (
                  <option key={m} value={m}>
                    {m.toUpperCase()}
                  </option>
                ))}
              </select>
            </label>

            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={indexUnique}
                disabled={!uniqueSupported}
                onChange={(e) => setIndexUnique(e.target.checked)}
              />
              <span>Unique index ({uniqueSupported ? "UNIQUE" : "requires BTREE"})</span>
            </label>

            {isPostgres && (
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={indexConcurrently}
                  onChange={(e) => setIndexConcurrently(e.target.checked)}
                />
                <span>Build CONCURRENTLY (non-blocking writes, outside transaction)</span>
              </label>
            )}

            <label className="checkbox-label span-2">
              <input
                type="checkbox"
                checked={rawExpressions}
                onChange={(e) => setRawExpressions(e.target.checked)}
              />
              <span>Use custom key expressions (functional index or MySQL prefix length)</span>
            </label>

            {!rawExpressions ? (
              <div className="span-2">
                <div className="object-section-head">
                  <span className="form-label">Indexed columns (in composite key order)</span>
                  <button
                    type="button"
                    className="button ghost small"
                    onClick={() => {
                      const unused =
                        availableColumns.find((c) => !indexColumns.includes(c.name))?.name ??
                        defaultColumnName;
                      setIndexColumns((prev) => [...prev, unused]);
                    }}
                  >
                    <Plus size={12} />
                    Add column
                  </button>
                </div>
                <div className="key-editor-list">
                  {indexColumns.map((col, idx) => (
                    <div className="key-editor-row" key={idx}>
                      <span className="mono">{idx + 1}.</span>
                      {availableColumns.length > 0 ? (
                        <select
                          aria-label={`Index column ${idx + 1}`}
                          value={col}
                          onChange={(e) =>
                            setIndexColumns((prev) =>
                              prev.map((item, i) => (i === idx ? e.target.value : item)),
                            )
                          }
                        >
                          {availableColumns.map((c) => (
                            <option key={c.id} value={c.name}>
                              {c.name} ({c.databaseType})
                            </option>
                          ))}
                        </select>
                      ) : (
                        <input
                          aria-label={`Index column ${idx + 1}`}
                          value={col}
                          onChange={(e) =>
                            setIndexColumns((prev) =>
                              prev.map((item, i) => (i === idx ? e.target.value : item)),
                            )
                          }
                        />
                      )}
                      <button
                        type="button"
                        className="icon-button"
                        aria-label={`Move index column ${idx + 1} up`}
                        disabled={idx === 0}
                        onClick={() =>
                          setIndexColumns((prev) => {
                            const next = [...prev];
                            [next[idx - 1], next[idx]] = [next[idx]!, next[idx - 1]!];
                            return next;
                          })
                        }
                      >
                        <ArrowUp size={12} />
                      </button>
                      <button
                        type="button"
                        className="icon-button"
                        aria-label={`Move index column ${idx + 1} down`}
                        disabled={idx === indexColumns.length - 1}
                        onClick={() =>
                          setIndexColumns((prev) => {
                            const next = [...prev];
                            [next[idx], next[idx + 1]] = [next[idx + 1]!, next[idx]!];
                            return next;
                          })
                        }
                      >
                        <ArrowDown size={12} />
                      </button>
                      {indexColumns.length > 1 && (
                        <button
                          type="button"
                          className="icon-button"
                          aria-label={`Remove index column ${idx + 1}`}
                          onClick={() =>
                            setIndexColumns((prev) => prev.filter((_, i) => i !== idx))
                          }
                        >
                          <Trash2 size={12} />
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            ) : (
              <div className="span-2">
                <div className="object-section-head">
                  <span className="form-label">Key expressions</span>
                  <button
                    type="button"
                    className="button ghost small"
                    onClick={() =>
                      setIndexExpressions((prev) => [
                        ...prev,
                        isPostgres ? `lower("col")` : "`col`(16)",
                      ])
                    }
                  >
                    <Plus size={12} />
                    Add expression
                  </button>
                </div>
                <div className="key-editor-list">
                  {indexExpressions.map((expr, idx) => (
                    <div className="key-editor-row" key={idx}>
                      <span className="mono">{idx + 1}.</span>
                      <input
                        aria-label={`Index key expression ${idx + 1}`}
                        value={expr}
                        onChange={(e) =>
                          setIndexExpressions((prev) =>
                            prev.map((item, i) => (i === idx ? e.target.value : item)),
                          )
                        }
                      />
                      {indexExpressions.length > 1 && (
                        <button
                          type="button"
                          className="icon-button"
                          aria-label={`Remove index key expression ${idx + 1}`}
                          onClick={() =>
                            setIndexExpressions((prev) => prev.filter((_, i) => i !== idx))
                          }
                        >
                          <Trash2 size={12} />
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}

            {isPostgres && (
              <label className="span-2">
                Partial index predicate (optional WHERE clause)
                <input
                  aria-label="Partial index WHERE predicate"
                  value={indexPredicate}
                  onChange={(e) => setIndexPredicate(e.target.value)}
                  placeholder="status = 'active' AND deleted_at IS NULL"
                />
              </label>
            )}
          </div>
        )}

        {mode.kind === "dropIndex" && (
          <div className="form-grid">
            <p className="confirmation-description span-2">
              You are preparing to drop index <code>{mode.index.name}</code> on{" "}
              <code>{`${mode.table.schema}.${mode.table.name}`}</code>.
            </p>
            {isPostgres && (
              <label className="checkbox-label span-2">
                <input
                  type="checkbox"
                  checked={dropConcurrently}
                  onChange={(e) => setDropConcurrently(e.target.checked)}
                />
                <span>Drop CONCURRENTLY (avoids locking concurrent reads/writes)</span>
              </label>
            )}
          </div>
        )}

        {mode.kind === "addConstraint" && (
          <div className="form-grid">
            <label>
              Constraint kind
              <select
                aria-label="Constraint kind"
                value={constraintKind}
                onChange={(e) => setConstraintKind(e.target.value as ConstraintKind)}
              >
                <option value="unique">UNIQUE</option>
                <option value="check">CHECK</option>
                <option value="foreignKey">FOREIGN KEY</option>
                <option value="primaryKey" disabled={hasExistingPrimaryKey}>
                  PRIMARY KEY{hasExistingPrimaryKey ? " (already defined)" : ""}
                </option>
              </select>
            </label>

            <label>
              Constraint name
              <input
                aria-label="Constraint name"
                value={constraintName}
                disabled={!isPostgres && constraintKind === "primaryKey"}
                onChange={(e) => {
                  setConstraintNameTouched(true);
                  setConstraintName(e.target.value);
                }}
              />
            </label>

            {constraintKind !== "check" && (
              <div className="span-2">
                <div className="object-section-head">
                  <span className="form-label">Constraint columns</span>
                  <button
                    type="button"
                    className="button ghost small"
                    onClick={() => {
                      const unused =
                        availableColumns.find((c) => !constraintColumns.includes(c.name))?.name ??
                        defaultColumnName;
                      setConstraintColumns((prev) => [...prev, unused]);
                    }}
                  >
                    <Plus size={12} />
                    Add column
                  </button>
                </div>
                <div className="key-editor-list">
                  {constraintColumns.map((col, idx) => (
                    <div className="key-editor-row" key={idx}>
                      <span className="mono">{idx + 1}.</span>
                      {availableColumns.length > 0 ? (
                        <select
                          aria-label={`Constraint column ${idx + 1}`}
                          value={col}
                          onChange={(e) =>
                            setConstraintColumns((prev) =>
                              prev.map((item, i) => (i === idx ? e.target.value : item)),
                            )
                          }
                        >
                          {availableColumns.map((c) => (
                            <option key={c.id} value={c.name}>
                              {c.name} ({c.databaseType})
                            </option>
                          ))}
                        </select>
                      ) : (
                        <input
                          aria-label={`Constraint column ${idx + 1}`}
                          value={col}
                          onChange={(e) =>
                            setConstraintColumns((prev) =>
                              prev.map((item, i) => (i === idx ? e.target.value : item)),
                            )
                          }
                        />
                      )}
                      {constraintColumns.length > 1 && (
                        <button
                          type="button"
                          className="icon-button"
                          aria-label={`Remove constraint column ${idx + 1}`}
                          onClick={() =>
                            setConstraintColumns((prev) => prev.filter((_, i) => i !== idx))
                          }
                        >
                          <Trash2 size={12} />
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}

            {constraintKind === "check" && (
              <label className="span-2">
                CHECK boolean expression
                <input
                  aria-label="CHECK expression"
                  value={checkExpression}
                  onChange={(e) => setCheckExpression(e.target.value)}
                  placeholder='quantity > 0 AND "Unit Price" >= 0'
                />
              </label>
            )}

            {constraintKind === "foreignKey" && (
              <>
                <label className="span-2">
                  Referenced table
                  {baseTables.length > 0 ? (
                    <select
                      aria-label="Referenced table"
                      value={`${targetSchema}.${targetTable}`}
                      onChange={(e) => {
                        const [s, ...rest] = e.target.value.split(".");
                        if (s && rest.length) {
                          setTargetSchema(s);
                          setTargetTable(rest.join("."));
                        }
                      }}
                    >
                      {baseTables.map((entry) => {
                        const key = `${entry.table.schema}.${entry.table.name}`;
                        return (
                          <option key={key} value={key}>
                            {key}
                          </option>
                        );
                      })}
                    </select>
                  ) : (
                    <input
                      aria-label="Referenced table"
                      value={targetTable}
                      onChange={(e) => setTargetTable(e.target.value)}
                    />
                  )}
                </label>

                <div className="span-2">
                  <span className="form-label">Referenced target columns</span>
                  <div className="key-editor-list">
                    {constraintColumns.map((srcCol, idx) => (
                      <div className="key-editor-row" key={idx}>
                        <span className="mono">{`${srcCol} →`}</span>
                        {targetTableColumns.length > 0 ? (
                          <select
                            aria-label={`Referenced column ${idx + 1}`}
                            value={targetColumns[idx] ?? ""}
                            onChange={(e) =>
                              setTargetColumns((prev) =>
                                prev.map((item, i) => (i === idx ? e.target.value : item)),
                              )
                            }
                          >
                            {targetTableColumns.map((c) => (
                              <option key={c.id} value={c.name}>
                                {c.name} ({c.databaseType})
                              </option>
                            ))}
                          </select>
                        ) : (
                          <input
                            aria-label={`Referenced column ${idx + 1}`}
                            value={targetColumns[idx] ?? ""}
                            onChange={(e) =>
                              setTargetColumns((prev) =>
                                prev.map((item, i) => (i === idx ? e.target.value : item)),
                              )
                            }
                          />
                        )}
                      </div>
                    ))}
                  </div>
                </div>

                <label>
                  ON DELETE
                  <select
                    aria-label="Foreign key ON DELETE action"
                    value={onDelete}
                    onChange={(e) => setOnDelete(e.target.value)}
                  >
                    {REFERENTIAL_ACTIONS.map((a) => (
                      <option key={a} value={a}>
                        {a}
                      </option>
                    ))}
                  </select>
                </label>

                <label>
                  ON UPDATE
                  <select
                    aria-label="Foreign key ON UPDATE action"
                    value={onUpdate}
                    onChange={(e) => setOnUpdate(e.target.value)}
                  >
                    {REFERENTIAL_ACTIONS.map((a) => (
                      <option key={a} value={a}>
                        {a}
                      </option>
                    ))}
                  </select>
                </label>
              </>
            )}

            {isPostgres && (constraintKind === "check" || constraintKind === "foreignKey") && (
              <label className="checkbox-label span-2">
                <input
                  type="checkbox"
                  checked={notValid}
                  onChange={(e) => setNotValid(e.target.checked)}
                />
                <span>Add as NOT VALID (skip full-table validation scan of existing rows)</span>
              </label>
            )}
          </div>
        )}

        {mode.kind === "dropConstraint" && (
          <div className="form-grid">
            <p className="confirmation-description span-2">
              You are preparing to drop constraint <code>{mode.constraint.name}</code> (
              <code>{mode.constraint.kind}</code>) on{" "}
              <code>{`${mode.table.schema}.${mode.table.name}`}</code>.
            </p>
            {isPostgres && (
              <label className="checkbox-label span-2">
                <input
                  type="checkbox"
                  checked={dropCascade}
                  onChange={(e) => setDropCascade(e.target.checked)}
                />
                <span>Drop with CASCADE (also drops dependent objects)</span>
              </label>
            )}
          </div>
        )}

        {preview?.warnings.map((warning) => (
          <div className="safety-warning" key={warning}>
            <AlertTriangle size={15} />
            <span>{warning}</span>
          </div>
        ))}

        {preview && (
          <div>
            <div className="object-section-head">
              <span className="eyebrow">GENERATED DDL PREVIEW</span>
              {preview.statement.destructive && (
                <span className="lock-badge blocker">Destructive DDL</span>
              )}
            </div>
            <pre
              className="sql-preview"
              aria-label="Generated DDL preview"
              data-testid="index-constraint-sql-preview"
            >
              {preview.statement.sql}
            </pre>
          </div>
        )}

        <InlineError message={previewError ?? executionError} />
      </div>

      <footer className="modal-footer">
        <button type="button" className="button ghost" onClick={onClose} disabled={preparing}>
          Cancel
        </button>
        {preview && (
          <>
            <button
              type="button"
              className="button ghost"
              disabled={preparing}
              onClick={() => {
                void navigator.clipboard
                  ?.writeText(preview.statement.sql)
                  .then(() => useStudio.getState().notify("Copied DDL to clipboard"))
                  .catch((err) => useStudio.getState().notify(messageOf(err)));
              }}
            >
              <Copy size={13} />
              Copy DDL
            </button>
            <button
              type="button"
              className="button ghost"
              disabled={preparing}
              onClick={() => {
                useStudio.getState().openQuery(profile.id, preview.statement.sql);
                onClose();
              }}
            >
              <FileCode2 size={13} />
              Open in SQL editor
            </button>
          </>
        )}
        <button
          type="button"
          className={`button ${preview?.statement.destructive ? "danger" : "primary"}`}
          disabled={!preview || preparing || profile.safeMode === "readOnly"}
          onClick={() => void handlePrepareReview()}
        >
          {preparing ? <Loader2 size={14} className="spin" /> : <Play size={14} />}
          Review &amp; authorize…
        </button>
      </footer>
    </Modal>
  );
}
