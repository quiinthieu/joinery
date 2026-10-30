import { client, messageOf } from "./ipc/client";
import { resultCache } from "./data/result-cache";
import { useStudio } from "./state/studio";
import type { ResourceUsage, ResultInfo } from "./ipc/generated";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { EditorState } from "@codemirror/state";
import { CompletionContext } from "@codemirror/autocomplete";
import { sql } from "@codemirror/lang-sql";
import { CompletionSession } from "./features/studio/completion-session";
import { sqlDialect } from "./features/studio/sql-identifiers";
import type { CompletionResponse } from "./data/completion-protocol";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
function workerRoundtrip(worker: Worker, payload: unknown) {
  return new Promise<unknown>((resolve, reject) => {
    const timer = setTimeout(() => {
      worker.terminate();
      reject(new Error("Native worker timed out"));
    }, 5000);
    worker.onmessage = ({ data }: MessageEvent) => {
      clearTimeout(timer);
      worker.terminate();
      resolve(data);
    };
    worker.onerror = () => {
      clearTimeout(timer);
      worker.terminate();
      reject(new Error("Native worker could not load"));
    };
    worker.postMessage(payload);
  });
}
export async function runNativeSmoke() {
  const started = performance.now();
  const report: Record<string, unknown> = { passed: false, checks: [] };
  try {
    await useStudio.getState().initialize();
    assert(useStudio.getState().bootstrap, "Native bootstrap failed");
    for (const [command, args] of [
      [
        "completion_catalog",
        {
          sessionId: "missing-native-fixture",
          revision: "obsolete",
          table: { schema: "public", name: "missing" },
        },
      ],
      [
        "completion_schema",
        {
          sessionId: "missing-native-fixture",
          revision: "obsolete",
          table: { schema: "public", name: "missing" },
        },
      ],
      [
        "referencing_foreign_keys",
        {
          sessionId: "missing-native-fixture",
          table: { schema: "public", name: "missing" },
        },
      ],
      [
        "prepare_browse_table",
        {
          sessionId: "missing-native-fixture",
          table: { schema: "public", name: "missing" },
          query: { sort: [], filters: [], rawWhere: "1 = 1", rawOrderBy: null },
        },
      ],
      [
        "prepare_table_changes",
        {
          sessionId: "missing-native-fixture",
          changes: { resultId: "missing-result", updates: [], deletes: [], inserts: [] },
        },
      ],
    ] as const) {
      let denied = false;
      try {
        await client.call(command, args);
      } catch (error) {
        denied =
          !!error &&
          typeof error === "object" &&
          "code" in error &&
          error.code === "connectionClosed";
      }
      assert(denied, `Native ${command} must reach the Rust disconnected-session guard`);
    }
    report.bootstrapMs = performance.now() - started;
    const shellDeadline = performance.now() + 3000;
    while (
      !document.querySelector('[aria-label="New query tab"]') &&
      performance.now() < shellDeadline
    )
      await new Promise((resolve) => setTimeout(resolve, 25));
    assert(document.querySelector('[aria-label="New query tab"]'), "Native shell did not mount");
    report.shellDomReadyMs = performance.now() - started;
    const checkpoints: unknown[] = [];
    const checkpoint = async (phase: string, cycle?: number) => {
      const value = await client.call<{
        nativeElapsedMs: number;
        usage: ResourceUsage;
        options: { cleanupCycles: number; idleMs: number };
      }>("native_smoke_checkpoint", { phase, cycle: cycle ?? null });
      if (cycle === undefined)
        checkpoints.push({ ...value, frontendRetainedBytes: resultCache.retainedBytes });
      return value;
    };
    const { options } = await checkpoint("shell");
    report.options = options;
    // Leave a measurable empty-idle interval. This is a hidden test WebView;
    // its shell timing is deliberately not labeled visible first paint.
    await new Promise((resolve) => setTimeout(resolve, options.idleMs));
    await checkpoint("loading");
    const result = await client.call<ResultInfo>("native_smoke_fixture");
    const buffer = await client.range(result.id, 0, 128);
    assert(
      buffer instanceof ArrayBuffer && buffer.byteLength > 0,
      "Tauri channel did not return an ArrayBuffer",
    );
    await resultCache.ensure(result.id, 0, result.retainedRows);
    assert(
      resultCache.read(result.id, "c0", 0, true)?.value === "9223372036854775807",
      "Exact integer changed in native IPC",
    );
    assert(
      resultCache.read(result.id, "c1", 0, true)?.value === null,
      "NULL changed in native IPC",
    );
    assert(
      resultCache.read(result.id, "c2", 0, true)?.value === "",
      "Empty string changed in native IPC",
    );
    assert(
      resultCache.read(result.id, "c3", 0, true)?.value === "héllo;世界",
      "Unicode changed in native IPC",
    );
    report.binaryBytes = buffer.byteLength;
    const formatted = (await workerRoundtrip(
      new Worker(new URL("./workers/formatter.worker.ts", import.meta.url), { type: "module" }),
      { id: 1, sql: "select 1", language: "postgresql" },
    )) as { sql?: string };
    assert(formatted.sql?.includes("SELECT"), "Native SQL formatter failed");
    const completionWorker = new Worker(
      new URL("./workers/completion.worker.ts", import.meta.url),
      { type: "module" },
    );
    const table = { schema: "public", name: "completion.names" };
    const completion = new CompletionSession(
      (message) => completionWorker.postMessage(message),
      async () => ({
        table,
        columns: [{ ...result.columns[0], name: 'No"tes' }],
        primaryKey: [],
        foreignKeys: [],
        editable: false,
      }),
      () => {
        report.completionMetadataError = true;
      },
    );
    try {
      completionWorker.onmessage = ({ data }: MessageEvent<CompletionResponse>) =>
        completion.receive(data);
      completion.configure({
        engine: "postgres",
        sessionId: "native-synthetic",
        tables: [{ table, kind: "BASE TABLE", estimatedRows: null, totalBytes: null }],
      });
      const doc = 'SELECT f."No" FROM public."completion.names" f';
      const position = doc.indexOf('"No"') + 3;
      const state = EditorState.create({
        doc,
        selection: { anchor: position },
        extensions: [sql({ dialect: sqlDialect("postgres") })],
      });
      const suggestion = await completion.request(new CompletionContext(state, position, true));
      assert(suggestion?.options[0]?.apply === 'f."No""tes"', "Native scoped completion failed");
      const accepted = state
        .update({
          changes: {
            from: suggestion.from,
            to: suggestion.to,
            insert: suggestion.options[0].apply as string,
          },
        })
        .state.doc.toString();
      assert(
        accepted === 'SELECT f."No""tes" FROM public."completion.names" f',
        "Native completion changed surrounding SQL",
      );
    } finally {
      completion.dispose();
      completionWorker.terminate();
    }
    const tabId = useStudio
      .getState()
      .openQuery(null, "SELECT exact_integer, null_value, empty_value, unicode;");
    useStudio
      .getState()
      .updateTab(tabId, { results: [result], activeResultId: result.id, status: "completed" });
    const deadline = performance.now() + 8000;
    while (
      (!document.querySelector("canvas") || !document.querySelector(".cm-editor")) &&
      performance.now() < deadline
    )
      await new Promise((resolve) => setTimeout(resolve, 25));
    assert(document.querySelector("canvas"), "Native Canvas grid did not mount");
    assert(document.querySelector(".cm-editor"), "Native CodeMirror editor did not mount");
    const { usage: loaded } = await checkpoint("loaded");
    assert(loaded.resultCount === 1 && loaded.retainedBytes > 0, "Native fixture was not retained");
    useStudio.getState().updateTab(tabId, { status: "running" });
    await getCurrentWindow().close();
    const closeDeadline = performance.now() + 2000;
    while (!document.querySelector('[role="dialog"]') && performance.now() < closeDeadline)
      await new Promise((resolve) => setTimeout(resolve, 25));
    assert(
      document.querySelector('[role="dialog"]')?.getAttribute("aria-label") === "Close Joinery?",
      "Native close did not protect pending work",
    );
    const keepWorking = [
      ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
    ].find((button) => button.textContent === "Keep working");
    assert(keepWorking, "Native close confirmation is missing its cancel action");
    keepWorking.click();
    useStudio.getState().updateTab(tabId, { status: "completed" });
    useStudio.getState().closeTab(tabId);
    const waitForRelease = async () => {
      const deadline = performance.now() + 3000;
      while (performance.now() < deadline) {
        const usage = await client.call<ResourceUsage>("resource_usage");
        if (usage.resultCount === 0 && usage.retainedBytes === 0) {
          assert(usage.sessionCount === 0 && usage.activeRuns === 0, "Native resources leaked");
          assert(resultCache.retainedBytes === 0, "Frontend Arrow pages leaked");
          return;
        }
        await new Promise((resolve) => setTimeout(resolve, 25));
      }
      throw new Error("Native result resources were not released after closing the tab");
    };
    await waitForRelease();
    await checkpoint("cycling");
    // Exercise the real tab-close path repeatedly, including four concurrent
    // native binary/worker page requests during each close.
    for (let cycle = 0; cycle < options.cleanupCycles; cycle++) {
      const next = await client.call<ResultInfo>("native_smoke_fixture");
      const id = useStudio.getState().openQuery(null, "SELECT lifecycle_fixture;");
      useStudio.getState().updateTab(id, { results: [next], activeResultId: next.id });
      await resultCache.ensure(next.id, 0, next.retainedRows);
      assert(resultCache.retainedBytes > 0, "Lifecycle fixture did not populate Arrow cache");
      const pending = [128, 256, 384, 512].map((row) =>
        resultCache.ensure(next.id, row, next.retainedRows),
      );
      useStudio.getState().closeTab(id);
      await Promise.all(pending);
      await waitForRelease();
      await checkpoint("cycling", cycle + 1);
    }
    report.cleanupCycles = options.cleanupCycles;
    await checkpoint("released");
    await new Promise((resolve) => setTimeout(resolve, options.idleMs));
    await checkpoint("idle");
    report.checkpoints = checkpoints;
    report.passed = true;
    report.checks = [
      "native bootstrap",
      "completion IPC disconnected-session guards",
      "raw binary channel",
      "Arrow worker",
      "exact integer",
      "NULL and empty",
      "Unicode",
      "formatter worker",
      "scoped completion worker and quoted acceptance",
      "CodeMirror",
      "Canvas grid",
      "native close guard",
      "repeated result/tab cleanup",
      "in-flight Arrow release",
    ];
    report.frontendElapsedMs = performance.now() - started;
  } catch (error) {
    report.error = messageOf(error);
  }
  await client.call("native_smoke_report", { report });
}
