import { useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  ArrowLeftRight,
  Bookmark,
  Code2,
  Database,
  FileCode2,
  Gauge,
  History,
  Network,
  Search,
  type LucideIcon,
} from "lucide-react";
import type { HistoryEntry, HistoryPage } from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { Modal } from "../../components/ui/Modal";
import {
  historyAccessEpoch,
  historyClearPending,
  historySearchError,
  useHistoryAccessEpoch,
} from "./history-access";
import { tabNavigation } from "../../components/ui/tabs";
import { useStudio, type TabKind } from "../../state/studio";
import { ObjectKindGlyph } from "./ObjectInspectorView";
import {
  buildSearchEverywhereResults,
  SEARCH_SCOPES,
  type SearchCommandInput,
  type SearchResultItem,
  type SearchScope,
} from "./search-everywhere";
import {
  effectiveShortcutBadge,
  effectiveShortcutBinding,
  matchesKeyboardEvent,
} from "./shortcuts";

function TabKindGlyph({ kind }: { kind: TabKind }) {
  switch (kind) {
    case "query":
      return <FileCode2 size={16} />;
    case "table":
      return <Database size={16} />;
    case "object":
      return <Code2 size={16} />;
    case "history":
      return <History size={16} />;
    case "processes":
      return <Activity size={16} />;
    case "diagram":
      return <Network size={16} />;
    case "schemaDiff":
      return <ArrowLeftRight size={16} />;
    case "benchmark":
      return <Gauge size={16} />;
  }
}

export function SearchEverywhere({
  commands,
  initialScope = "all",
  onActivateTab,
  onClose,
}: {
  commands: readonly SearchCommandInput<LucideIcon>[];
  initialScope?: SearchScope;
  onActivateTab(tabId: string): void;
  onClose(): void;
}) {
  const state = useStudio();
  const [search, setSearch] = useState(""),
    [scope, setScope] = useState<SearchScope>(initialScope),
    [activeIndex, setActiveIndex] = useState(0),
    [history, setHistory] = useState<{
      query: string;
      scope: SearchScope;
      page: HistoryPage;
    } | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [opening, setOpening] = useState(false);
  const [retry, setRetry] = useState(0);
  const epoch = useHistoryAccessEpoch();
  const generation = useRef(0);
  const detail = useRef(0);
  const mounted = useRef(false);
  function invalidate() {
    generation.current += 1;
    detail.current += 1;
    setOpening(false);
    setHistory(null);
  }
  function close() {
    invalidate();
    mounted.current = false;
    onClose();
  }
  function changeScope(next: SearchScope) {
    if (next !== scope) invalidate();
    setScope(next);
    setActiveIndex(0);
  }
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      generation.current += 1;
      detail.current += 1;
    };
  }, []);
  useEffect(() => {
    generation.current += 1;
    detail.current += 1;
    setHistory(null);
    setScope(initialScope);
    setActiveIndex(0);
  }, [initialScope]);
  useEffect(() => {
    const version = ++generation.current;
    detail.current += 1;
    setOpening(false);
    setHistory(null);
    setHistoryError(null);
    if ((scope !== "all" && scope !== "history") || historyClearPending()) {
      setHistoryLoading(false);
      return;
    }
    const invalid = historySearchError(search);
    if (invalid) {
      setHistoryError(invalid);
      setHistoryLoading(false);
      return;
    }
    setHistoryLoading(true);
    const timer = setTimeout(() => {
      void client
        .call<HistoryPage>("history_page", { request: { search, cursor: null, limit: 80 } })
        .then((page) => {
          if (mounted.current && generation.current === version && historyAccessEpoch() === epoch)
            setHistory({ query: search, scope, page });
        })
        .catch((cause) => {
          if (mounted.current && generation.current === version && historyAccessEpoch() === epoch)
            setHistoryError(messageOf(cause));
        })
        .finally(() => {
          if (mounted.current && generation.current === version && historyAccessEpoch() === epoch)
            setHistoryLoading(false);
        });
    }, 120);
    return () => {
      clearTimeout(timer);
      generation.current += 1;
    };
  }, [search, scope, epoch, retry]);
  const currentHistory = history?.query === search && history.scope === scope ? history.page : null;

  const currentTab = state.tabs.find((tab) => tab.id === state.activeTabId);
  const preferredConnectionId = currentTab?.connectionId ?? state.selectedConnectionId ?? null;

  const searchObjectsBadge = effectiveShortcutBadge("searchObjects", state.shortcuts);
  const paletteCommands = useMemo<readonly SearchCommandInput<LucideIcon>[]>(
    () => [
      ...commands,
      {
        label: "Search database objects",
        shortcut: searchObjectsBadge,
        contextBadge: "Navigation",
        icon: Search,
        keepOpen: true,
        action: () => {
          changeScope("objects");
        },
      },
    ],
    [commands, searchObjectsBadge],
  );

  const output = useMemo(
    () =>
      buildSearchEverywhereResults({
        query: search,
        scope,
        commands: paletteCommands,
        tabs: state.tabs,
        activeTabId: state.activeTabId,
        preferredConnectionId,
        profiles: state.bootstrap?.profiles ?? [],
        sessions: state.sessions,
        catalog: state.catalog,
        objects: state.objects,
        savedQueries: state.savedQueries,
        history: currentHistory?.entries ?? [],
        historyMatchedQuery: currentHistory ? search : undefined,
      }),
    [
      currentHistory,
      paletteCommands,
      preferredConnectionId,
      scope,
      search,
      state.activeTabId,
      state.bootstrap?.profiles,
      state.catalog,
      state.objects,
      state.savedQueries,
      state.sessions,
      state.tabs,
    ],
  );

  const items = output.items;
  const clampedIndex = items.length > 0 ? Math.min(activeIndex, items.length - 1) : 0;

  function triggerItem(item: SearchResultItem<LucideIcon>) {
    if (item.kind === "command") {
      if (item.disabledReason) {
        useStudio.getState().notify(item.disabledReason);
        return;
      }
      if (item.keepOpen) {
        item.action();
        inputRef.current?.focus();
        return;
      }
      close();
      item.action();
      return;
    }
    if (item.kind === "tab") {
      close();
      onActivateTab(item.tabId);
      return;
    }
    if (item.kind === "object") {
      close();
      void useStudio
        .getState()
        .openObject(item.connectionId, item.object)
        .catch((err) => useStudio.getState().notify(messageOf(err)));
      return;
    }
    if (item.kind === "savedQuery") {
      close();
      useStudio.getState().openSavedQuery(item.savedQuery.id);
      return;
    }
    if (opening || historyClearPending()) return;
    const version = ++detail.current;
    const access = historyAccessEpoch();
    setOpening(true);
    setHistoryError(null);
    void client
      .call<HistoryEntry | null>("history_entry", { id: item.entry.id })
      .then((entry) => {
        if (!mounted.current || detail.current !== version || historyAccessEpoch() !== access)
          return;
        if (!entry) throw new Error("This history entry was deleted or cleared. Refresh history.");
        if (
          entry.id !== item.entry.id ||
          entry.connectionId !== item.entry.connectionId ||
          entry.startedAt !== item.entry.startedAt
        )
          throw new Error("The history entry identity changed. Refresh history.");
        if (entry.sql !== null) {
          useStudio.getState().openQuery(entry.connectionId, entry.sql);
          close();
        } else {
          useStudio.getState().notify("Only execution metadata was saved for this query.");
          setHistoryError("SQL text was not stored for this query.");
        }
      })
      .catch((cause) => {
        if (mounted.current && detail.current === version && historyAccessEpoch() === access)
          setHistoryError(messageOf(cause));
      })
      .finally(() => {
        if (mounted.current && detail.current === version) setOpening(false);
      });
  }

  function moveActive(nextIndex: number) {
    setActiveIndex(nextIndex);
    requestAnimationFrame(() => {
      const row = listRef.current?.querySelector<HTMLElement>(
        `[data-palette-index="${nextIndex}"]`,
      );
      if (row && typeof row.scrollIntoView === "function") {
        row.scrollIntoView({ block: "nearest" });
      }
    });
  }

  const emptyMessage =
    scope === "objects"
      ? output.connectedConnectionCount === 0
        ? "Connect a database to search its tables, views, routines, triggers, and indexes"
        : "No matching database objects"
      : scope === "saved"
        ? "No matching saved queries"
        : scope === "history"
          ? "No matching query history"
          : "No matching commands";

  return (
    <Modal title="Command palette" onClose={close}>
      <div className="palette-search">
        <Search size={17} />
        <input
          ref={inputRef}
          autoFocus
          aria-label="Search commands"
          placeholder={
            scope === "objects"
              ? "Search tables, views, routines, triggers, indexes…"
              : scope === "saved"
                ? "Search saved queries in your library…"
                : scope === "history"
                  ? "Search executed SQL or connection names…"
                  : "What would you like to do?"
          }
          value={search}
          onChange={(e) => {
            invalidate();
            setSearch(e.target.value);
            setActiveIndex(0);
          }}
          onKeyDown={(event) => {
            if (
              matchesKeyboardEvent(
                event.nativeEvent,
                effectiveShortcutBinding("searchObjects", state.shortcuts),
              ) ||
              ((event.metaKey || event.ctrlKey) &&
                event.shiftKey &&
                event.key.toLowerCase() === "o")
            ) {
              event.preventDefault();
              changeScope("objects");
              return;
            }
            if (
              matchesKeyboardEvent(
                event.nativeEvent,
                effectiveShortcutBinding("openPalette", state.shortcuts),
              ) ||
              ((event.metaKey || event.ctrlKey) &&
                !event.shiftKey &&
                event.key.toLowerCase() === "k")
            ) {
              event.preventDefault();
              changeScope("all");
              return;
            }
            if (event.key === "ArrowDown" && items.length > 0) {
              event.preventDefault();
              moveActive((clampedIndex + 1) % items.length);
              return;
            }
            if (event.key === "ArrowUp" && items.length > 0) {
              event.preventDefault();
              moveActive((clampedIndex - 1 + items.length) % items.length);
              return;
            }
            if (event.key === "Enter" && items[clampedIndex]) {
              event.preventDefault();
              event.stopPropagation();
              triggerItem(items[clampedIndex]);
            }
          }}
        />
      </div>
      <div className="palette-scopes-bar">
        <div
          className="palette-scopes"
          role="tablist"
          aria-label="Search scope"
          onKeyDown={tabNavigation}
        >
          {SEARCH_SCOPES.map((tab) => {
            const selected = scope === tab.id;
            return (
              <button
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={selected}
                tabIndex={selected ? 0 : -1}
                className={selected ? "active" : ""}
                onClick={(event) => {
                  changeScope(tab.id);
                  if (event.detail > 0) inputRef.current?.focus();
                }}
              >
                {tab.label}
              </button>
            );
          })}
        </div>
        {(scope === "objects" || scope === "all") && output.offlineConnectionCount > 0 && (
          <span className="palette-offline-note">
            {output.connectedConnectionCount > 0
              ? `${output.connectedConnectionCount} connected · ${output.offlineConnectionCount} offline`
              : `${output.offlineConnectionCount} offline`}
          </span>
        )}
      </div>
      {(scope === "all" || scope === "history") && (
        <div className="palette-history-summary">
          {historyLoading && <p role="status">Searching history…</p>}
          {opening && <p role="status">Opening saved SQL…</p>}
          {historyError && (
            <p role="alert">
              {historyError}{" "}
              <button
                type="button"
                onClick={() => {
                  invalidate();
                  setRetry((value) => value + 1);
                }}
              >
                Retry history
              </button>
            </p>
          )}
          <p>
            {currentHistory?.nextCursor
              ? "More matching history is available. "
              : "History shows at most 80 matches. "}
            <button
              type="button"
              onClick={() => {
                useStudio.getState().openUtility("history");
                close();
              }}
            >
              Open query history
            </button>
          </p>
        </div>
      )}
      <div className="palette-commands" ref={listRef}>
        {items.map((item, idx) => {
          const selected = idx === clampedIndex;
          const isDisabled = item.kind === "command" && Boolean(item.disabledReason);
          return (
            <button
              key={item.id}
              type="button"
              data-palette-index={idx}
              disabled={item.kind === "history" && opening}
              aria-disabled={isDisabled || undefined}
              className={`${selected ? "selected" : ""} ${isDisabled ? "disabled" : ""}`.trim()}
              onMouseEnter={() => setActiveIndex(idx)}
              onClick={() => triggerItem(item)}
            >
              {item.kind === "command" ? (
                <item.icon size={16} />
              ) : item.kind === "tab" ? (
                <TabKindGlyph kind={item.tabKind} />
              ) : item.kind === "object" ? (
                <ObjectKindGlyph kind={item.object.kind} size={15} />
              ) : item.kind === "savedQuery" ? (
                <Bookmark size={15} />
              ) : (
                <History size={15} />
              )}
              <span className="palette-item-text">
                <span className="palette-item-label">{item.label}</span>
                {item.subtitle && (
                  <small
                    className={`palette-item-subtitle${item.kind === "history" ? " palette-history-summary" : ""}`}
                  >
                    {item.subtitle}
                  </small>
                )}
              </span>
              {item.kind === "command" && item.contextBadge && (
                <em className="palette-context-badge">{item.contextBadge}</em>
              )}
              {item.kind === "object" && item.environment && (
                <em className={`environment ${item.environment}`}>{item.environment}</em>
              )}
              <kbd>{item.shortcut}</kbd>
            </button>
          );
        })}
        {!items.length && <div className="grid-empty">{emptyMessage}</div>}
      </div>
    </Modal>
  );
}
