import { snippetCompletion, type Completion } from "@codemirror/autocomplete";
import type { EditorState } from "@codemirror/state";
import type { EngineKind, SqlSnippet, SqlSnippetPlace } from "../../ipc/generated";
import { statementTokensAt, type CompletionTarget, type SqlToken } from "./sql-context";
import type { SqlTemplateApplier } from "./sql-template-context";

export interface EffectiveSqlSnippet {
  trigger: string;
  title: string;
  body: string;
  engine: EngineKind | null;
  /** Omitted on a built-in. A custom snippet uses this only when its trigger is not built in. */
  place?: SqlSnippetPlace;
  isCustom: boolean;
}

const READ_TEMPLATES: readonly Omit<EffectiveSqlSnippet, "isCustom">[] = [
  {
    trigger: "ctxsel",
    title: "Select rows from current schema",
    body: "SELECT ${1:*}\nFROM {{schema}}.${2:table_name};${0}",
    engine: null,
  },
  {
    trigger: "wrap",
    title: "Wrap selected SQL as a subquery",
    body: "SELECT ${1:*}\nFROM (\n{{selection}}\n) ${2:selected_rows};${0}",
    engine: null,
  },
  {
    trigger: "sel",
    title: "Select rows from table",
    body: "SELECT ${1:*}\nFROM ${2:table_name}\nWHERE ${3:1 = 1};",
    engine: null,
  },
  {
    trigger: "cnt",
    title: "Count rows in table",
    body: "SELECT COUNT(*) AS ${1:total}\nFROM ${2:table_name};",
    engine: null,
  },
  {
    trigger: "join",
    title: "Inner join clause",
    body: "JOIN ${1:partner_table} ${2:p} ON ${3:p.id = t.partner_id}",
    engine: null,
  },
  {
    trigger: "ljoin",
    title: "Left outer join clause",
    body: "LEFT JOIN ${1:partner_table} ${2:p} ON ${3:p.id = t.partner_id}",
    engine: null,
  },
  {
    trigger: "cte",
    title: "Common table expression (WITH)",
    body: "WITH ${1:cte_name} AS (\n\tSELECT ${2:*}\n\tFROM ${3:table_name}\n)\nSELECT ${0:*}\nFROM ${1:cte_name};",
    engine: null,
  },
  {
    trigger: "lim",
    title: "Limit and offset clause",
    body: "LIMIT ${1:100} OFFSET ${2:0}",
    engine: null,
  },
];

const WRITE_TEMPLATES: readonly Omit<EffectiveSqlSnippet, "isCustom">[] = [
  {
    trigger: "ins",
    title: "Insert row",
    body: "INSERT INTO ${1:table_name} (${2:column_name})\nVALUES (${3:value});",
    engine: null,
  },
  {
    trigger: "upd",
    title: "Update rows with WHERE",
    body: "UPDATE ${1:table_name}\nSET ${2:column_name} = ${3:value}\nWHERE ${4:id = 1};",
    engine: null,
  },
  {
    trigger: "del",
    title: "Delete rows with WHERE",
    body: "DELETE FROM ${1:table_name}\nWHERE ${2:id = 1};",
    engine: null,
  },
];

export function builtInSqlSnippets(engine: EngineKind): EffectiveSqlSnippet[] {
  if (engine === "redis") return [];
  const items: EffectiveSqlSnippet[] = READ_TEMPLATES.map((t) => ({ ...t, isCustom: false }));
  if (engine !== "snowflake") {
    for (const writeTemplate of WRITE_TEMPLATES) {
      items.push({ ...writeTemplate, isCustom: false });
    }
    items.push({
      trigger: "txn",
      title: "Transaction block",
      body: engine === "mysql" ? "START TRANSACTION;\n${0}\nCOMMIT;" : "BEGIN;\n${0}\nCOMMIT;",
      engine,
      isCustom: false,
    });
  }
  return items;
}

/**
 * Merges dialect built-in live templates with user-defined snippets.
 * User snippets matching `engine` override global (`engine === null`) user
 * snippets, and both override built-in templates with the same trigger.
 */
export function effectiveSqlSnippets(
  engine: EngineKind,
  userSnippets: readonly SqlSnippet[] = [],
): EffectiveSqlSnippet[] {
  if (engine === "redis") return [];
  const byTrigger = new Map<string, EffectiveSqlSnippet>();
  for (const builtin of builtInSqlSnippets(engine)) {
    byTrigger.set(builtin.trigger.toLowerCase(), builtin);
  }
  for (const custom of userSnippets) {
    if (custom.engine !== null) continue;
    byTrigger.set(custom.trigger.toLowerCase(), {
      trigger: custom.trigger,
      title: custom.title,
      body: custom.body,
      engine: null,
      place: custom.place,
      isCustom: true,
    });
  }
  for (const custom of userSnippets) {
    if (custom.engine !== engine) continue;
    byTrigger.set(custom.trigger.toLowerCase(), {
      trigger: custom.trigger,
      title: custom.title,
      body: custom.body,
      engine: custom.engine,
      place: custom.place,
      isCustom: true,
    });
  }
  return [...byTrigger.values()].sort((a, b) =>
    a.trigger.toLowerCase().localeCompare(b.trigger.toLowerCase()),
  );
}

export interface SqlTemplateSite {
  /** Nothing but comments precedes the caret in this statement. */
  atStatementStart: boolean;
  /** The caret is still in the top-level FROM or JOIN list of a query. */
  inFromClause: boolean;
  /** A top-level FROM has already been seen, including later clauses. */
  afterFrom: boolean;
}

const STATEMENT_TEMPLATES = new Set(["ctxsel", "sel", "cnt", "cte", "ins", "upd", "del", "txn"]);
const CLAUSE_END = new Set([
  "WHERE",
  "GROUP",
  "ORDER",
  "HAVING",
  "LIMIT",
  "OFFSET",
  "SET",
  "VALUES",
  "RETURNING",
  "WINDOW",
  "QUALIFY",
  "FOR",
  "FETCH",
]);

const NESTED_QUERY_KEYWORD = new Set(["FROM", "JOIN", "AS", "IN", "EXISTS", "SELECT"]);
const NESTED_QUERY_MARK = new Set(["=", "<", ">", "<=", ">=", "<>", "!=", ","]);
const STATEMENT_HEAD = new Set(["SELECT", "WITH", "INSERT", "UPDATE", "DELETE"]);

/**
 * Built-in completion placement for one statement. Insert template stays unfiltered.
 * shortcut: a subquery inside a function argument stays unclassified, upgrade
 * when that site needs templates. A leading EXPLAIN wrapper is skipped.
 */
function priorTemplateTokens(tokens: readonly SqlToken[], from: number): SqlToken[] {
  const list = tokens.filter((token) => !token.kind.endsWith("Comment"));
  const container = list.find((token) => token.children && token.from < from && token.to >= from);
  if (container?.children) {
    const before = list.filter((token) => token.to <= container.from);
    const introducer = before.at(-1);
    const keyword = introducer?.kind === "Keyword" ? introducer.text.toUpperCase() : "";
    if (
      (introducer && NESTED_QUERY_MARK.has(introducer.text)) ||
      NESTED_QUERY_KEYWORD.has(keyword) ||
      explainStatementParen(before, container.children, from)
    ) {
      return priorTemplateTokens(container.children, from);
    }
  }
  return list.filter((token) => token.to <= from);
}

/** A `(SELECT ...)` after EXPLAIN is the statement. An option list such as `(ANALYZE)` is not. */
function explainStatementParen(
  before: readonly SqlToken[],
  children: readonly SqlToken[],
  from: number,
): boolean {
  if (before.length === 0 || withoutExplainWrapper(before).length !== 0) return false;
  const inner = children.filter((token) => !token.kind.endsWith("Comment") && token.to <= from);
  const head = inner[0];
  return !head || (head.kind === "Keyword" && STATEMENT_HEAD.has(head.text.toUpperCase()));
}

const EXPLAIN_OPTION = new Set(["ANALYZE", "VERBOSE", "EXTENDED", "PARTITIONS"]);

/** Drop EXPLAIN and one leading option list so the inner statement is classified. */
function withoutExplainWrapper(prior: readonly SqlToken[]): readonly SqlToken[] {
  const upper = (token?: SqlToken) => token?.text.toUpperCase() ?? "";
  if (upper(prior[0]) !== "EXPLAIN") return prior;
  let i = 1;
  const options = prior[i];
  if (options?.children && options.text.endsWith(")")) i++;
  while (EXPLAIN_OPTION.has(upper(prior[i]))) i++;
  if (upper(prior[i]) === "FORMAT") {
    i++;
    if (prior[i]?.text === "=") i++;
    const format = prior[i];
    if (
      format &&
      !format.children &&
      !["SELECT", "WITH", "INSERT", "UPDATE", "DELETE"].includes(upper(format))
    )
      i++;
  }
  return prior.slice(i);
}

export function sqlTemplateSite(state: EditorState, from: number): SqlTemplateSite {
  const statement = statementTokensAt(state, from);
  const prior = statement
    ? withoutExplainWrapper(priorTemplateTokens(statement.tokens, from))
    : undefined;
  if (!prior?.length) return { atStatementStart: true, inFromClause: false, afterFrom: false };
  const word = (token?: SqlToken) => (token?.kind === "Keyword" ? token.text.toUpperCase() : "");
  const head = word(prior[0]);
  let inFrom = false;
  let sawFrom = false;
  for (const token of prior) {
    const keyword = word(token);
    if (keyword === "FROM" || keyword === "JOIN") {
      sawFrom = true;
      inFrom = true;
      continue;
    }
    if (inFrom && token.text === ",") continue;
    if (CLAUSE_END.has(keyword)) inFrom = false;
  }
  const query = head === "SELECT" || head === "WITH";
  return {
    atStatementStart: false,
    inFromClause: query && inFrom,
    afterFrom: query && sawFrom,
  };
}

function templatePlace(trigger: string) {
  const key = trigger.toLowerCase();
  if (STATEMENT_TEMPLATES.has(key)) return "statement";
  if (key === "join" || key === "ljoin") return "from";
  if (key === "lim") return "query";
  return "any";
}

/**
 * Builds CodeMirror `snippetCompletion` options for bare unquoted prefixes
 * (`target.parts.length === 1`), never firing after `alias.` qualifiers.
 * A site limits built-in triggers. A custom trigger uses its declared site
 * and stays available when that site is missing. A built-in trigger keeps its site.
 */
export function snippetCompletions(
  engine: EngineKind,
  target: CompletionTarget,
  userSnippets: readonly SqlSnippet[] = [],
  applyTemplate?: SqlTemplateApplier,
  site?: SqlTemplateSite,
): Completion[] {
  if (engine === "redis" || target.parts.length !== 1 || target.parts[0].quoted) {
    return [];
  }
  const prefix = target.parts[0].value.toLowerCase();
  const candidates = effectiveSqlSnippets(engine, userSnippets).filter((item) => {
    if (prefix.length > 0 && !item.trigger.toLowerCase().startsWith(prefix)) return false;
    if (!site) return true;
    const builtinPlace = templatePlace(item.trigger);
    const place = builtinPlace === "any" ? (item.place ?? "any") : builtinPlace;
    if (place === "statement") return site.atStatementStart;
    if (place === "from") return site.inFromClause;
    if (place === "query") return site.afterFrom;
    return true;
  });
  candidates.sort((a, b) => {
    const aExact = prefix.length > 0 && a.trigger.toLowerCase() === prefix ? 0 : 1;
    const bExact = prefix.length > 0 && b.trigger.toLowerCase() === prefix ? 0 : 1;
    if (aExact !== bExact) return aExact - bExact;
    return a.trigger.toLowerCase().localeCompare(b.trigger.toLowerCase());
  });
  return candidates.map((item) => {
    const completion = snippetCompletion(item.body, {
      label: item.trigger,
      detail: item.isCustom ? `${item.title} · snippet` : `${item.title} · template`,
      type: "keyword",
    });
    if (applyTemplate)
      completion.apply = (editor, selected, from, to) =>
        applyTemplate(item.body, editor, selected, from, to);
    return completion;
  });
}
