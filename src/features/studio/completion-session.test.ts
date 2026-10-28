import { afterEach, expect, test, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { sql } from "@codemirror/lang-sql";
import { CompletionContext } from "@codemirror/autocomplete";
import type { TableInfo, TableRef, TableSchema } from "../../ipc/generated";
import { CompletionWorkerIndex, type CompletionRequest } from "../../data/completion-protocol";
import { CompletionSchemas, CompletionSession } from "./completion-session";
import { sqlDialect } from "./sql-identifiers";

const tables: TableInfo[] = ["orders", "customers"].map((name) => ({
  table: { schema: "public", name },
  kind: "BASE TABLE",
  estimatedRows: null,
  totalBytes: null,
}));
const config = { engine: "postgres" as const, sessionId: "fixture", tables };
const schema = (name = "orders", column = "notes") =>
  ({
    table: { schema: "public", name },
    columns: [{ name: column, databaseType: "text" }],
  }) as TableSchema;
function state(text = "SELECT o.no| FROM orders o") {
  return EditorState.create({
    doc: text.replace("|", ""),
    selection: { anchor: text.indexOf("|") },
    extensions: [sql({ dialect: sqlDialect("postgres") })],
  });
}
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function setup(
  load: (sessionId: string, table: TableRef) => Promise<TableSchema> = vi.fn(async () => schema()),
  automatic = true,
) {
  const worker = new CompletionWorkerIndex(),
    messages: CompletionRequest[] = [];
  const notify = vi.fn();
  const session = new CompletionSession(
    (message) => {
      messages.push(message);
      const response = worker.handle(message);
      if (automatic) queueMicrotask(() => session.receive(response));
    },
    load,
    notify,
  );
  session.configure(config);
  return { session, messages, notify, load, worker };
}
function request(session: CompletionSession, editor: EditorState) {
  return session.request(new CompletionContext(editor, editor.selection.main.head, true));
}
afterEach(() => vi.useRealTimers());

test("loads scoped columns and preserves successful metadata when another table is denied", async () => {
  vi.useFakeTimers();
  const load = vi.fn(async (_session: string, table: { name: string }) => {
    if (table.name === "customers") throw new Error("permission denied");
    return schema();
  });
  const { session, notify } = setup(load);
  const result = request(session, state("SELECT o.no| FROM orders o JOIN customers c ON true"));
  await vi.advanceTimersByTimeAsync(100);
  expect((await result)?.options.map((i) => i.apply)).toEqual(["o.notes"]);
  expect(notify).toHaveBeenCalledOnce();
  session.dispose();
  expect(vi.getTimerCount()).toBe(0);
});

test("document edits, caret changes and context replacement discard pending responses", async () => {
  vi.useFakeTimers();
  const { session, messages } = setup(undefined, false);
  const original = state();
  const first = request(session, original);
  await vi.advanceTimersByTimeAsync(100);
  const old = messages.find((m) => m.type === "prefix")!;
  expect(old.type).toBe("prefix");
  const moved = original.update({ selection: { anchor: 1 } }).state;
  session.update(moved);
  expect(await first).toBeNull();
  const next = request(session, original);
  await vi.advanceTimersByTimeAsync(0);
  if (old.type === "prefix")
    session.receive({
      type: "suggestions",
      id: old.id,
      revision: old.revision,
      stamp: old.stamp,
      items: [{ label: "stale", type: "property" }],
    });
  session.update(original.update({ changes: { from: 11, insert: "t" } }).state);
  expect(await next).toBeNull();
  const refreshed = request(session, original);
  session.configure({ ...config, tables: [...tables] });
  expect(await refreshed).toBeNull();
  const switched = request(session, original);
  session.configure({ ...config, sessionId: "replacement" });
  expect(await switched).toBeNull();
  session.dispose();
  expect(vi.getTimerCount()).toBe(0);
});

test("an obsolete metadata response cannot install columns into a new query scope", async () => {
  vi.useFakeTimers();
  const old = deferred<TableSchema>(),
    fresh = deferred<TableSchema>();
  const load = vi
    .fn()
    .mockImplementationOnce(() => old.promise)
    .mockImplementationOnce(() => fresh.promise);
  const { session, messages } = setup(load);
  const first = request(session, state());
  await vi.advanceTimersByTimeAsync(100);
  const second = request(session, state("SELECT c.na| FROM customers c"));
  await vi.advanceTimersByTimeAsync(100);
  fresh.resolve(schema("customers", "name"));
  await vi.advanceTimersByTimeAsync(0);
  expect((await second)?.options.map((i) => i.apply)).toEqual(['c."name"']);
  old.resolve(schema("orders", "obsolete"));
  await vi.advanceTimersByTimeAsync(0);
  expect(await first).toBeNull();
  expect(
    messages
      .filter((m) => m.type === "replace")
      .some((m) => m.type === "replace" && m.items.some((i) => i.label.includes("obsolete"))),
  ).toBe(false);
  session.dispose();
});

test("typing expressions reuses metadata and does not rebuild the catalog", async () => {
  vi.useFakeTimers();
  const { session, load, messages } = setup();
  const first = request(session, state());
  await vi.advanceTimersByTimeAsync(100);
  await first;
  const second = request(session, state("SELECT o.not| FROM orders o WHERE id=12"));
  await vi.advanceTimersByTimeAsync(0);
  expect((await second)?.options.map((i) => i.apply)).toEqual(["o.notes"]);
  expect(load).toHaveBeenCalledOnce();
  expect(messages.filter((m) => m.type === "replace" && m.section === "catalog")).toHaveLength(1);
  session.dispose();
});

test("quoted acceptance replaces the whole identifier, and disposal settles a slow request", async () => {
  vi.useFakeTimers();
  const { session } = setup(vi.fn(async () => schema("orders", 'No"tes')));
  const editor = state('SELECT o."No|pe" FROM orders o');
  const pending = request(session, editor);
  await vi.advanceTimersByTimeAsync(100);
  const result = (await pending)!;
  expect(result.options[0].apply).toBe('o."No""tes"');
  expect(
    editor
      .update({
        changes: { from: result.from, to: result.to, insert: result.options[0].apply as string },
      })
      .state.doc.toString(),
  ).toBe('SELECT o."No""tes" FROM orders o');
  session.dispose();
  const slow = setup(vi.fn(() => new Promise<TableSchema>(() => {})));
  const waiting = request(slow.session, state());
  await vi.advanceTimersByTimeAsync(100);
  slow.session.dispose();
  expect(await waiting).toBeNull();
  expect(vi.getTimerCount()).toBe(0);
});

test("an old failed cache entry cannot delete a new request for the same table", async () => {
  const cache = new CompletionSchemas(),
    old = deferred<TableSchema>(),
    fresh = deferred<TableSchema>();
  const notify = vi.fn(),
    load = vi.fn(() => fresh.promise);
  const previous = cache.get("orders", () => old.promise, notify);
  cache.retain([]);
  const next = cache.get("orders", load, notify);
  old.reject(new Error("old session failed"));
  await previous;
  await Promise.resolve();
  expect(cache.get("orders", load, notify)).toBe(next);
  expect(load).toHaveBeenCalledOnce();
  expect(notify).not.toHaveBeenCalled();
  fresh.resolve(schema());
  expect(await next).toEqual(schema());
});

test("bounds in-flight reads through rapid scope changes and drops obsolete queued entries", async () => {
  const cache = new CompletionSchemas(),
    held = Array.from({ length: 8 }, () => deferred<TableSchema>());
  const notify = vi.fn(),
    load = vi.fn(async () => schema());
  held.forEach((entry, i) => cache.get(String(i), () => entry.promise, notify));
  cache.retain([]);
  const obsolete = cache.get("obsolete", load, notify);
  cache.retain([]);
  const current = cache.get("current", load, notify);
  expect(load).not.toHaveBeenCalled();
  held.forEach((entry) => entry.resolve(schema()));
  await expect(obsolete).resolves.toBeNull();
  await expect(current).resolves.toEqual(schema());
  expect(load).toHaveBeenCalledOnce();
});
