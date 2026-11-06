import { describe, expect, it } from "vitest";
import { evaluateJsonPath } from "./jsonpath";

describe("evaluateJsonPath", () => {
  const sample = {
    store: {
      book: [
        { title: "Snowflake Architecture", price: 59.99, active: true },
        { title: "Database Internals", price: 45.0, active: false },
        { title: "Designing Data-Intensive Applications", price: 39.99, active: true },
      ],
      bicycle: { color: "red", price: 199.95 },
    },
    version: 1,
  };

  it("evaluates root expression", () => {
    const res = evaluateJsonPath("$", sample);
    expect(res.ok).toBe(true);
    if (res.ok) {
      expect(res.matches).toHaveLength(1);
      expect(res.matches[0].path).toBe("$");
      expect(res.matches[0].value).toBe(sample);
    }
  });

  it("evaluates simple dot property", () => {
    const res = evaluateJsonPath("$.store.bicycle.color", sample);
    expect(res.ok).toBe(true);
    if (res.ok) {
      expect(res.matches).toHaveLength(1);
      expect(res.matches[0].path).toBe("$.store.bicycle.color");
      expect(res.matches[0].value).toBe("red");
    }
  });

  it("evaluates array indexing and negative index", () => {
    const res0 = evaluateJsonPath("$.store.book[0].title", sample);
    expect(res0.ok).toBe(true);
    if (res0.ok) {
      expect(res0.matches[0].value).toBe("Snowflake Architecture");
    }

    const resLast = evaluateJsonPath("$.store.book[-1].title", sample);
    expect(resLast.ok).toBe(true);
    if (resLast.ok) {
      expect(resLast.matches[0].value).toBe("Designing Data-Intensive Applications");
    }
  });

  it("evaluates array wildcard [*]", () => {
    const res = evaluateJsonPath("$.store.book[*].title", sample);
    expect(res.ok).toBe(true);
    if (res.ok) {
      expect(res.matches).toHaveLength(3);
      expect(res.matches.map((m) => m.value)).toEqual([
        "Snowflake Architecture",
        "Database Internals",
        "Designing Data-Intensive Applications",
      ]);
    }
  });

  it("evaluates recursive descent $..price", () => {
    const res = evaluateJsonPath("$..price", sample);
    expect(res.ok).toBe(true);
    if (res.ok) {
      expect(res.matches).toHaveLength(4);
      expect(res.matches.map((m) => m.value)).toEqual([59.99, 45.0, 39.99, 199.95]);
    }
  });

  it("evaluates array slice [0:2]", () => {
    const res = evaluateJsonPath("$.store.book[0:2].title", sample);
    expect(res.ok).toBe(true);
    if (res.ok) {
      expect(res.matches).toHaveLength(2);
      expect(res.matches.map((m) => m.value)).toEqual([
        "Snowflake Architecture",
        "Database Internals",
      ]);
    }
  });

  it("evaluates filter expressions [?(@.price < 50)] and [?(@.active == true)]", () => {
    const resPrice = evaluateJsonPath("$.store.book[?(@.price < 50)].title", sample);
    expect(resPrice.ok).toBe(true);
    if (resPrice.ok) {
      expect(resPrice.matches).toHaveLength(2);
      expect(resPrice.matches.map((m) => m.value)).toEqual([
        "Database Internals",
        "Designing Data-Intensive Applications",
      ]);
    }

    const resActive = evaluateJsonPath("$.store.book[?(@.active == true)].title", sample);
    expect(resActive.ok).toBe(true);
    if (resActive.ok) {
      expect(resActive.matches).toHaveLength(2);
      expect(resActive.matches.map((m) => m.value)).toEqual([
        "Snowflake Architecture",
        "Designing Data-Intensive Applications",
      ]);
    }
  });

  it("handles bracket notation with quotes", () => {
    const res = evaluateJsonPath("$['store']['bicycle']['price']", sample);
    expect(res.ok).toBe(true);
    if (res.ok) {
      expect(res.matches[0].value).toBe(199.95);
    }
  });

  it("returns error for invalid syntax", () => {
    const res1 = evaluateJsonPath("$.store.[*]", sample);
    expect(res1.ok).toBe(false);

    const res2 = evaluateJsonPath("$.store..", sample);
    expect(res2.ok).toBe(false);

    const res3 = evaluateJsonPath("$.store[unclosed", sample);
    expect(res3.ok).toBe(false);
  });
});
