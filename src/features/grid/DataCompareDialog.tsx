import { useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowLeftRight,
  Check,
  Database,
  KeyRound,
  Layers,
  Loader2,
  Play,
} from "lucide-react";
import { InlineError, Modal } from "../../components/ui/Modal";
import { tabNavigation } from "../../components/ui/tabs";
import { client, messageOf } from "../../ipc/client";
import type {
  BrowseApproval,
  BrowseQuery,
  ColumnInfo,
  CompareRequest,
  CompareSide,
  ConnectionProfile,
  DataCompareReport,
  PreparedPlan,
  SessionInfo,
  TableInfo,
  TableRef,
  TableSchema,
} from "../../ipc/generated";
import type { StudioTab } from "../../state/studio";
import { ReviewDialog } from "../safety/ReviewDialog";
import { EMPTY_QUERY, hasRawClauses } from "./table-query";

interface SnapshotCandidate {
  id: string;
  label: string;
  resultIds: string[];
  columns: ColumnInfo[];
  rowCount: number;
  truncatedHint: boolean;
  tabId: string;
}

interface SideSelectionState {
  mode: "snapshot" | "liveTable";
  snapshotId: string;
  connectionId: string;
  tableKey: string;
  whereClause: string;
  inheritedQuery: BrowseQuery | null;
  useInheritedQuery: boolean;
}

interface PendingBrowseReview {
  side: "left" | "right";
  plan: PreparedPlan;
  profile: ConnectionProfile;
  leftApproval: BrowseApproval | null;
}

type DiffFilterTab = "all" | "different" | "onlyLeft" | "onlyRight" | "anomalies";

function tableRefKey(table: TableRef): string {
  return `${table.schema}.${table.name}`;
}

function parseTableKey(value: string, available: TableInfo[]): TableRef | null {
  const exact = available.find((item) => tableRefKey(item.table) === value);
  return exact ? exact.table : null;
}

function collectSnapshots(tabs: StudioTab[], profiles: ConnectionProfile[]): SnapshotCandidate[] {
  const profileNames = new Map(profiles.map((p) => [p.id, p.name]));
  const items: SnapshotCandidate[] = [];
  for (const tab of tabs) {
    const connPrefix = tab.connectionId
      ? `${profileNames.get(tab.connectionId) ?? "Connection"} · `
      : "";
    if (tab.kind === "query") {
      for (const res of tab.results) {
        if (res.columns.length === 0) continue;
        const suffix = res.status === "truncated" ? " · truncated" : "";
        items.push({
          id: `query:${tab.id}:${res.id}`,
          label: `${connPrefix}${tab.title} · Query ${res.statementIndex + 1} (${res.retainedRows.toLocaleString()} rows${suffix})`,
          resultIds: [res.id],
          columns: res.columns,
          rowCount: res.retainedRows,
          truncatedHint: res.status === "truncated",
          tabId: tab.id,
        });
      }
    } else if (tab.kind === "table" && tab.table && tab.pages.length > 0) {
      const cols = tab.schema?.columns ?? tab.pages[0]?.result.columns ?? [];
      if (cols.length === 0) continue;
      const truncated = tab.hasMore || tab.pages.some((page) => page.result.status === "truncated");
      const suffix = truncated ? " · loaded pages only" : "";
      items.push({
        id: `table:${tab.id}`,
        label: `${connPrefix}${tab.table.schema}.${tab.table.name} snapshot (${tab.loadedRows.toLocaleString()} rows${suffix})`,
        resultIds: tab.pages.map((page) => page.result.id),
        columns: cols,
        rowCount: tab.loadedRows,
        truncatedHint: truncated,
        tabId: tab.id,
      });
    }
  }
  return items;
}

function normalizeColName(name: string, ignoreCase: boolean): string {
  return ignoreCase ? name.toLowerCase() : name;
}

function formatCellValue(value: string | null) {
  if (value === null) {
    return <span className="cell-token">NULL</span>;
  }
  if (value === "") {
    return <span className="cell-token compare-empty-token">&quot;&quot;</span>;
  }
  return value;
}

function buildInitialSides(
  initialTab: StudioTab | undefined,
  tabs: StudioTab[],
  liveProfiles: ConnectionProfile[],
  catalog: Record<string, TableInfo[]>,
  snapshots: SnapshotCandidate[],
): { left: SideSelectionState; right: SideSelectionState } {
  const firstLiveConn = liveProfiles[0]?.id ?? "";
  const firstLiveTable = firstLiveConn
    ? catalog[firstLiveConn]?.[0]
      ? tableRefKey(catalog[firstLiveConn][0].table)
      : ""
    : "";
  const firstSnap = snapshots[0]?.id ?? "";

  const makeDefaultSide = (mode: "snapshot" | "liveTable"): SideSelectionState => ({
    mode,
    snapshotId: firstSnap,
    connectionId: firstLiveConn,
    tableKey: firstLiveTable,
    whereClause: "",
    inheritedQuery: null,
    useInheritedQuery: false,
  });

  let left = makeDefaultSide(liveProfiles.length > 0 ? "liveTable" : "snapshot");
  let right = makeDefaultSide(liveProfiles.length > 0 ? "liveTable" : "snapshot");

  if (
    initialTab?.kind === "table" &&
    initialTab.table &&
    initialTab.connectionId &&
    liveProfiles.some((p) => p.id === initialTab.connectionId)
  ) {
    const hasFilter = initialTab.query.filters.length > 0 || hasRawClauses(initialTab.query);
    left = {
      mode: "liveTable",
      snapshotId: firstSnap,
      connectionId: initialTab.connectionId,
      tableKey: tableRefKey(initialTab.table),
      whereClause: "",
      inheritedQuery: hasFilter ? initialTab.query : null,
      useInheritedQuery: hasFilter,
    };

    const otherLiveTab = tabs.find(
      (t) =>
        t.id !== initialTab.id &&
        t.kind === "table" &&
        !!t.table &&
        !!t.connectionId &&
        liveProfiles.some((p) => p.id === t.connectionId),
    );
    if (otherLiveTab && otherLiveTab.table && otherLiveTab.connectionId) {
      const otherHasFilter =
        otherLiveTab.query.filters.length > 0 || hasRawClauses(otherLiveTab.query);
      right = {
        mode: "liveTable",
        snapshotId: firstSnap,
        connectionId: otherLiveTab.connectionId,
        tableKey: tableRefKey(otherLiveTab.table),
        whereClause: "",
        inheritedQuery: otherHasFilter ? otherLiveTab.query : null,
        useInheritedQuery: otherHasFilter,
      };
    } else {
      const otherSnap = snapshots.find((s) => s.tabId !== initialTab.id);
      if (otherSnap) {
        right = {
          ...makeDefaultSide("snapshot"),
          snapshotId: otherSnap.id,
        };
      } else {
        const secondConn =
          liveProfiles.find((p) => p.id !== initialTab.connectionId)?.id ?? initialTab.connectionId;
        const connTables = catalog[secondConn] ?? [];
        const altTable =
          connTables.find(
            (item) =>
              secondConn !== initialTab.connectionId ||
              tableRefKey(item.table) !== tableRefKey(initialTab.table!),
          ) ?? connTables[0];
        right = {
          mode: "liveTable",
          snapshotId: firstSnap,
          connectionId: secondConn,
          tableKey: altTable ? tableRefKey(altTable.table) : "",
          whereClause: "",
          inheritedQuery: null,
          useInheritedQuery: false,
        };
      }
    }
  } else if (initialTab) {
    const tabSnaps = snapshots.filter((s) => s.tabId === initialTab.id);
    const activeSnap =
      initialTab.kind === "query" && initialTab.activeResultId
        ? (tabSnaps.find((s) => s.resultIds[0] === initialTab.activeResultId) ?? tabSnaps[0])
        : tabSnaps[0];
    if (activeSnap) {
      left = {
        ...makeDefaultSide("snapshot"),
        snapshotId: activeSnap.id,
      };
      const secondInTab = tabSnaps.find((s) => s.id !== activeSnap.id);
      const otherTabSnap = snapshots.find((s) => s.tabId !== initialTab.id);
      if (secondInTab) {
        right = { ...makeDefaultSide("snapshot"), snapshotId: secondInTab.id };
      } else if (otherTabSnap) {
        right = { ...makeDefaultSide("snapshot"), snapshotId: otherTabSnap.id };
      } else if (liveProfiles.length > 0) {
        right = makeDefaultSide("liveTable");
      }
    }
  }

  return { left, right };
}

export function DataCompareDialog({
  initialTab,
  tabs,
  profiles,
  sessions,
  catalog,
  onCatalogLoaded,
  onClose,
}: {
  initialTab?: StudioTab;
  tabs: StudioTab[];
  profiles: ConnectionProfile[];
  sessions: Record<string, SessionInfo>;
  catalog: Record<string, TableInfo[]>;
  onCatalogLoaded(connectionId: string, tables: TableInfo[]): void;
  onClose(): void;
}) {
  const liveProfiles = profiles.filter(
    (p) => (p.engine === "postgres" || p.engine === "mysql") && Boolean(sessions[p.id]),
  );
  const snapshots = collectSnapshots(tabs, profiles);

  const [initialPair] = useState(() =>
    buildInitialSides(initialTab, tabs, liveProfiles, catalog, snapshots),
  );
  const [left, setLeft] = useState<SideSelectionState>(initialPair.left);
  const [right, setRight] = useState<SideSelectionState>(initialPair.right);
  const [ignoreColumnCase, setIgnoreColumnCase] = useState(false);
  const [customKeyColumns, setCustomKeyColumns] = useState<string[] | null>(null);
  const [liveSchemas, setLiveSchemas] = useState<Record<string, TableSchema>>(() => {
    const seeded: Record<string, TableSchema> = {};
    for (const tab of tabs) {
      if (tab.kind === "table" && tab.connectionId && tab.table && tab.schema) {
        seeded[`${tab.connectionId}:${tableRefKey(tab.table)}`] = tab.schema;
      }
    }
    return seeded;
  });
  const [loadingMeta, setLoadingMeta] = useState(false);
  const [comparing, setComparing] = useState(false);
  const [pendingReview, setPendingReview] = useState<PendingBrowseReview | null>(null);
  const [report, setReport] = useState<DataCompareReport | null>(null);
  const [filterTab, setFilterTab] = useState<DiffFilterTab>("all");
  const [changedColsOnly, setChangedColsOnly] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const compareSeqRef = useRef(0);

  // Load missing catalogs for selected live connections
  useEffect(() => {
    const neededConns = new Set<string>();
    for (const side of [left, right]) {
      if (
        side.mode === "liveTable" &&
        side.connectionId &&
        sessions[side.connectionId] &&
        !catalog[side.connectionId]
      ) {
        neededConns.add(side.connectionId);
      }
    }
    if (neededConns.size === 0) return;
    let active = true;
    void Promise.all(
      [...neededConns].map(async (connId) => {
        const sess = sessions[connId];
        if (!sess) return;
        const tables = await client.call<TableInfo[]>("catalog", { sessionId: sess.id });
        if (!active) return;
        onCatalogLoaded(connId, tables);
        const firstKey = tables[0] ? tableRefKey(tables[0].table) : "";
        if (firstKey) {
          setLeft((prev) =>
            prev.mode === "liveTable" && prev.connectionId === connId && !prev.tableKey
              ? { ...prev, tableKey: firstKey }
              : prev,
          );
          setRight((prev) =>
            prev.mode === "liveTable" && prev.connectionId === connId && !prev.tableKey
              ? { ...prev, tableKey: firstKey }
              : prev,
          );
        }
      }),
    ).catch((err) => {
      if (active) setError(messageOf(err));
    });
    return () => {
      active = false;
    };
  }, [
    left.mode,
    left.connectionId,
    right.mode,
    right.connectionId,
    sessions,
    catalog,
    onCatalogLoaded,
  ]);

  // Load missing TableSchema for selected live tables so columns & primary keys are available
  useEffect(() => {
    const missing: { cacheKey: string; sessionId: string; table: TableRef }[] = [];
    for (const side of [left, right]) {
      if (side.mode !== "liveTable" || !side.connectionId || !side.tableKey) continue;
      const sess = sessions[side.connectionId];
      const tables = catalog[side.connectionId] ?? [];
      const tableRef = parseTableKey(side.tableKey, tables);
      if (!sess || !tableRef) continue;
      const cacheKey = `${side.connectionId}:${side.tableKey}`;
      if (!liveSchemas[cacheKey] && !missing.some((m) => m.cacheKey === cacheKey)) {
        missing.push({ cacheKey, sessionId: sess.id, table: tableRef });
      }
    }
    if (missing.length === 0) return;
    let active = true;
    setLoadingMeta(true);
    void Promise.all(
      missing.map(async (item) => {
        const schema = await client.call<TableSchema>("table_schema", {
          sessionId: item.sessionId,
          table: item.table,
        });
        return [item.cacheKey, schema] as const;
      }),
    )
      .then((entries) => {
        if (!active) return;
        setLiveSchemas((prev) => {
          const next = { ...prev };
          for (const [k, v] of entries) next[k] = v;
          return next;
        });
      })
      .catch((err) => {
        if (active) setError(messageOf(err));
      })
      .finally(() => {
        if (active) setLoadingMeta(false);
      });
    return () => {
      active = false;
    };
  }, [
    left.mode,
    left.connectionId,
    left.tableKey,
    right.mode,
    right.connectionId,
    right.tableKey,
    sessions,
    catalog,
    liveSchemas,
  ]);

  function columnsForSide(side: SideSelectionState): ColumnInfo[] {
    if (side.mode === "snapshot") {
      return snapshots.find((s) => s.id === side.snapshotId)?.columns ?? [];
    }
    if (!side.connectionId || !side.tableKey) return [];
    return liveSchemas[`${side.connectionId}:${side.tableKey}`]?.columns ?? [];
  }

  const leftColumns = columnsForSide(left);
  const rightColumns = columnsForSide(right);

  const rightLookup = new Map<string, ColumnInfo>();
  for (const col of rightColumns) {
    const norm = normalizeColName(col.name, ignoreColumnCase);
    if (!rightLookup.has(norm)) {
      rightLookup.set(norm, col);
    }
  }

  const sharedColumns: {
    name: string;
    leftCol: ColumnInfo;
    rightCol: ColumnInfo;
  }[] = [];
  for (const leftCol of leftColumns) {
    const match = rightLookup.get(normalizeColName(leftCol.name, ignoreColumnCase));
    if (match) {
      sharedColumns.push({ name: leftCol.name, leftCol, rightCol: match });
    }
  }

  const defaultKeyColumns = (() => {
    const bothPk = sharedColumns
      .filter((c) => c.leftCol.primaryKey && c.rightCol.primaryKey)
      .map((c) => c.name);
    if (bothPk.length > 0) return bothPk;
    const leftPkCount = leftColumns.filter((c) => c.primaryKey).length;
    const rightPkCount = rightColumns.filter((c) => c.primaryKey).length;
    if (leftPkCount > 0 && rightPkCount === 0) {
      const alignedLeft = sharedColumns.filter((c) => c.leftCol.primaryKey).map((c) => c.name);
      if (alignedLeft.length === leftPkCount) return alignedLeft;
    }
    if (rightPkCount > 0 && leftPkCount === 0) {
      const alignedRight = sharedColumns.filter((c) => c.rightCol.primaryKey).map((c) => c.name);
      if (alignedRight.length === rightPkCount) return alignedRight;
    }
    return [];
  })();

  const sharedNormSet = new Set(
    sharedColumns.map((c) => normalizeColName(c.name, ignoreColumnCase)),
  );
  const effectiveKeys =
    customKeyColumns !== null
      ? customKeyColumns.filter((k) => sharedNormSet.has(normalizeColName(k, ignoreColumnCase)))
      : defaultKeyColumns;

  function toggleKeyColumn(colName: string) {
    const norm = normalizeColName(colName, ignoreColumnCase);
    const exists = effectiveKeys.some((k) => normalizeColName(k, ignoreColumnCase) === norm);
    const next = exists
      ? effectiveKeys.filter((k) => normalizeColName(k, ignoreColumnCase) !== norm)
      : [...effectiveKeys, colName];
    setCustomKeyColumns(next);
    setReport(null);
  }

  function swapSides() {
    setLeft(right);
    setRight(left);
    setCustomKeyColumns(null);
    setReport(null);
    setError(null);
  }

  function buildBrowseQuery(side: SideSelectionState): BrowseQuery {
    const base =
      side.useInheritedQuery && side.inheritedQuery ? side.inheritedQuery : { ...EMPTY_QUERY };
    const trimmedWhere = side.whereClause.trim();
    if (!trimmedWhere) return base;
    const combinedWhere = base.whereClause
      ? `(${base.whereClause}) AND (${trimmedWhere})`
      : trimmedWhere;
    return {
      ...base,
      whereClause: combinedWhere,
    };
  }

  function resolveCompareSide(
    side: SideSelectionState,
    sideLabel: "Left" | "Right",
    approval: BrowseApproval | null,
  ): {
    payload: CompareSide;
    liveMeta: {
      sessionId: string;
      profile: ConnectionProfile;
      table: TableRef;
      query: BrowseQuery;
    } | null;
  } {
    if (side.mode === "snapshot") {
      const snap = snapshots.find((s) => s.id === side.snapshotId);
      if (!snap) {
        throw new Error(`Select a retained snapshot on the ${sideLabel.toLowerCase()} side.`);
      }
      return {
        payload: {
          kind: "snapshot",
          resultIds: snap.resultIds,
          label: snap.label,
          truncatedHint: snap.truncatedHint,
        },
        liveMeta: null,
      };
    }

    const profile = liveProfiles.find((p) => p.id === side.connectionId);
    const sess = side.connectionId ? sessions[side.connectionId] : undefined;
    if (!profile || !sess) {
      throw new Error(`Select an active connection on the ${sideLabel.toLowerCase()} side.`);
    }
    const table = parseTableKey(side.tableKey, catalog[side.connectionId] ?? []);
    if (!table) {
      throw new Error(`Select a table on the ${sideLabel.toLowerCase()} side.`);
    }
    const query = buildBrowseQuery(side);
    return {
      payload: {
        kind: "liveTable",
        sessionId: sess.id,
        table,
        query,
        approval,
        label: `${profile.name} · ${table.schema}.${table.name}${
          query.filters.length > 0 || hasRawClauses(query) ? " (filtered)" : ""
        }`,
      },
      liveMeta: {
        sessionId: sess.id,
        profile,
        table,
        query,
      },
    };
  }

  async function executeCompare(
    leftApproval: BrowseApproval | null,
    rightApproval: BrowseApproval | null,
  ) {
    setError(null);
    let resolvedLeft: ReturnType<typeof resolveCompareSide>;
    let resolvedRight: ReturnType<typeof resolveCompareSide>;
    try {
      resolvedLeft = resolveCompareSide(left, "Left", leftApproval);
      resolvedRight = resolveCompareSide(right, "Right", rightApproval);
    } catch (err) {
      setError(messageOf(err));
      return;
    }

    if (effectiveKeys.length === 0) {
      setError("Select at least one shared key column before comparing.");
      return;
    }

    const seq = ++compareSeqRef.current;
    setComparing(true);
    try {
      if (!leftApproval && resolvedLeft.liveMeta && hasRawClauses(resolvedLeft.liveMeta.query)) {
        const plan = await client.call<PreparedPlan | null>("prepare_browse_table", {
          sessionId: resolvedLeft.liveMeta.sessionId,
          table: resolvedLeft.liveMeta.table,
          query: resolvedLeft.liveMeta.query,
        });
        if (seq !== compareSeqRef.current) return;
        if (plan?.requiresApproval) {
          setComparing(false);
          setPendingReview({
            side: "left",
            plan,
            profile: resolvedLeft.liveMeta.profile,
            leftApproval: null,
          });
          return;
        }
      }

      if (!rightApproval && resolvedRight.liveMeta && hasRawClauses(resolvedRight.liveMeta.query)) {
        const plan = await client.call<PreparedPlan | null>("prepare_browse_table", {
          sessionId: resolvedRight.liveMeta.sessionId,
          table: resolvedRight.liveMeta.table,
          query: resolvedRight.liveMeta.query,
        });
        if (seq !== compareSeqRef.current) return;
        if (plan?.requiresApproval) {
          setComparing(false);
          setPendingReview({
            side: "right",
            plan,
            profile: resolvedRight.liveMeta.profile,
            leftApproval,
          });
          return;
        }
      }

      const request: CompareRequest = {
        left: resolvedLeft.payload,
        right: resolvedRight.payload,
        keyColumns: effectiveKeys,
        ignoreColumnCase,
      };
      const nextReport = await client.call<DataCompareReport>("compare_data", {
        request,
      });
      if (seq !== compareSeqRef.current) return;
      setReport(nextReport);
      const totalAnomalies =
        nextReport.leftAnomalies.nullKeyCount +
        nextReport.rightAnomalies.nullKeyCount +
        nextReport.leftAnomalies.duplicateKeyRows +
        nextReport.rightAnomalies.duplicateKeyRows;
      if (nextReport.diffs.length === 0 && totalAnomalies > 0) {
        setFilterTab("anomalies");
      } else {
        setFilterTab("all");
      }
    } catch (err) {
      if (seq !== compareSeqRef.current) return;
      setError(messageOf(err));
    } finally {
      if (seq === compareSeqRef.current) {
        setComparing(false);
      }
    }
  }

  function renderSideCard(
    sideTitle: "Left" | "Right",
    stateValue: SideSelectionState,
    onChange: (next: SideSelectionState) => void,
  ) {
    const prefix = sideTitle.toLowerCase();
    const availableTables = stateValue.connectionId ? (catalog[stateValue.connectionId] ?? []) : [];

    return (
      <div className="compare-side-card">
        <div className="compare-side-head">
          <strong>{sideTitle} dataset</strong>
          <span className="import-pill muted">
            {stateValue.mode === "liveTable" ? (
              <>
                <Database size={11} /> Live table
              </>
            ) : (
              <>
                <Layers size={11} /> Retained snapshot
              </>
            )}
          </span>
        </div>

        <label className="field">
          <span>Source mode</span>
          <select
            aria-label={`${sideTitle} source mode`}
            value={stateValue.mode}
            disabled={comparing}
            onChange={(event) => {
              const mode = event.target.value as "snapshot" | "liveTable";
              onChange({
                ...stateValue,
                mode,
                snapshotId: stateValue.snapshotId || snapshots[0]?.id || "",
                connectionId: stateValue.connectionId || liveProfiles[0]?.id || "",
              });
              setCustomKeyColumns(null);
              setReport(null);
            }}
          >
            <option value="liveTable" disabled={liveProfiles.length === 0}>
              Live table (PostgreSQL / MySQL)
            </option>
            <option value="snapshot" disabled={snapshots.length === 0}>
              Retained tab snapshot
            </option>
          </select>
        </label>

        {stateValue.mode === "snapshot" ? (
          <label className="field">
            <span>Retained result</span>
            <select
              aria-label={`${sideTitle} snapshot`}
              value={stateValue.snapshotId}
              disabled={comparing || snapshots.length === 0}
              onChange={(event) => {
                onChange({ ...stateValue, snapshotId: event.target.value });
                setCustomKeyColumns(null);
                setReport(null);
              }}
            >
              {snapshots.length === 0 ? (
                <option value="">No retained tab results open</option>
              ) : (
                snapshots.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.label}
                  </option>
                ))
              )}
            </select>
          </label>
        ) : (
          <>
            <div className="compare-live-selects">
              <label className="field">
                <span>Connection</span>
                <select
                  aria-label={`${sideTitle} connection`}
                  value={stateValue.connectionId}
                  disabled={comparing || liveProfiles.length === 0}
                  onChange={(event) => {
                    const nextConn = event.target.value;
                    const firstTbl = catalog[nextConn]?.[0]
                      ? tableRefKey(catalog[nextConn][0].table)
                      : "";
                    onChange({
                      ...stateValue,
                      connectionId: nextConn,
                      tableKey: firstTbl,
                      inheritedQuery: null,
                      useInheritedQuery: false,
                    });
                    setCustomKeyColumns(null);
                    setReport(null);
                  }}
                >
                  {liveProfiles.length === 0 ? (
                    <option value="">No connected PG/MySQL session</option>
                  ) : (
                    liveProfiles.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name} ({p.engine === "mysql" ? "MySQL" : "PostgreSQL"})
                      </option>
                    ))
                  )}
                </select>
              </label>

              <label className="field">
                <span>Table</span>
                <select
                  aria-label={`${sideTitle} table`}
                  value={stateValue.tableKey}
                  disabled={comparing || availableTables.length === 0}
                  onChange={(event) => {
                    onChange({
                      ...stateValue,
                      tableKey: event.target.value,
                      inheritedQuery: null,
                      useInheritedQuery: false,
                    });
                    setCustomKeyColumns(null);
                    setReport(null);
                  }}
                >
                  {availableTables.length === 0 ? (
                    <option value="">Select table</option>
                  ) : (
                    availableTables.map((item) => {
                      const key = tableRefKey(item.table);
                      return (
                        <option key={key} value={key}>
                          {key}
                        </option>
                      );
                    })
                  )}
                </select>
              </label>
            </div>

            <label className="field">
              <span>Optional WHERE filter</span>
              <input
                type="text"
                aria-label={`${sideTitle} SQL filter`}
                placeholder="e.g. tenant_id = 42"
                value={stateValue.whereClause}
                disabled={comparing}
                onChange={(event) => {
                  onChange({ ...stateValue, whereClause: event.target.value });
                  setReport(null);
                }}
              />
            </label>

            {stateValue.inheritedQuery ? (
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  aria-label={`Include ${prefix} tab active filter`}
                  checked={stateValue.useInheritedQuery}
                  disabled={comparing}
                  onChange={(event) => {
                    onChange({
                      ...stateValue,
                      useInheritedQuery: event.target.checked,
                    });
                    setReport(null);
                  }}
                />
                <span>Include active table tab filters</span>
              </label>
            ) : null}
          </>
        )}
      </div>
    );
  }

  if (pendingReview) {
    const activeReview = pendingReview;
    return (
      <ReviewDialog
        key={activeReview.plan.id}
        plan={activeReview.plan}
        profile={activeReview.profile}
        onExecute={async (approval) => {
          const token: BrowseApproval = {
            planId: activeReview.plan.id,
            approval,
          };
          setPendingReview(null);
          if (activeReview.side === "left") {
            await executeCompare(token, null);
          } else {
            await executeCompare(activeReview.leftApproval, token);
          }
        }}
        onClose={() =>
          setPendingReview((curr) => (curr?.plan.id === activeReview.plan.id ? null : curr))
        }
      />
    );
  }

  const totalAnomalies = report
    ? report.leftAnomalies.nullKeyCount +
      report.rightAnomalies.nullKeyCount +
      report.leftAnomalies.duplicateKeyRows +
      report.rightAnomalies.duplicateKeyRows
    : 0;
  const totalDiffCount = report
    ? report.differentCount + report.onlyLeftCount + report.onlyRightCount
    : 0;

  const filteredDiffs = report
    ? report.diffs.filter((row) => {
        if (filterTab === "all") return true;
        if (filterTab === "different") return row.kind === "different";
        if (filterTab === "onlyLeft") return row.kind === "onlyLeft";
        if (filterTab === "onlyRight") return row.kind === "onlyRight";
        return false;
      })
    : [];

  const kindMismatchCols = report ? report.alignedColumns.filter((c) => c.kindMismatch) : [];

  return (
    <Modal
      title="Compare data"
      subtitle="Key-aware bounded diff across live tables or retained result snapshots (up to 25,000 rows / 16 MiB per side)"
      wide
      busy={comparing}
      onClose={onClose}
    >
      <div className="modal-body compare-dialog-body">
        <div className="compare-sides-grid">
          {renderSideCard("Left", left, setLeft)}
          <div className="compare-swap-col">
            <button
              type="button"
              className="icon-button"
              title="Swap left and right datasets"
              aria-label="Swap left and right datasets"
              disabled={comparing}
              onClick={swapSides}
            >
              <ArrowLeftRight size={14} />
            </button>
          </div>
          {renderSideCard("Right", right, setRight)}
        </div>

        <div className="compare-key-panel">
          <div className="compare-key-header">
            <span className="import-section-title">
              <KeyRound size={12} /> Key columns ({effectiveKeys.length} selected ·{" "}
              {sharedColumns.length} shared column{sharedColumns.length === 1 ? "" : "s"})
            </span>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={ignoreColumnCase}
                disabled={comparing}
                onChange={(event) => {
                  setIgnoreColumnCase(event.target.checked);
                  setCustomKeyColumns(null);
                  setReport(null);
                }}
              />
              <span>Ignore column name case</span>
            </label>
          </div>

          {loadingMeta ? (
            <div className="compare-key-empty">
              <Loader2 size={13} className="spin" /> Loading table column metadata…
            </div>
          ) : sharedColumns.length === 0 ? (
            <div className="compare-key-empty">
              No shared column names found between Left and Right.
              {!ignoreColumnCase
                ? " Try enabling “Ignore column name case” if comparing uppercase and lowercase schemas."
                : ""}
            </div>
          ) : (
            <div className="compare-key-chips" role="group" aria-label="Comparison key columns">
              {sharedColumns.map((col) => {
                const selected = effectiveKeys.some(
                  (k) =>
                    normalizeColName(k, ignoreColumnCase) ===
                    normalizeColName(col.name, ignoreColumnCase),
                );
                const isPk = col.leftCol.primaryKey || col.rightCol.primaryKey;
                return (
                  <button
                    key={col.name}
                    type="button"
                    className={`compare-key-chip${selected ? " active" : ""}`}
                    aria-pressed={selected}
                    disabled={comparing}
                    onClick={() => toggleKeyColumn(col.name)}
                  >
                    {selected ? <Check size={11} /> : null}
                    <span>{col.name}</span>
                    {isPk ? <small>PK</small> : null}
                  </button>
                );
              })}
            </div>
          )}
        </div>

        {error ? <InlineError message={error} /> : null}

        {report ? (
          <div className="compare-report">
            <div className="compare-provenance-bar">
              <span>
                <strong>Left:</strong> {report.leftLabel} ({report.leftTotalRows.toLocaleString()}{" "}
                row{report.leftTotalRows === 1 ? "" : "s"}
                {report.leftTruncated ? " · snapshot truncated" : ""})
              </span>
              <span>
                <strong>Right:</strong> {report.rightLabel} (
                {report.rightTotalRows.toLocaleString()} row
                {report.rightTotalRows === 1 ? "" : "s"}
                {report.rightTruncated ? " · snapshot truncated" : ""})
              </span>
              <span className="import-pill">
                {report.identicalCount.toLocaleString()} identical
              </span>
            </div>

            {report.leftTruncated ||
            report.rightTruncated ||
            report.diffsTruncated ||
            report.unalignedLeftColumns.length > 0 ||
            report.unalignedRightColumns.length > 0 ||
            kindMismatchCols.length > 0 ||
            totalAnomalies > 0 ? (
              <div className="import-issues-panel compare-warnings">
                <div className="import-issues-header">
                  <AlertTriangle size={14} />
                  <strong>Comparison notes</strong>
                </div>
                <ul className="import-issues-list">
                  {report.leftTruncated || report.rightTruncated ? (
                    <li>
                      <span>
                        One or more retained snapshots were truncated or only partially paged.
                        Switch to Live table mode or add a SQL filter for full-table certainty.
                      </span>
                    </li>
                  ) : null}
                  {report.unalignedLeftColumns.length > 0 ? (
                    <li>
                      <code>Left only cols</code>
                      <span>{report.unalignedLeftColumns.join(", ")}</span>
                    </li>
                  ) : null}
                  {report.unalignedRightColumns.length > 0 ? (
                    <li>
                      <code>Right only cols</code>
                      <span>{report.unalignedRightColumns.join(", ")}</span>
                    </li>
                  ) : null}
                  {kindMismatchCols.length > 0 ? (
                    <li>
                      <code>Type mismatch</code>
                      <span>
                        {kindMismatchCols
                          .map((c) => `${c.name} (${c.leftType} vs ${c.rightType})`)
                          .join(", ")}
                      </span>
                    </li>
                  ) : null}
                  {totalAnomalies > 0 ? (
                    <li>
                      <code>Key anomalies</code>
                      <span>
                        {report.leftAnomalies.duplicateKeyRows +
                          report.rightAnomalies.duplicateKeyRows}{" "}
                        duplicate-key rows and{" "}
                        {report.leftAnomalies.nullKeyCount + report.rightAnomalies.nullKeyCount}{" "}
                        NULL-key rows were excluded from 1:1 row matching.
                      </span>
                    </li>
                  ) : null}
                  {report.diffsTruncated ? (
                    <li>
                      <code>Display cap</code>
                      <span>
                        Showing the first {report.diffs.length.toLocaleString()} row differences
                        (summary counts reflect all compared rows).
                      </span>
                    </li>
                  ) : null}
                </ul>
              </div>
            ) : null}

            <div className="compare-tabs-row">
              <div
                className="result-tabs compare-filter-tabs"
                role="tablist"
                aria-label="Comparison difference categories"
                onKeyDown={tabNavigation}
              >
                {(
                  [
                    ["all", "All differences", totalDiffCount],
                    ["different", "Modified", report.differentCount],
                    ["onlyLeft", "Only in left", report.onlyLeftCount],
                    ["onlyRight", "Only in right", report.onlyRightCount],
                    ["anomalies", "Duplicate & NULL keys", totalAnomalies],
                  ] as const
                ).map(([id, label, count]) => (
                  <button
                    key={id}
                    type="button"
                    role="tab"
                    aria-selected={filterTab === id}
                    tabIndex={filterTab === id ? 0 : -1}
                    className={filterTab === id ? "active" : ""}
                    onClick={() => setFilterTab(id)}
                  >
                    {label}
                    <span>{count.toLocaleString()}</span>
                  </button>
                ))}
              </div>

              {filterTab !== "anomalies" ? (
                <label className="checkbox-row">
                  <input
                    type="checkbox"
                    checked={changedColsOnly}
                    onChange={(event) => setChangedColsOnly(event.target.checked)}
                  />
                  <span>Changed columns only</span>
                </label>
              ) : null}
            </div>

            {filterTab === "anomalies" ? (
              <div className="compare-anomalies-panel">
                {totalAnomalies === 0 ? (
                  <div className="compare-empty-diff">
                    No duplicate keys or NULL key values were found on either side.
                  </div>
                ) : (
                  <>
                    {(
                      [
                        ["Left", report.leftLabel, report.leftAnomalies],
                        ["Right", report.rightLabel, report.rightAnomalies],
                      ] as const
                    ).map(([sideName, sideLabel, anom]) =>
                      anom.duplicateKeySamples.length > 0 || anom.nullKeySamples.length > 0 ? (
                        <div key={sideName} className="compare-anomaly-group">
                          <div className="import-section-title">
                            {sideName} ({sideLabel}) — {anom.duplicateKeyRows} duplicate-key rows,{" "}
                            {anom.nullKeyCount} NULL-key rows
                          </div>
                          {anom.duplicateKeySamples.length > 0 ? (
                            <ul className="import-issues-list">
                              {anom.duplicateKeySamples.map((dup, index) => (
                                <li key={index}>
                                  <code>
                                    (
                                    {report.keyColumns
                                      .map((k, idx) => `${k}=${dup.keyValues[idx] ?? ""}`)
                                      .join(", ")}
                                    )
                                  </code>
                                  <span>
                                    {dup.occurrences} occurrences on {sideName.toLowerCase()}
                                    {dup.counterpartOccurrences > 0
                                      ? `, ${dup.counterpartOccurrences} on opposite side (excluded from 1:1 diff)`
                                      : ""}
                                  </span>
                                </li>
                              ))}
                            </ul>
                          ) : null}
                          {anom.nullKeySamples.length > 0 ? (
                            <div className="import-sample-scroll">
                              <table className="import-sample-table">
                                <thead>
                                  <tr>
                                    {report.alignedColumns.map((col) => (
                                      <th key={col.name}>{col.name}</th>
                                    ))}
                                  </tr>
                                </thead>
                                <tbody>
                                  {anom.nullKeySamples.map((row, rIdx) => (
                                    <tr key={rIdx}>
                                      {row.map((val, cIdx) => (
                                        <td key={cIdx}>{formatCellValue(val)}</td>
                                      ))}
                                    </tr>
                                  ))}
                                </tbody>
                              </table>
                            </div>
                          ) : null}
                        </div>
                      ) : null,
                    )}
                  </>
                )}
              </div>
            ) : filteredDiffs.length === 0 ? (
              <div className="compare-empty-diff">
                {totalDiffCount === 0
                  ? `All ${report.identicalCount.toLocaleString()} unique keyed rows match identically.`
                  : "No rows in this difference category."}
              </div>
            ) : (
              <div className="compare-diff-scroll">
                <table className="import-sample-table compare-diff-table">
                  <thead>
                    <tr>
                      <th>Status</th>
                      {report.alignedColumns
                        .filter((col) => {
                          if (!changedColsOnly || col.isKey) return true;
                          return filteredDiffs.some(
                            (d) =>
                              d.kind !== "different" ||
                              d.cells.some((cell) => cell.column === col.name),
                          );
                        })
                        .map((col) => (
                          <th key={col.name}>
                            {col.name}
                            {col.isKey ? " (PK)" : ""}
                            <small>
                              {col.leftType === col.rightType
                                ? col.leftType
                                : `${col.leftType} ↔ ${col.rightType}`}
                            </small>
                          </th>
                        ))}
                    </tr>
                  </thead>
                  <tbody>
                    {filteredDiffs.map((diff, rowIdx) => {
                      const diffMap = new Map(diff.cells.map((c) => [c.column, c]));
                      return (
                        <tr key={rowIdx} className={`compare-row-${diff.kind}`}>
                          <td>
                            <span className={`compare-status-badge ${diff.kind}`}>
                              {diff.kind === "different"
                                ? "Modified"
                                : diff.kind === "onlyLeft"
                                  ? "Only in left"
                                  : "Only in right"}
                            </span>
                          </td>
                          {report.alignedColumns.map((col, colIdx) => {
                            if (
                              changedColsOnly &&
                              !col.isKey &&
                              !filteredDiffs.some(
                                (d) =>
                                  d.kind !== "different" ||
                                  d.cells.some((cell) => cell.column === col.name),
                              )
                            ) {
                              return null;
                            }
                            const cellDiff = diffMap.get(col.name);
                            if (diff.kind === "onlyLeft") {
                              return (
                                <td key={col.name}>
                                  {formatCellValue(diff.leftRow?.[colIdx] ?? null)}
                                </td>
                              );
                            }
                            if (diff.kind === "onlyRight") {
                              return (
                                <td key={col.name}>
                                  {formatCellValue(diff.rightRow?.[colIdx] ?? null)}
                                </td>
                              );
                            }
                            if (!cellDiff) {
                              return (
                                <td key={col.name} className="compare-cell-same">
                                  {formatCellValue(diff.leftRow?.[colIdx] ?? null)}
                                </td>
                              );
                            }
                            return (
                              <td key={col.name} className="compare-cell-changed">
                                <div className="compare-cell-pair">
                                  <span className="compare-val-left">
                                    {formatCellValue(cellDiff.left)}
                                  </span>
                                  <span className="compare-arrow">→</span>
                                  <span className="compare-val-right">
                                    {formatCellValue(cellDiff.right)}
                                  </span>
                                  {cellDiff.typeMismatch ? (
                                    <span className="compare-type-badge">type</span>
                                  ) : null}
                                </div>
                              </td>
                            );
                          })}
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        ) : null}
      </div>

      <div className="modal-footer">
        <button type="button" className="button ghost" disabled={comparing} onClick={onClose}>
          Close
        </button>
        <button
          type="button"
          className="button primary"
          disabled={comparing || loadingMeta || effectiveKeys.length === 0}
          onClick={() => void executeCompare(null, null)}
        >
          {comparing ? (
            <>
              <Loader2 size={13} className="spin" />
              Comparing…
            </>
          ) : (
            <>
              <Play size={12} fill="currentColor" />
              Compare datasets
            </>
          )}
        </button>
      </div>
    </Modal>
  );
}
