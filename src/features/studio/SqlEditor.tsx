import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { EditorState, Compartment } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
  rectangularSelection,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  bracketMatching,
  HighlightStyle,
  syntaxHighlighting,
  foldGutter,
} from "@codemirror/language";
import { tags } from "@lezer/highlight";
import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  closeCompletion,
} from "@codemirror/autocomplete";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { sql } from "@codemirror/lang-sql";
import type { CompletionCatalog, EngineKind, TableInfo, TableSchema } from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import type { CompletionResponse } from "../../data/completion-protocol";
import { useStudio } from "../../state/studio";
import { selectedStatement } from "./sql-context";
import { sqlDialect } from "./sql-identifiers";
import { CompletionSession } from "./completion-session";

export interface EditorHandle {
  selection(): string;
  focus(): void;
  format(): void;
}
interface Props {
  value: string;
  onChange(value: string): void;
  onRun(sql: string): void;
  engine: EngineKind;
  sessionId: string | null;
  tables: TableInfo[];
  database?: string;
  defaultSchema?: string;
  theme: string;
  contextEpoch?: string;
}
const themeCompartment = new Compartment(),
  languageCompartment = new Compartment();

function editorTheme(theme: string) {
  const dark = theme !== "light";
  return EditorView.theme(
    {
      "&": {
        height: "100%",
        fontSize: "13px",
        backgroundColor: "transparent",
        color: dark ? "#d8e1d7" : "#29372a",
      },
      ".cm-content": {
        fontFamily: "'SFMono-Regular', Consolas, 'Liberation Mono', monospace",
        padding: "20px 0",
        caretColor: dark ? "#c1dcb4" : "#397344",
      },
      ".cm-line": { padding: "0 20px 0 12px", lineHeight: "1.9" },
      ".cm-gutters": {
        backgroundColor: "transparent",
        color: dark ? "#69796d" : "#69796b",
        border: "none",
        minWidth: "48px",
        fontSize: "11px",
      },
      ".cm-activeLineGutter": {
        backgroundColor: "transparent",
        color: dark ? "#afbea9" : "#526a52",
      },
      ".cm-activeLine": { backgroundColor: dark ? "#ffffff03" : "#00000003" },
      ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
        backgroundColor: dark ? "#a6c59725" : "#4b8e4330",
      },
      ".cm-cursor": { borderLeftColor: dark ? "#b6d7a8" : "#397344" },
      ".cm-scroller": { overflow: "auto" },
      ".cm-tooltip": {
        backgroundColor: dark ? "#252d27" : "#ffffff",
        color: dark ? "#d8e1d7" : "#243425",
        border: "1px solid var(--border)",
        borderRadius: "8px",
        boxShadow: "0 12px 30px #0003",
      },
    },
    { dark },
  );
}
function highlighting(theme: string) {
  const dark = theme !== "light";
  return syntaxHighlighting(
    HighlightStyle.define([
      { tag: tags.keyword, color: dark ? "#c9b1df" : "#754d94" },
      { tag: tags.string, color: dark ? "#b6d29f" : "#44713b" },
      { tag: [tags.number, tags.bool, tags.null], color: dark ? "#dfc68e" : "#936722" },
      { tag: tags.comment, color: dark ? "#87988a" : "#718271", fontStyle: "italic" },
      { tag: tags.operator, color: dark ? "#b5c3b6" : "#4c6750" },
    ]),
  );
}

export const SqlEditor = forwardRef<EditorHandle, Props>(function SqlEditor(props, ref) {
  const [live, setLive] = useState<CompletionCatalog | null>(null);
  const [contextError, setContextError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [switchingContext, setSwitchingContext] = useState(false);
  const runtimeState = useStudio((s) =>
    props.sessionId ? s.sessionStates[props.sessionId] : undefined,
  );
  const availableContexts = useStudio((s) =>
    props.sessionId ? s.sessionContexts[props.sessionId] : undefined,
  );
  const liveRef = useRef<CompletionCatalog | null>(null);
  const configure = () => {
    const config = latest.current;
    const snapshot = liveRef.current;
    if (config.engine === "redis") return config;
    return {
      ...config,
      tables: snapshot?.tables ?? [],
      sessionId: snapshot ? config.sessionId : null,
      database: snapshot?.identity.database,
      searchPath: snapshot?.identity.searchPath,
      unqualifiedTables: snapshot?.unqualifiedTables,
      caseSensitiveTableNames: snapshot?.identity.caseSensitiveTableNames ?? undefined,
      contextRevision: snapshot?.revision,
    };
  };
  const host = useRef<HTMLDivElement>(null),
    view = useRef<EditorView | null>(null),
    latest = useRef(props);
  latest.current = props;
  const completion = useRef<CompletionSession | null>(null),
    formatter = useRef<Worker | null>(null),
    sequence = useRef(0);
  const formatting = useRef<{ id: number; source: string } | null>(null);

  useImperativeHandle(
    ref,
    () => ({
      selection() {
        const editor = view.current;
        if (!editor) return latest.current.value;
        return latest.current.engine === "redis"
          ? redisSelection(editor.state)
          : selectedStatement(editor.state);
      },
      focus() {
        view.current?.focus();
      },
      format() {
        if (!view.current || latest.current.engine === "redis") return;
        if (!formatter.current) {
          formatter.current = new Worker(
            new URL("../../workers/formatter.worker.ts", import.meta.url),
            { type: "module" },
          );
          formatter.current.onmessage = ({
            data,
          }: MessageEvent<{ id: number; sql?: string; error?: string }>) => {
            if (data.id !== formatting.current?.id) return;
            if (data.error) {
              useStudio.getState().notify(data.error);
              return;
            }
            const editor = view.current;
            if (
              editor &&
              data.sql !== undefined &&
              editor.state.doc.toString() === formatting.current.source
            )
              editor.dispatch({
                changes: { from: 0, to: editor.state.doc.length, insert: data.sql },
              });
          };
        }
        formatting.current = { id: ++sequence.current, source: view.current.state.doc.toString() };
        formatter.current.postMessage({
          id: formatting.current.id,
          sql: formatting.current.source,
          language:
            latest.current.engine === "postgres"
              ? "postgresql"
              : latest.current.engine === "mysql"
                ? "mysql"
                : "snowflake",
        });
      },
    }),
    [],
  );

  useEffect(() => {
    const worker = new Worker(new URL("../../workers/completion.worker.ts", import.meta.url), {
      type: "module",
    });
    const session = new CompletionSession(
      (message) => worker.postMessage(message),
      (sessionId, table, contextRevision) =>
        contextRevision
          ? client.call<TableSchema>("completion_schema", {
              sessionId,
              table,
              revision: contextRevision,
            })
          : client.call<TableSchema>("table_schema", { sessionId, table }),
      (error) => useStudio.getState().notify(messageOf(error)),
    );
    completion.current = session;
    session.configure(configure());
    worker.onmessage = ({ data }: MessageEvent<CompletionResponse>) => session.receive(data);
    const state = EditorState.create({
      doc: latest.current.value,
      extensions: [
        lineNumbers(),
        highlightActiveLineGutter(),
        history(),
        foldGutter(),
        drawSelection(),
        rectangularSelection(),
        bracketMatching(),
        closeBrackets(),
        highlightActiveLine(),
        highlightSelectionMatches(),
        languageCompartment.of(editorLanguage(latest.current.engine)),
        themeCompartment.of([
          editorTheme(latest.current.theme),
          highlighting(latest.current.theme),
        ]),
        autocompletion({
          override: [(context) => session.request(context)],
        }),
        keymap.of([
          {
            key: "Mod-Enter",
            run: (editor) => {
              try {
                latest.current.onRun(
                  latest.current.engine === "redis"
                    ? redisSelection(editor.state)
                    : selectedStatement(editor.state),
                );
              } catch (error) {
                useStudio.getState().notify(messageOf(error));
              }
              return true;
            },
          },
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...historyKeymap,
          ...searchKeymap,
          indentWithTab,
        ]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged || update.selectionSet) session.update(update.state);
          if (update.docChanged) latest.current.onChange(update.state.doc.toString());
        }),
      ],
    });
    view.current = new EditorView({ state, parent: host.current! });
    session.update(state);
    return () => {
      session.dispose();
      completion.current = null;
      view.current?.destroy();
      view.current = null;
      worker.terminate();
      formatter.current?.terminate();
    };
  }, []);

  useEffect(() => {
    const editor = view.current;
    if (editor && editor.state.doc.toString() !== props.value)
      editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: props.value } });
  }, [props.value]);
  useEffect(() => {
    view.current?.dispatch({
      effects: themeCompartment.reconfigure([editorTheme(props.theme), highlighting(props.theme)]),
    });
  }, [props.theme]);
  useEffect(() => {
    view.current?.dispatch({
      effects: languageCompartment.reconfigure(editorLanguage(props.engine)),
    });
  }, [props.engine]);
  useEffect(() => {
    if (completion.current?.configure(configure()) && view.current) {
      closeCompletion(view.current);
      completion.current.update(view.current.state);
    }
  }, [props.tables, props.sessionId, props.engine, props.database, props.defaultSchema, live]);
  useEffect(() => {
    let cancelled = false;
    liveRef.current = null;
    setLive(null);
    setContextError(null);
    if (props.engine === "redis") return;
    completion.current?.configure(configure());
    if (view.current) closeCompletion(view.current);
    if (props.sessionId) {
      void useStudio.getState().refreshSessionContexts(props.sessionId);
      void client
        .call<CompletionCatalog>("completion_catalog", { sessionId: props.sessionId })
        .then((snapshot) => {
          if (!cancelled) {
            liveRef.current = snapshot;
            setLive(snapshot);
          }
        })
        .catch((error) => {
          if (!cancelled) setContextError(messageOf(error));
        });
    }
    return () => {
      cancelled = true;
    };
  }, [props.engine, props.sessionId, props.tables, props.contextEpoch, refresh]);
  const currentContext = runtimeState?.activeContext ?? "";
  const contextOptions =
    availableContexts && availableContexts.length > 0
      ? currentContext && !availableContexts.includes(currentContext)
        ? [currentContext, ...availableContexts]
        : availableContexts
      : [];
  const showContextSwitcher =
    Boolean(props.sessionId) &&
    contextOptions.length > 0 &&
    runtimeState !== undefined &&
    runtimeState.contextKind !== "none";
  return (
    <div className="sql-editor" style={{ display: "flex", flexDirection: "column" }}>
      {props.engine !== "redis" && props.sessionId && (
        <div className="editor-context-row">
          {showContextSwitcher && (
            <select
              className="context-switcher"
              aria-label={props.engine === "mysql" ? "Active database" : "Active schema"}
              value={currentContext}
              disabled={switchingContext}
              onChange={(event) => {
                const target = event.target.value;
                if (!target || !props.sessionId || target === currentContext) return;
                setSwitchingContext(true);
                void useStudio
                  .getState()
                  .switchSessionContext(props.sessionId, target)
                  .catch((error) => useStudio.getState().notify(messageOf(error)))
                  .finally(() => setSwitchingContext(false));
              }}
            >
              {!currentContext && <option value="">No active context</option>}
              {contextOptions.map((item) => (
                <option key={item} value={item}>
                  {item}
                </option>
              ))}
            </select>
          )}
          <div
            aria-live="polite"
            aria-atomic="true"
            className="completion-context"
            style={{
              fontSize: 11,
              padding: "4px 12px",
              display: "flex",
              alignItems: "center",
              gap: 8,
              minWidth: 0,
              flex: 1,
            }}
          >
            <span
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                flex: 1,
              }}
              title={contextError ?? live?.identity.searchPath.join(", ")}
            >
              {contextError
                ? `Completion unavailable: ${contextError}`
                : live
                  ? `${live.identity.database || "No database"} · ${live.identity.role}${props.engine !== "mysql" ? ` · ${live.identity.searchPath.join(", ")}` : ""}`
                  : "Loading completion context…"}
            </span>
            <button
              className="button ghost small"
              style={{ flexShrink: 0 }}
              aria-label="Refresh completion context"
              onClick={() => setRefresh((n) => n + 1)}
            >
              Refresh
            </button>
          </div>
        </div>
      )}
      <div ref={host} style={{ flex: 1, minHeight: 0 }} />
    </div>
  );
});

function redisSelection(state: EditorState) {
  const selection = state.selection.main;
  return selection.empty
    ? state.doc.lineAt(selection.head).text
    : state.sliceDoc(selection.from, selection.to);
}

function editorLanguage(engine: EngineKind) {
  return [
    engine === "redis"
      ? []
      : sql({
          dialect: sqlDialect(engine),
        }),
    EditorView.contentAttributes.of({
      "aria-label": engine === "redis" ? "Redis command editor" : "SQL editor",
      spellcheck: "false",
    }),
  ];
}
