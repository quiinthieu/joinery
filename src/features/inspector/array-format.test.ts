import { describe, expect, it } from "vitest";
import {
  parseUniversalArray,
  filterArrayElements,
  exportArrayAsJson,
  exportArrayAsCsv,
} from "./array-format";

describe("array-format", () => {
  it("parses 1D JSON array strings", () => {
    const raw = '["production", "dba", "cloud"]';
    const res = parseUniversalArray(raw);
    expect(res.ok).toBe(true);
    expect(res.format).toBe("json");
    expect(res.totalElements).toBe(3);
    expect(res.elements[0].indexLabel).toBe("[0]");
    expect(res.elements[0].displayValue).toBe("production");
    expect(res.elements[0].kind).toBe("string");
    expect(res.elements[2].displayValue).toBe("cloud");
  });

  it("parses mixed type JSON arrays", () => {
    const raw = '[100, 2.5, true, null, "text", {"k": "v"}, [1, 2]]';
    const res = parseUniversalArray(raw);
    expect(res.ok).toBe(true);
    expect(res.elements).toHaveLength(7);
    expect(res.elements[0].kind).toBe("number");
    expect(res.elements[0].lengthOrSize).toBe("int");
    expect(res.elements[1].kind).toBe("number");
    expect(res.elements[1].lengthOrSize).toBe("float");
    expect(res.elements[2].kind).toBe("boolean");
    expect(res.elements[3].kind).toBe("null");
    expect(res.elements[3].displayValue).toBe("NULL");
    expect(res.elements[4].kind).toBe("string");
    expect(res.elements[5].kind).toBe("object");
    expect(res.elements[6].kind).toBe("array");
  });

  it("parses PostgreSQL 1D array literals", () => {
    const raw = '{"alpha", "beta", "gamma"}';
    const res = parseUniversalArray(raw);
    expect(res.ok).toBe(true);
    expect(res.format).toBe("postgres");
    expect(res.elements).toHaveLength(3);
    expect(res.elements[0].indexLabel).toBe("[1]");
    expect(res.elements[0].displayValue).toBe("alpha");
  });

  it("parses PostgreSQL 2D matrix literals", () => {
    const raw = "{{10, 20}, {30, 40}}";
    const res = parseUniversalArray(raw);
    expect(res.ok).toBe(true);
    expect(res.dimensions).toBe(2);
    expect(res.elements).toHaveLength(4);
    expect(res.elements[0].indexLabel).toBe("[1][1]");
    expect(res.elements[0].displayValue).toBe("10");
    expect(res.elements[3].indexLabel).toBe("[2][2]");
    expect(res.elements[3].displayValue).toBe("40");
  });

  it("filters array elements by search query", () => {
    const res = parseUniversalArray('["apple", "banana", "cherry", "avocado"]');
    expect(res.ok).toBe(true);

    const filtered = filterArrayElements(res.elements, "an");
    expect(filtered).toHaveLength(1);
    expect(filtered[0].displayValue).toBe("banana");

    const filteredIndex = filterArrayElements(res.elements, "[2]");
    expect(filteredIndex).toHaveLength(1);
    expect(filteredIndex[0].displayValue).toBe("cherry");
  });

  it("exports elements as JSON and CSV", () => {
    const res = parseUniversalArray('["hello", 42, null]');
    expect(res.ok).toBe(true);

    const json = exportArrayAsJson(res.elements);
    expect(json).toBe('[\n  "hello",\n  42,\n  null\n]');

    const csv = exportArrayAsCsv(res.elements);
    expect(csv).toContain('"Index","Type","Value","Size"');
    expect(csv).toContain('"[0]","string","hello","5 chars"');
    expect(csv).toContain('"[1]","number","42","int"');
    expect(csv).toContain('"[2]","null","NULL","—"');
  });
});
