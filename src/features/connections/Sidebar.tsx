import { useMemo, useRef, useState, type KeyboardEvent } from "react";
import {
  Activity,
  Bookmark,
  Braces,
  ChevronDown,
  ChevronRight,
  Database,
  Eye,
  Hash,
  History,
  Layers,
  ListTree,
  Pencil,
  Plus,
  Search,
  Settings2,
  SquareTerminal,
  Star,
  Table2,
  Unplug,
  Zap,
} from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import { useStudio } from "../../state/studio";
import type {
  ConnectionProfile,
  DatabaseObjectKind,
  DatabaseObjectSummary,
  TableInfo,
} from "../../ipc/generated";
import { orderedGroups } from "./GroupManager";
import {
  buildExplorerSchemas,
  EXPLORER_FILTER_CHIPS,
  flattenVisibleExplorerNodes,
  resolveTreeKeyAction,
  type ExplorerCategoryId,
  type ExplorerFilterChip,
} from "./explorer-tree";

const EMPTY_TABLES: TableInfo[] = [];
const EMPTY_OBJECTS: DatabaseObjectSummary[] = [];

function ObjectKindIcon({ kind }: { kind: DatabaseObjectKind }) {
  switch (kind) {
    case "table":
    case "partitionedTable":
      return <Table2 size={13} />;
    case "view":
      return <Eye size={13} />;
    case "materializedView":
      return <Layers size={13} />;
    case "function":
    case "procedure":
      return <Braces size={13} />;
    case "sequence":
      return <Hash size={13} />;
    case "trigger":
      return <Zap size={13} />;
    case "index":
      return <ListTree size={13} />;
  }
}

function CategoryIcon({ category }: { category: ExplorerCategoryId }) {
  switch (category) {
    case "tables":
      return <Table2 size={12} />;
    case "views":
      return <Eye size={12} />;
    case "materializedViews":
      return <Layers size={12} />;
    case "routines":
      return <Braces size={12} />;
    case "sequences":
      return <Hash size={12} />;
    case "triggers":
      return <Zap size={12} />;
    case "indexes":
      return <ListTree size={12} />;
  }
}

export function Sidebar({
  onNew,
  onConnect,
  onEdit,
  onSettings,
  onDisconnect,
  onOpenLibrary,
}: {
  onNew(groupId?: string): void;
  onConnect(profile: ConnectionProfile): void;
  onEdit(profile: ConnectionProfile): void;
  onSettings(): void;
  onDisconnect(profile: ConnectionProfile): void;
  onOpenLibrary?(tab: "saved" | "bookmarks" | "recents"): void;
}) {
  const state = useStudio(
      useShallow(
        ({
          bootstrap,
          sessions,
          catalog,
          objects,
          savedQueries,
          bookmarks,
          selectedConnectionId,
          selectConnection,
          refreshCatalog,
          notify,
          openQuery,
          openUtility,
          openObject,
        }) => ({
          bootstrap,
          sessions,
          catalog,
          objects,
          savedQueries,
          bookmarks,
          selectedConnectionId,
          selectConnection,
          refreshCatalog,
          notify,
          openQuery,
          openUtility,
          openObject,
        }),
      ),
    ),
    [search, setSearch] = useState(""),
    [filter, setFilter] = useState<ExplorerFilterChip>("all"),
    [collapsed, setCollapsed] = useState(new Set<string>()),
    [schemas, setSchemas] = useState(new Set<string>()),
    [collapsedCategories, setCollapsedCategories] = useState(new Set<string>()),
    [collapsedDuringSearch, setCollapsedDuringSearch] = useState(new Set<string>()),
    treeRef = useRef<HTMLDivElement>(null);

  const selected = state.bootstrap?.profiles.find(
    (profile) => profile.id === state.selectedConnectionId,
  );
  const selectedSession = selected ? state.sessions[selected.id] : undefined;
  const ordered = useMemo(
    () => orderedGroups(state.bootstrap?.groups ?? []),
    [state.bootstrap?.groups],
  );
  const hiddenGroups = new Set<string>();
  for (const { group } of ordered)
    if (group.parentId && (collapsed.has(group.parentId) || hiddenGroups.has(group.parentId)))
      hiddenGroups.add(group.id);

  const tables = selected ? (state.catalog[selected.id] ?? EMPTY_TABLES) : EMPTY_TABLES;
  const objects = selected ? (state.objects[selected.id] ?? EMPTY_OBJECTS) : EMPTY_OBJECTS;
  const isSearching = Boolean(search.trim());

  const schemaGroups = useMemo(
    () => buildExplorerSchemas(tables, objects, filter, search),
    [tables, objects, filter, search],
  );

  const isSchemaExpanded = (name: string) =>
    isSearching ? !collapsedDuringSearch.has(`schema:${name}`) : schemas.has(name);
  const isCategoryExpanded = (catKey: string) =>
    isSearching ? !collapsedDuringSearch.has(`cat:${catKey}`) : !collapsedCategories.has(catKey);

  const visibleNodes = useMemo(
    () =>
      flattenVisibleExplorerNodes(
        schemaGroups,
        (name) => (isSearching ? !collapsedDuringSearch.has(`schema:${name}`) : schemas.has(name)),
        (catKey) =>
          isSearching
            ? !collapsedDuringSearch.has(`cat:${catKey}`)
            : !collapsedCategories.has(catKey),
      ),
    [schemaGroups, isSearching, collapsedDuringSearch, schemas, collapsedCategories],
  );

  const setSchemaOpen = (name: string, open: boolean) => {
    if (isSearching) {
      setCollapsedDuringSearch((old) => {
        const next = new Set(old);
        if (open) next.delete(`schema:${name}`);
        else next.add(`schema:${name}`);
        return next;
      });
      return;
    }
    setSchemas((old) => {
      const next = new Set(old);
      if (open) next.add(name);
      else next.delete(name);
      return next;
    });
  };

  const setCategoryOpen = (catKey: string, open: boolean) => {
    if (isSearching) {
      setCollapsedDuringSearch((old) => {
        const next = new Set(old);
        if (open) next.delete(`cat:${catKey}`);
        else next.add(`cat:${catKey}`);
        return next;
      });
      return;
    }
    setCollapsedCategories((old) => {
      const next = new Set(old);
      if (open) next.delete(catKey);
      else next.add(catKey);
      return next;
    });
  };

  const toggleGroup = (id: string) =>
    setCollapsed((old) => {
      const next = new Set(old);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const handleTreeKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = (event.target as HTMLElement | null)?.closest<HTMLButtonElement>(
      "[data-tree-node]",
    );
    const currentId = target?.dataset.treeNode;
    if (!currentId) return;
    const action = resolveTreeKeyAction(visibleNodes, currentId, event.key);
    if (!action) return;
    event.preventDefault();
    switch (action.type) {
      case "focus": {
        const buttons = treeRef.current?.querySelectorAll<HTMLButtonElement>("[data-tree-node]");
        if (!buttons) return;
        for (const button of buttons) {
          if (button.dataset.treeNode === action.nodeId) {
            button.focus();
            break;
          }
        }
        break;
      }
      case "expandSchema":
        setSchemaOpen(action.schema, true);
        break;
      case "collapseSchema":
        setSchemaOpen(action.schema, false);
        break;
      case "expandCategory":
        setCategoryOpen(action.categoryKey, true);
        break;
      case "collapseCategory":
        setCategoryOpen(action.categoryKey, false);
        break;
    }
  };

  return (
    <aside className="sidebar" aria-label="Connections and database explorer">
      <div className="sidebar-brand">
        <img src="/joinery.svg" width={25} height={25} alt="" />
        <span>
          joinery<span className="brand-period">.</span>
        </span>
        <span className="version-tag">STUDIO</span>
      </div>
      <div className="sidebar-section-title">
        <span>CONNECTIONS</span>
        <button
          className="icon-button"
          aria-label="New connection"
          title="New connection"
          onClick={() => onNew()}
        >
          <Plus size={16} />
        </button>
      </div>
      <div className="connection-groups">
        {ordered
          .filter(({ group }) => !hiddenGroups.has(group.id))
          .map(({ group, depth }) => (
            <div key={group.id} className="connection-group" style={{ marginLeft: depth * 12 }}>
              <div className="group-heading">
                <button
                  className="group-toggle"
                  onClick={() => toggleGroup(group.id)}
                  aria-expanded={!collapsed.has(group.id)}
                >
                  {collapsed.has(group.id) ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
                  <span className={`environment-dot ${group.environment}`} />
                  {group.name}
                </button>
                <button
                  className="group-add icon-button"
                  aria-label={`Add ${group.name} connection`}
                  onClick={() => onNew(group.id)}
                >
                  <Plus size={12} />
                </button>
              </div>
              {!collapsed.has(group.id) && (
                <div className="group-children">
                  {state.bootstrap?.profiles
                    .filter((profile) => profile.groupId === group.id)
                    .map((profile) => (
                      <div
                        className={`connection-row ${state.selectedConnectionId === profile.id ? "selected" : ""}`}
                        key={profile.id}
                      >
                        <button
                          className="connection-select"
                          aria-current={
                            state.selectedConnectionId === profile.id ? "true" : undefined
                          }
                          title={`${profile.username}@${profile.host}/${profile.database}`}
                          onClick={() => {
                            state.selectConnection(profile.id);
                            if (!state.sessions[profile.id]) onConnect(profile);
                          }}
                        >
                          <Database size={14} />
                          <span>{profile.name}</span>
                          <i
                            className={state.sessions[profile.id] ? "online-dot" : "offline-dot"}
                          />
                        </button>
                        <div className="connection-actions">
                          {state.sessions[profile.id] && (
                            <button
                              className="icon-button"
                              aria-label={`Disconnect ${profile.name}`}
                              onClick={() => onDisconnect(profile)}
                            >
                              <Unplug size={12} />
                            </button>
                          )}
                          <button
                            className="icon-button"
                            aria-label={`Edit ${profile.name}`}
                            onClick={() => onEdit(profile)}
                          >
                            <Pencil size={12} />
                          </button>
                        </div>
                      </div>
                    ))}
                  {!state.bootstrap?.profiles.some((profile) => profile.groupId === group.id) && (
                    <span className="empty-group">No connections</span>
                  )}
                </div>
              )}
            </div>
          ))}
      </div>
      <div className="sidebar-divider" />
      <div className="sidebar-section-title">
        <span>DATABASE EXPLORER</span>
        {selected && selectedSession && (
          <button
            className="icon-button"
            title="Refresh schema"
            aria-label="Refresh schema"
            onClick={() =>
              void state
                .refreshCatalog(selected.id)
                .catch((error) => state.notify(String(error.message ?? error)))
            }
          >
            <Settings2 size={14} />
          </button>
        )}
      </div>
      <div className="schema-search">
        <Search size={13} />
        <input
          aria-label="Search tables"
          placeholder="Find an object…"
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            setCollapsedDuringSearch(new Set());
          }}
        />
        <kbd>⌕</kbd>
      </div>
      {selected && selectedSession && selected.engine !== "redis" && (
        <div className="explorer-filters" role="group" aria-label="Object kind filter">
          {EXPLORER_FILTER_CHIPS.map((chip) => (
            <button
              key={chip.id}
              type="button"
              className={`explorer-filter-chip ${filter === chip.id ? "active" : ""}`}
              aria-pressed={filter === chip.id}
              onClick={() => setFilter(chip.id)}
            >
              {chip.label}
            </button>
          ))}
        </div>
      )}
      <div className="schema-tree" ref={treeRef} onKeyDown={handleTreeKeyDown}>
        {!selected || !selectedSession ? (
          <div className="explorer-empty">
            <Database size={23} />
            <span>
              Connect a database
              <br />
              to explore its schema.
            </span>
          </div>
        ) : schemaGroups.length === 0 ? (
          <div className="explorer-empty">
            <Database size={21} />
            <span>
              {isSearching || filter !== "all"
                ? "No matching database objects."
                : selected.engine === "redis"
                  ? "Open SQL Studio to scan Redis keys."
                  : "No readable schema objects found."}
            </span>
          </div>
        ) : (
          schemaGroups.map((group) => {
            const schemaOpen = isSchemaExpanded(group.schema);
            return (
              <div className="schema-group" key={group.schema}>
                <button
                  type="button"
                  data-tree-node={`schema:${group.schema}`}
                  className="schema-heading"
                  aria-expanded={schemaOpen}
                  onClick={() => setSchemaOpen(group.schema, !schemaOpen)}
                >
                  {schemaOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                  <Database size={13} />
                  <span>{group.schema}</span>
                  <span className="schema-count">{group.totalCount}</span>
                </button>
                {schemaOpen &&
                  group.categories.map((cat) => {
                    const catOpen = isCategoryExpanded(cat.key);
                    return (
                      <div className="explorer-category" key={cat.key}>
                        <button
                          type="button"
                          data-tree-node={`cat:${cat.key}`}
                          className="explorer-category-heading"
                          aria-expanded={catOpen}
                          onClick={() => setCategoryOpen(cat.key, !catOpen)}
                        >
                          {catOpen ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
                          <CategoryIcon category={cat.id} />
                          <span>{cat.label}</span>
                          <span className="schema-count">{cat.items.length}</span>
                        </button>
                        {catOpen &&
                          cat.items.slice(0, 200).map((item) => (
                            <button
                              type="button"
                              key={item.key}
                              data-tree-node={`item:${item.key}`}
                              className="table-link"
                              title={item.tooltip}
                              onClick={() =>
                                void state
                                  .openObject(selected.id, item.object)
                                  .catch((error) => state.notify(String(error.message ?? error)))
                              }
                            >
                              <ObjectKindIcon kind={item.object.kind} />
                              <span>{item.primaryLabel}</span>
                              {item.secondaryLabel && (
                                <em className="explorer-item-badge" aria-hidden="true">
                                  {item.secondaryLabel}
                                </em>
                              )}
                            </button>
                          ))}
                        {catOpen && cat.items.length > 200 && (
                          <div className="tree-limit">
                            Search to narrow {cat.items.length} {cat.label.toLowerCase()}
                          </div>
                        )}
                      </div>
                    );
                  })}
              </div>
            );
          })
        )}
      </div>
      <nav className="sidebar-tools">
        <button onClick={() => state.openQuery()}>
          <SquareTerminal size={15} />
          SQL Studio<span>⌘ T</span>
        </button>
        {onOpenLibrary && (
          <>
            <button onClick={() => onOpenLibrary("saved")}>
              <Bookmark size={15} />
              Saved queries
              {state.savedQueries.length > 0 && <span>{state.savedQueries.length}</span>}
            </button>
            <button onClick={() => onOpenLibrary("bookmarks")}>
              <Star size={15} />
              Bookmarks
              {state.bookmarks.length > 0 && <span>{state.bookmarks.length}</span>}
            </button>
          </>
        )}
        <button onClick={() => state.openUtility("history")}>
          <History size={15} />
          Query history
        </button>
        <button
          onClick={() => {
            if (selected && selectedSession) state.openUtility("processes");
            else state.notify("Connect to a database to inspect processes.");
          }}
        >
          <Activity size={15} />
          Processes
        </button>
      </nav>
      <button className="sidebar-settings" onClick={onSettings}>
        <Settings2 size={15} />
        Preferences<span>⌘ ,</span>
      </button>
    </aside>
  );
}
