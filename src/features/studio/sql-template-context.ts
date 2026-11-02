import { pickedCompletion, snippet, type Completion } from "@codemirror/autocomplete";
import { ChangeSet, StateEffect, type Transaction } from "@codemirror/state";
import { isolateHistory } from "@codemirror/commands";
import type { EditorView } from "@codemirror/view";
import type { EngineKind } from "../../ipc/generated";

export type SqlTemplateApplier = (
  body: string,
  editor: EditorView,
  completion: Completion | undefined,
  from: number,
  to: number,
) => void;

export interface SqlTemplateContext {
  database?: string | null;
  schema?: string | null;
}

/** Dynamic text is inserted once, after parsing tab stops, in one transaction. */
export async function applySqlTemplate(
  body: string,
  engine: EngineKind,
  editor: EditorView,
  completion: Completion | undefined,
  from: number,
  to: number,
  options: {
    resolveContext(): Promise<SqlTemplateContext>;
    isCurrent(): boolean;
  },
): Promise<void> {
  const state = editor.state;
  const current = () => {
    if (
      !options.isCurrent() ||
      editor.state.doc !== state.doc ||
      !editor.state.selection.eq(state.selection)
    ) {
      throw new Error("The editor or console context changed. Insert the template again.");
    }
  };
  current();
  const variables = [...body.matchAll(/\{\{([^{}]*)\}\}/g)];
  for (const [, name] of variables) {
    if (!["database", "schema", "selection"].includes(name))
      throw new Error(
        `Unsupported template variable: {{${name}}}. Use database, schema or selection.`,
      );
  }
  if (/\{\{|\}\}/.test(body.replace(/\{\{([^{}]*)\}\}/g, "")))
    throw new Error("Incomplete template variable. Use {{database}}, {{schema}} or {{selection}}.");
  const selection = state.selection.main;
  const selected = state.sliceDoc(selection.from, selection.to);
  if (variables.some(([, name]) => name === "selection") && selection.empty)
    throw new Error("Select SQL in the editor before inserting this template.");
  const context = variables.some(([, name]) => name !== "selection")
    ? await options.resolveContext()
    : {};
  current();
  const quote = engine === "mysql" ? "`" : '"';
  const values: string[] = [];
  let nonce = 0;
  let marker = `JOINERY_TEMPLATE_VALUE_${nonce}_`;
  while (
    body.includes(marker) ||
    selected.includes(marker) ||
    Object.values(context).some((value) => value?.includes(marker))
  )
    marker = `JOINERY_TEMPLATE_VALUE_${++nonce}_`;
  const template = body.replace(/\{\{([^{}]*)\}\}/g, (_, name: string) => {
    let value = selected;
    if (name !== "selection") {
      const identifier = context[name as "database" | "schema"];
      if (!identifier || identifier.includes("\0"))
        throw new Error(
          `No observed current ${name}. Connect or refresh the console context, then try again.`,
        );
      value = quote + identifier.split(quote).join(quote + quote) + quote;
    }
    values.push(value);
    return `${marker}${values.length - 1}_END`;
  });
  const bytes = (value: string) => new TextEncoder().encode(value).length;
  if (bytes(template) + values.reduce((size, value) => size + bytes(value), 0) > 1024 * 1024)
    throw new Error("Expanded template exceeds the 1 MiB editor limit.");
  // Inert markers keep selected text out of CodeMirror's placeholder parser and
  // automatic indentation. Map tab stops through the literal replacements.
  snippet(template)(
    {
      state,
      dispatch(transaction: Transaction) {
        const text = transaction.newDoc.sliceString(from, transaction.changes.mapPos(to, 1));
        const replacements = values
          .flatMap((value, index) => {
            const token = `${marker}${index}_END`;
            const found: { from: number; to: number; insert: string }[] = [];
            let offset = 0;
            for (let at; (at = text.indexOf(token, offset)) >= 0; offset = at + token.length)
              found.push({ from: from + at, to: from + at + token.length, insert: value });
            return found;
          })
          .sort((a, b) => a.from - b.from);
        const changes = ChangeSet.of(replacements, transaction.newDoc.length, state.lineBreak);
        const inserted = changes
          .apply(transaction.newDoc)
          .sliceString(from, changes.mapPos(transaction.changes.mapPos(to, 1), 1));
        if (bytes(inserted) > 1024 * 1024)
          throw new Error("Expanded template exceeds the 1 MiB editor limit.");
        editor.dispatch({
          changes: transaction.changes.compose(changes),
          selection: transaction.newSelection.map(changes),
          effects: StateEffect.mapEffects(transaction.effects, changes),
          annotations: [
            isolateHistory.of("full"),
            ...(completion ? [pickedCompletion.of(completion)] : []),
          ],
          userEvent: "input.complete",
          scrollIntoView: true,
        });
      },
    },
    completion ?? null,
    from,
    to,
  );
}
