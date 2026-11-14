import { useEffect, useRef, useState, type FormEvent } from "react";
import { Modal, InlineError } from "../../components/ui/Modal";
import { client, messageOf } from "../../ipc/client";
import type {
  ColumnInspection,
  ConnectionProfile,
  PreparedPlan,
  SnowflakeDdlAction,
  SnowflakeDdlColumnDraft,
} from "../../ipc/generated";
import { revokeSchemaSelectionBeforeWork, useStudio } from "../../state/studio";
import { ReviewDialog } from "../safety/ReviewDialog";

export type SnowflakeDdlDialogRequest =
  | { kind: "createTable"; database: string; schema: string }
  | { kind: "createView"; database: string; schema: string }
  | {
      kind: "addColumn";
      browseRevision: string;
      targetId: string;
      inspectionRevision: string;
      qualifiedTarget: string;
    }
  | {
      kind: "alterColumnNullability";
      browseRevision: string;
      targetId: string;
      inspectionRevision: string;
      qualifiedTarget: string;
      columns: ColumnInspection[];
    }
  | {
      kind: "renameColumn";
      browseRevision: string;
      targetId: string;
      inspectionRevision: string;
      qualifiedTarget: string;
      columns: ColumnInspection[];
    }
  | {
      kind: "dropColumn";
      browseRevision: string;
      targetId: string;
      inspectionRevision: string;
      qualifiedTarget: string;
      columns: ColumnInspection[];
    }
  | { kind: "dropTable"; browseRevision: string; targetId: string; qualifiedTarget: string }
  | { kind: "dropView"; browseRevision: string; targetId: string; qualifiedTarget: string };

const TYPES = [
  "NUMBER(38,0)",
  "NUMBER(38,38)",
  "VARCHAR",
  "BOOLEAN",
  "FLOAT",
  "DATE",
  "TIME(9)",
  "TIMESTAMP_NTZ(9)",
  "TIMESTAMP_LTZ(9)",
  "TIMESTAMP_TZ(9)",
  "BINARY",
  "VARIANT",
  "OBJECT",
  "ARRAY",
];

const DIALOG_TITLES: Record<SnowflakeDdlDialogRequest["kind"], string> = {
  createTable: "Create Snowflake table",
  createView: "Create Snowflake view",
  addColumn: "Add nullable Snowflake column",
  alterColumnNullability: "Alter Snowflake column nullability",
  renameColumn: "Rename Snowflake column",
  dropColumn: "Drop Snowflake column",
  dropTable: "Drop Snowflake table",
  dropView: "Drop Snowflake view",
};

/** The owning console's opaque revisions remain authoritative; no primary-session fallback. */
export function SnowflakeDdlDialog({
  sessionId,
  ownerTabId,
  profile,
  request,
  onClose,
}: {
  sessionId: string;
  ownerTabId?: string;
  profile: ConnectionProfile;
  request: SnowflakeDdlDialogRequest;
  onClose(): void;
}) {
  const [schema, setSchema] = useState(
    request.kind === "createTable" || request.kind === "createView" ? request.schema : "",
  );
  const [name, setName] = useState(request.kind === "createView" ? "NEW_VIEW" : "NEW_TABLE");
  const [querySql, setQuerySql] = useState("SELECT 1 AS ID");
  const initialColumn =
    request.kind === "alterColumnNullability" ||
    request.kind === "renameColumn" ||
    request.kind === "dropColumn"
      ? request.columns[0]
      : undefined;
  const [selectedColumnName, setSelectedColumnName] = useState(initialColumn?.name ?? "");
  const [targetNullable, setTargetNullable] = useState(
    initialColumn ? !initialColumn.nullable : false,
  );
  const [renamedColumnName, setRenamedColumnName] = useState(
    initialColumn ? `${initialColumn.name}_RENAMED` : "",
  );
  const [columns, setColumns] = useState<SnowflakeDdlColumnDraft[]>([
    {
      name: request.kind === "addColumn" ? "NEW_COLUMN" : "ID",
      dataType: "NUMBER(38,0)",
      nullable: true,
    },
  ]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [plan, setPlan] = useState<PreparedPlan | null>(null);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const disconnect = () => {
    mounted.current = false;
    void useStudio
      .getState()
      .disconnectQuerySession(sessionId)
      .catch((err) => useStudio.getState().notify(messageOf(err)));
    onClose();
  };
  const update = (index: number, patch: Partial<SnowflakeDdlColumnDraft>) => {
    setColumns((current) =>
      current.map((column, i) => (i === index ? { ...column, ...patch } : column)),
    );
  };
  async function review(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const action: SnowflakeDdlAction = (() => {
        switch (request.kind) {
          case "createTable":
            return { kind: "createTable", database: request.database, schema, name, columns };
          case "createView":
            return { kind: "createView", database: request.database, schema, name, querySql };
          case "addColumn":
            return {
              kind: "addColumn",
              browseRevision: request.browseRevision,
              targetId: request.targetId,
              inspectionRevision: request.inspectionRevision,
              column: { ...columns[0], nullable: true },
            };
          case "alterColumnNullability":
            return {
              kind: "alterColumnNullability",
              browseRevision: request.browseRevision,
              targetId: request.targetId,
              inspectionRevision: request.inspectionRevision,
              columnName: selectedColumnName,
              nullable: targetNullable,
            };
          case "renameColumn":
            return {
              kind: "renameColumn",
              browseRevision: request.browseRevision,
              targetId: request.targetId,
              inspectionRevision: request.inspectionRevision,
              columnName: selectedColumnName,
              newName: renamedColumnName,
            };
          case "dropColumn":
            return {
              kind: "dropColumn",
              browseRevision: request.browseRevision,
              targetId: request.targetId,
              inspectionRevision: request.inspectionRevision,
              columnName: selectedColumnName,
            };
          case "dropTable":
            return {
              kind: "dropTable",
              browseRevision: request.browseRevision,
              targetId: request.targetId,
            };
          case "dropView":
            return {
              kind: "dropView",
              browseRevision: request.browseRevision,
              targetId: request.targetId,
            };
        }
      })();
      revokeSchemaSelectionBeforeWork(sessionId);
      const preflight = await client.call("preflight_snowflake_ddl", {
        request: { sessionId, action },
      });
      if (!mounted.current) return;
      const prepared = await client.call("prepare_snowflake_ddl", {
        sessionId,
        request: { revision: preflight.revision, action, sql: preflight.sql },
      });
      if (mounted.current) setPlan(prepared);
    } catch (err) {
      if (mounted.current) setError(messageOf(err));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  if (plan)
    return (
      <ReviewDialog
        key={plan.id}
        plan={plan}
        profile={profile}
        onClose={() => setPlan(null)}
        onDisconnect={disconnect}
        onExecute={async (approval) => {
          await useStudio.getState().executeSnowflakeDdl(plan, approval, ownerTabId);
          onClose();
        }}
      />
    );
  return (
    <Modal title={DIALOG_TITLES[request.kind]} onClose={onClose} wide busy={busy}>
      <form onSubmit={(event) => void review(event)}>
        <div className="modal-body modal-form">
          <p>
            Use a dedicated disposable schema without concurrent schema changes. DDL cannot be
            rolled back. The generated statement will require separate approval and may incur
            warehouse charges.
          </p>
          {request.kind === "createTable" || request.kind === "createView" ? (
            <>
              <label>
                Database
                <input value={request.database} readOnly />
              </label>
              <label>
                Schema
                <input
                  value={schema}
                  onChange={(event) => setSchema(event.target.value)}
                  required
                  disabled={busy}
                />
              </label>
              <label>
                {request.kind === "createView" ? "View name" : "Table name"}
                <input
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  required
                  disabled={busy}
                />
              </label>
              {request.kind === "createView" && (
                <label>
                  View SELECT query
                  <textarea
                    rows={5}
                    value={querySql}
                    onChange={(event) => setQuerySql(event.target.value)}
                    required
                    disabled={busy}
                  />
                </label>
              )}
            </>
          ) : (
            <p>
              Target: <code>{request.qualifiedTarget}</code>
            </p>
          )}
          {(request.kind === "createTable" || request.kind === "addColumn") && (
            <>
              <datalist id="snowflake-ddl-types">
                {TYPES.map((type) => (
                  <option key={type} value={type} />
                ))}
              </datalist>
              {columns.map((column, index) => (
                <fieldset key={index} disabled={busy}>
                  <legend>Column {index + 1}</legend>
                  <label>
                    Name
                    <input
                      aria-label={`Column ${index + 1} name`}
                      value={column.name}
                      onChange={(event) => update(index, { name: event.target.value })}
                      required
                    />
                  </label>
                  <label>
                    Data type
                    <input
                      aria-label={`Column ${index + 1} data type`}
                      list="snowflake-ddl-types"
                      value={column.dataType}
                      onChange={(event) => update(index, { dataType: event.target.value })}
                      required
                    />
                  </label>
                  {request.kind === "createTable" && (
                    <>
                      <label>
                        <input
                          type="checkbox"
                          checked={column.nullable}
                          onChange={(event) => update(index, { nullable: event.target.checked })}
                        />
                        Allow NULL in column {index + 1}
                      </label>
                      <button
                        type="button"
                        disabled={columns.length === 1}
                        onClick={() =>
                          setColumns((current) => current.filter((_, i) => i !== index))
                        }
                      >
                        Remove column {index + 1}
                      </button>
                    </>
                  )}
                </fieldset>
              ))}
              {request.kind === "createTable" && (
                <button
                  type="button"
                  disabled={busy || columns.length >= 256}
                  onClick={() =>
                    setColumns((current) => [
                      ...current,
                      { name: `COLUMN_${current.length + 1}`, dataType: "VARCHAR", nullable: true },
                    ])
                  }
                >
                  Add column
                </button>
              )}
              <p>
                Supported Snowflake scalar, temporal, binary, and semi-structured types only.
                Defaults, identity, keys, comments and expressions are unavailable. Added columns
                allow NULL.
              </p>
            </>
          )}
          {(request.kind === "alterColumnNullability" ||
            request.kind === "renameColumn" ||
            request.kind === "dropColumn") && (
            <>
              <label>
                Target column
                <select
                  aria-label="Target column"
                  value={selectedColumnName}
                  disabled={busy}
                  onChange={(event) => {
                    const nextName = event.target.value;
                    setSelectedColumnName(nextName);
                    const found = request.columns.find((col) => col.name === nextName);
                    if (found) {
                      setTargetNullable(!found.nullable);
                      setRenamedColumnName(`${found.name}_RENAMED`);
                    }
                  }}
                >
                  {request.columns.map((col) => (
                    <option key={col.name} value={col.name}>
                      {col.name} ({col.fullType}, {col.nullable ? "NULL" : "NOT NULL"})
                    </option>
                  ))}
                </select>
              </label>
              {request.kind === "alterColumnNullability" && (
                <label>
                  <input
                    type="checkbox"
                    checked={targetNullable}
                    disabled={busy}
                    onChange={(event) => setTargetNullable(event.target.checked)}
                  />
                  Allow NULL on selected column ({targetNullable ? "DROP NOT NULL" : "SET NOT NULL"}
                  )
                </label>
              )}
              {request.kind === "renameColumn" && (
                <label>
                  New column name
                  <input
                    aria-label="New column name"
                    value={renamedColumnName}
                    onChange={(event) => setRenamedColumnName(event.target.value)}
                    required
                    disabled={busy}
                  />
                </label>
              )}
              {request.kind === "dropColumn" && (
                <p>
                  DROP COLUMN permanently removes the column from the table; transaction rollback
                  cannot recover dropped columns.
                </p>
              )}
            </>
          )}
          {(request.kind === "dropTable" || request.kind === "dropView") && (
            <p>
              {request.kind === "dropTable"
                ? "DROP uses RESTRICT. It can still invalidate dependent views; transaction rollback cannot recover the table. Any Time Travel recovery is a separate operation."
                : "DROP VIEW permanently removes the view definition; dependent views or routines can fail on subsequent execution."}
            </p>
          )}
          <InlineError message={error} />
        </div>
        <footer className="modal-footer">
          <button type="button" className="button danger" onClick={disconnect}>
            Disconnect session
          </button>
          <button type="button" className="button ghost" disabled={busy} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="button primary" disabled={busy}>
            {busy ? "Checking target…" : "Generate & review SQL"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
