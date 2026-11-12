import { useEffect, useId, useRef, useState } from "react";
import type {
  BrowseCatalog,
  RelationCheckConstraints,
  RelationConstraintMappings,
  RelationConstraints,
  RelationDefinition,
  RelationInspection,
  RelationPreview,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { querySessionForTab, revokeSchemaSelectionBeforeWork, useStudio } from "../../state/studio";
import { Modal } from "../../components/ui/Modal";
import { SnowflakeDdlDialog, type SnowflakeDdlDialogRequest } from "./SnowflakeDdlDialog";
import { DdlPanel } from "./ObjectInspectorView";

const PAGE_ROWS = 25;
const PAGE_COLUMNS = 20;

function quotedName(...parts: string[]) {
  return parts.map((part) => `"${part.replaceAll('"', '""')}"`).join(".");
}

type KeyMapping = RelationConstraintMappings["mappings"][number];

function referencedDestination(catalog: BrowseCatalog | null, mapping: KeyMapping) {
  const reference = mapping.referencedTable;
  if (!catalog || mapping.kind !== "foreignKey" || mapping.status !== "available" || !reference)
    return null;
  if (reference.database !== catalog.database)
    return {
      reason:
        "The referenced table is in another database. Cross-database navigation is not available yet.",
      target: null,
    };
  // Count every exact identity before availability; a hidden/shadowed duplicate
  // must never let an available sibling win by elimination.
  const matches = catalog.relations.filter(
    (entry) =>
      entry.database === reference.database &&
      entry.table.schema === reference.table.schema &&
      entry.table.name === reference.table.name,
  );
  if (matches.length === 0)
    return {
      reason:
        "The referenced table is absent from this retained catalog. Refresh relations explicitly to check visibility.",
      target: null,
    };
  if (matches.length !== 1)
    return { reason: "The referenced table identity is ambiguous in this catalog.", target: null };
  if (matches[0].availability !== "available")
    return {
      reason: `The referenced table is unavailable (${matches[0].availability}).`,
      target: null,
    };
  return { reason: null, target: matches[0] };
}

function relationContextKey(
  state: ReturnType<typeof useStudio.getState>,
  sessionId: string,
  ownerTabId?: string,
) {
  const owner = state.tabs.find((tab) => tab.id === ownerTabId);
  const owned = owner && querySessionForTab(owner, state.sessions);
  const session =
    owned?.id === sessionId
      ? owned
      : Object.values(state.sessions).find((entry) => entry.id === sessionId);
  const runtime = state.sessionStates[sessionId];
  return JSON.stringify([
    sessionId,
    session?.id,
    session?.connectionId,
    !ownerTabId || owned?.id === sessionId,
    !!(
      state.sessionTransactionPending[sessionId] ||
      state.sessionMetadataRefreshRequired[sessionId] ||
      state.sessionDdlCatalogRefreshRequired[sessionId] ||
      state.sessionContextSwitching[sessionId] ||
      runtime?.transactionControlAuthority?.status === "rollbackOnly"
    ),
    state.sessionContextRevision[sessionId] ?? 0,
    // The first background observation installs an unknown SDK runtime. It
    // supplies no new context, so it must not revoke an already admitted read.
    runtime?.database ?? "",
    runtime?.activeContext ?? null,
    runtime?.role ?? null,
    runtime?.searchPath ?? [],
    runtime?.contextFingerprint ?? null,
  ]);
}

/** Every action carries server-held target revisions from this console session. */
export function RelationBrowser({
  sessionId,
  ownerTabId,
  onClose,
}: {
  sessionId: string;
  ownerTabId?: string;
  onClose(): void;
}) {
  const session = useStudio((state) => {
    const owner = state.tabs.find((tab) => tab.id === ownerTabId);
    const owned = owner && querySessionForTab(owner, state.sessions);
    return owned?.id === sessionId
      ? owned
      : Object.values(state.sessions).find((entry) => entry.id === sessionId);
  });
  const profile = useStudio((state) =>
    state.bootstrap?.profiles.find((entry) => entry.id === session?.connectionId),
  );
  const blocked = useStudio(
    (state) =>
      !!(
        state.sessionTransactionPending[sessionId] ||
        state.sessionMetadataRefreshRequired[sessionId] ||
        state.sessionDdlCatalogRefreshRequired[sessionId] ||
        state.sessionContextSwitching[sessionId] ||
        state.sessionStates[sessionId]?.transactionControlAuthority?.status === "rollbackOnly"
      ),
  );
  const ownerAvailable = useStudio((state) => {
    if (!ownerTabId) return true;
    const owner = state.tabs.find((tab) => tab.id === ownerTabId);
    return !!owner && querySessionForTab(owner, state.sessions)?.id === sessionId;
  });
  const contextKey = useStudio((state) => relationContextKey(state, sessionId, ownerTabId));
  const renderedOwner = ownerTabId
    ? useStudio.getState().tabs.find((tab) => tab.id === ownerTabId)
    : undefined;
  const canDdl =
    session?.capabilities.snowflakeReviewedDdl === true &&
    profile?.snowflake?.reviewedDdl === true &&
    profile.safeMode !== "readOnly";
  const [ddlRequest, setDdlRequest] = useState<SnowflakeDdlDialogRequest | null>(null);
  const [catalog, setCatalog] = useState<BrowseCatalog | null>(null);
  const [inspection, setInspection] = useState<RelationInspection | null>(null);
  const [preview, setPreview] = useState<RelationPreview | null>(null);
  const [definition, setDefinition] = useState<{
    contextKey: string;
    value: RelationDefinition;
  } | null>(null);
  const [constraints, setConstraints] = useState<{
    contextKey: string;
    value: RelationConstraints;
  } | null>(null);
  const [constraintsError, setConstraintsError] = useState<string | null>(null);
  const [mappings, setMappings] = useState<{
    contextKey: string;
    value: RelationConstraintMappings;
  } | null>(null);
  const [mappingsError, setMappingsError] = useState<string | null>(null);
  const [checks, setChecks] = useState<{
    contextKey: string;
    value: RelationCheckConstraints;
  } | null>(null);
  const [checksError, setChecksError] = useState<string | null>(null);
  const [selected, setSelected] = useState("");
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [rowPage, setRowPage] = useState(0);
  const [columnPage, setColumnPage] = useState(0);
  const generation = useRef(0);
  const activationEpoch = generation.current;
  const navigationId = useId();
  const relationSelector = useRef<HTMLSelectElement>(null);
  const retainedSelection = useRef({ catalog, mappings, selected, sessionId, ownerTabId });
  retainedSelection.current = { catalog, mappings, selected, sessionId, ownerTabId };
  const mounted = useRef(false);
  const renderedContext = useRef(contextKey);
  renderedContext.current = contextKey;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      generation.current++;
    };
  }, []);
  useEffect(() => {
    const invalidate = () => {
      generation.current++;
      setCatalog(null);
      setInspection(null);
      setPreview(null);
      setDefinition(null);
      setConstraints(null);
      setConstraintsError(null);
      setMappings(null);
      setMappingsError(null);
      setChecks(null);
      setChecksError(null);
      setSelected("");
      setBusy(false);
      setError(null);
    };
    invalidate();
    // Observe every transition, including a hold set and cleared within one
    // React batch. A later response cannot revive its earlier authority.
    return useStudio.subscribe((next, previous) => {
      if (
        relationContextKey(next, sessionId, ownerTabId) !==
        relationContextKey(previous, sessionId, ownerTabId)
      )
        invalidate();
    });
  }, [sessionId, ownerTabId]);

  const run = async (
    work: (current: () => boolean) => Promise<void>,
    onFailure?: (message: string) => void,
  ) => {
    const request = ++generation.current;
    const current = () =>
      mounted.current &&
      request === generation.current &&
      renderedContext.current === contextKey &&
      relationContextKey(useStudio.getState(), sessionId, ownerTabId) === contextKey;
    setBusy(true);
    setError(null);
    try {
      revokeSchemaSelectionBeforeWork(sessionId);
      await work(current);
    } catch (err) {
      if (current()) {
        if (onFailure) {
          onFailure(messageOf(err));
          return;
        }
        setCatalog(null);
        setInspection(null);
        setPreview(null);
        setDefinition(null);
        setConstraints(null);
        setConstraintsError(null);
        setMappings(null);
        setMappingsError(null);
        setChecks(null);
        setChecksError(null);
        setSelected("");
        setError(messageOf(err));
      }
    } finally {
      if (current()) setBusy(false);
    }
  };
  const refresh = () => {
    setCatalog(null);
    setInspection(null);
    setPreview(null);
    setDefinition(null);
    setConstraints(null);
    setConstraintsError(null);
    setMappings(null);
    setMappingsError(null);
    setChecks(null);
    setChecksError(null);
    setSelected("");
    setRowPage(0);
    setColumnPage(0);
    void run(async (current) => {
      const result = await client.call("relation_catalog", { sessionId });
      if (current()) setCatalog(result);
    });
  };
  // Opening the browser is explicit user intent to load current visible metadata.
  useEffect(() => {
    // Defer dispatch until after React's development effect replay. One open
    // must not start duplicate catalog reads during StrictMode setup/cleanup.
    const timer = setTimeout(refresh, 0);
    return () => clearTimeout(timer);
  }, [sessionId]);
  const inspect = () => {
    const target = catalog?.relations.find((entry) => entry.targetId === selected);
    if (!catalog || !target || target.availability !== "available") return;
    setInspection(null);
    setPreview(null);
    setRowPage(0);
    setColumnPage(0);
    void run(async (current) => {
      const result = await client.call("inspect_relation", {
        sessionId,
        request: { revision: catalog.revision, targetId: target.targetId },
      });
      if (current()) setInspection(result);
    });
  };
  const loadPreview = () => {
    if (!catalog || !inspection) return;
    setPreview(null);
    setRowPage(0);
    setColumnPage(0);
    void run(async (current) => {
      const result = await client.call("preview_relation", {
        sessionId,
        request: {
          revision: catalog.revision,
          targetId: inspection.targetId,
          inspectionRevision: inspection.revision,
        },
      });
      if (current()) setPreview(result);
    });
  };
  const loadDefinition = () => {
    const target = catalog?.relations.find((entry) => entry.targetId === selected);
    if (!catalog || target?.availability !== "available" || blocked || !ownerAvailable) return;
    setDefinition(null);
    void run(async (current) => {
      const result = await client.call("define_relation", {
        sessionId,
        request: { revision: catalog.revision, targetId: target.targetId },
      });
      if (!current()) return;
      if (
        result.revision !== catalog.revision ||
        result.targetId !== target.targetId ||
        result.database !== catalog.database ||
        result.table.schema !== target.table.schema ||
        result.table.name !== target.table.name
      )
        throw new Error("Relation definition changed. Refresh relations and try again.");
      setDefinition({ contextKey, value: result });
    });
  };
  const loadConstraints = () => {
    const target = catalog?.relations.find((entry) => entry.targetId === selected);
    if (!catalog || target?.availability !== "available" || blocked || !ownerAvailable) return;
    setConstraints(null);
    setConstraintsError(null);
    void run(async (current) => {
      const result = await client.call("inspect_relation_constraints", {
        sessionId,
        request: { revision: catalog.revision, targetId: target.targetId },
      });
      if (!current()) return;
      if (
        result.revision !== catalog.revision ||
        result.targetId !== target.targetId ||
        result.database !== catalog.database ||
        result.table.schema !== target.table.schema ||
        result.table.name !== target.table.name
      )
        throw new Error("Key constraint target changed. Refresh relations and try again.");
      setConstraints({ contextKey, value: result });
    }, setConstraintsError);
  };
  const target = catalog?.relations.find((entry) => entry.targetId === selected);
  const loadChecks = () => {
    if (
      !catalog ||
      target?.availability !== "available" ||
      blocked ||
      !ownerAvailable ||
      activationEpoch !== generation.current ||
      renderedContext.current !== contextKey ||
      relationContextKey(useStudio.getState(), sessionId, ownerTabId) !== contextKey
    )
      return;
    setChecks(null);
    setChecksError(null);
    void run(async (current) => {
      const result = await client.call("inspect_relation_check_constraints", {
        sessionId,
        request: { revision: catalog.revision, targetId: target.targetId },
      });
      if (!current()) return;
      if (
        result.revision !== catalog.revision ||
        result.targetId !== target.targetId ||
        result.database !== catalog.database ||
        result.table.schema !== target.table.schema ||
        result.table.name !== target.table.name ||
        result.constraints.some(
          (constraint) =>
            constraint.catalog !== catalog.database ||
            constraint.schema !== target.table.schema ||
            constraint.tableName !== target.table.name,
        )
      )
        throw new Error("CHECK constraint target changed. Refresh relations and try again.");
      setChecks({ contextKey, value: result });
    }, setChecksError);
  };
  const loadMappings = () => {
    if (!catalog || target?.availability !== "available" || blocked || !ownerAvailable) return;
    setMappings(null);
    setMappingsError(null);
    void run(async (current) => {
      const result = await client.call("inspect_relation_constraint_mappings", {
        sessionId,
        request: { revision: catalog.revision, targetId: target.targetId },
      });
      if (!current()) return;
      if (
        result.revision !== catalog.revision ||
        result.targetId !== target.targetId ||
        result.database !== catalog.database ||
        result.table.schema !== target.table.schema ||
        result.table.name !== target.table.name
      )
        throw new Error("Key column mapping target changed. Refresh relations and try again.");
      if (
        result.mappings.some(
          (mapping) =>
            mapping.status === "available" &&
            (mapping.columns.length === 0 ||
              (mapping.kind === "foreignKey" &&
                (!mapping.referencedTable ||
                  mapping.columns.some((column) => column.referencedColumnName === null)))),
        )
      )
        throw new Error("Key column mappings are incomplete. Refresh relations and try again.");
      setMappings({ contextKey, value: result });
    }, setMappingsError);
  };
  const selectReferencedTable = (mapping: KeyMapping) => {
    const fresh = useStudio.getState();
    const owner = ownerTabId ? fresh.tabs.find((tab) => tab.id === ownerTabId) : undefined;
    const liveSession = ownerTabId
      ? owner && querySessionForTab(owner, fresh.sessions)
      : Object.values(fresh.sessions).find((entry) => entry.id === sessionId);
    const runtime = fresh.sessionStates[sessionId];
    const retained = retainedSelection.current;
    if (
      !mounted.current ||
      activationEpoch !== generation.current ||
      busy ||
      !liveSession ||
      liveSession.id !== sessionId ||
      !liveSession.capabilities.relationBrowsing ||
      (ownerTabId && owner?.revision !== renderedOwner?.revision) ||
      fresh.sessionTransactionPending[sessionId] ||
      fresh.sessionMetadataRefreshRequired[sessionId] ||
      fresh.sessionDdlCatalogRefreshRequired[sessionId] ||
      fresh.sessionContextSwitching[sessionId] ||
      runtime?.transactionControlAuthority?.status === "rollbackOnly" ||
      renderedContext.current !== contextKey ||
      relationContextKey(fresh, sessionId, ownerTabId) !== contextKey ||
      retained.catalog !== catalog ||
      retained.mappings !== mappings ||
      retained.selected !== selected ||
      retained.sessionId !== sessionId ||
      retained.ownerTabId !== ownerTabId ||
      !catalog ||
      !mappings ||
      !target ||
      mappings.contextKey !== contextKey ||
      mappings.value.revision !== catalog.revision ||
      mappings.value.targetId !== target.targetId ||
      mappings.value.database !== catalog.database ||
      mappings.value.table.schema !== target.table.schema ||
      mappings.value.table.name !== target.table.name ||
      target.database !== catalog.database ||
      target.availability !== "available" ||
      !mappings.value.mappings.includes(mapping) ||
      mapping.columns.length === 0 ||
      mapping.columns.some((column) => column.referencedColumnName === null)
    )
      return;
    const destination = referencedDestination(catalog, mapping)?.target;
    if (!destination) return;
    generation.current++;
    setSearch("");
    setDdlRequest(null);
    setSelected(destination.targetId);
    setInspection(null);
    setPreview(null);
    setDefinition(null);
    setConstraints(null);
    setConstraintsError(null);
    setMappings(null);
    setMappingsError(null);
    setChecks(null);
    setChecksError(null);
    setError(null);
    setRowPage(0);
    setColumnPage(0);
    relationSelector.current?.focus();
  };
  const matches =
    catalog?.relations.filter((entry) =>
      `${entry.table.schema}.${entry.table.name}`
        .toLocaleLowerCase()
        .includes(search.toLocaleLowerCase()),
    ) ?? [];
  const columns =
    preview?.columns.slice(columnPage * PAGE_COLUMNS, (columnPage + 1) * PAGE_COLUMNS) ?? [];
  const rows = preview?.rows.slice(rowPage * PAGE_ROWS, (rowPage + 1) * PAGE_ROWS) ?? [];
  const qualified =
    target && catalog
      ? [catalog.database, target.table.schema, target.table.name]
          .map((part) => `"${part.replaceAll('"', '""')}"`)
          .join(".")
      : "";
  if (ddlRequest && profile)
    return (
      <SnowflakeDdlDialog
        sessionId={sessionId}
        ownerTabId={ownerTabId}
        profile={profile}
        request={ddlRequest}
        onClose={onClose}
      />
    );
  return (
    <Modal
      title="Browse relations"
      subtitle="Visible relations in this console’s current database. Read-only preview: up to 500 unordered rows, 2 MiB. Concurrent schema changes can invalidate results."
      wide
      onClose={onClose}
    >
      <div className="modal-body relation-browser">
        <div className="query-actions">
          <button className="button small" onClick={refresh}>
            Refresh relations
          </button>
          <span role="status">
            {busy
              ? "Loading…"
              : catalog
                ? `${catalog.relations.length} visible relations · ${catalog.database}`
                : "Refresh required"}
          </span>
        </div>
        {error && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
        <label>
          Find relation
          <input
            aria-label="Find relation"
            disabled={busy}
            value={search}
            onChange={(event) => {
              generation.current++;
              setSearch(event.target.value);
              setSelected("");
              setInspection(null);
              setPreview(null);
              setDefinition(null);
              setConstraints(null);
              setConstraintsError(null);
              setMappings(null);
              setMappingsError(null);
              setChecks(null);
              setChecksError(null);
              setBusy(false);
            }}
          />
        </label>
        <label>
          Relation
          <select
            ref={relationSelector}
            aria-label="Relation"
            disabled={busy || !catalog}
            value={selected}
            onChange={(event) => {
              generation.current++;
              setSelected(event.target.value);
              setInspection(null);
              setPreview(null);
              setDefinition(null);
              setConstraints(null);
              setConstraintsError(null);
              setMappings(null);
              setMappingsError(null);
              setChecks(null);
              setChecksError(null);
              setBusy(false);
            }}
          >
            <option value="">Select a relation</option>
            {matches.map((entry) => (
              <option key={entry.targetId} value={entry.targetId}>
                {entry.table.schema}.{entry.table.name} · {entry.tableType}
                {entry.traits.temporary ? " · temporary" : ""}
                {entry.availability !== "available" ? ` · ${entry.availability}` : ""}
              </option>
            ))}
          </select>
        </label>
        {target && (
          <p>
            {target.tableType} ·{" "}
            {Object.entries(target.traits)
              .map(
                ([name, value]) => `${name}: ${value === null ? "unknown" : value ? "yes" : "no"}`,
              )
              .join(" · ")}
          </p>
        )}
        {target && target.availability !== "available" && (
          <p role="note">
            Inspection is unavailable for this target ({target.availability}). Refresh after
            resolving the ambiguity or unsupported metadata.
          </p>
        )}
        <div className="query-actions">
          <button
            className="button small"
            disabled={busy || target?.availability !== "available"}
            onClick={inspect}
          >
            Inspect relation
          </button>
          <button className="button small" disabled={busy || !inspection} onClick={loadPreview}>
            Preview rows
          </button>
          <button
            className="button small"
            disabled={busy || blocked || !ownerAvailable || target?.availability !== "available"}
            onClick={loadDefinition}
          >
            View definition
          </button>
          <button
            className="button small"
            disabled={busy || blocked || !ownerAvailable || target?.availability !== "available"}
            onClick={loadConstraints}
          >
            View key constraints
          </button>
          <button
            className="button small"
            disabled={busy || blocked || !ownerAvailable || target?.availability !== "available"}
            onClick={loadMappings}
          >
            View key column mappings
          </button>
          <button
            className="button small"
            disabled={busy || blocked || !ownerAvailable || target?.availability !== "available"}
            onClick={loadChecks}
          >
            View CHECK constraints
          </button>
          {canDdl && (
            <>
              <button
                className="button small"
                disabled={busy || blocked || !catalog}
                onClick={() =>
                  catalog &&
                  setDdlRequest({
                    kind: "createTable",
                    database: catalog.database,
                    schema: useStudio.getState().sessionStates[sessionId]?.activeContext ?? "",
                  })
                }
              >
                Create table…
              </button>
              <button
                className="button small"
                disabled={
                  busy ||
                  blocked ||
                  !catalog ||
                  !inspection ||
                  target?.availability !== "available" ||
                  target.tableType !== "BASE TABLE"
                }
                onClick={() =>
                  catalog &&
                  inspection &&
                  target &&
                  setDdlRequest({
                    kind: "addColumn",
                    browseRevision: catalog.revision,
                    targetId: target.targetId,
                    inspectionRevision: inspection.revision,
                    qualifiedTarget: qualified,
                  })
                }
              >
                Add nullable column…
              </button>
              <button
                className="button small danger"
                disabled={
                  busy ||
                  blocked ||
                  !catalog ||
                  target?.availability !== "available" ||
                  target.tableType !== "BASE TABLE"
                }
                onClick={() =>
                  catalog &&
                  target &&
                  setDdlRequest({
                    kind: "dropTable",
                    browseRevision: catalog.revision,
                    targetId: target.targetId,
                    qualifiedTarget: qualified,
                  })
                }
              >
                Drop table…
              </button>
            </>
          )}
        </div>
        {definition &&
          definition.contextKey === contextKey &&
          !blocked &&
          definition.value.targetId === selected &&
          definition.value.revision === catalog?.revision && (
            <section aria-label="Relation definition">
              <h3>Snowflake relation definition</h3>
              <p role="note">
                Snowflake-generated DDL may use CREATE OR REPLACE, normalize types, or omit hidden
                policy and tag information. It is not proof that replay is safe or complete.
              </p>
              <DdlPanel
                connectionId={null}
                ddlStatus={{ status: "available", ddl: definition.value.ddl, truncated: false }}
              />
            </section>
          )}
        {target?.availability === "available" && !blocked && ownerAvailable && (
          <section aria-label="Relation CHECK constraints">
            <h3>CHECK constraints</h3>
            <p role="note">
              These role-visible CHECK clauses are read-only metadata. They do not report
              enforcement or validation, establish engine eligibility, or grant row editing
              authority. Metadata can lag schema changes.
            </p>
            {checksError ? (
              <p className="inline-error" role="alert">
                CHECK constraints are unavailable: {checksError}
              </p>
            ) : checks &&
              checks.contextKey === contextKey &&
              checks.value.targetId === selected &&
              checks.value.revision === catalog?.revision ? (
              checks.value.constraints.length === 0 ? (
                <p>No visible CHECK constraints returned.</p>
              ) : (
                <div className="relation-table-scroll">
                  <table>
                    <caption>CHECK constraint inventory</caption>
                    <thead>
                      <tr>
                        <th>Constraint</th>
                        <th>Table</th>
                        <th>CHECK clause</th>
                      </tr>
                    </thead>
                    <tbody>
                      {checks.value.constraints.map((constraint) => (
                        <tr
                          key={JSON.stringify([
                            constraint.catalog,
                            constraint.schema,
                            constraint.tableName,
                            constraint.name,
                          ])}
                        >
                          <th scope="row">
                            {quotedName(constraint.catalog, constraint.schema, constraint.name)}
                          </th>
                          <td>
                            {quotedName(
                              constraint.catalog,
                              constraint.schema,
                              constraint.tableName,
                            )}
                          </td>
                          <td>
                            <pre>{constraint.checkClause}</pre>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )
            ) : (
              <p>CHECK constraints have not been fetched.</p>
            )}
          </section>
        )}
        {target?.availability === "available" && !blocked && ownerAvailable && (
          <section aria-label="Relation key constraints">
            <h3>Key constraints</h3>
            <p role="note">
              Visible key declarations may not be enforced and do not establish row editing safety.
              This declaration inventory excludes column mappings, CHECK constraints, and NOT NULL
              constraints; column inspection reports nullability.
            </p>
            {constraintsError ? (
              <p className="inline-error" role="alert">
                Key constraints are unavailable: {constraintsError}
              </p>
            ) : constraints &&
              constraints.contextKey === contextKey &&
              constraints.value.targetId === selected &&
              constraints.value.revision === catalog?.revision ? (
              constraints.value.constraints.length === 0 ? (
                <p>No visible declared primary, unique, or foreign keys were returned.</p>
              ) : (
                <div className="relation-table-scroll">
                  <table>
                    <caption>Key constraint inventory</caption>
                    <thead>
                      <tr>
                        <th>Name</th>
                        <th>Type</th>
                        <th>Enforced</th>
                        <th>Deferrable</th>
                        <th>Initially deferred</th>
                        <th>Rely</th>
                        <th>Comment</th>
                        <th>Referenced constraint</th>
                        <th>Match option</th>
                        <th>Update rule</th>
                        <th>Delete rule</th>
                      </tr>
                    </thead>
                    <tbody>
                      {constraints.value.constraints.map((constraint) => (
                        <tr
                          key={JSON.stringify([
                            constraint.catalog,
                            constraint.schema,
                            constraint.name,
                          ])}
                        >
                          <th scope="row">
                            {[constraint.catalog, constraint.schema, constraint.name]
                              .map((part) => `"${part.replaceAll('"', '""')}"`)
                              .join(".")}
                          </th>
                          <td>
                            {constraint.kind === "primaryKey"
                              ? "Primary key"
                              : constraint.kind === "unique"
                                ? "Unique"
                                : "Foreign key"}
                          </td>
                          <td>{constraint.enforced ? "yes" : "no"}</td>
                          <td>{constraint.isDeferrable ? "yes" : "no"}</td>
                          <td>{constraint.initiallyDeferred ? "yes" : "no"}</td>
                          <td>{constraint.rely ? "yes" : "no"}</td>
                          <td>{constraint.comment ?? "—"}</td>
                          <td>
                            {constraint.reference
                              ? [
                                  constraint.reference.catalog,
                                  constraint.reference.schema,
                                  constraint.reference.name,
                                ]
                                  .map((part) => `"${part.replaceAll('"', '""')}"`)
                                  .join(".")
                              : constraint.referenceStatus === "notVisible"
                                ? "Not visible"
                                : "Not applicable"}
                          </td>
                          <td>{constraint.reference?.matchOption ?? "—"}</td>
                          <td>{constraint.reference?.updateRule ?? "—"}</td>
                          <td>{constraint.reference?.deleteRule ?? "—"}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )
            ) : (
              <p>Key constraints have not been fetched.</p>
            )}
          </section>
        )}
        {target?.availability === "available" && !blocked && ownerAvailable && (
          <section aria-label="Relation key column mappings">
            <h3>Key column mappings</h3>
            <p role="note">
              Visible column mappings can lag schema changes and do not establish row editing
              safety. Foreign key columns are paired with referenced columns in local key order.
            </p>
            {mappingsError ? (
              <p className="inline-error" role="alert">
                Key column mappings are unavailable: {mappingsError}
              </p>
            ) : mappings &&
              mappings.contextKey === contextKey &&
              mappings.value.targetId === selected &&
              mappings.value.revision === catalog?.revision ? (
              mappings.value.mappings.length === 0 ? (
                <p>No visible key column mappings were returned.</p>
              ) : (
                <div className="relation-table-scroll">
                  <table>
                    <caption>Key column mapping inventory</caption>
                    <thead>
                      <tr>
                        <th>Constraint</th>
                        <th>Type</th>
                        <th>Status</th>
                        <th>Ordinal</th>
                        <th>Column</th>
                        <th>Referenced table</th>
                        <th>Referenced column</th>
                        <th>Navigation</th>
                      </tr>
                    </thead>
                    <tbody>
                      {mappings.value.mappings.flatMap((mapping, mappingIndex) => {
                        const identity = quotedName(mapping.catalog, mapping.schema, mapping.name);
                        const kind =
                          mapping.kind === "primaryKey"
                            ? "Primary key"
                            : mapping.kind === "unique"
                              ? "Unique"
                              : "Foreign key";
                        const status =
                          mapping.status === "available"
                            ? "Available"
                            : mapping.status === "notVisible"
                              ? "Not visible"
                              : "Unsupported";
                        const destination = referencedDestination(catalog, mapping);
                        const columns =
                          mapping.status === "available"
                            ? [...mapping.columns].sort(
                                (left, right) => left.ordinal - right.ordinal,
                              )
                            : [null];
                        return columns.map((column, index) => (
                          <tr
                            key={JSON.stringify([
                              mapping.catalog,
                              mapping.schema,
                              mapping.name,
                              column?.ordinal,
                            ])}
                          >
                            <th scope="row">{identity}</th>
                            <td>{kind}</td>
                            <td>{status}</td>
                            <td>{column?.ordinal ?? "—"}</td>
                            <td>{column ? quotedName(column.columnName) : "—"}</td>
                            <td>
                              {mapping.status !== "available"
                                ? "—"
                                : mapping.referencedTable
                                  ? quotedName(
                                      mapping.referencedTable.database,
                                      mapping.referencedTable.table.schema,
                                      mapping.referencedTable.table.name,
                                    )
                                  : "Not applicable"}
                            </td>
                            <td>
                              {column?.referencedColumnName == null
                                ? mapping.status === "available" && mapping.kind !== "foreignKey"
                                  ? "Not applicable"
                                  : "—"
                                : quotedName(column.referencedColumnName)}
                            </td>
                            <td>
                              {destination && mapping.referencedTable && index === 0 ? (
                                <>
                                  <button
                                    className="button small"
                                    aria-label={`Select referenced table ${quotedName(mapping.referencedTable.database, mapping.referencedTable.table.schema, mapping.referencedTable.table.name)}`}
                                    aria-describedby={
                                      destination.reason
                                        ? `${navigationId}-${mappingIndex}`
                                        : undefined
                                    }
                                    disabled={
                                      busy ||
                                      !session?.capabilities.relationBrowsing ||
                                      !destination.target
                                    }
                                    onClick={() => selectReferencedTable(mapping)}
                                  >
                                    Select referenced table
                                  </button>
                                  {destination.reason && (
                                    <p id={`${navigationId}-${mappingIndex}`} role="note">
                                      {destination.reason}
                                    </p>
                                  )}
                                </>
                              ) : (
                                "—"
                              )}
                            </td>
                          </tr>
                        ));
                      })}
                    </tbody>
                  </table>
                </div>
              )
            ) : (
              <p>Key column mappings have not been fetched.</p>
            )}
          </section>
        )}
        {inspection && (
          <>
            <h3>
              {inspection.database}.{inspection.table.schema}.{inspection.table.name}
            </h3>
            <p>Column metadata does not establish edit eligibility.</p>
            <details open={!preview}>
              <summary>{inspection.columns.length} columns</summary>
              <div className="relation-table-scroll">
                <table>
                  <caption>Column inspection</caption>
                  <thead>
                    <tr>
                      <th>Name</th>
                      <th>Type</th>
                      <th>Nullable</th>
                      <th>Snowflake expression default</th>
                      <th>Descriptor default / Iceberg initial default</th>
                      <th>Write default</th>
                      <th>Identity</th>
                      <th>Kind</th>
                      <th>Comment</th>
                    </tr>
                  </thead>
                  <tbody>
                    {inspection.columns.map((column) => (
                      <tr key={column.name}>
                        <th scope="row">{column.name}</th>
                        <td>{column.fullType}</td>
                        <td>{column.nullable ? "yes" : "no"}</td>
                        <td>{column.defaultExpression ?? "—"}</td>
                        <td>{column.descriptorDefaultExpression ?? "—"}</td>
                        <td>
                          {column.writeDefaultStatus === "notReported"
                            ? "not reported"
                            : column.writeDefaultExpression === null
                              ? "NULL"
                              : column.writeDefaultExpression === ""
                                ? "“”"
                                : column.writeDefaultExpression}
                        </td>
                        <td>
                          {column.identity.isIdentity === null
                            ? "unknown"
                            : column.identity.isIdentity
                              ? `${column.identity.generation ?? "identity"}; start ${column.identity.start ?? "unknown"}; increment ${column.identity.increment ?? "unknown"}`
                              : "no"}
                        </td>
                        <td>{column.kind}</td>
                        <td>{column.comment ?? "—"}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </details>
          </>
        )}
        {preview && (
          <>
            <p role="status">
              {preview.rows.length} retained rows
              {preview.truncated ? " · Preview limit reached" : ""}. NULL is shown as ∅; an empty
              string as “”. This unordered sample has no continuation.
            </p>
            <div className="query-actions">
              <button disabled={rowPage === 0} onClick={() => setRowPage(rowPage - 1)}>
                Previous rows
              </button>
              <span>
                Rows {preview.rows.length ? rowPage * PAGE_ROWS + 1 : 0}–
                {Math.min((rowPage + 1) * PAGE_ROWS, preview.rows.length)}
              </span>
              <button
                disabled={(rowPage + 1) * PAGE_ROWS >= preview.rows.length}
                onClick={() => setRowPage(rowPage + 1)}
              >
                Next rows
              </button>
              <button disabled={columnPage === 0} onClick={() => setColumnPage(columnPage - 1)}>
                Previous columns
              </button>
              <span>
                Columns {columnPage * PAGE_COLUMNS + 1}–
                {Math.min((columnPage + 1) * PAGE_COLUMNS, preview.columns.length)}
              </span>
              <button
                disabled={(columnPage + 1) * PAGE_COLUMNS >= preview.columns.length}
                onClick={() => setColumnPage(columnPage + 1)}
              >
                Next columns
              </button>
            </div>
            <div className="relation-table-scroll">
              <table>
                <caption>Read-only preview</caption>
                <thead>
                  <tr>
                    {columns.map((column) => (
                      <th key={column.id}>{column.name}</th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row, index) => (
                    <tr key={rowPage * PAGE_ROWS + index}>
                      {columns.map((column, i) => {
                        const value = row[columnPage * PAGE_COLUMNS + i];
                        return (
                          <td key={column.id}>
                            {value === null ? (
                              <span aria-label="NULL">∅</span>
                            ) : value === "" ? (
                              <span aria-label="Empty string">“”</span>
                            ) : (
                              value
                            )}
                          </td>
                        );
                      })}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}
