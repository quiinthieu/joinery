import { describe, expect, it } from "vitest";
import { CompletionIndex } from "./completion-index";

describe("completion prefix index", () => {
  it("caps suggestions and preserves identifier spelling in a large catalog", () => {
    const index = new CompletionIndex();
    index.replace(Array.from({ length: 100000 }, (_, i) => ({ label: `Order_${String(i).padStart(6, "0")}`, type: "type" })));
    const suggestions = index.prefix("ORDER_000", 20);
    expect(suggestions).toHaveLength(20); expect(suggestions[0].label).toBe("Order_000000");
    expect(index.prefix("absent")).toHaveLength(0);
  });
});
