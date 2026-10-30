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
    store.clear("a");
    expect(store.count).toBe(1);
  });
  it("rejects edits before exceeding the memory budget", () => {
    const store = new DirtyStore(200);
    expect(() =>
      store.set({ resultId: "r", row: 0, column: "c", original: "", value: "x".repeat(100) }),
    ).toThrow();
    expect(store.count).toBe(0);
  });
  it("applies batches atomically with a single notification and rolls back on budget overflow", () => {
    const store = new DirtyStore(600);
    let notifications = 0;
    store.subscribe(() => {
      notifications++;
    });
    store.setBatch([
      { resultId: "r", row: 0, column: "c0", original: "a", value: "b" },
      { resultId: "r", row: 0, column: "c1", original: "x", value: "y" },
      { resultId: "r", row: 0, column: "c2", original: "same", value: "same" },
    ]);
    expect(store.count).toBe(2);
    expect(notifications).toBe(1);

    // An oversized batch throws before mutating any cell or notifying listeners.
    expect(() =>
      store.setBatch([
        { resultId: "r", row: 1, column: "c0", original: "a", value: "ok" },
        { resultId: "r", row: 1, column: "c1", original: "a", value: "z".repeat(400) },
      ]),
    ).toThrow(/exceed 8 MiB/);
    expect(store.count).toBe(2);
    expect(store.get("r", 1, "c0")).toBeUndefined();
    expect(notifications).toBe(1);

    // Reverting existing dirty cells frees their byte budget within the same batch.
    store.setBatch([
      { resultId: "r", row: 0, column: "c0", original: "b", value: "a" },
      { resultId: "r", row: 1, column: "c0", original: "m", value: "n" },
    ]);
    expect(store.get("r", 0, "c0")).toBeUndefined();
    expect(store.get("r", 1, "c0")?.value).toBe("n");
    expect(notifications).toBe(2);
  });
});
