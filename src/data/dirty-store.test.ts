import { describe, expect, it } from "vitest";
import { DirtyStore } from "./dirty-store";

describe("dirty cell tracking", () => {
  it("keeps NULL separate from empty text and removes reverted edits", () => {
    const store = new DirtyStore();
    store.set({ resultId: "r", row: 1, column: "c", original: null, value: "" });
    expect(store.count).toBe(1);
    store.set({ resultId: "r", row: 1, column: "c", original: "", value: null });
    expect(store.count).toBe(0);
  });
  it("preserves original values across further edits and isolates result identities", () => {
    const store = new DirtyStore();
    store.set({ resultId: "a", row: 0, column: "c", original: "first", value: "second" });
    store.set({ resultId: "a", row: 0, column: "c", original: "second", value: "third" });
    store.set({ resultId: "b", row: 0, column: "c", original: "other", value: "value" });
    expect(store.get("a", 0, "c")?.original).toBe("first");
    store.clear("a"); expect(store.count).toBe(1);
  });
  it("rejects edits before exceeding the memory budget", () => {
    const store = new DirtyStore(200);
    expect(() => store.set({ resultId: "r", row: 0, column: "c", original: "", value: "x".repeat(100) })).toThrow();
    expect(store.count).toBe(0);
  });
});
