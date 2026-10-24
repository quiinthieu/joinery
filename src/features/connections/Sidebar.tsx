import { useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  Database,
  Plus,
  Search,
  Settings2,
  Table2,
  SquareTerminal,
  History,
  Activity,
  Pencil,
  Unplug,
} from "lucide-react";
import { useStudio } from "../../state/studio";
import type { ConnectionProfile } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useShallow } from "zustand/react/shallow";
const EMPTY_TABLES: import("../../ipc/generated").TableInfo[] = [];

export function Sidebar({
  onNew,
  onConnect,
  onEdit,
  onSettings,
  onDisconnect,
}: {
  onNew(groupId?: string): void;
  onConnect(profile: ConnectionProfile): void;
  onEdit(profile: ConnectionProfile): void;
  onSettings(): void;
  onDisconnect(profile: ConnectionProfile): void;
}) {
  const state = useStudio(
      useShallow(
        ({
          bootstrap,
          sessions,
          catalog,
          selectedConnectionId,
          selectConnection,
          refreshCatalog,
          notify,
          openQuery,
          openUtility,
          openTable,
        }) => ({
          bootstrap,
          sessions,
          catalog,
          selectedConnectionId,
          selectConnection,
          refreshCatalog,
          notify,
          openQuery,
          openUtility,
          openTable,
        }),
      ),
    ),
    [search, setSearch] = useState(""),
    [collapsed, setCollapsed] = useState(new Set<string>()),
    [schemas, setSchemas] = useState(new Set<string>());
  const selected = state.bootstrap?.profiles.find(
    (profile) => profile.id === state.selectedConnectionId,
  );
  const tables = selected ? (state.catalog[selected.id] ?? EMPTY_TABLES) : EMPTY_TABLES;
  const grouped = useMemo(() => {
    const groups = new Map<string, typeof tables>();
    const needle = search.toLowerCase();
    for (const table of tables)
      if (`${table.table.schema}.${table.table.name}`.toLowerCase().includes(needle)) {
        const list = groups.get(table.table.schema) ?? [];
        list.push(table);
        groups.set(table.table.schema, list);
      }
    return groups;
  }, [tables, search]);
  const toggle = (id: string) =>
    setCollapsed((old) => {
      const next = new Set(old);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  return (
    <aside className="sidebar">
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
        {(state.bootstrap?.groups ?? []).map((group) => (
          <div key={group.id} className="connection-group">
            <div className="group-heading">
              <button
                className="group-toggle"
                onClick={() => toggle(group.id)}
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
                        title={`${profile.username}@${profile.host}/${profile.database}`}
                        onClick={() => {
                          state.selectConnection(profile.id);
                          if (!state.sessions[profile.id]) onConnect(profile);
                        }}
                      >
                        <Database size={14} />
                        <span>{profile.name}</span>
                        <i className={state.sessions[profile.id] ? "online-dot" : "offline-dot"} />
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
        {selected && state.sessions[selected.id] && (
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
          placeholder="Find a table…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <kbd>⌕</kbd>
      </div>
      <div className="schema-tree">
        {!selected || !state.sessions[selected.id] ? (
          <div className="explorer-empty">
            <Database size={23} />
            <span>
              Connect a database
              <br />
              to explore its schema.
            </span>
          </div>
        ) : (
          [...grouped].map(([name, entries]) => (
            <div className="schema-group" key={name}>
              <button
                className="schema-heading"
                onClick={() =>
                  setSchemas((old) => {
                    const next = new Set(old);
                    next.has(name) ? next.delete(name) : next.add(name);
                    return next;
                  })
                }
              >
                {schemas.has(name) || search ? (
                  <ChevronDown size={12} />
                ) : (
                  <ChevronRight size={12} />
                )}
                <Database size={13} />
                <span>{name}</span>
                <span className="schema-count">{entries.length}</span>
              </button>
              {(schemas.has(name) || search) &&
                entries.slice(0, 200).map(({ table }) => (
                  <button
                    key={table.name}
                    className="table-link"
                    onClick={() =>
                      void state
                        .openTable(selected.id, table)
                        .catch((error) => state.notify(String(error.message ?? error)))
                    }
                  >
                    <Table2 size={13} />
                    <span>{table.name}</span>
                  </button>
                ))}
              {(schemas.has(name) || search) && entries.length > 200 && (
                <div className="tree-limit">Search to narrow {entries.length} tables</div>
              )}
            </div>
          ))
        )}
      </div>
      <nav className="sidebar-tools">
        <button onClick={() => state.openQuery()}>
          <SquareTerminal size={15} />
          SQL Studio<span>⌘ T</span>
        </button>
        <button onClick={() => state.openUtility("history")}>
          <History size={15} />
          Query history
        </button>
        <button
          onClick={() => {
            if (selected && state.sessions[selected.id]) state.openUtility("processes");
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
