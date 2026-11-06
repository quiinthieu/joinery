import { describe, expect, it, vi, beforeEach } from "vitest";
import { inferParamKind } from "./ParameterDialog";
import { useStudio } from "../../state/studio";
import { client } from "../../ipc/client";
import type { Scratchpad } from "../../ipc/generated";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn().mockResolvedValue(undefined) },
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
  isTauri: () => false,
}));

describe("Empirical Challenger M3-IT2: Parameter UUID & Identifier Heuristics", () => {
  it("strictly infers expected kind for all 9 required dispatch test cases", () => {
    // 1. :user_uuid -> uuid
    expect(inferParamKind(":user_uuid")).toBe("uuid");

    // 2. $client_guid -> uuid
    expect(inferParamKind("$client_guid")).toBe("uuid");

    // 3. userUuid -> uuid
    expect(inferParamKind("userUuid")).toBe("uuid");

    // 4. :uuid -> uuid
    expect(inferParamKind(":uuid")).toBe("uuid");

    // 5. $guid -> uuid
    expect(inferParamKind("$guid")).toBe("uuid");

    // 6. userId -> integer
    expect(inferParamKind("userId")).toBe("integer");

    // 7. user_id -> integer
    expect(inferParamKind("user_id")).toBe("integer");

    // 8. grid -> text
    expect(inferParamKind("grid")).toBe("text");

    // 9. valid -> text
    expect(inferParamKind("valid")).toBe("text");
  });

  it("adversarially stress-tests boundary cases between UUID, ID, boolean, and text", () => {
    // UUID / GUID variations
    expect(inferParamKind("UUID")).toBe("uuid");
    expect(inferParamKind("GUID")).toBe("uuid");
    expect(inferParamKind("account_uuid_v4")).toBe("uuid");
    expect(inferParamKind("device_guid_str")).toBe("uuid");

    // Substrings with 'id' vs 'uuid'
    expect(inferParamKind("guid_id")).toBe("uuid"); // contains guid -> uuid takes precedence over _id
    expect(inferParamKind("uuid_id")).toBe("uuid"); // contains uuid -> uuid takes precedence over _id

    // ID variations (integer)
    expect(inferParamKind("id")).toBe("integer");
    expect(inferParamKind(":id")).toBe("integer");
    expect(inferParamKind("$id")).toBe("integer");
    expect(inferParamKind("accountId")).toBe("integer");
    expect(inferParamKind("account_id")).toBe("integer");
    expect(inferParamKind("order_id")).toBe("integer");
    expect(inferParamKind("val_id")).toBe("integer"); // ends with _id -> integer

    // Non-ID words ending in 'id' (text)
    expect(inferParamKind("grid")).toBe("text");
    expect(inferParamKind("valid")).toBe("text");
    expect(inferParamKind("solid")).toBe("text");
    expect(inferParamKind("fluid")).toBe("text");
    expect(inferParamKind("pyramid")).toBe("text");
    expect(inferParamKind("squid")).toBe("text"); // wait! squid contains 'id' but not uuid/guid or _id!

    // Boolean vs text for 'valid'
    expect(inferParamKind("is_valid")).toBe("boolean");
    expect(inferParamKind("has_valid_token")).toBe("boolean");
    expect(inferParamKind("valid")).toBe("text");
  });
});

describe("Empirical Challenger M3-IT2: Scratchpad IPC Contract & Serialization", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("verifies Scratchpad updatedAt serializes as a JSON number without BigInt errors", () => {
    const sp: Scratchpad = {
      id: "sp-serial-test-1",
      name: "Serialization Test",
      content: "SELECT 1;",
      connectionId: "conn-1",
      updatedAt: 1730000000000,
    };

    // Ensure updatedAt is a JS number
    expect(typeof sp.updatedAt).toBe("number");
    expect(Number.isSafeInteger(sp.updatedAt)).toBe(true);

    // JSON.stringify must succeed without throwing TypeError: Do not know how to serialize a BigInt
    let serialized = "";
    expect(() => {
      serialized = JSON.stringify(sp);
    }).not.toThrow();

    expect(serialized).toContain('"updatedAt":1730000000000');

    // Deserialization check
    const parsed = JSON.parse(serialized) as Scratchpad;
    expect(parsed.updatedAt).toBe(1730000000000);
    expect(typeof parsed.updatedAt).toBe("number");
  });

  it("verifies auto-save in studio store preserves scratchpad name and connection ID", async () => {
    vi.useFakeTimers();
    const mockCall = vi.mocked(client.call);

    // Seed scratchpads in state
    useStudio.setState({
      scratchpads: [
        {
          id: "sp-autosave-preserve",
          name: "Original Scratchpad Name",
          content: "SELECT 1;",
          connectionId: "conn-preserve-42",
          updatedAt: 1000,
        },
      ],
    });

    // Open a query tab linked to this scratchpad
    const tabId = useStudio.getState().openQuery("conn-preserve-42");
    useStudio.getState().updateTab(tabId, {
      scratchpadId: "sp-autosave-preserve",
      title: "Some Tab Title",
      sql: "SELECT 1;",
    });

    mockCall.mockResolvedValueOnce({
      id: "sp-autosave-preserve",
      name: "Original Scratchpad Name",
      content: "SELECT 2; -- auto saved",
      connectionId: "conn-preserve-42",
      updatedAt: 2000,
    });

    // Trigger SQL content update in tab
    useStudio.getState().updateTab(tabId, {
      sql: "SELECT 2; -- auto saved",
    });

    // Fast-forward past the 400ms debounce
    vi.advanceTimersByTime(450);

    // Verify client.call was invoked with the original scratchpad name and connection ID
    expect(mockCall).toHaveBeenCalledWith("save_scratchpad", {
      id: "sp-autosave-preserve",
      name: "Original Scratchpad Name",
      content: "SELECT 2; -- auto saved",
      connectionId: "conn-preserve-42",
    });

    // Verify state updated properly after promise resolution
    await Promise.resolve();
    const updated = useStudio.getState().scratchpads.find((s) => s.id === "sp-autosave-preserve");
    expect(updated?.content).toBe("SELECT 2; -- auto saved");
    expect(updated?.name).toBe("Original Scratchpad Name");
    expect(updated?.connectionId).toBe("conn-preserve-42");

    vi.useRealTimers();
  });
});
