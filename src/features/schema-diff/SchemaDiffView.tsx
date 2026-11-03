import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import {
  AlertTriangle,
  ArrowLeftRight,
  ArrowUpRight,
  Camera,
  ChevronDown,
  ChevronRight,
  Code2,
  Copy,
  FileCode2,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-react";
import type {
  DatabaseObjectKind,
  DatabaseObjectRef,
  EngineKind,
  SchemaDdlPlan,
  SchemaDiffKind,
  SchemaDiffReport,
  SchemaMemberCategory,
  SchemaObjectDiff,
  SchemaSnapshot,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { InlineError } from "../../components/ui/Modal";
import { useStudio, type StudioTab } from "../../state/studio";

export const MAX_CAPTURED_SCHEMA_SNAPSHOTS = 16;

export interface CapturedSchemaSnapshot {
  id: string;
  connectionId: string | null;
  connectionName: string;
  capturedAtIso: string;
  snapshot: SchemaSnapshot;
}

class SchemaSnapshotStore {
  private items: CapturedSchemaSnapshot[] = [];
  private listeners = new Set<() => void>();

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  snapshot = () => this.items;

  capture(
    connectionId: string | null,
    connectionName: string,
    snapshot: SchemaSnapshot,
  ): CapturedSchemaSnapshot {
    const entry: CapturedSchemaSnapshot = {
      id: `snap-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      connectionId,
      connectionName,
      capturedAtIso: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
      snapshot,
    };
    this.items = [entry, ...this.items].slice(0, MAX_CAPTURED_SCHEMA_SNAPSHOTS);
    this.emit();
    return entry;
  }

  remove(id: string) {
    this.items = this.items.filter((item) => item.id !== id);
    this.emit();
  }

  clear() {
    if (this.items.length === 0) return;
    this.items = [];
    this.emit();
  }

  private emit() {
    for (const listener of this.listeners) listener();
  }
}

export const schemaSnapshots = new SchemaSnapshotStore();

type DiffSideTarget =
  { mode: "live"; connectionId: string; schema: string } | { mode: "snapshot"; snapshotId: string };

type DiffKindFilter = "all" | SchemaDiffKind;

export type SchemaDdlMode =
  "none" | "migrate_left_to_right" | "migrate_right_to_left" | "full_left" | "full_right";

const OBJECT_KIND_LABELS: Record<DatabaseObjectKind, string> = {
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

const MEMBER_CATEGORY_LABELS: Record<SchemaMemberCategory, string> = {
  column: "Column",
  constraint: "Constraint",
  index: "Index",
  trigger: "Trigger",
  definition: "Definition",
  property: "Property",
};

function diffKindBadgeLabel(kind: SchemaDiffKind): string {
  switch (kind) {
    case "modified":
      return "Modified";
    case "onlyLeft":
      return "Only in left";
    case "onlyRight":
      return "Only in right";
  }
}

function diffKindBadgeClass(kind: SchemaDiffKind): string {
  switch (kind) {
    case "modified":
      return "compare-status-badge different";
    case "onlyLeft":
      return "compare-status-badge onlyLeft";
    case "onlyRight":
      return "compare-status-badge onlyRight";
  }
}

function objectDiffKey(entry: SchemaObjectDiff): string {
  return `${entry.objectKind}:${entry.name}:${entry.identity ?? ""}`;
}

export function filterSchemaDiffEntries(
  entries: readonly SchemaObjectDiff[],
  kindFilter: DiffKindFilter,
  query: string,
): SchemaObjectDiff[] {
  const trimmed = query.trim().toLowerCase();
  return entries.filter((entry) => {
    if (kindFilter !== "all" && entry.kind !== kindFilter) return false;
    if (!trimmed) return true;
    if (
      entry.name.toLowerCase().includes(trimmed) ||
      entry.quotedName.toLowerCase().includes(trimmed) ||
      entry.summary.toLowerCase().includes(trimmed) ||
      OBJECT_KIND_LABELS[entry.objectKind].toLowerCase().includes(trimmed)
    ) {
      return true;
    }
    return entry.memberDiffs.some(
      (member) =>
        member.name.toLowerCase().includes(trimmed) ||
        member.quotedName.toLowerCase().includes(trimmed) ||
        (member.leftValue?.toLowerCase().includes(trimmed) ?? false) ||
        (member.rightValue?.toLowerCase().includes(trimmed) ?? false),
    );
  });
}

export function SchemaDiffView({
  tab,
  onOpenObject,
}: {
  tab: StudioTab;
  onOpenObject?(connectionId: string, object: DatabaseObjectRef): void;
}) {
  const profiles = useStudio((s) => s.bootstrap?.profiles ?? []);
  const sessions = useStudio((s) => s.sessions);
  const catalog = useStudio((s) => s.catalog);
  const objects = useStudio((s) => s.objects);
  const notify = useStudio((s) => s.notify);
  const openQuery = useStudio((s) => s.openQuery);
  const captured = useSyncExternalStore(schemaSnapshots.subscribe, schemaSnapshots.snapshot);

  const primaryConnectionId = tab.connectionId ?? "";
  const primarySession = primaryConnectionId ? sessions[primaryConnectionId] : undefined;
  const targetEngine: EngineKind | null =
    primarySession?.engine ??
    profiles.find((p) => p.id === primaryConnectionId)?.engine ??
    Object.values(sessions)[0]?.engine ??
    null;

  const compatibleConnections = useMemo(() => {
    return Object.entries(sessions)
      .filter(([, sess]) => !targetEngine || sess.engine === targetEngine)
      .map(([connId, sess]) => {
        const profile = profiles.find((p) => p.id === connId);
        const schemaSet = new Set<string>();
        if (profile?.database && sess.engine === "mysql") {
          schemaSet.add(profile.database);
        }
        for (const table of catalog[connId] ?? []) {
          schemaSet.add(table.table.schema);
        }
        for (const obj of objects[connId] ?? []) {
          schemaSet.add(obj.object.schema);
        }
        if (connId === primaryConnectionId && tab.diagramSchema) {
          schemaSet.add(tab.diagramSchema);
        }
        if (schemaSet.size === 0) {
          schemaSet.add(sess.engine === "mysql" ? (profile?.database ?? "mysql") : "public");
        }
        return {
          connectionId: connId,
          connectionName: profile?.name ?? connId,
          session: sess,
          schemas: [...schemaSet].sort(),
        };
      });
  }, [catalog, objects, primaryConnectionId, profiles, sessions, tab.diagramSchema, targetEngine]);

  const compatibleSnapshots = useMemo(
    () => captured.filter((item) => !targetEngine || item.snapshot.engine === targetEngine),
    [captured, targetEngine],
  );

  const initialLeftSchema = tab.diagramSchema || compatibleConnections[0]?.schemas[0] || "public";
  const initialRightSchema = useMemo(() => {
    const firstConn = compatibleConnections[0];
    if (!firstConn) return initialLeftSchema;
    const other = firstConn.schemas.find((s) => s !== initialLeftSchema);
    return other ?? initialLeftSchema;
  }, [compatibleConnections, initialLeftSchema]);

  const [leftTarget, setLeftTarget] = useState<DiffSideTarget>(() => ({
    mode: "live",
    connectionId: primaryConnectionId || compatibleConnections[0]?.connectionId || "",
    schema: initialLeftSchema,
  }));
  const [rightTarget, setRightTarget] = useState<DiffSideTarget>(() => ({
    mode: "live",
    connectionId: primaryConnectionId || compatibleConnections[0]?.connectionId || "",
    schema: initialRightSchema,
  }));

  const [report, setReport] = useState<SchemaDiffReport | null>(null);
  const [resolvedPair, setResolvedPair] = useState<{
    left: SchemaSnapshot;
    right: SchemaSnapshot;
  } | null>(null);
  const [ddlMode, setDdlMode] = useState<SchemaDdlMode>("none");
  const [ddlPlan, setDdlPlan] = useState<SchemaDdlPlan | null>(null);
  const [ddlLoading, setDdlLoading] = useState(false);
  const [loading, setLoading] = useState(false);
  const [capturingSide, setCapturingSide] = useState<"left" | "right" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [kindFilter, setKindFilter] = useState<DiffKindFilter>("all");
  const [search, setSearch] = useState("");
  const [expandedKeys, setExpandedKeys] = useState<Set<string>>(() => new Set());
  const requestSeq = useRef(0);
  const ddlRequestSeq = useRef(0);

  async function resolveSideSnapshot(target: DiffSideTarget): Promise<SchemaSnapshot> {
    if (target.mode === "snapshot") {
      const found = compatibleSnapshots.find((item) => item.id === target.snapshotId);
      if (!found) {
        throw new Error("The selected captured schema snapshot is no longer available.");
      }
      return found.snapshot;
    }
    const conn = compatibleConnections.find((c) => c.connectionId === target.connectionId);
    if (!conn) {
      throw new Error("Connect a compatible database session before snapshotting this schema.");
    }
    const schemaName = target.schema.trim();
    if (!schemaName) {
      throw new Error("Enter a schema name to snapshot.");
    }
    return client.call<SchemaSnapshot>("schema_snapshot", {
      sessionId: conn.session.id,
      schema: schemaName,
    });
  }

  async function runComparison(customLeft = leftTarget, customRight = rightTarget) {
    const reqId = ++requestSeq.current;
    setLoading(true);
    setError(null);
    try {
      const [leftSnap, rightSnap] = await Promise.all([
        resolveSideSnapshot(customLeft),
        resolveSideSnapshot(customRight),
      ]);
      if (reqId !== requestSeq.current) return;
      const nextReport = await client.call<SchemaDiffReport>("compare_schemas", {
        left: leftSnap,
        right: rightSnap,
      });
      if (reqId !== requestSeq.current) return;
      setResolvedPair({ left: leftSnap, right: rightSnap });
      setReport(nextReport);
      setExpandedKeys(
        new Set(
          nextReport.entries
            .filter((e) => e.kind === "modified")
            .slice(0, 25)
            .map(objectDiffKey),
        ),
      );
      if (ddlMode !== "none") {
        void loadDdlPreview(ddlMode, { left: leftSnap, right: rightSnap });
      }
    } catch (failure) {
      if (reqId !== requestSeq.current) return;
      setReport(null);
      setResolvedPair(null);
      setDdlPlan(null);
      setError(messageOf(failure));
    } finally {
      if (reqId === requestSeq.current) setLoading(false);
    }
  }

  async function loadDdlPreview(
    mode: SchemaDdlMode,
    pair: { left: SchemaSnapshot; right: SchemaSnapshot } | null = resolvedPair,
  ) {
    if (mode === "none" || !pair) {
      setDdlPlan(null);
      return;
    }
    const reqId = ++ddlRequestSeq.current;
    setDdlLoading(true);
    try {
      let plan: SchemaDdlPlan;
      if (mode === "migrate_left_to_right") {
        plan = await client.call<SchemaDdlPlan>("generate_migration_ddl", {
          left: pair.left,
          right: pair.right,
          targetSchema: pair.left.schema,
        });
      } else if (mode === "migrate_right_to_left") {
        plan = await client.call<SchemaDdlPlan>("generate_migration_ddl", {
          left: pair.right,
          right: pair.left,
          targetSchema: pair.right.schema,
        });
      } else if (mode === "full_left") {
        plan = await client.call<SchemaDdlPlan>("generate_schema_ddl", {
          snapshot: pair.left,
          targetSchema: pair.left.schema,
        });
      } else {
        plan = await client.call<SchemaDdlPlan>("generate_schema_ddl", {
          snapshot: pair.right,
          targetSchema: pair.right.schema,
        });
      }
      if (reqId !== ddlRequestSeq.current) return;
      setDdlPlan(plan);
    } catch (failure) {
      if (reqId !== ddlRequestSeq.current) return;
      setDdlPlan(null);
      setError(messageOf(failure));
    } finally {
      if (reqId === ddlRequestSeq.current) setDdlLoading(false);
    }
  }

  function handleSelectDdlMode(nextMode: SchemaDdlMode) {
    const toggled = ddlMode === nextMode ? "none" : nextMode;
    setDdlMode(toggled);
    void loadDdlPreview(toggled, resolvedPair);
  }

  async function handleCopyDdl() {
    if (!ddlPlan?.script) return;
    try {
      await navigator.clipboard?.writeText(ddlPlan.script);
      notify("Copied DDL script to clipboard");
    } catch {
      notify("Could not copy DDL to clipboard");
    }
  }

  function handleOpenDdlInEditor() {
    if (!ddlPlan?.script) return;
    const preferredConnId =
      (ddlMode === "migrate_right_to_left" || ddlMode === "full_right"
        ? rightTarget.mode === "live"
          ? rightTarget.connectionId
          : null
        : leftTarget.mode === "live"
          ? leftTarget.connectionId
          : null) ??
      primaryConnectionId ??
      compatibleConnections[0]?.connectionId ??
      null;
    openQuery(preferredConnId, ddlPlan.script);
    notify(`Opened DDL script for ${ddlPlan.targetSchema} in SQL editor`);
  }

  async function handleCaptureSide(side: "left" | "right") {
    const target = side === "left" ? leftTarget : rightTarget;
    if (target.mode !== "live") return;
    const conn = compatibleConnections.find((c) => c.connectionId === target.connectionId);
    if (!conn) return;
    setCapturingSide(side);
    try {
      const snap = await resolveSideSnapshot(target);
      const saved = schemaSnapshots.capture(conn.connectionId, conn.connectionName, snap);
      notify(`Captured snapshot for ${snap.schema} (${saved.capturedAtIso})`);
    } catch (failure) {
      setError(messageOf(failure));
    } finally {
      setCapturingSide(null);
    }
  }

  function handleSwapSides() {
    const prevLeft = leftTarget;
    const prevRight = rightTarget;
    setLeftTarget(prevRight);
    setRightTarget(prevLeft);
    void runComparison(prevRight, prevLeft);
  }

  useEffect(() => {
    if (compatibleConnections.length === 0 && compatibleSnapshots.length < 2) return;
    void runComparison();
    return () => {
      requestSeq.current++;
    };
  }, [primarySession?.id, tab.diagramSchema]);

  const filteredEntries = useMemo(
    () => (report ? filterSchemaDiffEntries(report.entries, kindFilter, search) : []),
    [report, kindFilter, search],
  );

  function renderSideSelector(
    side: "left" | "right",
    target: DiffSideTarget,
    setTarget: (next: DiffSideTarget) => void,
  ) {
    const sideLabel = side === "left" ? "Left" : "Right";
    const selectValue =
      target.mode === "snapshot" ? `snapshot:${target.snapshotId}` : `live:${target.connectionId}`;

    return (
      <div className="schema-diff-side-card">
        <div className="schema-diff-side-header">
          <span className="eyebrow">{sideLabel.toUpperCase()} SCHEMA</span>
          {target.mode === "live" && (
            <button
              type="button"
              className="button ghost small"
              aria-label={`Capture ${side} schema snapshot`}
              disabled={loading || capturingSide !== null || !target.schema.trim()}
              onClick={() => void handleCaptureSide(side)}
            >
              <Camera size={12} />
              {capturingSide === side ? "Capturing…" : "Capture snapshot"}
            </button>
          )}
        </div>
        <div className="schema-diff-side-controls">
          <label className="schema-diff-field">
            <span>Source</span>
            <select
              aria-label={`${sideLabel} schema source`}
              value={selectValue}
              onChange={(event) => {
                const val = event.target.value;
                if (val.startsWith("snapshot:")) {
                  setTarget({ mode: "snapshot", snapshotId: val.slice("snapshot:".length) });
                } else if (val.startsWith("live:")) {
                  const connId = val.slice("live:".length);
                  const conn = compatibleConnections.find((c) => c.connectionId === connId);
                  setTarget({
                    mode: "live",
                    connectionId: connId,
                    schema: conn?.schemas[0] ?? "public",
                  });
                }
              }}
            >
              {compatibleConnections.map((conn) => (
                <option key={`live:${conn.connectionId}`} value={`live:${conn.connectionId}`}>
                  Live · {conn.connectionName}
                </option>
              ))}
              {compatibleSnapshots.map((snap) => (
                <option key={`snapshot:${snap.id}`} value={`snapshot:${snap.id}`}>
                  Snapshot · {snap.connectionName} · {snap.snapshot.schema} ({snap.capturedAtIso})
                </option>
              ))}
            </select>
          </label>
          {target.mode === "live" && (
            <label className="schema-diff-field">
              <span>Schema / database</span>
              <div className="schema-diff-schema-input-row">
                <input
                  type="text"
                  aria-label={`${sideLabel} schema name`}
                  placeholder="Schema name"
                  value={target.schema}
                  list={`schema-diff-${side}-datalist`}
                  onChange={(event) =>
                    setTarget({
                      ...target,
                      schema: event.target.value,
                    })
                  }
                />
                <datalist id={`schema-diff-${side}-datalist`}>
                  {(
                    compatibleConnections.find((c) => c.connectionId === target.connectionId)
                      ?.schemas ?? []
                  ).map((schemaName) => (
                    <option key={schemaName} value={schemaName} />
                  ))}
                </datalist>
              </div>
            </label>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="utility-view schema-diff-view">
      <header className="utility-header">
        <div>
          <span className="eyebrow">SCHEMA DIFF</span>
          <h1>Compare Schemas &amp; Snapshots</h1>
          <p>
            Deterministic, read-only structural comparison of tables, views, columns, constraints,
            indexes, triggers, routines, and sequences.
          </p>
        </div>
        <div className="diagram-toolbar">
          <button
            type="button"
            className="button ghost"
            aria-label="Swap left and right schemas"
            disabled={loading}
            onClick={handleSwapSides}
          >
            <ArrowLeftRight size={14} />
            Swap sides
          </button>
          <button
            type="button"
            className="button primary"
            aria-label="Compare schemas"
            disabled={loading}
            onClick={() => void runComparison()}
          >
            <RefreshCw size={14} className={loading ? "spin" : ""} />
            {loading ? "Comparing…" : "Compare schemas"}
          </button>
        </div>
      </header>

      <div className="schema-diff-body">
        <div className="schema-diff-sides-grid">
          {renderSideSelector("left", leftTarget, setLeftTarget)}
          {renderSideSelector("right", rightTarget, setRightTarget)}
        </div>

        {compatibleSnapshots.length > 0 && (
          <div className="schema-diff-snapshots-strip" aria-label="Captured schema snapshots">
            <span className="schema-diff-snapshots-label">
              Captured snapshots ({compatibleSnapshots.length}):
            </span>
            <div className="dependency-chip-list">
              {compatibleSnapshots.map((item) => (
                <span key={item.id} className="schema-diff-snapshot-pill">
                  <span>
                    {item.connectionName} · <strong>{item.snapshot.schema}</strong> (
                    {item.snapshot.tables.length} relations)
                  </span>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={`Remove snapshot ${item.snapshot.schema}`}
                    onClick={() => schemaSnapshots.remove(item.id)}
                  >
                    <Trash2 size={11} />
                  </button>
                </span>
              ))}
            </div>
          </div>
        )}

        {error && <InlineError message={error} />}

        {report && (
          <>
            <div className="compare-provenance-bar" role="status">
              <span>
                Left: <strong>{report.leftLabel}</strong> (
                {report.leftServerVersion.split(" ").slice(0, 2).join(" ")})
              </span>
              <span>↔</span>
              <span>
                Right: <strong>{report.rightLabel}</strong> (
                {report.rightServerVersion.split(" ").slice(0, 2).join(" ")})
              </span>
              <span className="import-pill">Identical: {report.identicalCount}</span>
            </div>

            {(report.leftTruncated ||
              report.rightTruncated ||
              report.leftRestricted ||
              report.rightRestricted) && (
              <div className="schema-diff-notice" role="note">
                <AlertTriangle size={14} />
                <span>
                  {report.leftTruncated || report.rightTruncated
                    ? "One or both schemas exceeded the snapshot object budget and were truncated. "
                    : ""}
                  {report.leftRestricted || report.rightRestricted
                    ? "Some columns, constraints, or routine/view definitions were withheld by database privileges."
                    : ""}
                </span>
              </div>
            )}

            {(report.leftUnsupported.length > 0 || report.rightUnsupported.length > 0) && (
              <section
                className="schema-diff-unsupported"
                aria-label="Unsupported schema objects disclosure"
              >
                <div className="schema-diff-unsupported-head">
                  <AlertTriangle size={13} />
                  <strong>
                    Unsupported objects disclosed (
                    {report.leftUnsupported.length + report.rightUnsupported.length})
                  </strong>
                </div>
                <ul className="schema-diff-unsupported-list">
                  {report.leftUnsupported.map((obj) => (
                    <li key={`left:${obj.objectType}:${obj.name}`}>
                      <code>{obj.name}</code> ({obj.objectType} · left) — {obj.reason}
                    </li>
                  ))}
                  {report.rightUnsupported.map((obj) => (
                    <li key={`right:${obj.objectType}:${obj.name}`}>
                      <code>{obj.name}</code> ({obj.objectType} · right) — {obj.reason}
                    </li>
                  ))}
                </ul>
              </section>
            )}

            <div className="compare-tabs-row">
              <div
                className="explorer-filters compare-filter-tabs"
                role="group"
                aria-label="DDL generator mode"
              >
                {(
                  [
                    {
                      id: "migrate_left_to_right",
                      label: "Migrate Left → Right DDL",
                    },
                    {
                      id: "migrate_right_to_left",
                      label: "Migrate Right → Left DDL",
                    },
                    {
                      id: "full_left",
                      label: "Full Left Schema DDL",
                    },
                    {
                      id: "full_right",
                      label: "Full Right Schema DDL",
                    },
                  ] as const
                ).map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    className={`explorer-filter-chip ${ddlMode === item.id ? "active" : ""}`}
                    aria-pressed={ddlMode === item.id}
                    disabled={ddlLoading || loading}
                    onClick={() => handleSelectDdlMode(item.id)}
                  >
                    <Code2 size={12} />
                    {item.label}
                  </button>
                ))}
              </div>
            </div>

            {ddlMode !== "none" && ddlPlan && (
              <section
                className="structure-section schema-ddl-preview"
                aria-label="Generated DDL preview"
                data-testid="schema-ddl-preview"
              >
                <div className="schema-diff-card-header">
                  <div className="schema-diff-title-static">
                    <FileCode2 size={14} />
                    <strong>DDL Preview · {ddlPlan.targetSchema}</strong>
                  </div>
                  <span className="sql-nav-badge">
                    {ddlPlan.statements.length}{" "}
                    {ddlPlan.statements.length === 1 ? "statement" : "statements"}
                  </span>
                  <span
                    className={
                      ddlPlan.destructiveCount > 0
                        ? "compare-status-badge onlyLeft"
                        : "compare-status-badge identical"
                    }
                  >
                    {ddlPlan.destructiveCount > 0
                      ? `${ddlPlan.destructiveCount} destructive`
                      : "Non-destructive"}
                  </span>
                  <div className="diagram-toolbar" style={{ marginLeft: "auto" }}>
                    <button
                      type="button"
                      className="button ghost small"
                      aria-label="Copy generated DDL"
                      disabled={!ddlPlan.script}
                      onClick={() => void handleCopyDdl()}
                    >
                      <Copy size={12} />
                      Copy DDL
                    </button>
                    <button
                      type="button"
                      className="button ghost small"
                      aria-label="Open generated DDL in SQL editor"
                      disabled={!ddlPlan.script}
                      onClick={handleOpenDdlInEditor}
                    >
                      <ArrowUpRight size={12} />
                      Open in SQL editor
                    </button>
                  </div>
                </div>

                {ddlPlan.warnings.length > 0 && (
                  <ul className="schema-diff-unsupported-list" aria-label="DDL warnings">
                    {ddlPlan.warnings.map((warning, idx) => (
                      <li key={idx}>
                        <AlertTriangle size={12} /> {warning}
                      </li>
                    ))}
                  </ul>
                )}

                {ddlPlan.statements.length === 0 ? (
                  <div className="compare-empty-diff">
                    No DDL statements needed — schemas are structurally identical.
                  </div>
                ) : (
                  <pre className="sql-code-block" data-testid="schema-ddl-script">
                    <code>{ddlPlan.script}</code>
                  </pre>
                )}
              </section>
            )}

            <div className="compare-tabs-row">
              <div
                className="explorer-filters compare-filter-tabs"
                role="group"
                aria-label="Diff kind filter"
              >
                {(
                  [
                    { id: "all", label: `All differences (${report.entries.length})` },
                    { id: "modified", label: `Modified (${report.modifiedCount})` },
                    { id: "onlyLeft", label: `Only in left (${report.onlyLeftCount})` },
                    { id: "onlyRight", label: `Only in right (${report.onlyRightCount})` },
                  ] as const
                ).map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    className={`explorer-filter-chip ${kindFilter === item.id ? "active" : ""}`}
                    aria-pressed={kindFilter === item.id}
                    onClick={() => setKindFilter(item.id)}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
              <label className="diagram-filter">
                <Search size={13} />
                <input
                  aria-label="Filter schema differences"
                  placeholder="Filter objects or members…"
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                />
              </label>
            </div>

            {filteredEntries.length === 0 ? (
              <div className="compare-empty-diff">
                {report.entries.length === 0
                  ? `No structural differences found across ${report.identicalCount} compared objects.`
                  : "No schema differences match the current filter."}
              </div>
            ) : (
              <div className="schema-diff-list" role="list">
                {filteredEntries.map((entry) => {
                  const key = objectDiffKey(entry);
                  const hasMembers = entry.memberDiffs.length > 0;
                  const expanded = hasMembers && expandedKeys.has(key);
                  const liveSide =
                    entry.kind === "onlyRight"
                      ? rightTarget.mode === "live"
                        ? rightTarget
                        : null
                      : leftTarget.mode === "live"
                        ? leftTarget
                        : rightTarget.mode === "live"
                          ? rightTarget
                          : null;

                  return (
                    <article
                      key={key}
                      role="listitem"
                      className="schema-diff-card"
                      data-testid={`schema-diff-object-${entry.name}`}
                    >
                      <div className="schema-diff-card-header">
                        {hasMembers ? (
                          <button
                            type="button"
                            className="schema-diff-expand"
                            aria-expanded={expanded}
                            aria-label={`${expanded ? "Collapse" : "Expand"} ${entry.name} changes`}
                            onClick={() =>
                              setExpandedKeys((old) => {
                                const next = new Set(old);
                                if (next.has(key)) next.delete(key);
                                else next.add(key);
                                return next;
                              })
                            }
                          >
                            {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                            <code>{entry.quotedName}</code>
                          </button>
                        ) : (
                          <div className="schema-diff-title-static">
                            <code>{entry.quotedName}</code>
                          </div>
                        )}
                        <span className="sql-nav-badge">
                          {OBJECT_KIND_LABELS[entry.objectKind]}
                        </span>
                        <span className={diffKindBadgeClass(entry.kind)}>
                          {diffKindBadgeLabel(entry.kind)}
                        </span>
                        <span className="schema-diff-summary">{entry.summary}</span>
                        {liveSide && onOpenObject && (
                          <button
                            type="button"
                            className="button ghost small"
                            onClick={() =>
                              onOpenObject(liveSide.connectionId, {
                                schema: liveSide.schema.trim(),
                                name: entry.name,
                                kind: entry.objectKind,
                                identity: entry.identity,
                              })
                            }
                          >
                            <ArrowUpRight size={12} />
                            Inspect
                          </button>
                        )}
                      </div>

                      {expanded && (
                        <div className="structure-section schema-diff-members">
                          <table
                            className="structure-table"
                            aria-label={`Member differences for ${entry.name}`}
                          >
                            <thead>
                              <tr>
                                <th>Category</th>
                                <th>Member</th>
                                <th>Change</th>
                                <th>Left</th>
                                <th>Right</th>
                              </tr>
                            </thead>
                            <tbody>
                              {entry.memberDiffs.map((member) => (
                                <tr key={`${member.category}:${member.name}`}>
                                  <td>{MEMBER_CATEGORY_LABELS[member.category]}</td>
                                  <td>
                                    <code>{member.quotedName}</code>
                                  </td>
                                  <td>
                                    <span className={diffKindBadgeClass(member.kind)}>
                                      {diffKindBadgeLabel(member.kind)}
                                    </span>
                                  </td>
                                  <td>
                                    {member.leftValue ? (
                                      <code>{member.leftValue}</code>
                                    ) : (
                                      <span className="muted">—</span>
                                    )}
                                  </td>
                                  <td>
                                    {member.rightValue ? (
                                      <code>{member.rightValue}</code>
                                    ) : (
                                      <span className="muted">—</span>
                                    )}
                                  </td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        </div>
                      )}
                    </article>
                  );
                })}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
