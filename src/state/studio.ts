import { create } from "zustand";
import type {
  AppError,
  Bootstrap,
  BrowseQuery,
  ConnectionProfile,
  QueryEvent,
  ResultInfo,
  SessionInfo,
  TableInfo,
  TableRef,
  TableSchema,
} from "../ipc/generated";
import { client, messageOf } from "../ipc/client";
import { resultCache } from "../data/result-cache";
import { dirtyStore } from "../data/dirty-store";
import { EMPTY_QUERY, isEmptyQuery, sameQuery, tableTabTitle } from "../features/grid/table-query";

export type TabKind = "query" | "table" | "benchmark" | "history" | "processes";
export type TabStatus =
  "idle" | "queued" | "running" | "completed" | "failed" | "cancelled" | "outcomeUnknown";
export interface TablePageState {
  start: number;
  result: ResultInfo;
  after: (string | null)[] | null;
}
export interface StudioTab {
  id: string;
  kind: TabKind;
  title: string;
  connectionId: string | null;
  sql: string;
  results: ResultInfo[];
  activeResultId: string | null;
  status: TabStatus;
  runId: string | null;
  elapsedMs: number;
  error: AppError | null;
  table: TableRef | null;
  /** Server-side sort/filter for table tabs; never persisted. */
  query: BrowseQuery;
  schema: TableSchema | null;
  pages: TablePageState[];
  pageKeys: Record<number, (string | null)[] | null>;
  loadedRows: number;
  hasMore: boolean;
  nextKey: (string | null)[] | null;
  loadingPage: boolean;
  view: "data" | "structure";
  revision: number;
  metrics: Record<number, { affectedRows: string; elapsedMs: number }>;
  dataSessionId: string | null;
  querySession: SessionInfo | null;
  querySessionClosed: boolean;
}

interface StudioState {
  bootstrap: Bootstrap | null;
  error: string | null;
  sessions: Record<string, SessionInfo>;
  catalog: Record<string, TableInfo[]>;
  selectedConnectionId: string | null;
  tabs: StudioTab[];
  activeTabId: string | null;
  theme: "dark" | "light" | "contrast";
  density: "comfortable" | "compact";
  sidebarVisible: boolean;
  inspectorVisible: boolean;
  toast: string | null;
  initialize(): Promise<void>;
  notify(message: string): void;
  setTheme(theme: StudioState["theme"]): void;
  setSession(session: SessionInfo): void;
  bindQuerySession(tabId: string, session: SessionInfo): boolean;
  retireSession(sessionId: string): void;
  updateProfile(profile: ConnectionProfile): void;
  selectConnection(id: string): void;
  refreshCatalog(id: string): Promise<void>;
  openQuery(connectionId?: string | null, sql?: string): string;
  openUtility(kind: "history" | "processes" | "benchmark"): void;
  openTable(connectionId: string, table: TableRef, query?: BrowseQuery): Promise<void>;
  /** Applies a new sort/filter; returns a reason when refused. */
  setTableQuery(id: string, query: BrowseQuery): Promise<string | null>;
  loadNextPage(id: string): Promise<void>;
  ensureTablePage(id: string, row: number): Promise<void>;
  refreshTable(id: string): Promise<void>;
  updateTab(
    id: string,
    update: Partial<StudioTab> | ((tab: StudioTab) => Partial<StudioTab>),
  ): void;
  closeTab(id: string): void;
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
    error: null,
    table: null,
    query: EMPTY_QUERY,
    schema: null,
    pages: [],
    pageKeys: {},
    loadedRows: 0,
    hasMore: false,
    nextKey: null,
    loadingPage: false,
    view: "data",
    revision: 0,
    metrics: {},
    dataSessionId: null,
    querySession: null,
    querySessionClosed: false,
  };
}
export function querySessionForTab(tab: StudioTab, sessions: Record<string, SessionInfo>) {
  if (tab.kind === "query" && tab.querySession)
    return tab.querySessionClosed ? undefined : tab.querySession;
  return tab.connectionId ? sessions[tab.connectionId] : undefined;
}
const storedTheme = localStorage.getItem("joinery.theme");

function requestIsCurrent(id: string, revision: number, connectionId: string, sessionId: string) {
  const state = useStudio.getState();
  return (
    state.tabs.some((tab) => tab.id === id && tab.revision === revision) &&
    state.sessions[connectionId]?.id === sessionId
  );
}
function verifyPageColumns(schema: TableSchema | null, result: ResultInfo) {
  if (!schema || JSON.stringify(schema.columns) !== JSON.stringify(result.columns)) {
    void client.call("release_result", { resultId: result.id }).catch(() => {});
    throw new Error("The table schema changed. Refresh the table before continuing.");
  }
}

export const useStudio = create<StudioState>((set, get) => ({
  bootstrap: null,
  error: null,
  sessions: {},
  catalog: {},
  selectedConnectionId: null,
  tabs: [],
  activeTabId: null,
  theme: storedTheme === "light" || storedTheme === "contrast" ? storedTheme : "dark",
  density: "comfortable",
  sidebarVisible: true,
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
          })) ?? [];
      const existing = get().tabs;
      const tabs = [
        ...restored.filter((draft) => !existing.some((tab) => tab.id === draft.id)),
        ...existing,
      ];
      set({
        bootstrap,
        error: null,
        ...(workspace
          ? {
              theme: workspace.theme as StudioState["theme"],
              density: workspace.density as StudioState["density"],
              sidebarVisible: workspace.sidebarVisible,
              inspectorVisible: workspace.inspectorVisible,
              tabs,
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
  setSession(session) {
    set((state) => ({
      sessions: { ...state.sessions, [session.connectionId]: session },
      selectedConnectionId: session.connectionId,
    }));
  },
  bindQuerySession(tabId, session) {
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
      dirtyStore.clear(result.id);
      void client.call("release_result", { resultId: result.id }).catch(() => {});
    }
    get().updateTab(tabId, {
      querySession: session,
      querySessionClosed: false,
      results: [],
      activeResultId: null,
      dataSessionId: null,
      runId: null,
      error: null,
      status: "idle",
      metrics: {},
      elapsedMs: 0,
      revision: current.revision + 1,
    });
    return true;
  },
  retireSession(sessionId) {
    set((state) => ({
      sessions: Object.fromEntries(
        Object.entries(state.sessions).filter(([, s]) => s.id !== sessionId),
      ),
      tabs: state.tabs.map((t) =>
        t.querySession?.id === sessionId ? { ...t, querySessionClosed: true } : t,
      ),
    }));
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
  async refreshCatalog(id) {
    const session = get().sessions[id];
    if (!session) return;
    const catalog = await client.call<TableInfo[]>("catalog", { sessionId: session.id });
    if (get().sessions[id]?.id === session.id)
      set((state) => ({ catalog: { ...state.catalog, [id]: catalog } }));
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
            : "-- A little space to think.\nSELECT current_database(), current_user, version();");
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
    return next.id;
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
      { history: "Query history", processes: "Processes", benchmark: "Grid benchmark" }[kind],
      get().selectedConnectionId,
    );
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
  },
  async openTable(connectionId, table, query = EMPTY_QUERY) {
    const existing = get().tabs.find(
      (t) =>
        t.kind === "table" &&
        t.connectionId === connectionId &&
        t.table?.schema === table.schema &&
        t.table.name === table.name &&
        sameQuery(t.query, query),
    );
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const next = tab("table", tableTabTitle(table.name, query), connectionId);
    next.table = table;
    next.query = query;
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
    await get().refreshTable(next.id);
  },
  async setTableQuery(id, query) {
    const current = get().tabs.find((t) => t.id === id);
    if (!current?.table) return "This tab does not browse a table";
    if (current.pages.some((page) => dirtyStore.hasResult(page.result.id)))
      return "Review or discard pending edits before changing sort or filters";
    if (sameQuery(current.query, query)) return null;
    get().updateTab(id, { query });
    await get().refreshTable(id);
    return null;
  },
  async refreshTable(id) {
    const current = get().tabs.find((t) => t.id === id),
      session = current?.connectionId ? get().sessions[current.connectionId] : null;
    if (!current?.table || !session) return;
    for (const page of current.pages) {
      resultCache.release(page.result.id);
      dirtyStore.clear(page.result.id);
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
      await get().loadNextPage(id);
    } catch (error) {
      if (requestIsCurrent(id, current.revision + 1, current.connectionId!, session.id))
        get().updateTab(id, {
          error: { code: "database", message: messageOf(error) },
          loadingPage: false,
          hasMore: false,
        });
    }
  },
  async loadNextPage(id) {
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
    get().updateTab(id, { loadingPage: true });
    const start = current.loadedRows,
      after = current.nextKey;
    try {
      const page = await client.call<import("../ipc/generated").TablePage>("browse_table", {
        sessionId: session.id,
        table: current.table,
        after,
        query: isEmptyQuery(current.query) ? null : current.query,
      });
      if (
        !get().tabs.some((t) => t.id === id && t.revision === current.revision) ||
        get().sessions[current.connectionId!]?.id !== session.id
      ) {
        void client.call("release_result", { resultId: page.result.id });
        return;
      }
      const latest = get().tabs.find((t) => t.id === id)!;
      verifyPageColumns(latest.schema, page.result);
      const pages = [...latest.pages, { start, after, result: page.result }];
      if (pages.length > 8) {
        const evict = pages.findIndex(
          (p) => !dirtyStore.hasResult(p.result.id) && p.start < start - 1000,
        );
        if (evict >= 0) {
          const [old] = pages.splice(evict, 1);
          resultCache.release(old.result.id);
          void client.call("release_result", { resultId: old.result.id });
        }
      }
      get().updateTab(id, {
        pages,
        pageKeys: { ...latest.pageKeys, [start]: after },
        loadedRows: start + page.result.retainedRows,
        nextKey: page.nextKey,
        hasMore: page.hasMore,
      });
    } catch (error) {
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, {
          error: { code: "database", message: messageOf(error) },
          hasMore: false,
        });
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
        void client.call("release_result", { resultId: page.result.id });
        return;
      }
      verifyPageColumns(latest.schema, page.result);
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
          void client.call("release_result", { resultId: old.result.id });
        }
      }
      get().updateTab(id, { pages });
    } catch (error) {
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, { error: { code: "database", message: messageOf(error) } });
    } finally {
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, { loadingPage: false });
    }
  },
  updateTab(id, update) {
    set((state) => ({
      tabs: state.tabs.map((t) =>
        t.id === id ? { ...t, ...(typeof update === "function" ? update(t) : update) } : t,
      ),
    }));
  },
  closeTab(id) {
    const current = get().tabs.find((t) => t.id === id);
    if (!current) return;
    if (current.querySession && !current.querySessionClosed)
      void client
        .call("disconnect", { sessionId: current.querySession.id })
        .catch((error) => get().notify(messageOf(error)));
    else if (current.runId && ["queued", "running"].includes(current.status))
      void client.call("cancel_run", { runId: current.runId }).catch(() => {});
    for (const result of [...current.results, ...current.pages.map((p) => p.result)]) {
      dirtyStore.clear(result.id);
      resultCache.release(result.id);
      void client.call("release_result", { resultId: result.id }).catch(() => {});
    }
    set((state) => {
      const index = state.tabs.findIndex((t) => t.id === id),
        tabs = state.tabs.filter((t) => t.id !== id);
      return {
        tabs,
        activeTabId:
          state.activeTabId === id ? (tabs[Math.max(0, index - 1)]?.id ?? null) : state.activeTabId,
      };
    });
  },
  removeConnection(id) {
    const state = get();
    const session = state.sessions[id];
    if (session) void client.call("disconnect", { sessionId: session.id });
    for (const t of state.tabs.filter((t) => t.connectionId === id)) get().closeTab(t.id);
    get().disconnected(id);
    set((state) => ({
      bootstrap: state.bootstrap
        ? { ...state.bootstrap, profiles: state.bootstrap.profiles.filter((p) => p.id !== id) }
        : null,
      selectedConnectionId: state.selectedConnectionId === id ? null : state.selectedConnectionId,
    }));
  },
  disconnected(connectionId) {
    set((state) => {
      const sessions = { ...state.sessions };
      delete sessions[connectionId];
      return {
        sessions,
        tabs: state.tabs.map((t) =>
          t.connectionId === connectionId && t.querySession
            ? { ...t, querySessionClosed: true }
            : t,
        ),
      };
    });
  },
  resetConnectionResults(connectionId) {
    get().disconnected(connectionId);
    for (const tab of get().tabs.filter((t) => t.connectionId === connectionId)) {
      for (const result of [...tab.results, ...tab.pages.map((p) => p.result)]) {
        resultCache.release(result.id);
        dirtyStore.clear(result.id);
        void client.call("release_result", { resultId: result.id }).catch(() => {});
      }
      get().updateTab(tab.id, {
        results: [],
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
        error: null,
        revision: tab.revision + 1,
      });
    }
  },
  applyEvents(tabId, events, revision) {
    for (const event of events) {
      if (event.type === "sessionClosed") {
        get().retireSession(event.sessionId);
        get().notify(event.reason);
      }
      if (event.type === "notice") get().notify(event.message);
    }
    set((state) => {
      const current = state.tabs.find((t) => t.id === tabId);
      if (!current || current.revision !== revision) {
        for (const event of events)
          if (event.type === "result")
            void client.call("release_result", { resultId: event.result.id }).catch(() => {});
        return {};
      }
      const next = { ...current, results: [...current.results] };
      for (const event of events) {
        if (event.type === "started") {
          next.runId = event.runId;
          next.status = "running";
        }
        if (event.type === "result") {
          const at = next.results.findIndex((r) => r.id === event.result.id);
          if (at >= 0) next.results[at] = event.result;
          else next.results.push(event.result);
          if (!next.activeResultId) next.activeResultId = event.result.id;
        }
        if (event.type === "progress")
          next.results = next.results.map((r) =>
            r.id === event.resultId ? { ...r, retainedRows: event.retainedRows } : r,
          );
        if (event.type === "error") next.error = event.error;
        if (event.type === "statementCompleted") {
          next.elapsedMs += event.elapsedMs;
          next.metrics = {
            ...next.metrics,
            [event.index]: { affectedRows: event.affectedRows, elapsedMs: event.elapsedMs },
          };
        }
        if (event.type === "finished") {
          next.status = event.status;
          next.elapsedMs = event.elapsedMs;
          next.runId = null;
        }
      }
      return { tabs: state.tabs.map((t) => (t.id === tabId ? next : t)) };
    });
  },
}));

export function beginEventBatch(tabId: string) {
  const revision = useStudio.getState().tabs.find((tab) => tab.id === tabId)?.revision ?? -1;
  let events: QueryEvent[] = [],
    frame = 0;
  const flush = () => {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    const batch = events;
    events = [];
    useStudio.getState().applyEvents(tabId, batch, revision);
  };
  return (event: QueryEvent) => {
    events.push(event);
    if (event.type === "finished") flush();
    else if (!frame) frame = requestAnimationFrame(flush);
  };
}
