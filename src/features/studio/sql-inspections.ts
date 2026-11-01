import type { EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import type { TableRef, TableSchema } from "../../ipc/generated";
import { statementTokensAt, tableLookup, type SqlToken } from "./sql-context";
import { identifierValue, readIdentifier } from "./sql-identifiers";
import type { SqlSymbolContext } from "./sql-navigation";

export interface MissingSqlColumn {
  name: string;
  qualifier: string;
  table: TableRef;
  from: number;
  to: number;
  line: number;
}

export interface SqlColumnInspection {
  tables: TableRef[];
  checked: number;
  issues: MissingSqlColumn[];
  columnsFor(issue: MissingSqlColumn): readonly string[];
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

export const columnInspectionLimits = {
  relations: 32,
  columnsPerTable: 2048,
  columns: 8192,
  bytes: 4 * 1024 * 1024,
};
const tableKey = (table: TableRef) => JSON.stringify([table.schema, table.name]);

/** Flat direct-table SELECTs, with ordered JOIN operands and no query nesting. */
export function prepareColumnInspection(
  state: EditorState,
  pos: number,
  context: SqlSymbolContext,
) {
  const unsupported = () =>
    new Error(
      "Column inspection supports a direct-table SELECT with bounded JOINs, without subqueries.",
    );
  if (context.engine !== "postgres" && context.engine !== "mysql") throw unsupported();
  const statement = statementTokensAt(state, pos);
  if (!statement) throw new Error("Statement is incomplete or exceeds the inspection limit.");
  const clean = (tokens: readonly SqlToken[]) => tokens.filter((t) => !t.kind.endsWith("Comment"));
  const tokens = clean(statement.tokens);
  const nameToken = (token?: SqlToken) =>
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
  const lookup = tableLookup(
    [...context.tables],
    context.engine,
    context.database,
    undefined,
    undefined,
    context.unqualifiedTables,
    context.caseSensitiveTableNames,
  );
  const key = (raw: string) => {
    const value = identifierValue(readIdentifier(raw), context.engine);
    return context.engine === "mysql" && context.caseSensitiveTableNames === false
      ? value.toLowerCase()
      : value;
  };
  interface Binding {
    key: string;
    table: TableRef;
  }
  type Scope = Binding[];
  const bindings: Scope = [];
  const conditions: { tokens: SqlToken[]; scope: Scope }[] = [];
  const clauses = new Set(["WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "FOR", "FETCH"]);
  const joinModifiers = new Set(["INNER", "LEFT", "RIGHT", "FULL", "OUTER", "CROSS", "NATURAL"]);
  const joins = new Set([
    "",
    "INNER",
    "LEFT",
    "LEFT OUTER",
    "RIGHT",
    "RIGHT OUTER",
    "FULL",
    "FULL OUTER",
    "CROSS",
    "NATURAL",
    "NATURAL INNER",
    "NATURAL LEFT",
    "NATURAL LEFT OUTER",
    "NATURAL RIGHT",
    "NATURAL RIGHT OUTER",
    "NATURAL FULL",
    "NATURAL FULL OUTER",
  ]);
  const joinAt = (index: number) => {
    const modifiers: string[] = [];
    while (joinModifiers.has(word(tokens[index]))) modifiers.push(word(tokens[index++]));
    return word(tokens[index]) === "JOIN" ? { modifiers, next: index + 1 } : null;
  };
  let next = from + 1;
  function atom(): Binding {
    const relation: SqlToken[] = [];
    if (!nameToken(tokens[next])) throw unsupported();
    relation.push(tokens[next++]);
    while (tokens[next]?.text === "." && nameToken(tokens[next + 1])) {
      relation.push(tokens[next + 1]);
      next += 2;
    }
    if (relation.length > 2) throw unsupported();
    const explicitAlias = word(tokens[next]) === "AS";
    if (explicitAlias) next++;
    const alias = nameToken(tokens[next]) ? tokens[next++] : undefined;
    if (explicitAlias && !alias) throw unsupported();
    for (const token of [...relation, ...(alias ? [alias] : [])]) checkName(token);
    // Never substitute a guessed search path for observed visibility.
    if (relation.length === 1 && context.unqualifiedTables === undefined)
      throw new Error("Visible-table metadata is unavailable; no column claim was made.");
    const table = lookup.resolve(relation.map((token) => readIdentifier(token.text)));
    const info = table && context.tables.find((item) => tableKey(item.table) === tableKey(table));
    if (
      !table ||
      !info ||
      !["table", "partitionedTable", "r", "p", "BASE TABLE"].includes(info.kind)
    )
      throw new Error(
        "A unique direct base table could not be resolved; no column claim was made.",
      );
    const binding = { key: key((alias ?? relation.at(-1))!.text), table };
    if (
      bindings.length >= columnInspectionLimits.relations ||
      bindings.some((b) => b.key === binding.key)
    )
      throw unsupported();
    bindings.push(binding);
    return binding;
  }
  let operand: Scope = [atom()];
  const pending: { left: Scope; optional: boolean }[] = [];
  const finishJoins = () => {
    while (pending.length) {
      const join = pending.pop()!;
      if (!join.optional) throw unsupported();
      operand = [...join.left, ...operand];
    }
  };
  while (next < tokens.length && !clauses.has(word(tokens[next])) && tokens[next].text !== ";") {
    if (tokens[next].text === ",") {
      finishJoins();
      next++;
      operand = [atom()];
      continue;
    }
    const join = joinAt(next);
    if (join) {
      const { modifiers } = join;
      if (
        !joins.has(modifiers.join(" ")) ||
        (context.engine === "mysql" && modifiers.includes("FULL"))
      )
        throw unsupported();
      const natural = modifiers.includes("NATURAL"),
        cross = modifiers.includes("CROSS");
      pending.push({
        left: operand,
        optional:
          context.engine === "mysql" &&
          !modifiers.some((m) => ["LEFT", "RIGHT", "FULL"].includes(m)),
      });
      next = join.next;
      operand = [atom()];
      // CROSS/NATURAL joins close immediately. MySQL permits CROSS ... ON/USING.
      if (
        natural ||
        (cross && !(context.engine === "mysql" && ["ON", "USING"].includes(word(tokens[next]))))
      )
        operand = [...pending.pop()!.left, ...operand];
      continue;
    }
    const condition = word(tokens[next++]);
    if (condition !== "ON" && condition !== "USING") throw unsupported();
    const owner = pending.pop();
    if (!owner) throw unsupported();
    operand = [...owner.left, ...operand];
    if (condition === "USING") {
      const list = tokens[next++];
      const names = list?.children && clean(list.children);
      // The lexer classifies unqualified names such as id as dialect keywords.
      const usingName = (t: SqlToken) =>
        nameToken(t) ||
        (["Keyword", "Type", "Builtin"].includes(t.kind) &&
          /^[A-Za-z_][A-Za-z0-9_$]*$/.test(t.text) &&
          ![
            "SELECT",
            "FROM",
            "WITH",
            "WHERE",
            "JOIN",
            "ON",
            "USING",
            "AS",
            "INSERT",
            "UPDATE",
            "DELETE",
          ].includes(t.text.toUpperCase()));
      if (
        !list?.text.endsWith(")") ||
        !names?.length ||
        names.length % 2 === 0 ||
        names.some((t, i) => (i % 2 ? t.text !== "," : !usingName(t)))
      )
        throw unsupported();
      for (const token of names.filter((_, i) => i % 2 === 0)) checkName(token);
    } else {
      const start = next;
      while (
        next < tokens.length &&
        !clauses.has(word(tokens[next])) &&
        tokens[next].text !== ";" &&
        tokens[next].text !== "," &&
        !["ON", "USING"].includes(word(tokens[next])) &&
        !joinAt(next)
      )
        next++;
      if (next === start) throw unsupported();
      conditions.push({ tokens: tokens.slice(start, next), scope: operand });
    }
  }
  finishJoins();
  const tables = [
    ...new Map(bindings.map((binding) => [tableKey(binding.table), binding.table])).values(),
  ];
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
  function scan(raw: readonly SqlToken[], depth: number, scope: Scope) {
    if (depth > 16) throw unsupported();
    const list = clean(raw);
    for (let i = 0; i < list.length; i++) {
      if (++visited > 4096) throw unsupported();
      const token = list[i];
      if (forbidden.has(word(token))) throw unsupported();
      if (token.children) {
        if (!token.text.endsWith(")")) throw unsupported();
        scan(token.children, depth + 1, scope);
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
        const binding = scope.find((item) => item.key === key(token.text));
        if (!binding)
          throw new Error(
            "An unresolved qualifier prevents this inspection; no column claim was made.",
          );
        references.push({
          name: identifierValue(readIdentifier(column.text), context.engine),
          qualifier: token.text,
          table: binding.table,
          from: column.from,
          to: column.to,
          line: state.doc.lineAt(column.from).number,
        });
      }
      i += 2;
    }
  }
  scan(tokens.slice(1, from), 0, bindings);
  for (const condition of conditions) scan(condition.tokens, 0, condition.scope);
  scan(tokens.slice(next), 0, bindings);
  if (!references.length)
    throw new Error("No supported qualified columns to inspect in this SELECT.");
  const assertCurrent = (current: EditorState) => {
    if (!current.doc.eq(state.doc))
      throw new Error("SQL changed. Inspect the current statement again.");
  };
  return {
    tables,
    finish(current: EditorState, schemas: TableSchema[]): SqlColumnInspection {
      assertCurrent(current);
      const schemaMap = new Map(schemas.map((schema) => [tableKey(schema.table), schema]));
      const columnKey = (name: string) => (context.engine === "mysql" ? name.toLowerCase() : name);
      if (
        schemas.length !== tables.length ||
        schemaMap.size !== tables.length ||
        tables.some((table) => !schemaMap.has(tableKey(table))) ||
        schemas.some(
          (schema) =>
            !schema.columns.length ||
            schema.columns.length > columnInspectionLimits.columnsPerTable ||
            new Set(schema.columns.map((c) => columnKey(c.name))).size !== schema.columns.length,
        ) ||
        schemas.reduce((n, schema) => n + schema.columns.length, 0) >
          columnInspectionLimits.columns ||
        new TextEncoder().encode(JSON.stringify(schemas)).length > columnInspectionLimits.bytes
      )
        throw new Error("Complete table metadata is unavailable; no column claim was made.");
      const columns = new Map(
        schemas.map((schema) => [
          tableKey(schema.table),
          schema.columns.map((column) => column.name),
        ]),
      );
      const existing = new Map(
        [...columns].map(([table, names]) => [table, new Set(names.map(columnKey))]),
      );
      // PostgreSQL exposes these physical system columns outside attnum > 0.
      // They are valid references, but are not offered as catalog corrections.
      if (context.engine === "postgres")
        for (const names of existing.values())
          for (const name of ["tableoid", "xmin", "cmin", "xmax", "cmax", "ctid"]) names.add(name);
      const issues = references.filter(
        (ref) => !existing.get(tableKey(ref.table))!.has(columnKey(ref.name)),
      );
      const columnsFor = (issue: MissingSqlColumn) => {
        if (!issues.includes(issue))
          throw new Error("Choose a column from this inspection's table metadata.");
        return columns.get(tableKey(issue.table))!;
      };
      return {
        tables,
        checked: references.length,
        issues,
        columnsFor,
        preview(current, issue, column) {
          assertCurrent(current);
          if (!columnsFor(issue).includes(column))
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
