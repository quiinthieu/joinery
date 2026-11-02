import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { EditorState, Compartment, RangeSet, StateEffect, StateField } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  GutterMarker,
  gutter,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
  rectangularSelection,
} from "@codemirror/view";
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
  isolateHistory,
} from "@codemirror/commands";
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
  type Completion,
} from "@codemirror/autocomplete";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { sql } from "@codemirror/lang-sql";
import type {
  AppError,
  CompletionCatalog,
  DatabaseObjectRef,
  DatabaseObjectSummary,
  EngineKind,
  ObjectDetails,
  QueryParamSlot,
  SessionContextOptions,
  TableInfo,
  TableRef,
  TableSchema,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import type { CompletionResponse } from "../../data/completion-protocol";
import {
  prepareEditorFormatJob,
  resolveEditorFormatReplacement,
  sqlFormatterLanguage,
  type EditorFormatJob,
} from "../../data/sql-format";
import { useStudio } from "../../state/studio";
import { selectedStatement, selectedStatementSlice } from "./sql-context";
import {
  adjacentStatement,
  analyzeSqlRename,
  type SqlRenameTarget,
  resolveExecutionError,
  type SqlExecutionOrigin,
  resolveSymbolAt,
  schemaCacheKey,
  tablesNeededForNavigationAt,
  type ResolvedSqlSymbol,
  type SqlSymbolContext,
} from "./sql-navigation";
import { SqlNavigationOverlay } from "./SqlNavigationOverlay";
import { SqlAliasRenameDialog } from "./SqlAliasRenameDialog";
import { SqlColumnInspectionDialog } from "./SqlColumnInspectionDialog";
import {
  columnInspectionLimits,
  prepareColumnInspection,
  type MissingSqlColumn,
  type SqlColumnInspection,
} from "./sql-inspections";
import { sqlDialect } from "./sql-identifiers";
import { CompletionSession } from "./completion-session";
import { effectiveSqlSnippets } from "./sql-snippets";
import { applySqlTemplate } from "./sql-template-context";
import { SqlTemplateDialog } from "./SqlTemplateDialog";

export interface EditorHandle {
  selection(): string;
  executionSlice(): { sql: string; fromUtf16: number };
  revealRange(fromUtf16: number, toUtf16: number): void;
  navigateStatement(direction: -1 | 1): boolean;
  goToDefinition(): Promise<boolean>;
  findUsages(): Promise<boolean>;
  showQuickDocs(): Promise<boolean>;
  focus(): void;
  format(): void;
}
interface Props {
  value: string;
  onChange(value: string): void;
  onRun(sql: string, fromUtf16?: number): void;
  onOpenParameters?(slots: QueryParamSlot[]): void;
  onOpenObject?(
    object: DatabaseObjectRef,
    highlightedColumn?: string,
    catalogRevision?: string,
  ): void;
  error?: AppError | null;
  errorOrigin?: SqlExecutionOrigin;
  engine: EngineKind;
  sessionId: string | null;
  tables: TableInfo[];
  objectExplorerSupported?: boolean;
  database?: string;
  defaultSchema?: string;
  theme: string;
  contextEpoch?: string;
}
const themeCompartment = new Compartment(),
  languageCompartment = new Compartment();

interface EditorErrorMark {
  from: number;
  to: number;
  message: string;
}

const setEditorErrorEffect = StateEffect.define<EditorErrorMark | null>();

const errorStateField = StateField.define<EditorErrorMark | null>({
  create() {
    return null;
  },
  update(current, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setEditorErrorEffect)) {
        if (!effect.value) return null;
        const max = tr.state.doc.length;
        return {
          from: Math.max(0, Math.min(max, effect.value.from)),
          to: Math.max(0, Math.min(max, effect.value.to)),
          message: effect.value.message,
        };
      }
    }
    return tr.docChanged ? null : current;
  },
  provide: (field) =>
    EditorView.decorations.compute([field], (state): DecorationSet => {
      const mark = state.field(field);
      if (!mark || mark.to <= mark.from || mark.to > state.doc.length) return Decoration.none;
      return Decoration.set([
        Decoration.mark({
          class: "cm-sql-error-mark",
          attributes: { title: mark.message },
        }).range(mark.from, mark.to),
      ]);
    }),
});

class SqlErrorGutterMarker extends GutterMarker {
  constructor(readonly message: string) {
    super();
  }
  eq(other: SqlErrorGutterMarker) {
    return this.message === other.message;
  }
  toDOM() {
    const badge = document.createElement("span");
    badge.className = "cm-sql-error-gutter";
    badge.textContent = "!";
    badge.title = this.message;
    badge.setAttribute("aria-hidden", "true");
    return badge;
  }
}

const errorGutterExtension = gutter({
  class: "cm-sql-error-gutter-col",
  markers(editorView) {
    const mark = editorView.state.field(errorStateField);
    if (!mark) return RangeSet.empty;
    const line = editorView.state.doc.lineAt(Math.min(mark.from, editorView.state.doc.length));
    return RangeSet.of([new SqlErrorGutterMarker(mark.message).range(line.from)]);
  },
});

function moveCaretToAdjacentStatement(editor: EditorView, direction: -1 | 1): boolean {
  const target = adjacentStatement(editor.state, direction);
  if (!target) return false;
  editor.dispatch({
    selection: { anchor: target.from },
    effects: EditorView.scrollIntoView(target.from, { y: "center" }),
  });
  return true;
}

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
      ".cm-sql-error-mark": {
        textDecoration: "underline wavy var(--danger)",
        textDecorationSkipInk: "none",
        textUnderlineOffset: "3px",
        backgroundColor: dark ? "#d9776c1f" : "#c2413618",
        borderRadius: "2px",
      },
      ".cm-sql-error-gutter-col": {
        width: "14px",
      },
      ".cm-sql-error-gutter-col .cm-gutterElement": {
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: "0",
      },
      ".cm-sql-error-gutter": {
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: "12px",
        height: "12px",
        borderRadius: "50%",
        backgroundColor: "var(--danger)",
        color: "#fff",
        fontSize: "9px",
        fontWeight: "700",
        lineHeight: "1",
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
  const [template, setTemplate] = useState<{
    editor: EditorView;
    state: EditorState;
    context: string;
    snippets: typeof sqlSnippets;
  } | null>(null);
  const [paramSlots, setParamSlots] = useState<QueryParamSlot[]>([]);
  const [navOverlay, setNavOverlay] = useState<{
    mode: "docs" | "usages";
    symbol: ResolvedSqlSymbol;
  } | null>(null);
  const [navDetails, setNavDetails] = useState<ObjectDetails | null>(null);
  const [navDetailsLoading, setNavDetailsLoading] = useState(false);
  const [navDetailsError, setNavDetailsError] = useState<string | null>(null);
  const [renameError, setRenameError] = useState<string | null>(null);
  const [inspectionBusy, setInspectionBusy] = useState(false);
  const [inspectionError, setInspectionError] = useState<string | null>(null);
  const [inspection, setInspection] = useState<{
    result: SqlColumnInspection;
    editor: EditorView;
    state: EditorState;
    context: string;
    sequence: number;
  } | null>(null);
  const [rename, setRename] = useState<{
    target: SqlRenameTarget;
    editor: EditorView;
    state: EditorState;
    context: string;
    sequence: number;
  } | null>(null);
  const runtimeState = useStudio((s) =>
    props.sessionId ? s.sessionStates[props.sessionId] : undefined,
  );
  const availableContexts = useStudio((s) =>
    props.sessionId ? s.sessionContexts[props.sessionId] : undefined,
  );
  const sqlSnippets = useStudio((s) => s.sqlSnippets);
  const snippetsRef = useRef(sqlSnippets);
  snippetsRef.current = sqlSnippets;
  const liveRef = useRef<CompletionCatalog | null>(null);
  const liveObjectsRef = useRef<DatabaseObjectSummary[]>([]);
  const schemaCacheRef = useRef<Map<string, TableSchema>>(new Map());
  const navRequestSeq = useRef(0);
  const inspectionRequest = useRef<number | null>(null);
  const contextLoadRef = useRef<Promise<void> | null>(null);
  const templateRequest = useRef(0);
  const clearNavigation = () => {
    navRequestSeq.current += 1;
    templateRequest.current += 1;
    setNavOverlay(null);
    setNavDetails(null);
    setNavDetailsLoading(false);
    setNavDetailsError(null);
    setRename(null);
    setTemplate(null);
    setRenameError(null);
    setInspection(null);
    inspectionRequest.current = null;
    setInspectionBusy(false);
    setInspectionError(null);
  };
  const configure = () => {
    const config = latest.current;
    const snapshot = liveRef.current;
    if (config.engine === "redis") return config;
    return {
      ...config,
      tables: snapshot?.tables ?? [],
      snippets: snippetsRef.current,
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
  const formatting = useRef<EditorFormatJob | null>(null);

  const loadTableSchemaCached = async (
    sessionId: string,
    table: TableRef,
    contextRevision?: string,
  ): Promise<TableSchema> => {
    const key = schemaCacheKey(table);
    const cache = schemaCacheRef.current;
    const cached = cache.get(key);
    if (cached) return cached;
    const loaded = contextRevision
      ? await client.call<TableSchema>("completion_schema", {
          sessionId,
          table,
          revision: contextRevision,
        })
      : await client.call<TableSchema>("table_schema", { sessionId, table });
    if (
      schemaCacheRef.current === cache &&
      latest.current.sessionId === sessionId &&
      (!contextRevision || liveRef.current?.revision === contextRevision)
    )
      cache.set(key, loaded);
    return loaded;
  };

  const buildSymbolContext = (): SqlSymbolContext => {
    const config = latest.current;
    const snapshot = liveRef.current;
    return {
      engine: config.engine,
      tables: snapshot?.tables ?? [],
      databaseObjects: liveObjectsRef.current,
      schemas: schemaCacheRef.current,
      database: snapshot?.identity.database ?? config.database,
      defaultSchema: config.defaultSchema,
      searchPath: snapshot?.identity.searchPath,
      unqualifiedTables: snapshot?.unqualifiedTables,
      caseSensitiveTableNames: snapshot?.identity.caseSensitiveTableNames ?? undefined,
    };
  };

  const revealEditorRange = (fromUtf16: number, toUtf16: number) => {
    const editor = view.current;
    if (!editor) return;
    const max = editor.state.doc.length;
    const from = Math.max(0, Math.min(max, Math.trunc(fromUtf16)));
    const to = Math.max(from, Math.min(max, Math.trunc(toUtf16)));
    editor.dispatch({
      selection: to > from ? { anchor: to, head: from } : { anchor: from },
      effects: EditorView.scrollIntoView(from, { y: "center" }),
    });
    editor.focus();
  };

  const resolveAtPosition = async (pos?: number): Promise<ResolvedSqlSymbol | null> => {
    const editor = view.current;
    if (!editor || latest.current.engine === "redis") return null;
    const document = editor.state.doc;
    const sessionAtStart = latest.current.sessionId;
    const request = navRequestSeq.current;
    const targetPos = pos ?? editor.state.selection.main.head;
    await contextLoadRef.current;
    if (
      view.current !== editor ||
      editor.state.doc !== document ||
      latest.current.sessionId !== sessionAtStart ||
      navRequestSeq.current !== request
    )
      return null;
    const initialCtx = buildSymbolContext();
    const sessionId = latest.current.sessionId;
    if (sessionId) {
      const needed = tablesNeededForNavigationAt(editor.state, targetPos, initialCtx).filter(
        (t) => !schemaCacheRef.current.has(schemaCacheKey(t)),
      );
      if (needed.length > 0) {
        const rev = liveRef.current?.revision;
        await Promise.all(
          needed
            .slice(0, 8)
            .map((table) => loadTableSchemaCached(sessionId, table, rev).catch(() => null)),
        );
      }
    }
    const currentEditor = view.current;
    if (
      !currentEditor ||
      currentEditor !== editor ||
      currentEditor.state.doc !== document ||
      latest.current.sessionId !== sessionAtStart ||
      navRequestSeq.current !== request
    )
      return null;
    return resolveSymbolAt(
      currentEditor.state,
      Math.min(targetPos, currentEditor.state.doc.length),
      buildSymbolContext(),
    );
  };

  const performGoToSymbol = (symbol: ResolvedSqlSymbol) => {
    if (symbol.kind === "cte" || symbol.kind === "alias" || symbol.kind === "projection") {
      clearNavigation();
      revealEditorRange(symbol.declarationSpan.from, symbol.declarationSpan.to);
      return;
    }
    if (!latest.current.onOpenObject) {
      useStudio
        .getState()
        .notify(
          "Object structure navigation is unavailable for this query session. Use quick SQL documentation.",
        );
      return;
    }
    clearNavigation();
    if (symbol.kind === "column") {
      latest.current.onOpenObject(symbol.object, symbol.name, liveRef.current?.revision);
    } else {
      latest.current.onOpenObject(symbol.object, undefined, liveRef.current?.revision);
    }
  };

  const runGoToDefinition = async (pos?: number): Promise<boolean> => {
    const reqId = ++navRequestSeq.current;
    const symbol = await resolveAtPosition(pos);
    if (reqId !== navRequestSeq.current) return false;
    if (!symbol) {
      useStudio
        .getState()
        .notify(
          "No unambiguous SQL symbol found. Use a qualified name or refresh the connection metadata.",
        );
      return false;
    }
    performGoToSymbol(symbol);
    return true;
  };

  const runOpenOverlay = async (mode: "docs" | "usages", pos?: number): Promise<boolean> => {
    const reqId = ++navRequestSeq.current;
    const symbol = await resolveAtPosition(pos);
    if (reqId !== navRequestSeq.current) return false;
    if (!symbol) {
      useStudio
        .getState()
        .notify(
          "No unambiguous SQL symbol found. Use a qualified name or refresh the connection metadata.",
        );
      return false;
    }
    setNavOverlay({ mode, symbol });
    setNavDetails(null);
    setNavDetailsError(null);
    const catalogObject =
      symbol.kind === "object"
        ? symbol.object
        : symbol.kind === "column"
          ? symbol.object
          : symbol.kind === "alias"
            ? symbol.object
            : null;
    const sessionId = latest.current.sessionId;
    if (catalogObject && sessionId && latest.current.objectExplorerSupported) {
      setNavDetailsLoading(true);
      void client
        .call<ObjectDetails>("object_details", { sessionId, object: catalogObject })
        .then((details) => {
          if (navRequestSeq.current === reqId) setNavDetails(details);
        })
        .catch((error) => {
          if (navRequestSeq.current === reqId) setNavDetailsError(messageOf(error));
        })
        .finally(() => {
          if (navRequestSeq.current === reqId) setNavDetailsLoading(false);
        });
    } else {
      setNavDetailsLoading(false);
    }
    return true;
  };

  const renameContext = () =>
    JSON.stringify([
      latest.current.engine,
      latest.current.sessionId,
      latest.current.contextEpoch,
      liveRef.current?.revision,
    ]);
  const insertTemplate = async (
    body: string,
    editor: EditorView,
    option: Completion | undefined,
    from: number,
    to: number,
  ) => {
    const request = ++templateRequest.current;
    const context = renameContext();
    const snippets = snippetsRef.current;
    const engine = latest.current.engine;
    await applySqlTemplate(body, engine, editor, option, from, to, {
      isCurrent: () =>
        templateRequest.current === request &&
        view.current === editor &&
        renameContext() === context &&
        snippetsRef.current === snippets,
      resolveContext: async () => {
        await contextLoadRef.current;
        if (renameContext() !== context || view.current !== editor)
          throw new Error("The console context changed. Insert the template again.");
        const sessionId = latest.current.sessionId;
        if (!sessionId)
          throw new Error(
            "Connect this console before using database or schema template variables.",
          );
        const { state: runtime } = await client.call<SessionContextOptions>("session_contexts", {
          sessionId,
        });
        if (runtime.sessionId !== sessionId || runtime.transactionState === "failed")
          throw new Error(
            "Current console context is unavailable. Recover the transaction and try again.",
          );
        return { database: runtime.database, schema: runtime.activeContext };
      },
    });
    editor.focus();
  };
  const runSqlRename = () => {
    clearNavigation();
    const editor = view.current;
    if (!editor || latest.current.engine === "redis") return false;
    try {
      const target = analyzeSqlRename(
        editor.state,
        editor.state.selection.main.head,
        latest.current.engine,
      );
      setRename({
        target,
        editor,
        state: editor.state,
        context: renameContext(),
        sequence: navRequestSeq.current,
      });
    } catch (cause) {
      setRenameError(messageOf(cause));
    }
    return true;
  };
  const closeRename = () => {
    const editor = rename?.editor;
    setRename(null);
    queueMicrotask(() => {
      if (editor && view.current === editor) editor.focus();
    });
  };
  const applyRename = (name: string) => {
    if (!rename) return;
    try {
      if (
        rename.editor !== view.current ||
        rename.context !== renameContext() ||
        rename.sequence !== navRequestSeq.current
      )
        throw new Error("The SQL context changed. Start a new rename preview.");
      const changes = rename.target.preview(rename.editor.state, name);
      rename.editor.dispatch({
        changes,
        annotations: isolateHistory.of("full"),
        userEvent: "input.rename",
      });
      closeRename();
    } catch (cause) {
      closeRename();
      setRenameError(messageOf(cause));
    }
  };

  const runColumnInspection = async () => {
    clearNavigation();
    const editor = view.current;
    const sessionId = latest.current.sessionId;
    if (!editor || !sessionId) {
      setInspectionError("Connect this console before inspecting columns.");
      return;
    }
    const state = editor.state;
    const request = navRequestSeq.current;
    inspectionRequest.current = request;
    const current = () =>
      view.current === editor &&
      editor.state.doc.eq(state.doc) &&
      latest.current.sessionId === sessionId &&
      request === navRequestSeq.current;
    setInspectionBusy(true);
    try {
      await contextLoadRef.current;
      if (!current()) return;
      const snapshot = liveRef.current;
      if (!snapshot) throw new Error("Current console metadata is unavailable. Refresh and retry.");
      const context = renameContext();
      const plan = prepareColumnInspection(state, state.selection.main.head, buildSymbolContext());
      // Completion caches may be permission-filtered or out of date. This
      // separate read requires complete metadata on the active session lane.
      const schemas: TableSchema[] = [];
      let columns = 0,
        bytes = 0;
      for (const table of plan.tables) {
        if (!current() || renameContext() !== context) return;
        const schema = await client.call<TableSchema>("inspection_schema", {
          sessionId,
          revision: snapshot.revision,
          table,
        });
        if (!current() || renameContext() !== context) return;
        columns += schema.columns.length;
        bytes += new TextEncoder().encode(JSON.stringify(schema)).length;
        if (
          schema.columns.length > columnInspectionLimits.columnsPerTable ||
          columns > columnInspectionLimits.columns ||
          bytes > columnInspectionLimits.bytes
        )
          throw new Error(
            "Complete table metadata exceeds the inspection limit; no column claim was made.",
          );
        schemas.push(schema);
      }
      if (!current() || renameContext() !== context) return;
      setInspection({
        result: plan.finish(editor.state, schemas),
        editor,
        state,
        context,
        sequence: request,
      });
    } catch (error) {
      if (current()) setInspectionError(`Column inspection unavailable: ${messageOf(error)}`);
    } finally {
      // A navigation request can supersede the result without starting a new
      // inspection. Release only this request's busy state in that case.
      if (inspectionRequest.current === request && view.current === editor) {
        inspectionRequest.current = null;
        setInspectionBusy(false);
      }
    }
  };
  const closeInspection = () => {
    clearNavigation();
    queueMicrotask(() => view.current?.focus());
  };
  const applyColumnCorrection = (issue: MissingSqlColumn, column: string) => {
    if (!inspection) return;
    try {
      if (
        view.current !== inspection.editor ||
        renameContext() !== inspection.context ||
        navRequestSeq.current !== inspection.sequence
      )
        throw new Error("Inspection context changed. Inspect the current statement again.");
      const change = inspection.result.preview(inspection.editor.state, issue, column);
      inspection.editor.dispatch({
        changes: change,
        annotations: isolateHistory.of("full"),
        userEvent: "input.column-correction",
      });
      closeInspection();
    } catch (error) {
      closeInspection();
      setInspectionError(messageOf(error));
    }
  };
  const navActionsRef = useRef({
    runGoToDefinition,
    runOpenOverlay,
    runSqlRename,
  });
  navActionsRef.current = {
    runGoToDefinition,
    runOpenOverlay,
    runSqlRename,
  };

  useImperativeHandle(
    ref,
    () => ({
      selection() {
        const editor = view.current;
        if (!editor) return latest.current.value;
        return latest.current.engine === "redis"
          ? redisSelectionSlice(editor.state).sql
          : selectedStatement(editor.state);
      },
      executionSlice() {
        const editor = view.current;
        if (!editor) return { sql: latest.current.value, fromUtf16: 0 };
        if (latest.current.engine === "redis") {
          return redisSelectionSlice(editor.state);
        }
        const slice = selectedStatementSlice(editor.state);
        return { sql: slice.sql, fromUtf16: slice.fromUtf16 };
      },
      revealRange(fromUtf16: number, toUtf16: number) {
        revealEditorRange(fromUtf16, toUtf16);
      },
      navigateStatement(direction: -1 | 1) {
        const editor = view.current;
        if (!editor || latest.current.engine === "redis") return false;
        const moved = moveCaretToAdjacentStatement(editor, direction);
        if (moved) editor.focus();
        return moved;
      },
      goToDefinition() {
        return navActionsRef.current.runGoToDefinition();
      },
      findUsages() {
        return navActionsRef.current.runOpenOverlay("usages");
      },
      showQuickDocs() {
        return navActionsRef.current.runOpenOverlay("docs");
      },
      focus() {
        view.current?.focus();
      },
      format() {
        const editor = view.current;
        const language = sqlFormatterLanguage(latest.current.engine);
        if (!editor || !language) return;
        if (!formatter.current) {
          formatter.current = new Worker(
            new URL("../../workers/formatter.worker.ts", import.meta.url),
            { type: "module" },
          );
          formatter.current.onmessage = ({
            data,
          }: MessageEvent<{ id: number; sql?: string; error?: string }>) => {
            const job = formatting.current;
            if (!job || data.id !== job.id) return;
            formatting.current = null;
            if (data.error) {
              useStudio.getState().notify(data.error);
              return;
            }
            const currentEditor = view.current;
            if (!currentEditor || data.sql === undefined) return;
            const replacement = resolveEditorFormatReplacement(currentEditor.state, job, data.sql);
            if (!replacement) return;
            currentEditor.dispatch({
              changes: {
                from: replacement.from,
                to: replacement.to,
                insert: replacement.insert,
              },
              ...(replacement.selection ? { selection: replacement.selection } : {}),
            });
          };
        }
        const job = prepareEditorFormatJob(editor.state, ++sequence.current);
        formatting.current = job;
        formatter.current.postMessage({
          id: job.id,
          sql: job.source,
          language,
          settings: useStudio.getState().sqlFormatSettings,
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
        loadTableSchemaCached(sessionId, table, contextRevision),
      (error) => useStudio.getState().notify(messageOf(error)),
      (body, editor, option, from, to) => {
        void insertTemplate(body, editor, option, from, to).catch((error) =>
          useStudio.getState().notify(messageOf(error)),
        );
      },
    );
    completion.current = session;
    session.configure(configure());
    worker.onmessage = ({ data }: MessageEvent<CompletionResponse>) => session.receive(data);
    const state = EditorState.create({
      doc: latest.current.value,
      extensions: [
        errorStateField,
        errorGutterExtension,
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
                const slice =
                  latest.current.engine === "redis"
                    ? redisSelectionSlice(editor.state)
                    : selectedStatementSlice(editor.state);
                latest.current.onRun(slice.sql, slice.fromUtf16);
              } catch (error) {
                useStudio.getState().notify(messageOf(error));
              }
              return true;
            },
          },
          {
            key: "Alt-Mod-ArrowUp",
            run: (editor) =>
              latest.current.engine !== "redis" && moveCaretToAdjacentStatement(editor, -1),
          },
          {
            key: "Ctrl-Alt-ArrowUp",
            run: (editor) =>
              latest.current.engine !== "redis" && moveCaretToAdjacentStatement(editor, -1),
          },
          {
            key: "Alt-Mod-ArrowDown",
            run: (editor) =>
              latest.current.engine !== "redis" && moveCaretToAdjacentStatement(editor, 1),
          },
          {
            key: "Ctrl-Alt-ArrowDown",
            run: (editor) =>
              latest.current.engine !== "redis" && moveCaretToAdjacentStatement(editor, 1),
          },
          {
            key: "Shift-F6",
            run: () => navActionsRef.current.runSqlRename(),
          },
          {
            key: "F12",
            run: () => {
              if (latest.current.engine === "redis") return false;
              void navActionsRef.current.runGoToDefinition();
              return true;
            },
          },
          {
            key: "Shift-F12",
            run: () => {
              if (latest.current.engine === "redis") return false;
              void navActionsRef.current.runOpenOverlay("usages");
              return true;
            },
          },
          {
            key: "F1",
            run: () => {
              if (latest.current.engine === "redis") return false;
              void navActionsRef.current.runOpenOverlay("docs");
              return true;
            },
          },
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...historyKeymap,
          ...searchKeymap,
          indentWithTab,
        ]),
        EditorView.domEventHandlers({
          mousedown(event, editorView) {
            if (
              latest.current.engine === "redis" ||
              event.button !== 0 ||
              !(event.metaKey || event.ctrlKey) ||
              event.shiftKey ||
              event.altKey
            ) {
              return false;
            }
            const pos = editorView.posAtCoords({ x: event.clientX, y: event.clientY });
            if (pos === null) return false;
            event.preventDefault();
            editorView.dispatch({ selection: { anchor: pos } });
            void navActionsRef.current.runGoToDefinition(pos);
            return true;
          },
        }),
        EditorView.updateListener.of((update) => {
          if (update.docChanged || update.selectionSet) session.update(update.state);
          if (update.docChanged) {
            formatting.current = null;
            clearNavigation();
            latest.current.onChange(update.state.doc.toString());
          }
        }),
      ],
    });
    view.current = new EditorView({ state, parent: host.current! });
    session.update(state);
    return () => {
      navRequestSeq.current += 1;
      templateRequest.current += 1;
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
    const editor = view.current;
    if (!editor) return;
    const resolved =
      props.engine === "redis"
        ? null
        : resolveExecutionError(editor.state.doc.toString(), props.error, props.errorOrigin);
    if (!resolved) {
      if (editor.state.field(errorStateField) !== null) {
        editor.dispatch({ effects: setEditorErrorEffect.of(null) });
      }
      return;
    }
    editor.dispatch({
      effects: setEditorErrorEffect.of({
        from: resolved.from,
        to: resolved.to,
        message: props.error!.message,
      }),
    });
  }, [props.engine, props.error, props.errorOrigin, props.value]);
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
    formatting.current = null;
  }, [props.engine, props.sessionId]);
  useEffect(() => {
    if (completion.current?.configure(configure()) && view.current) {
      closeCompletion(view.current);
      completion.current.update(view.current.state);
    }
  }, [
    props.tables,
    props.sessionId,
    props.engine,
    props.database,
    props.defaultSchema,
    live,
    sqlSnippets,
  ]);
  useEffect(() => {
    let cancelled = false;
    clearNavigation();
    liveRef.current = null;
    liveObjectsRef.current = [];
    schemaCacheRef.current = new Map();
    contextLoadRef.current = null;
    setLive(null);
    setContextError(null);
    if (props.engine === "redis") return;
    completion.current?.configure(configure());
    if (view.current) closeCompletion(view.current);
    if (props.sessionId) {
      void useStudio.getState().refreshSessionContexts(props.sessionId);
      contextLoadRef.current = client
        .call<CompletionCatalog>("completion_catalog", { sessionId: props.sessionId })
        .then(async (snapshot) => {
          if (!cancelled) {
            liveRef.current = snapshot;
            setLive(snapshot);
            if (props.objectExplorerSupported) {
              try {
                const objects = await client.call<DatabaseObjectSummary[]>("database_objects", {
                  sessionId: props.sessionId,
                });
                if (!cancelled) liveObjectsRef.current = objects;
              } catch {
                /* Catalog privileges may be narrower than SQL access. */
              }
            }
          }
        })
        .catch((error) => {
          if (!cancelled) setContextError(messageOf(error));
        });
    }
    return () => {
      cancelled = true;
    };
  }, [
    props.engine,
    props.sessionId,
    props.tables,
    props.contextEpoch,
    props.objectExplorerSupported,
    refresh,
  ]);
  useEffect(() => {
    if (props.engine === "redis" || !props.value.trim()) {
      setParamSlots([]);
      return;
    }
    let active = true;
    const timer = setTimeout(() => {
      void client
        .call<QueryParamSlot[]>("extract_query_params", {
          engine: props.engine,
          sql: props.value,
        })
        .then((slots) => {
          if (active) setParamSlots(slots);
        })
        .catch(() => {
          if (active) setParamSlots([]);
        });
    }, 80);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [props.engine, props.value]);
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
    <div
      className="sql-editor"
      style={{ display: "flex", flexDirection: "column", position: "relative" }}
    >
      {props.engine !== "redis" &&
        (props.sessionId || paramSlots.length > 0 || props.value.trim()) && (
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
                {!props.sessionId
                  ? "Connect to run parameterized SQL"
                  : contextError
                    ? `Completion unavailable: ${contextError}`
                    : live
                      ? `${live.identity.database || "No database"} · ${live.identity.role}${props.engine !== "mysql" ? ` · ${live.identity.searchPath.join(", ")}` : ""}`
                      : "Loading completion context…"}
              </span>
              {paramSlots.length > 0 && props.onOpenParameters && (
                <button
                  type="button"
                  className="button ghost small"
                  style={{ flexShrink: 0 }}
                  aria-label={`Configure query parameters (${paramSlots.length})`}
                  onClick={() => props.onOpenParameters?.(paramSlots)}
                >
                  Parameters ({paramSlots.length})
                </button>
              )}
              <button
                type="button"
                className="button ghost small"
                aria-label="Insert SQL template"
                onClick={() => {
                  const editor = view.current;
                  if (!editor) return;
                  clearNavigation();
                  setTemplate({
                    editor,
                    state: editor.state,
                    context: renameContext(),
                    snippets: snippetsRef.current,
                  });
                }}
              >
                Insert template
              </button>
              <button
                type="button"
                className="button ghost small"
                style={{ flexShrink: 0 }}
                aria-label="Inspect SQL columns"
                disabled={inspectionBusy}
                onClick={() => void runColumnInspection()}
              >
                {inspectionBusy ? "Inspecting…" : "Inspect columns"}
              </button>
              <button
                type="button"
                className="button ghost small"
                style={{ flexShrink: 0 }}
                title="Rename SQL alias or CTE · Shift-F6"
                aria-label="Rename SQL alias or CTE"
                onClick={() => navActionsRef.current.runSqlRename()}
              >
                Rename alias / CTE
              </button>
              <button
                type="button"
                className="button ghost small"
                style={{ flexShrink: 0 }}
                title="Quick SQL documentation · F1"
                aria-label="Quick SQL documentation"
                onClick={() => void navActionsRef.current.runOpenOverlay("docs")}
              >
                Docs
              </button>
              {props.sessionId && (
                <button
                  className="button ghost small"
                  style={{ flexShrink: 0 }}
                  aria-label="Refresh completion context"
                  onClick={() => setRefresh((n) => n + 1)}
                >
                  Refresh
                </button>
              )}
            </div>
          </div>
        )}
      {inspectionError && (
        <div role="alert" style={{ padding: 8 }}>
          {inspectionError}{" "}
          <button
            type="button"
            className="button ghost small"
            onClick={() => setInspectionError(null)}
          >
            Dismiss
          </button>
        </div>
      )}
      {template && (
        <SqlTemplateDialog
          templates={effectiveSqlSnippets(props.engine, template.snippets)}
          onClose={() => {
            templateRequest.current += 1;
            setTemplate(null);
            view.current?.focus();
          }}
          onInsert={async (body) => {
            if (
              view.current !== template.editor ||
              template.editor.state.doc !== template.state.doc ||
              !template.editor.state.selection.eq(template.state.selection) ||
              renameContext() !== template.context ||
              snippetsRef.current !== template.snippets
            )
              throw new Error("The editor or console context changed. Open Insert template again.");
            const { from, to } = template.state.selection.main;
            await insertTemplate(body, template.editor, undefined, from, to);
            setTemplate(null);
          }}
        />
      )}
      {inspection && (
        <SqlColumnInspectionDialog
          result={inspection.result}
          state={inspection.state}
          onApply={applyColumnCorrection}
          onClose={closeInspection}
        />
      )}
      {renameError && (
        <div role="alert" style={{ padding: 8 }}>
          {renameError}{" "}
          <button type="button" className="button ghost small" onClick={() => setRenameError(null)}>
            Dismiss
          </button>
        </div>
      )}
      {rename && (
        <SqlAliasRenameDialog
          target={rename.target}
          state={rename.state}
          onApply={applyRename}
          onClose={closeRename}
        />
      )}
      {navOverlay && (
        <SqlNavigationOverlay
          mode={navOverlay.mode}
          symbol={navOverlay.symbol}
          objectDetails={navDetails}
          objectDetailsLoading={navDetailsLoading}
          objectDetailsError={navDetailsError}
          objectExplorerSupported={Boolean(props.objectExplorerSupported)}
          onJumpInEditor={(from, to) => revealEditorRange(from, to)}
          onOpenDefinition={() => performGoToSymbol(navOverlay.symbol)}
          onOpenObject={
            props.onOpenObject
              ? (object, col) => {
                  clearNavigation();
                  props.onOpenObject?.(object, col, liveRef.current?.revision);
                }
              : undefined
          }
          onSwitchMode={(nextMode) =>
            setNavOverlay((prev) => (prev ? { ...prev, mode: nextMode } : null))
          }
          onClose={() => {
            clearNavigation();
            view.current?.focus();
          }}
        />
      )}
      <div ref={host} style={{ flex: 1, minHeight: 0 }} />
    </div>
  );
});

function redisSelectionSlice(state: EditorState): { sql: string; fromUtf16: number } {
  const selection = state.selection.main;
  if (!selection.empty) {
    return { sql: state.sliceDoc(selection.from, selection.to), fromUtf16: selection.from };
  }
  const line = state.doc.lineAt(selection.head);
  return { sql: line.text, fromUtf16: line.from };
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
