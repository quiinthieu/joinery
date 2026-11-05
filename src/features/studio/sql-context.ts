import type { EditorState } from "@codemirror/state";
import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import type { EngineKind, TableInfo, TableRef, TableSchema } from "../../ipc/generated";
import type { CompletionItem } from "../../data/completion-index";
import {
  identifierValue,
  quoteIdentifier,
  readIdentifier,
  type Identifier,
} from "./sql-identifiers";

export interface StatementSlice {
  sql: string;
  fromUtf16: number;
  toUtf16: number;
}

export function selectedStatementSlice(state: EditorState): StatementSlice {
  const selected = state.selection.main;
  if (!selected.empty) {
    return {
      sql: state.sliceDoc(selected.from, selected.to),
      fromUtf16: selected.from,
      toUtf16: selected.to,
    };
  }
  const tree = ensureSyntaxTree(state, state.doc.length, 75);
  if (!tree) throw new Error("Select the SQL to run while this large document finishes parsing.");
  let previous: { from: number; to: number } | null = null;
  for (let node = tree.topNode.firstChild; node; node = node.nextSibling) {
    if (node.name !== "Statement") continue;
    if (selected.head < node.to) {
      return {
        sql: state.sliceDoc(node.from, node.to),
        fromUtf16: node.from,
        toUtf16: node.to,
      };
    }
    previous = node;
  }
  return previous
    ? {
        sql: state.sliceDoc(previous.from, previous.to),
        fromUtf16: previous.from,
        toUtf16: previous.to,
      }
    : { sql: "", fromUtf16: 0, toUtf16: 0 };
}

export function selectedStatement(state: EditorState): string {
  return selectedStatementSlice(state).sql;
}

export function tableLookup(
  tables: TableInfo[],
  engine: EngineKind,
  database?: string,
  defaultSchema?: string,
  searchPath?: string[],
  unqualifiedTables?: TableRef[],
  caseSensitiveTableNames?: boolean,
) {
  // MySQL uses database.table, never database.schema.table.
  if (engine === "mysql") database = undefined;
  const unqualified =
    unqualifiedTables &&
    new Set(unqualifiedTables.map((table) => JSON.stringify([table.schema, table.name])));
  const exact = new Map<string, TableRef[]>(),
    folded = new Map<string, TableRef[]>();
  const add = (parts: string[], table: TableRef) => {
    for (const [map, key] of [
      [exact, JSON.stringify(parts)],
      [folded, JSON.stringify(parts.map((p) => p.toLowerCase()))],
    ] as const) {
      const bucket = map.get(key);
      if (bucket) bucket.push(table);
      else map.set(key, [table]);
    }
  };
  for (const { table } of tables) {
    add([table.name], table);
    add([table.schema, table.name], table);
    if (database) {
      add([database, table.schema, table.name], table);
      const normalizedDb = identifierValue({ value: database, quoted: false }, engine);
      if (normalizedDb !== database) add([normalizedDb, table.schema, table.name], table);
    }
  }
  return {
    engine,
    database,
    resolve(parts: Identifier[]) {
      const names = parts.map((p) => identifierValue(p, engine));
      let candidates = exact.get(JSON.stringify(names)) ?? [];
      if (engine === "mysql" && caseSensitiveTableNames === false)
        candidates = folded.get(JSON.stringify(names.map((p) => p.toLowerCase()))) ?? [];
      if (parts.length === 1 && unqualified) {
        candidates = candidates.filter((table) =>
          unqualified.has(JSON.stringify([table.schema, table.name])),
        );
        return candidates.length === 1 ? candidates[0] : undefined;
      }
      if (parts.length === 1 && searchPath) {
        for (const schema of searchPath) {
          const matches = candidates.filter((t) => t.schema === schema);
          if (matches.length === 1) return matches[0];
        }
        return undefined;
      }
      // lower_case_table_names varies by MySQL server/platform. Never collapse
      // case twins; prefer exact spelling, then a unique folded candidate.
      if (!candidates.length && engine === "mysql" && caseSensitiveTableNames === undefined)
        candidates = folded.get(JSON.stringify(names.map((p) => p.toLowerCase()))) ?? [];
      if (parts.length === 1 && candidates.length > 1) {
        const schema = defaultSchema ?? (engine === "postgres" ? "public" : undefined);
        if (schema) {
          const preferred = candidates.filter((t) => t.schema === schema);
          if (preferred.length === 1) return preferred[0];
        }
      }
      return candidates.length === 1 ? candidates[0] : undefined;
    },
  };
}
export type TableLookup = ReturnType<typeof tableLookup>;

export function catalogCompletions(tables: TableInfo[], lookup: TableLookup): CompletionItem[] {
  const database = lookup.database;
  const schemas = new Set<string>();
  const shortNames = new Map<string, TableRef | undefined>();
  const items: CompletionItem[] = [];
  const add = (path: string[], type: string, detail: string) =>
    items.push({ label: path.join("."), path, type, detail });
  for (const { table } of tables) {
    schemas.add(table.schema);
    if (!shortNames.has(table.name))
      shortNames.set(table.name, lookup.resolve([{ value: table.name, quoted: true }]));
    const short = shortNames.get(table.name);
    if (short?.schema === table.schema && short.name === table.name)
      add([table.name], "type", table.schema);
    add([table.schema, table.name], "type", "table");
    if (database) add([database, table.schema, table.name], "type", "table");
  }
  for (const schema of schemas) {
    add([schema], "namespace", "schema");
    if (database) add([database, schema], "namespace", "schema");
  }
  if (database) add([database], "namespace", "database");
  return items;
}

type Node = ReturnType<typeof syntaxTree>["topNode"];
export interface SqlToken {
  kind: string;
  from: number;
  to: number;
  text: string;
  children?: SqlToken[];
}
type Token = SqlToken;
const isName = (token?: Token) =>
  token?.kind === "Identifier" || token?.kind === "QuotedIdentifier";
const word = (token?: Token) => (token?.kind === "Keyword" ? token.text.toUpperCase() : "");
const isComment = (token: Token) => token.kind.endsWith("Comment");

const NON_ALIAS_KEYWORDS = new Set([
  "ALL",
  "AND",
  "ANTI",
  "ANY",
  "APPLY",
  "AS",
  "ASC",
  "ASOF",
  "AT",
  "BEFORE",
  "BETWEEN",
  "BY",
  "CASE",
  "CAST",
  "CHANGES",
  "CONNECT",
  "CREATE",
  "CROSS",
  "CURRENT",
  "DEFAULT",
  "DELETE",
  "DESC",
  "DISTINCT",
  "DROP",
  "ELSE",
  "END",
  "EXCEPT",
  "EXISTS",
  "FALSE",
  "FETCH",
  "FOR",
  "FORCE",
  "FROM",
  "FULL",
  "GROUP",
  "HAVING",
  "IGNORE",
  "ILIKE",
  "IN",
  "INDEX",
  "INNER",
  "INSERT",
  "INTERSECT",
  "INTO",
  "IS",
  "JOIN",
  "LATERAL",
  "LEFT",
  "LIKE",
  "LIMIT",
  "LOCK",
  "MATCH_RECOGNIZE",
  "MINUS",
  "NATURAL",
  "NOT",
  "NULL",
  "OFFSET",
  "ON",
  "ONLY",
  "OR",
  "ORDER",
  "OUTER",
  "OVER",
  "PARTITION",
  "PIVOT",
  "QUALIFY",
  "RETURNING",
  "RIGHT",
  "SAMPLE",
  "SEED",
  "SELECT",
  "SEMI",
  "SET",
  "SOME",
  "START",
  "STRAIGHT_JOIN",
  "TABLESAMPLE",
  "THEN",
  "TO",
  "TRUE",
  "UNION",
  "UNIQUE",
  "UNPIVOT",
  "UPDATE",
  "USE",
  "USING",
  "VALUES",
  "WHEN",
  "WHERE",
  "WINDOW",
  "WITH",
]);

function isRelationOrAliasToken(token?: Token): boolean {
  if (isName(token)) return true;
  if (!token) return false;
  if (token.kind !== "Keyword" && token.kind !== "Builtin" && token.kind !== "Type") {
    return false;
  }
  return (
    /^[A-Za-z_][A-Za-z0-9_$]*$/.test(token.text) &&
    !NON_ALIAS_KEYWORDS.has(token.text.toUpperCase())
  );
}

function tokens(state: EditorState, node: Node): Token[] | null {
  let budget = 4096;
  function read(parent: Node, depth: number): Token[] {
    if (depth > 64) throw new Error("scope depth");
    const output: Token[] = [];
    function append(child: Node) {
      if (--budget < 0) throw new Error("scope size");
      if (child.name === "CompositeIdentifier") {
        for (let part = child.firstChild; part; part = part.nextSibling) append(part);
      } else if (child.to > child.from) {
        const token: Token = {
          kind: child.name,
          from: child.from,
          to: child.to,
          text: state.sliceDoc(child.from, child.to),
        };
        if (child.name === "Parens")
          token.children = read(child, depth + 1).filter((t) => t.kind !== "(" && t.kind !== ")");
        const previous = output.at(-1);
        // CodeMirror's SQL lexer recognizes ASCII identifiers only. Recover
        // Unicode identifier fragments from error leaves, never from strings
        // or comments, preserving its lexical boundaries and UTF-16 offsets.
        const unicode =
          token.kind === "⚠" && /^[\p{L}\p{M}_][\p{L}\p{N}\p{M}_$]*$/u.test(token.text);
        if (unicode) token.kind = "Identifier";
        if (
          previous &&
          previous.to === token.from &&
          ((previous.kind === "Identifier" && token.kind === "Identifier") ||
            (previous.kind === "Keyword" && unicode) ||
            (previous.kind === "Identifier" &&
              /[^\x00-\x7f]/.test(previous.text) &&
              token.kind === "Keyword"))
        ) {
          previous.kind = "Identifier";
          previous.to = token.to;
          previous.text += token.text;
          return;
        }
        // The SQL lexer emits adjacent quoted tokens for doubled delimiters.
        if (
          token.kind === "QuotedIdentifier" &&
          previous?.kind === token.kind &&
          previous.to === token.from &&
          previous.text[0] === token.text[0]
        ) {
          previous.to = token.to;
          previous.text += token.text;
        } else output.push(token);
      }
    }
    for (let child = parent.firstChild; child; child = child.nextSibling) append(child);
    return output;
  }
  if (node.to - node.from > 65536) return null;
  try {
    return read(node, 0);
  } catch {
    return null;
  }
}

function statementAt(state: EditorState, position: number, complete: boolean): Node | null {
  const tree = ensureSyntaxTree(state, complete ? state.doc.length : position, 10);
  if (!tree) return null;
  let node: Node | null = tree.resolveInner(position, -1);
  while (node && node.name !== "Statement") node = node.parent;
  if (node) return node;
  const previous = tree.topNode.childBefore(position);
  if (
    previous?.name === "Statement" &&
    !state.sliceDoc(previous.from, previous.to).endsWith(";") &&
    !state.sliceDoc(previous.to, position).trim()
  )
    return previous;
  return null;
}

export function statementTokensAt(
  state: EditorState,
  position: number,
): { from: number; to: number; tokens: SqlToken[] } | null {
  const stmt = statementAt(state, position, true);
  if (!stmt) return null;
  const parsed = tokens(state, stmt);
  if (!parsed) return null;
  return { from: stmt.from, to: stmt.to, tokens: parsed };
}

export function allStatementTokenBlocks(
  state: EditorState,
): Array<{ from: number; to: number; tokens: SqlToken[] }> {
  const tree = ensureSyntaxTree(state, state.doc.length, 75);
  if (!tree) return [];
  const blocks: Array<{ from: number; to: number; tokens: SqlToken[] }> = [];
  for (let node = tree.topNode.firstChild; node; node = node.nextSibling) {
    if (node.name !== "Statement") continue;
    const parsed = tokens(state, node);
    if (parsed && parsed.length > 0) {
      blocks.push({ from: node.from, to: node.to, tokens: parsed });
    }
  }
  return blocks;
}

export interface CompletionTarget {
  from: number;
  to: number;
  parts: Identifier[];
}
export function completionTarget(state: EditorState, position: number): CompletionTarget | null {
  const tree = ensureSyntaxTree(state, position, 10);
  if (!tree) return null;
  const at = tree.resolveInner(position, -1);
  if (/String|Comment/.test(at.name)) return null;
  const statement = statementAt(state, position, false);
  if (!statement) return { from: position, to: position, parts: [{ value: "", quoted: false }] };
  const list = tokens(state, statement);
  if (!list) return null;
  const flatten = (list: Token[]): Token[] =>
    list.flatMap((t) => (t.children ? flatten(t.children) : [t]));
  const leaves = flatten(list);
  let i = leaves.findIndex((t) => t.from < position && t.to >= position);
  if (i < 0) return { from: position, to: position, parts: [{ value: "", quoted: false }] };
  const current = leaves[i];
  if (isComment(current) || current.kind.includes("String")) return null;
  const isWordToken = (t?: Token) =>
    isName(t) ||
    ((t?.kind === "Keyword" || t?.kind === "Type" || t?.kind === "Builtin") &&
      /^[A-Za-z_][A-Za-z0-9_$]*$/.test(t.text));
  if (!isWordToken(current) && current.kind !== ".") return null;
  const parts: Identifier[] =
    current.kind === "."
      ? [{ value: "", quoted: false }]
      : [readIdentifier(state.sliceDoc(current.from, position), position === current.to)];
  let from = current.from;
  if (current.kind !== ".") i--;
  while (
    i >= 1 &&
    leaves[i].kind === "." &&
    isRelationOrAliasToken(leaves[i - 1]) &&
    parts.length < 4
  ) {
    parts.unshift(readIdentifier(leaves[i - 1].text));
    from = leaves[i - 1].from;
    i -= 2;
  }
  return { from, to: current.kind === "." ? position : current.to, parts };
}

interface Qualifier {
  path: string[];
  sql: string[];
}
export interface TableReference {
  table: TableRef;
  qualifiers: Qualifier[];
}
export interface SqlScope {
  references: TableReference[];
  names: string[];
  allowBare: boolean;
}
const emptyScope = (): SqlScope => ({ references: [], names: [], allowBare: false });

function withNames(list: Token[], engine: EngineKind) {
  const names: string[] = [];
  if (word(list[0]) !== "WITH") return names;
  let i = word(list[1]) === "RECURSIVE" ? 2 : 1;
  while (isName(list[i])) {
    names.push(identifierValue(readIdentifier(list[i++].text), engine));
    if (list[i]?.children) i++; // optional output column list
    if (word(list[i++]) !== "AS") break;
    while (["NOT", "MATERIALIZED"].includes(word(list[i]))) i++;
    if (!list[i++]?.children || list[i]?.text !== ",") break;
    i++;
  }
  return names;
}

function queryBlockAt(
  state: EditorState,
  lookup: TableLookup,
  position: number,
): { block: Token[]; shadows: Set<string> } | null {
  const statement = statementAt(state, position, true);
  if (!statement) return null;
  const parsed = tokens(state, statement);
  if (!parsed) return null;
  const shadows = new Set<string>();
  const clean = (list: Token[]) => list.filter((t) => !isComment(t));
  let block = clean(parsed);
  // Descend through expression parentheses too, but only replace the query
  // scope at a subquery. Never inherit correlated outer table bindings.
  function descend(list: Token[]) {
    withNames(clean(list), lookup.engine).forEach((name) => shadows.add(name));
    for (const token of list) {
      if (
        !token.children ||
        position <= token.from ||
        position > token.to ||
        (position === token.to && token.text.endsWith(")"))
      )
        continue;
      const child = clean(token.children);
      if (["SELECT", "WITH", "UPDATE", "DELETE", "INSERT"].includes(word(child[0]))) block = child;
      descend(child);
    }
  }
  descend(block);
  // These constructs require projection/escape semantics beyond direct base
  // bindings. Decline them instead of attaching physical columns to new names.
  if (
    (block.some((t) => word(t) === "INSERT") && block.some((t) => word(t) === "SELECT")) ||
    block.some(
      (t, i) =>
        t.text.toUpperCase() === "U" &&
        block[i + 1]?.text === "&" &&
        block[i + 2]?.kind === "QuotedIdentifier",
    )
  )
    return null;
  // Set-operation arms have separate bindings.
  let start = 0,
    end = block.length;
  for (let i = 0; i < block.length; i++) {
    if (!["UNION", "INTERSECT", "EXCEPT"].includes(word(block[i]))) continue;
    if (position >= block[i].to)
      start = i + (word(block[i + 1]) === "ALL" || word(block[i + 1]) === "DISTINCT" ? 2 : 1);
    else {
      end = i;
      break;
    }
  }
  block = block.slice(start, end);
  if (
    start > 0 &&
    block.some((t) => ["ORDER", "LIMIT", "OFFSET"].includes(word(t)) && position >= t.from)
  )
    return null;
  if (
    !["SELECT", "WITH", "UPDATE", "DELETE", "INSERT", "EXPLAIN"].includes(word(block[0])) ||
    block.some((t) => t.kind === "⚠")
  )
    return null;
  return { block, shadows };
}

interface ParsedBinding {
  table?: TableRef;
  qualifier?: Qualifier;
  alias?: string;
  aliasSql?: string;
  name: string;
  isJoin: boolean;
}

function parseBlockBindings(
  block: Token[],
  shadows: Set<string>,
  lookup: TableLookup,
  maxPos = Number.POSITIVE_INFINITY,
  caretPos?: number,
): ParsedBinding[] {
  const bindings: ParsedBinding[] = [];
  let inFrom = false;
  for (let i = 0; i < block.length; i++) {
    if (block[i].to > maxPos) break;
    const keyword = word(block[i]);
    if (
      [
        "WHERE",
        "GROUP",
        "ORDER",
        "HAVING",
        "LIMIT",
        "OFFSET",
        "SET",
        "VALUES",
        "RETURNING",
        "QUALIFY",
        "WINDOW",
        "FOR",
      ].includes(keyword)
    )
      inFrom = false;
    const relation =
      ["FROM", "JOIN", "UPDATE", "INTO"].includes(keyword) ||
      (keyword === "USING" && word(block[0]) === "DELETE" && !block[i + 1]?.children) ||
      (inFrom && block[i].text === ",");
    if (!relation) continue;
    inFrom = keyword !== "INTO";
    let next = i + 1;
    while (
      block[next] &&
      block[next].to <= maxPos &&
      ["ONLY", "LATERAL"].includes(word(block[next]))
    )
      next++;
    const nameFrom = block[next]?.from ?? block[i].to;
    const parts: Identifier[] = [];
    if (block[next] && block[next].to <= maxPos && isRelationOrAliasToken(block[next])) {
      parts.push(readIdentifier(block[next++].text));
      while (
        block[next]?.text === "." &&
        block[next + 1] &&
        block[next + 1].to <= maxPos &&
        isRelationOrAliasToken(block[next + 1]) &&
        parts.length < 3
      ) {
        parts.push(readIdentifier(block[next + 1].text));
        next += 2;
      }
    }
    const incompleteName = block[next] && block[next].to <= maxPos && block[next]?.text === ".";
    const nameTo = incompleteName ? block[next].to : (block[next - 1]?.to ?? nameFrom);
    let derived = false;
    if (block[next] && block[next].to <= maxPos && block[next]?.children) {
      derived = keyword !== "INTO";
      next++;
    }
    if (!parts.length && !derived) continue;
    if (block[next] && block[next].to <= maxPos && word(block[next]) === "AS") next++;
    const alias =
      block[next] && block[next].to <= maxPos && isRelationOrAliasToken(block[next])
        ? block[next++]
        : undefined;
    if (alias && block[next] && block[next].to <= maxPos && block[next]?.children) {
      derived = true;
      next++;
    }
    const name = alias
      ? identifierValue(readIdentifier(alias.text), lookup.engine)
      : identifierValue(parts.at(-1) ?? { value: "", quoted: false }, lookup.engine);
    const table =
      !derived &&
      !incompleteName &&
      !(parts.length === 1 && shadows.has(identifierValue(parts[0], lookup.engine)))
        ? lookup.resolve(parts)
        : undefined;
    const activelyTypingTarget =
      caretPos !== undefined &&
      !derived &&
      !alias &&
      (!table || keyword === "JOIN") &&
      caretPos >= nameFrom &&
      caretPos <= nameTo;
    if (!activelyTypingTarget) {
      bindings.push({
        table,
        name,
        alias: alias ? name : undefined,
        aliasSql: alias?.text,
        qualifier: alias ? { path: [name], sql: [alias.text] } : undefined,
        isJoin: keyword === "JOIN",
      });
    }
    i = next - 1;
  }
  return bindings;
}

export function referencedTables(
  state: EditorState,
  lookup: TableLookup,
  position = state.selection.main.head,
): SqlScope {
  const resolved = queryBlockAt(state, lookup, position);
  if (!resolved) return emptyScope();
  const { block, shadows } = resolved;
  const bindings = parseBlockBindings(block, shadows, lookup, Number.POSITIVE_INFINITY, position);
  const names = [...shadows, ...bindings.map((b) => b.name)];
  const counts = new Map<string, number>();
  bindings.forEach((b) => counts.set(b.name, (counts.get(b.name) ?? 0) + 1));
  const references = new Map<string, TableReference>();
  for (const binding of bindings) {
    if (!binding.table || (binding.qualifier && counts.get(binding.name) !== 1)) continue;
    const key = JSON.stringify(binding.table);
    if (references.size >= 8 && !references.has(key)) continue;
    const entry = references.get(key) ?? { table: binding.table, qualifiers: [] };
    const paths = [
      ...(counts.get(binding.name) === 1 ? [[binding.table.name]] : []),
      [binding.table.schema, binding.table.name],
      ...(lookup.database ? [[lookup.database, binding.table.schema, binding.table.name]] : []),
    ];
    entry.qualifiers.push(
      ...(binding.qualifier
        ? [binding.qualifier]
        : paths.map((path) => ({ path, sql: path.map((p) => quoteIdentifier(p, lookup.engine)) }))),
    );
    references.set(key, entry);
  }
  return {
    references: [...references.values()],
    names,
    allowBare:
      bindings.length > 0 &&
      bindings.length <= 8 &&
      bindings.every((b) => b.table && counts.get(b.name) === 1),
  };
}

export function columnCompletions(
  scope: SqlScope,
  schemas: (TableSchema | null)[],
  engine: EngineKind,
): CompletionItem[] {
  const items: CompletionItem[] = [];
  const counts = new Map<string, number>();
  schemas.forEach((schema, i) =>
    schema?.columns.forEach((c) =>
      counts.set(
        c.name,
        (counts.get(c.name) ?? 0) +
          scope.references[i].qualifiers.filter((q) => q.path.length === 1).length,
      ),
    ),
  );
  const quoteCache = new Map<string, string>();
  const quote = (name: string) => {
    if (!quoteCache.has(name)) quoteCache.set(name, quoteIdentifier(name, engine));
    return quoteCache.get(name)!;
  };
  schemas.forEach((schema, i) =>
    schema?.columns.forEach((column) => {
      const qualifiers = [...scope.references[i].qualifiers];
      if (scope.allowBare && schemas.every(Boolean) && counts.get(column.name) === 1)
        qualifiers.push({ path: [], sql: [] });
      for (const qualifier of qualifiers) {
        const label = [...qualifier.sql, quote(column.name)].join(".");
        items.push({
          label,
          apply: label,
          path: [...qualifier.path, column.name],
          type: "property",
          detail: `${schema.table.name} · ${column.databaseType}`,
        });
      }
    }),
  );
  return items;
}

const tableKey = (table: TableRef) => JSON.stringify([table.schema, table.name]);
const sameTable = (a?: TableRef, b?: TableRef) =>
  Boolean(a && b && a.schema === b.schema && a.name === b.name);

function partMatchesPrefix(candidate: string, part: Identifier): boolean {
  return part.quoted
    ? candidate.startsWith(part.value)
    : candidate.toLowerCase().startsWith(part.value.toLowerCase());
}

function partMatchesExact(candidate: string, part: Identifier, engine: EngineKind): boolean {
  return engine === "mysql" && !part.quoted
    ? candidate.toLowerCase() === part.value.toLowerCase()
    : candidate === identifierValue(part, engine);
}

function matchesPartnerPrefix(
  partner: TableRef,
  parts: Identifier[],
  lookup: TableLookup,
): boolean {
  const { engine, database } = lookup;
  if (parts.length === 1) {
    return (
      partMatchesPrefix(partner.name, parts[0]) ||
      partMatchesPrefix(partner.schema, parts[0]) ||
      Boolean(database && partMatchesPrefix(database, parts[0]))
    );
  }
  if (parts.length === 2) {
    return (
      (partMatchesExact(partner.schema, parts[0], engine) &&
        partMatchesPrefix(partner.name, parts[1])) ||
      Boolean(
        database &&
        partMatchesExact(database, parts[0], engine) &&
        partMatchesPrefix(partner.schema, parts[1]),
      )
    );
  }
  if (parts.length === 3 && database) {
    return (
      partMatchesExact(database, parts[0], engine) &&
      partMatchesExact(partner.schema, parts[1], engine) &&
      partMatchesPrefix(partner.name, parts[2])
    );
  }
  return false;
}

function aliasStem(tableName: string): string {
  const words = tableName
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean);
  const initials = words
    .map((w) => w[0])
    .join("")
    .toLowerCase();
  if (/^[a-z][a-z0-9_]*$/.test(initials)) return initials;
  const ascii = tableName.toLowerCase().replace(/[^a-z0-9_]/g, "");
  if (/^[a-z][a-z0-9_]*$/.test(ascii)) return ascii.slice(0, 3);
  return "t";
}

function pickPartnerAlias(tableName: string, usedLower: Set<string>, engine: EngineKind): string {
  const stem = aliasStem(tableName);
  const valid = (candidate: string) =>
    !usedLower.has(candidate.toLowerCase()) && quoteIdentifier(candidate, engine) === candidate;
  if (valid(stem)) return stem;
  for (let n = 2; n < 100; n++) {
    const candidate = `${stem}${n}`;
    if (valid(candidate)) return candidate;
  }
  return quoteIdentifier(`${stem}_join`, engine);
}

function qualifierForBinding(
  binding: ParsedBinding,
  allBindings: ParsedBinding[],
  shadows: Set<string>,
  lookup: TableLookup,
): { value: string; sql: string; schemaQualified: boolean } | null {
  if (!binding.table) return null;
  const { engine } = lookup;
  const fold = (s: string) => (engine === "mysql" ? s.toLowerCase() : s);
  const sameNameCount = allBindings.filter((b) => fold(b.name) === fold(binding.name)).length;
  if (binding.alias && binding.aliasSql) {
    if (sameNameCount !== 1) return null;
    return { value: binding.alias, sql: binding.aliasSql, schemaQualified: false };
  }
  const shortNorm = identifierValue({ value: binding.table.name, quoted: true }, engine);
  const shortUnique =
    sameNameCount === 1 &&
    !shadows.has(shortNorm) &&
    sameTable(lookup.resolve([{ value: binding.table.name, quoted: true }]), binding.table);
  if (shortUnique) {
    return {
      value: binding.table.name,
      sql: quoteIdentifier(binding.table.name, engine),
      schemaQualified: false,
    };
  }
  const sameTableCount = allBindings.filter(
    (b) => !b.alias && b.table && sameTable(b.table, binding.table),
  ).length;
  if (sameTableCount !== 1) return null;
  return {
    value: binding.table.name,
    sql: `${quoteIdentifier(binding.table.schema, engine)}.${quoteIdentifier(binding.table.name, engine)}`,
    schemaQualified: true,
  };
}

export function joinCompletions(
  state: EditorState,
  position: number,
  target: CompletionTarget,
  scope: SqlScope,
  schemas: (TableSchema | null)[],
  tables: TableInfo[],
  lookup: TableLookup,
): CompletionItem[] {
  const resolved = queryBlockAt(state, lookup, position);
  if (!resolved) return [];
  const { block, shadows } = resolved;
  const { engine, database } = lookup;

  let anchorIdx = -1;
  for (let i = 0; i < block.length; i++) {
    if (block[i].to <= target.from) anchorIdx = i;
    else break;
  }
  if (anchorIdx < 0) return [];
  const anchorWord = word(block[anchorIdx]);
  const nextAfterAnchor = block[anchorIdx + 1];

  const isJoinTarget =
    anchorWord === "JOIN" && (!nextAfterAnchor || nextAfterAnchor.from >= target.from);
  const isOnParensStart =
    anchorWord === "ON" &&
    nextAfterAnchor?.kind === "Parens" &&
    position > nextAfterAnchor.from &&
    position <= nextAfterAnchor.to &&
    (!nextAfterAnchor.children?.[0] || nextAfterAnchor.children[0].from >= target.from);
  const isOnPredicate =
    (anchorWord === "ON" && (!nextAfterAnchor || nextAfterAnchor.from >= target.from)) ||
    isOnParensStart;

  if (!isJoinTarget && !isOnPredicate) return [];

  if (isJoinTarget) {
    let prefixIdx = anchorIdx - 1;
    while (
      prefixIdx >= 0 &&
      ["INNER", "LEFT", "RIGHT", "FULL", "OUTER"].includes(word(block[prefixIdx]))
    ) {
      prefixIdx--;
    }
    if (prefixIdx >= 0 && ["CROSS", "NATURAL"].includes(word(block[prefixIdx]))) return [];
  }

  const schemaByTable = new Map<string, TableSchema>();
  scope.references.forEach((ref, idx) => {
    const schema = schemas[idx];
    if (schema) schemaByTable.set(tableKey(ref.table), schema);
  });
  if (!schemaByTable.size) return [];

  const visibleTables = new Set(tables.map((t) => tableKey(t.table)));

  if (isJoinTarget) {
    const priorBindings = parseBlockBindings(block, shadows, lookup, block[anchorIdx].from);
    if (!priorBindings.length) return [];
    const anyAliased = priorBindings.some((b) => Boolean(b.alias));
    const usedLower = new Set(
      [...shadows, ...priorBindings.map((b) => b.name)].map((n) => n.toLowerCase()),
    );
    const boundTableKeys = new Set(
      priorBindings.flatMap((b) => (b.table ? [tableKey(b.table)] : [])),
    );
    const freshItems: CompletionItem[] = [];
    const rejoinItems: CompletionItem[] = [];
    const seenApply = new Set<string>();

    const emitJoin = (
      bound: ParsedBinding,
      partner: TableRef,
      boundCols: string[],
      partnerCols: string[],
      constraintName: string,
    ) => {
      if (!boundCols.length || boundCols.length !== partnerCols.length) return;
      if (!visibleTables.has(tableKey(partner))) return;
      if (!matchesPartnerPrefix(partner, target.parts, lookup)) return;

      const collision = usedLower.has(partner.name.toLowerCase());
      const partnerAlias =
        anyAliased || collision ? pickPartnerAlias(partner.name, usedLower, engine) : undefined;
      const simulatedPartner: ParsedBinding = {
        table: partner,
        alias: partnerAlias,
        aliasSql: partnerAlias,
        name: partnerAlias ?? partner.name,
        isJoin: true,
      };
      const boundQual = qualifierForBinding(
        bound,
        [...priorBindings, simulatedPartner],
        shadows,
        lookup,
      );
      if (!boundQual) return;

      const partnerShortNorm = identifierValue({ value: partner.name, quoted: true }, engine);
      const shortResolved = !shadows.has(partnerShortNorm)
        ? lookup.resolve([{ value: partner.name, quoted: true }])
        : undefined;
      const partnerTableSql =
        target.parts.length === 3 && database
          ? `${quoteIdentifier(database, engine)}.${quoteIdentifier(partner.schema, engine)}.${quoteIdentifier(partner.name, engine)}`
          : target.parts.length === 1 && sameTable(shortResolved, partner)
            ? quoteIdentifier(partner.name, engine)
            : `${quoteIdentifier(partner.schema, engine)}.${quoteIdentifier(partner.name, engine)}`;
      const partnerQualSql = partnerAlias ?? partnerTableSql;
      const predicate = boundCols
        .map(
          (col, k) =>
            `${boundQual.sql}.${quoteIdentifier(col, engine)} = ${partnerQualSql}.${quoteIdentifier(partnerCols[k], engine)}`,
        )
        .join(" AND ");
      const apply = partnerAlias
        ? `${partnerTableSql} ${partnerAlias} ON ${predicate}`
        : `${partnerTableSql} ON ${predicate}`;
      if (seenApply.has(apply)) return;
      seenApply.add(apply);
      const item: CompletionItem = {
        label: apply,
        apply,
        type: "interface",
        detail: `${constraintName} · FK join`,
      };
      if (boundTableKeys.has(tableKey(partner))) rejoinItems.push(item);
      else freshItems.push(item);
    };

    for (let idx = priorBindings.length - 1; idx >= 0; idx--) {
      const bound = priorBindings[idx];
      if (!bound.table) continue;
      const boundSchema = schemaByTable.get(tableKey(bound.table));
      if (!boundSchema) continue;
      for (const fk of boundSchema.foreignKeys ?? []) {
        emitJoin(bound, fk.target, fk.columns, fk.targetColumns, fk.name);
      }
      for (const fk of boundSchema.inboundForeignKeys ?? []) {
        emitJoin(bound, fk.source, fk.targetColumns, fk.columns, fk.name);
      }
    }
    return [...freshItems, ...rejoinItems];
  }

  // isOnPredicate: JOIN <partner> [AS] [alias] ON <target>
  const bindingsBeforeOn = parseBlockBindings(block, shadows, lookup, block[anchorIdx].from);
  if (bindingsBeforeOn.length < 2) return [];
  const joined = bindingsBeforeOn.at(-1)!;
  if (!joined.isJoin || !joined.table) return [];
  const joinedQual = qualifierForBinding(joined, bindingsBeforeOn, shadows, lookup);
  if (!joinedQual) return [];
  const joinedSchema = schemaByTable.get(tableKey(joined.table));
  const priorBindings = bindingsBeforeOn.slice(0, -1);
  const items: CompletionItem[] = [];
  const seenApply = new Set<string>();

  const matchDirection = (
    qual: { value: string; sql: string; schemaQualified: boolean },
    table: TableRef,
    cols: string[],
  ): boolean => {
    if (target.parts.length === 1) {
      const p0 = target.parts[0];
      return (
        p0.value === "" ||
        partMatchesPrefix(qual.value, p0) ||
        (qual.schemaQualified && partMatchesPrefix(table.schema, p0)) ||
        cols.some((c) => partMatchesPrefix(c, p0))
      );
    }
    if (target.parts.length === 2) {
      const [p0, p1] = target.parts;
      if (qual.schemaQualified) {
        return partMatchesExact(table.schema, p0, engine) && partMatchesPrefix(table.name, p1);
      }
      return partMatchesExact(qual.value, p0, engine) && cols.some((c) => partMatchesPrefix(c, p1));
    }
    if (target.parts.length === 3 && qual.schemaQualified) {
      const [p0, p1, p2] = target.parts;
      return (
        partMatchesExact(table.schema, p0, engine) &&
        partMatchesExact(table.name, p1, engine) &&
        cols.some((c) => partMatchesPrefix(c, p2))
      );
    }
    return false;
  };

  for (let idx = priorBindings.length - 1; idx >= 0; idx--) {
    const bound = priorBindings[idx];
    if (!bound.table) continue;
    const boundQual = qualifierForBinding(bound, bindingsBeforeOn, shadows, lookup);
    if (!boundQual) continue;
    const boundSchema = schemaByTable.get(tableKey(bound.table));

    const pairs: { boundCols: string[]; joinedCols: string[]; name: string }[] = [];
    for (const fk of boundSchema?.foreignKeys ?? []) {
      if (sameTable(fk.target, joined.table))
        pairs.push({ boundCols: fk.columns, joinedCols: fk.targetColumns, name: fk.name });
    }
    for (const fk of boundSchema?.inboundForeignKeys ?? []) {
      if (sameTable(fk.source, joined.table))
        pairs.push({ boundCols: fk.targetColumns, joinedCols: fk.columns, name: fk.name });
    }
    for (const fk of joinedSchema?.inboundForeignKeys ?? []) {
      if (sameTable(fk.source, bound.table))
        pairs.push({ boundCols: fk.columns, joinedCols: fk.targetColumns, name: fk.name });
    }
    for (const fk of joinedSchema?.foreignKeys ?? []) {
      if (sameTable(fk.target, bound.table))
        pairs.push({ boundCols: fk.targetColumns, joinedCols: fk.columns, name: fk.name });
    }

    for (const pair of pairs) {
      if (!pair.boundCols.length || pair.boundCols.length !== pair.joinedCols.length) continue;
      const matchesBoundFirst = matchDirection(boundQual, bound.table, pair.boundCols);
      const matchesJoinedFirst = matchDirection(joinedQual, joined.table, pair.joinedCols);
      if (!matchesBoundFirst && !matchesJoinedFirst) continue;
      const joinedFirst = !matchesBoundFirst && matchesJoinedFirst;
      const [leftQual, leftCols, rightQual, rightCols] = joinedFirst
        ? [joinedQual.sql, pair.joinedCols, boundQual.sql, pair.boundCols]
        : [boundQual.sql, pair.boundCols, joinedQual.sql, pair.joinedCols];
      const apply = leftCols
        .map(
          (col, k) =>
            `${leftQual}.${quoteIdentifier(col, engine)} = ${rightQual}.${quoteIdentifier(rightCols[k], engine)}`,
        )
        .join(" AND ");
      if (seenApply.has(apply)) continue;
      seenApply.add(apply);
      items.push({
        label: apply,
        apply,
        type: "interface",
        detail: `${pair.name} · FK join`,
      });
    }
  }
  return items;
}
