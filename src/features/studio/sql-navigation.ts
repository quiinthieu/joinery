import type { EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import type {
  AppError,
  ColumnInfo,
  DatabaseObjectRef,
  DatabaseObjectSummary,
  EngineKind,
  ErrorLocation,
  TableInfo,
  TableRef,
  TableSchema,
} from "../../ipc/generated";
import { tableInfoKindToObjectKind } from "../connections/explorer-tree";
import {
  allStatementTokenBlocks,
  statementTokensAt,
  tableLookup,
  type SqlToken,
  type TableLookup,
} from "./sql-context";
import {
  identifierValue,
  quoteIdentifier,
  readIdentifier,
  type Identifier,
} from "./sql-identifiers";

export interface StatementRange {
  from: number;
  to: number;
}

export interface ResolvedDocumentErrorRange {
  from: number;
  to: number;
  caret: number;
  line: number;
  column: number;
}

export interface SqlLocalOccurrence {
  from: number;
  to: number;
  line: number;
  column: number;
  lineText: string;
  isDeclaration: boolean;
}

export type ResolvedSqlSymbol = { usagesLimited?: boolean } & (
  | {
      kind: "cte";
      name: string;
      tokenSpan: StatementRange;
      declarationSpan: StatementRange;
      columns: string[];
      occurrences: SqlLocalOccurrence[];
    }
  | {
      kind: "alias";
      name: string;
      tokenSpan: StatementRange;
      declarationSpan: StatementRange;
      table: TableRef | null;
      object: DatabaseObjectRef | null;
      cteName: string | null;
      schema: TableSchema | null;
      occurrences: SqlLocalOccurrence[];
    }
  | {
      kind: "projection";
      name: string;
      relationName: string;
      tokenSpan: StatementRange;
      declarationSpan: StatementRange;
      sourceTable: TableRef;
      sourceColumn: ColumnInfo;
      occurrences: SqlLocalOccurrence[];
    }
  | {
      kind: "column";
      name: string;
      tokenSpan: StatementRange;
      table: TableRef;
      object: DatabaseObjectRef;
      column: ColumnInfo | null;
      schema: TableSchema | null;
      occurrences: SqlLocalOccurrence[];
    }
  | {
      kind: "object";
      object: DatabaseObjectRef;
      table: TableRef | null;
      schema: TableSchema | null;
      summary: DatabaseObjectSummary | null;
      estimatedRows: number | null;
      totalBytes: number | null;
      tokenSpan: StatementRange;
      occurrences: SqlLocalOccurrence[];
    }
);

export interface SqlSymbolContext {
  engine: EngineKind;
  tables: readonly TableInfo[];
  databaseObjects?: readonly DatabaseObjectSummary[];
  schemas?: ReadonlyMap<string, TableSchema>;
  database?: string;
  defaultSchema?: string;
  searchPath?: string[];
  unqualifiedTables?: TableRef[];
  caseSensitiveTableNames?: boolean;
}

export const schemaCacheKey = (table: TableRef): string =>
  JSON.stringify([table.schema, table.name]);

const isIgnorableStatementChild = (name: string) =>
  name === "Semi" || name === ";" || name.endsWith("Comment");

/**
 * Computes ordered non-empty SQL statement spans in UTF-16 code units, ignoring
 * leading/standalone comments, empty semicolons, and semicolons inside strings,
 * quoted identifiers, dollar-quoted strings, or comments.
 */
export function statementRanges(state: EditorState): StatementRange[] {
  const tree = ensureSyntaxTree(state, state.doc.length, 75);
  if (!tree) return [];
  const ranges: StatementRange[] = [];
  for (let node = tree.topNode.firstChild; node; node = node.nextSibling) {
    if (node.name !== "Statement") continue;
    let firstSqlChild: typeof node | null = null;
    for (let child = node.firstChild; child; child = child.nextSibling) {
      if (child.to <= child.from || isIgnorableStatementChild(child.name)) continue;
      if (!state.sliceDoc(child.from, child.to).trim()) continue;
      firstSqlChild = child;
      break;
    }
    if (!firstSqlChild) continue;
    ranges.push({ from: firstSqlChild.from, to: node.to });
  }
  return ranges;
}

/**
 * Finds the previous (`-1`) or next (`1`) SQL statement relative to the main
 * caret position (`state.selection.main.head`).
 */
export function adjacentStatement(state: EditorState, direction: -1 | 1): StatementRange | null {
  const ranges = statementRanges(state);
  if (ranges.length === 0) return null;
  const head = state.selection.main.head;
  const containingIndex = ranges.findIndex((range) => head >= range.from && head <= range.to);
  if (containingIndex >= 0) {
    return ranges[containingIndex + direction] ?? null;
  }
  if (direction === 1) {
    return ranges.find((range) => range.from > head) ?? null;
  }
  for (let i = ranges.length - 1; i >= 0; i -= 1) {
    if (ranges[i].from < head) return ranges[i];
  }
  return null;
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

function clampUtf16Boundary(text: string, pos: number, roundUp: boolean): number {
  const clamped = Math.max(0, Math.min(text.length, Number.isFinite(pos) ? Math.trunc(pos) : 0));
  if (clamped > 0 && clamped < text.length) {
    const prev = text.charCodeAt(clamped - 1);
    const curr = text.charCodeAt(clamped);
    if (isHighSurrogate(prev) && isLowSurrogate(curr)) {
      return roundUp ? clamped + 1 : clamped - 1;
    }
  }
  return clamped;
}

function stepForwardCodePoint(text: string, pos: number): number {
  if (pos >= text.length) return text.length;
  const curr = text.charCodeAt(pos);
  if (pos + 1 < text.length && isHighSurrogate(curr) && isLowSurrogate(text.charCodeAt(pos + 1))) {
    return pos + 2;
  }
  return pos + 1;
}

function stepBackwardCodePoint(text: string, pos: number): number {
  if (pos <= 0) return 0;
  const prev = text.charCodeAt(pos - 1);
  if (pos >= 2 && isLowSurrogate(prev) && isHighSurrogate(text.charCodeAt(pos - 2))) {
    return pos - 2;
  }
  return pos - 1;
}

function lineAndColumnAtUtf16(text: string, targetUtf16: number): { line: number; column: number } {
  const limit = clampUtf16Boundary(text, targetUtf16, false);
  let line = 1;
  let column = 1;
  let i = 0;
  while (i < limit) {
    const code = text.charCodeAt(i);
    if (code === 0x0a /* \n */) {
      line += 1;
      column = 1;
      i += 1;
    } else if (code === 0x0d /* \r */) {
      if (i + 1 < text.length && text.charCodeAt(i + 1) === 0x0a) {
        i += 1;
      } else {
        column += 1;
        i += 1;
      }
    } else if (i + 1 < limit && isHighSurrogate(code) && isLowSurrogate(text.charCodeAt(i + 1))) {
      column += 1;
      i += 2;
    } else {
      column += 1;
      i += 1;
    }
  }
  return { line, column };
}

/**
 * Translates a backend `ErrorLocation` (relative to the submitted SQL slice at
 * `offsetUtf16`) into safe UTF-16 document bounds and 1-based Unicode scalar
 * `(line, column)` coordinates suitable for CodeMirror decorations and jumping.
 */
export function resolveDocumentErrorRange(
  docText: string,
  location: ErrorLocation,
  offsetUtf16 = 0,
): ResolvedDocumentErrorRange {
  const base = Number.isFinite(offsetUtf16) ? Math.max(0, Math.trunc(offsetUtf16)) : 0;
  const caret = clampUtf16Boundary(docText, base + location.span.startUtf16, false);
  let from = caret;
  let to = clampUtf16Boundary(
    docText,
    Math.max(base + location.span.startUtf16, base + location.span.endUtf16),
    true,
  );
  if (docText.length > 0 && from === to) {
    if (from < docText.length && docText[from] !== "\n" && docText[from] !== "\r") {
      to = stepForwardCodePoint(docText, from);
    } else if (from > 0) {
      from = stepBackwardCodePoint(docText, from);
    } else {
      to = stepForwardCodePoint(docText, from);
    }
  }
  const { line, column } = lineAndColumnAtUtf16(docText, caret);
  return { from, to, caret, line, column };
}

export interface SqlExecutionOrigin {
  document: string;
  query: string;
  fromUtf16: number;
}

/** CodeMirror stores one UTF-16 newline unit per line break. */
export function editorDocument(text: string): string {
  return text.replace(/\r\n?/g, "\n");
}

/** Resolve only against the exact document and submitted slice that failed. */
export function resolveExecutionError(
  document: string,
  error?: AppError | null,
  origin?: SqlExecutionOrigin,
): ResolvedDocumentErrorRange | null {
  const span = error?.location?.span;
  if (
    !origin ||
    !span ||
    editorDocument(origin.document) !== editorDocument(document) ||
    !origin.query ||
    !Number.isSafeInteger(origin.fromUtf16) ||
    origin.fromUtf16 < 0 ||
    origin.document.slice(origin.fromUtf16, origin.fromUtf16 + origin.query.length) !==
      origin.query ||
    !Number.isSafeInteger(span.startUtf16) ||
    !Number.isSafeInteger(span.endUtf16) ||
    span.startUtf16 < 0 ||
    span.endUtf16 < span.startUtf16 ||
    span.endUtf16 > origin.query.length
  )
    return null;
  const rawStart = origin.fromUtf16 + span.startUtf16;
  const rawEnd = origin.fromUtf16 + span.endUtf16;
  const from = editorDocument(origin.document.slice(0, rawStart)).length;
  const to = editorDocument(origin.document.slice(0, rawEnd)).length;
  const resolved = resolveDocumentErrorRange(editorDocument(document), {
    ...error!.location!,
    span: { ...span, startUtf16: from, endUtf16: to },
  });
  return {
    ...resolved,
    from: Math.max(
      editorDocument(origin.document.slice(0, origin.fromUtf16)).length,
      resolved.from,
    ),
    to: Math.min(
      editorDocument(origin.document.slice(0, origin.fromUtf16 + origin.query.length)).length,
      resolved.to,
    ),
  };
}

// --- Symbol resolution (E11 Go to definition / usages, E16 Quick documentation) ---

const RESERVED_NON_SYMBOL_KEYWORDS = new Set([
  "SELECT",
  "FROM",
  "WHERE",
  "JOIN",
  "INNER",
  "LEFT",
  "RIGHT",
  "FULL",
  "OUTER",
  "CROSS",
  "NATURAL",
  "ON",
  "USING",
  "AND",
  "OR",
  "NOT",
  "IN",
  "IS",
  "NULL",
  "TRUE",
  "FALSE",
  "AS",
  "WITH",
  "RECURSIVE",
  "INSERT",
  "INTO",
  "VALUES",
  "UPDATE",
  "SET",
  "DELETE",
  "GROUP",
  "BY",
  "ORDER",
  "HAVING",
  "LIMIT",
  "OFFSET",
  "UNION",
  "INTERSECT",
  "EXCEPT",
  "ALL",
  "DISTINCT",
  "CASE",
  "WHEN",
  "THEN",
  "ELSE",
  "END",
  "ASC",
  "DESC",
  "NULLS",
  "FIRST",
  "LAST",
  "RETURNING",
  "EXISTS",
  "BETWEEN",
  "LIKE",
  "ILIKE",
  "EXPLAIN",
  "ANALYZE",
  "CAST",
  "OVER",
  "PARTITION",
  "WINDOW",
  "QUALIFY",
  "FOR",
  "LATERAL",
  "ONLY",
  "MATERIALIZED",
]);

const isCommentToken = (token?: SqlToken) => Boolean(token && token.kind.endsWith("Comment"));
const keywordUpper = (token?: SqlToken) =>
  token?.kind === "Keyword" ? token.text.toUpperCase() : "";
const isRelationDeclName = (token?: SqlToken) =>
  token?.kind === "Identifier" || token?.kind === "QuotedIdentifier";
const isSymbolNameToken = (token?: SqlToken) =>
  Boolean(
    token &&
    (token.kind === "Identifier" ||
      token.kind === "QuotedIdentifier" ||
      ((token.kind === "Keyword" || token.kind === "Type") &&
        !RESERVED_NON_SYMBOL_KEYWORDS.has(token.text.toUpperCase()))),
  );

function cleanTokens(list: readonly SqlToken[]): SqlToken[] {
  return list.filter((t) => !isCommentToken(t));
}

function buildLookup(context: SqlSymbolContext): TableLookup {
  return tableLookup(
    [...context.tables],
    context.engine,
    context.database,
    context.defaultSchema,
    context.searchPath,
    context.unqualifiedTables,
    context.caseSensitiveTableNames,
  );
}

function normalizeSymbolKey(ident: Identifier, engine: EngineKind): string {
  const val = identifierValue(ident, engine);
  return engine === "mysql" && !ident.quoted ? val.toLowerCase() : val;
}

function sameNormalizedName(
  candidateName: string,
  ident: Identifier,
  engine: EngineKind,
  caseSensitiveOverride?: boolean,
): boolean {
  if (engine === "mysql" && caseSensitiveOverride === false) {
    return candidateName.toLowerCase() === ident.value.toLowerCase();
  }
  return candidateName === identifierValue(ident, engine);
}

interface CteDefinition {
  name: string;
  key: string;
  nameToken: SqlToken;
  columns: string[];
  columnTokens: SqlToken[];
  bodyToken: SqlToken;
  visibleFrom: number;
  visibleTo: number;
}

interface DetailedBinding {
  relationTokens: SqlToken[];
  parts: Identifier[];
  aliasToken?: SqlToken;
  aliasName?: string;
  aliasKey?: string;
  name: string;
  nameKey: string;
  derived: boolean;
  bodyToken?: SqlToken;
  columnAliases?: boolean;
  cte?: CteDefinition;
  table?: TableRef;
}

interface LocatedChain {
  list: SqlToken[];
  chainTokens: SqlToken[];
  parts: Identifier[];
  clickedIndex: number;
  prevToken?: SqlToken;
  nextToken?: SqlToken;
}

function locateChainInTokens(rawList: readonly SqlToken[], pos: number): LocatedChain | null {
  const list = cleanTokens(rawList);
  for (const token of list) {
    if (token.children && pos >= token.from && pos <= token.to) {
      const inner = locateChainInTokens(token.children, pos);
      if (inner) return inner;
    }
  }
  let hit = list.findIndex((t) => isSymbolNameToken(t) && pos > t.from && pos < t.to);
  if (hit < 0) {
    hit = list.findIndex((t) => isSymbolNameToken(t) && pos === t.from);
  }
  if (hit < 0) {
    hit = list.findIndex((t) => isSymbolNameToken(t) && pos === t.to);
  }
  if (hit < 0) return null;

  let start = hit;
  while (start >= 2 && list[start - 1]?.text === "." && isSymbolNameToken(list[start - 2])) {
    start -= 2;
  }
  let end = hit;
  while (end + 2 < list.length && list[end + 1]?.text === "." && isSymbolNameToken(list[end + 2])) {
    end += 2;
  }
  const chainTokens: SqlToken[] = [];
  for (let k = start; k <= end; k += 2) {
    chainTokens.push(list[k]);
  }
  const clickedIndex = (hit - start) / 2;
  return {
    list,
    chainTokens,
    parts: chainTokens.map((t) => readIdentifier(t.text)),
    clickedIndex,
    prevToken: list[start - 1],
    nextToken: list[end + 1],
  };
}

function collectCtesInList(
  rawList: readonly SqlToken[],
  scopeTo: number,
  engine: EngineKind,
  out: CteDefinition[],
) {
  const list = cleanTokens(rawList);
  if (keywordUpper(list[0]) === "WITH") {
    const recursive = keywordUpper(list[1]) === "RECURSIVE";
    let i = recursive ? 2 : 1;
    while (isRelationDeclName(list[i])) {
      const nameToken = list[i++];
      const ident = readIdentifier(nameToken.text);
      const name = identifierValue(ident, engine);
      const key = normalizeSymbolKey(ident, engine);
      const columns: string[] = [];
      const columnTokens: SqlToken[] = [];
      if (list[i]?.children) {
        for (const colTok of cleanTokens(list[i].children!)) {
          if (colTok.text === ",") continue;
          if (isSymbolNameToken(colTok)) {
            columns.push(identifierValue(readIdentifier(colTok.text), engine));
            columnTokens.push(colTok);
          }
        }
        i += 1;
      }
      if (keywordUpper(list[i++]) !== "AS") break;
      while (["NOT", "MATERIALIZED"].includes(keywordUpper(list[i]))) i += 1;
      const bodyToken = list[i++];
      if (!bodyToken?.children) break;
      out.push({
        name,
        key,
        nameToken,
        columns,
        columnTokens,
        bodyToken,
        visibleFrom: recursive ? nameToken.from : bodyToken.to,
        visibleTo: scopeTo,
      });
      collectCtesInList(bodyToken.children, bodyToken.to, engine, out);
      if (list[i]?.text !== ",") break;
      i += 1;
    }
  }
  for (const token of list) {
    if (token.children && keywordUpper(list[0]) !== "WITH") {
      collectCtesInList(token.children, token.to, engine, out);
    } else if (token.children && keywordUpper(list[0]) === "WITH") {
      const isCteBody = out.some(
        (cte) => cte.visibleTo === token.to || cte.visibleFrom === token.to,
      );
      if (!isCteBody) collectCtesInList(token.children, token.to, engine, out);
    }
  }
}

function queryBlockTokensAt(rawList: readonly SqlToken[], pos: number): SqlToken[] {
  let block = cleanTokens(rawList);
  // Strip leading WITH header so main query FROM bindings parse cleanly.
  if (keywordUpper(block[0]) === "WITH") {
    let i = keywordUpper(block[1]) === "RECURSIVE" ? 2 : 1;
    while (isRelationDeclName(block[i])) {
      i += 1;
      if (block[i]?.children) i += 1;
      if (keywordUpper(block[i++]) !== "AS") break;
      while (["NOT", "MATERIALIZED"].includes(keywordUpper(block[i]))) i += 1;
      if (!block[i++]?.children) break;
      if (block[i]?.text !== ",") break;
      i += 1;
    }
    block = block.slice(i);
  }

  // Isolate set-operation arm (UNION / INTERSECT / EXCEPT).
  let start = 0;
  let end = block.length;
  for (let i = 0; i < block.length; i += 1) {
    if (!["UNION", "INTERSECT", "EXCEPT"].includes(keywordUpper(block[i]))) continue;
    if (pos >= block[i].to) {
      start = i + (["ALL", "DISTINCT"].includes(keywordUpper(block[i + 1])) ? 2 : 1);
    } else {
      end = i;
      break;
    }
  }
  return block.slice(start, end);
}

function queryScopesAt(rawList: readonly SqlToken[], pos: number): SqlToken[][] {
  const scopes: SqlToken[][] = [];
  function visit(list: readonly SqlToken[]) {
    const clean = cleanTokens(list);
    if (["SELECT", "WITH", "UPDATE", "DELETE", "INSERT"].includes(keywordUpper(clean[0]))) {
      scopes.push(queryBlockTokensAt(clean, pos));
    }
    for (const token of clean) {
      if (token.children && pos >= token.from && pos <= token.to) visit(token.children);
    }
  }
  visit(rawList);
  return scopes.reverse();
}

function parseDetailedBindings(
  block: readonly SqlToken[],
  visibleCtes: ReadonlyMap<string, CteDefinition>,
  lookup: TableLookup,
): DetailedBinding[] {
  const bindings: DetailedBinding[] = [];
  const engine = lookup.engine;
  let inFrom = false;

  for (let i = 0; i < block.length; i += 1) {
    const kw = keywordUpper(block[i]);
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
      ].includes(kw)
    ) {
      inFrom = false;
    }
    const isRel =
      ["FROM", "JOIN", "UPDATE", "INTO"].includes(kw) ||
      (kw === "USING" && keywordUpper(block[0]) === "DELETE" && !block[i + 1]?.children) ||
      (inFrom && block[i].text === ",");
    if (!isRel) continue;
    inFrom = kw !== "INTO";
    let next = i + 1;
    while (block[next] && ["ONLY", "LATERAL"].includes(keywordUpper(block[next]))) {
      next += 1;
    }
    const relationTokens: SqlToken[] = [];
    const parts: Identifier[] = [];
    if (block[next] && isSymbolNameToken(block[next])) {
      relationTokens.push(block[next]);
      parts.push(readIdentifier(block[next].text));
      next += 1;
      while (
        block[next]?.text === "." &&
        block[next + 1] &&
        isSymbolNameToken(block[next + 1]) &&
        parts.length < 3
      ) {
        relationTokens.push(block[next + 1]);
        parts.push(readIdentifier(block[next + 1].text));
        next += 2;
      }
    }
    let derived = false;
    let bodyToken: SqlToken | undefined;
    if (block[next]?.children) {
      derived = kw !== "INTO";
      // A table-valued function's argument list is not a derived SELECT.
      if (derived && parts.length === 0) bodyToken = block[next];
      next += 1;
    }
    if (!parts.length && !derived) continue;
    if (keywordUpper(block[next]) === "AS") next += 1;
    const aliasToken =
      block[next] &&
      isRelationDeclName(block[next]) &&
      !RESERVED_NON_SYMBOL_KEYWORDS.has(block[next].text.toUpperCase())
        ? block[next++]
        : undefined;
    const columnAliases = !!(aliasToken && block[next]?.children);
    if (columnAliases) {
      derived = true;
      next += 1;
    }
    const aliasIdent = aliasToken ? readIdentifier(aliasToken.text) : undefined;
    const fallbackIdent = parts.at(-1) ?? { value: "", quoted: false };
    const name = aliasIdent
      ? identifierValue(aliasIdent, engine)
      : identifierValue(fallbackIdent, engine);
    const nameKey = aliasIdent
      ? normalizeSymbolKey(aliasIdent, engine)
      : normalizeSymbolKey(fallbackIdent, engine);
    const cte =
      !derived && parts.length === 1
        ? visibleCtes.get(normalizeSymbolKey(parts[0], engine))
        : undefined;
    const table = !derived && !cte && parts.length > 0 ? lookup.resolve(parts) : undefined;
    bindings.push({
      relationTokens,
      parts,
      aliasToken,
      aliasName: aliasIdent ? identifierValue(aliasIdent, engine) : undefined,
      aliasKey: aliasIdent ? normalizeSymbolKey(aliasIdent, engine) : undefined,
      name,
      nameKey,
      derived,
      bodyToken,
      columnAliases,
      cte,
      table,
    });
    i = next - 1;
  }
  return bindings;
}

interface ProjectionOutput {
  ident: Identifier;
  declaration: SqlToken;
  aliasDeclaration: boolean;
  relationName: string;
  sourceTable: TableRef;
  sourceColumn: ColumnInfo;
}

/** Statement-local, bounded inference. Unknown outputs never mean an absent column. */
function projectionResolver(
  tokens: readonly SqlToken[],
  statementEnd: number,
  context: SqlSymbolContext,
  lookup: TableLookup,
) {
  const cache = new Map<number, ProjectionOutput[] | null>();
  const { allCtes } = ctesAt(tokens, statementEnd, statementEnd, context.engine);
  const ctesByBody = new Map(allCtes.map((cte) => [cte.bodyToken.from, cte]));

  function outputs(binding: DetailedBinding, depth = 0): ProjectionOutput[] | null {
    if (binding.columnAliases) return null;
    const body = binding.cte?.bodyToken ?? binding.bodyToken;
    if (!body?.children || depth >= 16) return null;
    if (cache.has(body.from)) return cache.get(body.from)!;
    if (cache.size >= 128) return null;
    // Install the unknown value before descending, including recursive CTEs.
    cache.set(body.from, null);
    const raw = cleanTokens(body.children);
    if (raw.some((t) => ["UNION", "INTERSECT", "EXCEPT"].includes(keywordUpper(t)))) return null;
    const block = queryBlockTokensAt(raw, body.to - 1);
    if (keywordUpper(block[0]) !== "SELECT") return null;
    const from = block.findIndex((t) => keywordUpper(t) === "FROM");
    if (from < 0) return null;
    let start = ["DISTINCT", "ALL"].includes(keywordUpper(block[1])) ? 2 : 1;
    const select = block.slice(start, from);
    if (!select.length || select.length > 2048) return null;
    const visible = ctesAt(tokens, statementEnd, block[0].from, context.engine).visibleCtes;
    const bindings = parseDetailedBindings(block, visible, lookup);
    const cte = ctesByBody.get(body.from);
    const relationName = cte?.name ?? binding.aliasName ?? "subquery";
    const result: ProjectionOutput[] = [];
    start = 0;
    for (let i = 0; i <= select.length; i += 1) {
      if (i < select.length && select[i].text !== ",") continue;
      const item = select.slice(start, i);
      start = i + 1;
      let alias: SqlToken | undefined;
      if (item.length >= 3 && keywordUpper(item.at(-2)) === "AS") {
        alias = item.pop();
        item.pop();
      } else if (item.length >= 2 && isRelationDeclName(item.at(-1)) && item.at(-2)?.text !== ".") {
        alias = item.pop();
      }
      if (alias && !isSymbolNameToken(alias)) return null;
      if (!item.length || item.length > 7 || item.length % 2 === 0) return null;
      for (let j = 0; j < item.length; j += 1) {
        if (j % 2 === 0 ? !isSymbolNameToken(item[j]) : item[j].text !== ".") return null;
      }
      const names = item.filter((_, j) => j % 2 === 0);
      const parts = names.map((t) => readIdentifier(t.text));
      let source: ReturnType<typeof column> = { known: false };
      if (parts.length === 1) {
        const candidates = bindings.map((b) => column(b, parts[0], depth + 1));
        if (candidates.some((c) => !c.known)) return null;
        const owners = candidates.filter((c) => c.source);
        if (owners.length === 1) source = owners[0];
      } else if (parts.length === 2) {
        const owners = bindings.filter(
          (b) => b.nameKey === normalizeSymbolKey(parts[0], context.engine),
        );
        if (owners.length === 1) source = column(owners[0], parts[1], depth + 1);
      } else {
        // Schema-qualified columns still need a matching, unaliased FROM binding.
        const table = lookup.resolve(parts.slice(0, -1));
        const owners = table
          ? bindings.filter(
              (b) => !b.aliasToken && b.table && schemaCacheKey(b.table) === schemaCacheKey(table),
            )
          : [];
        if (owners.length === 1) source = column(owners[0], parts.at(-1)!, depth + 1);
      }
      if (!source.source) return null;
      const declaration = alias ?? names.at(-1)!;
      result.push({
        ident: readIdentifier(declaration.text),
        declaration,
        aliasDeclaration: !!alias,
        relationName,
        sourceTable: source.source.table,
        sourceColumn: source.source.column,
      });
      if (result.length > 256) return null;
    }
    if (cte?.columnTokens.length) {
      // Partial column lists and alias-column lists are deliberately not inferred.
      if (cte.columnTokens.length !== result.length) return null;
      for (let i = 0; i < result.length; i += 1) {
        result[i] = {
          ...result[i],
          ident: readIdentifier(cte.columnTokens[i].text),
          declaration: cte.columnTokens[i],
          aliasDeclaration: true,
        };
      }
    }
    cache.set(body.from, result);
    return result;
  }

  function column(
    binding: DetailedBinding,
    ident: Identifier,
    depth = 0,
  ): {
    known: boolean;
    source?: { table: TableRef; column: ColumnInfo };
    output?: ProjectionOutput;
  } {
    if (binding.table) {
      const schema = context.schemas?.get(schemaCacheKey(binding.table));
      if (!schema) return { known: false };
      const found = findColumnInSchema(schema, ident, context.engine);
      return { known: true, source: found ? { table: binding.table, column: found } : undefined };
    }
    const projected = outputs(binding, depth);
    if (!projected) return { known: false };
    const matches = projected.filter((o) =>
      sameNormalizedName(identifierValue(o.ident, context.engine), ident, context.engine, false),
    );
    if (matches.length > 1) return { known: false };
    const output = matches[0];
    return {
      known: true,
      output,
      source: output
        ? {
            table: output.sourceTable,
            column: output.sourceColumn,
          }
        : undefined,
    };
  }

  function declarationAt(token: SqlToken): ProjectionOutput | undefined {
    for (const cte of allCtes) {
      if (token.from < cte.nameToken.to || token.to > cte.bodyToken.to) continue;
      const projected = outputs({
        cte,
        derived: false,
        relationTokens: [],
        parts: [],
        name: cte.name,
        nameKey: cte.key,
      });
      const found = projected?.find((o) => o.aliasDeclaration && o.declaration.from === token.from);
      if (found) return found;
    }
    for (const scope of queryScopesAt(tokens, token.from)) {
      const visible = ctesAt(
        tokens,
        statementEnd,
        scope[0]?.from ?? token.from,
        context.engine,
      ).visibleCtes;
      for (const binding of parseDetailedBindings(scope, visible, lookup)) {
        const body = binding.bodyToken;
        if (!body || token.from <= body.from || token.to >= body.to) continue;
        const found = outputs(binding)?.find(
          (o) => o.aliasDeclaration && o.declaration.from === token.from,
        );
        if (found) return found;
      }
    }
    return undefined;
  }
  return { outputs, column, declarationAt };
}

function makeOccurrence(
  state: EditorState,
  from: number,
  to: number,
  isDeclaration: boolean,
): SqlLocalOccurrence {
  const cmLine = state.doc.lineAt(Math.max(0, Math.min(state.doc.length, from)));
  const line = cmLine.number;
  const column = [...cmLine.text.slice(0, from - cmLine.from)].length + 1;
  const lineText = cmLine.text.trim().slice(0, 120);
  return { from, to, line, column, lineText, isDeclaration };
}

export interface AliasRenameTarget {
  name: string;
  occurrences: SqlLocalOccurrence[];
  preview(state: EditorState, name: string): { from: number; to: number; insert: string }[];
}

/** A complete, bounded local edit analysis. Never reuse the capped navigation usages. */
export function analyzeAliasRename(
  state: EditorState,
  pos: number,
  engine: EngineKind,
): AliasRenameTarget {
  const unsupported = () => new Error("Alias rename cannot safely analyze this SQL shape.");
  const tokens = statementTokensAt(state, pos)?.tokens;
  if (!tokens) throw new Error("Statement is incomplete or exceeds the rename analysis limit.");
  const tree = ensureSyntaxTree(state, state.doc.length, 75);
  if (!tree) throw unsupported();
  const first = tokens[0]?.from ?? pos;
  const last = tokens.at(-1)?.to ?? pos;
  let invalid = false;
  tree.iterate({
    from: first,
    to: last,
    enter(node) {
      // Lezer's SQL lexer inserts an empty identifier after the dot in alias.*.
      if (
        node.type.isError &&
        !(node.from === node.to && state.sliceDoc(node.from - 1, node.from + 1) === ".*")
      )
        invalid = true;
    },
  });
  if (invalid) throw unsupported();

  type Scope = DetailedBinding[];
  type Reference = {
    token: SqlToken;
    env: Scope[];
    binding?: DetailedBinding;
    qualified: boolean;
    unsupported: boolean;
  };
  const references: Reference[] = [];
  const declarations = new Map<number, DetailedBinding>();
  const allScopes: Scope[] = [];
  const key = (token: SqlToken) => {
    const value = identifierValue(readIdentifier(token.text), engine);
    return engine === "mysql" ? value.toLowerCase() : value;
  };
  const exceedsIdentifierLimit = (name: string) =>
    (engine === "postgres" && new TextEncoder().encode(name).length > 63) ||
    (engine === "mysql" && [...name].length > 64);
  const resolve = (env: Scope[], name: string, target?: DetailedBinding, replacement?: string) => {
    for (const scope of env) {
      const binding = scope.find((item) => (item === target ? replacement : item.nameKey) === name);
      if (binding) return binding;
    }
    return undefined;
  };
  const forbidden = new Set([
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "MINUS",
    "LATERAL",
    "APPLY",
    "PIVOT",
    "UNPIVOT",
    "MATCH_RECOGNIZE",
    "TABLESAMPLE",
    "ASOF",
    "STRAIGHT_JOIN",
    "INTO",
    "CONNECT",
    "START",
    "MODEL",
    "ONLY",
  ]);
  let queryCount = 0;
  function visit(raw: readonly SqlToken[], outer: Scope[], depth: number) {
    if (depth > 16) throw new Error("Query nesting exceeds the rename analysis limit.");
    const list = cleanTokens(raw);
    for (const token of list) {
      if (
        (token.kind === "Keyword" || token.kind === "Identifier") &&
        forbidden.has(token.text.toUpperCase())
      )
        throw unsupported();
      if (token.children && !token.text.endsWith(")")) throw unsupported();
    }
    const isQuery = ["SELECT", "WITH"].includes(keywordUpper(list[0]));
    if (!isQuery) {
      if (
        list.some((token) =>
          ["SELECT", "WITH", "UPDATE", "DELETE", "INSERT"].includes(keywordUpper(token)),
        )
      )
        throw unsupported();
      scan(list, outer, depth);
      return;
    }
    if (++queryCount > 64) throw new Error("Query count exceeds the rename analysis limit.");
    const block = queryBlockTokensAt(list, list.at(-1)?.to ?? pos);
    if (keywordUpper(block[0]) !== "SELECT") throw unsupported();
    const scope = parseDetailedBindings(block, new Map(), tableLookup([], engine));
    for (const binding of scope) {
      if (binding.columnAliases || (binding.derived && (!binding.bodyToken || !binding.aliasToken)))
        throw unsupported();
      if (
        binding.bodyToken &&
        !["SELECT", "WITH"].includes(keywordUpper(cleanTokens(binding.bodyToken.children ?? [])[0]))
      )
        throw unsupported();
      const nameToken = binding.aliasToken ?? binding.relationTokens.at(-1);
      if (!nameToken) throw unsupported();
      if (exceedsIdentifierLimit(readIdentifier(nameToken.text).value)) throw unsupported();
      binding.nameKey = key(nameToken);
      if (binding.aliasToken) declarations.set(binding.aliasToken.from, binding);
    }
    if (new Set(scope.map((item) => item.nameKey)).size !== scope.length) throw unsupported();
    allScopes.push(scope);
    // WITH bodies and non-lateral FROM bodies cannot see this query's FROM aliases.
    for (const [index, token] of list.entries()) {
      if (token.from >= block[0].from) break;
      if (token.children && ["AS", "MATERIALIZED"].includes(keywordUpper(list[index - 1]))) {
        if (!["SELECT", "WITH"].includes(keywordUpper(cleanTokens(token.children)[0])))
          throw unsupported();
        visit(token.children, outer, depth + 1);
      }
    }
    scan(block, [scope, ...outer], depth, scope);
  }
  function joinEnvironments(list: SqlToken[], scope: Scope, outer: Scope[]) {
    const environments = new Map<number, Scope[]>();
    const atoms = new Map(
      scope.map((binding) => [binding.relationTokens[0]?.from ?? binding.bodyToken!.from, binding]),
    );
    // Each pending JOIN owns its left operand. A subsequent ON closes the nearest
    // pending JOIN, so a JOIN b JOIN c ON ... ON ... keeps b/c separate from a.
    const pending: { left: Scope; implicit: boolean; cross: boolean }[] = [];
    let operand: Scope = [];
    let condition: Scope[] | undefined;
    let inFrom = false;
    let needsRelation = false;
    const finish = () => {
      if (needsRelation || (engine !== "mysql" && pending.length)) throw unsupported();
      pending.length = 0;
    };
    for (const [index, token] of list.entries()) {
      const kw = keywordUpper(token);
      if (kw === "FROM") {
        if (inFrom) throw unsupported();
        inFrom = true;
        needsRelation = true;
      } else if (!inFrom) continue;
      else if (
        [
          "WHERE",
          "GROUP",
          "ORDER",
          "HAVING",
          "LIMIT",
          "OFFSET",
          "WINDOW",
          "QUALIFY",
          "FOR",
          "FETCH",
        ].includes(kw)
      ) {
        finish();
        break;
      } else if (token.text === ",") {
        // JOIN binds more tightly than comma: earlier comma groups are not ON operands.
        finish();
        operand = [];
        condition = undefined;
        needsRelation = true;
      } else if (kw === "JOIN") {
        if (needsRelation || !operand.length) throw unsupported();
        const modifiers: string[] = [];
        for (let previous = index - 1; previous >= 0; previous--) {
          const modifier = keywordUpper(list[previous]);
          if (!["NATURAL", "INNER", "LEFT", "RIGHT", "FULL", "OUTER", "CROSS"].includes(modifier))
            break;
          modifiers.push(modifier);
        }
        pending.push({
          left: operand,
          implicit: modifiers.includes("CROSS") || modifiers.includes("NATURAL"),
          cross: modifiers.includes("CROSS"),
        });
        operand = [];
        condition = undefined;
        needsRelation = true;
      } else if (kw === "ON" || kw === "USING") {
        const join = pending.pop();
        if (!join || needsRelation) throw unsupported();
        operand = [...join.left, ...operand];
        condition = [operand, ...outer];
        if (kw === "USING" && keywordUpper(list[index + 2]) === "AS") throw unsupported();
      } else if (atoms.has(token.from)) {
        if (!needsRelation) throw unsupported();
        const binding = atoms.get(token.from)!;
        operand = [binding];
        needsRelation = false;
        const end =
          binding.aliasToken?.to ?? binding.bodyToken?.to ?? binding.relationTokens.at(-1)!.to;
        const following = list.find((item) => item.from >= end);
        // MySQL also allows CROSS JOIN ... ON/USING, equivalent to INNER JOIN.
        if (
          engine === "mysql" &&
          pending.at(-1)?.cross &&
          ["ON", "USING"].includes(keywordUpper(following))
        )
          pending[pending.length - 1].implicit = false;
        while (pending.at(-1)?.implicit) operand = [...pending.pop()!.left, ...operand];
      }
      if (condition) environments.set(token.from, condition);
    }
    if (inFrom) finish();
    return environments;
  }
  function scan(list: SqlToken[], env: Scope[], depth: number, scope: Scope = []) {
    const relationNames = new Set(
      scope.flatMap((binding) => binding.relationTokens.map((token) => token.from)),
    );
    const bodies = new Set(
      scope.flatMap((binding) => (binding.bodyToken ? [binding.bodyToken.from] : [])),
    );
    const environments = scope.length ? joinEnvironments(list, scope, env.slice(1)) : new Map();
    for (let index = 0; index < list.length; index++) {
      const token = list[index];
      const tokenEnv: Scope[] = environments.get(token.from) ?? env;
      if (token.children) {
        visit(token.children, bodies.has(token.from) ? tokenEnv.slice(1) : tokenEnv, depth + 1);
        continue;
      }
      if (
        !isSymbolNameToken(token) ||
        declarations.has(token.from) ||
        relationNames.has(token.from) ||
        list[index - 1]?.text === "."
      )
        continue;
      const qualified = list[index + 1]?.text === ".";
      references.push({
        token,
        env: tokenEnv,
        binding: resolve(tokenEnv, key(token)),
        qualified,
        unsupported:
          qualified &&
          ((!isSymbolNameToken(list[index + 2]) && list[index + 2]?.text !== "*") ||
            list[index + 3]?.text === "." ||
            !!list[index + 3]?.children),
      });
    }
  }
  if (!["SELECT", "WITH"].includes(keywordUpper(cleanTokens(tokens)[0]))) throw unsupported();
  visit(tokens, [], 0);
  const hit = (token: SqlToken) => pos >= token.from && pos <= token.to;
  const target =
    [...declarations.values()].find((binding) => hit(binding.aliasToken!)) ??
    references.find((ref) => ref.qualified && hit(ref.token))?.binding;
  if (!target?.aliasToken)
    throw new Error("Place the caret on a table alias or its qualified reference.");
  const uses = references.filter((ref) => ref.binding === target);
  if (uses.some((ref) => !ref.qualified || ref.unsupported)) throw unsupported();
  const occurrences = [
    makeOccurrence(state, target.aliasToken.from, target.aliasToken.to, true),
    ...uses.map((ref) => makeOccurrence(state, ref.token.from, ref.token.to, false)),
  ].sort((a, b) => a.from - b.from);
  if (occurrences.length > 500) throw new Error("Alias exceeds the 500 occurrence rename limit.");
  return {
    name: identifierValue(readIdentifier(target.aliasToken.text), engine),
    occurrences,
    preview(current, name) {
      if (current.doc !== state.doc)
        throw new Error("The document changed. Start a new rename preview.");
      if (!name || name.length > 128 || /[\u0000-\u001f\u007f]/u.test(name))
        throw new Error("Use an alias of 1–128 characters without control characters.");
      if (exceedsIdentifierLimit(name))
        throw new Error("Alias exceeds this database's identifier length limit.");
      const replacement = engine === "mysql" ? name.toLowerCase() : name;
      if (replacement === target.nameKey) throw new Error("Choose a different alias.");
      const owning = allScopes.find((scope) => scope.includes(target))!;
      if (owning.some((binding) => binding !== target && binding.nameKey === replacement))
        throw new Error("Alias conflicts with another relation in this query.");
      for (const ref of references) {
        const after = resolve(
          ref.env,
          ref.binding === target ? replacement : key(ref.token),
          target,
          replacement,
        );
        if (
          (ref.qualified && after !== ref.binding) ||
          (!ref.qualified && key(ref.token) === replacement && after === target)
        )
          throw new Error("Rename would capture or conflict with another reference.");
      }
      return occurrences.map((occurrence) => {
        const original = readIdentifier(state.sliceDoc(occurrence.from, occurrence.to));
        const quote = engine === "mysql" ? "`" : '"';
        return {
          from: occurrence.from,
          to: occurrence.to,
          insert: original.quoted
            ? quote + name.split(quote).join(quote + quote) + quote
            : quoteIdentifier(name, engine),
        };
      });
    },
  };
}

function dedupeOccurrences(items: SqlLocalOccurrence[]): SqlLocalOccurrence[] {
  const byRange = new Map<string, SqlLocalOccurrence>();
  for (const item of items) {
    const key = `${item.from}:${item.to}`;
    const existing = byRange.get(key);
    if (!existing || (item.isDeclaration && !existing.isDeclaration)) {
      byRange.set(key, item);
    }
  }
  return [...byRange.values()].sort((a, b) => a.from - b.from);
}

function walkChains(
  rawList: readonly SqlToken[],
  visit: (chain: SqlToken[], parts: Identifier[], nextToken?: SqlToken) => void,
) {
  const list = cleanTokens(rawList);
  let i = 0;
  while (i < list.length) {
    const tok = list[i];
    if (tok.children) {
      walkChains(tok.children, visit);
      i += 1;
      continue;
    }
    if (!isSymbolNameToken(tok)) {
      i += 1;
      continue;
    }
    const chain: SqlToken[] = [tok];
    let end = i;
    while (
      end + 2 < list.length &&
      list[end + 1]?.text === "." &&
      isSymbolNameToken(list[end + 2])
    ) {
      chain.push(list[end + 2]);
      end += 2;
    }
    visit(
      chain,
      chain.map((t) => readIdentifier(t.text)),
      list[end + 1],
    );
    i = end + 1;
  }
}

function tableToObjectMeta(
  table: TableRef,
  context: SqlSymbolContext,
): {
  object: DatabaseObjectRef;
  summary: DatabaseObjectSummary | null;
  estimatedRows: number | null;
  totalBytes: number | null;
} {
  const summary =
    context.databaseObjects?.find(
      (s) =>
        s.object.schema === table.schema &&
        s.object.name === table.name &&
        ["table", "partitionedTable", "view", "materializedView"].includes(s.object.kind),
    ) ?? null;
  const info = context.tables.find(
    (t) => t.table.schema === table.schema && t.table.name === table.name,
  );
  const kind = summary
    ? summary.object.kind
    : info
      ? tableInfoKindToObjectKind(info.kind)
      : "table";
  return {
    object: summary?.object ?? {
      schema: table.schema,
      name: table.name,
      kind,
      identity: null,
    },
    summary,
    estimatedRows: summary?.estimatedRows ?? info?.estimatedRows ?? null,
    totalBytes: summary?.totalBytes ?? info?.totalBytes ?? null,
  };
}

function resolveNonTabularObject(
  parts: Identifier[],
  preferRoutines: boolean,
  context: SqlSymbolContext,
): DatabaseObjectSummary | null {
  const objects = context.databaseObjects ?? [];
  if (!objects.length || parts.length === 0 || parts.length > 3) return null;
  const { engine, database } = context;
  let schemaIdent: Identifier | undefined;
  let nameIdent: Identifier;
  if (parts.length === 1) {
    nameIdent = parts[0];
  } else if (parts.length === 2) {
    schemaIdent = parts[0];
    nameIdent = parts[1];
  } else {
    if (!database || !sameNormalizedName(database, parts[0], engine)) return null;
    schemaIdent = parts[1];
    nameIdent = parts[2];
  }
  const allowedKinds = preferRoutines
    ? ["function", "procedure"]
    : ["sequence", "function", "procedure"];
  let candidates = objects.filter(
    (entry) =>
      allowedKinds.includes(entry.object.kind) &&
      sameNormalizedName(
        entry.object.name,
        nameIdent,
        engine,
        engine === "mysql" ? false : undefined,
      ) &&
      (!schemaIdent ||
        sameNormalizedName(
          entry.object.schema,
          schemaIdent,
          engine,
          context.caseSensitiveTableNames,
        )),
  );
  if (!candidates.length) return null;
  if (!schemaIdent) {
    const pathOrder =
      context.searchPath ??
      (context.defaultSchema ? [context.defaultSchema] : engine === "postgres" ? ["public"] : []);
    candidates =
      pathOrder
        .map((schema) => candidates.filter((c) => c.object.schema === schema))
        .find((matches) => matches.length > 0) ?? [];
  }
  return candidates.length === 1 ? candidates[0] : null;
}

function findColumnInSchema(
  schema: TableSchema | null | undefined,
  colIdent: Identifier,
  engine: EngineKind,
): ColumnInfo | null {
  if (!schema) return null;
  return (
    schema.columns.find((col) => sameNormalizedName(col.name, colIdent, engine, false)) ??
    schema.columns.find((col) => sameNormalizedName(col.name, colIdent, engine, true)) ??
    null
  );
}

function ctesAt(tokens: readonly SqlToken[], end: number, pos: number, engine: EngineKind) {
  const allCtes: CteDefinition[] = [];
  collectCtesInList(tokens, end, engine, allCtes);
  const visibleCtes = new Map<string, CteDefinition>();
  for (const cte of [...new Map(allCtes.map((c) => [c.nameToken.from, c])).values()].sort(
    (a, b) => a.nameToken.from - b.nameToken.from,
  )) {
    if (pos >= cte.visibleFrom && pos <= cte.visibleTo) {
      visibleCtes.set(cte.key, cte);
    }
  }

  return { allCtes, visibleCtes };
}

/**
 * Returns the distinct catalog `TableRef`s relevant to resolving a symbol at
 * `pos` so callers can ensure their `TableSchema`s are loaded on demand.
 */
export function tablesNeededForNavigationAt(
  state: EditorState,
  pos: number,
  context: SqlSymbolContext,
): TableRef[] {
  if (context.engine === "redis") return [];
  const lookup = buildLookup(context);
  const needed = new Map<string, TableRef>();
  const stmt = statementTokensAt(state, pos);
  if (stmt) {
    const { visibleCtes } = ctesAt(stmt.tokens, stmt.to, pos, context.engine);
    const bindings = queryScopesAt(stmt.tokens, pos).flatMap((scope) =>
      parseDetailedBindings(scope, visibleCtes, lookup),
    );
    const located = locateChainInTokens(stmt.tokens, pos);
    const preferred = located
      ? bindings.find((b) => b.nameKey === normalizeSymbolKey(located.parts[0], context.engine))
      : undefined;
    const visited = new Set<number>();
    const visit = (binding: DetailedBinding, depth = 0) => {
      if (binding.table) needed.set(schemaCacheKey(binding.table), binding.table);
      const body = binding.cte?.bodyToken ?? binding.bodyToken;
      if (!body?.children || depth >= 16 || visited.has(body.from) || visited.size >= 128) return;
      visited.add(body.from);
      const block = queryBlockTokensAt(body.children, body.to - 1);
      const visible = ctesAt(
        stmt.tokens,
        stmt.to,
        block[0]?.from ?? body.from,
        context.engine,
      ).visibleCtes;
      for (const inner of parseDetailedBindings(block, visible, lookup)) visit(inner, depth + 1);
    };
    if (preferred) visit(preferred);
    for (const binding of bindings) visit(binding);
    if (located && !preferred) {
      for (let len = 1; len <= Math.min(3, located.parts.length); len += 1) {
        const resolved = lookup.resolve(located.parts.slice(0, len));
        if (resolved) needed.set(schemaCacheKey(resolved), resolved);
      }
    }
  }
  return [...needed.values()];
}

/**
 * Resolves the SQL symbol at `pos` (local CTE, local table alias, table/view
 * column, or catalog object) along with all local occurrences in the editor.
 */
function resolveSymbolCore(
  state: EditorState,
  pos: number,
  context: SqlSymbolContext,
  lookup = buildLookup(context),
  projectionCache = new Map<number, ReturnType<typeof projectionResolver>>(),
): ResolvedSqlSymbol | null {
  if (context.engine === "redis") return null;
  const tree = ensureSyntaxTree(state, state.doc.length, 75);
  if (tree) {
    const left = tree.resolveInner(pos, -1);
    const right = tree.resolveInner(pos, 1);
    if (
      (/String|Comment/.test(left.name) && pos > left.from && pos < left.to) ||
      (/String|Comment/.test(right.name) && pos > right.from && pos < right.to)
    ) {
      return null;
    }
  }
  const stmt = statementTokensAt(state, pos);
  if (!stmt) return null;
  const located = locateChainInTokens(stmt.tokens, pos);
  if (!located) return null;

  const { engine } = context;
  const clickedToken = located.chainTokens[located.clickedIndex];
  const tokenSpan = { from: clickedToken.from, to: clickedToken.to };

  const { allCtes, visibleCtes } = ctesAt(stmt.tokens, stmt.to, pos, engine);
  let projections = projectionCache.get(stmt.from);
  if (!projections) {
    projections = projectionResolver(stmt.tokens, stmt.to, context, lookup);
    projectionCache.set(stmt.from, projections);
  }

  const scopes = queryScopesAt(stmt.tokens, pos);
  const block = scopes[0] ?? [];
  const localBindings = parseDetailedBindings(block, visibleCtes, lookup);
  const bindings = [...localBindings];
  const shadowed = new Set(localBindings.map((b) => b.nameKey));
  for (const scope of scopes.slice(1)) {
    const outer = parseDetailedBindings(scope, visibleCtes, lookup);
    bindings.push(...outer.filter((b) => !shadowed.has(b.nameKey)));
    for (const b of outer) shadowed.add(b.nameKey);
  }

  const buildCteResult = (cte: CteDefinition): ResolvedSqlSymbol => {
    const occurrences: SqlLocalOccurrence[] = [];
    return {
      kind: "cte",
      name: cte.name,
      tokenSpan,
      declarationSpan: { from: cte.nameToken.from, to: cte.nameToken.to },
      columns: cte.columns,
      occurrences: dedupeOccurrences(occurrences),
    };
  };

  const buildAliasResult = (binding: DetailedBinding): ResolvedSqlSymbol => {
    const decl = binding.aliasToken!;
    const occurrences: SqlLocalOccurrence[] = [];
    const schema = binding.table
      ? (context.schemas?.get(schemaCacheKey(binding.table)) ?? null)
      : null;
    const meta = binding.table ? tableToObjectMeta(binding.table, context) : null;
    return {
      kind: "alias",
      name: binding.aliasName!,
      tokenSpan,
      declarationSpan: { from: decl.from, to: decl.to },
      table: binding.table ?? null,
      object: meta?.object ?? null,
      cteName: binding.cte?.name ?? null,
      schema,
      occurrences: dedupeOccurrences(occurrences),
    };
  };

  const buildObjectResult = (
    object: DatabaseObjectRef,
    table: TableRef | null,
    summary: DatabaseObjectSummary | null,
    estimatedRows: number | null,
    totalBytes: number | null,
  ): ResolvedSqlSymbol => {
    const schema = table ? (context.schemas?.get(schemaCacheKey(table)) ?? null) : null;
    const occurrences: SqlLocalOccurrence[] = [];
    return {
      kind: "object",
      object,
      table,
      schema,
      summary,
      estimatedRows,
      totalBytes,
      tokenSpan,
      occurrences: dedupeOccurrences(occurrences),
    };
  };

  const buildColumnResult = (table: TableRef, colIdent: Identifier): ResolvedSqlSymbol | null => {
    const schema = context.schemas?.get(schemaCacheKey(table)) ?? null;
    const colInfo = findColumnInSchema(schema, colIdent, engine);
    if (!colInfo) return null;
    const canonicalName = colInfo.name;
    const meta = tableToObjectMeta(table, context);

    const occurrences: SqlLocalOccurrence[] = [];
    return {
      kind: "column",
      name: canonicalName,
      tokenSpan,
      table,
      object: meta.object,
      column: colInfo,
      schema,
      occurrences: dedupeOccurrences(occurrences),
    };
  };

  const buildProjectionResult = (output: ProjectionOutput): ResolvedSqlSymbol => ({
    kind: "projection",
    name: identifierValue(output.ident, engine),
    relationName: output.relationName,
    tokenSpan,
    declarationSpan: { from: output.declaration.from, to: output.declaration.to },
    sourceTable: output.sourceTable,
    sourceColumn: output.sourceColumn,
    occurrences: [],
  });

  // 1. Direct hit on CTE declaration name
  const directCte = allCtes.find(
    (cte) => clickedToken.from === cte.nameToken.from && clickedToken.to === cte.nameToken.to,
  );
  if (directCte) return buildCteResult(directCte);

  // 2. Direct hit on alias declaration token
  const directAliasBinding = localBindings.find(
    (b) =>
      b.aliasToken &&
      clickedToken.from === b.aliasToken.from &&
      clickedToken.to === b.aliasToken.to,
  );
  if (directAliasBinding) return buildAliasResult(directAliasBinding);

  // 3. Direct hit on a FROM / JOIN / UPDATE / INTO relation name
  const directRelBinding = localBindings.find((b) =>
    b.relationTokens.some((rt) => rt.from === clickedToken.from && rt.to === clickedToken.to),
  );
  if (directRelBinding) {
    if (directRelBinding.cte) return buildCteResult(directRelBinding.cte);
    if (directRelBinding.table) {
      const meta = tableToObjectMeta(directRelBinding.table, context);
      return buildObjectResult(
        meta.object,
        directRelBinding.table,
        meta.summary,
        meta.estimatedRows,
        meta.totalBytes,
      );
    }
    return null;
  }

  const directProjection = projections.declarationAt(clickedToken);
  if (directProjection) return buildProjectionResult(directProjection);

  // 4. Dotted chain (alias.col, table.col, schema.table, schema.table.col, schema.fn(...))
  if (located.chainTokens.length >= 2) {
    const { parts, clickedIndex, nextToken } = located;
    if (nextToken?.children) {
      const routine = resolveNonTabularObject(parts, true, context);
      if (routine) {
        return buildObjectResult(
          routine.object,
          null,
          routine,
          routine.estimatedRows,
          routine.totalBytes,
        );
      }
    }

    if (parts.length === 2) {
      const firstKey = normalizeSymbolKey(parts[0], engine);
      const matchingBindings = bindings.filter((b) => b.nameKey === firstKey);
      if (matchingBindings.length === 1) {
        const bound = matchingBindings[0];
        if (clickedIndex === 0) {
          if (bound.aliasToken) return buildAliasResult(bound);
          if (bound.cte) return buildCteResult(bound.cte);
          if (bound.table) {
            const meta = tableToObjectMeta(bound.table, context);
            return buildObjectResult(
              meta.object,
              bound.table,
              meta.summary,
              meta.estimatedRows,
              meta.totalBytes,
            );
          }
        } else if (clickedIndex === 1) {
          if (bound.table) return buildColumnResult(bound.table, parts[1]);
          const output = projections.column(bound, parts[1]).output;
          return output ? buildProjectionResult(output) : null;
        }
      }
      if (matchingBindings.length > 0) return null;
      const resolvedTable = lookup.resolve(parts);
      if (resolvedTable) {
        const meta = tableToObjectMeta(resolvedTable, context);
        return buildObjectResult(
          meta.object,
          resolvedTable,
          meta.summary,
          meta.estimatedRows,
          meta.totalBytes,
        );
      }
      const nonTabular = resolveNonTabularObject(parts, false, context);
      if (nonTabular) {
        return buildObjectResult(
          nonTabular.object,
          null,
          nonTabular,
          nonTabular.estimatedRows,
          nonTabular.totalBytes,
        );
      }
    }

    if (parts.length === 3) {
      const tablePrefix = lookup.resolve(parts.slice(0, 2));
      if (tablePrefix) {
        if (clickedIndex <= 1) {
          const meta = tableToObjectMeta(tablePrefix, context);
          return buildObjectResult(
            meta.object,
            tablePrefix,
            meta.summary,
            meta.estimatedRows,
            meta.totalBytes,
          );
        }
        return buildColumnResult(tablePrefix, parts[2]);
      }
      const threePartTable = lookup.resolve(parts);
      if (threePartTable) {
        const meta = tableToObjectMeta(threePartTable, context);
        return buildObjectResult(
          meta.object,
          threePartTable,
          meta.summary,
          meta.estimatedRows,
          meta.totalBytes,
        );
      }
    }

    if (parts.length === 4) {
      const tablePrefix = lookup.resolve(parts.slice(0, 3));
      if (tablePrefix) {
        if (clickedIndex <= 2) {
          const meta = tableToObjectMeta(tablePrefix, context);
          return buildObjectResult(
            meta.object,
            tablePrefix,
            meta.summary,
            meta.estimatedRows,
            meta.totalBytes,
          );
        }
        return buildColumnResult(tablePrefix, parts[3]);
      }
    }
    return null;
  }

  // 5. Bare single identifier
  const ident = located.parts[0];
  const key = normalizeSymbolKey(ident, engine);

  if (located.nextToken?.children && keywordUpper(located.prevToken) !== "INTO") {
    const routine = resolveNonTabularObject([ident], true, context);
    if (routine) {
      return buildObjectResult(
        routine.object,
        null,
        routine,
        routine.estimatedRows,
        routine.totalBytes,
      );
    }
  }

  const localSelect = block.slice(
    0,
    block.findIndex((t) => keywordUpper(t) === "FROM") < 0
      ? block.length
      : block.findIndex((t) => keywordUpper(t) === "FROM"),
  );
  if (
    localSelect.some(
      (token, i) =>
        keywordUpper(localSelect[i - 1]) === "AS" &&
        isSymbolNameToken(token) &&
        normalizeSymbolKey(readIdentifier(token.text), engine) === key,
    )
  )
    return null;

  // Bare columns resolve in the nearest scope. Unknown/derived bindings and
  // repeated relations prevent an assertion of unique ownership.
  for (const scope of scopes) {
    const scopedBindings = parseDetailedBindings(scope, visibleCtes, lookup);
    const candidates = scopedBindings.map((binding) => ({
      binding,
      resolved: projections.column(binding, ident),
    }));
    if (candidates.some((c) => !c.resolved.known)) return null;
    const owners = candidates.filter((c) => c.resolved.source);
    if (owners.length > 1) return null;
    if (owners.length === 1) {
      const owner = owners[0];
      return owner.resolved.output
        ? buildProjectionResult(owner.resolved.output)
        : buildColumnResult(owner.binding.table!, ident);
    }
  }

  const cteMatch = visibleCtes.get(key);
  if (cteMatch) {
    return buildCteResult(cteMatch);
  }

  const nonTabular = resolveNonTabularObject([ident], false, context);
  if (nonTabular) {
    return buildObjectResult(
      nonTabular.object,
      null,
      nonTabular,
      nonTabular.estimatedRows,
      nonTabular.totalBytes,
    );
  }

  return null;
}

function symbolIdentity(symbol: ResolvedSqlSymbol): string {
  if (symbol.kind === "alias" || symbol.kind === "cte" || symbol.kind === "projection") {
    return JSON.stringify([
      symbol.kind,
      symbol.declarationSpan.from,
      symbol.declarationSpan.to,
      symbol.name,
    ]);
  }
  if (symbol.kind === "column")
    return JSON.stringify([symbol.kind, symbol.table.schema, symbol.table.name, symbol.name]);
  return JSON.stringify([
    symbol.kind,
    symbol.object.schema,
    symbol.object.name,
    symbol.object.kind,
    symbol.object.identity,
  ]);
}

/** Every candidate is resolved in its own scope; spelling alone is not a usage. */
export function resolveSymbolAt(
  state: EditorState,
  pos: number,
  context: SqlSymbolContext,
): ResolvedSqlSymbol | null {
  const lookup = buildLookup(context);
  const projectionCache = new Map<number, ReturnType<typeof projectionResolver>>();
  const symbol = resolveSymbolCore(state, pos, context, lookup, projectionCache);
  if (!symbol) return null;
  const identity = symbolIdentity(symbol);
  const blocks =
    symbol.kind === "object"
      ? allStatementTokenBlocks(state)
      : [statementTokensAt(state, pos)].filter((b) => b !== null);
  const occurrences: SqlLocalOccurrence[] =
    symbol.kind === "projection"
      ? [makeOccurrence(state, symbol.declarationSpan.from, symbol.declarationSpan.to, true)]
      : [];
  let inspected = 0;
  for (const block of blocks) {
    walkChains(block.tokens, (chain) => {
      if (inspected >= 10000 || occurrences.length >= 500) return;
      let matched: { from: number; to: number; declaration: boolean } | null = null;
      for (const token of chain) {
        inspected += 1;
        const candidate = resolveSymbolCore(state, token.from, context, lookup, projectionCache);
        if (!candidate || symbolIdentity(candidate) !== identity) continue;
        if (symbol.kind === "object") {
          matched = { from: chain[0].from, to: token.to, declaration: false };
        } else {
          const declaration =
            (symbol.kind === "cte" || symbol.kind === "alias" || symbol.kind === "projection") &&
            token.from === symbol.declarationSpan.from;
          matched = { from: token.from, to: token.to, declaration };
          break;
        }
      }
      if (matched)
        occurrences.push(makeOccurrence(state, matched.from, matched.to, matched.declaration));
    });
  }
  return {
    ...symbol,
    occurrences: dedupeOccurrences(occurrences),
    usagesLimited: inspected >= 10000 || occurrences.length >= 500,
  };
}
