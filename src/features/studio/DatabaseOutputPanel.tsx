import { useMemo, useState } from "react";
import { AlertCircle, AlertTriangle, CheckCircle2, Info, Terminal, Trash2 } from "lucide-react";

export const MAX_DATABASE_OUTPUT_ENTRIES = 200;

export type DatabaseOutputLevel = "info" | "success" | "warning" | "error";

export interface DatabaseOutputEntry {
  id: string;
  timestampMs: number;
  level: DatabaseOutputLevel;
  runId?: string | null;
  statementIndex?: number;
  elapsedMs?: number;
  code?: string;
  message: string;
}

export function appendDatabaseOutputEntries(
  existing: readonly DatabaseOutputEntry[] | undefined,
  incoming: readonly DatabaseOutputEntry[],
): DatabaseOutputEntry[] {
  if (incoming.length === 0) return existing ? [...existing] : [];
  const merged = existing ? [...existing, ...incoming] : [...incoming];
  if (merged.length <= MAX_DATABASE_OUTPUT_ENTRIES) return merged;
  return merged.slice(merged.length - MAX_DATABASE_OUTPUT_ENTRIES);
}

function formatClockTime(timestampMs: number): string {
  const date = new Date(timestampMs);
  const hh = String(date.getHours()).padStart(2, "0");
  const mm = String(date.getMinutes()).padStart(2, "0");
  const ss = String(date.getSeconds()).padStart(2, "0");
  const ms = String(date.getMilliseconds()).padStart(3, "0");
  return `${hh}:${mm}:${ss}.${ms}`;
}

function LevelGlyph({ level }: { level: DatabaseOutputLevel }) {
  switch (level) {
    case "error":
      return <AlertCircle size={13} aria-hidden="true" />;
    case "warning":
      return <AlertTriangle size={13} aria-hidden="true" />;
    case "success":
      return <CheckCircle2 size={13} aria-hidden="true" />;
    case "info":
      return <Info size={13} aria-hidden="true" />;
  }
}

export function DatabaseOutputPanel({
  entries,
  onClear,
}: {
  entries: readonly DatabaseOutputEntry[];
  onClear(): void;
}) {
  const [filterMode, setFilterMode] = useState<"all" | "issues">("all");
  const [query, setQuery] = useState("");

  const issueCount = useMemo(
    () => entries.filter((e) => e.level === "error" || e.level === "warning").length,
    [entries],
  );

  const visibleEntries = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return entries.filter((entry) => {
      if (filterMode === "issues" && entry.level !== "error" && entry.level !== "warning") {
        return false;
      }
      if (!needle) return true;
      return (
        entry.message.toLowerCase().includes(needle) ||
        (entry.code?.toLowerCase().includes(needle) ?? false)
      );
    });
  }, [entries, filterMode, query]);

  return (
    <section
      className="database-output-panel"
      role="region"
      aria-label="Database output"
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
      }}
    >
      <div
        className="database-output-toolbar"
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 8,
          padding: "8px 14px",
          borderBottom: "1px solid var(--border-subtle, rgba(128, 128, 128, 0.18))",
          flexWrap: "wrap",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <div className="segmented" role="group" aria-label="Output level filter">
            <button
              type="button"
              className={filterMode === "all" ? "active" : ""}
              aria-pressed={filterMode === "all"}
              onClick={() => setFilterMode("all")}
            >
              All ({entries.length})
            </button>
            <button
              type="button"
              className={filterMode === "issues" ? "active" : ""}
              aria-pressed={filterMode === "issues"}
              onClick={() => setFilterMode("issues")}
            >
              Warnings &amp; errors ({issueCount})
            </button>
          </div>
          <input
            type="search"
            aria-label="Filter database output"
            placeholder="Filter messages or error codes…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            style={{ width: 220, height: 28, fontSize: 12 }}
          />
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span className="eyebrow" style={{ fontSize: 10 }}>
            Session-only · max {MAX_DATABASE_OUTPUT_ENTRIES} entries
          </span>
          <button
            type="button"
            className="button ghost small"
            aria-label="Clear database output"
            disabled={entries.length === 0}
            onClick={onClear}
          >
            <Trash2 size={12} />
            Clear
          </button>
        </div>
      </div>

      {visibleEntries.length === 0 ? (
        <div className="results-empty" style={{ flex: 1 }}>
          <Terminal size={24} />
          <h3>{entries.length === 0 ? "No session output yet" : "No matching output entries"}</h3>
          <p>
            {entries.length === 0
              ? "Statement completions, row counts, server notices, and error diagnostics appear here in memory."
              : "Adjust the level filter or search text to view recorded session events."}
          </p>
        </div>
      ) : (
        <div
          role="log"
          aria-label="Session output entries"
          style={{
            flex: 1,
            overflow: "auto",
            padding: "6px 14px",
            fontFamily: "var(--font-mono, monospace)",
            fontSize: 12,
          }}
        >
          {visibleEntries.map((entry) => (
            <div
              key={entry.id}
              data-output-level={entry.level}
              style={{
                display: "flex",
                alignItems: "flex-start",
                gap: 8,
                padding: "5px 0",
                borderBottom: "1px solid var(--border-subtle, rgba(128, 128, 128, 0.1))",
                color:
                  entry.level === "error"
                    ? "var(--danger, #e5534b)"
                    : entry.level === "warning"
                      ? "var(--warning, #d29922)"
                      : "inherit",
              }}
            >
              <span className="muted" style={{ flexShrink: 0 }}>
                {formatClockTime(entry.timestampMs)}
              </span>
              <span style={{ flexShrink: 0, marginTop: 1 }}>
                <LevelGlyph level={entry.level} />
              </span>
              {entry.statementIndex !== undefined && (
                <span className="schema-count" style={{ flexShrink: 0 }}>
                  #{entry.statementIndex + 1}
                </span>
              )}
              {entry.code && <code style={{ flexShrink: 0, fontSize: 11 }}>{entry.code}</code>}
              <span style={{ flex: 1, whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>
                {entry.message}
              </span>
              {entry.elapsedMs !== undefined && (
                <span className="muted" style={{ flexShrink: 0 }}>
                  {entry.elapsedMs} ms
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
