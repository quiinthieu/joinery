import type { EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import type { TableRef, TableSchema } from "../../ipc/generated";
import { statementTokensAt, tableLookup, type SqlToken } from "./sql-context";
import { identifierValue, readIdentifier } from "./sql-identifiers";
import type { SqlSymbolContext } from "./sql-navigation";

export interface MissingSqlColumn {
  name: string;
  from: number;
  to: number;
  line: number;
}

export interface SqlColumnInspection {
  table: TableRef;
  checked: number;
  issues: MissingSqlColumn[];
  columns: string[];
  preview(
    state: EditorState,
    issue: MissingSqlColumn,
    column: string,
  ): {
    from: number;
    to: number;
    insert: string;
  };
}

/** Deliberately bounded: one direct table, no query nesting or JOIN inference. */
export function prepareColumnInspection(
  state: EditorState,
  pos: number,
  context: SqlSymbolContext,
) {
  const unsupported = () =>
    new Error(
      "Column inspection supports a single direct-table SELECT without joins or subqueries.",
    );
  if (context.engine !== "postgres" && context.engine !== "mysql") throw unsupported();
  const statement = statementTokensAt(state, pos);
  if (!statement) throw new Error("Statement is incomplete or exceeds the inspection limit.");
  const clean = (tokens: readonly SqlToken[]) => tokens.filter((t) => !t.kind.endsWith("Comment"));
  const tokens = clean(statement.tokens);
  const nameToken = (token?: SqlToken): token is SqlToken =>
    !!token && ["Identifier", "QuotedIdentifier"].includes(token.kind);
  const checkName = (token: SqlToken) => {
    const name = readIdentifier(token.text).value;
    if (
      (context.engine === "postgres" && new TextEncoder().encode(name).length > 63) ||
      (context.engine === "mysql" && [...name].length > 64)
    )
      throw unsupported();
  };
  const word = (token?: SqlToken) => (token?.kind === "Keyword" ? token.text.toUpperCase() : "");
  if (word(tokens[0]) !== "SELECT") throw unsupported();
  const tree = ensureSyntaxTree(state, state.doc.length, 75);
  if (!tree) throw unsupported();
  let invalid = false;
  tree.iterate({
    from: tokens[0].from,
    to: tokens.at(-1)!.to,
    enter(node) {
      if (
        node.type.isError &&
        !(node.from === node.to && state.sliceDoc(node.from - 1, node.from + 1) === ".*")
      )
        invalid = true;
    },
  });
  if (invalid) throw unsupported();
  const from = tokens.findIndex((token) => word(token) === "FROM");
  if (from < 0) throw unsupported();
  let next = from + 1;
  const relation: SqlToken[] = [];
  if (!nameToken(tokens[next])) throw unsupported();
  relation.push(tokens[next++]);
  while (tokens[next]?.text === "." && nameToken(tokens[next + 1])) {
    relation.push(tokens[next + 1]);
    next += 2;
  }
  if (relation.length > 2) throw unsupported();
  if (word(tokens[next]) === "AS") next++;
  const alias = nameToken(tokens[next]) ? tokens[next++] : undefined;
  for (const token of [...relation, ...(alias ? [alias] : [])]) checkName(token);
  if (
    tokens[next] &&
    !["WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "FOR", "FETCH"].includes(
      word(tokens[next]),
    ) &&
    tokens[next].text !== ";"
  )
    throw unsupported();
  // Unqualified relations require the server-observed visible-table set. Never
  // substitute a guessed default schema when deciding that a column is absent.
  if (relation.length === 1 && context.unqualifiedTables === undefined)
    throw new Error("Visible-table metadata is unavailable; no column claim was made.");
  const lookup = tableLookup(
    [...context.tables],
    context.engine,
    context.database,
    undefined,
    undefined,
    context.unqualifiedTables,
    context.caseSensitiveTableNames,
  );
  const table = lookup.resolve(relation.map((token) => readIdentifier(token.text)));
  const info =
    table &&
    context.tables.find(
      (item) => item.table.schema === table.schema && item.table.name === table.name,
    );
  if (!table || !info || !["table", "partitionedTable", "r", "p", "BASE TABLE"].includes(info.kind))
    throw new Error("A unique direct base table could not be resolved; no column claim was made.");
  const key = (raw: string) => {
    const value = identifierValue(readIdentifier(raw), context.engine);
    return context.engine === "mysql" && context.caseSensitiveTableNames === false
      ? value.toLowerCase()
      : value;
  };
  const qualifier = key((alias ?? relation.at(-1))!.text);
  const references: MissingSqlColumn[] = [];
  const forbidden = new Set([
    "SELECT",
    "WITH",
    "JOIN",
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "INTO",
    "LATERAL",
    "APPLY",
    "WINDOW",
    "QUALIFY",
    "TABLESAMPLE",
  ]);
  let visited = 0;
  function scan(raw: readonly SqlToken[], depth: number) {
    if (depth > 16) throw unsupported();
    const list = clean(raw);
    for (let i = 0; i < list.length; i++) {
      if (++visited > 4096) throw unsupported();
      const token = list[i];
      if (forbidden.has(word(token))) throw unsupported();
      if (token.children) {
        if (!token.text.endsWith(")")) throw unsupported();
        scan(token.children, depth + 1);
        continue;
      }
      if (!nameToken(token) || list[i + 1]?.text !== ".") continue;
      const column = list[i + 2];
      if (column?.text === "*") {
        i += 2;
        continue;
      }
      if (!nameToken(column) || list[i + 3]?.text === ".") throw unsupported();
      checkName(token);
      checkName(column);
      // Qualified function names are not column references; their arguments
      // are still visited on the next iteration.
      if (!list[i + 3]?.children) {
        if (
          context.engine === "mysql" &&
          readIdentifier(column.text).value.toLowerCase() === "_rowid"
        )
          throw unsupported();
        if (key(token.text) !== qualifier)
          throw new Error(
            "An unresolved qualifier prevents this inspection; no column claim was made.",
          );
        references.push({
          name: identifierValue(readIdentifier(column.text), context.engine),
          from: column.from,
          to: column.to,
          line: state.doc.lineAt(column.from).number,
        });
      }
      i += 2;
    }
  }
  scan(tokens.slice(1, from), 0);
  scan(tokens.slice(next), 0);
  if (!references.length)
    throw new Error("No supported qualified columns to inspect in this SELECT.");
  const assertCurrent = (current: EditorState) => {
    if (!current.doc.eq(state.doc))
      throw new Error("SQL changed. Inspect the current statement again.");
  };
  return {
    table,
    finish(current: EditorState, schema: TableSchema): SqlColumnInspection {
      assertCurrent(current);
      if (
        schema.table.schema !== table.schema ||
        schema.table.name !== table.name ||
        !schema.columns.length ||
        schema.columns.length > 2048 ||
        new Set(schema.columns.map((column) => column.name)).size !== schema.columns.length
      )
        throw new Error("Complete table metadata is unavailable; no column claim was made.");
      const columns = schema.columns.map((column) => column.name);
      const columnKey = (name: string) => (context.engine === "mysql" ? name.toLowerCase() : name);
      const existing = new Set(columns.map(columnKey));
      // PostgreSQL exposes these physical system columns outside attnum > 0.
      // They are valid references, but are not offered as catalog corrections.
      if (context.engine === "postgres")
        for (const name of ["tableoid", "xmin", "cmin", "xmax", "cmax", "ctid"]) existing.add(name);
      const issues = references.filter((ref) => !existing.has(columnKey(ref.name)));
      return {
        table,
        checked: references.length,
        issues,
        columns,
        preview(current, issue, column) {
          assertCurrent(current);
          if (!issues.includes(issue) || !columns.includes(column))
            throw new Error("Choose a column from this inspection's table metadata.");
          const quote = context.engine === "mysql" ? "`" : '"';
          return {
            from: issue.from,
            to: issue.to,
            insert: quote + column.split(quote).join(quote + quote) + quote,
          };
        },
      };
    },
  };
}
