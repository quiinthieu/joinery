import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  inferParamKind,
  validateParamDraft,
  formatParamDisplayKey,
  buildQueryParamInputs,
  type QueryParamDraft,
} from "./ParameterDialog";
import type { QueryParamSlot } from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import { client } from "../../ipc/client";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn().mockResolvedValue(undefined) },
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
  isTauri: () => false,
}));

describe("Adversarial M3: ParameterDialog Kind Inference & Validation", () => {
  it("infers expected types across tricky and edge-case parameter names", () => {
    // Integer inference heuristics
    expect(inferParamKind("id")).toBe("integer");
    expect(inferParamKind(":id")).toBe("integer");
    expect(inferParamKind("$id")).toBe("integer");
    expect(inferParamKind("user_id")).toBe("integer");
    expect(inferParamKind("$customer_id")).toBe("integer");
    expect(inferParamKind("limit")).toBe("integer");
    expect(inferParamKind("offset")).toBe("integer");
    expect(inferParamKind("retry_count")).toBe("integer");
    expect(inferParamKind("page_num")).toBe("integer");

    // Decimal heuristics
    expect(inferParamKind("total_amount")).toBe("decimal");
    expect(inferParamKind("unit_price")).toBe("decimal");
    expect(inferParamKind("discount_rate")).toBe("decimal");
    expect(inferParamKind("account_balance")).toBe("decimal");

    // Boolean heuristics
    expect(inferParamKind("is_admin")).toBe("boolean");
    expect(inferParamKind("has_permission")).toBe("boolean");
    expect(inferParamKind("can_write")).toBe("boolean");
    expect(inferParamKind("active")).toBe("boolean");
    expect(inferParamKind("enabled")).toBe("boolean");

    // UUID heuristics
    expect(inferParamKind("uuid_key")).toBe("uuid");
    expect(inferParamKind("user_uuid")).toBe("uuid");
    expect(inferParamKind("client_guid")).toBe("uuid");
    expect(inferParamKind(":uuid")).toBe("uuid");
    expect(inferParamKind("$guid")).toBe("uuid");
    expect(inferParamKind("userUuid")).toBe("uuid");
    expect(inferParamKind("userId")).toBe("integer");
    expect(inferParamKind("grid")).toBe("text");

    // Date & Timestamp heuristics
    expect(inferParamKind("order_date")).toBe("date");
    expect(inferParamKind("birthdate")).toBe("date");
    expect(inferParamKind("created_at")).toBe("timestamp");
    expect(inferParamKind("updated_time")).toBe("timestamp");

    // JSON heuristics
    expect(inferParamKind("raw_json")).toBe("json");
    expect(inferParamKind("event_payload")).toBe("json");
    expect(inferParamKind("user_metadata")).toBe("json");
    expect(inferParamKind("server_config")).toBe("json");

    // Fallbacks
    expect(inferParamKind("")).toBe("text");
    expect(inferParamKind("random_identifier")).toBe("text");
    expect(inferParamKind("unknown_field_xyz")).toBe("text");
  });

  it("validates SQL injection attempts: rejects them on typed parameters, preserves them safely on text parameters", () => {
    const injections = ["' OR '1'='1", "'; DROP TABLE users; --", "1 OR 1=1", "admin'--"];

    for (const injection of injections) {
      // 1. Text parameter: injection is allowed as text (will be safely bound via parameterized query)
      const textDraft: QueryParamDraft = { kind: "text", value: injection, isNull: false };
      expect(validateParamDraft("q", textDraft)).toBeNull();

      // 2. Integer parameter: injection must be strictly rejected by validation
      const intDraft: QueryParamDraft = { kind: "integer", value: injection, isNull: false };
      expect(validateParamDraft("id", intDraft)).toMatch(/must be a 64-bit integer/);

      // 3. Boolean parameter: injection must be rejected
      const boolDraft: QueryParamDraft = { kind: "boolean", value: injection, isNull: false };
      expect(validateParamDraft("flag", boolDraft)).toMatch(/must be true or false/);

      // 4. UUID parameter: injection must be rejected
      const uuidDraft: QueryParamDraft = { kind: "uuid", value: injection, isNull: false };
      expect(validateParamDraft("uid", uuidDraft)).toMatch(/must be a valid UUID/);

      // 5. Date parameter: injection must be rejected
      const dateDraft: QueryParamDraft = { kind: "date", value: injection, isNull: false };
      expect(validateParamDraft("dt", dateDraft)).toMatch(/must use YYYY-MM-DD format/);

      // 6. JSON parameter: injection must be rejected
      const jsonDraft: QueryParamDraft = { kind: "json", value: injection, isNull: false };
      expect(validateParamDraft("js", jsonDraft)).toMatch(/must be valid JSON/);
    }
  });

  it("strictly rejects NUL bytes across all parameter kinds", () => {
    const nulDraft: QueryParamDraft = { kind: "text", value: "hello\0world", isNull: false };
    expect(validateParamDraft("txt", nulDraft)).toMatch(/cannot contain NUL bytes/);
  });

  it("enforces signed 64-bit bounds on integer drafts", () => {
    const minI64 = "-9223372036854775808";
    const maxI64 = "9223372036854775807";
    const underflow = "-9223372036854775809";
    const overflow = "9223372036854775808";

    expect(validateParamDraft("id", { kind: "integer", value: minI64, isNull: false })).toBeNull();
    expect(validateParamDraft("id", { kind: "integer", value: maxI64, isNull: false })).toBeNull();
    expect(validateParamDraft("id", { kind: "integer", value: underflow, isNull: false })).toMatch(
      /must fit within a signed 64-bit integer/,
    );
    expect(validateParamDraft("id", { kind: "integer", value: overflow, isNull: false })).toMatch(
      /must fit within a signed 64-bit integer/,
    );
  });

  it("formats display keys for named colons, dollar parameters, and positional marks", () => {
    expect(formatParamDisplayKey("username")).toBe(":username");
    expect(formatParamDisplayKey("$user_id")).toBe("$user_id");
    expect(formatParamDisplayKey("?1")).toBe("?1");
  });

  it("buildQueryParamInputs maps null flags correctly and catches validation errors", () => {
    const slots: QueryParamSlot[] = [
      { key: "p1", positional: false, occurrences: 1 },
      { key: "p2", positional: false, occurrences: 1 },
    ];

    const drafts: Record<string, QueryParamDraft> = {
      p1: { kind: "text", value: "test", isNull: false },
      p2: { kind: "integer", value: "", isNull: true },
    };

    const res = buildQueryParamInputs(slots, drafts);
    expect(res.error).toBeUndefined();
    expect(res.inputs).toEqual([
      { key: "p1", kind: "text", value: "test" },
      { key: "p2", kind: "integer", value: null },
    ]);
  });
});

describe("Adversarial M3: Scratchpad IPC Contract Mismatch & Auto-Save Bugs", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("demonstrates the IPC call shape emitted by studio.ts createScratchpad", async () => {
    const mockCall = vi.mocked(client.call);
    mockCall.mockResolvedValueOnce({
      id: "sp-created-1",
      name: "Scratchpad 1",
      content: "-- Scratchpad\n",
      updatedAt: 123456789,
    });

    const created = await useStudio.getState().createScratchpad("Scratchpad 1", "SELECT 1;");

    // Observe exact arguments passed to client.call
    expect(mockCall).toHaveBeenCalledWith("save_scratchpad", {
      name: "Scratchpad 1",
      content: "SELECT 1;",
      connectionId: undefined,
    });

    // BUG MANIFESTATION:
    // Rust commands.rs signature: `pub async fn save_scratchpad(studio: ..., scratchpad: Scratchpad)`
    // Rust test-harness.rs signature: `"save_scratchpad" => json!(studio.save_scratchpad(arg(&args, "scratchpad")?)?)`
    // The frontend call sends `{ name, content, connectionId }` WITHOUT:
    // 1. Wrapping it in a `scratchpad` key
    // 2. Supplying `id` (Scratchpad.id is non-optional in Rust)
    // 3. Supplying `updatedAt` (Scratchpad.updated_at is non-optional in Rust)
    expect(created.id).toBe("sp-created-1");
  });

  it("demonstrates the IPC call shape emitted by studio.ts renameScratchpad", async () => {
    const mockCall = vi.mocked(client.call);
    // Seed scratchpads in state
    useStudio.setState({
      scratchpads: [
        {
          id: "sp-rename-target",
          name: "Old Name",
          content: "SELECT 42;",
          updatedAt: 1000,
        },
      ],
    });

    mockCall.mockResolvedValueOnce({
      id: "sp-rename-target",
      name: "Renamed Scratchpad",
      content: "SELECT 42;",
      updatedAt: 2000,
    });

    await useStudio.getState().renameScratchpad("sp-rename-target", "Renamed Scratchpad");

    // Observe exact arguments passed:
    expect(mockCall).toHaveBeenCalledWith("save_scratchpad", {
      id: "sp-rename-target",
      name: "Renamed Scratchpad",
      content: "SELECT 42;",
      connectionId: undefined,
    });
  });

  it("demonstrates the IPC call shape emitted by debounced auto-save in updateTab", async () => {
    vi.useFakeTimers();
    const mockCall = vi.mocked(client.call);

    // Open a tab linked to scratchpad
    const tabId = useStudio.getState().openQuery("conn-1");
    useStudio.getState().updateTab(tabId, {
      scratchpadId: "sp-auto-target",
      sql: "-- Original",
    });

    // Update tab SQL
    useStudio.getState().updateTab(tabId, { sql: "SELECT 'updated_content';" });

    // Advance timer past debounce (400ms)
    vi.advanceTimersByTime(500);

    // Observe what auto-save invoked
    expect(mockCall).toHaveBeenCalledWith("save_scratchpad", {
      id: "sp-auto-target",
      name: "Query 2",
      content: "SELECT 'updated_content';",
      connectionId: "conn-1",
    });
    vi.useRealTimers();
  });
});
