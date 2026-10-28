import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
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
import type { EngineKind, TableInfo, TableSchema } from "../../ipc/generated";
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
      (sessionId, table) => client.call<TableSchema>("table_schema", { sessionId, table }),
      (error) => useStudio.getState().notify(messageOf(error)),
    );
    completion.current = session;
    session.configure(latest.current);
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
    if (completion.current?.configure(props) && view.current) {
      closeCompletion(view.current);
      completion.current.update(view.current.state);
    }
  }, [props.tables, props.sessionId, props.engine, props.database, props.defaultSchema]);
  return <div className="sql-editor" ref={host} />;
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
