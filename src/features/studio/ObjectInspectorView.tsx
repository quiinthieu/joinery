import { useEffect, useState } from "react";
import {
  ArrowRight,
  Braces,
  Copy,
  Database,
  Eye,
  FileCode2,
  Hash,
  Layers,
  ListTree,
  Lock,
  Table2,
  Trash2,
  Wrench,
  Zap,
} from "lucide-react";
import type {
  ConnectionProfile,
  DatabaseObjectKind,
  DatabaseObjectRef,
  DependencyEdge,
  ObjectDdlStatus,
  ObjectDetails,
  SessionInfo,
  TableInfo,
  TableRef,
  TableStorageStats,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";
import { objectTabTitle } from "../connections/explorer-tree";
import { IndexConstraintDialog, type IndexConstraintDialogMode } from "./IndexConstraintDialog";
import { TableMaintenanceDialog } from "./TableMaintenanceDialog";
import {
  deadTupleSummary,
  describeIndexUsage,
  formatStatCount,
  formatStorageBytes,
} from "./storage-stats";

const KIND_LABELS: Record<DatabaseObjectKind, string> = {
  table: "Table",
  partitionedTable: "Partitioned table",
  view: "View",
  materializedView: "Materialized view",
  function: "Function",
  procedure: "Procedure",
  sequence: "Sequence",
  trigger: "Trigger",
  index: "Index",
};

export function ObjectKindGlyph({ kind, size = 13 }: { kind: DatabaseObjectKind; size?: number }) {
  switch (kind) {
    case "table":
    case "partitionedTable":
      return <Table2 size={size} />;
    case "view":
      return <Eye size={size} />;
    case "materializedView":
      return <Layers size={size} />;
    case "function":
    case "procedure":
      return <Braces size={size} />;
    case "sequence":
      return <Hash size={size} />;
    case "trigger":
      return <Zap size={size} />;
    case "index":
      return <ListTree size={size} />;
  }
}

export function DdlPanel({
  connectionId,
  ddlStatus,
  loading,
  error,
  onOpenInEditor,
  openLabel = "Open in SQL editor",
}: {
  connectionId: string | null;
  ddlStatus: ObjectDdlStatus | null;
  loading?: boolean;
  error?: string | null;
  onOpenInEditor?: (ddl: string) => void;
  openLabel?: string;
}) {
  if (loading && !ddlStatus) {
    return <div className="grid-empty">Loading DDL definition…</div>;
  }
  if (error && !ddlStatus) {
    return (
      <div className="object-inspector-scroll">
        <div className="inline-error" role="alert">
          {error}
        </div>
      </div>
    );
  }
  if (!ddlStatus) {
    return <div className="grid-empty">Connect to inspect object DDL.</div>;
  }
  if (ddlStatus.status === "unavailable") {
    return (
      <div className="ddl-panel">
        <div className="ddl-unavailable" role="note">
          <Lock size={15} />
          <div>
            <strong>DDL unavailable</strong>
            <p>{ddlStatus.reason}</p>
          </div>
        </div>
      </div>
    );
  }
  const copyDdl = () => {
    void navigator.clipboard
      .writeText(ddlStatus.ddl)
      .then(() => useStudio.getState().notify("Copied DDL to clipboard"))
      .catch((err) => useStudio.getState().notify(messageOf(err)));
  };
  return (
    <div className="ddl-panel">
      <div className="ddl-toolbar">
        <div className="ddl-toolbar-meta">
          <span className="eyebrow">READ-ONLY DEFINITION</span>
          {ddlStatus.truncated && (
            <span className="truncated-label">Truncated to 256 KiB preview limit</span>
          )}
        </div>
        <div className="ddl-toolbar-actions">
          <button type="button" className="button ghost small" onClick={copyDdl}>
            <Copy size={12} />
            Copy DDL
          </button>
          {(onOpenInEditor || connectionId) && (
            <button
              type="button"
              className="button ghost small"
              onClick={() =>
                onOpenInEditor
                  ? onOpenInEditor(ddlStatus.ddl)
                  : useStudio.getState().openQuery(connectionId, ddlStatus.ddl)
              }
            >
              <FileCode2 size={12} />
              {openLabel}
            </button>
          )}
        </div>
      </div>
      <pre className="ddl-code" aria-label="Object DDL">
        <code>{ddlStatus.ddl}</code>
      </pre>
    </div>
  );
}

export function DependencySection({
  connectionId,
  uses,
  usedBy,
  onOpenObject,
}: {
  connectionId: string | null;
  uses: readonly DependencyEdge[];
  usedBy: readonly DependencyEdge[];
  onOpenObject?: (object: DatabaseObjectRef) => void;
}) {
  const openDep = (edge: DependencyEdge) => {
    if (onOpenObject) return onOpenObject(edge.object);
    if (!connectionId) return;
    void useStudio
      .getState()
      .openObject(connectionId, edge.object, "structure")
      .catch((err) => useStudio.getState().notify(messageOf(err)));
  };

  return (
    <section className="object-section" aria-label="Object dependencies">
      <h3 className="object-section-title">Dependencies</h3>
      <div className="dependency-grid">
        <div className="dependency-column">
          <div className="dependency-column-head">
            <span>Uses</span>
            <span className="schema-count">{uses.length}</span>
          </div>
          {uses.length === 0 ? (
            <div className="dependency-empty">No outbound object dependencies</div>
          ) : (
            <div className="dependency-chip-list">
              {uses.map((edge) => {
                const label = `${edge.object.schema}.${objectTabTitle(edge.object)}`;
                return (
                  <button
                    type="button"
                    key={`uses:${edge.object.schema}:${edge.object.kind}:${edge.object.name}:${edge.object.identity ?? ""}:${edge.relation}`}
                    className="dependency-chip"
                    disabled={!onOpenObject && !connectionId}
                    onClick={() => openDep(edge)}
                    title={`Open ${KIND_LABELS[edge.object.kind].toLowerCase()} ${label} (${edge.relation})`}
                  >
                    <ObjectKindGlyph kind={edge.object.kind} size={12} />
                    <span className="mono">{label}</span>
                    <small>{edge.relation}</small>
                  </button>
                );
              })}
            </div>
          )}
        </div>
        <div className="dependency-column">
          <div className="dependency-column-head">
            <span>Used by</span>
            <span className="schema-count">{usedBy.length}</span>
          </div>
          {usedBy.length === 0 ? (
            <div className="dependency-empty">No inbound dependent objects</div>
          ) : (
            <div className="dependency-chip-list">
              {usedBy.map((edge) => {
                const label = `${edge.object.schema}.${objectTabTitle(edge.object)}`;
                return (
                  <button
                    type="button"
                    key={`usedBy:${edge.object.schema}:${edge.object.kind}:${edge.object.name}:${edge.object.identity ?? ""}:${edge.relation}`}
                    className="dependency-chip"
                    disabled={!onOpenObject && !connectionId}
                    onClick={() => openDep(edge)}
                    title={`Open ${KIND_LABELS[edge.object.kind].toLowerCase()} ${label} (${edge.relation})`}
                  >
                    <ObjectKindGlyph kind={edge.object.kind} size={12} />
                    <span className="mono">{label}</span>
                    <small>{edge.relation}</small>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

const EMPTY_CATALOG_TABLES: readonly TableInfo[] = [];

export function ObjectInspectorView({
  tab,
  profile,
  session,
  providedDetails,
  onOpenObject,
  onOpenInEditor,
  openLabel,
}: {
  providedDetails?: ObjectDetails;
  tab: StudioTab;
  profile?: ConnectionProfile;
  session?: SessionInfo;
  onOpenObject?: (object: DatabaseObjectRef) => void;
  onOpenInEditor?: (ddl: string) => void;
  openLabel?: string;
}) {
  const [loadedDetails, setDetails] = useState<ObjectDetails | null>(null),
    [loading, setLoading] = useState(false),
    [error, setError] = useState<string | null>(null),
    [editorMode, setEditorMode] = useState<IndexConstraintDialogMode | null>(null),
    [refreshNonce, setRefreshNonce] = useState(0);
  const storeProfile = useStudio((s) =>
    tab.connectionId
      ? (s.bootstrap?.profiles.find((p) => p.id === tab.connectionId) ?? null)
      : null,
  );
  const activeProfile = profile ?? storeProfile;
  const catalogTables = useStudio((s) =>
    tab.connectionId ? (s.catalog[tab.connectionId] ?? EMPTY_CATALOG_TABLES) : EMPTY_CATALOG_TABLES,
  );

  const details = providedDetails ?? loadedDetails;
  const target = tab.object;
  const sessionId = session?.id ?? null;
  const supported = Boolean(session?.capabilities.objectExplorer);

  useEffect(() => {
    if (providedDetails) return;
    if (!target || !sessionId || !supported) {
      setDetails(null);
      setError(null);
      setLoading(false);
      return;
    }
    let active = true;
    setLoading(true);
    setError(null);
    void client
      .call<ObjectDetails>("object_details", { sessionId, object: target })
      .then((result) => {
        if (active) setDetails(result);
      })
      .catch((err) => {
        if (active) {
          setDetails(null);
          setError(messageOf(err));
        }
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [
    providedDetails,
    sessionId,
    supported,
    tab.id,
    tab.revision,
    target?.schema,
    target?.name,
    target?.kind,
    target?.identity,
    refreshNonce,
  ]);

  if (!target) {
    return <div className="grid-empty">No database object selected.</div>;
  }
  if (!session) {
    return (
      <div className="utility-empty">
        <Database size={26} />
        <h3>This connection is offline</h3>
        <p>Reconnect to inspect {objectTabTitle(target)}.</p>
      </div>
    );
  }
  if (!supported) {
    return (
      <div className="utility-empty">
        <Database size={26} />
        <h3>Extended object details unavailable</h3>
        <p>This database engine does not expose extended object inspection.</p>
      </div>
    );
  }
  if (tab.view === "ddl") {
    return (
      <div className="object-inspector-scroll">
        <DdlPanel
          connectionId={tab.connectionId}
          ddlStatus={details?.ddl ?? null}
          loading={loading}
          error={error}
          onOpenInEditor={onOpenInEditor}
          openLabel={openLabel}
        />
      </div>
    );
  }
  if (loading && !details) {
    return <div className="grid-empty">Loading object metadata…</div>;
  }
  if (error && !details) {
    return (
      <div className="object-inspector-scroll">
        <div className="inline-error" role="alert">
          {error}
        </div>
      </div>
    );
  }
  if (!details) {
    return <div className="grid-empty">No metadata available.</div>;
  }

  const triggerMeta = details.triggers[0] ?? null;
  const indexMeta = details.indexes[0] ?? null;

  return (
    <div className="object-inspector-scroll">
      <div className="object-summary-card">
        <div className="object-summary-head">
          <span className="key-badge">{KIND_LABELS[details.object.kind].toUpperCase()}</span>
          <h2 className="mono">{`${details.object.schema}.${objectTabTitle(details.object)}`}</h2>
          {details.ownerTable && (onOpenObject || tab.connectionId) && (
            <button
              type="button"
              className="button ghost small"
              onClick={() =>
                onOpenObject
                  ? onOpenObject({ ...details.ownerTable!, kind: "table", identity: null })
                  : void useStudio
                      .getState()
                      .openTable(tab.connectionId!, details.ownerTable!, undefined, "structure")
              }
            >
              <Table2 size={12} />
              {`${details.ownerTable.schema}.${details.ownerTable.name}`}
              <ArrowRight size={11} />
            </button>
          )}
          {details.object.kind === "index" &&
            details.ownerTable &&
            indexMeta &&
            !indexMeta.primary &&
            tab.connectionId &&
            activeProfile &&
            (session?.capabilities.ddlEditing ??
              (activeProfile.engine === "postgres" || activeProfile.engine === "mysql")) && (
              <button
                type="button"
                className="button ghost small danger-text"
                disabled={activeProfile.safeMode === "readOnly"}
                title={
                  activeProfile.safeMode === "readOnly"
                    ? "Connection is in read-only safe mode"
                    : `Drop index ${indexMeta.name}`
                }
                onClick={() =>
                  setEditorMode({
                    kind: "dropIndex",
                    table: details.ownerTable!,
                    index: indexMeta,
                  })
                }
              >
                <Trash2 size={12} />
                Drop index…
              </button>
            )}
        </div>
        {details.comment && <p className="object-comment">{details.comment}</p>}

        {details.routine && (
          <dl className="object-meta-grid" aria-label="Routine properties">
            <div>
              <dt>Arguments</dt>
              <dd className="mono">{details.routine.arguments || "()"}</dd>
            </div>
            <div>
              <dt>Identity signature</dt>
              <dd className="mono">{`(${details.routine.identityArguments})`}</dd>
            </div>
            <div>
              <dt>Return type</dt>
              <dd className="mono">{details.routine.returnType ?? "void / procedure"}</dd>
            </div>
            <div>
              <dt>Language</dt>
              <dd className="mono">{details.routine.language}</dd>
            </div>
            <div>
              <dt>Volatility</dt>
              <dd>{details.routine.volatility ?? "—"}</dd>
            </div>
            <div>
              <dt>Security</dt>
              <dd>{details.routine.securityDefiner ? "SECURITY DEFINER" : "SECURITY INVOKER"}</dd>
            </div>
          </dl>
        )}

        {details.sequence && (
          <dl className="object-meta-grid" aria-label="Sequence properties">
            <div>
              <dt>Data type</dt>
              <dd className="mono">{details.sequence.dataType}</dd>
            </div>
            <div>
              <dt>Last value</dt>
              <dd className="mono">
                {details.sequence.lastValue ?? "Not readable with current privileges"}
              </dd>
            </div>
            <div>
              <dt>Start / Increment</dt>
              <dd className="mono">{`${details.sequence.startValue} / +${details.sequence.incrementBy}`}</dd>
            </div>
            <div>
              <dt>Min / Max</dt>
              <dd className="mono">{`${details.sequence.minValue} .. ${details.sequence.maxValue}`}</dd>
            </div>
            <div>
              <dt>Cache</dt>
              <dd className="mono">{details.sequence.cacheSize}</dd>
            </div>
            <div>
              <dt>Cycle</dt>
              <dd>{details.sequence.cycle ? "Yes (cycles)" : "No"}</dd>
            </div>
          </dl>
        )}

        {triggerMeta && (
          <dl className="object-meta-grid" aria-label="Trigger properties">
            <div>
              <dt>Target table</dt>
              <dd className="mono">{triggerMeta.tableName}</dd>
            </div>
            <div>
              <dt>Timing</dt>
              <dd>{triggerMeta.timing}</dd>
            </div>
            <div>
              <dt>Events</dt>
              <dd>{triggerMeta.events.join(" OR ")}</dd>
            </div>
            <div>
              <dt>Orientation</dt>
              <dd>{`FOR EACH ${triggerMeta.orientation}`}</dd>
            </div>
            <div>
              <dt>Status</dt>
              <dd>
                <span className={`state-pill ${triggerMeta.enabled ? "active" : ""}`}>
                  {triggerMeta.enabled ? "Enabled" : "Disabled"}
                </span>
              </dd>
            </div>
          </dl>
        )}

        {indexMeta &&
          (() => {
            const usage = describeIndexUsage(indexMeta);
            return (
              <dl className="object-meta-grid" aria-label="Index properties">
                <div>
                  <dt>Method</dt>
                  <dd className="mono">{indexMeta.method}</dd>
                </div>
                <div>
                  <dt>Uniqueness</dt>
                  <dd>
                    {indexMeta.primary ? "PRIMARY KEY" : indexMeta.unique ? "UNIQUE" : "Non-unique"}
                  </dd>
                </div>
                <div>
                  <dt>Keys / Expressions</dt>
                  <dd className="mono">{indexMeta.expressions.join(", ") || "—"}</dd>
                </div>
                <div>
                  <dt>Partial predicate</dt>
                  <dd className="mono">
                    {indexMeta.predicate ? `WHERE ${indexMeta.predicate}` : "—"}
                  </dd>
                </div>
                <div>
                  <dt>Size</dt>
                  <dd className="mono">{formatStorageBytes(indexMeta.sizeBytes)}</dd>
                </div>
                <div>
                  <dt>Scans / usage</dt>
                  <dd>
                    {usage.unused ? (
                      <span className="lock-badge waiting">{usage.label}</span>
                    ) : (
                      <span className="mono">
                        {usage.label}
                        {usage.detail ? ` · ${usage.detail}` : ""}
                      </span>
                    )}
                  </dd>
                </div>
                <div>
                  <dt>Estimated cardinality</dt>
                  <dd className="mono">{formatStatCount(indexMeta.cardinality)}</dd>
                </div>
              </dl>
            );
          })()}
      </div>

      {details.storage && (
        <StorageStatsSection
          storage={details.storage}
          connectionId={tab.connectionId}
          table={
            details.object.kind === "table" || details.object.kind === "partitionedTable"
              ? { schema: details.object.schema, name: details.object.name }
              : null
          }
          onMaintained={() => setRefreshNonce((n) => n + 1)}
        />
      )}

      <DependencySection
        connectionId={tab.connectionId}
        uses={details.uses}
        usedBy={details.usedBy}
        onOpenObject={onOpenObject}
      />

      <section className="object-section" aria-label="Definition">
        <h3 className="object-section-title">DDL Definition</h3>
        <DdlPanel
          connectionId={tab.connectionId}
          ddlStatus={details.ddl}
          loading={loading}
          error={error}
          onOpenInEditor={onOpenInEditor}
          openLabel={openLabel}
        />
      </section>
      {editorMode && activeProfile && session && (
        <IndexConstraintDialog
          mode={editorMode}
          schema={null}
          details={details}
          profile={activeProfile}
          session={session}
          catalogTables={catalogTables}
          onClose={() => setEditorMode(null)}
          onSuccess={(msg) => {
            useStudio.getState().notify(msg);
            if (tab.connectionId) void useStudio.getState().refreshCatalog(tab.connectionId);
            void useStudio.getState().closeTab(tab.id);
          }}
        />
      )}
    </div>
  );
}

export function StorageStatsSection({
  storage,
  connectionId = null,
  table = null,
  columns = [],
  onMaintained,
}: {
  storage: TableStorageStats;
  connectionId?: string | null;
  table?: TableRef | null;
  columns?: readonly string[];
  onMaintained?: () => void;
}) {
  const [maintenanceOpen, setMaintenanceOpen] = useState(false);
  const profile = useStudio((s) =>
    connectionId ? (s.bootstrap?.profiles.find((p) => p.id === connectionId) ?? null) : null,
  );
  const session = useStudio((s) => (connectionId ? (s.sessions[connectionId] ?? null) : null));
  const canMaintainEngine =
    Boolean(connectionId && table) &&
    (profile?.engine === "postgres" ||
      profile?.engine === "mysql" ||
      session?.engine === "postgres" ||
      session?.engine === "mysql");
  const isReadOnly = profile?.safeMode === "readOnly";
  const hasMetrics =
    storage.totalBytes !== null ||
    storage.tableBytes !== null ||
    storage.indexBytes !== null ||
    storage.estimatedRows !== null ||
    storage.liveTuples !== null ||
    storage.sequentialScans !== null;
  const tuples = deadTupleSummary(storage);
  const indexShare =
    storage.totalBytes &&
    storage.totalBytes > 0 &&
    storage.indexBytes !== null &&
    storage.indexBytes !== undefined
      ? ` (${Math.round((storage.indexBytes / storage.totalBytes) * 100)}%)`
      : "";
  return (
    <section className="object-section storage-stats-section" aria-label="Storage and activity">
      <div className="storage-stats-head">
        <div>
          <h3 className="object-section-title">Storage &amp; activity</h3>
          <p className="storage-stats-note">
            Planner and collector estimates only — Joinery never runs automatic COUNT(*) or ANALYZE.
          </p>
        </div>
        <div className="structure-toolbar-actions">
          {storage.statsStale && (
            <span className="lock-badge waiting" data-testid="storage-stats-stale">
              Stale / unanalyzed estimate
            </span>
          )}
          {canMaintainEngine && connectionId && table && (
            <button
              type="button"
              className="button ghost small"
              aria-label="Table maintenance"
              disabled={!session || isReadOnly}
              title={
                isReadOnly
                  ? "Connection is in read-only safe mode"
                  : !session
                    ? "Connect this profile to run table maintenance"
                    : "Run reviewed ANALYZE / VACUUM / OPTIMIZE maintenance"
              }
              onClick={() => setMaintenanceOpen(true)}
            >
              <Wrench size={12} />
              Maintenance…
            </button>
          )}
        </div>
      </div>
      {hasMetrics && (
        <dl className="object-meta-grid" aria-label="Storage breakdown">
          <div>
            <dt>Total footprint</dt>
            <dd className="mono">{formatStorageBytes(storage.totalBytes)}</dd>
          </div>
          <div>
            <dt>Table / heap</dt>
            <dd className="mono">{formatStorageBytes(storage.tableBytes)}</dd>
          </div>
          <div>
            <dt>Indexes</dt>
            <dd className="mono">{`${formatStorageBytes(storage.indexBytes)}${indexShare}`}</dd>
          </div>
          <div>
            <dt>Estimated rows</dt>
            <dd className="mono">
              {storage.estimatedRows !== null
                ? formatStatCount(storage.estimatedRows)
                : "Unanalyzed (no estimate)"}
            </dd>
          </div>
          {storage.freeBytes !== null && storage.freeBytes !== undefined && (
            <div>
              <dt>Free / reclaimable</dt>
              <dd className="mono">{formatStorageBytes(storage.freeBytes)}</dd>
            </div>
          )}
          {tuples && (
            <div>
              <dt>Live / dead tuples</dt>
              <dd className="mono">
                {tuples.label}
                {tuples.bloated && (
                  <>
                    {" "}
                    <span className="lock-badge blocker">High dead tuples</span>
                  </>
                )}
              </dd>
            </div>
          )}
          {(storage.sequentialScans !== null || storage.indexScans !== null) && (
            <div>
              <dt>Table vs. index scans</dt>
              <dd className="mono">
                {`${formatStatCount(storage.sequentialScans)} seq · ${formatStatCount(storage.indexScans)} index`}
              </dd>
            </div>
          )}
          {(storage.lastAnalyze !== null || storage.statsStale || storage.liveTuples !== null) && (
            <div>
              <dt>Last analyze / update</dt>
              <dd className="mono">{storage.lastAnalyze ?? "Never analyzed"}</dd>
            </div>
          )}
          {(storage.lastVacuum !== null || storage.liveTuples !== null) && (
            <div>
              <dt>Last vacuum</dt>
              <dd className="mono">{storage.lastVacuum ?? "Never vacuumed"}</dd>
            </div>
          )}
        </dl>
      )}
      {storage.restrictedReason && (
        <div className="ddl-unavailable storage-stats-restricted" role="note">
          <Lock size={15} />
          <div>
            <strong>Statistics restricted or unavailable</strong>
            <p>{storage.restrictedReason}</p>
          </div>
        </div>
      )}
      {maintenanceOpen && connectionId && table && (
        <TableMaintenanceDialog
          connectionId={connectionId}
          table={table}
          columns={columns}
          storage={storage}
          onClose={() => setMaintenanceOpen(false)}
          onCompleted={() => onMaintained?.()}
        />
      )}
    </section>
  );
}
