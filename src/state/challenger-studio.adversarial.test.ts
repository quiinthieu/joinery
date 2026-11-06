import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStudio } from "./studio";
import { client } from "../ipc/client";
import type { AppError, QueryEvent, Scratchpad, SessionInfo } from "../ipc/generated";
import {
  inferParamKind,
  validateParamDraft,
  buildQueryParamInputs,
  formatParamDisplayKey,
  type QueryParamDraft,
} from "../features/studio/ParameterDialog";

vi.mock("../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
  isTauri: () => false,
}));

const mockSession: SessionInfo = {
  id: "session-test",
  connectionId: "conn-test",
  engine: "snowflake",
  serverVersion: "8.0",
  latencyMs: 5,
  encrypted: true,
  capabilities: {
    sql: true,
    transactions: true,
    cancellation: true,
    tableBrowsing: true,
    tableEditing: true,
    tableFiltering: true,
    processes: true,
  },
};

describe("Milestone 6 Phase 2 Challenger: Studio State & Query Ergonomics Adversarial Suite", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useStudio.setState({
      bootstrap: null,
      tabs: [],
      activeTabId: null,
      closedTabs: [],
      sessions: { "conn-test": mockSession },
      scratchpads: [],
    });
  });

  describe("1. Query Lifecycle Cleanup (statementErrors reset on started, activeQueryId reset on finished)", () => {
    it("resets statementErrors and activeQueryId immediately when started event arrives", () => {
      const tabId = useStudio.getState().openQuery("conn-test");
      let tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

      // Seed prior failure with statementErrors and active query handle
      const err: AppError = { code: "database", message: "Initial syntax failure" };
      useStudio.getState().applyEvents(
        tabId,
        [
          { type: "started", runId: "run-old" },
          { type: "queryHandle", runId: "run-old", statementIndex: 0, queryId: "sf-old-handle" },
          { type: "error", error: err, statementIndex: 0 },
          { type: "finished", runId: "run-old", status: "failed", elapsedMs: 10 },
        ],
        tab.revision,
      );

      tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
      expect(tab.status).toBe("failed");
      expect(tab.error?.message).toBe("Initial syntax failure");
      expect(tab.statementErrors?.[0]?.message).toBe("Initial syntax failure");

      // Now new query starts: 'started' event MUST wipe statementErrors, error, and activeQueryId
      useStudio
        .getState()
        .applyEvents(tabId, [{ type: "started", runId: "run-new" }], tab.revision);

      tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
      expect(tab.status).toBe("running");
      expect(tab.runId).toBe("run-new");
      expect(tab.activeQueryId).toBeNull();
      expect(tab.error).toBeNull();
      expect(tab.statementErrors).toEqual({});
      expect(Object.keys(tab.statementErrors || {}).length).toBe(0);
    });

    it("ensures activeQueryId is strictly reset to null upon finished event across all termination statuses", () => {
      const statuses: Array<"completed" | "failed" | "cancelled"> = [
        "completed",
        "failed",
        "cancelled",
      ];

      for (const st of statuses) {
        const tabId = useStudio.getState().openQuery("conn-test");
        let tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

        // Query receives query handle
        useStudio.getState().applyEvents(
          tabId,
          [
            { type: "started", runId: `run-${st}` },
            {
              type: "queryHandle",
              runId: `run-${st}`,
              statementIndex: 0,
              queryId: `sf-handle-${st}`,
            },
          ],
          tab.revision,
        );

        tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
        expect(tab.activeQueryId).toBe(`sf-handle-${st}`);

        // Query finishes with status
        useStudio
          .getState()
          .applyEvents(
            tabId,
            [{ type: "finished", runId: `run-${st}`, status: st, elapsedMs: 50 }],
            tab.revision,
          );

        tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
        expect(tab.status).toBe(st);
        expect(tab.runId).toBeNull();
        expect(tab.activeQueryId).toBeNull();
      }
    });

    it("populates tab.error with the lowest index statement error when multi-statement batch fails", () => {
      const tabId = useStudio.getState().openQuery("conn-test");
      const tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

      const errStmt3: AppError = { code: "database", message: "Error in statement 3" };
      const errStmt1: AppError = { code: "database", message: "Error in statement 1" };

      useStudio.getState().applyEvents(
        tabId,
        [
          { type: "started", runId: "multi-run" },
          { type: "error", error: errStmt3, statementIndex: 3 },
          { type: "error", error: errStmt1, statementIndex: 1 },
          { type: "finished", runId: "multi-run", status: "failed", elapsedMs: 40 },
        ],
        tab.revision,
      );

      const updatedTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
      // Stmt 1 has index 1 < index 3, so tab.error should be statement 1's error
      expect(updatedTab.error?.message).toBe("Error in statement 1");
      expect(updatedTab.statementErrors?.[1]?.message).toBe("Error in statement 1");
      expect(updatedTab.statementErrors?.[3]?.message).toBe("Error in statement 3");
    });
  });

  describe("2. save_scratchpad IPC payload format and persistence conformance", () => {
    it("dispatches save_scratchpad with conforming fields when creating and auto-saving scratchpads", async () => {
      const mockSavedScratchpad: Scratchpad = {
        id: "sp-test-101",
        name: "My Analytics Scratch",
        content: "SELECT * FROM sales;",
        connectionId: "conn-test",
        updatedAt: Date.now(),
      };
      (client.call as any).mockResolvedValueOnce(mockSavedScratchpad);

      // Create new scratchpad via createScratchpad
      useStudio.setState({ selectedConnectionId: "conn-test" });
      const created = await useStudio
        .getState()
        .createScratchpad("My Analytics Scratch", "SELECT * FROM sales;");

      expect(client.call).toHaveBeenCalledWith("save_scratchpad", {
        name: "My Analytics Scratch",
        content: "SELECT * FROM sales;",
        connectionId: "conn-test",
      });
      expect(created.id).toBe("sp-test-101");

      const inStore = useStudio.getState().scratchpads.find((s) => s.id === "sp-test-101");
      expect(inStore).toBeDefined();
      expect(inStore?.name).toBe("My Analytics Scratch");
    });

    it("serializes Scratchpad updatedAt as a safe JSON number", async () => {
      const nowMs = 1730000000123;
      const sp: Scratchpad = {
        id: "sp-num-1",
        name: "Number Test",
        content: "SELECT 1;",
        updatedAt: nowMs,
      };

      expect(typeof sp.updatedAt).toBe("number");
      expect(Number.isInteger(sp.updatedAt)).toBe(true);
      expect(JSON.stringify(sp)).toContain(`"updatedAt":${nowMs}`);
    });
  });

  describe("3. Dollar parameter extraction and UUID heuristic classification", () => {
    it("classifies parameter keys according to heuristic rules", () => {
      // UUID & GUID heuristics
      expect(inferParamKind("$user_uuid")).toBe("uuid");
      expect(inferParamKind(":client_guid")).toBe("uuid");
      expect(inferParamKind("$guid")).toBe("uuid");
      expect(inferParamKind(":uuid")).toBe("uuid");
      expect(inferParamKind("targetUuid")).toBe("uuid");

      // Integer heuristics
      expect(inferParamKind("$user_id")).toBe("integer");
      expect(inferParamKind(":account_id")).toBe("integer");
      expect(inferParamKind("orderId")).toBe("integer");
      expect(inferParamKind("$limit")).toBe("integer");
      expect(inferParamKind("$offset")).toBe("integer");
      expect(inferParamKind(":item_count")).toBe("integer");

      // Non-ID words ending in 'id' must NOT be classified as integer
      expect(inferParamKind("$grid")).toBe("text");
      expect(inferParamKind(":valid")).toBe("text");
      expect(inferParamKind("solid")).toBe("text");
      expect(inferParamKind("fluid")).toBe("text");
      expect(inferParamKind("pyramid")).toBe("text");
      expect(inferParamKind("squid")).toBe("text");

      // Decimal heuristics
      expect(inferParamKind("$unit_price")).toBe("decimal");
      expect(inferParamKind(":total_amount")).toBe("decimal");
      expect(inferParamKind("taxRate")).toBe("decimal");

      // Boolean heuristics
      expect(inferParamKind("$is_active")).toBe("boolean");
      expect(inferParamKind(":has_access")).toBe("boolean");
      expect(inferParamKind("enabled")).toBe("boolean");

      // Timestamp & Date
      expect(inferParamKind("$created_at")).toBe("timestamp");
      expect(inferParamKind(":birth_date")).toBe("date");

      // JSON
      expect(inferParamKind("$payload_json")).toBe("json");
      expect(inferParamKind(":metadata")).toBe("json");
    });

    it("formats display keys preserving dollar or colon prefixes", () => {
      expect(formatParamDisplayKey("$user_id")).toBe("$user_id");
      expect(formatParamDisplayKey("user_id")).toBe(":user_id");
      expect(formatParamDisplayKey("?")).toBe("?");
      expect(formatParamDisplayKey("plain_param")).toBe(":plain_param");
    });

    it("validates UUID drafts strictly rejecting malformed strings", () => {
      const validUuidDraft: QueryParamDraft = {
        kind: "uuid",
        value: "550e8400-e29b-41d4-a716-446655440000",
        isNull: false,
      };
      expect(validateParamDraft("$uuid", validUuidDraft)).toBeNull();

      // Invalid UUID length
      const invalidShort: QueryParamDraft = {
        kind: "uuid",
        value: "550e8400-e29b-41d4-a716-44665544000",
        isNull: false,
      };
      expect(validateParamDraft("$uuid", invalidShort)).toContain("must be a valid UUID");

      // Invalid UUID characters
      const invalidChars: QueryParamDraft = {
        kind: "uuid",
        value: "550e8400-e29b-41d4-a716-44665544000Z",
        isNull: false,
      };
      expect(validateParamDraft("$uuid", invalidChars)).toContain("must be a valid UUID");
    });

    it("validates 64-bit integer drafts rejecting overflows and non-integers", () => {
      // Valid positive i64
      expect(
        validateParamDraft("$int", {
          kind: "integer",
          value: "9223372036854775808",
          isNull: false,
        }),
      ).toContain("must fit within a signed 64-bit integer");

      // Valid negative i64
      expect(
        validateParamDraft("$int", {
          kind: "integer",
          value: "-9223372036854775808",
          isNull: false,
        }),
      ).toBeNull();

      // Non integer
      expect(
        validateParamDraft("$int", {
          kind: "integer",
          value: "123.45",
          isNull: false,
        }),
      ).toContain("must be a 64-bit integer");
    });

    it("detects and rejects NUL bytes across all parameter kinds", () => {
      const nulDraft: QueryParamDraft = {
        kind: "text",
        value: "hello\0world",
        isNull: false,
      };
      expect(validateParamDraft("$text", nulDraft)).toContain("cannot contain NUL bytes");
    });

    it("builds QueryParamInputs correctly and handles NULL values", () => {
      const slots = [
        { key: "$id", positional: false, occurrences: 1 },
        { key: "$status", positional: false, occurrences: 1 },
      ];
      const drafts: Record<string, QueryParamDraft> = {
        $id: { kind: "integer", value: "101", isNull: false },
        $status: { kind: "text", value: "OPEN", isNull: true },
      };

      const result = buildQueryParamInputs(slots, drafts);
      expect(result.error).toBeUndefined();
      expect(result.inputs).toHaveLength(2);
      expect(result.inputs![0]).toEqual({ key: "$id", kind: "integer", value: "101" });
      expect(result.inputs![1]).toEqual({ key: "$status", kind: "text", value: null });
    });
  });
});
