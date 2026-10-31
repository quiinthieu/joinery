import { useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  Code2,
  Database,
  FileCode2,
  Gauge,
  History,
  Search,
  type LucideIcon,
} from "lucide-react";
import type { HistoryEntry } from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { Modal } from "../../components/ui/Modal";
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
    [history, setHistory] = useState<HistoryEntry[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setScope(initialScope);
    setActiveIndex(0);
  }, [initialScope]);

  useEffect(() => {
    let active = true;
    void client
      .call<HistoryEntry[]>("history", { search: "" })
      .then((entries) => {
        if (active && Array.isArray(entries)) setHistory(entries);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  const currentTab = state.tabs.find((tab) => tab.id === state.activeTabId);
  const preferredConnectionId = currentTab?.connectionId ?? state.selectedConnectionId ?? null;

  const paletteCommands = useMemo<readonly SearchCommandInput<LucideIcon>[]>(
    () => [
      ...commands,
      {
        label: "Search database objects",
        shortcut: "⇧⌘ O",
        icon: Search,
        keepOpen: true,
        action: () => {
          setScope("objects");
          setActiveIndex(0);
        },
      },
    ],
    [commands],
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
        history,
      }),
    [
      history,
      paletteCommands,
      preferredConnectionId,
      scope,
      search,
      state.activeTabId,
      state.bootstrap?.profiles,
      state.catalog,
      state.objects,
      state.sessions,
      state.tabs,
    ],
  );

  const items = output.items;
  const clampedIndex = items.length > 0 ? Math.min(activeIndex, items.length - 1) : 0;

  function triggerItem(item: SearchResultItem<LucideIcon>) {
    if (item.kind === "command") {
      if (item.keepOpen) {
        item.action();
        inputRef.current?.focus();
        return;
      }
      onClose();
      item.action();
      return;
    }
    if (item.kind === "tab") {
      onClose();
      onActivateTab(item.tabId);
      return;
    }
    if (item.kind === "object") {
      onClose();
      void useStudio
        .getState()
        .openObject(item.connectionId, item.object)
        .catch((err) => useStudio.getState().notify(messageOf(err)));
      return;
    }
    onClose();
    if (item.entry.sql) {
      useStudio.getState().openQuery(item.entry.connectionId, item.entry.sql);
    } else {
      useStudio.getState().notify("Only execution metadata was saved for this query.");
    }
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
      : scope === "history"
        ? "No matching query history"
        : "No matching commands";

  return (
    <Modal title="Command palette" onClose={onClose}>
      <div className="palette-search">
        <Search size={17} />
        <input
          ref={inputRef}
          autoFocus
          aria-label="Search commands"
          placeholder={
            scope === "objects"
              ? "Search tables, views, routines, triggers, indexes…"
              : scope === "history"
                ? "Search executed SQL or connection names…"
                : "What would you like to do?"
          }
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            setActiveIndex(0);
          }}
          onKeyDown={(event) => {
            if (
              (event.metaKey || event.ctrlKey) &&
              event.shiftKey &&
              event.key.toLowerCase() === "o"
            ) {
              event.preventDefault();
              setScope("objects");
              setActiveIndex(0);
              return;
            }
            if (
              (event.metaKey || event.ctrlKey) &&
              !event.shiftKey &&
              event.key.toLowerCase() === "k"
            ) {
              event.preventDefault();
              setScope("all");
              setActiveIndex(0);
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
                  setScope(tab.id);
                  setActiveIndex(0);
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
      <div className="palette-commands" ref={listRef}>
        {items.map((item, idx) => {
          const selected = idx === clampedIndex;
          return (
            <button
              key={item.id}
              type="button"
              data-palette-index={idx}
              className={selected ? "selected" : ""}
              onMouseEnter={() => setActiveIndex(idx)}
              onClick={() => triggerItem(item)}
            >
              {item.kind === "command" ? (
                <item.icon size={16} />
              ) : item.kind === "tab" ? (
                <TabKindGlyph kind={item.tabKind} />
              ) : item.kind === "object" ? (
                <ObjectKindGlyph kind={item.object.kind} size={15} />
              ) : (
                <History size={15} />
              )}
              <span className="palette-item-text">
                <span className="palette-item-label">{item.label}</span>
                {item.subtitle && <small className="palette-item-subtitle">{item.subtitle}</small>}
              </span>
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
