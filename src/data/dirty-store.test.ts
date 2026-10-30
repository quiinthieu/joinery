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

  it("tracks staged row deletes disjoint from cell updates and manages draft insert rows", () => {
    const store = new DirtyStore();
    store.set({ resultId: "p1", row: 2, column: "c1", original: "old", value: "edited" });
    expect(store.pendingCountForTab("tab-1", ["p1"])).toBe(1);

    // Deleting row 2 drops its cell edit and blocks further edits until undeleted.
    store.setDeleted("p1", [2, 0], true);
    expect(store.get("p1", 2, "c1")).toBeUndefined();
    expect(store.deletesForResult("p1")).toEqual([0, 2]);
    expect(store.hasResult("p1")).toBe(true);
    expect(() =>
      store.set({ resultId: "p1", row: 2, column: "c1", original: "old", value: "blocked" }),
    ).toThrow(/Undo deletion/);

    store.setDeleted("p1", [2], false);
    expect(store.deletesForResult("p1")).toEqual([0]);

    // Draft insert rows support explicit values, NULL, and reverting to <default> (undefined).
    const draft = store.addInsertRow("tab-1");
    store.setInsertCell("tab-1", draft.id, "c0", "1001");
    store.setInsertCell("tab-1", draft.id, "c2", null);
    expect(store.insertsForTab("tab-1")[0].values).toEqual({ c0: "1001", c2: null });
    store.setInsertCell("tab-1", draft.id, "c2", undefined);
    expect("c2" in store.insertsForTab("tab-1")[0].values).toBe(false);
    expect(store.pendingCountForTab("tab-1", ["p1"])).toBe(2);

    store.clearTab("tab-1", ["p1"]);
    expect(store.pendingCountForTab("tab-1", ["p1"])).toBe(0);
    expect(store.count).toBe(0);
  });
});
