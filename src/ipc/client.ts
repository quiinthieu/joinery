import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import type { AppError, QueryEvent } from "./generated";

export interface StudioClient {
  call<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  execute(planId: string, approval: string | null, onEvent: (event: QueryEvent) => void): Promise<string>;
  range(resultId: string, start: number, count: number): Promise<ArrayBuffer>;
}

class NativeClient implements StudioClient {
  call<T>(command: string, args: Record<string, unknown> = {}) { return invoke<T>(command, args); }
  execute(planId: string, approval: string | null, onEvent: (event: QueryEvent) => void) {
    const onEventChannel = new Channel<QueryEvent>();
    onEventChannel.onmessage = onEvent;
    return this.call<string>("execute_plan", { planId, approval, onEvent: onEventChannel });
  }
  range(resultId: string, start: number, count: number) {
    return new Promise<ArrayBuffer>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("Timed out loading result data")), 10000);
      const onData = new Channel<ArrayBuffer>();
      onData.onmessage = (buffer) => {
        clearTimeout(timer);
        if (!(buffer instanceof ArrayBuffer)) reject(new Error("The desktop runtime returned an invalid binary result"));
        else resolve(buffer);
      };
      this.call("result_range", { resultId, start, count, onData }).catch((error) => { clearTimeout(timer); reject(error); });
    });
  }
}

class BrowserTestClient implements StudioClient {
  constructor(private readonly url: string, private readonly token: string) {
    if (new URL(url).hostname !== "127.0.0.1") throw new Error("Test transport must use loopback");
  }
  private headers() { return { "Content-Type": "application/json", Authorization: `Bearer ${this.token}` }; }
  async call<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    const response = await fetch(`${this.url}/rpc/${command}`, { method: "POST", headers: this.headers(), body: JSON.stringify(args) });
    if (!response.ok) throw new Error("The browser test service is unavailable");
    const body = await response.json() as { result: T; error?: AppError };
    if (body.error) throw body.error;
    return body.result;
  }
  async execute(planId: string, approval: string | null, onEvent: (event: QueryEvent) => void) {
    const streamId = crypto.randomUUID();
    const runId = await this.call<string>("execute_plan", { planId, approval, streamId });
    const poll = async () => {
      try {
        const response = await fetch(`${this.url}/events/${streamId}`, { headers: this.headers() });
        if (!response.ok) throw new Error("The execution stream disconnected");
        const events = await response.json() as QueryEvent[];
        events.forEach(onEvent);
        if (!events.some((event) => event.type === "finished")) setTimeout(() => void poll(), 60);
      } catch {
        onEvent({ type: "error", error: { code: "connectionClosed", message: "The test execution stream disconnected" }, statementIndex: 0 });
        onEvent({ type: "finished", runId, status: "outcomeUnknown", elapsedMs: 0 });
      }
    };
    void poll();
    return runId;
  }
  async range(resultId: string, start: number, count: number) {
    const response = await fetch(`${this.url}/result/${resultId}?start=${start}&count=${count}`, { headers: this.headers() });
    if (!response.ok) throw await response.json();
    return response.arrayBuffer();
  }
}

class PreviewClient implements StudioClient {
  async call<T>(command: string): Promise<T> {
    if (command === "bootstrap") return { profiles: [], groups: [
      { id: "production", name: "Production", parentId: null, environment: "production" },
      { id: "staging", name: "Staging", parentId: null, environment: "staging" },
      { id: "local", name: "Local", parentId: null, environment: "local" },
    ], passphraseConfigured: false, supportedEngines: ["postgres"], resultBudgetBytes: 64 * 1024 * 1024, rowLimit: 100000 } as T;
    if (command === "resource_usage") return { retainedBytes: 0, resultCount: 0, sessionCount: 0, activeRuns: 0 } as T;
    if (command === "history") return [] as T;
    throw new Error("Open Joinery on your desktop to connect to a database. You can explore the grid benchmark here.");
  }
  async execute(): Promise<string> { throw new Error("Database execution is available in the desktop app"); }
  async range(): Promise<ArrayBuffer> { throw new Error("No database results are available in browser preview"); }
}

export const client: StudioClient = isTauri()
  ? new NativeClient()
  : import.meta.env.MODE === "integration" && import.meta.env.VITE_TEST_BRIDGE_URL
    ? new BrowserTestClient(import.meta.env.VITE_TEST_BRIDGE_URL, import.meta.env.VITE_TEST_BRIDGE_TOKEN ?? "")
    : new PreviewClient();

export const desktopAvailable = isTauri() || import.meta.env.MODE === "integration";
export function messageOf(error: unknown): string {
  if (error && typeof error === "object" && "message" in error) return String(error.message);
  return String(error);
}
