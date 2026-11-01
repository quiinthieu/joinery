import type { EditorState, Text } from "@codemirror/state";
import { format } from "sql-formatter";
import type { EngineKind, SqlFormatSettings } from "../ipc/generated";

export type SqlFormatterDialect = "postgresql" | "mysql" | "snowflake";

export const DEFAULT_SQL_FORMAT_SETTINGS: SqlFormatSettings = {
  keywordCase: "upper",
  indentWidth: 2,
  linesBetweenQueries: 1,
};

export function sqlFormatterLanguage(engine: EngineKind): SqlFormatterDialect | null {
  switch (engine) {
    case "postgres":
      return "postgresql";
    case "mysql":
      return "mysql";
    case "snowflake":
      return "snowflake";
    case "redis":
      return null;
  }
}

export function formatSqlText(
  sql: string,
  language: SqlFormatterDialect,
  settings: SqlFormatSettings = DEFAULT_SQL_FORMAT_SETTINGS,
): string {
  return format(sql, {
    language,
    keywordCase: settings.keywordCase,
    tabWidth: settings.indentWidth,
    linesBetweenQueries: settings.linesBetweenQueries,
    // Match Joinery's bind syntax without consuming casts or Snowflake paths.
    // Keeping the original token text also preserves case-sensitive bind keys.
    paramTypes: {
      custom: [{ regex: "(?<![A-Za-z0-9_$'\"`@:]):[A-Za-z_][A-Za-z0-9_]*" }],
      numbered: language === "mysql" ? [] : ["$"],
      positional: language !== "postgresql",
    },
  });
}

export interface EditorFormatJob {
  id: number;
  document: Text;
  from: number;
  to: number;
  source: string;
  isSelection: boolean;
}

export interface EditorFormatReplacement {
  from: number;
  to: number;
  insert: string;
  selection?: { anchor: number; head: number };
}

export function prepareEditorFormatJob(state: EditorState, id: number): EditorFormatJob {
  const main = state.selection.main;
  if (!main.empty) {
    return {
      id,
      document: state.doc,
      from: main.from,
      to: main.to,
      source: state.sliceDoc(main.from, main.to),
      isSelection: true,
    };
  }
  return {
    id,
    document: state.doc,
    from: 0,
    to: state.doc.length,
    source: state.doc.toString(),
    isSelection: false,
  };
}

export function resolveEditorFormatReplacement(
  state: EditorState,
  job: EditorFormatJob,
  formattedSql: string,
): EditorFormatReplacement | null {
  // Any edit or replacement invalidates the worker's offsets, even if the
  // selected text happens to remain identical at those offsets.
  if (state.doc !== job.document) return null;
  if (job.isSelection) {
    if (
      job.from < 0 ||
      job.to > state.doc.length ||
      state.doc.sliceString(job.from, job.to) !== job.source
    ) {
      return null;
    }
    const trailingBreak = job.source.endsWith("\r\n")
      ? "\r\n"
      : job.source.endsWith("\n")
        ? "\n"
        : "";
    const insert =
      trailingBreak && !/\r?\n$/.test(formattedSql)
        ? `${formattedSql}${trailingBreak}`
        : formattedSql;
    return {
      from: job.from,
      to: job.to,
      insert,
      selection: { anchor: job.from, head: job.from + insert.length },
    };
  }
  if (state.doc.toString() !== job.source) {
    return null;
  }
  return {
    from: 0,
    to: state.doc.length,
    insert: formattedSql,
  };
}
