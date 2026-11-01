import { snippetCompletion, type Completion } from "@codemirror/autocomplete";
import type { EngineKind, SqlSnippet } from "../../ipc/generated";
import type { CompletionTarget } from "./sql-context";

export interface EffectiveSqlSnippet {
  trigger: string;
  title: string;
  body: string;
  engine: EngineKind | null;
  isCustom: boolean;
}

const READ_TEMPLATES: readonly Omit<EffectiveSqlSnippet, "isCustom">[] = [
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
      isCustom: true,
    });
  }
  return [...byTrigger.values()].sort((a, b) =>
    a.trigger.toLowerCase().localeCompare(b.trigger.toLowerCase()),
  );
}

/**
 * Builds CodeMirror `snippetCompletion` options for bare unquoted prefixes
 * (`target.parts.length === 1`), never firing after `alias.` qualifiers.
 */
export function snippetCompletions(
  engine: EngineKind,
  target: CompletionTarget,
  userSnippets: readonly SqlSnippet[] = [],
): Completion[] {
  if (engine === "redis" || target.parts.length !== 1 || target.parts[0].quoted) {
    return [];
  }
  const prefix = target.parts[0].value.toLowerCase();
  const candidates = effectiveSqlSnippets(engine, userSnippets).filter(
    (item) => prefix.length === 0 || item.trigger.toLowerCase().startsWith(prefix),
  );
  candidates.sort((a, b) => {
    const aExact = prefix.length > 0 && a.trigger.toLowerCase() === prefix ? 0 : 1;
    const bExact = prefix.length > 0 && b.trigger.toLowerCase() === prefix ? 0 : 1;
    if (aExact !== bExact) return aExact - bExact;
    return a.trigger.toLowerCase().localeCompare(b.trigger.toLowerCase());
  });
  return candidates.map((item) =>
    snippetCompletion(item.body, {
      label: item.trigger,
      detail: item.isCustom ? `${item.title} · snippet` : `${item.title} · template`,
      type: "keyword",
    }),
  );
}
