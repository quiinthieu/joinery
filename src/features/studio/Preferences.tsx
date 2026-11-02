import { useState, type FormEvent } from "react";
import {
  Check,
  Code2,
  LockKeyhole,
  Monitor,
  Moon,
  Pencil,
  Plus,
  Sun,
  Loader2,
  Trash2,
} from "lucide-react";
import { Modal, InlineError } from "../../components/ui/Modal";
import type {
  EngineKind,
  SqlFormatStyle,
  SqlKeywordCase,
  SqlSnippet,
  SqlSnippetPlace,
} from "../../ipc/generated";
import { client, desktopAvailable, messageOf } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import { sqlFormatStyle, updateSqlFormatStyle, type SqlFormatScope } from "../../data/sql-format";
import { GroupManager } from "../connections/GroupManager";
import { MetadataRecovery } from "./MetadataRecovery";

function engineBadgeLabel(engine: EngineKind | null): string {
  switch (engine) {
    case "postgres":
      return "PostgreSQL";
    case "mysql":
      return "MySQL";
    case "snowflake":
      return "Snowflake";
    default:
      return "All SQL";
  }
}

export function Preferences({ onClose }: { onClose(): void }) {
  const state = useStudio(),
    [password, setPassword] = useState(""),
    [previous, setPrevious] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null),
    [formatError, setFormatError] = useState<string | null>(null),
    [formatPending, setFormatPending] = useState(0),
    [formatScope, setFormatScope] = useState<SqlFormatScope>("global"),
    [editingSnippetId, setEditingSnippetId] = useState<string | null>(null),
    [snippetTrigger, setSnippetTrigger] = useState(""),
    [snippetTitle, setSnippetTitle] = useState(""),
    [snippetEngine, setSnippetEngine] = useState<"" | "postgres" | "mysql" | "snowflake">(""),
    [snippetPlace, setSnippetPlace] = useState<SqlSnippetPlace>("any"),
    [snippetBody, setSnippetBody] = useState(""),
    [snippetBusy, setSnippetBusy] = useState(false),
    [snippetError, setSnippetError] = useState<string | null>(null);
  async function save(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await client.call("set_passphrase", { passphrase: password, previous: previous || null });
      await state.initialize();
      state.notify("Write passphrase updated");
      setPassword("");
      setPrevious("");
    } catch (error) {
      setError(messageOf(error));
    } finally {
      setBusy(false);
    }
  }
  const formatStyle = sqlFormatStyle(state.sqlFormatSettings, formatScope);
  const hasFormatOverride =
    formatScope !== "global" && !!state.sqlFormatSettings.dialects?.[formatScope];
  async function updateFormatSettings(patch: Partial<SqlFormatStyle> | null) {
    setFormatError(null);
    setFormatPending((pending) => pending + 1);
    try {
      await state.saveSqlFormatSettings(
        updateSqlFormatStyle(useStudio.getState().sqlFormatSettings, formatScope, patch),
      );
    } catch (err) {
      setFormatError(messageOf(err));
    } finally {
      setFormatPending((pending) => pending - 1);
    }
  }
  function resetSnippetForm() {
    setEditingSnippetId(null);
    setSnippetTrigger("");
    setSnippetTitle("");
    setSnippetEngine("");
    setSnippetPlace("any");
    setSnippetBody("");
    setSnippetError(null);
  }
  function startEditSnippet(snippet: SqlSnippet) {
    setEditingSnippetId(snippet.id);
    setSnippetTrigger(snippet.trigger);
    setSnippetTitle(snippet.title);
    setSnippetEngine(
      snippet.engine === "postgres" || snippet.engine === "mysql" || snippet.engine === "snowflake"
        ? snippet.engine
        : "",
    );
    setSnippetPlace(snippet.place ?? "any");
    setSnippetBody(snippet.body);
    setSnippetError(null);
  }
  async function handleSaveSnippet(event: FormEvent) {
    event.preventDefault();
    setSnippetBusy(true);
    setSnippetError(null);
    try {
      const saved = await state.saveSqlSnippet({
        id: editingSnippetId,
        trigger: snippetTrigger.trim(),
        title: snippetTitle.trim(),
        body: snippetBody,
        engine: snippetEngine ? snippetEngine : null,
        place: snippetPlace,
      });
      state.notify(
        editingSnippetId ? `Updated snippet ${saved.trigger}` : `Saved snippet ${saved.trigger}`,
      );
      resetSnippetForm();
    } catch (err) {
      setSnippetError(messageOf(err));
    } finally {
      setSnippetBusy(false);
    }
  }
  async function handleDeleteSnippet(snippet: SqlSnippet) {
    setSnippetBusy(true);
    setSnippetError(null);
    try {
      await state.deleteSqlSnippet(snippet.id);
      if (editingSnippetId === snippet.id) resetSnippetForm();
      state.notify(`Deleted snippet ${snippet.trigger}`);
    } catch (err) {
      setSnippetError(messageOf(err));
    } finally {
      setSnippetBusy(false);
    }
  }
  return (
    <Modal
      title="Preferences"
      subtitle="Make this space your own."
      onClose={onClose}
      busy={busy || snippetBusy || formatPending > 0}
    >
      <div className="modal-body preferences">
        <h3>Appearance</h3>
        <div className="theme-options">
          {(
            [
              { id: "dark", label: "Forest dark", icon: Moon },
              { id: "light", label: "Paper light", icon: Sun },
              { id: "contrast", label: "High contrast", icon: Monitor },
            ] as const
          ).map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              className={`theme-option ${state.theme === id ? "selected" : ""}`}
              aria-pressed={state.theme === id}
              onClick={() => state.setTheme(id)}
            >
              <Icon size={19} />
              <span>{label}</span>
              {state.theme === id && <Check size={13} />}
            </button>
          ))}
        </div>
        <label>
          Display density
          <select
            aria-label="Display density"
            value={state.density}
            onChange={(event) =>
              useStudio.setState({ density: event.target.value as "comfortable" | "compact" })
            }
          >
            <option value="comfortable">Comfortable</option>
            <option value="compact">Compact</option>
          </select>
        </label>
        <div className="preference-divider" />
        <section className="format-settings">
          <h3>
            <Code2 size={15} />
            SQL formatting
          </h3>
          <p className="form-note">
            Format SQL (⇧⌘F) formats the active selection when text is highlighted, or the full
            console otherwise.
          </p>
          <label>
            Formatting style
            <select
              aria-label="Formatting style"
              disabled={busy || snippetBusy}
              value={formatScope}
              onChange={(event) => setFormatScope(event.target.value as SqlFormatScope)}
            >
              <option value="global">Global defaults</option>
              <option value="postgres">PostgreSQL</option>
              <option value="mysql">MySQL / MariaDB</option>
              <option value="snowflake">Snowflake</option>
            </select>
          </label>
          {formatScope !== "global" && (
            <p className="form-note">
              {hasFormatOverride
                ? "This engine uses a custom style."
                : "This engine inherits global defaults. Changing a field creates a custom style."}
            </p>
          )}
          <div className="format-settings-grid">
            <label>
              SQL keyword case
              <select
                aria-label="SQL keyword case"
                disabled={busy || snippetBusy}
                value={formatStyle.keywordCase}
                onChange={(event) =>
                  void updateFormatSettings({
                    keywordCase: event.target.value as SqlKeywordCase,
                  })
                }
              >
                <option value="upper">UPPERCASE</option>
                <option value="lower">lowercase</option>
                <option value="preserve">Preserve</option>
              </select>
            </label>
            <label>
              SQL indent width
              <select
                aria-label="SQL indent width"
                disabled={busy || snippetBusy}
                value={String(formatStyle.indentWidth)}
                onChange={(event) =>
                  void updateFormatSettings({
                    indentWidth: Number(event.target.value),
                  })
                }
              >
                <option value="2">2 spaces</option>
                <option value="4">4 spaces</option>
              </select>
            </label>
            <label>
              Lines between SQL queries
              <select
                aria-label="Lines between SQL queries"
                disabled={busy || snippetBusy}
                value={String(formatStyle.linesBetweenQueries)}
                onChange={(event) =>
                  void updateFormatSettings({
                    linesBetweenQueries: Number(event.target.value),
                  })
                }
              >
                <option value="1">1 blank line</option>
                <option value="2">2 blank lines</option>
              </select>
            </label>
          </div>
          {hasFormatOverride && (
            <button
              type="button"
              className="button ghost small"
              disabled={busy || snippetBusy}
              onClick={() => void updateFormatSettings(null)}
            >
              Use global defaults
            </button>
          )}
          <InlineError message={formatError} />
          {formatPending > 0 && <p role="status">Saving formatting preferences…</p>}
        </section>
        <div className="preference-divider" />
        <section className="snippet-manager">
          <h3>
            <Code2 size={15} />
            SQL snippets (live templates)
          </h3>
          <p className="form-note">
            Built-in triggers (<code>sel</code>, <code>cnt</code>, <code>join</code>,{" "}
            <code>ljoin</code>, <code>cte</code>, <code>lim</code>, <code>ins</code>,{" "}
            <code>upd</code>, <code>del</code>, <code>txn</code>, <code>ctxsel</code>,{" "}
            <code>wrap</code>) expand from SQL completion with <code>{"${1:placeholder}"}</code> tab
            stops. Use <code>{"${0}"}</code> for a final cursor position after editable fields.
            Custom snippets can add new triggers or override built-in ones globally or per dialect.{" "}
            Use <code>{"{{database}}"}</code> and <code>{"{{schema}}"}</code> for quoted identifiers
            from the connected console. Use <code>{"{{selection}}"}</code> with Insert template to
            wrap selected SQL verbatim. Double braces are reserved for these variables; unknown or
            missing values block insertion. A completion site limits a new trigger. Reusing a
            built-in trigger keeps that trigger's site. A snippet saved before sites existed stays
            available everywhere.
          </p>
          {state.sqlSnippets.length > 0 && (
            <div className="managed-snippets" role="list" aria-label="Custom SQL snippets">
              {state.sqlSnippets.map((snippet) => (
                <div className="managed-snippet" role="listitem" key={snippet.id}>
                  <code className="snippet-trigger-code">{snippet.trigger}</code>
                  <span className="snippet-engine-pill">{engineBadgeLabel(snippet.engine)}</span>
                  <span className="snippet-title-text" title={snippet.title}>
                    {snippet.title}
                  </span>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={`Edit snippet ${snippet.trigger}`}
                    disabled={busy || snippetBusy}
                    onClick={() => startEditSnippet(snippet)}
                  >
                    <Pencil size={12} />
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={`Delete snippet ${snippet.trigger}`}
                    disabled={busy || snippetBusy}
                    onClick={() => void handleDeleteSnippet(snippet)}
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
              ))}
            </div>
          )}
          <form className="snippet-form" onSubmit={(e) => void handleSaveSnippet(e)}>
            <div className="snippet-form-row">
              <label>
                Snippet trigger
                <input
                  aria-label="Snippet trigger"
                  value={snippetTrigger}
                  onChange={(e) => setSnippetTrigger(e.target.value)}
                  pattern="[A-Za-z_][A-Za-z0-9_]{0,31}"
                  maxLength={32}
                  placeholder="recent_orders"
                  required
                />
              </label>
              <label>
                Snippet engine
                <select
                  aria-label="Snippet engine"
                  value={snippetEngine}
                  onChange={(e) =>
                    setSnippetEngine(e.target.value as "" | "postgres" | "mysql" | "snowflake")
                  }
                >
                  <option value="">All SQL engines</option>
                  <option value="postgres">PostgreSQL</option>
                  <option value="mysql">MySQL / MariaDB</option>
                  <option value="snowflake">Snowflake</option>
                </select>
              </label>
            </div>
            <label>
              Completion site
              <select
                aria-label="Snippet completion site"
                value={snippetPlace}
                onChange={(e) => setSnippetPlace(e.target.value as SqlSnippetPlace)}
              >
                <option value="any">Anywhere</option>
                <option value="statement">Statement start</option>
                <option value="from">FROM or JOIN list</option>
                <option value="query">After FROM</option>
              </select>
            </label>
            <label>
              Snippet title
              <input
                aria-label="Snippet title"
                value={snippetTitle}
                onChange={(e) => setSnippetTitle(e.target.value)}
                maxLength={120}
                placeholder="Recent orders query"
                required
              />
            </label>
            <label>
              Snippet template
              <textarea
                aria-label="Snippet template"
                className="snippet-template-textarea"
                rows={3}
                value={snippetBody}
                onChange={(e) => setSnippetBody(e.target.value)}
                placeholder={
                  "SELECT * FROM ${1:orders} ORDER BY ${2:created_at} DESC LIMIT ${3:50};"
                }
                required
              />
            </label>
            <InlineError message={snippetError} />
            <div className="group-form-actions">
              <button className="button ghost small" type="submit" disabled={busy || snippetBusy}>
                <Plus size={12} />
                {editingSnippetId ? "Update snippet" : "Add snippet"}
              </button>
              {editingSnippetId && (
                <button
                  type="button"
                  className="button ghost small"
                  disabled={snippetBusy}
                  onClick={resetSnippetForm}
                >
                  Cancel edit
                </button>
              )}
            </div>
          </form>
        </section>
        <div className="preference-divider" />
        <h3>
          <LockKeyhole size={15} />
          Write authorization
        </h3>
        <p className="form-note">
          Production writes require this application passphrase and approval of the exact query
          batch.
        </p>
        <form onSubmit={(e) => void save(e)} className="passphrase-form">
          {state.bootstrap?.passphraseConfigured && (
            <label>
              Current passphrase
              <input
                type="password"
                value={previous}
                onChange={(e) => setPrevious(e.target.value)}
                autoComplete="current-password"
                required
              />
            </label>
          )}
          <label>
            {state.bootstrap?.passphraseConfigured ? "New passphrase" : "Set a write passphrase"}
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="new-password"
              minLength={12}
              maxLength={1024}
              placeholder="At least 12 characters"
              required
            />
          </label>
          <InlineError message={error} />
          <button
            className="button primary"
            type="submit"
            disabled={busy || snippetBusy || formatPending > 0 || !desktopAvailable}
          >
            {busy && <Loader2 size={14} className="spin" />}
            {state.bootstrap?.passphraseConfigured ? "Update passphrase" : "Set passphrase"}
          </button>
        </form>
        <div className="preference-divider" />
        <GroupManager />
        <div className="preference-divider" />
        <h3>Data privacy</h3>
        <p className="form-note">
          Result rows stay in memory. Passwords use your system credential store. History and draft
          text follow each connection's opt-in settings. Saved queries and custom snippets are
          stored explicitly in local metadata and included in its backups; avoid putting secrets in
          templates.
        </p>
        <div className="preference-divider" />
        <fieldset className="preference-recovery-lock" disabled={snippetBusy || formatPending > 0}>
          <MetadataRecovery onBusy={setBusy} />
        </fieldset>
      </div>
      <footer className="modal-footer">
        <button
          className="button ghost"
          onClick={onClose}
          disabled={busy || snippetBusy || formatPending > 0}
        >
          Done
        </button>
      </footer>
    </Modal>
  );
}
