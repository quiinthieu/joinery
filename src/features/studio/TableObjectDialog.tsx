import { useEffect, useMemo, useState } from "react";
import { AlertTriangle, Copy, ExternalLink, Plus, ShieldCheck, Trash2 } from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import { Modal } from "../../components/ui/Modal";
import { client } from "../../ipc/client";
import type {
  ColumnInfo,
  DatabaseObjectRef,
  EngineKind,
  PreparedPlan,
  TableColumnDraft,
  TableObjectAction,
  TableObjectPreview,
  TableRef,
} from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import { ReviewDialog } from "../safety/ReviewDialog";

export type TableObjectDialogRequest =
  | {
      kind: "createTable";
      schema: string;
    }
  | {
      kind: "addColumn";
      table: TableRef;
    }
  | {
      kind: "alterColumn";
      table: TableRef;
      column: ColumnInfo;
    }
  | {
      kind: "dropColumn";
      table: TableRef;
      column: ColumnInfo;
    }
  | {
      kind: "dropObject";
      object: DatabaseObjectRef;
    };

const POSTGRES_COMMON_TYPES = [
  "bigint",
  "integer",
  "smallint",
  "text",
  "varchar(255)",
  "boolean",
  "numeric(12,2)",
  "timestamptz",
  "date",
  "uuid",
  "jsonb",
] as const;

const MYSQL_COMMON_TYPES = [
  "bigint",
  "int",
  "smallint",
  "varchar(255)",
  "text",
  "tinyint(1)",
  "decimal(12,2)",
  "datetime",
  "timestamp",
  "date",
  "json",
] as const;

function defaultCreateColumns(engine: EngineKind): TableColumnDraft[] {
  return [
    {
      name: "id",
      dataType: "bigint",
      nullable: false,
      defaultExpression: null,
      autoIncrement: true,
      primaryKey: true,
    },
    {
      name: "name",
      dataType: engine === "mysql" ? "varchar(255)" : "text",
      nullable: false,
      defaultExpression: null,
      autoIncrement: false,
      primaryKey: false,
    },
  ];
}

export function TableObjectDialog({
  connectionId,
  request,
  onClose,
  onCompleted,
}: {
  connectionId: string;
  request: TableObjectDialogRequest;
  onClose(): void;
  onCompleted?(action: TableObjectAction): void;
}) {
  const { profile, session, refreshCatalog, notify, openQuery } = useStudio(
    useShallow((s) => ({
      profile: s.bootstrap?.profiles.find((p) => p.id === connectionId),
      session: s.sessions[connectionId],
      refreshCatalog: s.refreshCatalog,
      notify: s.notify,
      openQuery: s.openQuery,
    })),
  );

  const engine: EngineKind = profile?.engine ?? session?.engine ?? "postgres";
  const isPostgres = engine === "postgres";
  const isReadOnly = profile?.safeMode === "readOnly";
  const typeSuggestions = isPostgres ? POSTGRES_COMMON_TYPES : MYSQL_COMMON_TYPES;

  // Create Table state
  const [tableName, setTableName] = useState("new_table");
  const [tableComment, setTableComment] = useState("");
  const [columns, setColumns] = useState<TableColumnDraft[]>(() => defaultCreateColumns(engine));

  // Add / Alter Column state
  const [colName, setColName] = useState(
    request.kind === "alterColumn" ? request.column.name : "new_column",
  );
  const [colDataType, setColDataType] = useState(
    request.kind === "alterColumn"
      ? request.column.databaseType
      : engine === "mysql"
        ? "varchar(255)"
        : "text",
  );
  const [colNullable, setColNullable] = useState(
    request.kind === "alterColumn" ? request.column.nullable : true,
  );
  const [colDefault, setColDefault] = useState("");
  const [colAutoIncrement, setColAutoIncrement] = useState(false);
  const [colPrimaryKey, setColPrimaryKey] = useState(false);
  const [colUsingExpr, setColUsingExpr] = useState("");

  // Drop Column / Drop Object state
  const [cascade, setCascade] = useState(false);

  // Preview and execution state
  const [preview, setPreview] = useState<TableObjectPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [preparedPlan, setPreparedPlan] = useState<PreparedPlan | null>(null);

  const action = useMemo<TableObjectAction>(() => {
    switch (request.kind) {
      case "createTable":
        return {
          type: "createTable",
          table: {
            schema: request.schema,
            name: tableName.trim(),
          },
          columns: columns.map((col) => ({
            name: col.name.trim(),
            dataType: col.dataType.trim(),
            nullable: col.primaryKey || col.autoIncrement ? false : col.nullable,
            defaultExpression:
              col.autoIncrement || !col.defaultExpression?.trim()
                ? null
                : col.defaultExpression.trim(),
            autoIncrement: col.autoIncrement,
            primaryKey: col.primaryKey,
          })),
          comment: tableComment.trim() ? tableComment.trim() : null,
        };
      case "addColumn":
        return {
          type: "addColumn",
          table: request.table,
          column: {
            name: colName.trim(),
            dataType: colDataType.trim(),
            nullable: colPrimaryKey || colAutoIncrement ? false : colNullable,
            defaultExpression: colAutoIncrement || !colDefault.trim() ? null : colDefault.trim(),
            autoIncrement: colAutoIncrement,
            primaryKey: colPrimaryKey,
          },
        };
      case "alterColumn": {
        const trimmedNew = colName.trim();
        return {
          type: "alterColumn",
          table: request.table,
          columnName: request.column.name,
          newName: trimmedNew && trimmedNew !== request.column.name ? trimmedNew : null,
          dataType: colDataType.trim(),
          nullable: colNullable,
          defaultExpression: colDefault.trim() ? colDefault.trim() : null,
          usingExpression: isPostgres && colUsingExpr.trim() ? colUsingExpr.trim() : null,
        };
      }
      case "dropColumn":
        return {
          type: "dropColumn",
          table: request.table,
          columnName: request.column.name,
          cascade: isPostgres ? cascade : false,
        };
      case "dropObject":
        return {
          type: "dropObject",
          object: request.object,
          cascade: isPostgres ? cascade : false,
        };
    }
  }, [
    request,
    tableName,
    tableComment,
    columns,
    colName,
    colDataType,
    colNullable,
    colDefault,
    colAutoIncrement,
    colPrimaryKey,
    colUsingExpr,
    isPostgres,
    cascade,
  ]);

  useEffect(() => {
    let cancelled = false;
    void client
      .call<TableObjectPreview>("preview_table_object_ddl", {
        engine,
        action,
      })
      .then((res) => {
        if (cancelled) return;
        setPreview(res);
        setPreviewError(null);
      })
      .catch((err) => {
        if (cancelled) return;
        setPreview(null);
        setPreviewError(String(err?.message ?? err));
      });
    return () => {
      cancelled = true;
    };
  }, [engine, action]);

  const dialogTitle = (() => {
    switch (request.kind) {
      case "createTable":
        return "Create table";
      case "addColumn":
        return "Add column";
      case "alterColumn":
        return "Modify column";
      case "dropColumn":
        return "Drop column";
      case "dropObject":
        return request.object.kind === "view" || request.object.kind === "materializedView"
          ? "Drop view"
          : "Drop table";
    }
  })();

  const updateCreateColumn = (index: number, patch: Partial<TableColumnDraft>) => {
    setColumns((prev) =>
      prev.map((col, idx) => {
        if (idx !== index) return col;
        const next = { ...col, ...patch };
        if (next.autoIncrement) {
          next.nullable = false;
          next.defaultExpression = null;
          if (!isPostgres) {
            next.primaryKey = true;
          }
        }
        if (next.primaryKey) {
          next.nullable = false;
        }
        return next;
      }),
    );
  };

  const handleAddCreateColumn = () => {
    setColumns((prev) => [
      ...prev,
      {
        name: `col_${prev.length + 1}`,
        dataType: engine === "mysql" ? "varchar(255)" : "text",
        nullable: true,
        defaultExpression: null,
        autoIncrement: false,
        primaryKey: false,
      },
    ]);
  };

  const handleRemoveCreateColumn = (index: number) => {
    setColumns((prev) => (prev.length > 1 ? prev.filter((_, idx) => idx !== index) : prev));
  };

  const handleCopyDdl = async () => {
    if (!preview) return;
    try {
      await navigator.clipboard.writeText(preview.script);
      notify("Copied DDL to clipboard.");
    } catch {
      notify("Clipboard copy failed.");
    }
  };

  const handleOpenInEditor = () => {
    if (!preview) return;
    openQuery(connectionId, preview.script);
    onClose();
  };

  const handlePrepareAndReview = async () => {
    if (!session) {
      notify("Connect this database profile before executing DDL.");
      return;
    }
    setPreparing(true);
    try {
      const plan = await client.call<PreparedPlan>("prepare_table_object_ddl", {
        sessionId: session.id,
        action,
      });
      setPreparedPlan(plan);
    } catch (err) {
      notify(String((err as { message?: string })?.message ?? err));
    } finally {
      setPreparing(false);
    }
  };

  const handleExecuteApproved = async (approval: string | null) => {
    if (!preparedPlan) return;
    const outcome = await new Promise<{ status: string; errorMessage: string | null }>(
      (resolve, reject) => {
        let errorMessage: string | null = null;
        client
          .execute(preparedPlan.id, approval ?? "", (event) => {
            if (event.type === "error") {
              errorMessage = event.error.message;
            } else if (event.type === "finished") {
              resolve({ status: event.status, errorMessage });
            }
          })
          .catch(reject);
      },
    );
    setPreparedPlan(null);
    if (outcome.errorMessage) {
      notify(outcome.errorMessage);
      return;
    }
    if (outcome.status === "completed") {
      await refreshCatalog(connectionId);
      onCompleted?.(action);
      notify(`${dialogTitle} completed.`);
      onClose();
    }
  };

  if (preparedPlan && profile) {
    return (
      <ReviewDialog
        key={preparedPlan.id}
        plan={preparedPlan}
        profile={profile}
        onExecute={handleExecuteApproved}
        onClose={() => setPreparedPlan(null)}
      />
    );
  }

  return (
    <Modal title={dialogTitle} onClose={onClose} wide busy={preparing}>
      <div className="modal-form">
        <datalist id="table-column-types">
          {typeSuggestions.map((t) => (
            <option key={t} value={t} />
          ))}
        </datalist>

        {request.kind === "createTable" && (
          <>
            <div className="field-grid">
              <label>
                <span>Target schema</span>
                <input value={request.schema} readOnly disabled />
              </label>
              <label>
                <span>Table name</span>
                <input
                  aria-label="Table name"
                  value={tableName}
                  onChange={(e) => setTableName(e.target.value)}
                  placeholder="orders"
                />
              </label>
            </div>
            <label>
              <span>Table comment (optional)</span>
              <input
                aria-label="Table comment"
                value={tableComment}
                onChange={(e) => setTableComment(e.target.value)}
                placeholder="Optional description"
              />
            </label>

            <div className="structure-section-header">
              <strong>Columns ({columns.length})</strong>
              <button type="button" className="ghost-button small" onClick={handleAddCreateColumn}>
                <Plus size={12} />
                Add column
              </button>
            </div>

            <div className="ddl-column-list">
              {columns.map((col, idx) => (
                <div
                  key={idx}
                  className="ddl-column-row"
                  style={{
                    display: "grid",
                    gridTemplateColumns: "1.2fr 1.1fr 1fr auto auto auto auto",
                    gap: "8px",
                    alignItems: "center",
                    marginBottom: "8px",
                  }}
                >
                  <input
                    aria-label={`Column ${idx + 1} name`}
                    value={col.name}
                    onChange={(e) => updateCreateColumn(idx, { name: e.target.value })}
                    placeholder="column_name"
                  />
                  <input
                    aria-label={`Column ${idx + 1} data type`}
                    list="table-column-types"
                    value={col.dataType}
                    onChange={(e) => updateCreateColumn(idx, { dataType: e.target.value })}
                    placeholder="bigint / text"
                  />
                  <input
                    aria-label={`Column ${idx + 1} default`}
                    value={col.defaultExpression ?? ""}
                    disabled={col.autoIncrement}
                    onChange={(e) =>
                      updateCreateColumn(idx, {
                        defaultExpression: e.target.value || null,
                      })
                    }
                    placeholder={col.autoIncrement ? "auto-generated" : "Optional DEFAULT"}
                  />
                  <label className="checkbox-inline" title="Primary Key">
                    <input
                      type="checkbox"
                      aria-label={`Column ${idx + 1} primary key`}
                      checked={col.primaryKey}
                      onChange={(e) => updateCreateColumn(idx, { primaryKey: e.target.checked })}
                    />
                    <span>PK</span>
                  </label>
                  <label
                    className="checkbox-inline"
                    title={
                      isPostgres
                        ? "GENERATED BY DEFAULT AS IDENTITY"
                        : "AUTO_INCREMENT (requires PK)"
                    }
                  >
                    <input
                      type="checkbox"
                      aria-label={`Column ${idx + 1} auto increment`}
                      checked={col.autoIncrement}
                      onChange={(e) => updateCreateColumn(idx, { autoIncrement: e.target.checked })}
                    />
                    <span>{isPostgres ? "Identity" : "Auto-inc"}</span>
                  </label>
                  <label className="checkbox-inline" title="Allow NULL">
                    <input
                      type="checkbox"
                      aria-label={`Column ${idx + 1} nullable`}
                      checked={col.nullable}
                      disabled={col.primaryKey || col.autoIncrement}
                      onChange={(e) => updateCreateColumn(idx, { nullable: e.target.checked })}
                    />
                    <span>NULL</span>
                  </label>
                  <button
                    type="button"
                    className="ghost-button small"
                    aria-label={`Remove column ${idx + 1}`}
                    disabled={columns.length <= 1}
                    onClick={() => handleRemoveCreateColumn(idx)}
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
              ))}
            </div>
          </>
        )}

        {request.kind === "addColumn" && (
          <>
            <div className="field-grid">
              <label>
                <span>Target table</span>
                <input value={`${request.table.schema}.${request.table.name}`} readOnly disabled />
              </label>
              <label>
                <span>Column name</span>
                <input
                  aria-label="Column name"
                  value={colName}
                  onChange={(e) => setColName(e.target.value)}
                  placeholder="status"
                />
              </label>
            </div>
            <div className="field-grid">
              <label>
                <span>Data type</span>
                <input
                  aria-label="Column data type"
                  list="table-column-types"
                  value={colDataType}
                  onChange={(e) => setColDataType(e.target.value)}
                  placeholder="text"
                />
              </label>
              <label>
                <span>Default expression (optional)</span>
                <input
                  aria-label="Column default expression"
                  value={colDefault}
                  disabled={colAutoIncrement}
                  onChange={(e) => setColDefault(e.target.value)}
                  placeholder="'active'"
                />
              </label>
            </div>
            <div className="field-grid">
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  aria-label="Nullable column"
                  checked={colNullable}
                  disabled={colPrimaryKey || colAutoIncrement}
                  onChange={(e) => setColNullable(e.target.checked)}
                />
                <span>Allow NULL values</span>
              </label>
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  aria-label="Auto increment / identity"
                  checked={colAutoIncrement}
                  onChange={(e) => {
                    const checked = e.target.checked;
                    setColAutoIncrement(checked);
                    if (checked) {
                      setColNullable(false);
                      setColDefault("");
                      if (!isPostgres) setColPrimaryKey(true);
                    }
                  }}
                />
                <span>{isPostgres ? "GENERATED BY DEFAULT AS IDENTITY" : "AUTO_INCREMENT"}</span>
              </label>
            </div>
          </>
        )}

        {request.kind === "alterColumn" && (
          <>
            <div className="field-grid">
              <label>
                <span>Target column</span>
                <input
                  value={`${request.table.schema}.${request.table.name}.${request.column.name}`}
                  readOnly
                  disabled
                />
              </label>
              <label>
                <span>Column name (edit to rename)</span>
                <input
                  aria-label="Column name"
                  value={colName}
                  onChange={(e) => setColName(e.target.value)}
                />
              </label>
            </div>
            <div className="field-grid">
              <label>
                <span>Data type</span>
                <input
                  aria-label="Column data type"
                  list="table-column-types"
                  value={colDataType}
                  onChange={(e) => setColDataType(e.target.value)}
                />
              </label>
              <label>
                <span>Default expression (leave empty to drop default)</span>
                <input
                  aria-label="Column default expression"
                  value={colDefault}
                  onChange={(e) => setColDefault(e.target.value)}
                  placeholder="Empty drops DEFAULT"
                />
              </label>
            </div>
            <label className="checkbox-row">
              <input
                type="checkbox"
                aria-label="Nullable column"
                checked={colNullable}
                onChange={(e) => setColNullable(e.target.checked)}
              />
              <span>Allow NULL values</span>
            </label>
            {isPostgres && (
              <label>
                <span>PostgreSQL USING cast expression (optional)</span>
                <input
                  aria-label="PostgreSQL USING cast expression"
                  value={colUsingExpr}
                  onChange={(e) => setColUsingExpr(e.target.value)}
                  placeholder={`"${request.column.name}"::${colDataType || "text"}`}
                />
              </label>
            )}
          </>
        )}

        {request.kind === "dropColumn" && (
          <>
            <p>
              Drop column <strong>{request.column.name}</strong> ({request.column.databaseType})
              from{" "}
              <code>
                {request.table.schema}.{request.table.name}
              </code>
              .
            </p>
            {isPostgres && (
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  aria-label="Drop column CASCADE"
                  checked={cascade}
                  onChange={(e) => setCascade(e.target.checked)}
                />
                <span>
                  CASCADE (also drop views, indexes, triggers, or constraints depending on this
                  column)
                </span>
              </label>
            )}
          </>
        )}

        {request.kind === "dropObject" && (
          <>
            <p>
              Drop {request.object.kind === "view" ? "view" : "table"}{" "}
              <code>
                {request.object.schema}.{request.object.name}
              </code>
              .
            </p>
            {isPostgres && (
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  aria-label="Drop object CASCADE"
                  checked={cascade}
                  onChange={(e) => setCascade(e.target.checked)}
                />
                <span>CASCADE (also drop dependent views, foreign keys, or routines)</span>
              </label>
            )}
          </>
        )}

        {preview && preview.warnings.length > 0 && (
          <div className="warning-list" role="status">
            {preview.warnings.map((w) => (
              <div className="warning-item" key={w}>
                <AlertTriangle size={14} />
                <span>{w}</span>
              </div>
            ))}
          </div>
        )}

        {previewError && (
          <div className="error-banner" role="alert">
            <AlertTriangle size={14} />
            <span>{previewError}</span>
          </div>
        )}

        {preview && (
          <div className="ddl-preview-panel">
            <div className="structure-section-header">
              <strong>Generated DDL ({preview.statements.length} statement(s))</strong>
              {preview.statements.some((s) => s.destructive) && (
                <span className="pill danger">Destructive</span>
              )}
            </div>
            <pre className="sql-preview" aria-label="Generated DDL preview">
              {preview.script}
            </pre>
          </div>
        )}

        <div className="modal-actions">
          <button type="button" className="ghost-button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="ghost-button"
            disabled={!preview}
            onClick={() => void handleCopyDdl()}
          >
            <Copy size={13} />
            Copy DDL
          </button>
          <button
            type="button"
            className="ghost-button"
            disabled={!preview}
            onClick={handleOpenInEditor}
          >
            <ExternalLink size={13} />
            Open in SQL editor
          </button>
          <button
            type="button"
            className="primary-button"
            disabled={!preview || !session || isReadOnly || preparing}
            onClick={() => void handlePrepareAndReview()}
          >
            <ShieldCheck size={14} />
            {preparing ? "Preparing…" : "Review & authorize…"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
