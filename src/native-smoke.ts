import { client, messageOf } from "./ipc/client";
import { resultCache } from "./data/result-cache";
import { useStudio } from "./state/studio";
import type { ResultInfo } from "./ipc/generated";
import { getCurrentWindow } from "@tauri-apps/api/window";

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
    report.bootstrapMs = performance.now() - started;
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
    report.passed = true;
    report.checks = [
      "native bootstrap",
      "raw binary channel",
      "Arrow worker",
      "exact integer",
      "NULL and empty",
      "Unicode",
      "formatter worker",
      "CodeMirror",
      "Canvas grid",
      "native close guard",
    ];
    report.frontendElapsedMs = performance.now() - started;
  } catch (error) {
    report.error = messageOf(error);
  }
  await client.call("native_smoke_report", { report });
}
