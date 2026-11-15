import { afterEach, describe, expect, it, vi } from "vitest";
import type { QueryEvent, SnowflakeConsoleStart } from "./generated";

const native = vi.hoisted(() => ({ enabled: false, invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => native.enabled,
  invoke: native.invoke,
  Channel: class {
    onmessage?: (event: QueryEvent) => void;
  },
}));

const request: SnowflakeConsoleStart = {
  sessionId: "session",
  planId: "plan",
  planDigest: "a".repeat(64),
  clientRequestId: "00000000-0000-4000-8000-000000000001",
  approval: "approval",
};

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  vi.resetModules();
  native.invoke.mockReset();
});

describe("retained console transport", () => {
  it("fences delayed metadata calls after authoritative console connection and disconnect", async () => {
    native.enabled = true;
    native.invoke.mockResolvedValue({ id: "sealed-session", consoleMode: true });
    const { client } = await import("./client");
    await client.call("connect_prepared", {
      attemptId: "attempt",
      secrets: { password: null, sshPassword: null, keyPassphrase: null },
    });
    await expect(client.call("health", { sessionId: "sealed-session" })).rejects.toThrow(
      "reviewed SQL console",
    );
    await expect(
      client.call("inspect_relation", {
        sessionId: "sealed-session",
        request: { revision: "revision", targetId: "target" },
      }),
    ).rejects.toThrow("reviewed SQL console");
    await expect(
      client.call("preflight_snowflake_ddl", {
        request: {
          sessionId: "sealed-session",
          action: { kind: "dropTable", browseRevision: "r", targetId: "t" },
        },
      }),
    ).rejects.toThrow("reviewed SQL console");
    await client.call("disconnect", { sessionId: "sealed-session" });
    await expect(client.call("health", { sessionId: "sealed-session" })).rejects.toThrow(
      "reviewed SQL console",
    );
    expect(native.invoke.mock.calls.map(([command]) => command)).toEqual([
      "connect_prepared",
      "disconnect",
    ]);
  });

  it("keeps review, admission, events and SQL-free lookup on their dedicated native commands", async () => {
    native.enabled = true;
    native.invoke.mockResolvedValue(null);
    const { client } = await import("./client");
    const input = { sql: "SELECT 1", mode: "parsedStatements" as const, parameters: null };
    await client.prepareSnowflakeConsole("session", input);
    const events = vi.fn();
    await client.startSnowflakeConsole(request, events);
    await client.snowflakeConsoleRun(request.clientRequestId);
    await client.snowflakeConsoleRequest(request.clientRequestId);
    expect(native.invoke.mock.calls.map(([command]) => command)).toEqual([
      "prepare_snowflake_console",
      "start_snowflake_console",
      "snowflake_console_run",
      "snowflake_console_request",
    ]);
    expect(native.invoke.mock.calls[1][1].request).toEqual(request);
    const event = { type: "started", runId: "run", statementCount: 1 } as QueryEvent;
    native.invoke.mock.calls[1][1].onEvent.onmessage(event);
    expect(events).toHaveBeenCalledWith(event);
    expect(native.invoke.mock.calls[2][1]).toEqual({ clientRequestId: request.clientRequestId });
    expect(native.invoke.mock.calls[3][1]).toEqual({ clientRequestId: request.clientRequestId });
  });

  it("reports stream loss without inventing a remote terminal outcome or retrying start", async () => {
    native.enabled = false;
    vi.stubEnv("MODE", "integration");
    vi.stubEnv("VITE_TEST_BRIDGE_URL", "http://127.0.0.1:1422");
    vi.stubEnv("VITE_TEST_BRIDGE_TOKEN", "synthetic-only");
    const fetch = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ result: { kind: "started", runId: "run" } }),
      })
      .mockRejectedValueOnce(new Error("lost stream"))
      .mockResolvedValueOnce({ ok: true, json: async () => ({ result: null }) });
    vi.stubGlobal("fetch", fetch);
    const { client } = await import("./client");
    const received: QueryEvent[] = [];
    expect(await client.startSnowflakeConsole(request, (event) => received.push(event))).toEqual({
      kind: "started",
      runId: "run",
    });
    await vi.waitFor(() => expect(received).toHaveLength(1));
    expect(received[0].type).toBe("error");
    expect(received.some((event) => event.type === "finished")).toBe(false);
    await client.snowflakeConsoleRun(request.clientRequestId);
    expect(fetch).toHaveBeenCalledTimes(3);
    expect(
      fetch.mock.calls.filter(([url]) => url.endsWith("/start_snowflake_console")),
    ).toHaveLength(1);
    expect(JSON.parse(fetch.mock.calls[2][1].body)).toEqual({
      clientRequestId: request.clientRequestId,
    });
  });

  it("does not poll an execution stream after an authoritative pre-admission refusal", async () => {
    native.enabled = false;
    vi.stubEnv("MODE", "integration");
    vi.stubEnv("VITE_TEST_BRIDGE_URL", "http://127.0.0.1:1422");
    vi.stubEnv("VITE_TEST_BRIDGE_TOKEN", "synthetic-only");
    const result = {
      kind: "notAdmitted",
      clientRequestId: request.clientRequestId,
      sessionId: request.sessionId,
      planDigest: request.planDigest,
      code: "policyBlocked",
    };
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ result }) });
    vi.stubGlobal("fetch", fetch);
    const { client } = await import("./client");
    const onEvent = vi.fn();
    expect(await client.startSnowflakeConsole(request, onEvent)).toEqual(result);
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(onEvent).not.toHaveBeenCalled();
  });
});
