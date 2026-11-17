import type { SqlExecutionOrigin } from "../features/studio/sql-navigation";
import { create } from "zustand";
import type {
  AppError,
  BookmarkTarget,
  Bootstrap,
  BrowseApproval,
  BrowseQuery,
  ConnectionProfile,
  DatabaseObjectRef,
  DatabaseObjectSummary,
  OpenedSqlFile,
  PreparedPlan,
  QueryEvent,
  ResultInfo,
  SavedBookmark,
  SavedQuery,
  SavedQueryInput,
  SavedSqlFile,
  SavedTableLayout,
  SessionContextOptions,
  SessionInfo,
  SessionRuntimeState,
  SqlFormatSettings,
  SqlSnippet,
  SqlSnippetInput,
  VirtualForeignKey,
  VirtualForeignKeyInput,
  WorkspaceSqlResponse,
  TableInfo,
  TableRef,
  TableSchema,
  TransactionMode,
  TransactionControlResult,
  SnowflakeDmlResult,
  SnowflakeDdlResult,
  SnowflakeContextCatalog,
  SnowflakeContextTarget,
  SnowflakeGridFinishAction,
  SnowflakeGridResult,
  TransactionControlAuthority,
  SnowflakeConsolePlan,
  SnowflakeConsoleReceiptSummary,
} from "../ipc/generated";
import { client, messageOf } from "../ipc/client";
import { sealConsoleSession } from "../ipc/console-session-guard";
import {
  adoptConsoleNotAdmitted,
  consoleDecisionConflicts,
  consoleReceiptHeld,
  consoleReceiptBlocksSession,
  consoleRunStatus,
  rememberConsoleReceipts,
  consoleReceiptStorageProblem,
  validateConsoleHandoff,
  validateConsoleSummary,
  compactConsoleReceipts,
  consoleReceiptAccess,
  restoredConsoleReceipts,
  validateConsolePlan,
  validateConsoleReceipt,
  type SnowflakeConsoleReceipt,
} from "../features/studio/SnowflakeConsole";
import { DEFAULT_SQL_FORMAT_SETTINGS } from "../data/sql-format";
import {
  appendDatabaseOutputEntries,
  type DatabaseOutputEntry,
} from "../features/studio/DatabaseOutputPanel";
import { resultCache } from "../data/result-cache";
import { dirtyStore } from "../data/dirty-store";
import {
  databaseObjectKey,
  isTabularObjectKind,
  objectTabTitle,
  sameDatabaseObject,
  tableInfoKindToObjectKind,
} from "../features/connections/explorer-tree";
import {
  MAX_TABLE_LAYOUTS,
  isDefaultTableLayout,
  sameTableLayoutTarget,
} from "../features/grid/table-layout";
import {
  EMPTY_QUERY,
  hasRawClauses,
  isEmptyQuery,
  normalizeQuery,
  sameQuery,
  tableTabTitle,
} from "../features/grid/table-query";

import { planSnapshots, type PlanSource } from "../features/studio/plan-snapshots";
import {
  SHORTCUT_BY_ID,
  sanitizeShortcutOverrides,
  validateShortcutOverride,
  type ShortcutActionId,
  type ShortcutValidationResult,
} from "../features/studio/shortcuts";

export type TabKind =
  | "query"
  | "result"
  | "table"
  | "object"
  | "benchmark"
  | "history"
  | "processes"
  | "diagram"
  | "schemaDiff"
  | "workspaceSql"
  | "dataSearch";
export type TabStatus =
  | "idle"
  | "queued"
  | "running"
  | "completed"
  | "stopped"
  | "failed"
  | "cancelled"
  | "outcomeUnknown";
export interface TablePageState {
  start: number;
  result: ResultInfo;
  after: (string | null)[] | null;
}
export interface MetadataOrigin {
  tabId: string;
  sessionId: string;
  contextRevision: number;
  catalogRevision?: string;
}
export interface StudioTab {
  metadataOrigin?: MetadataOrigin;
  errorOrigin?: SqlExecutionOrigin;
  snowflakeDmlResult?: SnowflakeDmlResult;
  snowflakeDdlResult?: SnowflakeDdlResult;
  id: string;
  kind: TabKind;
  title: string;
  connectionId: string | null;
  sql: string;
  results: ResultInfo[];
  planSource?: PlanSource;
  activeResultId: string | null;
  status: TabStatus;
  runId: string | null;
  elapsedMs: number;
  runMetrics?: Extract<QueryEvent, { type: "runMetrics" }>;
  executionSummary?: Extract<QueryEvent, { type: "finished" }>["executionSummary"];
  clientStartedAt?: number;
  clientElapsedMs?: number;
  error: AppError | null;
  table: TableRef | null;
  object: DatabaseObjectRef | null;
  /** Server-side sort/filter for table tabs; never persisted. */
  query: BrowseQuery;
  schema: TableSchema | null;
  pages: TablePageState[];
  pageKeys: Record<number, (string | null)[] | null>;
  loadedRows: number;
  hasMore: boolean;
  nextKey: (string | null)[] | null;
  loadingPage: boolean;
  view: "data" | "structure" | "ddl";
  highlightedColumn?: string | null;
  revision: number;
  metrics: Record<number, { affectedRows: string; elapsedMs: number }>;
  dataSessionId: string | null;
  querySession: SessionInfo | null;
  querySessionClosed: boolean;
  fileToken: string | null;
  fileName: string | null;
  savedSql: string | null;
  savedQueryId: string | null;
  /** Whether the tab is pinned in the workspace tab strip. */
  pinned?: boolean;
  /** Schema shown by a diagram tab. */
  diagramSchema?: string;
  /** Active display presentation for data grids (`grid` | `transpose` | `record`). */
  gridMode?: "grid" | "transpose" | "record";
  /** Transient column layouts keyed by `resultId` for query and detached result tabs. */
  resultLayouts?: Record<string, SavedTableLayout>;
  /** Originating query tab identifier for a detached result tab. */
  originTabId?: string | null;
  /** Originating tab kind (`query` or `workspaceSql`) for a detached result tab. */
  originKind?: "query" | "workspaceSql" | null;
  /** Originating SQL text captured when a query result was detached into its own tab. */
  originSql?: string | null;
  /** Session-only Database Output log entries (bounded in memory; never persisted). */
  outputLog?: DatabaseOutputEntry[];
  /** Result IDs pinned inside a SQL query console so re-running SQL keeps them alongside new results. */
  pinnedResultIds?: string[];
  /** Human-readable labels captured when a result subtab was pinned (keyed by result ID). */
  pinnedResultLabels?: Record<string, string>;
}

export const MAX_PINNED_RESULTS_PER_TAB = 8;

export const MAX_RECENT_LOCATIONS = 50;
export const MAX_CLOSED_TABS = 10;

export interface ClosedTabSnapshot {
  id: string;
  kind: TabKind;
  title: string;
  connectionId: string | null;
  sql: string;
  table: TableRef | null;
  object: DatabaseObjectRef | null;
  query: BrowseQuery;
  view: StudioTab["view"];
  savedQueryId: string | null;
  pinned: boolean;
  diagramSchema?: string;
  closedAtMs: number;
}

export type RecentLocationTarget =
  | { kind: "table"; connectionId: string; table: TableRef }
  | { kind: "object"; connectionId: string; object: DatabaseObjectRef }
  | { kind: "savedQuery"; savedQueryId: string }
  | { kind: "tab"; tabId: string };

export interface RecentLocation {
  key: string;
  label: string;
  subtitle: string;
  visitedAtMs: number;
  target: RecentLocationTarget;
}

export type OpenLocationOutcome =
  { kind: "opened" } | { kind: "needsConnect"; profile: ConnectionProfile } | { kind: "missing" };

export type TableQueryOutcome =
  | { kind: "applied" }
  | { kind: "refused"; reason: string }
  | { kind: "review"; plan: PreparedPlan; query: BrowseQuery };

export interface NamedSession {
  info: SessionInfo;
  label: string;
  primary: boolean;
}

let sqlFormatSaveTail: Promise<void> = Promise.resolve();
let outputEntrySeq = 0;
let pinnedResultSeq = 0;
let sqlFormatSaveSequence = 0;
let sqlFormatPending = 0;
let sqlFormatPersisted = DEFAULT_SQL_FORMAT_SETTINGS;
const contextRequests = new Map<string, symbol>();
const pendingContextRequests = new Map<string, Set<symbol>>();
const schemaSwitchRequests = new Map<string, symbol>();

function beginContextRequest(sessionId: string): symbol {
  const request = Symbol();
  contextRequests.set(sessionId, request);
  const pending = pendingContextRequests.get(sessionId) ?? new Set<symbol>();
  pending.add(request);
  pendingContextRequests.set(sessionId, pending);
  return request;
}
function finishContextRequest(sessionId: string, request: symbol) {
  const pending = pendingContextRequests.get(sessionId);
  pending?.delete(request);
  if (pending?.size === 0) pendingContextRequests.delete(sessionId);
}

interface StudioState {
  bootstrap: Bootstrap | null;
  error: string | null;
  sessions: Record<string, SessionInfo>;
  consoleSessions: Record<string, true>;
  snowflakeConsoleReceipts: Record<string, SnowflakeConsoleReceipt>;
  snowflakeConsoleResultOwners: Record<string, string>;
  startSnowflakeConsole(tabId: string, plan: SnowflakeConsolePlan, approval: string): Promise<void>;
  refreshSnowflakeConsole(clientRequestId: string): Promise<void>;
  openSnowflakeConsoleResults(clientRequestId: string): Promise<void>;
  cancelSnowflakeConsole(clientRequestId: string): Promise<void>;
  discoverSnowflakeConsoleReceipt(summary: SnowflakeConsoleReceiptSummary): Promise<void>;
  handoffSnowflakeConsoleReceipt(clientRequestId: string): Promise<void>;
  releaseSnowflakeConsoleResults(clientRequestId: string): Promise<void>;
  sessionPool: Record<string, NamedSession[]>;
  sessionStates: Record<string, SessionRuntimeState>;
  sessionContexts: Record<string, string[]>;
  sessionContextRevision: Record<string, number>;
  sessionContextAuthorities: Record<string, SessionContextOptions>;
  snowflakeContextAuthorities: Record<string, SnowflakeContextCatalog>;
  snowflakeContextExpanded: Record<string, boolean>;
  sessionContextSwitching: Record<string, boolean>;
  sessionTransactionPending: Record<string, boolean>;
  sessionMetadataRefreshRequired: Record<string, boolean>;
  sessionDdlCatalogRefreshRequired: Record<string, boolean>;
  sessionSnowflakeGridPending: Record<string, boolean>;
  sessionSnowflakeGridTransactions: Record<
    string,
    { transactionId: string; recordingError: string | null }
  >;
  catalog: Record<string, TableInfo[]>;
  objects: Record<string, DatabaseObjectSummary[]>;
  selectedConnectionId: string | null;
  tabs: StudioTab[];
  activeTabId: string | null;
  closedTabs: ClosedTabSnapshot[];
  tableLayouts: SavedTableLayout[];
  savedQueries: SavedQuery[];
  bookmarks: SavedBookmark[];
  sqlFormatSettings: SqlFormatSettings;
  sqlSnippets: SqlSnippet[];
  virtualForeignKeys: VirtualForeignKey[];
  recentLocations: RecentLocation[];
  shortcuts: Record<string, string>;
  theme: "dark" | "light" | "contrast";
  density: "comfortable" | "compact";
  sidebarVisible: boolean;
  sidebarCollapsed: boolean;
  inspectorVisible: boolean;
  toast: string | null;
  initialize(): Promise<void>;
  notify(message: string): void;
  setTheme(theme: StudioState["theme"]): void;
  toggleSidebar(): void;
  toggleInspector(): void;
  setShortcut(actionId: ShortcutActionId, binding: string | null): ShortcutValidationResult;
  resetShortcuts(): void;
  setTableLayout(connectionId: string, table: TableRef, layout: SavedTableLayout | null): void;
  resetTableLayout(connectionId: string, table: TableRef): void;
  setResultLayout(tabId: string, resultId: string, layout: SavedTableLayout | null): void;
  togglePinResult(tabId: string, resultId?: string): boolean;
  closeResultSubtab(tabId: string, resultId: string): void;
  prepareQueryRunResults(tabId: string): Promise<void>;
  detachResultToTab(tabId: string, resultId?: string): string | null;
  openOriginatingSql(resultTabId: string): string | null;
  setWorkspaceSqlResult(tabId: string, sql: string, response: WorkspaceSqlResponse): void;
  setSession(session: SessionInfo): void;
  bindQuerySession(tabId: string, session: SessionInfo, label?: string): boolean;
  attachQuerySession(tabId: string, sessionId: string | null): boolean;
  renameQuerySession(sessionId: string, label: string): void;
  disconnectQuerySession(sessionId: string): Promise<void>;
  updateSessionLatency(sessionId: string, latencyMs: number): void;
  refreshSessionContexts(sessionId: string, throwOnError?: boolean): Promise<boolean>;
  refreshSnowflakeSessionContexts(sessionId: string, throwOnError?: boolean): Promise<boolean>;
  invalidateSessionContextAuthority(sessionId: string): void;
  isSessionContextPending(sessionId: string): boolean;
  switchSessionContext(sessionId: string, target: string): Promise<void>;
  switchSnowflakeSessionContext(sessionId: string, target: SnowflakeContextTarget): Promise<void>;
  setTransactionMode(sessionId: string, mode: TransactionMode): Promise<void>;
  commitTransaction(sessionId: string): Promise<void>;
  rollbackTransaction(sessionId: string): Promise<void>;
  executeTransactionControl(
    plan: PreparedPlan,
    approval?: string | null,
  ): Promise<TransactionControlResult>;
  executeSnowflakeDml(
    plan: PreparedPlan,
    approval?: string | null,
    ownerTabId?: string,
  ): Promise<SnowflakeDmlResult>;
  executeSnowflakeDdl(
    plan: PreparedPlan,
    approval?: string | null,
    ownerTabId?: string,
  ): Promise<SnowflakeDdlResult>;
  executeSnowflakeGridPlan(
    plan: PreparedPlan,
    approval?: string | null,
  ): Promise<SnowflakeGridResult>;
  finishSnowflakeGridTransaction(
    sessionId: string,
    transactionId: string,
    action: SnowflakeGridFinishAction,
  ): Promise<SnowflakeGridResult>;
  retireSession(sessionId: string): void;
  updateProfile(profile: ConnectionProfile): void;
  selectConnection(id: string): void;
  refreshCatalog(id: string, explicit?: boolean): Promise<void>;
  openQuery(connectionId?: string | null, sql?: string): string;
  openSqlFile(connectionId?: string | null): Promise<string | null>;
  saveSqlFile(tabId: string, options?: { saveAs?: boolean }): Promise<boolean>;
  saveQueryToLibrary(input: SavedQueryInput, attachTabId?: string): Promise<SavedQuery>;
  deleteSavedQuery(id: string): Promise<void>;
  openSavedQuery(id: string): string | null;
  toggleBookmark(target: BookmarkTarget, label: string): Promise<boolean>;
  deleteBookmark(id: string): Promise<void>;
  saveSqlFormatSettings(settings: SqlFormatSettings): Promise<SqlFormatSettings>;
  saveSqlSnippet(input: SqlSnippetInput): Promise<SqlSnippet>;
  deleteSqlSnippet(id: string): Promise<void>;
  saveVirtualForeignKey(input: VirtualForeignKeyInput): Promise<VirtualForeignKey>;
  deleteVirtualForeignKey(id: string): Promise<void>;
  clearTabOutput(tabId: string): void;
  openLocation(target: RecentLocationTarget): Promise<OpenLocationOutcome>;
  recordRecentLocation(entry: Omit<RecentLocation, "visitedAtMs">): void;
  openUtility(kind: "history" | "processes" | "benchmark" | "workspaceSql" | "dataSearch"): void;
  openDiagram(connectionId: string, schema: string): void;
  openSchemaDiff(connectionId: string, schema: string): void;
  openTable(
    connectionId: string,
    table: TableRef,
    query?: BrowseQuery,
    preferredView?: StudioTab["view"],
    highlightedColumn?: string | null,
  ): Promise<void>;
  openObject(
    connectionId: string,
    object: DatabaseObjectRef,
    preferredView?: StudioTab["view"],
    highlightedColumn?: string | null,
  ): Promise<void>;
  openSessionObject(
    tabId: string,
    object: DatabaseObjectRef,
    highlightedColumn?: string,
    catalogRevision?: string,
  ): void;
  openLinkedSessionObject(inspectorId: string, object: DatabaseObjectRef): void;
  appendSessionDdl(inspectorId: string, ddl: string): boolean;
  /** Applies a new sort/filter or requests review for non-Read raw SQL clauses. */
  setTableQuery(
    id: string,
    query: BrowseQuery,
    options?: { force?: boolean; approval?: BrowseApproval },
  ): Promise<TableQueryOutcome>;
  loadNextPage(id: string, approval?: BrowseApproval): Promise<void>;
  ensureTablePage(id: string, row: number): Promise<void>;
  refreshTable(id: string, approval?: BrowseApproval): Promise<void>;
  updateTab(
    id: string,
    update: Partial<StudioTab> | ((tab: StudioTab) => Partial<StudioTab>),
  ): void;
  togglePinTab(id: string): void;
  closeTab(id: string): void;
  closeOtherTabs(keepTabId: string): number;
  closeCleanTabs(): number;
  reopenClosedTab(): Promise<string | null>;
  removeConnection(id: string): void;
  disconnected(connectionId: string): void;
  resetConnectionResults(connectionId: string): void;
  applyEvents(tabId: string, events: QueryEvent[], revision: number): void;
}

let toastTimer: ReturnType<typeof setTimeout> | undefined;
function tab(kind: TabKind, title: string, connectionId: string | null): StudioTab {
  return {
    id: crypto.randomUUID(),
    kind,
    title,
    connectionId,
    sql: "",
    results: [],
    activeResultId: null,
    status: "idle",
    runId: null,
    elapsedMs: 0,
    runMetrics: undefined,
    executionSummary: undefined,
    clientStartedAt: undefined,
    clientElapsedMs: undefined,
    error: null,
    table: null,
    object: null,
    query: EMPTY_QUERY,
    schema: null,
    pages: [],
    pageKeys: {},
    loadedRows: 0,
    hasMore: false,
    nextKey: null,
    loadingPage: false,
    view: "data",
    highlightedColumn: null,
    revision: 0,
    metrics: {},
    dataSessionId: null,
    querySession: null,
    querySessionClosed: false,
    fileToken: null,
    fileName: null,
    savedSql: null,
    savedQueryId: null,
    pinned: false,
  };
}
export function isTabFileDirty(tab: StudioTab): boolean {
  return tab.kind === "query" && tab.fileName !== null && tab.sql !== tab.savedSql;
}
export function isTabProtectedFromBulkClose(
  allTabs: readonly StudioTab[],
  candidate: StudioTab,
): boolean {
  const resultIds = [
    ...candidate.results.map((r) => r.id),
    ...candidate.pages.map((p) => p.result.id),
  ];
  return (
    dirtyStore.pendingCountForTab(candidate.id, resultIds) > 0 ||
    candidate.results.some((r) => dirtyStore.pendingCountForTab(r.id, [r.id]) > 0) ||
    isTabFileDirty(candidate) ||
    ["queued", "running"].includes(candidate.status) ||
    isLastOwnerOfSecondarySession(allTabs, candidate)
  );
}
function snapshotClosedTab(candidate: StudioTab): ClosedTabSnapshot | null {
  if (
    candidate.metadataOrigin ||
    candidate.kind === "result" ||
    candidate.kind === "workspaceSql" ||
    candidate.kind === "dataSearch"
  ) {
    return null;
  }
  return {
    id: candidate.id,
    kind: candidate.kind,
    title: candidate.title,
    connectionId: candidate.connectionId,
    sql: candidate.sql,
    table: candidate.table,
    object: candidate.object,
    query: candidate.query,
    view: candidate.view,
    savedQueryId: candidate.savedQueryId,
    pinned: Boolean(candidate.pinned),
    diagramSchema: candidate.diagramSchema,
    closedAtMs: Date.now(),
  };
}
function orderPinnedFirst(tabs: readonly StudioTab[]): StudioTab[] {
  const pinned: StudioTab[] = [];
  const unpinned: StudioTab[] = [];
  for (const entry of tabs) {
    if (entry.pinned) pinned.push(entry);
    else unpinned.push(entry);
  }
  return [...pinned, ...unpinned];
}
export function bookmarkTargetKey(target: BookmarkTarget): string {
  if (target.kind === "savedQuery") return `saved:${target.savedQueryId}`;
  return `obj:${target.connectionId}:${databaseObjectKey(target.object)}`;
}
export function findBookmarkForTarget(
  bookmarks: readonly SavedBookmark[],
  target: BookmarkTarget,
): SavedBookmark | undefined {
  const key = bookmarkTargetKey(target);
  return bookmarks.find((entry) => bookmarkTargetKey(entry.target) === key);
}
export function bookmarkTargetForTab(
  tab: StudioTab,
  catalog: Readonly<Record<string, readonly TableInfo[] | undefined>>,
): { target: BookmarkTarget; defaultLabel: string } | null {
  if (tab.metadataOrigin) return null;
  if (tab.kind === "table" && tab.connectionId && tab.table) {
    const match = catalog[tab.connectionId]?.find(
      (entry) => entry.table.schema === tab.table!.schema && entry.table.name === tab.table!.name,
    );
    const kind = match ? tableInfoKindToObjectKind(match.kind) : "table";
    return {
      target: {
        kind: "object",
        connectionId: tab.connectionId,
        object: {
          schema: tab.table.schema,
          name: tab.table.name,
          kind,
          identity: null,
        },
      },
      defaultLabel: `${tab.table.schema}.${tab.table.name}`,
    };
  }
  if (tab.kind === "object" && tab.connectionId && tab.object) {
    return {
      target: {
        kind: "object",
        connectionId: tab.connectionId,
        object: tab.object,
      },
      defaultLabel: `${tab.object.schema}.${objectTabTitle(tab.object)}`,
    };
  }
  if (tab.kind === "query" && tab.savedQueryId) {
    return {
      target: {
        kind: "savedQuery",
        savedQueryId: tab.savedQueryId,
      },
      defaultLabel: tab.title,
    };
  }
  return null;
}
export function querySessionForTab(tab: StudioTab, sessions: Record<string, SessionInfo>) {
  if (tab.metadataOrigin) return undefined;
  if (tab.kind === "query" && tab.querySession)
    return tab.querySessionClosed ? undefined : tab.querySession;
  return tab.connectionId ? sessions[tab.connectionId] : undefined;
}
/** Metadata never adopts another session after its source console is closed or rebound. */
export function metadataSessionForTab(
  tab: StudioTab,
  state: Pick<StudioState, "tabs" | "sessions" | "sessionContextRevision">,
): SessionInfo | undefined {
  const origin = tab.metadataOrigin;
  if (!origin) return undefined;
  const console = state.tabs.find((entry) => entry.id === origin.tabId && entry.kind === "query");
  const session = console && querySessionForTab(console, state.sessions);
  return !session?.consoleMode &&
    session?.id === origin.sessionId &&
    session.connectionId === tab.connectionId &&
    (state.sessionContextRevision[session.id] ?? 0) === origin.contextRevision
    ? session
    : undefined;
}
export function isLastOwnerOfSecondarySession(tabs: readonly StudioTab[], tab: StudioTab): boolean {
  if (!tab.querySession || tab.querySessionClosed) return false;
  const sessionId = tab.querySession.id;
  return !tabs.some(
    (other) =>
      other.id !== tab.id && other.querySession?.id === sessionId && !other.querySessionClosed,
  );
}
export function sessionHasOpenTransaction(
  sessionStates: Readonly<Record<string, SessionRuntimeState | undefined>>,
  sessionId: string | null | undefined,
): boolean {
  if (!sessionId) return false;
  const txState = sessionStates[sessionId]?.transactionState;
  return (
    txState === undefined ||
    txState === "unknown" ||
    txState === "inTransaction" ||
    txState === "failed"
  );
}
function sessionInfoById(
  state: Partial<Pick<StudioState, "sessions" | "sessionPool" | "tabs">>,
  sessionId: string,
): SessionInfo | undefined {
  return (
    Object.values(state.sessions ?? {}).find((session) => session.id === sessionId) ??
    Object.values(state.sessionPool ?? {})
      .flat()
      .find((entry) => entry.info.id === sessionId)?.info ??
    (state.tabs ?? []).find((tab) => tab.querySession?.id === sessionId && !tab.querySessionClosed)
      ?.querySession ??
    undefined
  );
}
/** Retired console IDs remain sealed so stale mounted handlers cannot probe. */
export function isSnowflakeConsoleSession(
  state: Partial<Pick<StudioState, "consoleSessions" | "sessions" | "sessionPool" | "tabs">>,
  sessionId: string | null | undefined,
): boolean {
  return (
    !!sessionId &&
    (state.consoleSessions?.[sessionId] === true ||
      sessionInfoById(state, sessionId)?.consoleMode === true)
  );
}
export function assertOrdinarySession(sessionId: string): void {
  if (isSnowflakeConsoleSession(useStudio.getState(), sessionId))
    throw new Error(
      "This session belongs to the reviewed SQL console. Use a separate metadata connection; automatic SQL is disabled.",
    );
}
/** Receipt ownership outlives editor tabs and local Arrow caches. */
export function isConsoleOwnedResult(result: ResultInfo): boolean {
  const state = useStudio.getState();
  return (
    state.snowflakeConsoleResultOwners[result.id] === result.runId ||
    Object.values(state.snowflakeConsoleReceipts).some(
      (receipt) =>
        receipt.runId === result.runId || receipt.view?.run.metadata.runId === result.runId,
    )
  );
}

const receiptLookups = new Set<string>();

function unknownSdkRuntime(
  sessionId: string,
  authority?: TransactionControlAuthority,
): SessionRuntimeState {
  return {
    sessionId,
    transactionMode: "unknown",
    transactionState: "unknown",
    contextKind: "schema",
    activeContext: null,
    database: "",
    role: null,
    searchPath: [],
    transactionObservation: { status: "unavailable" },
    ...(authority?.status === "rollbackOnly" ? { transactionControlAuthority: authority } : {}),
  };
}
function hasSdkObservation(
  state: Pick<StudioState, "sessions" | "sessionPool" | "tabs" | "sessionStates">,
  sessionId: string,
): boolean {
  return (
    state.sessionStates[sessionId]?.transactionObservation !== undefined ||
    sessionInfoById(state, sessionId)?.capabilities.relationBrowsing === true
  );
}
function sessionRuntimeContextChanged(
  previous: SessionRuntimeState | undefined,
  next: SessionRuntimeState,
): boolean {
  return (
    !!previous &&
    (previous.activeContext !== next.activeContext ||
      previous.database !== next.database ||
      previous.role !== next.role ||
      (previous.warehouse ?? null) !== (next.warehouse ?? null) ||
      JSON.stringify(previous.searchPath) !== JSON.stringify(next.searchPath) ||
      (previous.contextFingerprint ?? null) !== (next.contextFingerprint ?? null))
  );
}
function isSessionActive(
  state: Pick<StudioState, "sessions" | "sessionPool" | "tabs">,
  sessionId: string,
): boolean {
  return (
    Object.values(state.sessions).some((s) => s.id === sessionId) ||
    Object.values(state.sessionPool).some((list) =>
      list.some((entry) => entry.info.id === sessionId),
    ) ||
    state.tabs.some((t) => t.querySession?.id === sessionId && !t.querySessionClosed)
  );
}
function normalizeSessionLabel(raw: string): string {
  // eslint-disable-next-line no-control-regex
  return raw
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 40);
}
function defaultSecondarySessionLabel(existing: readonly NamedSession[]): string {
  if (!existing.some((entry) => entry.label === "Isolated session")) return "Isolated session";
  let index = 2;
  while (existing.some((entry) => entry.label === `Isolated session ${index}`)) index += 1;
  return `Isolated session ${index}`;
}
function purgeSessionRuntimeMaps(
  state: Pick<
    StudioState,
    | "sessionStates"
    | "sessionContexts"
    | "sessionContextRevision"
    | "sessionContextAuthorities"
    | "snowflakeContextAuthorities"
    | "snowflakeContextExpanded"
    | "sessionContextSwitching"
    | "sessionTransactionPending"
    | "sessionMetadataRefreshRequired"
    | "sessionDdlCatalogRefreshRequired"
    | "sessionSnowflakeGridPending"
    | "sessionSnowflakeGridTransactions"
  >,
  sessionIds: Iterable<string>,
) {
  const sessionStates = { ...state.sessionStates };
  const sessionContexts = { ...state.sessionContexts };
  const sessionContextRevision = { ...state.sessionContextRevision };
  const sessionContextAuthorities = { ...state.sessionContextAuthorities };
  const snowflakeContextAuthorities = { ...state.snowflakeContextAuthorities };
  const snowflakeContextExpanded = { ...state.snowflakeContextExpanded };
  const sessionContextSwitching = { ...state.sessionContextSwitching };
  const sessionTransactionPending = { ...state.sessionTransactionPending };
  const sessionMetadataRefreshRequired = { ...state.sessionMetadataRefreshRequired };
  const sessionDdlCatalogRefreshRequired = { ...state.sessionDdlCatalogRefreshRequired };
  const sessionSnowflakeGridPending = { ...state.sessionSnowflakeGridPending };
  const sessionSnowflakeGridTransactions = { ...state.sessionSnowflakeGridTransactions };
  for (const id of sessionIds) {
    contextRequests.delete(id);
    pendingContextRequests.delete(id);
    schemaSwitchRequests.delete(id);
    delete sessionContextSwitching[id];
    delete sessionTransactionPending[id];
    delete sessionMetadataRefreshRequired[id];
    delete sessionDdlCatalogRefreshRequired[id];
    delete sessionSnowflakeGridPending[id];
    delete sessionSnowflakeGridTransactions[id];
    delete sessionContextAuthorities[id];
    delete snowflakeContextAuthorities[id];
    delete snowflakeContextExpanded[id];
    delete sessionStates[id];
    delete sessionContexts[id];
    delete sessionContextRevision[id];
  }
  return {
    sessionStates,
    sessionContexts,
    sessionContextRevision,
    sessionContextAuthorities,
    snowflakeContextAuthorities,
    snowflakeContextExpanded,
    sessionContextSwitching,
    sessionTransactionPending,
    sessionMetadataRefreshRequired,
    sessionDdlCatalogRefreshRequired,
    sessionSnowflakeGridPending,
    sessionSnowflakeGridTransactions,
  };
}
const storedTheme = localStorage.getItem("joinery.theme");

function requestIsCurrent(id: string, revision: number, connectionId: string, sessionId: string) {
  const state = useStudio.getState();
  return (
    state.tabs.some((tab) => tab.id === id && tab.revision === revision) &&
    state.sessions[connectionId]?.id === sessionId
  );
}
function asAppError(error: unknown): AppError {
  if (error && typeof error === "object" && "code" in error && "message" in error)
    return error as AppError;
  return { code: "database", message: messageOf(error) };
}
function retireAfterBrowseFailure(sessionId: string, error: AppError) {
  if (error.code === "connectionClosed" || error.code === "outcomeUnknown")
    useStudio.getState().retireSession(sessionId);
}
function samePhysicalColumnShape(
  expected: TableSchema["columns"],
  actual: ResultInfo["columns"],
): boolean {
  if (expected.length !== actual.length) return false;
  return actual.every((col, idx) => {
    const { foreignKey: _actualFk, ...actualRest } = col;
    const { foreignKey: _expectedFk, ...expectedRest } = expected[idx]!;
    return JSON.stringify(actualRest) === JSON.stringify(expectedRest);
  });
}

function isResultReferencedByOtherTabs(
  tabs: readonly StudioTab[],
  excludingTabIds: ReadonlySet<string>,
  resultId: string,
): boolean {
  for (const t of tabs) {
    if (excludingTabIds.has(t.id)) continue;
    if (t.results.some((r) => r.id === resultId)) return true;
    if (t.pages.some((p) => p.result.id === resultId)) return true;
  }
  return false;
}

export function reconcileTableSchemaVirtualFks(
  schema: TableSchema,
  vfks: readonly VirtualForeignKey[],
  connectionId: string,
): TableSchema {
  const colNames = new Set(schema.columns.map((c) => c.name));
  const foreignKeys = schema.foreignKeys.filter((fk) => !fk.virtualRelationship);
  const inboundForeignKeys = (schema.inboundForeignKeys ?? []).filter(
    (ifk) => !ifk.virtualRelationship,
  );
  for (const vfk of vfks) {
    if (vfk.connectionId !== connectionId) continue;
    if (
      vfk.source.schema === schema.table.schema &&
      vfk.source.name === schema.table.name &&
      vfk.columns.every((c) => colNames.has(c))
    ) {
      const dup = foreignKeys.some(
        (fk) =>
          fk.target.schema === vfk.target.schema &&
          fk.target.name === vfk.target.name &&
          fk.columns.length === vfk.columns.length &&
          fk.columns.every((c, i) => c === vfk.columns[i]) &&
          fk.targetColumns.length === vfk.targetColumns.length &&
          fk.targetColumns.every((c, i) => c === vfk.targetColumns[i]),
      );
      if (!dup) {
        foreignKeys.push({
          name: vfk.name,
          columns: [...vfk.columns],
          target: vfk.target,
          targetColumns: [...vfk.targetColumns],
          virtualRelationship: true,
        });
      }
    }
    if (
      vfk.target.schema === schema.table.schema &&
      vfk.target.name === schema.table.name &&
      vfk.targetColumns.every((c) => colNames.has(c))
    ) {
      const dup = inboundForeignKeys.some(
        (ifk) =>
          ifk.source.schema === vfk.source.schema &&
          ifk.source.name === vfk.source.name &&
          ifk.columns.length === vfk.columns.length &&
          ifk.columns.every((c, i) => c === vfk.columns[i]) &&
          ifk.targetColumns.length === vfk.targetColumns.length &&
          ifk.targetColumns.every((c, i) => c === vfk.targetColumns[i]),
      );
      if (!dup) {
        inboundForeignKeys.push({
          name: vfk.name,
          source: vfk.source,
          columns: [...vfk.columns],
          targetColumns: [...vfk.targetColumns],
          virtualRelationship: true,
        });
      }
    }
  }
  const fkCols = new Set(foreignKeys.flatMap((fk) => fk.columns));
  return {
    ...schema,
    foreignKeys,
    inboundForeignKeys,
    columns: schema.columns.map((col) => ({
      ...col,
      foreignKey: fkCols.has(col.name),
    })),
  };
}

function verifyPageColumns(schema: TableSchema | null, result: ResultInfo) {
  if (schema && samePhysicalColumnShape(schema.columns, result.columns))
    return result.editability?.status === "readOnly" ? { ...schema, editable: false } : schema;
  // A completed page may lose its edit lease while rows are fetched. Preserve
  // that known result, but narrow the whole table view until explicit refresh.
  // Every structural field must still match; this never promotes capabilities.
  if (
    schema &&
    result.editability?.status === "readOnly" &&
    JSON.stringify(schema.table) === JSON.stringify(result.table) &&
    schema.columns.length === result.columns.length &&
    result.columns.every((column, index) => {
      const {
        editable: _actualEdit,
        insertable: _actualInsert,
        foreignKey: _actualFk,
        ...actual
      } = column;
      const {
        editable: _expectedEdit,
        insertable: _expectedInsert,
        foreignKey: _expectedFk,
        ...expected
      } = schema.columns[index];
      return (
        !column.editable &&
        !column.insertable &&
        JSON.stringify(actual) === JSON.stringify(expected)
      );
    })
  )
    return {
      ...schema,
      editable: false,
      columns: result.columns.map((col, idx) => ({
        ...col,
        foreignKey: schema.columns[idx]?.foreignKey ?? col.foreignKey,
      })),
    };
  if (!isConsoleOwnedResult(result))
    void client.call("release_result", { resultId: result.id }).catch(() => {});
  throw new Error("The table schema changed. Refresh the table before continuing.");
}

/** Frontend scheduling only; the driver independently validates one-use authority. */
export function revokeSchemaSelectionBeforeWork(
  sessionId: string,
  transactionControl = false,
  allowGridTransaction = false,
) {
  const state = useStudio.getState();
  assertOrdinarySession(sessionId);
  if (state.sessionContextSwitching[sessionId])
    throw new Error("Wait for the schema change to finish first");
  if (state.sessionTransactionPending[sessionId])
    throw new Error("Wait for the transaction action to finish first");
  if (
    !allowGridTransaction &&
    (state.sessionSnowflakeGridPending[sessionId] ||
      state.sessionSnowflakeGridTransactions[sessionId])
  )
    throw new Error("Finish or roll back the open Snowflake grid transaction first");
  if (
    !transactionControl &&
    state.sessionStates[sessionId]?.transactionControlAuthority?.status === "rollbackOnly"
  )
    throw new Error("Only rollback recovery is available. Roll back or disconnect this session.");
  state.invalidateSessionContextAuthority(sessionId);
  const runtime = useStudio.getState().sessionStates[sessionId];
  if (runtime?.transactionControlAuthority?.status === "observed") {
    const { transactionControlAuthority: _authority, ...observed } = runtime;
    useStudio.setState((current) => ({
      sessionStates: { ...current.sessionStates, [sessionId]: observed },
    }));
  }
}

/** Guard a template's observation without changing the editor being validated. */
export async function observeTemplateSessionContext(
  sessionId: string,
): Promise<SessionRuntimeState> {
  revokeSchemaSelectionBeforeWork(sessionId);
  if (!isSessionActive(useStudio.getState(), sessionId))
    throw new Error("This database session is disconnected");
  const request = beginContextRequest(sessionId);
  try {
    const options = await client.call("session_contexts", { sessionId });
    if (
      contextRequests.get(sessionId) !== request ||
      !isSessionActive(useStudio.getState(), sessionId)
    )
      throw new Error("The console context changed. Insert the template again.");
    if (!options?.state || options.state.sessionId !== sessionId)
      throw new Error("Session observation is unavailable");
    // Publishing here can advance the editor context revision, dismissing the
    // template dialog before its own current-context checks and errors finish.
    // This read never grants schema-switch authority to the UI.
    return options.state;
  } finally {
    finishContextRequest(sessionId, request);
  }
}

export const useStudio = create<StudioState>((set, get) => ({
  bootstrap: null,
  error: null,
  sessions: {},
  consoleSessions: {},
  snowflakeConsoleReceipts: restoredConsoleReceipts(),
  snowflakeConsoleResultOwners: {},
  sessionPool: {},
  sessionStates: {},
  sessionContexts: {},
  sessionContextRevision: {},
  sessionContextAuthorities: {},
  snowflakeContextAuthorities: {},
  snowflakeContextExpanded: {},
  sessionContextSwitching: {},
  sessionTransactionPending: {},
  sessionMetadataRefreshRequired: {},
  sessionDdlCatalogRefreshRequired: {},
  sessionSnowflakeGridPending: {},
  sessionSnowflakeGridTransactions: {},
  catalog: {},
  objects: {},
  selectedConnectionId: null,
  tabs: [],
  activeTabId: null,
  closedTabs: [],
  tableLayouts: [],
  savedQueries: [],
  bookmarks: [],
  sqlFormatSettings: DEFAULT_SQL_FORMAT_SETTINGS,
  sqlSnippets: [],
  virtualForeignKeys: [],
  recentLocations: [],
  shortcuts: {},
  theme: storedTheme === "light" || storedTheme === "contrast" ? storedTheme : "dark",
  density: "comfortable",
  sidebarVisible: true,
  sidebarCollapsed: false,
  inspectorVisible: false,
  toast: null,
  async initialize() {
    try {
      const bootstrap = await client.call<Bootstrap>("bootstrap");
      const workspace = !get().bootstrap ? bootstrap.workspace : null;
      const restored =
        workspace?.drafts
          .filter((draft) =>
            bootstrap.profiles.some(
              (profile) => profile.id === draft.connectionId && profile.storeDrafts,
            ),
          )
          .map((draft) => ({
            ...tab("query", draft.title, draft.connectionId),
            id: draft.id,
            sql: draft.sql,
            pinned: Boolean(draft.pinned),
          })) ?? [];
      const restoredLayouts =
        workspace?.tableLayouts
          ?.filter(
            (layout) =>
              !isDefaultTableLayout(layout) &&
              bootstrap.profiles.some((profile) => profile.id === layout.connectionId),
          )
          .slice(-MAX_TABLE_LAYOUTS) ?? [];
      const existing = get().tabs;
      const tabs = orderPinnedFirst([
        ...restored.filter((draft) => !existing.some((tab) => tab.id === draft.id)),
        ...existing,
      ]);
      const sidebarVisible = workspace
        ? workspace.sidebarCollapsed
          ? false
          : workspace.sidebarVisible
        : get().sidebarVisible;
      set({
        bootstrap,
        savedQueries: bootstrap.savedQueries ?? [],
        bookmarks: bootstrap.bookmarks ?? [],
        sqlFormatSettings: bootstrap.sqlFormatSettings ?? DEFAULT_SQL_FORMAT_SETTINGS,
        sqlSnippets: bootstrap.sqlSnippets ?? [],
        virtualForeignKeys: bootstrap.virtualForeignKeys ?? [],
        error: null,
        ...(workspace
          ? {
              theme: workspace.theme as StudioState["theme"],
              density: workspace.density as StudioState["density"],
              sidebarVisible,
              sidebarCollapsed: !sidebarVisible,
              inspectorVisible: workspace.inspectorVisible,
              shortcuts: sanitizeShortcutOverrides(
                (workspace.shortcuts ?? {}) as Record<string, string>,
              ),
              tabs,
              tableLayouts: restoredLayouts,
              activeTabId:
                get().activeTabId ??
                restored.find((draft) => draft.id === workspace.activeDraftId)?.id ??
                restored[0]?.id ??
                null,
            }
          : {}),
      });
    } catch (error) {
      set({ error: messageOf(error) });
    }
  },
  notify(message) {
    clearTimeout(toastTimer);
    set({ toast: message });
    toastTimer = setTimeout(() => set({ toast: null }), 6000);
  },
  setTheme(theme) {
    localStorage.setItem("joinery.theme", theme);
    document.documentElement.dataset.theme = theme;
    set({ theme });
  },
  toggleSidebar() {
    set((state) => {
      const sidebarVisible = !state.sidebarVisible;
      return { sidebarVisible, sidebarCollapsed: !sidebarVisible };
    });
  },
  toggleInspector() {
    set((state) => ({ inspectorVisible: !state.inspectorVisible }));
  },
  setShortcut(actionId, binding) {
    const currentOverrides = get().shortcuts;
    if (binding === null || binding.trim() === "") {
      const next = { ...currentOverrides };
      delete next[actionId];
      set({ shortcuts: next });
      const def = SHORTCUT_BY_ID[actionId];
      return { valid: true, canonical: def.defaultBinding };
    }
    const validation = validateShortcutOverride(actionId, binding, currentOverrides);
    if (!validation.valid || !validation.canonical) {
      return validation;
    }
    const next = { ...currentOverrides };
    if (validation.canonical === SHORTCUT_BY_ID[actionId].defaultBinding) {
      delete next[actionId];
    } else {
      next[actionId] = validation.canonical;
    }
    set({ shortcuts: next });
    return validation;
  },
  resetShortcuts() {
    set({ shortcuts: {} });
  },
  setTableLayout(connectionId, table, layout) {
    set((state) => {
      const remaining = state.tableLayouts.filter(
        (entry) => !sameTableLayoutTarget(entry, connectionId, table),
      );
      if (!layout || isDefaultTableLayout(layout)) return { tableLayouts: remaining };
      return { tableLayouts: [...remaining, layout].slice(-MAX_TABLE_LAYOUTS) };
    });
  },
  resetTableLayout(connectionId, table) {
    get().setTableLayout(connectionId, table, null);
  },
  setResultLayout(tabId, resultId, layout) {
    set((state) => ({
      tabs: state.tabs.map((entry) => {
        if (entry.id !== tabId) return entry;
        const currentLayouts = { ...(entry.resultLayouts ?? {}) };
        if (!layout || isDefaultTableLayout(layout)) {
          delete currentLayouts[resultId];
        } else {
          currentLayouts[resultId] = layout;
        }
        return {
          ...entry,
          resultLayouts: Object.keys(currentLayouts).length > 0 ? currentLayouts : undefined,
        };
      }),
    }));
  },
  togglePinResult(tabId, resultId) {
    const snapshot = get();
    const current = snapshot.tabs.find((entry) => entry.id === tabId && entry.kind === "query");
    if (!current) return false;
    if (["queued", "running"].includes(current.status)) {
      get().notify("Wait for the query to finish before pinning its result.");
      return false;
    }
    const target = resultId
      ? current.results.find((r) => r.id === resultId)
      : (current.results.find((r) => r.id === current.activeResultId) ?? current.results[0]);
    if (!target) {
      get().notify("Run a query first to pin its result.");
      return false;
    }
    if (isConsoleOwnedResult(target)) {
      get().notify("Retained Snowflake console results stay managed by their console receipt.");
      return false;
    }
    const existingPinned = current.pinnedResultIds ?? [];
    if (existingPinned.includes(target.id)) {
      const nextIds = existingPinned.filter((id) => id !== target.id);
      const nextLabels = current.pinnedResultLabels ? { ...current.pinnedResultLabels } : undefined;
      if (nextLabels) delete nextLabels[target.id];
      set((state) => ({
        tabs: state.tabs.map((entry) =>
          entry.id === tabId
            ? {
                ...entry,
                pinnedResultIds: nextIds.length > 0 ? nextIds : undefined,
                pinnedResultLabels:
                  nextLabels && Object.keys(nextLabels).length > 0 ? nextLabels : undefined,
              }
            : entry,
        ),
      }));
      return false;
    }
    if (existingPinned.length >= MAX_PINNED_RESULTS_PER_TAB) {
      get().notify(
        `Pin at most ${MAX_PINNED_RESULTS_PER_TAB} results per query console, or open a result in its own tab.`,
      );
      return false;
    }
    dirtyStore.clearTab(target.id, [target.id]);
    const label = `Pinned #${++pinnedResultSeq} (Q${target.statementIndex + 1})`;
    const nextIds = [...existingPinned, target.id];
    const nextLabels = { ...(current.pinnedResultLabels ?? {}), [target.id]: label };
    set((state) => ({
      tabs: state.tabs.map((entry) =>
        entry.id === tabId
          ? {
              ...entry,
              results: entry.results.map((r) =>
                r.id === target.id
                  ? {
                      ...r,
                      columns: r.columns.map((c) => ({ ...c, editable: false, insertable: false })),
                      editability: {
                        status: "readOnly",
                        reason: "Pinned result snapshot is preserved read-only across console runs",
                      },
                    }
                  : r,
              ),
              pinnedResultIds: nextIds,
              pinnedResultLabels: nextLabels,
            }
          : entry,
      ),
    }));
    return true;
  },
  closeResultSubtab(tabId, resultId) {
    const snapshot = get();
    const current = snapshot.tabs.find((entry) => entry.id === tabId && entry.kind === "query");
    if (!current) return;
    const target = current.results.find((r) => r.id === resultId);
    if (!target) return;
    dirtyStore.clearTab(target.id, [target.id]);
    if (!isResultReferencedByOtherTabs(snapshot.tabs, new Set([tabId]), target.id)) {
      resultCache.release(target.id);
      if (!isConsoleOwnedResult(target)) {
        void client.call("release_result", { resultId: target.id }).catch(() => {});
      }
    }
    const idx = current.results.findIndex((r) => r.id === resultId);
    const remaining = current.results.filter((r) => r.id !== resultId);
    const nextPinned = current.pinnedResultIds?.filter((id) => id !== resultId);
    const nextLabels = current.pinnedResultLabels ? { ...current.pinnedResultLabels } : undefined;
    if (nextLabels) delete nextLabels[resultId];
    const nextLayouts = current.resultLayouts ? { ...current.resultLayouts } : undefined;
    if (nextLayouts) delete nextLayouts[resultId];
    const nextActive =
      current.activeResultId === resultId
        ? (remaining[Math.min(idx, remaining.length - 1)]?.id ?? null)
        : current.activeResultId;
    set((state) => ({
      tabs: state.tabs.map((entry) =>
        entry.id === tabId
          ? {
              ...entry,
              results: remaining,
              activeResultId: nextActive,
              pinnedResultIds: nextPinned && nextPinned.length > 0 ? nextPinned : undefined,
              pinnedResultLabels:
                nextLabels && Object.keys(nextLabels).length > 0 ? nextLabels : undefined,
              resultLayouts:
                nextLayouts && Object.keys(nextLayouts).length > 0 ? nextLayouts : undefined,
            }
          : entry,
      ),
    }));
  },
  async prepareQueryRunResults(tabId) {
    const snapshot = get();
    const current = snapshot.tabs.find((entry) => entry.id === tabId && entry.kind === "query");
    if (!current) return;
    const pinnedSet = new Set(current.pinnedResultIds ?? []);
    const keptResults = current.results.filter((r) => pinnedSet.has(r.id));
    const excluding = new Set([tabId]);
    for (const previous of current.results) {
      if (pinnedSet.has(previous.id)) continue;
      dirtyStore.clearTab(previous.id, [previous.id]);
      if (!isResultReferencedByOtherTabs(get().tabs, excluding, previous.id)) {
        resultCache.release(previous.id);
        if (!isConsoleOwnedResult(previous)) {
          await client.call("release_result", { resultId: previous.id });
        }
      }
    }
    const nextLayouts = current.resultLayouts
      ? Object.fromEntries(
          Object.entries(current.resultLayouts).filter(([rid]) => pinnedSet.has(rid)),
        )
      : undefined;
    get().updateTab(tabId, {
      results: keptResults,
      activeResultId: null,
      resultLayouts: nextLayouts && Object.keys(nextLayouts).length > 0 ? nextLayouts : undefined,
    });
  },
  detachResultToTab(tabId, resultId) {
    const state = get();
    const sourceTab = state.tabs.find(
      (entry) => entry.id === tabId && (entry.kind === "query" || entry.kind === "workspaceSql"),
    );
    if (!sourceTab) return null;
    if (["queued", "running"].includes(sourceTab.status)) {
      get().notify("Wait for the query to finish before opening its result in a tab.");
      return null;
    }
    const targetResult = resultId
      ? sourceTab.results.find((r) => r.id === resultId)
      : (sourceTab.results.find((r) => r.id === sourceTab.activeResultId) ?? sourceTab.results[0]);
    if (!targetResult) {
      get().notify("Run a query first to open its result in a tab.");
      return null;
    }
    if (isConsoleOwnedResult(targetResult)) {
      get().notify("Retained Snowflake console results stay managed by their console receipt.");
      return null;
    }
    dirtyStore.clearTab(targetResult.id, [targetResult.id]);
    const detachedResult: ResultInfo = {
      ...targetResult,
      columns: targetResult.columns.map((c) => ({ ...c, editable: false, insertable: false })),
      editability: {
        status: "readOnly",
        reason: "Detached result snapshot is read-only; open originating SQL or table to edit",
      },
    };
    const next = tab(
      "result",
      `${sourceTab.title} · Query ${targetResult.statementIndex + 1}`,
      sourceTab.connectionId,
    );
    next.results = [detachedResult];
    next.activeResultId = detachedResult.id;
    next.status = "completed";
    next.originTabId = sourceTab.id;
    next.originKind = sourceTab.kind === "workspaceSql" ? "workspaceSql" : "query";
    next.originSql = sourceTab.sql;
    if (sourceTab.resultLayouts?.[targetResult.id]) {
      next.resultLayouts = { [targetResult.id]: sourceTab.resultLayouts[targetResult.id] };
    }
    const remainingResults = sourceTab.results.filter((r) => r.id !== targetResult.id);
    const nextSourceLayouts = sourceTab.resultLayouts ? { ...sourceTab.resultLayouts } : undefined;
    if (nextSourceLayouts) delete nextSourceLayouts[targetResult.id];
    const nextPinnedIds = sourceTab.pinnedResultIds?.filter((id) => id !== targetResult.id);
    const nextPinnedLabels = sourceTab.pinnedResultLabels
      ? { ...sourceTab.pinnedResultLabels }
      : undefined;
    if (nextPinnedLabels) delete nextPinnedLabels[targetResult.id];
    set((prev) => ({
      tabs: orderPinnedFirst([
        ...prev.tabs.map((entry) =>
          entry.id === sourceTab.id
            ? {
                ...entry,
                results: remainingResults,
                activeResultId:
                  entry.activeResultId === targetResult.id
                    ? (remainingResults[0]?.id ?? null)
                    : entry.activeResultId,
                resultLayouts:
                  nextSourceLayouts && Object.keys(nextSourceLayouts).length > 0
                    ? nextSourceLayouts
                    : undefined,
                pinnedResultIds:
                  nextPinnedIds && nextPinnedIds.length > 0 ? nextPinnedIds : undefined,
                pinnedResultLabels:
                  nextPinnedLabels && Object.keys(nextPinnedLabels).length > 0
                    ? nextPinnedLabels
                    : undefined,
              }
            : entry,
        ),
        next,
      ]),
      activeTabId: next.id,
    }));
    get().recordRecentLocation({
      key: `tab:${next.id}`,
      label: next.title,
      subtitle: "Detached query result",
      target: { kind: "tab", tabId: next.id },
    });
    return next.id;
  },
  openOriginatingSql(resultTabId) {
    const state = get();
    const resultTab = state.tabs.find(
      (entry) => entry.id === resultTabId && entry.kind === "result",
    );
    if (!resultTab) return null;
    if (resultTab.originTabId) {
      const origin = state.tabs.find(
        (entry) =>
          entry.id === resultTab.originTabId &&
          (entry.kind === "query" || entry.kind === "workspaceSql"),
      );
      if (origin && (resultTab.originSql == null || origin.sql === resultTab.originSql)) {
        set({ activeTabId: origin.id });
        return origin.id;
      }
    }
    if (resultTab.originKind === "workspaceSql") {
      const existingWs = state.tabs.find((entry) => entry.kind === "workspaceSql");
      if (existingWs) {
        set((prev) => ({
          tabs: prev.tabs.map((t) =>
            t.id === existingWs.id ? { ...t, sql: resultTab.originSql ?? "" } : t,
          ),
          activeTabId: existingWs.id,
        }));
        return existingWs.id;
      }
      const next = tab("workspaceSql", "Workspace SQL", resultTab.connectionId);
      next.sql = resultTab.originSql ?? "";
      set((prev) => ({ tabs: [...prev.tabs, next], activeTabId: next.id }));
      return next.id;
    }
    return get().openQuery(resultTab.connectionId, resultTab.originSql ?? "");
  },
  setWorkspaceSqlResult(tabId, sql, response) {
    const snapshot = get();
    const current = snapshot.tabs.find(
      (entry) => entry.id === tabId && entry.kind === "workspaceSql",
    );
    if (!current) {
      if (
        !isConsoleOwnedResult(response.result) &&
        !isResultReferencedByOtherTabs(snapshot.tabs, new Set(), response.result.id)
      ) {
        void client.call("release_result", { resultId: response.result.id }).catch(() => {});
      }
      return;
    }
    const excluding = new Set([tabId]);
    for (const old of current.results) {
      if (
        old.id !== response.result.id &&
        !isResultReferencedByOtherTabs(snapshot.tabs, excluding, old.id)
      ) {
        dirtyStore.clearTab(old.id, [old.id]);
        resultCache.release(old.id);
        if (!isConsoleOwnedResult(old)) {
          void client.call("release_result", { resultId: old.id }).catch(() => {});
        }
      }
    }
    set((state) => ({
      tabs: state.tabs.map((entry) =>
        entry.id === tabId
          ? {
              ...entry,
              sql,
              results: [response.result],
              activeResultId: response.result.id,
              status: "completed",
              elapsedMs: response.elapsedMs,
              error: null,
            }
          : entry,
      ),
    }));
  },
  async startSnowflakeConsole(tabId, plan, approval) {
    const storageProblem = consoleReceiptStorageProblem();
    if (storageProblem) throw new Error(storageProblem);
    const active = get().tabs.find((entry) => entry.id === tabId);
    const session = active && querySessionForTab(active, get().sessions);
    if (!active || !session?.consoleMode || session.id !== plan.review.sessionId)
      throw new Error("The reviewed console session changed. Review again in the current session.");
    validateConsolePlan(plan, session.id);
    if (
      Object.values(get().snowflakeConsoleReceipts).some((entry) =>
        consoleReceiptBlocksSession(entry, session.id),
      )
    )
      throw new Error("Reconcile the previous console receipt before starting another run.");
    const clientRequestId = crypto.randomUUID();
    const revision = active.revision + 1;
    const receipt: SnowflakeConsoleReceipt = {
      clientRequestId,
      sessionId: session.id,
      connectionId: session.connectionId,
      planDigest: plan.digest,
      stepCount: plan.review.statements.length,
      tabId,
      tabRevision: revision,
      lookupPending: true,
      startPending: true,
    };
    const receipts = { ...get().snowflakeConsoleReceipts, [clientRequestId]: receipt };
    rememberConsoleReceipts(receipts); // Fails before any remote admission.
    set({ snowflakeConsoleReceipts: receipts });
    for (const result of active.results) {
      resultCache.release(result.id);
      dirtyStore.clearTab(result.id, [result.id]);
      if (!isConsoleOwnedResult(result))
        void client.call("release_result", { resultId: result.id }).catch(() => {});
    }
    get().updateTab(tabId, {
      results: [],
      activeResultId: null,
      status: "queued",
      runId: null,
      revision,
      dataSessionId: session.id,
      error: null,
      elapsedMs: 0,
      metrics: {},
      executionSummary: undefined,
      snowflakeDmlResult: undefined,
      snowflakeDdlResult: undefined,
      planSource: undefined,
      clientStartedAt: performance.now(),
    });
    const acceptsConsoleEvent = (event: QueryEvent) => {
      const resultId =
        event.type === "result"
          ? event.result.id
          : event.type === "progress"
            ? event.resultId
            : null;
      if (resultId === null) return true;
      const current = get().snowflakeConsoleReceipts[clientRequestId];
      if (!current) return false;
      if (!current.view) return !current.settledProof && !current.notAdmitted;
      return (
        current.view.run.state !== "terminal" ||
        current.view.pendingRecording ||
        current.view.results.some((result) => result.id === resultId)
      );
    };
    const publish = beginEventBatch(tabId, acceptsConsoleEvent);
    const batch = (event: QueryEvent) => {
      // Register ownership before a queued Result can meet stale-tab cleanup.
      const eventRunId =
        event.type === "started"
          ? event.runId
          : event.type === "result"
            ? event.result.runId
            : null;
      if (eventRunId) {
        const current = get().snowflakeConsoleReceipts[clientRequestId];
        if (!current) {
          if (acceptsConsoleEvent(event)) publish(event);
          return;
        }
        if (current.notAdmitted) {
          set((state) => ({
            snowflakeConsoleReceipts: {
              ...state.snowflakeConsoleReceipts,
              [clientRequestId]: {
                ...current,
                runId: current.runId ?? eventRunId,
                decisionConflict: true,
                lookupPending: true,
                error:
                  "An original run event contradicts the retained refusal; execution remains held.",
              },
            },
          }));
          return;
        }
        if (current.runId && current.runId !== eventRunId) {
          set((state) => ({
            snowflakeConsoleReceipts: {
              ...state.snowflakeConsoleReceipts,
              [clientRequestId]: {
                ...current,
                lookupPending: true,
                error: "Console event identity changed; reconcile the original receipt.",
              },
            },
          }));
          return;
        }
        if (!acceptsConsoleEvent(event)) return;
        if (event.type === "result")
          set((state) => ({
            snowflakeConsoleResultOwners: {
              ...state.snowflakeConsoleResultOwners,
              [event.result.id]: eventRunId,
            },
          }));
        set((state) => ({
          snowflakeConsoleReceipts: {
            ...state.snowflakeConsoleReceipts,
            [clientRequestId]: { ...current, runId: eventRunId },
          },
        }));
      }
      if (acceptsConsoleEvent(event)) publish(event);
    };
    try {
      const result = await client.startSnowflakeConsole(
        {
          sessionId: session.id,
          planId: plan.review.id,
          planDigest: plan.digest,
          clientRequestId,
          approval,
        },
        batch,
      );
      const recovered = get().snowflakeConsoleReceipts[clientRequestId];
      if (result.kind === "notAdmitted") {
        if (consoleDecisionConflicts(recovered, result))
          set((state) => ({
            snowflakeConsoleReceipts: {
              ...state.snowflakeConsoleReceipts,
              [clientRequestId]: { ...recovered, decisionConflict: true },
            },
          }));
        const settled = adoptConsoleNotAdmitted(recovered, result);
        const receipts = { ...get().snowflakeConsoleReceipts, [clientRequestId]: settled };
        // Keep the opaque ID so this durable decision can be recovered after reload.
        rememberConsoleReceipts(receipts);
        set({ snowflakeConsoleReceipts: receipts });
        const tab = get().tabs.find((entry) => entry.id === tabId && entry.revision === revision);
        if (tab)
          get().updateTab(tabId, {
            status: "failed",
            runId: null,
            error: {
              code: result.code,
              message: "No SQL was admitted. Review again to create a new request.",
            },
          });
        return;
      }
      const runId = result.runId;
      if (recovered.notAdmitted) {
        set((state) => ({
          snowflakeConsoleReceipts: {
            ...state.snowflakeConsoleReceipts,
            [clientRequestId]: { ...recovered, decisionConflict: true },
          },
        }));
        throw new Error(
          "The original start response contradicts the retained refusal; execution remains held.",
        );
      }
      if (recovered.runId && recovered.runId !== runId)
        throw new Error(
          "Start response conflicts with the original recovered run; reconcile preserved evidence.",
        );
      set((state) => ({
        snowflakeConsoleReceipts: {
          ...state.snowflakeConsoleReceipts,
          [clientRequestId]: {
            ...state.snowflakeConsoleReceipts[clientRequestId],
            runId,
            startPending: false,
          },
        },
      }));
    } catch (error) {
      // A lost start response never permits another start. Lookup the same ID.
      set((state) => ({
        snowflakeConsoleReceipts: {
          ...state.snowflakeConsoleReceipts,
          [clientRequestId]: {
            ...state.snowflakeConsoleReceipts[clientRequestId],
            startPending: false,
            error: `Start response unavailable: ${messageOf(error)}. Recovering the original receipt; SQL will not be resent.`,
          },
        },
      }));
    }
    await get().refreshSnowflakeConsole(clientRequestId);
  },
  async refreshSnowflakeConsole(clientRequestId) {
    if (receiptLookups.has(clientRequestId)) return;
    const before = get().snowflakeConsoleReceipts[clientRequestId];
    if (!before) return;
    receiptLookups.add(clientRequestId);
    try {
      const decision = await client.snowflakeConsoleRequest(clientRequestId);
      if (!decision)
        throw new Error(
          "No durable receipt found yet. Submission is not disproved; do not rerun to recover it.",
        );
      const current = get().snowflakeConsoleReceipts[clientRequestId];
      if (!current) return;
      if (consoleDecisionConflicts(current, decision)) {
        set((state) => ({
          snowflakeConsoleReceipts: {
            ...state.snowflakeConsoleReceipts,
            [clientRequestId]: { ...current, decisionConflict: true },
          },
        }));
        throw new Error(
          "Opposite native decisions were observed for this request; execution remains held and the original evidence was preserved.",
        );
      }
      if (decision.kind === "notAdmitted") {
        const settled = adoptConsoleNotAdmitted(current, decision);
        const receipts = { ...get().snowflakeConsoleReceipts, [clientRequestId]: settled };
        rememberConsoleReceipts(receipts);
        set({ snowflakeConsoleReceipts: receipts });
        const tab = get().tabs.find(
          (entry) => entry.id === current.tabId && entry.revision === current.tabRevision,
        );
        if (tab && querySessionForTab(tab, get().sessions)?.id === settled.sessionId)
          get().updateTab(tab.id, {
            status: "failed",
            runId: null,
            error: {
              code: decision.code,
              message: "No SQL was admitted. Review again to create a new request.",
            },
          });
        return;
      }
      const view = decision.view;
      validateConsoleReceipt(current, view);
      const authoritativeResults = view.run.state === "terminal" && !view.pendingRecording;
      const retained = new Map(view.results.map((result) => [result.id, result]));
      if (!authoritativeResults) {
        // A running lookup may have been serialized before a newer Result
        // event. Absence in that snapshot cannot mean explicit row release.
        for (const result of [
          ...(current.view?.results ?? []),
          ...get().tabs.flatMap((tab) => tab.results),
        ]) {
          if (result.runId !== view.run.metadata.runId) continue;
          const prior = retained.get(result.id);
          if (
            !prior ||
            result.retainedRows > prior.retainedRows ||
            (result.retainedRows === prior.retainedRows && prior.status === "streaming")
          )
            retained.set(result.id, result);
        }
      }
      const results = [...retained.values()].sort((a, b) => a.statementIndex - b.statementIndex);
      const liveIds = new Set(results.map((result) => result.id));
      const releasedIds = new Set(
        Object.entries(get().snowflakeConsoleResultOwners)
          .filter(
            ([id, run]) =>
              authoritativeResults && run === view.run.metadata.runId && !liveIds.has(id),
          )
          .map(([id]) => id),
      );
      for (const tab of get().tabs)
        for (const result of tab.results)
          if (
            authoritativeResults &&
            result.runId === view.run.metadata.runId &&
            !liveIds.has(result.id)
          )
            releasedIds.add(result.id);
      for (const id of releasedIds) {
        resultCache.release(id);
        dirtyStore.clearTab(id, [id]);
      }
      const next: SnowflakeConsoleReceipt = {
        ...current,
        view: { ...view, results },
        runId: view.run.metadata.runId,
        sessionId: view.run.metadata.sessionId,
        connectionId: view.run.metadata.connectionId,
        planDigest: view.run.metadata.planDigest,
        stepCount: view.run.metadata.stepCount,
        lookupPending: false,
        lastAccessed: consoleReceiptAccess(),
        error: undefined,
      };
      set((state) => ({
        snowflakeConsoleReceipts: { ...state.snowflakeConsoleReceipts, [clientRequestId]: next },
        snowflakeConsoleResultOwners: {
          ...Object.fromEntries(
            Object.entries(state.snowflakeConsoleResultOwners).filter(
              ([, run]) => !authoritativeResults || run !== view.run.metadata.runId,
            ),
          ),
          ...Object.fromEntries(results.map((result) => [result.id, result.runId])),
        },
        ...(releasedIds.size
          ? {
              tabs: state.tabs.map((tab) => ({
                ...tab,
                results: tab.results.filter((result) => !releasedIds.has(result.id)),
                activeResultId:
                  tab.activeResultId && releasedIds.has(tab.activeResultId)
                    ? null
                    : tab.activeResultId,
              })),
            }
          : {}),
      }));
      const tab = get().tabs.find(
        (entry) => entry.id === current.tabId && entry.revision === current.tabRevision,
      );
      if (tab && querySessionForTab(tab, get().sessions)?.id === next.sessionId) {
        get().updateTab(tab.id, {
          status: consoleRunStatus(view),
          runId: consoleReceiptHeld(next) ? next.runId! : null,
          results,
          activeResultId: results.some((r) => r.id === tab.activeResultId)
            ? tab.activeResultId
            : (results[0]?.id ?? null),
        });
      }
      if (current.cancellationRequested && !view.run.terminal)
        await client.call("cancel_run", { runId: next.runId! });
      if (view.run.observations.retired) get().retireSession(view.run.metadata.sessionId);
    } catch (error) {
      set((state) => ({
        snowflakeConsoleReceipts: {
          ...state.snowflakeConsoleReceipts,
          [clientRequestId]: {
            ...state.snowflakeConsoleReceipts[clientRequestId],
            error: messageOf(error),
            lookupPending: true,
          },
        },
      }));
    } finally {
      receiptLookups.delete(clientRequestId);
      set((state) => ({
        snowflakeConsoleReceipts: compactConsoleReceipts(state.snowflakeConsoleReceipts),
      }));
    }
  },
  async openSnowflakeConsoleResults(clientRequestId) {
    if (receiptLookups.has(clientRequestId))
      throw new Error("Wait for receipt reconciliation, then open retained results.");
    await get().refreshSnowflakeConsole(clientRequestId);
    const receipt = get().snowflakeConsoleReceipts[clientRequestId];
    const view = receipt?.view;
    if (!view || receipt.lookupPending)
      throw new Error("Reconcile this receipt before opening its retained results.");
    if (!view.results.length)
      throw new Error(
        "Retained result rows are unavailable. This does not mean zero rows were returned.",
      );
    const primary = get().sessions[view.run.metadata.connectionId];
    const originalSession = primary?.id === view.run.metadata.sessionId ? primary : undefined;
    // An absent original session stays unbound. Opening rows never logs in,
    // borrows a replacement physical session, or restores SQL/parameters.
    const tabId = get().openQuery(originalSession?.connectionId ?? null, "");
    const revision = get().tabs.find((tab) => tab.id === tabId)!.revision;
    get().updateTab(tabId, {
      title: "Console retained results",
      results: view.results,
      activeResultId: view.results[0].id,
      dataSessionId: view.run.metadata.sessionId,
      status: consoleRunStatus(view),
      runId: consoleReceiptHeld(receipt) ? view.run.metadata.runId : null,
    });
    set((state) => ({
      snowflakeConsoleReceipts: {
        ...state.snowflakeConsoleReceipts,
        [clientRequestId]: { ...receipt, tabId, tabRevision: revision },
      },
    }));
  },
  async discoverSnowflakeConsoleReceipt(summary) {
    validateConsoleSummary(summary);
    const current = get().snowflakeConsoleReceipts[summary.clientRequestId];
    if (!current)
      set((state) => ({
        snowflakeConsoleReceipts: {
          ...state.snowflakeConsoleReceipts,
          [summary.clientRequestId]: {
            clientRequestId: summary.clientRequestId,
            sessionId: summary.sessionId,
            planDigest: summary.planDigest,
            ...(summary.connectionId ? { connectionId: summary.connectionId } : {}),
            ...(summary.runId ? { runId: summary.runId } : {}),
            inventoried: true,
            nativeSettled: summary.archiveEligible,
            lastAccessed: consoleReceiptAccess(),
            lookupPending: true,
          },
        },
      }));
    await get().refreshSnowflakeConsole(summary.clientRequestId);
    await get().handoffSnowflakeConsoleReceipt(summary.clientRequestId);
  },
  async handoffSnowflakeConsoleReceipt(clientRequestId) {
    const receipt = get().snowflakeConsoleReceipts[clientRequestId];
    if (!receipt || consoleReceiptHeld(receipt)) return;
    const summary = await client.call("snowflake_console_receipt_summary", { clientRequestId });
    if (!summary)
      throw new Error(
        "Native inventory has not confirmed the receipt; its browser correlation remains preserved.",
      );
    const current = get().snowflakeConsoleReceipts[clientRequestId];
    if (!current) return;
    validateConsoleHandoff(current, summary);
    const receipts = {
      ...get().snowflakeConsoleReceipts,
      [clientRequestId]: {
        ...current,
        inventoried: true,
        nativeSettled: true,
        lastAccessed: consoleReceiptAccess(),
      },
    };
    rememberConsoleReceipts(receipts);
    set({ snowflakeConsoleReceipts: compactConsoleReceipts(receipts) });
  },
  async releaseSnowflakeConsoleResults(clientRequestId) {
    if (receiptLookups.has(clientRequestId))
      throw new Error("Wait for receipt reconciliation, then release retained result rows.");
    const receipt = get().snowflakeConsoleReceipts[clientRequestId];
    if (!receipt?.view || consoleReceiptHeld(receipt))
      throw new Error("Reconcile this settled receipt before releasing retained result rows.");
    receiptLookups.add(clientRequestId);
    try {
      await client.call("release_snowflake_console_results", { clientRequestId });
      // Native release is authoritative. A failed/lost response leaves local state
      // intact for SQL-free reconciliation; rows are never resubmitted.
      const ids = new Set([
        ...receipt.view.results.map((result) => result.id),
        ...Object.entries(get().snowflakeConsoleResultOwners)
          .filter(([, run]) => run === receipt.runId)
          .map(([id]) => id),
        ...get().tabs.flatMap((tab) =>
          tab.results.filter((result) => result.runId === receipt.runId).map((result) => result.id),
        ),
      ]);
      for (const id of ids) {
        resultCache.release(id);
        dirtyStore.clearTab(id, [id]);
      }
      set((state) => ({
        snowflakeConsoleReceipts: state.snowflakeConsoleReceipts[clientRequestId]
          ? {
              ...state.snowflakeConsoleReceipts,
              [clientRequestId]: {
                ...state.snowflakeConsoleReceipts[clientRequestId],
                view: { ...receipt.view!, results: [] },
              },
            }
          : state.snowflakeConsoleReceipts,
        snowflakeConsoleResultOwners: Object.fromEntries(
          Object.entries(state.snowflakeConsoleResultOwners).filter(([id]) => !ids.has(id)),
        ),
        tabs: state.tabs.map((tab) => ({
          ...tab,
          results: tab.results.filter((result) => !ids.has(result.id)),
          activeResultId:
            tab.activeResultId && ids.has(tab.activeResultId) ? null : tab.activeResultId,
        })),
      }));
    } finally {
      receiptLookups.delete(clientRequestId);
    }
    await get().refreshSnowflakeConsole(clientRequestId);
  },
  async cancelSnowflakeConsole(clientRequestId) {
    const receipt = get().snowflakeConsoleReceipts[clientRequestId];
    if (!receipt || receipt.notAdmitted) return;
    set((state) => ({
      snowflakeConsoleReceipts: {
        ...state.snowflakeConsoleReceipts,
        [clientRequestId]: { ...receipt, cancellationRequested: true },
      },
    }));
    if (receipt.runId) await client.call("cancel_run", { runId: receipt.runId });
    else await get().refreshSnowflakeConsole(clientRequestId);
  },
  setSession(session) {
    sealConsoleSession(session);
    set((state) => ({
      sessions: { ...state.sessions, [session.connectionId]: session },
      ...(session.consoleMode
        ? { consoleSessions: { ...state.consoleSessions, [session.id]: true } }
        : {}),
      selectedConnectionId: session.connectionId,
    }));
  },
  bindQuerySession(tabId, session, preferredLabel) {
    sealConsoleSession(session);
    const current = get().tabs.find((t) => t.id === tabId);
    if (
      !current ||
      current.kind !== "query" ||
      current.connectionId !== session.connectionId ||
      !["postgres", "mysql"].includes(session.engine) ||
      ["queued", "running"].includes(current.status) ||
      (current.querySession && !current.querySessionClosed)
    ) {
      void client
        .call("disconnect", { sessionId: session.id })
        .catch((error) => get().notify(messageOf(error)));
      return false;
    }
    for (const result of current.results) {
      resultCache.release(result.id);
      dirtyStore.clearTab(result.id, [result.id]);
      if (!isConsoleOwnedResult(result))
        void client.call("release_result", { resultId: result.id }).catch(() => {});
    }
    set((state) => {
      const existing = (state.sessionPool[session.connectionId] ?? []).filter(
        (entry) => entry.info.id !== session.id,
      );
      const normalized = preferredLabel ? normalizeSessionLabel(preferredLabel) : "";
      const label = normalized || defaultSecondarySessionLabel(existing);
      return {
        sessionPool: {
          ...state.sessionPool,
          [session.connectionId]: [...existing, { info: session, label, primary: false }],
        },
        tabs: state.tabs.map((t) =>
          t.id === tabId
            ? {
                ...t,
                querySession: session,
                querySessionClosed: false,
                results: [],
                planSource: undefined,
                activeResultId: null,
                dataSessionId: null,
                runId: null,
                error: null,
                status: "idle",
                metrics: {},
                elapsedMs: 0,
                runMetrics: undefined,
                executionSummary: undefined,
                clientStartedAt: undefined,
                clientElapsedMs: undefined,
                revision: current.revision + 1,
              }
            : t,
        ),
      };
    });
    return true;
  },
  attachQuerySession(tabId, sessionId) {
    const state = get();
    const current = state.tabs.find((t) => t.id === tabId);
    if (
      !current ||
      current.kind !== "query" ||
      !current.connectionId ||
      ["queued", "running"].includes(current.status)
    )
      return false;
    let targetInfo: SessionInfo | null = null;
    if (sessionId !== null) {
      const match = (state.sessionPool[current.connectionId] ?? []).find(
        (entry) => entry.info.id === sessionId,
      );
      if (!match) return false;
      if (current.querySession?.id === sessionId && !current.querySessionClosed) return true;
      targetInfo = match.info;
    } else if (!current.querySession && !current.querySessionClosed) {
      return true;
    }
    if (targetInfo) sealConsoleSession(targetInfo);
    for (const result of current.results) {
      resultCache.release(result.id);
      dirtyStore.clearTab(result.id, [result.id]);
      if (!isConsoleOwnedResult(result))
        void client.call("release_result", { resultId: result.id }).catch(() => {});
    }
    get().updateTab(tabId, {
      querySession: targetInfo,
      querySessionClosed: false,
      snowflakeDdlResult: undefined,
      snowflakeDmlResult: undefined,
      results: [],
      planSource: undefined,
      activeResultId: null,
      dataSessionId: null,
      runId: null,
      error: null,
      status: "idle",
      metrics: {},
      elapsedMs: 0,
      runMetrics: undefined,
      executionSummary: undefined,
      clientStartedAt: undefined,
      clientElapsedMs: undefined,
      revision: current.revision + 1,
    });
    return true;
  },
  renameQuerySession(sessionId, label) {
    const normalized = normalizeSessionLabel(label);
    if (!normalized) return;
    set((state) => {
      const nextPool: Record<string, NamedSession[]> = {};
      for (const [connId, entries] of Object.entries(state.sessionPool)) {
        nextPool[connId] = entries.map((entry) =>
          entry.info.id === sessionId ? { ...entry, label: normalized } : entry,
        );
      }
      return { sessionPool: nextPool };
    });
  },
  async disconnectQuerySession(sessionId) {
    try {
      await client.call("disconnect", { sessionId });
    } finally {
      get().retireSession(sessionId);
    }
  },
  updateSessionLatency(sessionId, latencyMs) {
    set((state) => {
      const sessions = { ...state.sessions };
      for (const [connId, info] of Object.entries(sessions)) {
        if (info.id === sessionId) sessions[connId] = { ...info, latencyMs };
      }
      const sessionPool: Record<string, NamedSession[]> = {};
      for (const [connId, entries] of Object.entries(state.sessionPool)) {
        sessionPool[connId] = entries.map((entry) =>
          entry.info.id === sessionId ? { ...entry, info: { ...entry.info, latencyMs } } : entry,
        );
      }
      const tabs = state.tabs.map((t) =>
        t.querySession?.id === sessionId
          ? { ...t, querySession: { ...t.querySession, latencyMs } }
          : t,
      );
      return { sessions, sessionPool, tabs };
    });
  },
  invalidateSessionContextAuthority(sessionId) {
    if (!hasSdkObservation(get(), sessionId)) return;
    contextRequests.delete(sessionId);
    set((state) => {
      const sessionContextAuthorities = { ...state.sessionContextAuthorities };
      const snowflakeContextAuthorities = { ...state.snowflakeContextAuthorities };
      delete sessionContextAuthorities[sessionId];
      delete snowflakeContextAuthorities[sessionId];
      return {
        sessionContextAuthorities,
        snowflakeContextAuthorities,
        sessionContexts: { ...state.sessionContexts, [sessionId]: [] },
      };
    });
  },
  isSessionContextPending(sessionId) {
    return (
      get().sessionTransactionPending[sessionId] === true ||
      get().sessionSnowflakeGridPending[sessionId] === true ||
      Boolean(get().sessionSnowflakeGridTransactions[sessionId]) ||
      (pendingContextRequests.get(sessionId)?.size ?? 0) > 0
    );
  },
  async refreshSessionContexts(sessionId, throwOnError = false) {
    if (isSnowflakeConsoleSession(get(), sessionId)) {
      if (throwOnError) assertOrdinarySession(sessionId);
      return false;
    }
    if (
      get().sessionSnowflakeGridPending[sessionId] ||
      get().sessionSnowflakeGridTransactions[sessionId]
    ) {
      if (throwOnError)
        throw new Error("Finish or roll back the open Snowflake grid transaction first");
      return false;
    }
    if (get().sessionMetadataRefreshRequired[sessionId] && !throwOnError) return false;
    if (get().sessionTransactionPending[sessionId]) {
      if (throwOnError) throw new Error("Wait for the transaction action to finish first");
      return false;
    }
    if (get().sessionContextSwitching[sessionId]) {
      if (throwOnError) throw new Error("Wait for the schema change to finish first");
      return false;
    }
    const sdk = hasSdkObservation(get(), sessionId);
    if (
      sdk &&
      get().snowflakeContextExpanded[sessionId] &&
      !(throwOnError && get().sessionDdlCatalogRefreshRequired[sessionId])
    ) {
      return get().refreshSnowflakeSessionContexts(sessionId, throwOnError);
    }
    get().invalidateSessionContextAuthority(sessionId);
    const request = beginContextRequest(sessionId);
    const previous = get().sessionStates[sessionId];
    try {
      if (sdk && isSessionActive(get(), sessionId)) {
        set((state) => ({
          sessionStates: {
            ...state.sessionStates,
            [sessionId]: unknownSdkRuntime(sessionId, previous?.transactionControlAuthority),
          },
          sessionContexts: { ...state.sessionContexts, [sessionId]: [] },
        }));
      }
      let options: SessionContextOptions | null;
      try {
        options =
          throwOnError && get().sessionDdlCatalogRefreshRequired[sessionId]
            ? await client.call("refresh_snowflake_ddl_metadata", { sessionId })
            : await client.call<SessionContextOptions | null>("session_contexts", {
                sessionId,
                ...(throwOnError && get().sessionMetadataRefreshRequired[sessionId]
                  ? { resumeMetadata: true }
                  : {}),
              });
        if (!options?.state || options.state.sessionId !== sessionId)
          throw new Error("Session observation is unavailable");
      } catch (error) {
        if (contextRequests.get(sessionId) !== request || !isSessionActive(get(), sessionId))
          return false;
        set((state) => ({
          sessionContexts: { ...state.sessionContexts, [sessionId]: [] },
          ...(sdk
            ? {
                sessionStates: {
                  ...state.sessionStates,
                  [sessionId]: unknownSdkRuntime(sessionId, previous?.transactionControlAuthority),
                },
                ...(sessionRuntimeContextChanged(previous, unknownSdkRuntime(sessionId))
                  ? {
                      sessionContextRevision: {
                        ...state.sessionContextRevision,
                        [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
                      },
                    }
                  : {}),
              }
            : {}),
        }));
        if (throwOnError) throw error;
        return false;
      }
      if (contextRequests.get(sessionId) !== request || !isSessionActive(get(), sessionId))
        return false;
      set((state) => {
        if (contextRequests.get(sessionId) !== request || !isSessionActive(state, sessionId))
          return {};
        const runtime = options.state;
        const contextChanged = sessionRuntimeContextChanged(previous, runtime);
        return {
          sessionStates: { ...state.sessionStates, [sessionId]: runtime },
          sessionContexts: { ...state.sessionContexts, [sessionId]: options.available },
          ...(throwOnError
            ? {
                sessionMetadataRefreshRequired: {
                  ...state.sessionMetadataRefreshRequired,
                  [sessionId]: false,
                },
                sessionDdlCatalogRefreshRequired: {
                  ...state.sessionDdlCatalogRefreshRequired,
                  [sessionId]: false,
                },
              }
            : {}),
          ...(sdk && options.revision
            ? {
                sessionContextAuthorities: {
                  ...state.sessionContextAuthorities,
                  [sessionId]: options,
                },
              }
            : {}),
          ...(contextChanged
            ? {
                sessionContextRevision: {
                  ...state.sessionContextRevision,
                  [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
                },
              }
            : {}),
        };
      });
      return contextRequests.get(sessionId) === request && isSessionActive(get(), sessionId);
    } finally {
      finishContextRequest(sessionId, request);
    }
  },
  async refreshSnowflakeSessionContexts(sessionId, throwOnError = false) {
    if (isSnowflakeConsoleSession(get(), sessionId)) {
      if (throwOnError) assertOrdinarySession(sessionId);
      return false;
    }
    if (
      get().sessionSnowflakeGridPending[sessionId] ||
      get().sessionSnowflakeGridTransactions[sessionId]
    ) {
      if (throwOnError)
        throw new Error("Finish or roll back the open Snowflake grid transaction first");
      return false;
    }
    if (get().sessionMetadataRefreshRequired[sessionId] && !throwOnError) return false;
    if (get().sessionTransactionPending[sessionId]) {
      if (throwOnError) throw new Error("Wait for the transaction action to finish first");
      return false;
    }
    if (get().sessionContextSwitching[sessionId]) {
      if (throwOnError) throw new Error("Wait for the schema change to finish first");
      return false;
    }
    get().invalidateSessionContextAuthority(sessionId);
    const request = beginContextRequest(sessionId);
    const previous = get().sessionStates[sessionId];
    try {
      if (isSessionActive(get(), sessionId)) {
        set((state) => ({
          snowflakeContextExpanded: { ...state.snowflakeContextExpanded, [sessionId]: true },
          sessionStates: {
            ...state.sessionStates,
            [sessionId]: unknownSdkRuntime(sessionId, previous?.transactionControlAuthority),
          },
          sessionContexts: { ...state.sessionContexts, [sessionId]: [] },
        }));
      }
      let catalog: SnowflakeContextCatalog;
      try {
        catalog = await client.call("snowflake_session_contexts", {
          sessionId,
          ...(throwOnError && get().sessionMetadataRefreshRequired[sessionId]
            ? { resumeMetadata: true }
            : {}),
        });
        if (!catalog?.state || catalog.state.sessionId !== sessionId || !catalog.revision)
          throw new Error("Session observation is unavailable");
      } catch (error) {
        if (contextRequests.get(sessionId) !== request || !isSessionActive(get(), sessionId))
          return false;
        const { code } = asAppError(error);
        if (code === "connectionClosed" || code === "outcomeUnknown") {
          get().retireSession(sessionId);
        } else {
          set((state) => ({
            sessionContexts: { ...state.sessionContexts, [sessionId]: [] },
            sessionStates: {
              ...state.sessionStates,
              [sessionId]: unknownSdkRuntime(sessionId, previous?.transactionControlAuthority),
            },
            ...(sessionRuntimeContextChanged(previous, unknownSdkRuntime(sessionId))
              ? {
                  sessionContextRevision: {
                    ...state.sessionContextRevision,
                    [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
                  },
                }
              : {}),
          }));
        }
        if (throwOnError) throw error;
        return false;
      }
      if (contextRequests.get(sessionId) !== request || !isSessionActive(get(), sessionId))
        return false;
      set((state) => {
        if (contextRequests.get(sessionId) !== request || !isSessionActive(state, sessionId))
          return {};
        const runtime = catalog.state;
        const contextChanged = sessionRuntimeContextChanged(previous, runtime);
        return {
          sessionStates: { ...state.sessionStates, [sessionId]: runtime },
          sessionContexts: { ...state.sessionContexts, [sessionId]: catalog.availableSchemas },
          snowflakeContextAuthorities: {
            ...state.snowflakeContextAuthorities,
            [sessionId]: catalog,
          },
          ...(runtime.database
            ? {
                sessionContextAuthorities: {
                  ...state.sessionContextAuthorities,
                  [sessionId]: {
                    state: runtime,
                    available: catalog.availableSchemas,
                    revision: catalog.revision,
                  },
                },
              }
            : {}),
          ...(throwOnError
            ? {
                sessionMetadataRefreshRequired: {
                  ...state.sessionMetadataRefreshRequired,
                  [sessionId]: false,
                },
                sessionDdlCatalogRefreshRequired: {
                  ...state.sessionDdlCatalogRefreshRequired,
                  [sessionId]: false,
                },
              }
            : {}),
          ...(contextChanged
            ? {
                sessionContextRevision: {
                  ...state.sessionContextRevision,
                  [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
                },
              }
            : {}),
        };
      });
      return contextRequests.get(sessionId) === request && isSessionActive(get(), sessionId);
    } finally {
      finishContextRequest(sessionId, request);
    }
  },
  async switchSessionContext(sessionId, target) {
    assertOrdinarySession(sessionId);
    if (get().sessionTransactionPending[sessionId])
      throw new Error("Wait for the transaction action to finish first");
    if (
      get().sessionSnowflakeGridPending[sessionId] ||
      get().sessionSnowflakeGridTransactions[sessionId]
    )
      throw new Error("Finish or roll back the open Snowflake grid transaction first");
    const sdk = hasSdkObservation(get(), sessionId);
    const authority = get().sessionContextAuthorities[sessionId];
    const runtime = get().sessionStates[sessionId];
    if (!isSessionActive(get(), sessionId))
      throw new Error("This database session is disconnected");
    if (get().sessionContextSwitching[sessionId])
      throw new Error("Wait for the schema change to finish first");
    if (
      sdk &&
      (!authority?.revision ||
        !authority.state.contextFingerprint ||
        authority.state !== runtime ||
        runtime.transactionState !== "idle" ||
        runtime.transactionObservation?.status !== "observed" ||
        runtime.transactionObservation.transactionId !== null ||
        !authority.available.includes(target))
    )
      throw new Error("Refresh session context before selecting an available schema");
    get().invalidateSessionContextAuthority(sessionId);
    const request = beginContextRequest(sessionId);
    schemaSwitchRequests.set(sessionId, request);
    set((state) => ({
      sessionContextSwitching: { ...state.sessionContextSwitching, [sessionId]: true },
      ...(sdk
        ? {
            sessionStates: { ...state.sessionStates, [sessionId]: unknownSdkRuntime(sessionId) },
            sessionContexts: { ...state.sessionContexts, [sessionId]: [] },
            sessionContextRevision: {
              ...state.sessionContextRevision,
              [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
            },
          }
        : {}),
    }));
    try {
      const options = sdk
        ? await client.call("switch_session_schema", {
            sessionId,
            request: {
              revision: authority!.revision!,
              database: authority!.state.database,
              schema: target,
              contextFingerprint: authority!.state.contextFingerprint!,
            },
          })
        : await client.call("switch_session_context", { sessionId, target });
      if (contextRequests.get(sessionId) !== request || !isSessionActive(get(), sessionId)) return;
      if (options.state.sessionId !== sessionId || (sdk && !options.revision))
        throw new Error("Schema switch observation is unavailable");
      set((state) => {
        if (contextRequests.get(sessionId) !== request || !isSessionActive(state, sessionId))
          return {};
        return {
          sessionStates: { ...state.sessionStates, [sessionId]: options.state },
          sessionContexts: { ...state.sessionContexts, [sessionId]: options.available },
          ...(sdk
            ? {
                sessionContextAuthorities: {
                  ...state.sessionContextAuthorities,
                  [sessionId]: options,
                },
              }
            : {}),
          sessionContextRevision: {
            ...state.sessionContextRevision,
            [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
          },
        };
      });
    } catch (error) {
      if (contextRequests.get(sessionId) !== request || !isSessionActive(get(), sessionId)) return;
      const { code } = asAppError(error);
      if (sdk && (code === "connectionClosed" || code === "outcomeUnknown"))
        get().retireSession(sessionId);
      throw error;
    } finally {
      finishContextRequest(sessionId, request);
      if (schemaSwitchRequests.get(sessionId) === request) {
        schemaSwitchRequests.delete(sessionId);
        set((state) => ({
          sessionContextSwitching: { ...state.sessionContextSwitching, [sessionId]: false },
        }));
      }
    }
  },
  async switchSnowflakeSessionContext(sessionId, target) {
    assertOrdinarySession(sessionId);
    if (get().sessionTransactionPending[sessionId])
      throw new Error("Wait for the transaction action to finish first");
    if (
      get().sessionSnowflakeGridPending[sessionId] ||
      get().sessionSnowflakeGridTransactions[sessionId]
    )
      throw new Error("Finish or roll back the open Snowflake grid transaction first");
    const authority = get().snowflakeContextAuthorities[sessionId];
    const runtime = get().sessionStates[sessionId];
    if (!isSessionActive(get(), sessionId))
      throw new Error("This database session is disconnected");
    if (get().sessionContextSwitching[sessionId])
      throw new Error("Wait for the schema change to finish first");
    const targetAllowed =
      authority !== undefined &&
      (target.kind === "role"
        ? authority.availableRoles.includes(target.role)
        : target.kind === "warehouse"
          ? authority.availableWarehouses.includes(target.warehouse)
          : target.kind === "database"
            ? authority.availableDatabases.includes(target.database)
            : Boolean(target.database) &&
              authority.state.database === target.database &&
              authority.availableSchemas.includes(target.schema));
    if (
      !authority?.revision ||
      !authority.state.contextFingerprint ||
      authority.state !== runtime ||
      runtime.transactionState !== "idle" ||
      runtime.transactionObservation?.status !== "observed" ||
      runtime.transactionObservation.transactionId !== null ||
      !targetAllowed
    ) {
      throw new Error("Refresh session context before selecting an available Snowflake context");
    }
    get().invalidateSessionContextAuthority(sessionId);
    const request = beginContextRequest(sessionId);
    schemaSwitchRequests.set(sessionId, request);
    set((state) => ({
      snowflakeContextExpanded: { ...state.snowflakeContextExpanded, [sessionId]: true },
      sessionContextSwitching: { ...state.sessionContextSwitching, [sessionId]: true },
      sessionStates: { ...state.sessionStates, [sessionId]: unknownSdkRuntime(sessionId) },
      sessionContexts: { ...state.sessionContexts, [sessionId]: [] },
      sessionContextRevision: {
        ...state.sessionContextRevision,
        [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
      },
    }));
    try {
      const catalog = await client.call("switch_snowflake_context", {
        sessionId,
        request: {
          revision: authority.revision,
          contextFingerprint: authority.state.contextFingerprint,
          target,
        },
      });
      if (contextRequests.get(sessionId) !== request || !isSessionActive(get(), sessionId)) return;
      if (catalog.state.sessionId !== sessionId || !catalog.revision)
        throw new Error("Snowflake context switch observation is unavailable");
      set((state) => {
        if (contextRequests.get(sessionId) !== request || !isSessionActive(state, sessionId))
          return {};
        return {
          sessionStates: { ...state.sessionStates, [sessionId]: catalog.state },
          sessionContexts: { ...state.sessionContexts, [sessionId]: catalog.availableSchemas },
          snowflakeContextAuthorities: {
            ...state.snowflakeContextAuthorities,
            [sessionId]: catalog,
          },
          ...(catalog.state.database
            ? {
                sessionContextAuthorities: {
                  ...state.sessionContextAuthorities,
                  [sessionId]: {
                    state: catalog.state,
                    available: catalog.availableSchemas,
                    revision: catalog.revision,
                  },
                },
              }
            : {}),
          sessionContextRevision: {
            ...state.sessionContextRevision,
            [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
          },
        };
      });
    } catch (error) {
      if (contextRequests.get(sessionId) !== request || !isSessionActive(get(), sessionId)) return;
      const { code } = asAppError(error);
      if (code === "connectionClosed" || code === "outcomeUnknown") get().retireSession(sessionId);
      throw error;
    } finally {
      finishContextRequest(sessionId, request);
      if (schemaSwitchRequests.get(sessionId) === request) {
        schemaSwitchRequests.delete(sessionId);
        set((state) => ({
          sessionContextSwitching: { ...state.sessionContextSwitching, [sessionId]: false },
        }));
      }
    }
  },
  async executeTransactionControl(plan, approval) {
    const sessionId = plan.sessionId;
    if (!isSessionActive(get(), sessionId))
      throw new Error("This database session is disconnected");
    if (get().sessionTransactionPending[sessionId])
      throw new Error("Wait for the transaction action to finish first");
    if (
      get().tabs.some(
        (tab) =>
          querySessionForTab(tab, get().sessions)?.id === sessionId &&
          (tab.status === "queued" || tab.status === "running"),
      )
    )
      throw new Error("Wait for this session's queries to finish first");
    revokeSchemaSelectionBeforeWork(sessionId, true);
    const request = beginContextRequest(sessionId);
    set((state) => ({
      sessionTransactionPending: { ...state.sessionTransactionPending, [sessionId]: true },
      sessionMetadataRefreshRequired: {
        ...state.sessionMetadataRefreshRequired,
        [sessionId]: true,
      },
      // Earlier query events carry the old tab revision. Keep their delayed
      // runtime observations from replacing this boundary's final authority.
      tabs: state.tabs.map((tab) =>
        querySessionForTab(tab, state.sessions)?.id === sessionId
          ? { ...tab, revision: tab.revision + 1 }
          : tab,
      ),
      sessionStates: { ...state.sessionStates, [sessionId]: unknownSdkRuntime(sessionId) },
      sessionContextRevision: {
        ...state.sessionContextRevision,
        [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
      },
    }));
    try {
      const result = await client.call("execute_transaction_control", {
        planId: plan.id,
        approval,
      });
      if (result.state.sessionId !== sessionId) {
        get().retireSession(sessionId);
        throw new Error("Invalid transaction action response");
      }
      if (result.retired || result.boundaryOutcome === "unknown") get().retireSession(sessionId);
      else if (contextRequests.get(sessionId) === request && isSessionActive(get(), sessionId)) {
        set((state) => ({ sessionStates: { ...state.sessionStates, [sessionId]: result.state } }));
      }
      // Keep command evidence truthful even when the owning tab has closed.
      const label =
        result.action === "begin" ? "BEGIN" : result.action === "commit" ? "COMMIT" : "ROLLBACK";
      const outcome =
        result.boundaryOutcome === "acknowledged"
          ? "acknowledged"
          : result.boundaryOutcome === "rejected"
            ? "rejected"
            : "outcome unknown";
      get().notify(
        `${label} ${outcome}${result.retired ? "; session retired" : result.state.transactionControlAuthority?.status === "rollbackOnly" ? "; only rollback recovery is available" : ""}${result.error ? `. ${result.error.message}` : ""}`,
      );
      return result;
    } catch (error) {
      const { code } = asAppError(error);
      if (code === "connectionClosed" || code === "outcomeUnknown") get().retireSession(sessionId);
      // Do not issue an automatic observer or replay a boundary. An owner can
      // explicitly refresh after an admission failure on a still-live session.
      throw error;
    } finally {
      finishContextRequest(sessionId, request);
      set((state) => {
        const pending = { ...state.sessionTransactionPending };
        delete pending[sessionId];
        return { sessionTransactionPending: pending };
      });
    }
  },
  async executeSnowflakeDml(plan, approval, ownerTabId) {
    const sessionId = plan.sessionId;
    if (!isSessionActive(get(), sessionId))
      throw new Error("This database session is disconnected");
    if (get().sessionTransactionPending[sessionId])
      throw new Error("Wait for the reviewed write or transaction action to finish first");
    if (
      get().tabs.some(
        (tab) =>
          querySessionForTab(tab, get().sessions)?.id === sessionId &&
          (tab.status === "queued" || tab.status === "running"),
      )
    )
      throw new Error("Wait for this session's queries to finish first");
    if (get().sessionStates[sessionId]?.transactionControlAuthority?.status === "rollbackOnly")
      throw new Error("Only rollback recovery is available for this session");
    const ownerTab = get().tabs.find((tab) => tab.id === ownerTabId);
    if (ownerTabId && (!ownerTab || querySessionForTab(ownerTab, get().sessions)?.id !== sessionId))
      throw new Error("The owning query session changed; prepare this write again");
    revokeSchemaSelectionBeforeWork(sessionId, true);
    const request = beginContextRequest(sessionId);
    set((state) => ({
      sessionTransactionPending: { ...state.sessionTransactionPending, [sessionId]: true },
      sessionMetadataRefreshRequired: {
        ...state.sessionMetadataRefreshRequired,
        [sessionId]: true,
      },
      // Earlier query events must not replace this write's final authority.
      tabs: state.tabs.map((tab) =>
        querySessionForTab(tab, state.sessions)?.id === sessionId
          ? {
              ...tab,
              revision: tab.revision + 1,
              ...(tab.id === ownerTabId
                ? {
                    snowflakeDmlResult: undefined,
                    snowflakeDdlResult: undefined,
                    status: "running" as const,
                    error: null,
                    runId: null,
                    dataSessionId: sessionId,
                    executionSummary: undefined,
                    clientStartedAt: performance.now(),
                    clientElapsedMs: undefined,
                  }
                : {}),
            }
          : tab,
      ),
      sessionStates: { ...state.sessionStates, [sessionId]: unknownSdkRuntime(sessionId) },
      sessionContextRevision: {
        ...state.sessionContextRevision,
        [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
      },
    }));
    const ownerRevision = get().tabs.find((tab) => tab.id === ownerTabId)?.revision;
    try {
      const result = await client.call("execute_snowflake_dml", {
        planId: plan.id,
        approval,
      });
      if (result.state.sessionId !== sessionId) {
        get().retireSession(sessionId);
        throw new Error("Invalid Snowflake DML response");
      }
      if (result.retired || result.statementOutcome === "unknown") {
        result.retired = true;
        get().retireSession(sessionId);
      } else if (contextRequests.get(sessionId) === request && isSessionActive(get(), sessionId)) {
        set((state) => ({ sessionStates: { ...state.sessionStates, [sessionId]: result.state } }));
      }
      // Preserve the statement evidence even when the owning tab has closed.
      set((state) => ({
        tabs: state.tabs.map((tab) =>
          tab.id === ownerTabId && tab.revision === ownerRevision && tab.dataSessionId === sessionId
            ? {
                ...tab,
                snowflakeDmlResult: result,
                status:
                  result.statementOutcome === "succeeded"
                    ? "completed"
                    : result.statementOutcome === "unknown"
                      ? "outcomeUnknown"
                      : "failed",
                error: result.error,
                clientElapsedMs: performance.now() - (tab.clientStartedAt ?? performance.now()),
              }
            : tab,
        ),
      }));
      const count =
        result.summary.status === "validated" && result.summary.affectedRows !== null
          ? `; affected rows ${result.summary.affectedRows}`
          : "; affected-row count unavailable";
      get().notify(
        `${result.kind.toUpperCase()} ${result.statementOutcome === "unknown" ? "outcome unknown" : result.statementOutcome}${count}${result.retired ? "; session retired" : result.state.transactionControlAuthority?.status === "rollbackOnly" ? "; only rollback recovery is available" : ""}${result.error ? `. ${result.error.message}` : ""}`,
      );
      return result;
    } catch (error) {
      const { code } = asAppError(error);
      if (code === "connectionClosed" || code === "outcomeUnknown") get().retireSession(sessionId);
      set((state) => ({
        tabs: state.tabs.map((tab) =>
          tab.id === ownerTabId && tab.revision === ownerRevision && tab.dataSessionId === sessionId
            ? {
                ...tab,
                status: "failed",
                error: asAppError(error),
                clientElapsedMs: performance.now() - (tab.clientStartedAt ?? performance.now()),
              }
            : tab,
        ),
      }));
      // No automatic observer or replay after a write admission failure.
      throw error;
    } finally {
      finishContextRequest(sessionId, request);
      set((state) => {
        const pending = { ...state.sessionTransactionPending };
        delete pending[sessionId];
        return { sessionTransactionPending: pending };
      });
    }
  },
  async executeSnowflakeDdl(plan, approval, ownerTabId) {
    const sessionId = plan.sessionId;
    if (!isSessionActive(get(), sessionId))
      throw new Error("This database session is disconnected");
    if (get().sessionTransactionPending[sessionId])
      throw new Error("Wait for the reviewed write or transaction action to finish first");
    if (
      get().tabs.some(
        (tab) =>
          querySessionForTab(tab, get().sessions)?.id === sessionId &&
          (tab.status === "queued" || tab.status === "running"),
      )
    )
      throw new Error("Wait for this session's queries to finish first");
    if (get().sessionStates[sessionId]?.transactionControlAuthority?.status === "rollbackOnly")
      throw new Error("Only rollback recovery is available for this session");
    const ownerTab = get().tabs.find((tab) => tab.id === ownerTabId);
    if (ownerTabId && (!ownerTab || querySessionForTab(ownerTab, get().sessions)?.id !== sessionId))
      throw new Error("The owning query session changed; prepare this write again");
    revokeSchemaSelectionBeforeWork(sessionId, true);
    const request = beginContextRequest(sessionId);
    set((state) => ({
      sessionTransactionPending: { ...state.sessionTransactionPending, [sessionId]: true },
      sessionMetadataRefreshRequired: {
        ...state.sessionMetadataRefreshRequired,
        [sessionId]: true,
      },
      sessionDdlCatalogRefreshRequired: {
        ...state.sessionDdlCatalogRefreshRequired,
        [sessionId]: true,
      },
      // Earlier query events must not replace this DDL statement's final evidence.
      tabs: state.tabs.map((tab) =>
        querySessionForTab(tab, state.sessions)?.id === sessionId
          ? {
              ...tab,
              revision: tab.revision + 1,
              ...(tab.id === ownerTabId
                ? {
                    snowflakeDdlResult: undefined,
                    snowflakeDmlResult: undefined,
                    status: "running" as const,
                    error: null,
                    runId: null,
                    dataSessionId: sessionId,
                    executionSummary: undefined,
                    clientStartedAt: performance.now(),
                    clientElapsedMs: undefined,
                  }
                : {}),
            }
          : tab,
      ),
      sessionStates: { ...state.sessionStates, [sessionId]: unknownSdkRuntime(sessionId) },
      sessionContextRevision: {
        ...state.sessionContextRevision,
        [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
      },
    }));
    const ownerRevision = get().tabs.find((tab) => tab.id === ownerTabId)?.revision;
    try {
      const result = await client.call("execute_snowflake_ddl", {
        planId: plan.id,
        approval,
      });
      if (result.state.sessionId !== sessionId) {
        get().retireSession(sessionId);
        throw new Error("Invalid Snowflake DDL response");
      }
      if (
        result.retired ||
        result.statementOutcome === "unknown" ||
        !isSessionActive(get(), sessionId)
      ) {
        result.retired = true;
        get().retireSession(sessionId);
      } else if (contextRequests.get(sessionId) === request && isSessionActive(get(), sessionId)) {
        set((state) => ({ sessionStates: { ...state.sessionStates, [sessionId]: result.state } }));
      }
      // Preserve the statement evidence even when the owning tab has closed.
      set((state) => ({
        tabs: state.tabs.map((tab) =>
          tab.id === ownerTabId && tab.revision === ownerRevision && tab.dataSessionId === sessionId
            ? {
                ...tab,
                snowflakeDdlResult: result,
                status:
                  result.statementOutcome === "succeeded"
                    ? "completed"
                    : result.statementOutcome === "unknown"
                      ? "outcomeUnknown"
                      : "failed",
                error: result.error,
                clientElapsedMs: performance.now() - (tab.clientStartedAt ?? performance.now()),
              }
            : tab,
        ),
      }));
      get().notify(
        `${result.qualifiedTarget}: ${result.statementOutcome === "unknown" ? "outcome unknown" : result.statementOutcome}. Schema changes cannot be rolled back; explicitly refresh metadata before further work.${result.retired ? " Session retired." : ""}${result.error ? ` ${result.error.message}` : ""}`,
      );
      return result;
    } catch (error) {
      const { code } = asAppError(error);
      if (code === "connectionClosed" || code === "outcomeUnknown") get().retireSession(sessionId);
      set((state) => ({
        tabs: state.tabs.map((tab) =>
          tab.id === ownerTabId && tab.revision === ownerRevision && tab.dataSessionId === sessionId
            ? {
                ...tab,
                status: code === "outcomeUnknown" ? "outcomeUnknown" : "failed",
                error: asAppError(error),
                clientElapsedMs: performance.now() - (tab.clientStartedAt ?? performance.now()),
              }
            : tab,
        ),
      }));
      // No automatic observer or replay after a write admission failure.
      throw error;
    } finally {
      finishContextRequest(sessionId, request);
      set((state) => {
        const pending = { ...state.sessionTransactionPending };
        delete pending[sessionId];
        return { sessionTransactionPending: pending };
      });
    }
  },
  async executeSnowflakeGridPlan(plan, approval) {
    const sessionId = plan.sessionId;
    if (!isSessionActive(get(), sessionId))
      throw new Error("This database session is disconnected");
    if (get().sessionTransactionPending[sessionId] || get().sessionSnowflakeGridPending[sessionId])
      throw new Error("Wait for the reviewed write or transaction action to finish first");
    if (
      get().tabs.some(
        (tab) =>
          querySessionForTab(tab, get().sessions)?.id === sessionId &&
          (tab.status === "queued" || tab.status === "running"),
      )
    )
      throw new Error("Wait for this session's queries to finish first");
    if (get().sessionStates[sessionId]?.transactionControlAuthority?.status === "rollbackOnly")
      throw new Error("Only rollback recovery is available for this session");
    const hadOpenGridTx = Boolean(get().sessionSnowflakeGridTransactions[sessionId]);
    revokeSchemaSelectionBeforeWork(sessionId, false, true);
    set((state) => ({
      sessionSnowflakeGridPending: { ...state.sessionSnowflakeGridPending, [sessionId]: true },
    }));
    try {
      const result = await client.call("execute_snowflake_grid_plan", {
        planId: plan.id,
        approval,
      });
      if (result.sessionId !== sessionId) {
        get().retireSession(sessionId);
        throw new Error("Invalid Snowflake grid execution response");
      }
      if (result.retired || result.outcome === "unknown" || !isSessionActive(get(), sessionId)) {
        result.retired = true;
        get().retireSession(sessionId);
      } else if (
        (result.outcome === "pending" || (hadOpenGridTx && result.outcome === "rejected")) &&
        result.transactionId
      ) {
        set((state) => ({
          sessionSnowflakeGridTransactions: {
            ...state.sessionSnowflakeGridTransactions,
            [sessionId]: {
              transactionId: result.transactionId!,
              recordingError: result.recordingError,
            },
          },
        }));
      } else {
        set((state) => {
          const nextGridTx = { ...state.sessionSnowflakeGridTransactions };
          delete nextGridTx[sessionId];
          return {
            sessionSnowflakeGridTransactions: nextGridTx,
            sessionMetadataRefreshRequired: {
              ...state.sessionMetadataRefreshRequired,
              [sessionId]: true,
            },
            sessionStates: {
              ...state.sessionStates,
              [sessionId]: unknownSdkRuntime(sessionId),
            },
            sessionContextRevision: {
              ...state.sessionContextRevision,
              [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
            },
          };
        });
      }
      const outcomeLabel =
        result.outcome === "committed"
          ? "committed"
          : result.outcome === "pending"
            ? `manual transaction pending (${result.transactionId ?? "active"})`
            : result.outcome === "rolledBack"
              ? "rolled back"
              : result.outcome === "rejected"
                ? "rejected"
                : "outcome unknown";
      get().notify(
        `Snowflake grid edits ${outcomeLabel}${result.retired ? "; session retired" : ""}${result.recordingError ? `. Recording blocked: ${result.recordingError}` : ""}`,
      );
      return result;
    } catch (error) {
      const { code } = asAppError(error);
      if (code === "connectionClosed" || code === "outcomeUnknown") get().retireSession(sessionId);
      throw error;
    } finally {
      set((state) => {
        const pending = { ...state.sessionSnowflakeGridPending };
        delete pending[sessionId];
        return { sessionSnowflakeGridPending: pending };
      });
    }
  },
  async finishSnowflakeGridTransaction(sessionId, transactionId, action) {
    if (get().sessionTransactionPending[sessionId] || get().sessionSnowflakeGridPending[sessionId])
      throw new Error("Wait for the reviewed write or transaction action to finish first");
    set((state) => ({
      sessionSnowflakeGridPending: { ...state.sessionSnowflakeGridPending, [sessionId]: true },
    }));
    try {
      const result = await client.call("finish_snowflake_grid_transaction", {
        transactionId,
        action,
      });
      if (result.retired || result.outcome === "unknown") {
        result.retired = true;
        get().retireSession(sessionId);
      } else if (
        (result.outcome === "pending" || result.outcome === "rejected") &&
        result.transactionId
      ) {
        set((state) => ({
          sessionSnowflakeGridTransactions: {
            ...state.sessionSnowflakeGridTransactions,
            [sessionId]: {
              transactionId: result.transactionId!,
              recordingError: result.recordingError,
            },
          },
        }));
      } else {
        set((state) => {
          const nextGridTx = { ...state.sessionSnowflakeGridTransactions };
          delete nextGridTx[sessionId];
          return {
            sessionSnowflakeGridTransactions: nextGridTx,
            ...(isSessionActive(state, sessionId)
              ? {
                  sessionMetadataRefreshRequired: {
                    ...state.sessionMetadataRefreshRequired,
                    [sessionId]: true,
                  },
                  sessionStates: {
                    ...state.sessionStates,
                    [sessionId]: unknownSdkRuntime(sessionId),
                  },
                  sessionContextRevision: {
                    ...state.sessionContextRevision,
                    [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
                  },
                }
              : {}),
          };
        });
      }
      const label = action === "commit" ? "COMMIT" : "ROLLBACK";
      const outcomeLabel =
        result.outcome === "committed"
          ? "committed"
          : result.outcome === "rolledBack"
            ? "rolled back"
            : result.outcome === "rejected"
              ? "rejected"
              : "outcome unknown";
      get().notify(
        `Snowflake grid ${label} ${outcomeLabel}${result.retired ? "; session retired" : ""}${result.recordingError ? `. Recording warning: ${result.recordingError}` : ""}`,
      );
      return result;
    } catch (error) {
      const { code } = asAppError(error);
      if (code === "connectionClosed" || code === "outcomeUnknown") get().retireSession(sessionId);
      throw error;
    } finally {
      set((state) => {
        const pending = { ...state.sessionSnowflakeGridPending };
        delete pending[sessionId];
        return { sessionSnowflakeGridPending: pending };
      });
    }
  },
  async setTransactionMode(sessionId, mode) {
    assertOrdinarySession(sessionId);
    const runtime = await client.call<SessionRuntimeState>("set_transaction_mode", {
      sessionId,
      mode,
    });
    set((state) => {
      if (!isSessionActive(state, sessionId)) return {};
      const prev = state.sessionStates[sessionId];
      const contextChanged = sessionRuntimeContextChanged(prev, runtime);
      return {
        sessionStates: { ...state.sessionStates, [sessionId]: runtime },
        ...(contextChanged
          ? {
              sessionContextRevision: {
                ...state.sessionContextRevision,
                [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
              },
            }
          : {}),
      };
    });
  },
  async commitTransaction(sessionId) {
    assertOrdinarySession(sessionId);
    let runtime: SessionRuntimeState;
    try {
      runtime = await client.call<SessionRuntimeState>("commit_transaction", { sessionId });
    } catch (error) {
      const { code } = asAppError(error);
      if (code === "connectionClosed" || code === "outcomeUnknown") get().retireSession(sessionId);
      else if ((code === "database" || code === "conflict") && isSessionActive(get(), sessionId)) {
        try {
          await get().refreshSessionContexts(sessionId, true);
        } catch (refreshError) {
          const { code: refreshCode } = asAppError(refreshError);
          if (refreshCode === "connectionClosed" || refreshCode === "outcomeUnknown")
            get().retireSession(sessionId);
        }
      }
      throw error;
    }
    if (!isSessionActive(get(), sessionId)) return;
    set((state) => {
      if (!isSessionActive(state, sessionId)) return {};
      return {
        sessionStates: { ...state.sessionStates, [sessionId]: runtime },
        sessionContextRevision: {
          ...state.sessionContextRevision,
          [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
        },
      };
    });
    get().notify("Transaction committed");
  },
  async rollbackTransaction(sessionId) {
    assertOrdinarySession(sessionId);
    let runtime: SessionRuntimeState;
    try {
      runtime = await client.call<SessionRuntimeState>("rollback_transaction", { sessionId });
    } catch (error) {
      const { code } = asAppError(error);
      if (code === "connectionClosed" || code === "outcomeUnknown") get().retireSession(sessionId);
      else if ((code === "database" || code === "conflict") && isSessionActive(get(), sessionId)) {
        try {
          await get().refreshSessionContexts(sessionId, true);
        } catch (refreshError) {
          const { code: refreshCode } = asAppError(refreshError);
          if (refreshCode === "connectionClosed" || refreshCode === "outcomeUnknown")
            get().retireSession(sessionId);
        }
      }
      throw error;
    }
    if (!isSessionActive(get(), sessionId)) return;
    set((state) => {
      if (!isSessionActive(state, sessionId)) return {};
      return {
        sessionStates: { ...state.sessionStates, [sessionId]: runtime },
        sessionContextRevision: {
          ...state.sessionContextRevision,
          [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
        },
      };
    });
    get().notify("Transaction rolled back");
  },
  retireSession(sessionId) {
    planSnapshots.removeSession(sessionId);
    set((state) => {
      const retiredConnections = Object.entries(state.sessions)
        .filter(([, s]) => s.id === sessionId)
        .map(([connId]) => connId);
      const objects = { ...state.objects };
      for (const connId of retiredConnections) delete objects[connId];
      const sessionPool: Record<string, NamedSession[]> = {};
      for (const [connId, entries] of Object.entries(state.sessionPool)) {
        const remaining = entries.filter((entry) => entry.info.id !== sessionId);
        if (remaining.length > 0) sessionPool[connId] = remaining;
      }
      return {
        sessions: Object.fromEntries(
          Object.entries(state.sessions).filter(([, s]) => s.id !== sessionId),
        ),
        sessionPool,
        ...purgeSessionRuntimeMaps(state, [sessionId]),
        objects,
        tabs: state.tabs.map((t) => ({
          ...t,
          ...(t.querySession?.id === sessionId ? { querySessionClosed: true } : {}),
          ...(t.planSource?.sessionId === sessionId ? { planSource: undefined } : {}),
        })),
      };
    });
  },
  updateProfile(profile) {
    set((state) => ({
      bootstrap: state.bootstrap
        ? {
            ...state.bootstrap,
            profiles: [...state.bootstrap.profiles.filter((p) => p.id !== profile.id), profile],
          }
        : null,
    }));
  },
  selectConnection(id) {
    set({ selectedConnectionId: id });
  },
  async refreshCatalog(id, explicit = false) {
    const session = get().sessions[id];
    if (!session) return;
    if (isSnowflakeConsoleSession(get(), session.id)) {
      const { catalog, objects } = get();
      if (catalog[id] !== undefined || objects[id] !== undefined) {
        const nextCatalog = { ...catalog };
        const nextObjects = { ...objects };
        delete nextCatalog[id];
        delete nextObjects[id];
        set({ catalog: nextCatalog, objects: nextObjects });
      }
      return;
    }
    if (!session.capabilities.tableBrowsing) {
      if (explicit && session.capabilities.objectExplorer) {
        assertOrdinarySession(session.id);
        const objects = await client
          .call<DatabaseObjectSummary[]>("database_objects", { sessionId: session.id })
          .catch((error) => {
            if ((error as Partial<AppError> | null)?.code === "unsupported") return [];
            throw error;
          });
        assertOrdinarySession(session.id);
        if (get().sessions[id]?.id === session.id)
          set((state) => ({
            catalog: { ...state.catalog, [id]: [] },
            objects: { ...state.objects, [id]: objects },
          }));
        return;
      }
      const { catalog, objects } = get();
      if (catalog[id] !== undefined || objects[id] !== undefined) {
        const nextCatalog = { ...catalog };
        const nextObjects = { ...objects };
        delete nextCatalog[id];
        delete nextObjects[id];
        set({ catalog: nextCatalog, objects: nextObjects });
      }
      return;
    }
    const catalog = await client.call<TableInfo[]>("catalog", { sessionId: session.id });
    assertOrdinarySession(session.id);
    const objects = session.capabilities.objectExplorer
      ? await client
          .call<DatabaseObjectSummary[]>("database_objects", { sessionId: session.id })
          .catch((error) => {
            if ((error as Partial<AppError> | null)?.code === "unsupported") return [];
            throw error;
          })
      : [];
    if (get().sessions[id]?.id === session.id)
      set((state) => ({
        catalog: { ...state.catalog, [id]: catalog },
        objects: { ...state.objects, [id]: objects },
      }));
  },
  openQuery(connectionId = get().selectedConnectionId, sql) {
    const count = get().tabs.filter((t) => t.kind === "query").length + 1;
    const next = tab("query", `Query ${count}`, connectionId);
    const engine = get().bootstrap?.profiles.find((profile) => profile.id === connectionId)?.engine;
    next.sql =
      sql ??
      (engine === "redis"
        ? "PING"
        : engine === "snowflake"
          ? "-- A little space to think.\nSELECT CURRENT_DATABASE(), CURRENT_USER(), CURRENT_VERSION();"
          : engine === "mysql"
            ? "-- A little space to think.\nSELECT DATABASE(), CURRENT_USER(), VERSION();"
            : engine === "sqlite"
              ? "-- A little space to think.\nSELECT sqlite_version(), datetime('now');"
              : "-- A little space to think.\nSELECT current_database(), current_user, version();");
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
    return next.id;
  },
  recordRecentLocation(entry) {
    const nextEntry: RecentLocation = { ...entry, visitedAtMs: Date.now() };
    set((state) => ({
      recentLocations: [
        nextEntry,
        ...state.recentLocations.filter((item) => item.key !== nextEntry.key),
      ].slice(0, MAX_RECENT_LOCATIONS),
    }));
  },
  async openSqlFile(connectionId = get().selectedConnectionId) {
    const opened = await client.call<OpenedSqlFile | null>("open_sql_file");
    if (!opened) return null;
    const next = tab("query", opened.fileName, connectionId);
    next.sql = opened.content;
    next.fileToken = opened.token;
    next.fileName = opened.fileName;
    next.savedSql = opened.content;
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
    get().recordRecentLocation({
      key: `tab:${next.id}`,
      label: opened.fileName,
      subtitle: "SQL file",
      target: { kind: "tab", tabId: next.id },
    });
    get().notify(`Opened ${opened.fileName}`);
    return next.id;
  },
  async saveSqlFile(tabId, options) {
    const current = get().tabs.find((t) => t.id === tabId);
    if (!current || current.kind !== "query") return false;
    const previousToken = current.fileToken;
    const contentToSave = current.sql;
    const rawSuggested = current.fileName ?? `${current.title.trim() || "query"}.sql`;
    const suggestedName = /\.sql$/i.test(rawSuggested) ? rawSuggested : `${rawSuggested}.sql`;
    let saved: SavedSqlFile | null;
    try {
      saved = await client.call<SavedSqlFile | null>("save_sql_file", {
        token: options?.saveAs ? null : previousToken,
        suggestedName,
        content: contentToSave,
      });
    } catch (error) {
      if (
        !options?.saveAs &&
        previousToken &&
        (error as Partial<AppError> | null)?.code === "approvalExpired"
      ) {
        saved = await client.call<SavedSqlFile | null>("save_sql_file", {
          token: null,
          suggestedName,
          content: contentToSave,
        });
      } else {
        throw error;
      }
    }
    if (!saved) return false;
    if (previousToken && previousToken !== saved.token) {
      void client.call("release_sql_file_grant", { token: previousToken }).catch(() => {});
    }
    get().updateTab(tabId, {
      fileToken: saved.token,
      fileName: saved.fileName,
      title: saved.fileName,
      savedSql: contentToSave,
    });
    get().recordRecentLocation({
      key: `tab:${tabId}`,
      label: saved.fileName,
      subtitle: "SQL file",
      target: { kind: "tab", tabId },
    });
    if (contentToSave.trim() && current.connectionId) {
      const boundProfile = get().bootstrap?.profiles.find((p) => p.id === current.connectionId);
      if (boundProfile && !boundProfile.snowflake?.reviewedConsole && boundProfile.storeDrafts) {
        void Promise.resolve(
          client.call("record_local_history", {
            input: {
              tabKey: `file:${saved.fileName}`.slice(0, 160),
              title: saved.fileName,
              connectionId: current.connectionId,
              trigger: "saved",
              sql: contentToSave,
            },
          }),
        ).catch(() => {});
      }
    }
    get().notify(`Saved ${saved.fileName}`);
    return true;
  },
  async saveQueryToLibrary(input, attachTabId) {
    const saved = await client.call<SavedQuery>("save_saved_query", { input });
    set((state) => {
      const savedQueries = [
        saved,
        ...state.savedQueries.filter((entry) => entry.id !== saved.id),
      ].sort((a, b) => b.updatedAtMs - a.updatedAtMs);
      const tabs = attachTabId
        ? state.tabs.map((t) =>
            t.id === attachTabId
              ? {
                  ...t,
                  savedQueryId: saved.id,
                  ...(t.fileName ? {} : { title: saved.title }),
                }
              : t,
          )
        : state.tabs;
      return {
        savedQueries,
        tabs,
        bootstrap: state.bootstrap ? { ...state.bootstrap, savedQueries } : null,
      };
    });
    const connName = saved.connectionId
      ? (get().bootstrap?.profiles.find((p) => p.id === saved.connectionId)?.name ?? null)
      : null;
    get().recordRecentLocation({
      key: `saved:${saved.id}`,
      label: saved.title,
      subtitle: `${connName ?? "Any connection"} · Saved query`,
      target: { kind: "savedQuery", savedQueryId: saved.id },
    });
    get().notify(`Saved "${saved.title}" to library`);
    return saved;
  },
  async deleteSavedQuery(id) {
    await client.call("delete_saved_query", { id });
    set((state) => {
      const savedQueries = state.savedQueries.filter((entry) => entry.id !== id);
      const bookmarks = state.bookmarks.filter(
        (entry) => !(entry.target.kind === "savedQuery" && entry.target.savedQueryId === id),
      );
      return {
        savedQueries,
        bookmarks,
        recentLocations: state.recentLocations.filter((entry) => entry.key !== `saved:${id}`),
        tabs: state.tabs.map((t) => (t.savedQueryId === id ? { ...t, savedQueryId: null } : t)),
        bootstrap: state.bootstrap ? { ...state.bootstrap, savedQueries, bookmarks } : null,
      };
    });
    get().notify("Deleted saved query");
  },
  openSavedQuery(id) {
    const state = get();
    const entry = state.savedQueries.find((item) => item.id === id);
    if (!entry) {
      state.notify("This saved query is no longer in the library.");
      return null;
    }
    const connName = entry.connectionId
      ? (state.bootstrap?.profiles.find((p) => p.id === entry.connectionId)?.name ?? null)
      : null;
    const existing = state.tabs.find(
      (t) => t.kind === "query" && t.savedQueryId === entry.id && t.fileName === null,
    );
    if (existing) {
      set({ activeTabId: existing.id });
      get().recordRecentLocation({
        key: `saved:${entry.id}`,
        label: entry.title,
        subtitle: `${connName ?? "Any connection"} · Saved query`,
        target: { kind: "savedQuery", savedQueryId: entry.id },
      });
      return existing.id;
    }
    const targetConnectionId =
      entry.connectionId && state.bootstrap?.profiles.some((p) => p.id === entry.connectionId)
        ? entry.connectionId
        : state.selectedConnectionId;
    const next = tab("query", entry.title, targetConnectionId);
    next.sql = entry.sql;
    next.savedQueryId = entry.id;
    set((prev) => ({ tabs: [...prev.tabs, next], activeTabId: next.id }));
    get().recordRecentLocation({
      key: `saved:${entry.id}`,
      label: entry.title,
      subtitle: `${connName ?? "Any connection"} · Saved query`,
      target: { kind: "savedQuery", savedQueryId: entry.id },
    });
    return next.id;
  },
  async toggleBookmark(target, label) {
    const existing = findBookmarkForTarget(get().bookmarks, target);
    if (existing) {
      await get().deleteBookmark(existing.id);
      get().notify(`Removed bookmark for ${existing.label}`);
      return false;
    }
    const created = await client.call<SavedBookmark>("save_bookmark", { label, target });
    set((state) => {
      const bookmarks = [
        created,
        ...state.bookmarks.filter((entry) => entry.id !== created.id),
      ].sort((a, b) => b.createdAtMs - a.createdAtMs);
      return {
        bookmarks,
        bootstrap: state.bootstrap ? { ...state.bootstrap, bookmarks } : null,
      };
    });
    get().notify(`Bookmarked ${created.label}`);
    return true;
  },
  async deleteBookmark(id) {
    await client.call("delete_bookmark", { id });
    set((state) => {
      const bookmarks = state.bookmarks.filter((entry) => entry.id !== id);
      return {
        bookmarks,
        bootstrap: state.bootstrap ? { ...state.bootstrap, bookmarks } : null,
      };
    });
  },
  async saveSqlFormatSettings(settings) {
    const sequence = ++sqlFormatSaveSequence;
    if (sqlFormatPending === 0) sqlFormatPersisted = get().sqlFormatSettings;
    sqlFormatPending++;
    set((state) => ({
      sqlFormatSettings: settings,
      bootstrap: state.bootstrap ? { ...state.bootstrap, sqlFormatSettings: settings } : null,
    }));
    const operation = sqlFormatSaveTail.then(async () => {
      try {
        const saved = await client.call<SqlFormatSettings>("save_sql_format_settings", {
          settings,
        });
        sqlFormatPersisted = saved;
        if (sequence === sqlFormatSaveSequence) {
          set((state) => ({
            sqlFormatSettings: saved,
            bootstrap: state.bootstrap ? { ...state.bootstrap, sqlFormatSettings: saved } : null,
          }));
        }
        return saved;
      } catch (error) {
        if (sequence === sqlFormatSaveSequence) {
          set((state) => ({
            sqlFormatSettings: sqlFormatPersisted,
            bootstrap: state.bootstrap
              ? { ...state.bootstrap, sqlFormatSettings: sqlFormatPersisted }
              : null,
          }));
        }
        throw error;
      } finally {
        sqlFormatPending--;
      }
    });
    sqlFormatSaveTail = operation.then(
      () => {},
      () => {},
    );
    return operation;
  },
  async saveSqlSnippet(input) {
    const saved = await client.call<SqlSnippet>("save_sql_snippet", { input });
    set((state) => {
      const sqlSnippets = [
        saved,
        ...state.sqlSnippets.filter((entry) => entry.id !== saved.id),
      ].sort((a, b) => a.trigger.toLowerCase().localeCompare(b.trigger.toLowerCase()));
      return {
        sqlSnippets,
        bootstrap: state.bootstrap ? { ...state.bootstrap, sqlSnippets } : null,
      };
    });
    return saved;
  },
  async deleteSqlSnippet(id) {
    await client.call("delete_sql_snippet", { id });
    set((state) => {
      const sqlSnippets = state.sqlSnippets.filter((entry) => entry.id !== id);
      return {
        sqlSnippets,
        bootstrap: state.bootstrap ? { ...state.bootstrap, sqlSnippets } : null,
      };
    });
  },
  async saveVirtualForeignKey(input) {
    const saved = await client.call<VirtualForeignKey>("save_virtual_foreign_key", { input });
    set((state) => {
      const virtualForeignKeys = [
        saved,
        ...state.virtualForeignKeys.filter((entry) => entry.id !== saved.id),
      ].sort(
        (a, b) =>
          a.source.schema.localeCompare(b.source.schema) ||
          a.source.name.localeCompare(b.source.name) ||
          a.name.toLowerCase().localeCompare(b.name.toLowerCase()) ||
          a.id.localeCompare(b.id),
      );
      return {
        virtualForeignKeys,
        tabs: state.tabs.map((t) =>
          t.kind === "table" && t.schema && t.connectionId === saved.connectionId
            ? {
                ...t,
                schema: reconcileTableSchemaVirtualFks(
                  t.schema,
                  virtualForeignKeys,
                  t.connectionId,
                ),
              }
            : t,
        ),
        bootstrap: state.bootstrap ? { ...state.bootstrap, virtualForeignKeys } : null,
      };
    });
    return saved;
  },
  async deleteVirtualForeignKey(id) {
    await client.call("delete_virtual_foreign_key", { id });
    set((state) => {
      const removed = state.virtualForeignKeys.find((entry) => entry.id === id);
      const virtualForeignKeys = state.virtualForeignKeys.filter((entry) => entry.id !== id);
      return {
        virtualForeignKeys,
        tabs: removed
          ? state.tabs.map((t) =>
              t.kind === "table" && t.schema && t.connectionId === removed.connectionId
                ? {
                    ...t,
                    schema: reconcileTableSchemaVirtualFks(
                      t.schema,
                      virtualForeignKeys,
                      t.connectionId,
                    ),
                  }
                : t,
            )
          : state.tabs,
        bootstrap: state.bootstrap ? { ...state.bootstrap, virtualForeignKeys } : null,
      };
    });
  },
  async openLocation(target) {
    const state = get();
    if (target.kind === "savedQuery") {
      return state.openSavedQuery(target.savedQueryId) ? { kind: "opened" } : { kind: "missing" };
    }
    if (target.kind === "tab") {
      const found = state.tabs.find((t) => t.id === target.tabId);
      if (!found) {
        state.notify("That tab has already been closed.");
        return { kind: "missing" };
      }
      set({ activeTabId: found.id });
      return { kind: "opened" };
    }
    const profile = state.bootstrap?.profiles.find((p) => p.id === target.connectionId);
    if (!profile) {
      state.notify("The connection profile for this item was deleted.");
      return { kind: "missing" };
    }
    if (!state.sessions[profile.id]) {
      state.selectConnection(profile.id);
      return { kind: "needsConnect", profile };
    }
    if (target.kind === "table") {
      await state.openTable(profile.id, target.table);
      return { kind: "opened" };
    }
    await state.openObject(profile.id, target.object);
    return { kind: "opened" };
  },
  openUtility(kind) {
    const existing = get().tabs.find(
      (t) =>
        t.kind === kind && (kind !== "processes" || t.connectionId === get().selectedConnectionId),
    );
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const next = tab(
      kind,
      {
        history: "Query history",
        processes: "Processes",
        benchmark: "Grid benchmark",
        workspaceSql: "Workspace SQL",
        dataSearch: "Data search",
      }[kind],
      get().selectedConnectionId,
    );
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
  },
  openDiagram(connectionId, schema) {
    const existing = get().tabs.find(
      (t) => t.kind === "diagram" && t.connectionId === connectionId && t.diagramSchema === schema,
    );
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const next = { ...tab("diagram", `${schema} diagram`, connectionId), diagramSchema: schema };
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
  },
  openSchemaDiff(connectionId, schema) {
    const existing = get().tabs.find(
      (t) =>
        t.kind === "schemaDiff" && t.connectionId === connectionId && t.diagramSchema === schema,
    );
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const next = {
      ...tab("schemaDiff", `${schema} schema diff`, connectionId),
      diagramSchema: schema,
    };
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
  },
  async openTable(connectionId, table, rawQuery = EMPTY_QUERY, preferredView, highlightedColumn) {
    const currentSession = get().sessions[connectionId];
    if (currentSession) assertOrdinarySession(currentSession.id);
    const query = normalizeQuery(rawQuery);
    const profileName =
      get().bootstrap?.profiles.find((p) => p.id === connectionId)?.name ?? connectionId;
    const existing = get().tabs.find(
      (t) =>
        t.kind === "table" &&
        t.connectionId === connectionId &&
        t.table?.schema === table.schema &&
        t.table.name === table.name &&
        sameQuery(t.query, query),
    );
    if (existing) {
      set((state) => ({
        activeTabId: existing.id,
        tabs:
          preferredView || highlightedColumn !== undefined
            ? state.tabs.map((t) =>
                t.id === existing.id
                  ? {
                      ...t,
                      ...(preferredView ? { view: preferredView } : {}),
                      ...(highlightedColumn !== undefined ? { highlightedColumn } : {}),
                    }
                  : t,
              )
            : state.tabs,
      }));
      get().recordRecentLocation({
        key: `table:${connectionId}:${table.schema}.${table.name}`,
        label: `${table.schema}.${table.name}`,
        subtitle: `${profileName} · Table`,
        target: { kind: "table", connectionId, table },
      });
      return;
    }
    const next = tab("table", tableTabTitle(table.name, query), connectionId);
    next.table = table;
    next.query = query;
    if (preferredView) next.view = preferredView;
    if (highlightedColumn !== undefined) next.highlightedColumn = highlightedColumn;
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
    get().recordRecentLocation({
      key: `table:${connectionId}:${table.schema}.${table.name}`,
      label: `${table.schema}.${table.name}`,
      subtitle: `${profileName} · Table`,
      target: { kind: "table", connectionId, table },
    });
    await get().refreshTable(next.id);
  },
  openLinkedSessionObject(inspectorId, object) {
    const state = get();
    const inspector = state.tabs.find((entry) => entry.id === inspectorId);
    if (!inspector?.metadataOrigin?.catalogRevision || !metadataSessionForTab(inspector, state))
      return;
    get().openSessionObject(
      inspector.metadataOrigin.tabId,
      object,
      undefined,
      inspector.metadataOrigin.catalogRevision,
    );
  },
  appendSessionDdl(inspectorId, ddl) {
    const state = get();
    const inspector = state.tabs.find((entry) => entry.id === inspectorId);
    if (!inspector?.metadataOrigin?.catalogRevision || !metadataSessionForTab(inspector, state))
      return false;
    const source = state.tabs.find((entry) => entry.id === inspector.metadataOrigin!.tabId)!;
    if (!ddl.trim() || ["queued", "running"].includes(source.status)) return false;
    // Preserve the editor text, file dirty tracking, results and session ownership.
    set({
      tabs: state.tabs.map((entry) =>
        entry.id === source.id
          ? { ...entry, sql: entry.sql + (entry.sql ? "\n\n" : "") + ddl }
          : entry,
      ),
      activeTabId: source.id,
    });
    return true;
  },
  openSessionObject(tabId, object, highlightedColumn, catalogRevision) {
    const state = get();
    const source = state.tabs.find((entry) => entry.id === tabId && entry.kind === "query");
    const session = source && querySessionForTab(source, state.sessions);
    if (!source?.connectionId || !session || session.connectionId !== source.connectionId) return;
    assertOrdinarySession(session.id);
    const origin: MetadataOrigin = {
      tabId,
      sessionId: session.id,
      contextRevision: state.sessionContextRevision[session.id] ?? 0,
      catalogRevision,
    };
    const existing = state.tabs.find(
      (entry) =>
        entry.kind === "object" &&
        entry.connectionId === source.connectionId &&
        entry.object &&
        sameDatabaseObject(entry.object, object) &&
        entry.metadataOrigin?.tabId === tabId &&
        entry.metadataOrigin.sessionId === session.id &&
        entry.metadataOrigin.contextRevision === origin.contextRevision &&
        entry.metadataOrigin.catalogRevision === catalogRevision,
    );
    if (existing) {
      set({
        activeTabId: existing.id,
        tabs: state.tabs.map((entry) =>
          entry.id === existing.id
            ? {
                ...entry,
                view: "structure",
                highlightedColumn: highlightedColumn ?? null,
                revision: entry.revision + 1,
              }
            : entry,
        ),
      });
      return;
    }
    const label =
      state.sessionPool[source.connectionId]?.find((entry) => entry.info.id === session.id)
        ?.label ?? "Shared session";
    const next = tab("object", `${objectTabTitle(object)} · ${label}`, source.connectionId);
    next.object = object;
    next.view = "structure";
    next.highlightedColumn = highlightedColumn ?? null;
    next.metadataOrigin = origin;
    set({ tabs: [...state.tabs, next], activeTabId: next.id });
  },
  async openObject(connectionId, object, preferredView, highlightedColumn) {
    const currentSession = get().sessions[connectionId];
    if (currentSession) assertOrdinarySession(currentSession.id);
    if (isTabularObjectKind(object.kind) && (currentSession?.capabilities.tableBrowsing ?? true)) {
      await get().openTable(
        connectionId,
        { schema: object.schema, name: object.name },
        EMPTY_QUERY,
        preferredView,
        highlightedColumn,
      );
      return;
    }
    const profileName =
      get().bootstrap?.profiles.find((p) => p.id === connectionId)?.name ?? connectionId;
    const existing = get().tabs.find(
      (t) =>
        t.kind === "object" &&
        t.connectionId === connectionId &&
        sameDatabaseObject(t.object, object),
    );
    if (existing) {
      set((state) => ({
        activeTabId: existing.id,
        tabs:
          preferredView || highlightedColumn !== undefined
            ? state.tabs.map((t) =>
                t.id === existing.id
                  ? {
                      ...t,
                      ...(preferredView ? { view: preferredView } : {}),
                      ...(highlightedColumn !== undefined ? { highlightedColumn } : {}),
                    }
                  : t,
              )
            : state.tabs,
      }));
      get().recordRecentLocation({
        key: `object:${connectionId}:${databaseObjectKey(object)}`,
        label: `${object.schema}.${objectTabTitle(object)}`,
        subtitle: `${profileName} · ${object.kind}`,
        target: { kind: "object", connectionId, object },
      });
      return;
    }
    const next = tab("object", objectTabTitle(object), connectionId);
    next.object = object;
    next.view = preferredView ?? "structure";
    if (highlightedColumn !== undefined) next.highlightedColumn = highlightedColumn;
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
    get().recordRecentLocation({
      key: `object:${connectionId}:${databaseObjectKey(object)}`,
      label: `${object.schema}.${objectTabTitle(object)}`,
      subtitle: `${profileName} · ${object.kind}`,
      target: { kind: "object", connectionId, object },
    });
  },
  async setTableQuery(id, rawQuery, options) {
    const current = get().tabs.find((t) => t.id === id);
    if (!current?.table) return { kind: "refused", reason: "This tab does not browse a table" };
    if (
      dirtyStore.pendingCountForTab(
        current.id,
        current.pages.map((page) => page.result.id),
      ) > 0
    )
      return {
        kind: "refused",
        reason: "Review or discard pending edits before changing sort or filters",
      };
    const session = current.connectionId ? get().sessions[current.connectionId] : null;
    if (!session) return { kind: "refused", reason: "Connect to change sort or filters" };
    assertOrdinarySession(session.id);
    const query = normalizeQuery(rawQuery);
    if (!options?.force && !options?.approval && sameQuery(current.query, query))
      return { kind: "applied" };
    if (hasRawClauses(query) && !options?.approval) {
      try {
        const plan = await client.call<PreparedPlan | null>("prepare_browse_table", {
          sessionId: session.id,
          table: current.table,
          query,
        });
        if (plan) return { kind: "review", plan, query };
      } catch (error) {
        const failure = asAppError(error);
        if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
          get().updateTab(id, { error: failure });
        retireAfterBrowseFailure(session.id, failure);
        return { kind: "applied" };
      }
    }
    get().updateTab(id, { query, error: null });
    await get().refreshTable(id, options?.approval);
    return { kind: "applied" };
  },
  async refreshTable(id, approval) {
    const current = get().tabs.find((t) => t.id === id),
      session = current?.connectionId ? get().sessions[current.connectionId] : null;
    if (!current?.table || !session) return;
    assertOrdinarySession(session.id);
    dirtyStore.clearInserts(current.id);
    for (const page of current.pages) {
      resultCache.release(page.result.id);
      dirtyStore.clear(page.result.id);
      if (!isConsoleOwnedResult(page.result))
        void client.call("release_result", { resultId: page.result.id });
    }
    get().updateTab(id, {
      pages: [],
      pageKeys: {},
      loadedRows: 0,
      nextKey: null,
      hasMore: true,
      loadingPage: true,
      schema: null,
      error: null,
      status: "idle",
      runId: null,
      revision: current.revision + 1,
      dataSessionId: session.id,
    });
    try {
      const schema = await client.call<TableSchema>("table_schema", {
        sessionId: session.id,
        table: current.table,
      });
      if (!requestIsCurrent(id, current.revision + 1, current.connectionId!, session.id)) return;
      get().updateTab(id, { schema, loadingPage: false });
      await get().loadNextPage(id, approval);
    } catch (error) {
      const failure = asAppError(error);
      if (requestIsCurrent(id, current.revision + 1, current.connectionId!, session.id))
        get().updateTab(id, {
          error: failure,
          loadingPage: false,
          hasMore: false,
        });
      retireAfterBrowseFailure(session.id, failure);
    }
  },
  async loadNextPage(id, approval) {
    const current = get().tabs.find((t) => t.id === id),
      session = current?.connectionId ? get().sessions[current.connectionId] : null;
    if (
      !current?.table ||
      !session ||
      current.dataSessionId !== session.id ||
      current.loadingPage ||
      !current.hasMore ||
      current.error
    )
      return;
    assertOrdinarySession(session.id);
    get().updateTab(id, { loadingPage: true });
    const start = current.loadedRows,
      after = current.nextKey;
    try {
      const page = await client.call<import("../ipc/generated").TablePage>("browse_table", {
        sessionId: session.id,
        table: current.table,
        after,
        query: isEmptyQuery(current.query) ? null : current.query,
        approval: approval ?? null,
      });
      if (
        !get().tabs.some((t) => t.id === id && t.revision === current.revision) ||
        get().sessions[current.connectionId!]?.id !== session.id
      ) {
        if (!isConsoleOwnedResult(page.result))
          void client.call("release_result", { resultId: page.result.id });
        return;
      }
      const latest = get().tabs.find((t) => t.id === id)!;
      const schema = verifyPageColumns(latest.schema, page.result);
      const pages = [...latest.pages, { start, after, result: page.result }];
      if (pages.length > 8) {
        const evict = pages.findIndex(
          (p) => !dirtyStore.hasResult(p.result.id) && p.start < start - 1000,
        );
        if (evict >= 0) {
          const [old] = pages.splice(evict, 1);
          resultCache.release(old.result.id);
          if (!isConsoleOwnedResult(old.result))
            void client.call("release_result", { resultId: old.result.id });
        }
      }
      get().updateTab(id, {
        schema,
        pages,
        pageKeys: { ...latest.pageKeys, [start]: after },
        loadedRows: start + page.result.retainedRows,
        nextKey: page.nextKey,
        hasMore: page.hasMore,
      });
    } catch (error) {
      const failure = asAppError(error);
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, {
          error: failure,
          loadingPage: false,
          hasMore: false,
        });
      retireAfterBrowseFailure(session.id, failure);
    } finally {
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, { loadingPage: false });
    }
  },
  async ensureTablePage(id, row) {
    const current = get().tabs.find((t) => t.id === id),
      session = current?.connectionId ? get().sessions[current.connectionId] : null;
    if (
      !current?.table ||
      !session ||
      current.dataSessionId !== session.id ||
      current.error ||
      current.loadingPage ||
      current.pages.some((p) => row >= p.start && row < p.start + p.result.retainedRows)
    )
      return;
    const start = Object.keys(current.pageKeys)
      .map(Number)
      .filter((key) => key <= row)
      .reduce((a, b) => Math.max(a, b), -1);
    if (start < 0) return;
    assertOrdinarySession(session.id);
    get().updateTab(id, { loadingPage: true });
    try {
      const page = await client.call<import("../ipc/generated").TablePage>("browse_table", {
        sessionId: session.id,
        table: current.table,
        after: current.pageKeys[start],
        query: isEmptyQuery(current.query) ? null : current.query,
      });
      const latest = get().tabs.find((t) => t.id === id);
      if (!latest || latest.revision !== current.revision) {
        if (!isConsoleOwnedResult(page.result))
          void client.call("release_result", { resultId: page.result.id });
        return;
      }
      const schema = verifyPageColumns(latest.schema, page.result);
      const pages = [
        ...latest.pages,
        { start, after: current.pageKeys[start], result: page.result },
      ];
      if (pages.length > 8) {
        const evict = pages.findIndex(
          (p) => Math.abs(p.start - start) > 1500 && !dirtyStore.hasResult(p.result.id),
        );
        if (evict >= 0) {
          const [old] = pages.splice(evict, 1);
          resultCache.release(old.result.id);
          if (!isConsoleOwnedResult(old.result))
            void client.call("release_result", { resultId: old.result.id });
        }
      }
      get().updateTab(id, { schema, pages });
    } catch (error) {
      const failure = asAppError(error);
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, { error: failure, loadingPage: false, hasMore: false });
      retireAfterBrowseFailure(session.id, failure);
    } finally {
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, { loadingPage: false });
    }
  },
  updateTab(id, update) {
    set((state) => ({
      tabs: state.tabs.map((t) => {
        if (t.id !== id) return t;
        const patch = typeof update === "function" ? update(t) : update;
        const changedSql = patch.sql !== undefined && patch.sql !== t.sql;
        return {
          ...t,
          ...patch,
          ...(changedSql || (patch.error === null && !patch.errorOrigin)
            ? { errorOrigin: undefined }
            : {}),
        };
      }),
    }));
  },
  togglePinTab(id) {
    set((state) => {
      const target = state.tabs.find((t) => t.id === id);
      if (!target) return {};
      const nextPinned = !target.pinned;
      const updated = state.tabs.map((t) => (t.id === id ? { ...t, pinned: nextPinned } : t));
      return { tabs: orderPinnedFirst(updated) };
    });
  },
  closeTab(id) {
    for (const receipt of Object.values(get().snowflakeConsoleReceipts))
      if (receipt.tabId === id && consoleReceiptHeld(receipt))
        void get()
          .cancelSnowflakeConsole(receipt.clientRequestId)
          .catch((error) => get().notify(messageOf(error)));

    planSnapshots.removeOwner(id);
    const stateSnapshot = get();
    const current = stateSnapshot.tabs.find((t) => t.id === id);
    if (!current) return;
    const closedSnapshot = snapshotClosedTab(current);
    if (current.fileToken) {
      void client.call("release_sql_file_grant", { token: current.fileToken }).catch(() => {});
    }
    const disconnectSecondary =
      current.querySession &&
      !current.querySessionClosed &&
      isLastOwnerOfSecondarySession(stateSnapshot.tabs, current)
        ? current.querySession.id
        : null;
    if (disconnectSecondary)
      void client
        .call("disconnect", { sessionId: disconnectSecondary })
        .catch((error) => get().notify(messageOf(error)));
    else if (current.runId && ["queued", "running"].includes(current.status))
      void client.call("cancel_run", { runId: current.runId }).catch(() => {});
    dirtyStore.clearInserts(current.id);
    const closingSet = new Set([id]);
    for (const result of [...current.results, ...current.pages.map((p) => p.result)]) {
      if (!isResultReferencedByOtherTabs(stateSnapshot.tabs, closingSet, result.id)) {
        dirtyStore.clearTab(result.id, [result.id]);
        resultCache.release(result.id);
        if (!isConsoleOwnedResult(result))
          void client.call("release_result", { resultId: result.id }).catch(() => {});
      }
    }
    set((state) => {
      const index = state.tabs.findIndex((t) => t.id === id),
        tabs = state.tabs.filter((t) => t.id !== id);
      let sessionPool = state.sessionPool;
      if (disconnectSecondary) {
        sessionPool = {};
        for (const [connId, entries] of Object.entries(state.sessionPool)) {
          const remaining = entries.filter((entry) => entry.info.id !== disconnectSecondary);
          if (remaining.length > 0) sessionPool[connId] = remaining;
        }
      }
      const closedTabs = closedSnapshot
        ? [
            closedSnapshot,
            ...state.closedTabs.filter((entry) => entry.id !== closedSnapshot.id),
          ].slice(0, MAX_CLOSED_TABS)
        : state.closedTabs;
      return {
        tabs,
        closedTabs,
        sessionPool,
        recentLocations: state.recentLocations.filter((entry) => entry.key !== `tab:${id}`),
        ...(disconnectSecondary ? purgeSessionRuntimeMaps(state, [disconnectSecondary]) : {}),
        activeTabId:
          state.activeTabId === id ? (tabs[Math.max(0, index - 1)]?.id ?? null) : state.activeTabId,
      };
    });
  },
  closeOtherTabs(keepTabId) {
    const stateSnapshot = get();
    const keepExists = stateSnapshot.tabs.some((t) => t.id === keepTabId);
    if (!keepExists) return 0;
    let skippedProtected = 0;
    const toCloseIds: string[] = [];
    for (const candidate of stateSnapshot.tabs) {
      if (candidate.id === keepTabId || candidate.pinned) continue;
      if (isTabProtectedFromBulkClose(stateSnapshot.tabs, candidate)) {
        skippedProtected += 1;
        continue;
      }
      toCloseIds.push(candidate.id);
    }
    for (const id of toCloseIds) {
      get().closeTab(id);
    }
    set({ activeTabId: keepTabId });
    if (skippedProtected > 0) {
      get().notify(
        `Kept ${skippedProtected} tab${skippedProtected === 1 ? "" : "s"} with unsaved changes or active sessions open.`,
      );
    }
    return toCloseIds.length;
  },
  closeCleanTabs() {
    const stateSnapshot = get();
    let skippedProtected = 0;
    const toCloseIds: string[] = [];
    for (const candidate of stateSnapshot.tabs) {
      if (candidate.pinned) continue;
      if (isTabProtectedFromBulkClose(stateSnapshot.tabs, candidate)) {
        skippedProtected += 1;
        continue;
      }
      toCloseIds.push(candidate.id);
    }
    for (const id of toCloseIds) {
      get().closeTab(id);
    }
    if (skippedProtected > 0) {
      get().notify(
        `Kept ${skippedProtected} modified or active tab${skippedProtected === 1 ? "" : "s"} open.`,
      );
    }
    return toCloseIds.length;
  },
  async reopenClosedTab() {
    const stateSnapshot = get();
    const [entry, ...remaining] = stateSnapshot.closedTabs;
    if (!entry) {
      get().notify("No recently closed tabs to reopen.");
      return null;
    }
    set({ closedTabs: remaining });

    if (entry.kind === "table" && entry.connectionId && entry.table) {
      if (get().sessions[entry.connectionId]) {
        await get().openTable(entry.connectionId, entry.table, entry.query, entry.view);
        const activeId = get().activeTabId;
        if (activeId && entry.pinned) {
          get().togglePinTab(activeId);
        }
        return activeId;
      }
      const reopened: StudioTab = {
        ...tab("table", entry.title, entry.connectionId),
        id: entry.id,
        table: entry.table,
        object: {
          schema: entry.table.schema,
          name: entry.table.name,
          kind: "table",
          identity: null,
        },
        query: entry.query,
        view: entry.view,
        pinned: entry.pinned,
      };
      set((state) => ({
        tabs: orderPinnedFirst([...state.tabs, reopened]),
        activeTabId: reopened.id,
      }));
      return reopened.id;
    }

    if (entry.kind === "object" && entry.connectionId && entry.object) {
      if (get().sessions[entry.connectionId]) {
        await get().openObject(entry.connectionId, entry.object, entry.view);
        const activeId = get().activeTabId;
        if (activeId && entry.pinned) {
          get().togglePinTab(activeId);
        }
        return activeId;
      }
      const reopened: StudioTab = {
        ...tab("object", entry.title, entry.connectionId),
        id: entry.id,
        object: entry.object,
        view: entry.view,
        pinned: entry.pinned,
      };
      set((state) => ({
        tabs: orderPinnedFirst([...state.tabs, reopened]),
        activeTabId: reopened.id,
      }));
      return reopened.id;
    }

    const reopenedId = get().tabs.some((t) => t.id === entry.id) ? crypto.randomUUID() : entry.id;
    const reopened: StudioTab = {
      ...tab(entry.kind, entry.title, entry.connectionId),
      id: reopenedId,
      sql: entry.sql,
      savedQueryId: entry.savedQueryId,
      pinned: entry.pinned,
      diagramSchema: entry.diagramSchema,
    };
    set((state) => ({
      tabs: orderPinnedFirst([...state.tabs, reopened]),
      activeTabId: reopened.id,
    }));
    return reopened.id;
  },
  removeConnection(id) {
    const state = get();
    const deletedProfile = state.bootstrap?.profiles.find((p) => p.id === id);
    const purgeSensitiveQueries = Boolean(
      deletedProfile &&
      (deletedProfile.environment === "production" || !deletedProfile.storeQueryText),
    );
    const sessionIds = new Set<string>();
    if (state.sessions[id]) sessionIds.add(state.sessions[id].id);
    for (const entry of state.sessionPool[id] ?? []) sessionIds.add(entry.info.id);
    for (const t of state.tabs) {
      if (t.connectionId === id && t.querySession && !t.querySessionClosed)
        sessionIds.add(t.querySession.id);
    }
    for (const sessionId of sessionIds) void client.call("disconnect", { sessionId });
    get().disconnected(id);
    for (const t of get().tabs.filter((t) => t.connectionId === id)) get().closeTab(t.id);
    set((state) => {
      const catalog = { ...state.catalog };
      delete catalog[id];
      const purgedQueryIds = new Set<string>();
      const savedQueries = state.savedQueries.flatMap((entry) => {
        if (entry.connectionId !== id) return [entry];
        if (purgeSensitiveQueries) {
          purgedQueryIds.add(entry.id);
          return [];
        }
        return [{ ...entry, connectionId: null }];
      });
      const bookmarks = state.bookmarks.filter((entry) => {
        if (entry.target.kind === "object" && entry.target.connectionId === id) return false;
        if (entry.target.kind === "savedQuery" && purgedQueryIds.has(entry.target.savedQueryId))
          return false;
        return true;
      });
      const virtualForeignKeys = state.virtualForeignKeys.filter(
        (entry) => entry.connectionId !== id,
      );
      const recentLocations = state.recentLocations.filter((entry) => {
        if (
          (entry.target.kind === "table" || entry.target.kind === "object") &&
          entry.target.connectionId === id
        )
          return false;
        if (entry.target.kind === "savedQuery" && purgedQueryIds.has(entry.target.savedQueryId))
          return false;
        return true;
      });
      return {
        catalog,
        savedQueries,
        bookmarks,
        virtualForeignKeys,
        recentLocations,
        closedTabs: state.closedTabs.filter((entry) => entry.connectionId !== id),
        tabs:
          purgedQueryIds.size > 0
            ? state.tabs.map((t) =>
                t.savedQueryId && purgedQueryIds.has(t.savedQueryId)
                  ? { ...t, savedQueryId: null }
                  : t,
              )
            : state.tabs,
        bootstrap: state.bootstrap
          ? {
              ...state.bootstrap,
              profiles: state.bootstrap.profiles.filter((p) => p.id !== id),
              savedQueries,
              bookmarks,
              virtualForeignKeys,
            }
          : null,
        tableLayouts: state.tableLayouts.filter((layout) => layout.connectionId !== id),
        selectedConnectionId: state.selectedConnectionId === id ? null : state.selectedConnectionId,
      };
    });
  },
  disconnected(connectionId) {
    planSnapshots.removeConnection(connectionId);
    set((state) => {
      const retiredIds = new Set<string>();
      if (state.sessions[connectionId]) retiredIds.add(state.sessions[connectionId].id);
      for (const entry of state.sessionPool[connectionId] ?? []) retiredIds.add(entry.info.id);
      for (const t of state.tabs) {
        if (t.connectionId === connectionId && t.querySession) retiredIds.add(t.querySession.id);
      }
      const sessions = { ...state.sessions };
      delete sessions[connectionId];
      const sessionPool = { ...state.sessionPool };
      delete sessionPool[connectionId];
      const objects = { ...state.objects };
      delete objects[connectionId];
      return {
        sessions,
        sessionPool,
        ...purgeSessionRuntimeMaps(state, retiredIds),
        objects,
        tabs: state.tabs.map((t) =>
          t.connectionId === connectionId
            ? {
                ...t,
                planSource: undefined,
                ...(t.querySession ? { querySessionClosed: true } : {}),
              }
            : t,
        ),
      };
    });
  },
  resetConnectionResults(connectionId) {
    get().disconnected(connectionId);
    for (const tab of get().tabs.filter((t) => t.connectionId === connectionId)) {
      dirtyStore.clearInserts(tab.id);
      for (const result of [...tab.results, ...tab.pages.map((p) => p.result)]) {
        resultCache.release(result.id);
        dirtyStore.clearTab(result.id, [result.id]);
        if (!isConsoleOwnedResult(result))
          void client.call("release_result", { resultId: result.id }).catch(() => {});
      }
      get().updateTab(tab.id, {
        results: [],
        pinnedResultIds: undefined,
        pinnedResultLabels: undefined,
        resultLayouts: undefined,
        snowflakeDdlResult: undefined,
        snowflakeDmlResult: undefined,
        planSource: undefined,
        pages: [],
        pageKeys: {},
        loadedRows: 0,
        hasMore: false,
        schema: null,
        nextKey: null,
        loadingPage: false,
        runId: null,
        activeResultId: null,
        status: "idle",
        executionSummary: undefined,
        error: null,
        revision: tab.revision + 1,
      });
    }
  },
  applyEvents(tabId, events, revision) {
    const runtimeUpdates: SessionRuntimeState[] = [];
    for (const event of events) {
      if (event.type === "sessionClosed") {
        get().retireSession(event.sessionId);
        get().notify(event.reason);
      }
      if (event.type === "notice") get().notify(event.message);
      if (event.type === "sessionState") {
        const source = get().tabs.find((tab) => tab.id === tabId && tab.revision === revision);
        const sourceSession = source && querySessionForTab(source, get().sessions);
        if (
          sourceSession?.id === event.state.sessionId &&
          !get().sessionTransactionPending[event.state.sessionId]
        ) {
          get().invalidateSessionContextAuthority(event.state.sessionId);
          contextRequests.delete(event.state.sessionId);
          runtimeUpdates.push(event.state);
        }
      }
    }
    if (runtimeUpdates.length > 0) {
      set((state) => {
        let changed = false;
        const sessionStates = { ...state.sessionStates };
        const sessionContexts = { ...state.sessionContexts };
        const sessionContextRevision = { ...state.sessionContextRevision };
        for (const runtime of runtimeUpdates) {
          if (!isSessionActive(state, runtime.sessionId)) continue;
          const prev = sessionStates[runtime.sessionId];
          sessionStates[runtime.sessionId] = runtime;
          if (runtime.transactionObservation !== undefined) sessionContexts[runtime.sessionId] = [];
          changed = true;
          if (sessionRuntimeContextChanged(prev, runtime)) {
            sessionContextRevision[runtime.sessionId] =
              (sessionContextRevision[runtime.sessionId] ?? 0) + 1;
          }
        }
        return changed ? { sessionStates, sessionContexts, sessionContextRevision } : {};
      });
    }
    set((state) => {
      const current = state.tabs.find((t) => t.id === tabId);
      if (!current || current.revision !== revision) {
        for (const event of events)
          if (event.type === "result" && !isConsoleOwnedResult(event.result))
            void client.call("release_result", { resultId: event.result.id }).catch(() => {});
        return {};
      }
      const next = { ...current, results: [...current.results] };
      const newOutput: DatabaseOutputEntry[] = [];
      const nowMs = Date.now();
      for (const event of events) {
        if (event.type === "started") {
          next.runId = event.runId;
          next.status = "running";
          next.executionSummary = undefined;
          newOutput.push({
            id: `out-${++outputEntrySeq}`,
            timestampMs: nowMs,
            level: "info",
            runId: event.runId,
            message: "Execution started",
          });
        }
        if (event.type === "notice") {
          newOutput.push({
            id: `out-${++outputEntrySeq}`,
            timestampMs: nowMs,
            level: "warning",
            runId: next.runId,
            message: event.message,
          });
        }
        if (event.type === "result") {
          const at = next.results.findIndex((r) => r.id === event.result.id);
          if (at >= 0) {
            if (next.results[at].status !== "streaming") {
              resultCache.release(event.result.id);
            }
            next.results[at] = event.result;
          } else {
            next.results.push(event.result);
          }
          if (!next.activeResultId) next.activeResultId = event.result.id;
          if (event.result.status !== "streaming" && event.result.columns.length > 0) {
            newOutput.push({
              id: `out-${++outputEntrySeq}`,
              timestampMs: nowMs,
              level: event.result.status === "truncated" ? "warning" : "success",
              runId: next.runId,
              statementIndex: event.result.statementIndex,
              message: `Result #${event.result.statementIndex + 1}: ${event.result.retainedRows.toLocaleString()} row${event.result.retainedRows === 1 ? "" : "s"}${event.result.status === "truncated" ? " (retention limit reached)" : ""}`,
            });
          }
        }
        if (event.type === "progress")
          next.results = next.results.map((r) =>
            r.id === event.resultId ? { ...r, retainedRows: event.retainedRows } : r,
          );
        if (event.type === "error") {
          next.error = event.error;
          newOutput.push({
            id: `out-${++outputEntrySeq}`,
            timestampMs: nowMs,
            level: "error",
            runId: next.runId,
            code: event.error.code,
            message: event.error.message,
          });
        }
        if (event.type === "statementCompleted") {
          next.elapsedMs += event.elapsedMs;
          next.metrics = {
            ...next.metrics,
            [event.index]: { affectedRows: event.affectedRows, elapsedMs: event.elapsedMs },
          };
          newOutput.push({
            id: `out-${++outputEntrySeq}`,
            timestampMs: nowMs,
            level: "success",
            runId: next.runId,
            statementIndex: event.index,
            elapsedMs: event.elapsedMs,
            message: `Statement #${event.index + 1} completed · ${event.affectedRows} affected`,
          });
        }
        if (event.type === "runMetrics" && event.runId === next.runId) next.runMetrics = event;
        if (event.type === "finished" && event.runId === next.runId) {
          if (next.clientStartedAt !== undefined)
            next.clientElapsedMs = Math.max(0, performance.now() - next.clientStartedAt);
          next.status = event.status;
          next.executionSummary = event.executionSummary;
          next.elapsedMs = event.elapsedMs;
          next.runId = null;
          if (!next.activeResultId && next.results.length > 0) {
            next.activeResultId = next.results[next.results.length - 1].id;
          }
          newOutput.push({
            id: `out-${++outputEntrySeq}`,
            timestampMs: nowMs,
            level:
              event.status === "completed"
                ? "info"
                : event.status === "cancelled"
                  ? "warning"
                  : "error",
            runId: event.runId,
            elapsedMs: event.elapsedMs,
            message: `Run ${event.status} in ${event.elapsedMs} ms`,
          });
        }
      }
      if (newOutput.length > 0) {
        next.outputLog = appendDatabaseOutputEntries(current.outputLog, newOutput);
      }
      return { tabs: state.tabs.map((t) => (t.id === tabId ? next : t)) };
    });
  },
  clearTabOutput(tabId) {
    set((state) => ({
      tabs: state.tabs.map((t) => (t.id === tabId ? { ...t, outputLog: [] } : t)),
    }));
  },
}));

export function beginEventBatch(
  tabId: string,
  acceptsEvent: (event: QueryEvent) => boolean = () => true,
) {
  const revision = useStudio.getState().tabs.find((tab) => tab.id === tabId)?.revision ?? -1;
  let events: QueryEvent[] = [],
    frame = 0;
  const flush = () => {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    const batch = events;
    events = [];
    useStudio.getState().applyEvents(tabId, batch.filter(acceptsEvent), revision);
  };
  return (event: QueryEvent) => {
    events.push(event);
    if (event.type === "finished") flush();
    else if (!frame) frame = requestAnimationFrame(flush);
  };
}
