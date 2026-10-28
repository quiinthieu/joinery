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

export function selectedStatement(state: EditorState): string {
  const selected = state.selection.main;
  if (!selected.empty) return state.sliceDoc(selected.from, selected.to);
  const tree = ensureSyntaxTree(state, state.doc.length, 75);
  if (!tree) throw new Error("Select the SQL to run while this large document finishes parsing.");
  let previous: { from: number; to: number } | null = null;
  for (let node = tree.topNode.firstChild; node; node = node.nextSibling) {
    if (node.name !== "Statement") continue;
    if (selected.head < node.to) return state.sliceDoc(node.from, node.to);
    previous = node;
  }
  return previous ? state.sliceDoc(previous.from, previous.to) : "";
}

export function tableLookup(
  tables: TableInfo[],
  engine: EngineKind,
  database?: string,
  defaultSchema?: string,
) {
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
    if (database) add([database, table.schema, table.name], table);
  }
  return {
    engine,
    database,
    resolve(parts: Identifier[]) {
      const names = parts.map((p) => identifierValue(p, engine));
      let candidates = exact.get(JSON.stringify(names)) ?? [];
      // lower_case_table_names varies by MySQL server/platform. Never collapse
      // case twins; prefer exact spelling, then a unique folded candidate.
      if (!candidates.length && engine === "mysql")
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
type TableLookup = ReturnType<typeof tableLookup>;

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
interface Token {
  kind: string;
  from: number;
  to: number;
  text: string;
  children?: Token[];
}
const isName = (token?: Token) =>
  token?.kind === "Identifier" || token?.kind === "QuotedIdentifier";
const word = (token?: Token) => (token?.kind === "Keyword" ? token.text.toUpperCase() : "");
const isComment = (token: Token) => token.kind.endsWith("Comment");

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
  if (!isName(current) && current.kind !== "Keyword" && current.kind !== ".") return null;
  const parts: Identifier[] =
    current.kind === "."
      ? [{ value: "", quoted: false }]
      : [readIdentifier(state.sliceDoc(current.from, position), position === current.to)];
  let from = current.from;
  if (current.kind !== ".") i--;
  while (i >= 1 && leaves[i].kind === "." && isName(leaves[i - 1]) && parts.length < 4) {
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

export function referencedTables(
  state: EditorState,
  lookup: TableLookup,
  position = state.selection.main.head,
): SqlScope {
  const statement = statementAt(state, position, true);
  if (!statement) return emptyScope();
  const parsed = tokens(state, statement);
  if (!parsed) return emptyScope();
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
    return emptyScope();
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
    return emptyScope();
  if (
    !["SELECT", "WITH", "UPDATE", "DELETE", "INSERT", "EXPLAIN"].includes(word(block[0])) ||
    block.some((t) => t.kind === "⚠")
  )
    return emptyScope();
  const bindings: { table?: TableRef; qualifier?: Qualifier; name: string }[] = [];
  let inFrom = false;
  for (let i = 0; i < block.length; i++) {
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
    while (["ONLY", "LATERAL"].includes(word(block[next]))) next++;
    const parts: Identifier[] = [];
    if (isName(block[next])) {
      parts.push(readIdentifier(block[next++].text));
      while (block[next]?.text === "." && isName(block[next + 1]) && parts.length < 3) {
        parts.push(readIdentifier(block[next + 1].text));
        next += 2;
      }
    }
    const incompleteName = block[next]?.text === ".";
    let derived = false;
    if (block[next]?.children) {
      derived = keyword !== "INTO";
      next++;
    }
    if (!parts.length && !derived) continue;
    if (word(block[next]) === "AS") next++;
    const alias = isName(block[next]) ? block[next++] : undefined;
    if (alias && block[next]?.children) {
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
    bindings.push({
      table,
      name,
      qualifier: alias ? { path: [name], sql: [alias.text] } : undefined,
    });
    i = next - 1;
  }
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
