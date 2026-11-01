import { expect, test, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { sql } from "@codemirror/lang-sql";
import {
  CompletionContext,
  autocompletion,
  hasNextSnippetField,
  nextSnippetField,
} from "@codemirror/autocomplete";
import type { SqlSnippet, TableInfo, TableSchema } from "../../ipc/generated";
import {
  DEFAULT_SQL_FORMAT_SETTINGS,
  formatSqlText,
  prepareEditorFormatJob,
  resolveEditorFormatReplacement,
  sqlFormatterLanguage,
} from "../../data/sql-format";
import { CompletionWorkerIndex } from "../../data/completion-protocol";
import { CompletionSession } from "./completion-session";
import { completionTarget } from "./sql-context";
import { sqlDialect } from "./sql-identifiers";
import { builtInSqlSnippets, effectiveSqlSnippets, snippetCompletions } from "./sql-snippets";

test("formats SQL text honoring keyword case, indent width, and lines between queries", () => {
  expect(sqlFormatterLanguage("postgres")).toBe("postgresql");
  expect(sqlFormatterLanguage("mysql")).toBe("mysql");
  expect(sqlFormatterLanguage("snowflake")).toBe("snowflake");
  expect(sqlFormatterLanguage("redis")).toBeNull();

  const raw = "select id, total from orders where status = 'paid'; select count(*) from orders;";
  const upperDefault = formatSqlText(raw, "postgresql", DEFAULT_SQL_FORMAT_SETTINGS);
  expect(upperDefault).toContain("SELECT\n  id,");
  expect(upperDefault).toContain(";\n\nSELECT");

  const lowerFourSpacesTwoLines = formatSqlText(raw, "postgresql", {
    keywordCase: "lower",
    indentWidth: 4,
    linesBetweenQueries: 2,
  });
  expect(lowerFourSpacesTwoLines).toContain("select\n    id,");
  expect(lowerFourSpacesTwoLines).toContain(";\n\n\nselect");

  const preserved = formatSqlText("Select id From orders", "mysql", {
    keywordCase: "preserve",
    indentWidth: 2,
    linesBetweenQueries: 1,
  });
  expect(preserved).toContain("Select\n  id\nFrom\n  orders");
});

test("dialect styles override global formatting without leaking into another engine", () => {
  const settings = {
    ...DEFAULT_SQL_FORMAT_SETTINGS,
    dialects: {
      postgres: { keywordCase: "lower" as const, indentWidth: 4, linesBetweenQueries: 2 },
      snowflake: { keywordCase: "preserve" as const, indentWidth: 4, linesBetweenQueries: 1 },
    },
  };
  const raw = "Select id From orders; Select total From orders;";
  expect(formatSqlText(raw, "postgresql", settings)).toContain("select\n    id");
  expect(formatSqlText(raw, "postgresql", settings)).toContain(";\n\n\nselect");
  expect(formatSqlText(raw, "mysql", settings)).toContain("SELECT\n  id");
  expect(formatSqlText(raw, "snowflake", settings)).toContain("Select\n    id");
});

test("selection formatting replaces only the selected range and preserves trailing line breaks", () => {
  const before = "select   1  from dual;\n";
  const middle = "select id, total from orders where total > 10;\n";
  const after = "select   2  from dual;";
  const doc = `${before}${middle}${after}`;
  const state = EditorState.create({
    doc,
    selection: { anchor: before.length, head: before.length + middle.length },
  });

  const job = prepareEditorFormatJob(state, 7);
  expect(job.isSelection).toBe(true);
  expect(job.source).toBe(middle);

  const formattedMiddle = formatSqlText(job.source, "postgresql", DEFAULT_SQL_FORMAT_SETTINGS);
  const replacement = resolveEditorFormatReplacement(state, job, formattedMiddle)!;
  expect(replacement).not.toBeNull();
  expect(replacement.insert.endsWith("\n")).toBe(true);

  const next = state.update({
    changes: {
      from: replacement.from,
      to: replacement.to,
      insert: replacement.insert,
    },
    selection: replacement.selection,
  }).state;
  const text = next.doc.toString();
  expect(text.startsWith(before)).toBe(true);
  expect(text.endsWith(after)).toBe(true);
  expect(text).toContain("SELECT\n  id,\n  total\nFROM\n  orders");
  expect(next.selection.main.from).toBe(before.length);
  expect(next.selection.main.to).toBe(before.length + replacement.insert.length);

  // Stale worker reply after user edits inside the selected range is rejected.
  const mutated = state.update({
    changes: { from: before.length, to: before.length + 6, insert: "SELECT" },
  }).state;
  expect(resolveEditorFormatReplacement(mutated, job, formattedMiddle)).toBeNull();
});

test.each(["postgresql", "mysql", "snowflake"] as const)(
  "%s formatting preserves bind names, literals and comments",
  (language) => {
    const formatted = formatSqlText(
      "select :lookup, :Camel_case, ':literal 🧪 日本語' as note -- :ignored\n/* :also_ignored */ where :lookup = 7;",
      language,
    );
    expect(formatted.match(/:lookup/g)).toHaveLength(2);
    expect(formatted).toContain(":Camel_case");
    expect(formatted).toContain("':literal 🧪 日本語'");
    expect(formatted).toContain("-- :ignored");
    expect(formatted).toContain("/* :also_ignored */");
    expect(formatted).toContain(":lookup = 7");
    const positional = formatSqlText(
      language === "postgresql" ? "select $1, :lookup" : "select ?, :lookup",
      language,
    );
    expect(positional).toContain(language === "postgresql" ? "$1" : "?");
    if (language === "snowflake") {
      const path = formatSqlText("select payload:field, $1, :lookup from events", language);
      expect(path).toContain("payload:field");
      expect(path).toContain("$1");
    }
    if (language === "postgresql") {
      const cast = formatSqlText("select (:lookup)::int, payload::jsonb ? 'key'", language);
      expect(cast).toContain("::int");
      expect(cast).toContain("payload::jsonb ? 'key'");
    }
  },
);

test("built-in and custom snippets respect dialect safety and override precedence", () => {
  const pgBuiltins = builtInSqlSnippets("postgres");
  const mysqlBuiltins = builtInSqlSnippets("mysql");
  const snowBuiltins = builtInSqlSnippets("snowflake");
  expect(builtInSqlSnippets("redis")).toEqual([]);

  expect(pgBuiltins.find((s) => s.trigger === "txn")?.body).toContain("BEGIN;");
  expect(mysqlBuiltins.find((s) => s.trigger === "txn")?.body).toContain("START TRANSACTION;");
  expect(snowBuiltins.some((s) => ["ins", "upd", "del", "txn"].includes(s.trigger))).toBe(false);
  expect(snowBuiltins.some((s) => s.trigger === "sel")).toBe(true);

  const userSnippets: SqlSnippet[] = [
    {
      id: "1",
      trigger: "sel",
      title: "Global select override",
      body: "SELECT ${1:id} FROM ${2:items};",
      engine: null,
      updatedAtMs: 1,
    },
    {
      id: "2",
      trigger: "sel",
      title: "Postgres select override",
      body: "SELECT ${1:id} FROM public.${2:items};",
      engine: "postgres",
      updatedAtMs: 2,
    },
    {
      id: "3",
      trigger: "recent_orders",
      title: "Recent orders",
      body: "SELECT * FROM orders ORDER BY created_at DESC LIMIT ${1:25};",
      engine: "postgres",
      updatedAtMs: 3,
    },
  ];

  const pgEffective = effectiveSqlSnippets("postgres", userSnippets);
  expect(pgEffective.find((s) => s.trigger === "sel")?.title).toBe("Postgres select override");
  expect(pgEffective.some((s) => s.trigger === "recent_orders")).toBe(true);

  const mysqlEffective = effectiveSqlSnippets("mysql", userSnippets);
  expect(mysqlEffective.find((s) => s.trigger === "sel")?.title).toBe("Global select override");
  expect(mysqlEffective.some((s) => s.trigger === "recent_orders")).toBe(false);
});

test("formatting rejects replies after edits outside the selection and after document replacement", () => {
  const state = EditorState.create({
    doc: "select 1;\nselect 2;",
    selection: { anchor: 10, head: 19 },
  });
  const job = prepareEditorFormatJob(state, 1);
  const changedOutside = state.update({ changes: { from: 0, to: 6, insert: "SELECT" } }).state;
  expect(resolveEditorFormatReplacement(changedOutside, job, "SELECT\n  2;")).toBeNull();
  const replaced = EditorState.create({ doc: state.doc.toString(), selection: state.selection });
  expect(resolveEditorFormatReplacement(replaced, job, "SELECT\n  2;")).toBeNull();
});

test("live templates never fire after alias qualifiers and expand CodeMirror tab stops cleanly", async () => {
  const qualifiedState = EditorState.create({
    doc: "SELECT o.sel FROM orders o",
    selection: { anchor: "SELECT o.sel".length },
    extensions: [sql({ dialect: sqlDialect("postgres") })],
  });
  const qualifiedTarget = completionTarget(qualifiedState, qualifiedState.selection.main.head)!;
  expect(qualifiedTarget.parts).toHaveLength(2);
  expect(snippetCompletions("postgres", qualifiedTarget, [])).toEqual([]);

  const worker = new CompletionWorkerIndex();
  const tables: TableInfo[] = [
    {
      table: { schema: "public", name: "orders" },
      kind: "BASE TABLE",
      estimatedRows: null,
      totalBytes: null,
    },
  ];
  const session = new CompletionSession(
    (msg) => {
      const res = worker.handle(msg);
      queueMicrotask(() => session.receive(res));
    },
    vi.fn(async () => ({ table: tables[0].table, columns: [] }) as unknown as TableSchema),
    vi.fn(),
  );
  const custom: SqlSnippet = {
    id: "custom-1",
    trigger: "by_status",
    title: "Orders by status",
    body: "SELECT ${1:*} FROM orders WHERE status = '${2:paid}';${0}",
    engine: "postgres",
    updatedAtMs: 10,
  };
  session.configure({
    engine: "postgres",
    sessionId: "fixture",
    tables,
    snippets: [custom],
  });

  const host = document.createElement("div");
  const editorView = new EditorView({
    parent: host,
    state: EditorState.create({
      doc: "by_status",
      selection: { anchor: "by_status".length },
      extensions: [sql({ dialect: sqlDialect("postgres") }), autocompletion()],
    }),
  });

  const result = (await session.request(
    new CompletionContext(editorView.state, editorView.state.selection.main.head, true),
  ))!;
  expect(result.options[0].label).toBe("by_status");
  expect(typeof result.options[0].apply).toBe("function");

  (
    result.options[0].apply as (
      view: EditorView,
      completion: unknown,
      from: number,
      to: number,
    ) => void
  )(editorView, result.options[0], result.from, result.to ?? editorView.state.doc.length);

  expect(editorView.state.doc.toString()).toBe("SELECT * FROM orders WHERE status = 'paid';");
  expect(
    editorView.state.sliceDoc(
      editorView.state.selection.main.from,
      editorView.state.selection.main.to,
    ),
  ).toBe("*");
  expect(hasNextSnippetField(editorView.state)).toBe(true);

  expect(nextSnippetField(editorView)).toBe(true);
  expect(
    editorView.state.sliceDoc(
      editorView.state.selection.main.from,
      editorView.state.selection.main.to,
    ),
  ).toBe("paid");

  expect(nextSnippetField(editorView)).toBe(true);
  expect(editorView.state.selection.main.head).toBe(editorView.state.doc.length);

  editorView.destroy();
  session.dispose();
});
