import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef } from "react";
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
  type Completion,
} from "@codemirror/autocomplete";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { sql, PostgreSQL, MySQL, StandardSQL } from "@codemirror/lang-sql";
import type { EngineKind, TableInfo, TableSchema } from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import type { CompletionItem } from "../../data/completion-index";
import { useStudio } from "../../state/studio";
import { referencedTables, selectedStatement, tableLookup } from "./sql-context";

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
  const completion = useRef<Worker | null>(null),
    formatter = useRef<Worker | null>(null),
    sequence = useRef(0),
    revision = useRef(0);
  const pending = useRef(new Map<number, (items: Completion[]) => void>());
  const formatting = useRef<{ id: number; source: string } | null>(null);
  const schemaCache = useRef(new Map<string, Promise<TableSchema>>());
  const lookup = useMemo(() => tableLookup(props.tables), [props.tables]);
  const referenceKey = JSON.stringify(referencedTables(props.value, lookup));
  const schemaItems = useMemo<CompletionItem[]>(
    () =>
      props.tables.flatMap(({ table }) => [
        { label: table.name, type: "type", detail: table.schema },
        { label: `${table.schema}.${table.name}`, type: "type" },
      ]),
    [props.tables],
  );

  useImperativeHandle(
    ref,
    () => ({
      selection() {
        const editor = view.current;
        if (!editor) return latest.current.value;
        return selectedStatement(editor.state);
      },
      focus() {
        view.current?.focus();
      },
      format() {
        if (!view.current) return;
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
    completion.current = worker;
    worker.onmessage = ({
      data,
    }: MessageEvent<{ type: string; id: number; revision: number; items?: Completion[] }>) => {
      if (data.type !== "suggestions") return;
      pending.current.get(data.id)?.(data.revision === revision.current ? (data.items ?? []) : []);
      pending.current.delete(data.id);
    };
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
        languageCompartment.of(
          sql({
            dialect:
              latest.current.engine === "postgres"
                ? PostgreSQL
                : latest.current.engine === "mysql"
                  ? MySQL
                  : StandardSQL,
          }),
        ),
        themeCompartment.of([
          editorTheme(latest.current.theme),
          highlighting(latest.current.theme),
        ]),
        autocompletion({
          override: [
            async (context) => {
              const word = context.matchBefore(/[\w.]+/);
              if (!word || (!context.explicit && word.from === word.to)) return null;
              const id = ++sequence.current;
              const items = await new Promise<Completion[]>((resolve) => {
                pending.current.set(id, resolve);
                worker.postMessage({
                  type: "prefix",
                  id,
                  prefix: word.text,
                  revision: revision.current,
                });
                setTimeout(() => {
                  if (pending.current.delete(id)) resolve([]);
                }, 300);
              });
              return { from: word.from, options: items };
            },
          ],
        }),
        keymap.of([
          {
            key: "Mod-Enter",
            run: (editor) => {
              try {
                latest.current.onRun(selectedStatement(editor.state));
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
          if (update.docChanged) latest.current.onChange(update.state.doc.toString());
        }),
        EditorView.contentAttributes.of({ "aria-label": "SQL editor", spellcheck: "false" }),
      ],
    });
    view.current = new EditorView({ state, parent: host.current! });
    return () => {
      view.current?.destroy();
      view.current = null;
      worker.terminate();
      formatter.current?.terminate();
      for (const resolve of pending.current.values()) resolve([]);
      pending.current.clear();
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
      effects: languageCompartment.reconfigure(
        sql({
          dialect:
            props.engine === "postgres"
              ? PostgreSQL
              : props.engine === "mysql"
                ? MySQL
                : StandardSQL,
        }),
      ),
    });
  }, [props.engine]);
  useEffect(() => {
    schemaCache.current.clear();
  }, [props.sessionId, props.tables]);
  useEffect(() => {
    const current = ++revision.current;
    let cancelled = false;
    const keywords = [
      "SELECT",
      "FROM",
      "WHERE",
      "JOIN",
      "LEFT JOIN",
      "ORDER BY",
      "GROUP BY",
      "LIMIT",
      "INSERT INTO",
      "UPDATE",
      "DELETE FROM",
      "RETURNING",
      "WITH",
      "EXPLAIN",
    ];
    const base = [...schemaItems, ...keywords.map((label) => ({ label, type: "keyword" }))];
    completion.current?.postMessage({ type: "replace", revision: current, items: base });
    let timer: ReturnType<typeof setTimeout> | undefined;
    if (props.sessionId) {
      const referenced: { table: TableSchema["table"]; aliases: string[] }[] =
        JSON.parse(referenceKey);
      timer = setTimeout(() => {
        void Promise.all(
          referenced.map(({ table }) => {
            const key = JSON.stringify(table);
            let request = schemaCache.current.get(key);
            if (!request) {
              request = client.call<TableSchema>("table_schema", {
                sessionId: props.sessionId,
                table,
              });
              schemaCache.current.set(key, request);
              request.catch(() => schemaCache.current.delete(key));
            }
            return request;
          }),
        )
          .then((schemas) => {
            if (cancelled || revision.current !== current) return;
            const columns = schemas.flatMap((schema, i) =>
              schema.columns.flatMap((column) =>
                [
                  column.name,
                  `${schema.table.name}.${column.name}`,
                  ...referenced[i].aliases.map((alias) => `${alias}.${column.name}`),
                ].map((label) => ({
                  label,
                  type: "property",
                  detail: `${schema.table.name} · ${column.databaseType}`,
                })),
              ),
            );
            completion.current?.postMessage({
              type: "replace",
              revision: current,
              items: [...base, ...columns],
            });
          })
          .catch((error) => {
            if (!cancelled) useStudio.getState().notify(messageOf(error));
          });
      }, 100);
    }
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [schemaItems, props.sessionId, referenceKey]);
  return <div className="sql-editor" ref={host} />;
});
