import type { EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import type { TableInfo, TableRef } from "../../ipc/generated";
import type { CompletionItem } from "../../data/completion-index";

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

export function tableLookup(tables: TableInfo[], database?: string, defaultSchema = "public") {
  const map = new Map<string, TableRef>();
  for (const { table } of tables) {
    map.set(`${table.schema}.${table.name}`.toLowerCase(), table);
    if (database) map.set(`${database}.${table.schema}.${table.name}`.toLowerCase(), table);
    if (
      !map.has(table.name.toLowerCase()) ||
      table.schema.toLowerCase() === defaultSchema.toLowerCase()
    )
      map.set(table.name.toLowerCase(), table);
  }
  return map;
}

export function catalogCompletions(tables: TableInfo[], database?: string): CompletionItem[] {
  const schemas = new Map<string, string>();
  const items: CompletionItem[] = [];
  for (const { table } of tables) {
    schemas.set(table.schema.toLowerCase(), table.schema);
    items.push(
      { label: table.name, type: "type", detail: table.schema },
      { label: `${table.schema}.${table.name}`, type: "type", detail: "table" },
    );
    if (database)
      items.push({
        label: `${database}.${table.schema}.${table.name}`,
        type: "type",
        detail: "table",
      });
  }
  for (const schema of schemas.values()) {
    items.push({ label: schema, type: "namespace", detail: "schema", apply: `${schema}.` });
    if (database)
      items.push({
        label: `${database}.${schema}`,
        type: "namespace",
        detail: "schema",
        apply: `${database}.${schema}.`,
      });
  }
  if (database)
    items.push({ label: database, type: "namespace", detail: "database", apply: `${database}.` });
  return items;
}

export function referencedTables(text: string, tables: Map<string, TableRef>) {
  const id = '(?:"(?:[^"]|"")+"|`(?:[^`]|``)+`|[\\p{L}_][\\p{L}\\p{N}_$]*)';
  const pattern = new RegExp(
    `\\b(?:from|join|update|into)\\s+(${id}(?:\\s*\\.\\s*${id}){0,2})(?:\\s+(?:as\\s+)?(${id}))?`,
    "giu",
  );
  const found = new Map<string, { table: TableRef; aliases: string[] }>();
  for (const match of text.matchAll(pattern)) {
    const name = match[1]
      .replace(/\s*\.\s*/g, ".")
      .replace(/"((?:[^"]|"")+)"/g, (_, value: string) => value.replace(/""/g, '"'))
      .replace(/`((?:[^`]|``)+)`/g, (_, value: string) => value.replace(/``/g, "`"));
    const table = tables.get(name.toLowerCase());
    if (!table) continue;
    const key = JSON.stringify(table),
      item = found.get(key) ?? { table, aliases: [] };
    const alias = match[2]
      ?.replace(/^["`]|["`]$/g, "")
      .replace(/``/g, "`")
      .replace(/""/g, '"');
    if (
      alias &&
      !/^(where|join|left|right|inner|outer|full|cross|on|group|order|limit|offset|set|returning|union|having|for|using)$/i.test(
        alias,
      )
    )
      item.aliases.push(alias);
    found.set(key, item);
    if (found.size >= 8) break;
  }
  return [...found.values()];
}
