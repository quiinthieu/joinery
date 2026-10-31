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
  Zap,
} from "lucide-react";
import type {
  ConnectionProfile,
  DatabaseObjectKind,
  DependencyEdge,
  ObjectDdlStatus,
  ObjectDetails,
  SessionInfo,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";
import { objectTabTitle } from "../connections/explorer-tree";

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
}: {
  connectionId: string | null;
  ddlStatus: ObjectDdlStatus | null;
  loading?: boolean;
  error?: string | null;
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
          <button
            type="button"
            className="button ghost small"
            onClick={() => useStudio.getState().openQuery(connectionId, ddlStatus.ddl)}
          >
            <FileCode2 size={12} />
            Open in SQL editor
          </button>
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
}: {
  connectionId: string | null;
  uses: readonly DependencyEdge[];
  usedBy: readonly DependencyEdge[];
}) {
  const openDep = (edge: DependencyEdge) => {
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

export function ObjectInspectorView({
  tab,
  session,
}: {
  tab: StudioTab;
  profile?: ConnectionProfile;
  session?: SessionInfo;
}) {
  const [details, setDetails] = useState<ObjectDetails | null>(null),
    [loading, setLoading] = useState(false),
    [error, setError] = useState<string | null>(null);

  const target = tab.object;
  const sessionId = session?.id ?? null;
  const supported = Boolean(session?.capabilities.objectExplorer);

  useEffect(() => {
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
    sessionId,
    supported,
    tab.id,
    tab.revision,
    target?.schema,
    target?.name,
    target?.kind,
    target?.identity,
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
          {details.ownerTable && tab.connectionId && (
            <button
              type="button"
              className="button ghost small"
              onClick={() =>
                void useStudio
                  .getState()
                  .openTable(tab.connectionId!, details.ownerTable!, undefined, "structure")
              }
            >
              <Table2 size={12} />
              {`${details.ownerTable.schema}.${details.ownerTable.name}`}
              <ArrowRight size={11} />
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

        {indexMeta && (
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
              <dd className="mono">{indexMeta.predicate ? `WHERE ${indexMeta.predicate}` : "—"}</dd>
            </div>
          </dl>
        )}
      </div>

      <DependencySection
        connectionId={tab.connectionId}
        uses={details.uses}
        usedBy={details.usedBy}
      />

      <section className="object-section" aria-label="Definition">
        <h3 className="object-section-title">DDL Definition</h3>
        <DdlPanel
          connectionId={tab.connectionId}
          ddlStatus={details.ddl}
          loading={loading}
          error={error}
        />
      </section>
    </div>
  );
}
