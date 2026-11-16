import { useEffect, useMemo, useState } from "react";
import { Camera, ExternalLink, History, RotateCcw, Search, Trash2 } from "lucide-react";
import type { LocalHistoryEntry } from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { InlineError, Modal } from "../../components/ui/Modal";
import { tabNavigation } from "../../components/ui/tabs";
import { useStudio, type StudioTab } from "../../state/studio";
import { computeLineDiff } from "./local-diff";

export function localHistoryTabKey(
  tab: Pick<StudioTab, "id" | "fileName" | "savedQueryId">,
): string {
  if (tab.fileName) return `file:${tab.fileName}`.slice(0, 160);
  if (tab.savedQueryId) return `saved:${tab.savedQueryId}`.slice(0, 160);
  return `tab:${tab.id}`.slice(0, 160);
}

function triggerBadge(trigger: LocalHistoryEntry["trigger"]): string {
  if (trigger === "executed") return "Executed";
  if (trigger === "saved") return "File saved";
  return "Snapshot";
}

export function LocalHistoryModal({ tab, onClose }: { tab: StudioTab | null; onClose(): void }) {
  const state = useStudio();
  const currentTabKey = tab && tab.kind === "query" ? localHistoryTabKey(tab) : null;
  const [scope, setScope] = useState<"tab" | "all">(currentTabKey ? "tab" : "all");
  const [entries, setEntries] = useState<LocalHistoryEntry[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const activeProfile = tab?.connectionId
    ? (state.bootstrap?.profiles.find((p) => p.id === tab.connectionId) ?? null)
    : null;
  const privacyBlocksSnapshot = Boolean(
    activeProfile && !activeProfile.storeQueryText && !activeProfile.storeDrafts,
  );

  async function reload(targetScope = scope) {
    setLoading(true);
    setError(null);
    try {
      const loaded = await client.call<LocalHistoryEntry[]>("local_history_entries", {
        tabKey: targetScope === "tab" && currentTabKey ? currentTabKey : null,
      });
      setEntries(loaded);
      setSelectedId((prev) =>
        prev && loaded.some((e) => e.id === prev) ? prev : (loaded[0]?.id ?? null),
      );
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void reload(scope);
  }, [scope, currentTabKey]);

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((e) => `${e.title} ${e.sql}`.toLowerCase().includes(q));
  }, [entries, filter]);

  const selected = useMemo(
    () => filtered.find((e) => e.id === selectedId) ?? filtered[0] ?? null,
    [filtered, selectedId],
  );

  const currentEditorSql = tab && tab.kind === "query" ? tab.sql : null;
  const diff = useMemo(
    () =>
      selected && currentEditorSql !== null
        ? computeLineDiff(selected.sql, currentEditorSql)
        : null,
    [currentEditorSql, selected],
  );

  async function captureManualSnapshot() {
    if (!tab || tab.kind !== "query" || !currentTabKey) return;
    if (!tab.sql.trim()) {
      setError("Write non-empty SQL in the active console before saving a snapshot.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const saved = await client.call<LocalHistoryEntry | null>("record_local_history", {
        input: {
          tabKey: currentTabKey,
          title: tab.title,
          connectionId: tab.connectionId,
          trigger: "snapshot",
          sql: tab.sql,
        },
      });
      if (!saved) {
        state.notify(
          "Snapshot skipped because this connection disables local SQL & draft storage.",
        );
      } else {
        await reload(scope);
        setSelectedId(saved.id);
      }
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="SQL Local History"
      subtitle="Inspect local SQL revisions, unified diffs against your active editor, and restore snapshots"
      wide
      busy={busy}
      onClose={onClose}
    >
      <div className="modal-body" style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div
          style={{
            display: "flex",
            flexWrap: "wrap",
            gap: 8,
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <div
            className="palette-scopes"
            role="tablist"
            aria-label="Local history scope"
            onKeyDown={tabNavigation}
          >
            {currentTabKey && (
              <button
                type="button"
                role="tab"
                aria-selected={scope === "tab"}
                tabIndex={scope === "tab" ? 0 : -1}
                className={scope === "tab" ? "active" : ""}
                onClick={() => setScope("tab")}
              >
                Current console ({tab?.title})
              </button>
            )}
            <button
              type="button"
              role="tab"
              aria-selected={scope === "all"}
              tabIndex={scope === "all" ? 0 : -1}
              className={scope === "all" ? "active" : ""}
              onClick={() => setScope("all")}
            >
              All SQL consoles & files
            </button>
          </div>

          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
            {tab?.kind === "query" && (
              <button
                type="button"
                className="button ghost small"
                disabled={busy || !tab.sql.trim() || privacyBlocksSnapshot}
                title={
                  privacyBlocksSnapshot
                    ? "Local SQL storage is disabled in this connection's privacy settings"
                    : "Save a manual revision checkpoint of the active editor"
                }
                onClick={() => void captureManualSnapshot()}
              >
                <Camera size={12} />
                Save snapshot
              </button>
            )}
            {entries.length > 0 && (
              <button
                type="button"
                className="button ghost small danger-text"
                disabled={busy}
                onClick={() => {
                  setBusy(true);
                  void client
                    .call("clear_local_history", {
                      tabKey: scope === "tab" && currentTabKey ? currentTabKey : null,
                    })
                    .then(() => reload(scope))
                    .catch((err) => setError(messageOf(err)))
                    .finally(() => setBusy(false));
                }}
              >
                <Trash2 size={12} />
                {scope === "tab" ? "Clear tab history" : "Clear all"}
              </button>
            )}
          </div>
        </div>

        <label className="diagram-filter" style={{ width: "100%" }}>
          <Search size={13} />
          <input
            aria-label="Filter local history"
            placeholder="Filter revisions by SQL text or title…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </label>

        {privacyBlocksSnapshot && scope === "tab" && (
          <div className="diagram-notice" role="status">
            <strong>{activeProfile?.name}</strong> disables both query-text and draft storage, so
            local SQL revisions are never written to disk for this connection.
          </div>
        )}

        <InlineError message={error} />

        {loading ? (
          <div className="grid-empty">Loading SQL revisions…</div>
        ) : filtered.length === 0 ? (
          <div className="grid-empty">
            <History size={18} />
            <span>
              No local SQL revisions recorded yet. Revisions are captured automatically when SQL
              executes or saves (when connection privacy permits), or via &ldquo;Save
              snapshot&rdquo;.
            </span>
          </div>
        ) : (
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "minmax(220px, 270px) 1fr",
              gap: 12,
              minHeight: 340,
              maxHeight: "56vh",
            }}
          >
            <div
              role="listbox"
              aria-label="SQL revisions"
              style={{
                border: "1px solid var(--border)",
                borderRadius: 6,
                overflowY: "auto",
                display: "flex",
                flexDirection: "column",
              }}
            >
              {filtered.map((entry) => {
                const isSelected = selected?.id === entry.id;
                return (
                  <div
                    key={entry.id}
                    role="option"
                    aria-selected={isSelected}
                    className={`library-row ${isSelected ? "selected" : ""}`}
                    style={{
                      padding: "6px 8px",
                      borderBottom: "1px solid var(--border)",
                      display: "flex",
                      alignItems: "center",
                      gap: 6,
                    }}
                  >
                    <button
                      type="button"
                      className="library-row-main"
                      style={{ flex: 1, textAlign: "left" }}
                      onClick={() => setSelectedId(entry.id)}
                    >
                      <span className="palette-item-text">
                        <span className="palette-item-label">{entry.title}</span>
                        <small className="palette-item-subtitle">
                          {new Date(entry.createdAtMs).toLocaleTimeString()} ·{" "}
                          {triggerBadge(entry.trigger)}
                        </small>
                      </span>
                    </button>
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={`Delete revision ${entry.title}`}
                      onClick={() =>
                        void client
                          .call("delete_local_history_entry", { id: entry.id })
                          .then(() => reload(scope))
                          .catch((err) => setError(messageOf(err)))
                      }
                    >
                      <Trash2 size={12} />
                    </button>
                  </div>
                );
              })}
            </div>

            {selected && (
              <div
                style={{
                  border: "1px solid var(--border)",
                  borderRadius: 6,
                  display: "flex",
                  flexDirection: "column",
                  minWidth: 0,
                  overflow: "hidden",
                }}
              >
                <div
                  style={{
                    padding: "8px 12px",
                    borderBottom: "1px solid var(--border)",
                    background: "var(--surface-hover)",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    gap: 8,
                    flexWrap: "wrap",
                  }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <strong>{selected.title}</strong>
                    <kbd>{triggerBadge(selected.trigger)}</kbd>
                    {diff && (
                      <small className="mono muted">
                        {diff.identical
                          ? "Identical to active editor"
                          : `+${diff.addedCount} / -${diff.removedCount} vs active editor`}
                      </small>
                    )}
                  </div>
                  <div style={{ display: "flex", gap: 6 }}>
                    {tab?.kind === "query" && (
                      <button
                        type="button"
                        className="button primary small"
                        disabled={diff?.identical === true}
                        onClick={() => {
                          state.updateTab(tab.id, { sql: selected.sql });
                          state.notify(
                            `Restored revision from ${new Date(selected.createdAtMs).toLocaleTimeString()}`,
                          );
                          onClose();
                        }}
                      >
                        <RotateCcw size={12} />
                        Restore to editor
                      </button>
                    )}
                    <button
                      type="button"
                      className="button ghost small"
                      onClick={() => {
                        state.openQuery(
                          selected.connectionId ?? tab?.connectionId ?? state.selectedConnectionId,
                          selected.sql,
                        );
                        onClose();
                      }}
                    >
                      <ExternalLink size={12} />
                      Open in new tab
                    </button>
                  </div>
                </div>

                <div
                  data-testid="local-history-diff"
                  style={{
                    flex: 1,
                    overflow: "auto",
                    fontFamily: "var(--font-mono, monospace)",
                    fontSize: 12,
                    lineHeight: 1.5,
                    background: "var(--bg)",
                  }}
                >
                  {diff ? (
                    diff.lines.map((line, idx) => {
                      const bg =
                        line.kind === "added"
                          ? "color-mix(in srgb, #22c55e 16%, transparent)"
                          : line.kind === "removed"
                            ? "color-mix(in srgb, #ef4444 16%, transparent)"
                            : undefined;
                      const prefixChar =
                        line.kind === "added" ? "+" : line.kind === "removed" ? "-" : " ";
                      return (
                        <div
                          key={idx}
                          style={{
                            display: "grid",
                            gridTemplateColumns: "36px 36px 18px 1fr",
                            padding: "0 8px",
                            background: bg,
                            whiteSpace: "pre",
                          }}
                        >
                          <span className="muted" style={{ userSelect: "none" }}>
                            {line.oldLineNo ?? ""}
                          </span>
                          <span className="muted" style={{ userSelect: "none" }}>
                            {line.newLineNo ?? ""}
                          </span>
                          <span style={{ userSelect: "none" }}>{prefixChar}</span>
                          <span>{line.text}</span>
                        </div>
                      );
                    })
                  ) : (
                    <pre style={{ margin: 0, padding: 12 }}>{selected.sql}</pre>
                  )}
                </div>
              </div>
            )}
          </div>
        )}
      </div>
      <footer className="modal-footer">
        <button type="button" className="button ghost" onClick={onClose}>
          Close
        </button>
      </footer>
    </Modal>
  );
}
