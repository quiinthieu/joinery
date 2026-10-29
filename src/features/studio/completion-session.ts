import type { EditorState } from "@codemirror/state";
import type { CompletionContext, CompletionResult } from "@codemirror/autocomplete";
import type { EngineKind, TableInfo, TableRef, TableSchema } from "../../ipc/generated";
import type { CompletionItem } from "../../data/completion-index";
import type {
  CompletionRequest,
  CompletionResponse,
  CompletionStamp,
} from "../../data/completion-protocol";
import { REDIS_COMMANDS } from "../redis/commands";
import {
  catalogCompletions,
  columnCompletions,
  completionTarget,
  referencedTables,
  tableLookup,
  type SqlScope,
} from "./sql-context";
import { identifierValue, quoteIdentifier } from "./sql-identifiers";

interface Configuration {
  engine: EngineKind;
  sessionId: string | null;
  tables: TableInfo[];
  database?: string;
  defaultSchema?: string;
  searchPath?: string[];
  unqualifiedTables?: TableRef[];
  contextRevision?: string;
}
interface SchemaEntry {
  start(): Promise<TableSchema>;
  promise: Promise<TableSchema | null>;
  finish(value: TableSchema | null): void;
  running: boolean;
}

/** Only current references are retained; at most eight IPC reads run at once. */
export class CompletionSchemas {
  private entries = new Map<string, SchemaEntry>();
  private running = 0;

  retain(keys: string[]) {
    for (const [key, entry] of this.entries) {
      if (keys.includes(key)) continue;
      entry.finish(null);
      this.entries.delete(key);
    }
  }

  get(key: string, start: () => Promise<TableSchema>, onError: (error: unknown) => void) {
    const cached = this.entries.get(key);
    if (cached) return cached.promise;
    if (this.entries.size >= 8) return Promise.resolve(null);
    let finish!: SchemaEntry["finish"];
    const promise = new Promise<TableSchema | null>((resolve) => {
      finish = resolve;
    });
    const entry: SchemaEntry = {
      promise,
      finish,
      running: false,
      start: async () => {
        try {
          return await start();
        } catch (error) {
          // A failure from an old session must not evict its replacement.
          if (this.entries.get(key) === entry) {
            this.entries.delete(key);
            onError(error);
          }
          throw error;
        }
      },
    };
    this.entries.set(key, entry);
    this.drain();
    return promise;
  }

  private drain() {
    for (const entry of this.entries.values()) {
      if (this.running >= 8) break;
      if (entry.running) continue;
      entry.running = true;
      this.running++;
      void entry
        .start()
        .then(entry.finish, () => entry.finish(null))
        .finally(() => {
          this.running--;
          this.drain();
        });
    }
  }
}

interface Pending {
  stamp: CompletionStamp;
  revision: number | null;
  finish(result: CompletionResult | null): void;
  resume(): void;
}
const keywords = [
  "SELECT",
  "FROM",
  "WHERE",
  "JOIN",
  "LEFT JOIN",
  "ORDER BY",
  "GROUP BY",
  "LIMIT",
  "INSERT INTO",
  "UPDATE",
  "DELETE FROM",
  "RETURNING",
  "WITH",
  "EXPLAIN",
];

/** Per-editor lifecycle for the existing worker, never application/global state. */
export class CompletionSession {
  private configuration?: Configuration;
  private lookup = tableLookup([], "postgres");
  private schemas = new CompletionSchemas();
  private state?: EditorState;
  private document = 0;
  private context = 0;
  private revision = 0;
  private sequence = 0;
  private scopeKey = "";
  private scope: SqlScope = { references: [], names: [], allowBare: false };
  private loading = false;
  private timer?: ReturnType<typeof setTimeout>;
  private pending = new Map<number, Pending>();
  private disposed = false;

  constructor(
    private send: (message: CompletionRequest) => void,
    private load: (
      sessionId: string,
      table: TableRef,
      contextRevision?: string,
    ) => Promise<TableSchema>,
    private notify: (error: unknown) => void,
  ) {}

  configure(config: Configuration) {
    const previous = this.configuration;
    if (
      previous &&
      previous.engine === config.engine &&
      previous.sessionId === config.sessionId &&
      previous.tables === config.tables &&
      previous.database === config.database &&
      previous.defaultSchema === config.defaultSchema &&
      previous.searchPath === config.searchPath &&
      previous.unqualifiedTables === config.unqualifiedTables &&
      previous.contextRevision === config.contextRevision
    )
      return false;
    this.configuration = config;
    this.context++;
    this.cancel();
    this.schemas.retain([]);
    this.lookup = tableLookup(
      config.tables,
      config.engine,
      config.database,
      config.defaultSchema,
      config.searchPath,
      config.unqualifiedTables,
    );
    const items =
      config.engine === "redis"
        ? REDIS_COMMANDS.map((label) => ({ label, type: "keyword" }))
        : [
            ...catalogCompletions(config.tables, this.lookup),
            ...keywords.map((label) => ({ label, type: "keyword" })),
          ];
    this.replace("catalog", items);
    this.scopeKey = "";
    return true;
  }

  update(state: EditorState) {
    if (this.disposed || !this.configuration) return;
    if (
      this.state?.doc !== state.doc ||
      this.state.selection.main.head !== state.selection.main.head
    ) {
      if (this.state?.doc !== state.doc) this.document++;
      this.cancelPending();
    }
    this.state = state;
    const config = this.configuration;
    const scope =
      config.engine === "redis"
        ? { references: [], names: [], allowBare: false }
        : referencedTables(state, this.lookup);
    const key = JSON.stringify(scope);
    if (key === this.scopeKey) return;
    this.cancel();
    this.scopeKey = key;
    this.scope = scope;
    const current = ++this.context;
    this.replace("columns", []);
    this.schemas.retain(scope.references.map((ref) => JSON.stringify(ref.table)));
    if (!config.sessionId || !scope.references.length) return;
    this.loading = true;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      const sessionId = config.sessionId!;
      void Promise.all(
        scope.references.map(({ table }) =>
          this.schemas.get(
            JSON.stringify(table),
            () => this.load(sessionId, table, config.contextRevision),
            (error) => {
              if (!this.disposed && this.context === current) this.notify(error);
            },
          ),
        ),
      ).then((schemas) => {
        if (this.disposed || this.context !== current) return;
        this.replace("columns", columnCompletions(scope, schemas, config.engine));
        this.loading = false;
        for (const pending of this.pending.values()) pending.resume();
      });
    }, 100);
  }

  request(context: CompletionContext): Promise<CompletionResult | null> | null {
    if (this.disposed || !this.configuration || context.aborted) return null;
    this.update(context.state);
    const config = this.configuration;
    const command = config.engine === "redis" ? context.matchBefore(/\w+/) : null;
    const target =
      config.engine === "redis"
        ? command && {
            from: command.from,
            to: command.to,
            parts: [{ value: command.text, quoted: false }],
          }
        : completionTarget(context.state, context.pos);
    if (!target || (!context.explicit && target.from === target.to)) return null;
    const id = ++this.sequence;
    const stamp = { document: this.document, context: this.context, position: context.pos };
    return new Promise((resolve) => {
      const timer = setTimeout(() => finish(null), 2000);
      const finish = (result: CompletionResult | null) => {
        clearTimeout(timer);
        this.pending.delete(id);
        resolve(result);
      };
      const pending: Pending = {
        stamp,
        revision: null,
        finish: (result) => finish(result && { ...result, from: target.from, to: target.to }),
        resume: () => {
          if (!this.pending.has(id) || !this.current(stamp) || context.aborted) {
            finish(null);
            return;
          }
          pending.revision = this.revision;
          const first = identifierValue(target.parts[0], config.engine);
          const columnsOnly =
            target.parts.length > 1 &&
            this.scope.names.some(
              (name) =>
                name === first ||
                (config.engine === "mysql" && name.toLowerCase() === first.toLowerCase()),
            );
          this.send({
            type: "prefix",
            id,
            stamp,
            revision: this.revision,
            parts: target.parts,
            engine: config.engine,
            columnsOnly,
          });
        },
      };
      this.pending.set(id, pending);
      context.addEventListener("abort", () => finish(null), { onDocChange: true });
      // Waiters live only in the pending map. A hung metadata read must not
      // retain one Promise.then closure (and old document) per cancelled query.
      if (!this.loading) pending.resume();
    });
  }

  receive(message: CompletionResponse) {
    if (message.type !== "suggestions") return;
    const pending = this.pending.get(message.id);
    if (!pending) return;
    if (
      !this.current(message.stamp) ||
      message.stamp.document !== pending.stamp.document ||
      message.stamp.context !== pending.stamp.context ||
      message.stamp.position !== pending.stamp.position ||
      message.revision !== pending.revision ||
      message.revision !== this.revision
    ) {
      pending.finish(null);
      return;
    }
    const engine = this.configuration!.engine;
    pending.finish({
      from: 0,
      filter: false,
      options: message.items.map((item) => {
        if (!item.path || item.apply) return item;
        const label = item.path.map((part) => quoteIdentifier(part, engine)).join(".");
        return { ...item, label, apply: label + (item.type === "namespace" ? "." : "") };
      }),
    });
  }

  dispose() {
    this.disposed = true;
    this.context++;
    this.cancel();
    this.schemas.retain([]);
    this.state = undefined;
  }

  private current(stamp: CompletionStamp) {
    return (
      !this.disposed &&
      stamp.document === this.document &&
      stamp.context === this.context &&
      stamp.position === this.state?.selection.main.head
    );
  }
  private replace(section: "catalog" | "columns", items: CompletionItem[]) {
    for (const pending of this.pending.values())
      if (pending.revision !== null) pending.finish(null);
    this.send({ type: "replace", section, revision: ++this.revision, items });
  }
  private cancelPending() {
    for (const pending of this.pending.values()) pending.finish(null);
  }
  private cancel() {
    clearTimeout(this.timer);
    this.timer = undefined;
    this.loading = false;
    this.cancelPending();
  }
}
