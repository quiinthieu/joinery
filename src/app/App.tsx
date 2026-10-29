import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  Activity,
  Check,
  Code2,
  Command,
  Database,
  FileCode2,
  Gauge,
  History,
  Keyboard,
  LockKeyhole,
  PanelLeftClose,
  PanelLeftOpen,
  PanelRight,
  Plus,
  Search,
  Wifi,
  X,
  ChevronRight,
} from "lucide-react";
import { Sidebar } from "../features/connections/Sidebar";
import { ConnectionDialog, ConnectDialog } from "../features/connections/ConnectionDialog";
import { Preferences } from "../features/studio/Preferences";
import { HistoryView, ProcessesView } from "../features/studio/UtilityViews";
import { ReviewDialog } from "../features/safety/ReviewDialog";
import { ValueDialog } from "../features/grid/ValueDialog";
import { Modal, InlineError } from "../components/ui/Modal";
import { tabNavigation } from "../components/ui/tabs";
import { client, messageOf } from "../ipc/client";
import type {
  ChangePreview,
  ConnectionProfile,
  PreparedPlan,
  ProcessInfo,
  ResourceUsage,
} from "../ipc/generated";
import { useStudio, beginEventBatch, querySessionForTab, type StudioTab } from "../state/studio";
import { flushWorkspace, watchWorkspace } from "../state/workspace";
import { dirtyStore } from "../data/dirty-store";
import { resultCache } from "../data/result-cache";
import type { CellTarget } from "../features/grid/DatabaseGrid";
import type { EditorHandle } from "../features/studio/SqlEditor";
import { Workbench, Welcome } from "./Workbench";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type Dialog =
  | { kind: "connection"; profile?: ConnectionProfile; groupId?: string }
  | { kind: "connect"; profile: ConnectionProfile; tabId?: string }
  | { kind: "settings" }
  | { kind: "palette" }
  | { kind: "value"; target: CellTarget }
  | {
      kind: "review";
      plan: PreparedPlan;
      preview?: ChangePreview;
      tabId: string;
      profile?: ConnectionProfile;
      onSuccess?(): void;
    }
  | { kind: "confirm"; title: string; description: string; action(): void | Promise<void> };
export function dirtyFor(tab: StudioTab) {
  return tab.pages.flatMap((page) => dirtyStore.forResult(page.result.id));
}

export function App() {
  const state = useStudio(),
    [dialog, setDialog] = useState<Dialog | null>(null),
    [selectedCell, setSelectedCell] = useState<CellTarget | null>(null),
    [usage, setUsage] = useState<ResourceUsage | null>(null);
  const editor = useRef<EditorHandle>(null);
  useSyncExternalStore(dirtyStore.subscribe, dirtyStore.snapshot);
  const current = state.tabs.find((tab) => tab.id === state.activeTabId);
  const profile = state.bootstrap?.profiles.find(
    (p) => p.id === (current?.connectionId ?? state.selectedConnectionId),
  );
  const session =
    current?.kind === "query"
      ? querySessionForTab(current, state.sessions)
      : profile
        ? state.sessions[profile.id]
        : null;
  const references = useRef({ current, dialog, profile });
  references.current = { current, dialog, profile };
  useEffect(() => {
    void state.initialize();
  }, []);
  useEffect(watchWorkspace, []);
  useLayoutEffect(() => {
    document.documentElement.dataset.theme = state.theme;
    document.documentElement.dataset.density = state.density;
  }, [state.theme, state.density]);
  useEffect(() => {
    const hasPendingWork = () =>
      useStudio
        .getState()
        .tabs.some(
          (tab) =>
            dirtyFor(tab).length ||
            ["queued", "running"].includes(tab.status) ||
            (tab.querySession && !tab.querySessionClosed),
        );
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (hasPendingWork()) {
        event.preventDefault();
        event.returnValue = "";
      }
    };
    if (!isTauri()) {
      window.addEventListener("beforeunload", beforeUnload);
      return () => window.removeEventListener("beforeunload", beforeUnload);
    }
    const windowHandle = getCurrentWindow();
    const closeSafely = async () => {
      await flushWorkspace();
      await windowHandle.destroy();
    };
    const unsubscribe = windowHandle.onCloseRequested((event) => {
      event.preventDefault();
      if (!hasPendingWork()) {
        void closeSafely().catch((error) => useStudio.getState().notify(messageOf(error)));
        return;
      }
      setDialog({
        kind: "confirm",
        title: "Close Joinery?",
        description:
          "Pending edits will be discarded, running queries cancelled, and open SQL transactions rolled back. Completed writes are not undone.",
        action: closeSafely,
      });
    });
    const quit = windowHandle.listen<number>("joinery:quit-requested", (event) => {
      void client.call("acknowledge_quit", { generation: event.payload });
      if (!hasPendingWork()) {
        void closeSafely().catch((error) => useStudio.getState().notify(messageOf(error)));
        return;
      }
      setDialog({
        kind: "confirm",
        title: "Close Joinery?",
        description:
          "Pending edits will be discarded, running queries cancelled, and open SQL transactions rolled back. Completed writes are not undone.",
        action: closeSafely,
      });
    });
    return () => {
      void unsubscribe.then((off) => off());
      void quit.then((off) => off());
    };
  }, []);
  useEffect(() => {
    const refresh = () => {
      if (document.visibilityState !== "hidden")
        void client
          .call<ResourceUsage>("resource_usage")
          .then(setUsage)
          .catch(() => {});
    };
    refresh();
    const timer = setInterval(refresh, 10000);
    return () => clearInterval(timer);
  }, []);
  useEffect(() => {
    if (!session) return;
    const timer = setInterval(() => {
      if (document.visibilityState !== "hidden")
        void client
          .call<number>("health", { sessionId: session.id })
          .then((latencyMs) => {
            const state = useStudio.getState();
            const owner = state.tabs.find(
              (t) => t.querySession?.id === session.id && !t.querySessionClosed,
            );
            if (owner) state.updateTab(owner.id, { querySession: { ...session, latencyMs } });
            else {
              const latest = state.sessions[session.connectionId];
              if (latest?.id === session.id) state.setSession({ ...latest, latencyMs });
            }
          })
          .catch(() => useStudio.getState().retireSession(session.id));
    }, 30000);
    return () => clearInterval(timer);
  }, [session?.id]);

  async function executePrepared(
    tabId: string,
    plan: PreparedPlan,
    approval: string | null,
    onSuccess?: () => void,
  ) {
    const active = useStudio.getState().tabs.find((tab) => tab.id === tabId);
    if (!active) return;
    if (
      active.kind === "query" &&
      querySessionForTab(active, useStudio.getState().sessions)?.id !== plan.sessionId
    ) {
      state.notify("The query session changed. Run the statement again in its current session.");
      return;
    }
    if (active.kind === "query") {
      for (const previous of active.results) {
        resultCache.release(previous.id);
        await client.call("release_result", { resultId: previous.id });
      }
      state.updateTab(tabId, { results: [], activeResultId: null });
    }
    const revision = active.revision + 1;
    state.updateTab(tabId, {
      status: "queued",
      error: null,
      elapsedMs: 0,
      metrics: {},
      revision,
      dataSessionId: plan.sessionId,
    });
    const batch = beginEventBatch(tabId);
    try {
      const runId = await client.execute(plan.id, approval, (event) => {
        batch(event);
        if (event.type === "finished" && event.status === "completed") onSuccess?.();
      });
      const latest = useStudio.getState().tabs.find((tab) => tab.id === tabId);
      if (!latest || latest.revision !== revision)
        void client.call("cancel_run", { runId }).catch(() => {});
      else if (latest.status === "queued") state.updateTab(tabId, { runId });
    } catch (error) {
      if (useStudio.getState().tabs.find((tab) => tab.id === tabId)?.revision === revision)
        state.updateTab(tabId, {
          status: "failed",
          error: { code: "database", message: messageOf(error) },
        });
    }
  }
  async function runSql(sql?: string) {
    const active = references.current.current;
    if (!active || active.kind !== "query" || ["queued", "running"].includes(active.status)) return;
    const selected = querySessionForTab(active, useStudio.getState().sessions);
    if (!selected) {
      state.notify("Connect this query tab to a database first.");
      return;
    }
    try {
      const plan = await client.call<PreparedPlan>("prepare_execution", {
        sessionId: selected.id,
        sql: sql ?? editor.current?.selection() ?? active.sql,
      });
      const target = state.bootstrap?.profiles.find((p) => p.id === active.connectionId);
      if (plan.requiresApproval)
        setDialog({ kind: "review", plan, tabId: active.id, profile: target });
      else await executePrepared(active.id, plan, null);
    } catch (error) {
      state.updateTab(active.id, {
        status: "failed",
        error: { code: "database", message: messageOf(error) },
      });
    }
  }
  async function reviewEdits() {
    const active = references.current.current;
    if (!active || active.kind !== "table") return;
    const session = active.connectionId ? useStudio.getState().sessions[active.connectionId] : null;
    if (!session) {
      state.notify("Reconnect and refresh the table before applying edits.");
      return;
    }
    if (active.dataSessionId !== session.id) {
      state.notify("This page belongs to an earlier session. Refresh the table before editing.");
      return;
    }
    const changes = dirtyFor(active);
    if (!changes.length) {
      state.notify("There are no pending edits.");
      return;
    }
    if (active.status === "outcomeUnknown") {
      state.notify("Verify the previous commit, then refresh before editing again.");
      return;
    }
    try {
      const grouped = new Map<string, typeof changes>();
      for (const change of changes)
        grouped.set(change.resultId, [...(grouped.get(change.resultId) ?? []), change]);
      const preview = await client.call<ChangePreview>("prepare_changes_batch", {
        sessionId: session.id,
        inputs: [...grouped].map(([resultId, changes]) => ({
          resultId,
          changes: changes.map(({ row, column, value }) => ({ row, column, value })),
        })),
      });
      setDialog({
        kind: "review",
        plan: preview.plan,
        preview,
        profile,
        tabId: active.id,
        onSuccess: () => {
          for (const page of active.pages) dirtyStore.clear(page.result.id);
          setSelectedCell(null);
          void state.refreshTable(active.id);
          state.notify("Changes committed");
        },
      });
    } catch (error) {
      state.notify(messageOf(error));
    }
  }
  function confirmDiscard(active: StudioTab, action: () => void) {
    if (
      dirtyFor(active).length ||
      ["queued", "running"].includes(active.status) ||
      (active.querySession && !active.querySessionClosed)
    )
      setDialog({
        kind: "confirm",
        title: "Close this workspace?",
        description:
          "Pending edits will be discarded and running queries cancelled. Closing an isolated session rolls back its open transactions. Completed writes are not undone.",
        action,
      });
    else action();
  }
  function closeWorkspace(id: string) {
    useStudio.getState().closeTab(id);
    requestAnimationFrame(() =>
      document
        .querySelector<HTMLButtonElement>('.workspace-tabs [role="tab"][aria-selected="true"]')
        ?.focus(),
    );
  }
  function disconnectProfile(profile: ConnectionProfile) {
    const action = async () => {
      const session = useStudio.getState().sessions[profile.id];
      const owned = useStudio
        .getState()
        .tabs.filter(
          (t) => t.connectionId === profile.id && t.querySession && !t.querySessionClosed,
        )
        .map((t) => t.querySession!.id);
      await Promise.all(
        [...new Set([...(session ? [session.id] : []), ...owned])].map((sessionId) =>
          client.call("disconnect", { sessionId }),
        ),
      );
      state.resetConnectionResults(profile.id);
    };
    if (
      state.tabs.some(
        (tab) =>
          tab.connectionId === profile.id &&
          (dirtyFor(tab).length ||
            ["queued", "running"].includes(tab.status) ||
            (tab.querySession && !tab.querySessionClosed)),
      )
    )
      setDialog({
        kind: "confirm",
        title: `Disconnect ${profile.name}?`,
        description:
          "This closes all its sessions and results, discards pending edits, cancels running queries, and rolls back open transactions.",
        action,
      });
    else void action().catch((error) => state.notify(messageOf(error)));
  }
  const actions = useRef({ runSql, reviewEdits });
  actions.current = { runSql, reviewEdits };
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || references.current.dialog) return;
      const key = event.key.toLowerCase(),
        active = references.current.current;
      if (key === "k") {
        event.preventDefault();
        setDialog({ kind: "palette" });
      } else if (key === "t") {
        event.preventDefault();
        useStudio.getState().openQuery();
      } else if (key === "w" && active) {
        event.preventDefault();
        confirmDiscard(active, () => closeWorkspace(active.id));
      } else if (key === "b") {
        event.preventDefault();
        useStudio.setState((state) => ({ sidebarVisible: !state.sidebarVisible }));
      } else if (key === "i") {
        event.preventDefault();
        useStudio.setState((state) => ({ inspectorVisible: !state.inspectorVisible }));
      } else if (key === ",") {
        event.preventDefault();
        setDialog({ kind: "settings" });
      } else if (key === "s") {
        event.preventDefault();
        void actions.current.reviewEdits();
      } else if (key === "." && active?.runId) {
        event.preventDefault();
        void client
          .call("cancel_run", { runId: active.runId })
          .catch((error) => useStudio.getState().notify(messageOf(error)));
      } else if (key === "enter" && event.shiftKey && active?.kind === "query") {
        event.preventDefault();
        void actions.current.runSql(active.sql);
      } else if (key === "f" && event.shiftKey) {
        event.preventDefault();
        editor.current?.format();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);
  async function cancelProcess(process: ProcessInfo) {
    if (!current || !session) return;
    try {
      const plan = await client.call<PreparedPlan>("prepare_process_cancel", {
        sessionId: session.id,
        id: process.id,
        identity: process.identity,
      });
      setDialog({ kind: "review", plan, tabId: current.id, profile });
    } catch (error) {
      state.notify(messageOf(error));
    }
  }

  return (
    <div className={`app-shell ${!state.sidebarVisible ? "sidebar-hidden" : ""}`}>
      {state.sidebarVisible && (
        <Sidebar
          onNew={(groupId) => setDialog({ kind: "connection", groupId })}
          onConnect={(profile) => setDialog({ kind: "connect", profile })}
          onEdit={(profile) => {
            if (state.tabs.some((tab) => tab.connectionId === profile.id && dirtyFor(tab).length))
              setDialog({
                kind: "confirm",
                title: "Discard pending edits?",
                description:
                  "Editing this connection will reconnect it and close its existing results.",
                action: () => setDialog({ kind: "connection", profile }),
              });
            else setDialog({ kind: "connection", profile });
          }}
          onSettings={() => setDialog({ kind: "settings" })}
          onDisconnect={disconnectProfile}
        />
      )}
      <div className="main-shell">
        <header className="workspace-header">
          <div className="workspace-heading">
            <button
              className="icon-button"
              aria-label="Toggle sidebar"
              aria-expanded={state.sidebarVisible}
              onClick={() => useStudio.setState({ sidebarVisible: !state.sidebarVisible })}
            >
              {state.sidebarVisible ? <PanelLeftClose size={17} /> : <PanelLeftOpen size={17} />}
            </button>
            <span className="workspace-label">Workspace</span>
            <ChevronRight size={12} />
            {profile ? (
              <>
                <Database size={14} />
                <span>{profile.database}</span>
                <span className={`environment ${profile.environment}`}>{profile.environment}</span>
              </>
            ) : (
              <span className="muted">Overview</span>
            )}
          </div>
          <div className="workspace-tools">
            <button className="command-trigger" onClick={() => setDialog({ kind: "palette" })}>
              <Search size={13} />
              <span>Jump to anything</span>
              <kbd>⌘ K</kbd>
            </button>
            <button
              className={`icon-button ${state.inspectorVisible ? "active" : ""}`}
              aria-label="Toggle inspector"
              aria-expanded={state.inspectorVisible}
              onClick={() => useStudio.setState({ inspectorVisible: !state.inspectorVisible })}
            >
              <PanelRight size={16} />
            </button>
          </div>
        </header>
        <div
          className="workspace-tabs"
          role="tablist"
          aria-label="Workspaces"
          onKeyDown={tabNavigation}
        >
          {state.tabs.map((tab) => (
            <div className={`workspace-tab ${tab.id === current?.id ? "active" : ""}`} key={tab.id}>
              <button
                role="tab"
                id={`workspace-tab-${tab.id}`}
                aria-controls="workspace-panel"
                aria-selected={tab.id === current?.id}
                tabIndex={tab.id === current?.id ? 0 : -1}
                onClick={() => {
                  useStudio.setState({ activeTabId: tab.id });
                  setSelectedCell(null);
                }}
              >
                {tab.kind === "query" ? (
                  <FileCode2 size={13} />
                ) : tab.kind === "table" ? (
                  <Database size={13} />
                ) : tab.kind === "history" ? (
                  <History size={13} />
                ) : tab.kind === "processes" ? (
                  <Activity size={13} />
                ) : (
                  <Gauge size={13} />
                )}
                <span>{tab.title}</span>
                {tab.kind === "table" && dirtyFor(tab).length > 0 && <i className="dirty-dot" />}
                {["queued", "running"].includes(tab.status) && (
                  <span className="tab-running">●</span>
                )}
              </button>
              <button
                className="tab-close"
                aria-label={`Close ${tab.title}`}
                onClick={() => confirmDiscard(tab, () => closeWorkspace(tab.id))}
              >
                <X size={12} />
              </button>
            </div>
          ))}
          <button
            className="icon-button new-query"
            aria-label="New query tab"
            title="New query tab · ⌘ T"
            onClick={() => state.openQuery()}
          >
            <Plus size={15} />
          </button>
        </div>
        <div className="workspace-body">
          <section
            className="workspace-content"
            id={current ? "workspace-panel" : undefined}
            role={current ? "tabpanel" : undefined}
            aria-labelledby={current ? `workspace-tab-${current.id}` : undefined}
          >
            {!current ? (
              <Welcome
                onConnect={() => setDialog({ kind: "connection" })}
                onQuery={() => state.openQuery()}
                onBenchmark={() => state.openUtility("benchmark")}
              />
            ) : current.kind === "history" ? (
              <HistoryView />
            ) : current.kind === "processes" ? (
              <ProcessesView tab={current} onCancel={(process) => void cancelProcess(process)} />
            ) : (
              <Workbench
                tab={current}
                profile={profile}
                session={session ?? undefined}
                editorRef={editor}
                onRun={(sql) => void runSql(sql)}
                onReview={() => void reviewEdits()}
                onConfirm={(title, description, action) =>
                  setDialog({ kind: "confirm", title, description, action })
                }
                onSelect={setSelectedCell}
                onOpenValue={(target) => setDialog({ kind: "value", target })}
                onNewSession={() =>
                  profile && setDialog({ kind: "connect", profile, tabId: current.id })
                }
              />
            )}
          </section>
          {state.inspectorVisible && (
            <aside
              className="inspector"
              aria-label="Selected cell inspector"
              aria-live="polite"
              aria-atomic="true"
            >
              <div className="inspector-title">
                INSPECTOR
                <button
                  className="icon-button"
                  aria-label="Close inspector"
                  onClick={() => useStudio.setState({ inspectorVisible: false })}
                >
                  <X size={13} />
                </button>
              </div>
              {selectedCell ? (
                <>
                  <h3>{selectedCell.column.name}</h3>
                  <dl>
                    <dt>Type</dt>
                    <dd>{selectedCell.column.databaseType}</dd>
                    <dt>Nullable</dt>
                    <dd>{selectedCell.column.nullable ? "Yes" : "No"}</dd>
                    <dt>Key</dt>
                    <dd>
                      {selectedCell.column.primaryKey
                        ? "Primary key"
                        : selectedCell.column.foreignKey
                          ? "Foreign key"
                          : "—"}
                    </dd>
                  </dl>
                  <div className="inspector-value">
                    {selectedCell.value === null ? (
                      <em>NULL</em>
                    ) : (
                      <pre>{selectedCell.value.slice(0, 4000)}</pre>
                    )}
                  </div>
                  <button
                    className="button ghost small"
                    onClick={() => setDialog({ kind: "value", target: selectedCell })}
                  >
                    {selectedCell.column.editable ? "Edit value" : "View full value"}
                  </button>
                </>
              ) : (
                <div className="inspector-empty">
                  <Database size={23} />
                  <p>Select a cell to see its value and type.</p>
                </div>
              )}
            </aside>
          )}
        </div>
        <footer className="status-bar">
          <div>
            {session ? (
              <>
                <span className="online-dot" />
                <span>{profile?.name}</span>
                <span className="status-separator">/</span>
                <span>{session.serverVersion.split(" ").slice(0, 2).join(" ")}</span>
                <span className="latency">
                  <Wifi size={11} />
                  {session.latencyMs} ms
                </span>
                {session.encrypted && <LockKeyhole size={11} />}
              </>
            ) : (
              <>
                <span className="offline-dot" />
                <span>{profile ? `${profile.name} · disconnected` : "No active connection"}</span>
                {profile && (
                  <button onClick={() => setDialog({ kind: "connect", profile })}>Connect</button>
                )}
              </>
            )}
          </div>
          <div>
            <span>
              {usage
                ? `${(usage.retainedBytes / 1024 / 1024).toFixed(1)} MB results`
                : "Memory only"}
            </span>
            <span className="status-separator">·</span>
            <button onClick={() => setDialog({ kind: "palette" })}>
              <Keyboard size={12} />
              Shortcuts
            </button>
          </div>
        </footer>
      </div>
      {state.error && (
        <div className="startup-error" role="alert">
          <InlineError message={state.error} />
          <button className="button ghost" onClick={() => void state.initialize()}>
            Retry
          </button>
        </div>
      )}
      {state.toast && (
        <div className="toast" role="status">
          <Check size={14} />
          <span>{state.toast}</span>
          <button
            className="icon-button"
            aria-label="Dismiss notification"
            onClick={() => useStudio.setState({ toast: null })}
          >
            <X size={12} />
          </button>
        </div>
      )}
      {dialog?.kind === "connection" && (
        <ConnectionDialog
          profile={dialog.profile}
          groupId={dialog.groupId}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "connect" && (
        <ConnectDialog
          profile={dialog.profile}
          onClose={() => setDialog(null)}
          onConnected={
            dialog.tabId
              ? (session) => {
                  if (state.bindQuerySession(dialog.tabId!, session))
                    state.notify("Isolated SQL session connected");
                }
              : undefined
          }
        />
      )}
      {dialog?.kind === "settings" && <Preferences onClose={() => setDialog(null)} />}
      {dialog?.kind === "review" && (
        <ReviewDialog
          plan={dialog.plan}
          preview={dialog.preview}
          profile={dialog.profile}
          onExecute={async (approval) => {
            await executePrepared(dialog.tabId, dialog.plan, approval, dialog.onSuccess);
          }}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "value" && (
        <ValueDialog target={dialog.target} onClose={() => setDialog(null)} />
      )}
      {dialog?.kind === "confirm" && (
        <Confirmation
          title={dialog.title}
          description={dialog.description}
          onConfirm={dialog.action}
          onClose={() => setDialog((current) => (current === dialog ? null : current))}
        />
      )}
      {dialog?.kind === "palette" && (
        <CommandPalette
          onClose={() => setDialog(null)}
          commands={[
            {
              label: "New query tab",
              shortcut: "⌘ T",
              icon: FileCode2,
              action: () => state.openQuery(),
            },
            {
              label: "New database connection",
              shortcut: "",
              icon: Database,
              action: () => setDialog({ kind: "connection" }),
            },
            {
              label: "Query history",
              shortcut: "",
              icon: History,
              action: () => state.openUtility("history"),
            },
            {
              label: "Grid benchmark",
              shortcut: "1M rows",
              icon: Gauge,
              action: () => state.openUtility("benchmark"),
            },
            {
              label: "Preferences",
              shortcut: "⌘ ,",
              icon: Command,
              action: () => setDialog({ kind: "settings" }),
            },
          ]}
        />
      )}
    </div>
  );
}

function Confirmation({
  title,
  description,
  onConfirm,
  onClose,
}: {
  title: string;
  description: string;
  onConfirm(): void | Promise<void>;
  onClose(): void;
}) {
  const [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null);
  return (
    <Modal title={title} onClose={onClose} busy={busy}>
      <div className="modal-body">
        <p className="confirmation-description">{description}</p>
        <InlineError message={error} />
      </div>
      <footer className="modal-footer">
        <button className="button ghost" disabled={busy} onClick={onClose}>
          Keep working
        </button>
        <button
          className="button danger"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            void Promise.resolve(onConfirm())
              .then(onClose)
              .catch((error) => {
                setError(messageOf(error));
                setBusy(false);
              });
          }}
        >
          Continue
        </button>
      </footer>
    </Modal>
  );
}
function CommandPalette({
  commands,
  onClose,
}: {
  commands: { label: string; shortcut: string; icon: typeof Code2; action(): unknown }[];
  onClose(): void;
}) {
  const [search, setSearch] = useState("");
  const matches = commands.filter((command) =>
    command.label.toLowerCase().includes(search.toLowerCase()),
  );
  return (
    <Modal title="Command palette" onClose={onClose}>
      <div className="palette-search">
        <Search size={17} />
        <input
          autoFocus
          aria-label="Search commands"
          placeholder="What would you like to do?"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && matches[0]) {
              event.preventDefault();
              event.stopPropagation();
              onClose();
              matches[0].action();
            }
          }}
        />
      </div>
      <div className="palette-commands">
        {matches.map(({ label, shortcut, icon: Icon, action }) => (
          <button
            key={label}
            onClick={() => {
              onClose();
              action();
            }}
          >
            <Icon size={16} />
            <span>{label}</span>
            <kbd>{shortcut}</kbd>
          </button>
        ))}
        {!matches.length && <div className="grid-empty">No matching commands</div>}
      </div>
    </Modal>
  );
}
