import { useMemo, useRef, useState } from "react";
import { Bookmark, Clock3, Database, FileCode2, Search, Star, Trash2 } from "lucide-react";
import type { ConnectionProfile, SavedBookmark } from "../../ipc/generated";
import { messageOf } from "../../ipc/client";
import { InlineError, Modal } from "../../components/ui/Modal";
import { tabNavigation } from "../../components/ui/tabs";
import { ObjectKindGlyph } from "./ObjectInspectorView";
import {
  useStudio,
  type OpenLocationOutcome,
  type RecentLocationTarget,
  type StudioTab,
} from "../../state/studio";

export type LibraryTab = "saved" | "bookmarks" | "recents";

const LIBRARY_TABS: readonly { id: LibraryTab; label: string }[] = [
  { id: "saved", label: "Saved queries" },
  { id: "bookmarks", label: "Bookmarks" },
  { id: "recents", label: "Recent locations" },
];

function bookmarkToLocationTarget(bookmark: SavedBookmark): RecentLocationTarget {
  if (bookmark.target.kind === "savedQuery") {
    return { kind: "savedQuery", savedQueryId: bookmark.target.savedQueryId };
  }
  return {
    kind: "object",
    connectionId: bookmark.target.connectionId,
    object: bookmark.target.object,
  };
}

export function SaveQueryDialog({
  tab,
  profiles,
  onClose,
}: {
  tab: StudioTab;
  profiles: readonly ConnectionProfile[];
  onClose(): void;
}) {
  const state = useStudio();
  const existing = tab.savedQueryId
    ? state.savedQueries.find((item) => item.id === tab.savedQueryId)
    : undefined;
  const activeProfile = profiles.find((p) => p.id === tab.connectionId);
  const initialTitle =
    existing?.title ??
    (tab.fileName ? tab.fileName.replace(/\.sql$/i, "") : tab.title) ??
    "Saved query";
  const initialConnectionId = existing
    ? (existing.connectionId ?? "")
    : activeProfile && activeProfile.storeQueryText
      ? activeProfile.id
      : "";

  const [title, setTitle] = useState(initialTitle);
  const [connectionId, setConnectionId] = useState(initialConnectionId);
  const [saveAsNew, setSaveAsNew] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  return (
    <Modal
      title={existing ? "Update saved query" : "Save query to library"}
      subtitle="Store reusable SQL in your local Joinery library"
      onClose={onClose}
      busy={busy}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          const trimmed = title.trim();
          if (!trimmed) {
            setError("Provide a title for this saved query");
            return;
          }
          setBusy(true);
          setError(null);
          void state
            .saveQueryToLibrary(
              {
                id: existing && !saveAsNew ? existing.id : null,
                title: trimmed,
                sql: tab.sql,
                connectionId: connectionId || null,
              },
              tab.id,
            )
            .then(() => onClose())
            .catch((err) => {
              setError(messageOf(err));
              setBusy(false);
            });
        }}
      >
        <div className="modal-body form-grid">
          <label className="field span-2">
            <span>Query title</span>
            <input
              aria-label="Query title"
              autoFocus
              maxLength={120}
              value={title}
              disabled={busy}
              onChange={(event) => setTitle(event.target.value)}
            />
          </label>
          <label className="field span-2">
            <span>Connection affinity</span>
            <select
              aria-label="Connection affinity"
              value={connectionId}
              disabled={busy}
              onChange={(event) => setConnectionId(event.target.value)}
            >
              <option value="">Any connection (detached)</option>
              {profiles.map((profile) => (
                <option key={profile.id} value={profile.id}>
                  {profile.name}
                </option>
              ))}
            </select>
          </label>
          {activeProfile && !activeProfile.storeQueryText && !existing && (
            <p className="field-hint span-2">
              {activeProfile.name} disables automatic query-text storage, so this saved query
              defaults to detached unless you bind it explicitly.
            </p>
          )}
          {existing && (
            <label className="checkbox-field span-2">
              <input
                type="checkbox"
                checked={saveAsNew}
                disabled={busy}
                onChange={(event) => setSaveAsNew(event.target.checked)}
              />
              <span>
                Save as a new library entry instead of updating &ldquo;{existing.title}&rdquo;
              </span>
            </label>
          )}
          <div className="span-2">
            <InlineError message={error} />
          </div>
        </div>
        <footer className="modal-footer">
          <button type="button" className="button ghost" disabled={busy} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="button primary" disabled={busy || !title.trim()}>
            {existing && !saveAsNew ? "Update query" : "Save query"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}

export function LibraryModal({
  initialTab = "saved",
  onNeedsConnect,
  onClose,
}: {
  initialTab?: LibraryTab;
  onNeedsConnect(profile: ConnectionProfile): void;
  onClose(): void;
}) {
  const state = useStudio();
  const [activeTab, setActiveTab] = useState<LibraryTab>(initialTab);
  const [filter, setFilter] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const profileById = useMemo(() => {
    const map = new Map<string, ConnectionProfile>();
    for (const profile of state.bootstrap?.profiles ?? []) map.set(profile.id, profile);
    return map;
  }, [state.bootstrap?.profiles]);

  const queryLower = filter.trim().toLowerCase();

  const entries = useMemo(() => {
    if (activeTab === "saved") {
      return state.savedQueries
        .filter((item) => {
          if (!queryLower) return true;
          const conn = item.connectionId ? (profileById.get(item.connectionId)?.name ?? "") : "";
          return `${item.title} ${item.sql} ${conn}`.toLowerCase().includes(queryLower);
        })
        .map((item) => {
          const connName = item.connectionId
            ? (profileById.get(item.connectionId)?.name ?? "Removed connection")
            : "Any connection";
          const preview = item.sql.replace(/\s+/g, " ").trim().slice(0, 96);
          return {
            key: `saved:${item.id}`,
            label: item.title,
            subtitle: `${connName} · ${preview}`,
            badge: "Saved query",
            kind: "saved" as const,
            savedId: item.id,
            target: { kind: "savedQuery" as const, savedQueryId: item.id },
          };
        });
    }
    if (activeTab === "bookmarks") {
      return state.bookmarks
        .filter((item) => {
          if (!queryLower) return true;
          return item.label.toLowerCase().includes(queryLower);
        })
        .map((item) => {
          const subtitle =
            item.target.kind === "savedQuery"
              ? "Saved query"
              : `${profileById.get(item.target.connectionId)?.name ?? "Connection"} · ${item.target.object.kind}`;
          return {
            key: `bm:${item.id}`,
            label: item.label,
            subtitle,
            badge: "Bookmark",
            kind: "bookmark" as const,
            bookmark: item,
            target: bookmarkToLocationTarget(item),
          };
        });
    }
    return state.recentLocations
      .filter((item) => {
        if (!queryLower) return true;
        return `${item.label} ${item.subtitle}`.toLowerCase().includes(queryLower);
      })
      .map((item) => ({
        key: `rec:${item.key}`,
        label: item.label,
        subtitle: item.subtitle,
        badge: "Recent",
        kind: "recent" as const,
        recentTarget: item.target,
        target: item.target,
      }));
  }, [
    activeTab,
    profileById,
    queryLower,
    state.bookmarks,
    state.recentLocations,
    state.savedQueries,
  ]);

  const clampedIndex = entries.length > 0 ? Math.min(selectedIndex, entries.length - 1) : 0;

  function openTarget(target: RecentLocationTarget) {
    void state
      .openLocation(target)
      .then((outcome: OpenLocationOutcome) => {
        onClose();
        if (outcome.kind === "needsConnect") {
          onNeedsConnect(outcome.profile);
        }
      })
      .catch((err) => state.notify(messageOf(err)));
  }

  return (
    <Modal
      title={
        activeTab === "recents"
          ? "Recent locations"
          : activeTab === "bookmarks"
            ? "Bookmarks"
            : "Saved queries"
      }
      onClose={onClose}
    >
      <div className="palette-search">
        <Search size={17} />
        <input
          ref={inputRef}
          autoFocus
          aria-label="Filter library"
          placeholder={
            activeTab === "saved"
              ? "Filter saved queries…"
              : activeTab === "bookmarks"
                ? "Filter bookmarks…"
                : "Filter recent locations…"
          }
          value={filter}
          onChange={(event) => {
            setFilter(event.target.value);
            setSelectedIndex(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown" && entries.length > 0) {
              event.preventDefault();
              setSelectedIndex((clampedIndex + 1) % entries.length);
            } else if (event.key === "ArrowUp" && entries.length > 0) {
              event.preventDefault();
              setSelectedIndex((clampedIndex - 1 + entries.length) % entries.length);
            } else if (event.key === "Enter" && entries[clampedIndex]) {
              event.preventDefault();
              openTarget(entries[clampedIndex].target);
            }
          }}
        />
      </div>
      <div className="palette-scopes-bar">
        <div
          className="palette-scopes"
          role="tablist"
          aria-label="Library sections"
          onKeyDown={tabNavigation}
        >
          {LIBRARY_TABS.map((tab) => {
            const selected = activeTab === tab.id;
            return (
              <button
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={selected}
                tabIndex={selected ? 0 : -1}
                className={selected ? "active" : ""}
                onClick={() => {
                  setActiveTab(tab.id);
                  setSelectedIndex(0);
                  inputRef.current?.focus();
                }}
              >
                {tab.label}
              </button>
            );
          })}
        </div>
      </div>
      <div className="palette-commands library-list">
        {entries.map((entry, idx) => (
          <div
            key={entry.key}
            className={`library-row ${idx === clampedIndex ? "selected" : ""}`}
            onMouseEnter={() => setSelectedIndex(idx)}
          >
            <button
              type="button"
              className="library-row-main"
              onClick={() => openTarget(entry.target)}
            >
              {entry.kind === "saved" ? (
                <Bookmark size={15} />
              ) : entry.kind === "bookmark" ? (
                entry.bookmark.target.kind === "object" ? (
                  <ObjectKindGlyph kind={entry.bookmark.target.object.kind} size={15} />
                ) : (
                  <Star size={15} />
                )
              ) : entry.recentTarget.kind === "object" ? (
                <ObjectKindGlyph kind={entry.recentTarget.object.kind} size={15} />
              ) : entry.recentTarget.kind === "table" ? (
                <Database size={15} />
              ) : entry.recentTarget.kind === "savedQuery" ? (
                <Bookmark size={15} />
              ) : (
                <FileCode2 size={15} />
              )}
              <span className="palette-item-text">
                <span className="palette-item-label">{entry.label}</span>
                <small className="palette-item-subtitle">{entry.subtitle}</small>
              </span>
              <kbd>{entry.badge}</kbd>
            </button>
            {entry.kind === "saved" && (
              <button
                type="button"
                className="icon-button library-row-delete"
                aria-label={`Delete saved query ${entry.label}`}
                title="Delete saved query"
                onClick={() =>
                  void state
                    .deleteSavedQuery(entry.savedId)
                    .catch((err) => state.notify(messageOf(err)))
                }
              >
                <Trash2 size={13} />
              </button>
            )}
            {entry.kind === "bookmark" && (
              <button
                type="button"
                className="icon-button library-row-delete"
                aria-label={`Remove bookmark ${entry.label}`}
                title="Remove bookmark"
                onClick={() =>
                  void state
                    .deleteBookmark(entry.bookmark.id)
                    .catch((err) => state.notify(messageOf(err)))
                }
              >
                <Trash2 size={13} />
              </button>
            )}
          </div>
        ))}
        {entries.length === 0 && (
          <div className="grid-empty">
            {activeTab === "saved" ? (
              <>
                <Bookmark size={18} />
                <span>
                  No saved queries yet. Use &ldquo;Save to library&rdquo; in any SQL console.
                </span>
              </>
            ) : activeTab === "bookmarks" ? (
              <>
                <Star size={18} />
                <span>No bookmarks yet. Press ⌘D on any table, object, or saved query.</span>
              </>
            ) : (
              <>
                <Clock3 size={18} />
                <span>No recent locations visited in this session yet.</span>
              </>
            )}
          </div>
        )}
      </div>
    </Modal>
  );
}
