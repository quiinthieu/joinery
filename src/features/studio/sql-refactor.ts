import type { ChangeSpec, EditorState } from "@codemirror/state";
import type {
  EngineKind,
  SqlUsageDocument,
  TableInfo,
  TableRef,
  TableSchema,
} from "../../ipc/generated";
import {
  identifierValue,
  quoteIdentifier,
  readIdentifier,
  type Identifier,
} from "./sql-identifiers";
import { statementTokensAt, tableLookup, type SqlToken } from "./sql-context";

const isComment = (t: SqlToken) => t.kind.endsWith("Comment");
const isName = (t?: SqlToken) => t?.kind === "Identifier" || t?.kind === "QuotedIdentifier";
const kw = (t?: SqlToken) => (t?.kind === "Keyword" ? t.text.toUpperCase() : "");

const CLAUSE_STOP_KEYWORDS = new Set([
  "WHERE",
  "GROUP",
  "HAVING",
  "WINDOW",
  "QUALIFY",
  "ORDER",
  "LIMIT",
  "OFFSET",
  "FETCH",
  "FOR",
  "UNION",
  "INTERSECT",
  "EXCEPT",
  "MINUS",
  "ON",
  "USING",
  "INNER",
  "LEFT",
  "RIGHT",
  "FULL",
  "CROSS",
  "NATURAL",
  "JOIN",
  "SET",
  "VALUES",
  "RETURNING",
]);

function stripComments(tokens: readonly SqlToken[]): SqlToken[] {
  return tokens
    .filter((t) => !isComment(t))
    .map((t) => (t.children ? { ...t, children: stripComments(t.children) } : t));
}

function normKey(raw: string, quoted: boolean, engine: EngineKind): string {
  const val = identifierValue({ value: raw, quoted }, engine);
  return engine === "mysql" ? val.toLowerCase() : val;
}

function tokenToIdent(tok: SqlToken): Identifier {
  return readIdentifier(tok.text);
}

interface CteEntry {
  name: string;
  nameToken: SqlToken;
  norm: string;
  headerFrom: number;
  parens: SqlToken;
  explicitColumns: string[] | null;
  materialized: boolean;
}

interface ParsedWithHeader {
  withToken: SqlToken;
  recursive: boolean;
  ctes: CteEntry[];
  bodyStartIndex: number;
}

function parseTopLevelWith(
  tokens: readonly SqlToken[],
  engine: EngineKind,
): ParsedWithHeader | null {
  if (kw(tokens[0]) !== "WITH") return null;
  let idx = 1;
  let recursive = false;
  if (kw(tokens[idx]) === "RECURSIVE") {
    recursive = true;
    idx++;
  }
  const ctes: CteEntry[] = [];
  while (idx < tokens.length) {
    const nameTok = tokens[idx];
    if (!isName(nameTok)) break;
    const ident = tokenToIdent(nameTok);
    const headerFrom = nameTok.from;
    idx++;

    let explicitColumns: string[] | null = null;
    if (tokens[idx]?.kind === "Parens" && kw(tokens[idx + 1]) === "AS") {
      const colTokens = tokens[idx].children ?? [];
      const cols: string[] = [];
      for (const ct of colTokens) {
        if (isName(ct)) cols.push(tokenToIdent(ct).value);
      }
      explicitColumns = cols;
      idx++;
    }

    if (kw(tokens[idx]) !== "AS") break;
    idx++;
    // Optional PostgreSQL MATERIALIZED / NOT MATERIALIZED
    let materialized = false;
    if (kw(tokens[idx]) === "NOT" && kw(tokens[idx + 1]) === "MATERIALIZED") {
      materialized = true;
      idx += 2;
    } else if (kw(tokens[idx]) === "MATERIALIZED") {
      materialized = true;
      idx++;
    }
    const parens = tokens[idx];
    if (parens?.kind !== "Parens") break;
    ctes.push({
      name: ident.value,
      nameToken: nameTok,
      norm: normKey(ident.value, ident.quoted, engine),
      headerFrom,
      parens,
      explicitColumns,
      materialized,
    });
    idx++;
    if (tokens[idx]?.text === ",") {
      idx++;
      continue;
    }
    break;
  }
  if (ctes.length === 0) return null;
  return {
    withToken: tokens[0],
    recursive,
    ctes,
    bodyStartIndex: idx,
  };
}

function collectUsedRelationNames(
  tokens: readonly SqlToken[],
  engine: EngineKind,
  into: Set<string>,
) {
  for (let i = 0; i < tokens.length; i++) {
    const tok = tokens[i];
    if (tok.children) collectUsedRelationNames(tok.children, engine, into);
    const word = kw(tok);
    if (word === "FROM" || word === "JOIN") {
      let j = i + 1;
      if (kw(tokens[j]) === "LATERAL") j++;
      if (tokens[j]?.kind === "Parens") {
        j++;
      } else {
        while (isName(tokens[j])) {
          const id = tokenToIdent(tokens[j]);
          into.add(normKey(id.value, id.quoted, engine));
          if (tokens[j + 1]?.text === ".") {
            j += 2;
          } else {
            j++;
            break;
          }
        }
      }
      if (kw(tokens[j]) === "AS" && isName(tokens[j + 1])) {
        const aliasId = tokenToIdent(tokens[j + 1]);
        into.add(normKey(aliasId.value, aliasId.quoted, engine));
      } else if (isName(tokens[j]) && !CLAUSE_STOP_KEYWORDS.has(tokens[j].text.toUpperCase())) {
        const aliasId = tokenToIdent(tokens[j]);
        into.add(normKey(aliasId.value, aliasId.quoted, engine));
      }
    }
  }
}

function collectDirectRelationAliases(
  tokens: readonly SqlToken[],
  engine: EngineKind,
  into: Set<string>,
) {
  for (let i = 0; i < tokens.length; i++) {
    const word = kw(tokens[i]);
    if (word === "FROM" || word === "JOIN" || tokens[i]?.text === ",") {
      let j = i + 1;
      if (kw(tokens[j]) === "LATERAL") j++;
      if (tokens[j]?.kind === "Parens") {
        j++;
        if (kw(tokens[j]) === "AS" && isName(tokens[j + 1])) {
          const aliasId = tokenToIdent(tokens[j + 1]);
          into.add(normKey(aliasId.value, aliasId.quoted, engine));
        } else if (isName(tokens[j]) && !CLAUSE_STOP_KEYWORDS.has(tokens[j].text.toUpperCase())) {
          const aliasId = tokenToIdent(tokens[j]);
          into.add(normKey(aliasId.value, aliasId.quoted, engine));
        }
      } else if (isName(tokens[j])) {
        const parts: Identifier[] = [];
        while (isName(tokens[j])) {
          parts.push(tokenToIdent(tokens[j]));
          if (tokens[j + 1]?.text === ".") {
            j += 2;
          } else {
            j++;
            break;
          }
        }
        let aliasId: Identifier | null = null;
        if (kw(tokens[j]) === "AS" && isName(tokens[j + 1])) {
          aliasId = tokenToIdent(tokens[j + 1]);
        } else if (isName(tokens[j]) && !CLAUSE_STOP_KEYWORDS.has(tokens[j].text.toUpperCase())) {
          aliasId = tokenToIdent(tokens[j]);
        }
        const boundId = aliasId ?? parts[parts.length - 1];
        if (boundId) {
          into.add(normKey(boundId.value, boundId.quoted, engine));
        }
      }
    }
  }
}

function collectEnclosingOuterAliases(
  tokens: readonly SqlToken[],
  targetFrom: number,
  targetTo: number,
  engine: EngineKind,
  into: Set<string>,
) {
  let bodyTokens = tokens;
  const nestedWith = parseTopLevelWith(tokens, engine);
  if (nestedWith) {
    for (const cte of nestedWith.ctes) {
      if (targetFrom >= cte.parens.from && targetTo <= cte.parens.to) {
        collectEnclosingOuterAliases(cte.parens.children ?? [], targetFrom, targetTo, engine, into);
        return;
      }
    }
    bodyTokens = tokens.slice(nestedWith.bodyStartIndex);
  }
  const hasFromOutsideTarget = bodyTokens.some(
    (t) => kw(t) === "FROM" && (t.to <= targetFrom || t.from >= targetTo),
  );
  if (hasFromOutsideTarget) {
    const nonTargetSlice = bodyTokens.filter((t) => t.to <= targetFrom || t.from >= targetTo);
    collectDirectRelationAliases(nonTargetSlice, engine, into);
  }
  for (const tok of bodyTokens) {
    if (tok.kind === "Parens" && tok.children && tok.from < targetFrom && tok.to > targetTo) {
      collectEnclosingOuterAliases(tok.children, targetFrom, targetTo, engine, into);
    }
  }
}

function assertSubquerySelfContained(
  subTokens: readonly SqlToken[],
  engine: EngineKind,
  outerAliases: ReadonlySet<string>,
  recursiveSelfNames: ReadonlySet<string>,
  inheritedBound: ReadonlySet<string> = new Set(),
) {
  const localBound = new Set(inheritedBound);
  let scanTokens = subTokens;
  const nestedWith = parseTopLevelWith(subTokens, engine);
  if (nestedWith) {
    for (const cte of nestedWith.ctes) {
      if (nestedWith.recursive) {
        localBound.add(cte.norm);
      }
      assertSubquerySelfContained(
        cte.parens.children ?? [],
        engine,
        outerAliases,
        recursiveSelfNames,
        localBound,
      );
      localBound.add(cte.norm);
    }
    scanTokens = subTokens.slice(nestedWith.bodyStartIndex);
  }
  collectDirectRelationAliases(scanTokens, engine, localBound);

  for (let i = 0; i < scanTokens.length; i++) {
    const tok = scanTokens[i];
    const word = kw(tok);
    if ((word === "FROM" || word === "JOIN") && isName(scanTokens[i + 1])) {
      if (scanTokens[i + 2]?.text !== ".") {
        const relId = tokenToIdent(scanTokens[i + 1]);
        const relNorm = normKey(relId.value, relId.quoted, engine);
        if (!inheritedBound.has(relNorm) && recursiveSelfNames.has(relNorm)) {
          throw new Error(
            `Cannot extract a recursive subquery that references enclosing CTE "${relId.value}" before its declaration.`,
          );
        }
      }
    }
    if (
      isName(tok) &&
      scanTokens[i - 1]?.text !== "." &&
      scanTokens[i + 1]?.text === "." &&
      (isName(scanTokens[i + 2]) || scanTokens[i + 2]?.text === "*") &&
      scanTokens[i + 3]?.text !== "."
    ) {
      const prevKw = kw(scanTokens[i - 1]);
      if (prevKw !== "FROM" && prevKw !== "JOIN" && prevKw !== "INTO" && prevKw !== "UPDATE") {
        const qualId = tokenToIdent(tok);
        const qualNorm = normKey(qualId.value, qualId.quoted, engine);
        if (!localBound.has(qualNorm) && outerAliases.has(qualNorm)) {
          throw new Error(
            `Cannot extract a correlated subquery that references outer alias "${qualId.value}" into a top-level CTE.`,
          );
        }
      }
    }
    if (tok.kind === "Parens" && tok.children) {
      assertSubquerySelfContained(
        tok.children,
        engine,
        outerAliases,
        recursiveSelfNames,
        localBound,
      );
    }
  }
}

export interface ExtractCtePlan {
  defaultName: string;
  subquerySql: string;
  replacementKind: "derivedTable" | "expressionSubquery" | "wrapStatement";
  preview(state: EditorState, cteName: string): { changes: ChangeSpec[]; previewSql: string };
}

function indentBlock(sql: string, spaces = 2): string {
  const pad = " ".repeat(spaces);
  return sql
    .trim()
    .split("\n")
    .map((line) => (line.trim().length > 0 ? `${pad}${line}` : line))
    .join("\n");
}

function applyChangesToDoc(
  docText: string,
  changes: Array<{ from: number; to: number; insert: string }>,
): string {
  const sorted = [...changes].sort((a, b) => b.from - a.from);
  let out = docText;
  for (const ch of sorted) {
    out = out.slice(0, ch.from) + ch.insert + out.slice(ch.to);
  }
  return out;
}

export function analyzeExtractCte(
  state: EditorState,
  engine: EngineKind,
  catalogTableNames: readonly string[] = [],
): ExtractCtePlan {
  if (engine === "redis") {
    throw new Error("Extract CTE is only available for SQL editors.");
  }
  const sel = state.selection.main;
  const block = statementTokensAt(state, sel.from);
  if (!block || block.tokens.length === 0) {
    throw new Error("Place the cursor inside a SQL SELECT query or subquery to extract a CTE.");
  }
  if (!sel.empty && sel.to > block.to) {
    throw new Error("Select a subquery within a single SQL statement.");
  }

  const cleanTokens = stripComments(block.tokens);
  if (cleanTokens.length === 0) {
    throw new Error("No SQL tokens found in the current statement.");
  }

  const withHeader = parseTopLevelWith(cleanTokens, engine);
  const existingNames = new Set<string>();
  for (const cte of withHeader?.ctes ?? []) {
    existingNames.add(cte.norm);
  }
  collectUsedRelationNames(cleanTokens, engine, existingNames);
  for (const tbl of catalogTableNames) {
    existingNames.add(normKey(tbl, true, engine));
  }

  // Locate target subquery either from explicit selection or innermost enclosing `(SELECT ...)` Parens
  interface FoundSubquery {
    replaceFrom: number;
    replaceTo: number;
    innerSql: string;
    kind: "derivedTable" | "expressionSubquery" | "wrapStatement";
    enclosingCteIndex: number | null;
    subTokens?: readonly SqlToken[];
  }

  const isSelectParens = (p: SqlToken): boolean => {
    if (p.kind !== "Parens" || !p.children || p.children.length === 0) return false;
    const first = kw(p.children[0]);
    return first === "SELECT" || first === "WITH";
  };

  const findEnclosingSubqueryParens = (
    toks: readonly SqlToken[],
    pos: number,
    enclosingCteIdx: number | null,
    skipRootCteParens: SqlToken | null,
  ): { parens: SqlToken; prevToken: SqlToken | undefined; cteIdx: number | null } | null => {
    for (let i = 0; i < toks.length; i++) {
      const tok = toks[i];
      if (pos < tok.from || pos > tok.to) continue;
      if (tok.children) {
        const deeper = findEnclosingSubqueryParens(
          tok.children,
          pos,
          enclosingCteIdx,
          skipRootCteParens,
        );
        if (deeper) return deeper;
      }
      if (tok !== skipRootCteParens && isSelectParens(tok)) {
        const prev = i > 0 ? toks[i - 1] : undefined;
        return { parens: tok, prevToken: prev, cteIdx: enclosingCteIdx };
      }
    }
    return null;
  };

  let found: FoundSubquery | null = null;

  if (!sel.empty) {
    // Trim whitespace around selection
    let sFrom = sel.from;
    let sTo = sel.to;
    while (sFrom < sTo && /\s/u.test(state.sliceDoc(sFrom, sFrom + 1))) sFrom++;
    while (sTo > sFrom && /[\s;]/u.test(state.sliceDoc(sTo - 1, sTo))) sTo--;

    // Check if selection matches a `(SELECT ...)` Parens or the interior of one
    const enclosing = (() => {
      if (withHeader) {
        for (let ci = 0; ci < withHeader.ctes.length; ci++) {
          const cte = withHeader.ctes[ci];
          if (sFrom >= cte.parens.from && sTo <= cte.parens.to) {
            return findEnclosingSubqueryParens(cte.parens.children ?? [], sFrom, ci, cte.parens);
          }
        }
      }
      const bodyTokens = withHeader ? cleanTokens.slice(withHeader.bodyStartIndex) : cleanTokens;
      return findEnclosingSubqueryParens(bodyTokens, sFrom, null, null);
    })();

    if (
      enclosing &&
      ((sFrom === enclosing.parens.from && sTo === enclosing.parens.to) ||
        (sFrom >= enclosing.parens.from + 1 &&
          sTo <= enclosing.parens.to - 1 &&
          state.sliceDoc(enclosing.parens.from + 1, sFrom).trim() === "" &&
          state.sliceDoc(sTo, enclosing.parens.to - 1).trim() === ""))
    ) {
      const prevKw = kw(enclosing.prevToken);
      const isDerived =
        prevKw === "FROM" ||
        prevKw === "JOIN" ||
        prevKw === "LATERAL" ||
        enclosing.prevToken?.text === ",";
      const inner = state.sliceDoc(enclosing.parens.from + 1, enclosing.parens.to - 1).trim();
      found = {
        replaceFrom: enclosing.parens.from,
        replaceTo: enclosing.parens.to,
        innerSql: inner,
        kind: isDerived ? "derivedTable" : "expressionSubquery",
        enclosingCteIndex: enclosing.cteIdx,
        subTokens: enclosing.parens.children,
      };
    } else {
      const selectedSql = state.sliceDoc(sFrom, sTo).trim();
      if (!/^SELECT\b/iu.test(selectedSql)) {
        throw new Error(
          "Select a complete `(SELECT ...)` subquery or top-level `SELECT ...` query to extract into a CTE.",
        );
      }
      const stmtFirstFrom = cleanTokens[0].from;
      const stmtLastTo =
        cleanTokens[cleanTokens.length - 1].text === ";"
          ? (cleanTokens[cleanTokens.length - 2]?.to ?? sTo)
          : cleanTokens[cleanTokens.length - 1].to;
      if (!withHeader && sFrom === stmtFirstFrom && sTo === stmtLastTo) {
        found = {
          replaceFrom: sFrom,
          replaceTo: sTo,
          innerSql: selectedSql,
          kind: "wrapStatement",
          enclosingCteIndex: null,
        };
      } else {
        found = {
          replaceFrom: sFrom,
          replaceTo: sTo,
          innerSql: selectedSql,
          kind: "expressionSubquery",
          enclosingCteIndex: null,
        };
      }
    }
  } else {
    // Cursor-based detection: find innermost enclosing `(SELECT ...)` subquery
    let enclosing: {
      parens: SqlToken;
      prevToken: SqlToken | undefined;
      cteIdx: number | null;
    } | null = null;
    if (withHeader) {
      for (let ci = 0; ci < withHeader.ctes.length; ci++) {
        const cte = withHeader.ctes[ci];
        if (sel.head >= cte.parens.from && sel.head <= cte.parens.to) {
          enclosing = findEnclosingSubqueryParens(
            cte.parens.children ?? [],
            sel.head,
            ci,
            cte.parens,
          );
          break;
        }
      }
    }
    if (!enclosing) {
      const bodyTokens = withHeader ? cleanTokens.slice(withHeader.bodyStartIndex) : cleanTokens;
      enclosing = findEnclosingSubqueryParens(bodyTokens, sel.head, null, null);
    }
    if (!enclosing) {
      throw new Error(
        "Place the cursor inside a parenthesized `(SELECT ...)` subquery, or select a `SELECT` query to extract.",
      );
    }
    const prevKw = kw(enclosing.prevToken);
    const isDerived =
      prevKw === "FROM" ||
      prevKw === "JOIN" ||
      prevKw === "LATERAL" ||
      enclosing.prevToken?.text === ",";
    const inner = state.sliceDoc(enclosing.parens.from + 1, enclosing.parens.to - 1).trim();
    found = {
      replaceFrom: enclosing.parens.from,
      replaceTo: enclosing.parens.to,
      innerSql: inner,
      kind: isDerived ? "derivedTable" : "expressionSubquery",
      enclosingCteIndex: enclosing.cteIdx,
      subTokens: enclosing.parens.children,
    };
  }

  if (found.kind !== "wrapStatement") {
    const outerAliases = new Set<string>();
    collectEnclosingOuterAliases(
      cleanTokens,
      found.replaceFrom,
      found.replaceTo,
      engine,
      outerAliases,
    );
    const recursiveSelfNames = new Set<string>();
    if (withHeader?.recursive && found.enclosingCteIndex !== null) {
      for (let i = found.enclosingCteIndex; i < withHeader.ctes.length; i++) {
        recursiveSelfNames.add(withHeader.ctes[i].norm);
      }
    }
    const checkTokens =
      found.subTokens ??
      cleanTokens.filter((t) => t.from >= found.replaceFrom && t.to <= found.replaceTo);
    if (outerAliases.size > 0 || recursiveSelfNames.size > 0) {
      assertSubquerySelfContained(checkTokens, engine, outerAliases, recursiveSelfNames);
    }
  }

  let candidateIdx = 1;
  let defaultName = engine === "snowflake" ? "EXTRACTED_CTE" : "extracted_cte";
  while (existingNames.has(normKey(defaultName, false, engine))) {
    candidateIdx++;
    defaultName =
      engine === "snowflake" ? `EXTRACTED_CTE_${candidateIdx}` : `extracted_cte_${candidateIdx}`;
  }

  const stmtStart = cleanTokens[0].from;
  const stmtEnd = block.to;

  return {
    defaultName,
    subquerySql: found.innerSql,
    replacementKind: found.kind,
    preview(currentState: EditorState, rawName: string) {
      if (currentState.doc !== state.doc) {
        throw new Error("The SQL buffer changed. Start Extract CTE again.");
      }
      const trimmed = rawName.trim();
      if (!trimmed) {
        throw new Error("Enter a CTE name.");
      }
      const candidateNorm = normKey(trimmed, true, engine);
      if (existingNames.has(candidateNorm)) {
        throw new Error(
          `"${trimmed}" collides with an existing CTE, table, or alias in this statement.`,
        );
      }
      const quotedCte = quoteIdentifier(trimmed, engine);
      const formattedBody = indentBlock(found.innerSql, 2);

      const rawChanges: Array<{ from: number; to: number; insert: string }> = [];
      if (found.kind === "wrapStatement") {
        rawChanges.push({
          from: found.replaceFrom,
          to: found.replaceTo,
          insert: `WITH ${quotedCte} AS (\n${formattedBody}\n)\nSELECT * FROM ${quotedCte}`,
        });
      } else {
        const callSiteReplacement =
          found.kind === "derivedTable" ? quotedCte : `(SELECT * FROM ${quotedCte})`;
        if (!withHeader) {
          rawChanges.push({
            from: stmtStart,
            to: stmtStart,
            insert: `WITH ${quotedCte} AS (\n${formattedBody}\n)\n`,
          });
        } else if (found.enclosingCteIndex !== null) {
          const targetCte = withHeader.ctes[found.enclosingCteIndex];
          rawChanges.push({
            from: targetCte.headerFrom,
            to: targetCte.headerFrom,
            insert: `${quotedCte} AS (\n${formattedBody}\n),\n`,
          });
        } else {
          const lastCte = withHeader.ctes[withHeader.ctes.length - 1];
          rawChanges.push({
            from: lastCte.parens.to,
            to: lastCte.parens.to,
            insert: `,\n${quotedCte} AS (\n${formattedBody}\n)`,
          });
        }
        rawChanges.push({
          from: found.replaceFrom,
          to: found.replaceTo,
          insert: callSiteReplacement,
        });
      }

      const fullStmt = state.sliceDoc(stmtStart, stmtEnd);
      const relativeChanges = rawChanges.map((c) => ({
        from: c.from - stmtStart,
        to: c.to - stmtStart,
        insert: c.insert,
      }));
      const previewSql = applyChangesToDoc(fullStmt, relativeChanges);
      return {
        changes: rawChanges,
        previewSql,
      };
    },
  };
}

export interface InlineCtePlan {
  cteName: string;
  referenceCount: number;
  changes: Array<{ from: number; to: number; insert: string }>;
  previewSql: string;
}

const FROM_END_KEYWORDS = new Set([
  "WHERE",
  "GROUP",
  "HAVING",
  "WINDOW",
  "QUALIFY",
  "ORDER",
  "LIMIT",
  "OFFSET",
  "FETCH",
  "FOR",
  "UNION",
  "INTERSECT",
  "EXCEPT",
  "MINUS",
  "SET",
  "VALUES",
  "RETURNING",
  "SELECT",
]);

interface CteCallSite {
  from: number;
  to: number;
  hasExplicitAlias: boolean;
}

function collectCteCallSitesInTokens(
  tokens: readonly SqlToken[],
  targetNorm: string,
  engine: EngineKind,
  into: CteCallSite[],
) {
  let startIdx = 0;
  const innerWith = parseTopLevelWith(tokens, engine);
  if (innerWith) {
    let shadowed = false;
    for (const innerCte of innerWith.ctes) {
      if (!shadowed) {
        collectCteCallSitesInTokens(innerCte.parens.children ?? [], targetNorm, engine, into);
      }
      if (innerCte.norm === targetNorm) {
        shadowed = true;
      }
    }
    if (shadowed) return;
    startIdx = innerWith.bodyStartIndex;
  }

  let inFromClause = false;
  for (let i = startIdx; i < tokens.length; i++) {
    const tok = tokens[i];
    const word = kw(tok);
    if (word === "FROM") {
      inFromClause = true;
    } else if (FROM_END_KEYWORDS.has(word) || tok.text === ";") {
      inFromClause = false;
    }

    if (tok.children) {
      collectCteCallSitesInTokens(tok.children, targetNorm, engine, into);
    }

    if (word === "FROM" || word === "JOIN" || (inFromClause && tok.text === ",")) {
      let j = i + 1;
      if (kw(tokens[j]) === "LATERAL") j++;
      const relTok = tokens[j];
      if (isName(relTok) && tokens[j - 1]?.text !== "." && tokens[j + 1]?.text !== ".") {
        const relIdent = tokenToIdent(relTok);
        if (normKey(relIdent.value, relIdent.quoted, engine) === targetNorm) {
          const nextTok = tokens[j + 1];
          const hasExplicitAlias =
            (kw(nextTok) === "AS" && isName(tokens[j + 2])) ||
            (isName(nextTok) && !CLAUSE_STOP_KEYWORDS.has(nextTok.text.toUpperCase()));
          into.push({
            from: relTok.from,
            to: relTok.to,
            hasExplicitAlias,
          });
        }
      }
    }
  }
}

function findIdentifierTokenAtPos(tokens: readonly SqlToken[], pos: number): SqlToken | null {
  for (let i = 0; i < tokens.length; i++) {
    const tok = tokens[i];
    if (tok.children && pos >= tok.from && pos <= tok.to) {
      const nested = findIdentifierTokenAtPos(tok.children, pos);
      if (nested) return nested;
    }
    if (
      isName(tok) &&
      pos >= tok.from &&
      pos <= tok.to &&
      tokens[i - 1]?.text !== "." &&
      tokens[i + 1]?.text !== "."
    ) {
      return tok;
    }
  }
  return null;
}

export function analyzeInlineCte(state: EditorState, engine: EngineKind): InlineCtePlan {
  if (engine === "redis") {
    throw new Error("Inline CTE is only available for SQL editors.");
  }
  const sel = state.selection.main;
  const block = statementTokensAt(state, sel.head);
  if (!block) {
    throw new Error("Place the cursor inside a WITH SQL statement to inline a CTE.");
  }
  const cleanTokens = stripComments(block.tokens);
  if (cleanTokens.length === 0) {
    throw new Error("Place the cursor inside a WITH SQL statement to inline a CTE.");
  }

  const withHeader = parseTopLevelWith(cleanTokens, engine);
  if (!withHeader) {
    throw new Error("This statement has no top-level WITH Common Table Expression to inline.");
  }

  let targetIdx = -1;
  for (let i = 0; i < withHeader.ctes.length; i++) {
    const cte = withHeader.ctes[i];
    if (sel.head >= cte.headerFrom && sel.head <= cte.parens.to) {
      targetIdx = i;
      break;
    }
  }
  if (
    targetIdx < 0 &&
    withHeader.ctes.length === 1 &&
    sel.head >= withHeader.withToken.from &&
    sel.head <= withHeader.withToken.to
  ) {
    targetIdx = 0;
  }
  if (targetIdx < 0) {
    const identTok = findIdentifierTokenAtPos(cleanTokens, sel.head);
    if (identTok) {
      const ident = tokenToIdent(identTok);
      const norm = normKey(ident.value, ident.quoted, engine);
      for (let i = withHeader.ctes.length - 1; i >= 0; i--) {
        if (withHeader.ctes[i].norm === norm && identTok.from > withHeader.ctes[i].parens.to) {
          targetIdx = i;
          break;
        }
      }
    }
  }
  if (targetIdx < 0) {
    throw new Error("Place the cursor on a CTE definition or a FROM/JOIN reference to that CTE.");
  }

  const targetCte = withHeader.ctes[targetIdx];
  if (targetCte.explicitColumns !== null) {
    throw new Error(
      `CTE "${targetCte.name}" declares an explicit column rename list and cannot be inlined automatically.`,
    );
  }
  if (targetCte.materialized) {
    throw new Error(
      `CTE "${targetCte.name}" uses MATERIALIZED / NOT MATERIALIZED; remove the materialization hint before inlining.`,
    );
  }

  const selfRefs: CteCallSite[] = [];
  collectCteCallSitesInTokens(targetCte.parens.children ?? [], targetCte.norm, engine, selfRefs);
  if (selfRefs.length > 0) {
    throw new Error(`Recursive CTE "${targetCte.name}" cannot be inlined.`);
  }

  const callSites: CteCallSite[] = [];
  let shadowedLater = false;
  for (let i = targetIdx + 1; i < withHeader.ctes.length; i++) {
    const laterCte = withHeader.ctes[i];
    if (!shadowedLater) {
      collectCteCallSitesInTokens(
        laterCte.parens.children ?? [],
        targetCte.norm,
        engine,
        callSites,
      );
    }
    if (laterCte.norm === targetCte.norm) {
      shadowedLater = true;
    }
  }
  if (!shadowedLater) {
    const bodyTokens = cleanTokens.slice(withHeader.bodyStartIndex);
    collectCteCallSitesInTokens(bodyTokens, targetCte.norm, engine, callSites);
  }

  if (callSites.length === 0) {
    throw new Error(`CTE "${targetCte.name}" has no FROM or JOIN references to inline.`);
  }

  const innerSql = state.sliceDoc(targetCte.parens.from + 1, targetCte.parens.to - 1).trim();
  const cteNameSql = state.sliceDoc(targetCte.nameToken.from, targetCte.nameToken.to);
  const subqueryText =
    !innerSql.includes("\n") && innerSql.length <= 72
      ? `(${innerSql})`
      : `(\n${indentBlock(innerSql, 2)}\n)`;

  let removeFrom: number;
  let removeTo: number;
  if (withHeader.ctes.length === 1) {
    const bodyFirst = cleanTokens[withHeader.bodyStartIndex];
    removeFrom = withHeader.withToken.from;
    removeTo = bodyFirst ? bodyFirst.from : targetCte.parens.to;
  } else if (targetIdx < withHeader.ctes.length - 1) {
    removeFrom = targetCte.headerFrom;
    removeTo = withHeader.ctes[targetIdx + 1].headerFrom;
  } else {
    removeFrom = withHeader.ctes[targetIdx - 1].parens.to;
    removeTo = targetCte.parens.to;
  }

  const rawChanges: Array<{ from: number; to: number; insert: string }> = [
    { from: removeFrom, to: removeTo, insert: "" },
    ...callSites.map((site) => ({
      from: site.from,
      to: site.to,
      insert: site.hasExplicitAlias ? subqueryText : `${subqueryText} AS ${cteNameSql}`,
    })),
  ];

  const stmtStart = cleanTokens[0].from;
  const stmtEnd = block.to;
  const fullStmt = state.sliceDoc(stmtStart, stmtEnd);
  const relativeChanges = rawChanges.map((c) => ({
    from: c.from - stmtStart,
    to: c.to - stmtStart,
    insert: c.insert,
  }));
  const previewSql = applyChangesToDoc(fullStmt, relativeChanges);

  return {
    cteName: targetCte.name,
    referenceCount: callSites.length,
    changes: rawChanges,
    previewSql,
  };
}

export interface WildcardExpansionPlan {
  tablesToFetch: TableRef[];
  wildcardText: string;
  finish(
    currentState: EditorState,
    schemas: readonly TableSchema[],
  ): {
    from: number;
    to: number;
    insert: string;
    columnCount: number;
    previewSql: string;
  };
}

interface SelectRelationBinding {
  qualifierSql: string | null;
  qualifierNorm: string;
  source:
    | { kind: "table"; table: TableRef; tableNorm: string }
    | { kind: "cte"; cteName: string; columns: string[] | null }
    | { kind: "derived"; alias: string | null; columns: string[] | null };
}

function inferCteProjectionColumns(cte: CteEntry): string[] | null {
  if (cte.explicitColumns && cte.explicitColumns.length > 0) {
    return cte.explicitColumns;
  }
  const inner = cte.parens.children ?? [];
  if (kw(inner[0]) !== "SELECT") return null;
  let idx = 1;
  if (kw(inner[idx]) === "DISTINCT" || kw(inner[idx]) === "ALL") idx++;
  const fromIdx = inner.findIndex((t, i) => i >= idx && kw(t) === "FROM");
  const endIdx = fromIdx >= 0 ? fromIdx : inner.length;
  if (endIdx <= idx) return null;

  const items: SqlToken[][] = [];
  let current: SqlToken[] = [];
  for (let i = idx; i < endIdx; i++) {
    if (inner[i].text === ",") {
      items.push(current);
      current = [];
    } else {
      current.push(inner[i]);
    }
  }
  if (current.length > 0) items.push(current);

  const cols: string[] = [];
  for (const item of items) {
    if (item.length === 0) return null;
    if (item.some((t) => t.text === "*")) return null;
    if (item.length >= 2 && kw(item[item.length - 2]) === "AS" && isName(item[item.length - 1])) {
      cols.push(tokenToIdent(item[item.length - 1]).value);
    } else if (isName(item[item.length - 1])) {
      cols.push(tokenToIdent(item[item.length - 1]).value);
    } else {
      return null;
    }
  }
  return cols.length > 0 ? cols : null;
}

export function prepareExpandWildcard(
  state: EditorState,
  position: number,
  context: {
    engine: EngineKind;
    tables: TableInfo[];
    database?: string;
    defaultSchema?: string;
    searchPath?: string[];
  },
): WildcardExpansionPlan {
  const { engine } = context;
  if (engine === "redis") {
    throw new Error("Wildcard expansion is only available for SQL editors.");
  }
  const block = statementTokensAt(state, position);
  if (!block || block.tokens.length === 0) {
    throw new Error("Place the cursor on `*` or `alias.*` inside a SQL SELECT query.");
  }
  const cleanTokens = stripComments(block.tokens);
  const withHeader = parseTopLevelWith(cleanTokens, engine);
  const cteMap = new Map<string, CteEntry>();
  for (const cte of withHeader?.ctes ?? []) {
    cteMap.set(cte.norm, cte);
  }

  // Find the deepest token list (top-level query or subquery `Parens`) containing `position` that starts a SELECT
  const findSelectTokenList = (
    toks: readonly SqlToken[],
    visibleCtes: ReadonlyMap<string, CteEntry>,
  ): { tokens: readonly SqlToken[]; ctes: ReadonlyMap<string, CteEntry> } | null => {
    for (const tok of toks) {
      if (tok.kind === "Parens" && tok.children && position >= tok.from && position <= tok.to) {
        const nestedWith = parseTopLevelWith(tok.children, engine);
        const mergedCtes = new Map(visibleCtes);
        if (nestedWith) {
          for (const c of nestedWith.ctes) mergedCtes.set(c.norm, c);
        }
        const subTokens = nestedWith ? tok.children.slice(nestedWith.bodyStartIndex) : tok.children;
        const deeper = findSelectTokenList(subTokens, mergedCtes);
        if (deeper) return deeper;
        if (kw(subTokens[0]) === "SELECT") {
          return { tokens: subTokens, ctes: mergedCtes };
        }
      }
    }
    return null;
  };

  let activeCtes = new Map<string, CteEntry>();
  let selectTokens: readonly SqlToken[] | null = null;

  if (withHeader) {
    for (const cte of withHeader.ctes) {
      if (position >= cte.parens.from && position <= cte.parens.to) {
        const insideCte = findSelectTokenList([cte.parens], new Map(activeCtes));
        if (insideCte) {
          selectTokens = insideCte.tokens;
          activeCtes = new Map(insideCte.ctes);
        }
        break;
      }
      activeCtes.set(cte.norm, cte);
    }
  }
  if (!selectTokens) {
    const bodyTokens = withHeader ? cleanTokens.slice(withHeader.bodyStartIndex) : cleanTokens;
    const nested = findSelectTokenList(bodyTokens, cteMap);
    if (nested) {
      selectTokens = nested.tokens;
      activeCtes = new Map(nested.ctes);
    } else if (kw(bodyTokens[0]) === "SELECT") {
      selectTokens = bodyTokens;
      activeCtes = cteMap;
    }
  }

  if (!selectTokens || kw(selectTokens[0]) !== "SELECT") {
    throw new Error("Place the cursor inside a `SELECT` projection containing `*` or `alias.*`.");
  }

  let projStart = 1;
  if (kw(selectTokens[projStart]) === "DISTINCT" || kw(selectTokens[projStart]) === "ALL") {
    projStart++;
  }
  const fromIdx = selectTokens.findIndex((t, i) => i >= projStart && kw(t) === "FROM");
  if (fromIdx < 0) {
    throw new Error("Cannot expand `*` without a `FROM` clause.");
  }

  // Locate wildcard items in `[projStart, fromIdx)`
  interface WildcardCandidate {
    from: number;
    to: number;
    qualifierIdent: Identifier | null;
    rawText: string;
  }
  const wildcards: WildcardCandidate[] = [];
  let itemStart = projStart;
  for (let i = projStart; i <= fromIdx; i++) {
    if (i === fromIdx || selectTokens[i].text === ",") {
      const slice = selectTokens.slice(itemStart, i);
      if (slice.length === 1 && slice[0].text === "*") {
        wildcards.push({
          from: slice[0].from,
          to: slice[0].to,
          qualifierIdent: null,
          rawText: "*",
        });
      } else if (
        slice.length === 3 &&
        isName(slice[0]) &&
        slice[1].text === "." &&
        slice[2].text === "*"
      ) {
        wildcards.push({
          from: slice[0].from,
          to: slice[2].to,
          qualifierIdent: tokenToIdent(slice[0]),
          rawText: state.sliceDoc(slice[0].from, slice[2].to),
        });
      }
      itemStart = i + 1;
    }
  }

  if (wildcards.length === 0) {
    throw new Error("No `*` or `alias.*` wildcard found in the current SELECT projection.");
  }

  const targetWildcard =
    wildcards.find((w) => position >= w.from && position <= w.to) ??
    (wildcards.length === 1 ? wildcards[0] : null);
  if (!targetWildcard) {
    throw new Error(
      "Multiple wildcards exist in this SELECT list; place the cursor on the specific `*` to expand.",
    );
  }

  // Parse FROM / JOIN relations for this SELECT block
  const lookup = tableLookup(
    context.tables,
    engine,
    context.database,
    context.defaultSchema,
    context.searchPath,
  );
  const relations: SelectRelationBinding[] = [];
  let idx = fromIdx;
  while (idx < selectTokens.length) {
    const word = kw(selectTokens[idx]);
    if (
      idx > fromIdx &&
      CLAUSE_STOP_KEYWORDS.has(word) &&
      word !== "JOIN" &&
      word !== "INNER" &&
      word !== "LEFT" &&
      word !== "RIGHT" &&
      word !== "FULL" &&
      word !== "CROSS" &&
      word !== "NATURAL" &&
      word !== "ON" &&
      word !== "USING"
    ) {
      break;
    }
    if (word === "FROM" || word === "JOIN" || selectTokens[idx].text === ",") {
      let j = idx + 1;
      if (kw(selectTokens[j]) === "LATERAL") j++;
      if (selectTokens[j]?.kind === "Parens") {
        const subParens = selectTokens[j];
        j++;
        let aliasTok: SqlToken | undefined;
        if (kw(selectTokens[j]) === "AS" && isName(selectTokens[j + 1])) {
          aliasTok = selectTokens[j + 1];
          j += 2;
        } else if (
          isName(selectTokens[j]) &&
          !CLAUSE_STOP_KEYWORDS.has(selectTokens[j].text.toUpperCase())
        ) {
          aliasTok = selectTokens[j];
          j++;
        }
        let explicitCols: string[] | null = null;
        if (selectTokens[j]?.kind === "Parens") {
          explicitCols = (selectTokens[j].children ?? [])
            .filter((ct) => isName(ct))
            .map((ct) => tokenToIdent(ct).value);
          j++;
        } else {
          const fakeCte: CteEntry = {
            name: aliasTok?.text ?? "sub",
            nameToken: aliasTok ?? subParens,
            norm: "",
            headerFrom: subParens.from,
            parens: subParens,
            explicitColumns: null,
            materialized: false,
          };
          explicitCols = inferCteProjectionColumns(fakeCte);
        }
        const aliasId = aliasTok ? tokenToIdent(aliasTok) : null;
        relations.push({
          qualifierSql: aliasTok ? quoteIdentifier(aliasId!.value, engine) : null,
          qualifierNorm: aliasId ? normKey(aliasId.value, aliasId.quoted, engine) : "",
          source: {
            kind: "derived",
            alias: aliasId?.value ?? null,
            columns: explicitCols,
          },
        });
        idx = j;
        continue;
      }

      if (isName(selectTokens[j])) {
        const parts: Identifier[] = [];
        const rawParts: SqlToken[] = [];
        while (isName(selectTokens[j])) {
          parts.push(tokenToIdent(selectTokens[j]));
          rawParts.push(selectTokens[j]);
          if (selectTokens[j + 1]?.text === ".") {
            j += 2;
          } else {
            j++;
            break;
          }
        }
        let aliasTok: SqlToken | undefined;
        if (kw(selectTokens[j]) === "AS" && isName(selectTokens[j + 1])) {
          aliasTok = selectTokens[j + 1];
          j += 2;
        } else if (
          isName(selectTokens[j]) &&
          !CLAUSE_STOP_KEYWORDS.has(selectTokens[j].text.toUpperCase())
        ) {
          aliasTok = selectTokens[j];
          j++;
        }
        const aliasId = aliasTok ? tokenToIdent(aliasTok) : null;
        const lastPart = parts[parts.length - 1];
        const qualId = aliasId ?? lastPart;
        const qualNorm = normKey(qualId.value, qualId.quoted, engine);
        const qualSql = aliasId
          ? quoteIdentifier(aliasId.value, engine)
          : quoteIdentifier(lastPart.value, engine);

        if (
          parts.length === 1 &&
          activeCtes.has(normKey(parts[0].value, parts[0].quoted, engine))
        ) {
          const cte = activeCtes.get(normKey(parts[0].value, parts[0].quoted, engine))!;
          relations.push({
            qualifierSql: qualSql,
            qualifierNorm: qualNorm,
            source: {
              kind: "cte",
              cteName: cte.name,
              columns: inferCteProjectionColumns(cte),
            },
          });
        } else {
          const resolved = lookup.resolve(parts);
          if (!resolved) {
            throw new Error(
              `Cannot resolve relation "${rawParts.map((p) => p.text).join(".")}" in the active catalog. Connect or refresh completion metadata first.`,
            );
          }
          relations.push({
            qualifierSql: qualSql,
            qualifierNorm: qualNorm,
            source: {
              kind: "table",
              table: resolved,
              tableNorm: normKey(resolved.name, true, engine),
            },
          });
        }
        idx = j;
        continue;
      }
    }
    idx++;
  }

  if (relations.length === 0) {
    throw new Error("Could not resolve any relations in the `FROM` clause.");
  }

  const targetRelations = targetWildcard.qualifierIdent
    ? relations.filter((r) => {
        const want = normKey(
          targetWildcard.qualifierIdent!.value,
          targetWildcard.qualifierIdent!.quoted,
          engine,
        );
        return r.qualifierNorm === want;
      })
    : relations;

  if (targetRelations.length === 0) {
    throw new Error(
      `Qualifier "${targetWildcard.qualifierIdent?.value}" does not match any table or alias in the FROM clause.`,
    );
  }

  const tablesToFetch: TableRef[] = [];
  const seenFetch = new Set<string>();
  for (const rel of targetRelations) {
    if (rel.source.kind === "table") {
      const key = `${rel.source.table.schema}\0${rel.source.table.name}`;
      if (!seenFetch.has(key)) {
        seenFetch.add(key);
        tablesToFetch.push(rel.source.table);
      }
    }
  }

  const stmtStart = cleanTokens[0].from;
  const stmtEnd = block.to;

  return {
    tablesToFetch,
    wildcardText: targetWildcard.rawText,
    finish(currentState: EditorState, schemas: readonly TableSchema[]) {
      if (currentState.doc !== state.doc) {
        throw new Error("The SQL buffer changed before wildcard expansion completed.");
      }
      const schemaMap = new Map<string, TableSchema>();
      for (const s of schemas) {
        schemaMap.set(`${s.table.schema}\0${s.table.name}`, s);
      }
      const useQualifier = Boolean(targetWildcard.qualifierIdent) || relations.length > 1;
      const expandedCols: string[] = [];

      for (const rel of targetRelations) {
        let colNames: string[] | null = null;
        if (rel.source.kind === "table") {
          const s = schemaMap.get(`${rel.source.table.schema}\0${rel.source.table.name}`);
          if (!s || s.columns.length === 0) {
            throw new Error(
              `No columns returned for ${rel.source.table.schema}.${rel.source.table.name}.`,
            );
          }
          colNames = s.columns.map((c) => c.name);
        } else if (rel.source.kind === "cte") {
          if (!rel.source.columns || rel.source.columns.length === 0) {
            throw new Error(
              `CTE "${rel.source.cteName}" does not have statically resolvable output columns (expand inner wildcards or add explicit CTE column names first).`,
            );
          }
          colNames = rel.source.columns;
        } else {
          if (!rel.source.columns || rel.source.columns.length === 0) {
            throw new Error(
              `Derived subquery${rel.source.alias ? ` "${rel.source.alias}"` : ""} does not have statically resolvable output columns.`,
            );
          }
          colNames = rel.source.columns;
        }

        for (const colName of colNames) {
          const quotedCol = quoteIdentifier(colName, engine);
          if (useQualifier && rel.qualifierSql) {
            expandedCols.push(`${rel.qualifierSql}.${quotedCol}`);
          } else {
            expandedCols.push(quotedCol);
          }
        }
      }

      const insert = expandedCols.join(", ");
      const fullStmt = state.sliceDoc(stmtStart, stmtEnd);
      const previewSql = applyChangesToDoc(fullStmt, [
        {
          from: targetWildcard.from - stmtStart,
          to: targetWildcard.to - stmtStart,
          insert,
        },
      ]);
      return {
        from: targetWildcard.from,
        to: targetWildcard.to,
        insert,
        columnCount: expandedCols.length,
        previewSql,
      };
    },
  };
}

export function collectWorkspaceSqlDocuments(
  tabs: ReadonlyArray<{
    id: string;
    title: string;
    kind: string;
    connectionId: string | null;
    savedQueryId?: string | null;
    sql: string;
  }>,
  connectionId: string | null,
  activeTabOverride?: { id: string; sql: string },
): SqlUsageDocument[] {
  const docs: SqlUsageDocument[] = [];
  for (const tab of tabs) {
    if (tab.kind !== "query") continue;
    if (connectionId && tab.connectionId && tab.connectionId !== connectionId) continue;
    const sql =
      activeTabOverride && tab.id === activeTabOverride.id ? activeTabOverride.sql : tab.sql;
    if (!sql.trim()) continue;
    docs.push({
      id: tab.id,
      title: tab.title,
      sourceKind: "openTab",
      savedQueryId: tab.savedQueryId ?? null,
      sql,
    });
  }
  return docs;
}
