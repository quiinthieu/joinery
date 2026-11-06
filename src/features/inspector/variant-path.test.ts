import { describe, expect, it } from "vitest";
import { generateSnowflakePath, inferDefaultSnowflakeCast } from "./variant-path";

describe("variant-path", () => {
  it("generates Snowflake colon and dot path with inferred cast", () => {
    const expr = generateSnowflakePath({
      columnName: "payload_json",
      path: ["store", "book", 0, "price"],
      valueType: "number",
    });
    expect(expr).toBe("payload_json:store.book[0].price::number");
  });

  it("generates Snowflake string path with ::string cast", () => {
    const expr = generateSnowflakePath({
      columnName: "payload_json",
      path: ["store", "book", 0, "title"],
      valueType: "string",
    });
    expect(expr).toBe("payload_json:store.book[0].title::string");
  });

  it("handles top-level array indexing", () => {
    const expr = generateSnowflakePath({
      columnName: "tags_array",
      path: [0],
      valueType: "string",
    });
    expect(expr).toBe("tags_array[0]::string");
  });

  it("quotes identifiers with spaces or special characters", () => {
    const expr = generateSnowflakePath({
      columnName: "payload_json",
      path: ["user address", "zip-code"],
      valueType: "string",
    });
    expect(expr).toBe('payload_json:"user address"."zip-code"::string');
  });

  it("supports bracket style syntax", () => {
    const expr = generateSnowflakePath({
      columnName: "payload_json",
      path: ["store", "book", 0, "price"],
      valueType: "number",
      style: "bracket",
    });
    expect(expr).toBe("payload_json['store']['book'][0]['price']::number");
  });

  it("supports custom cast override and (No cast)", () => {
    const expr1 = generateSnowflakePath({
      columnName: "payload_json",
      path: ["created_date"],
      cast: "date",
    });
    expect(expr1).toBe("payload_json:created_date::date");

    const exprNone = generateSnowflakePath({
      columnName: "payload_json",
      path: ["raw_variant"],
      cast: "none",
    });
    expect(exprNone).toBe("payload_json:raw_variant");
  });

  it("quotes SQL reserved keywords in colon and dot syntax (case-insensitive)", () => {
    expect(
      generateSnowflakePath({
        columnName: "payload",
        path: ["SELECT"],
        valueType: "string",
      }),
    ).toBe('payload:"SELECT"::string');

    expect(
      generateSnowflakePath({
        columnName: "payload",
        path: ["from"],
        valueType: "string",
      }),
    ).toBe('payload:"from"::string');

    expect(
      generateSnowflakePath({
        columnName: "payload",
        path: ["store", "order", 0, "limit"],
        valueType: "number",
      }),
    ).toBe('payload:store."order"[0]."limit"::number');
  });

  it("properly escapes single quotes and backslashes in bracket syntax", () => {
    expect(
      generateSnowflakePath({
        columnName: "col",
        path: ["user'name"],
        style: "bracket",
        valueType: "string",
      }),
    ).toBe("col['user''name']::string");

    expect(
      generateSnowflakePath({
        columnName: "col",
        path: ["dir\\file"],
        style: "bracket",
        valueType: "string",
      }),
    ).toBe("col['dir\\\\file']::string");

    expect(
      generateSnowflakePath({
        columnName: "col",
        path: ["folder\\"],
        style: "bracket",
        valueType: "string",
      }),
    ).toBe("col['folder\\\\']::string");
  });

  it("infers casts for int, float, date, and timestamp types", () => {
    expect(inferDefaultSnowflakeCast("int")).toBe("int");
    expect(inferDefaultSnowflakeCast("integer")).toBe("int");
    expect(inferDefaultSnowflakeCast("float")).toBe("float");
    expect(inferDefaultSnowflakeCast("date")).toBe("date");
    expect(inferDefaultSnowflakeCast("timestamp")).toBe("timestamp_ntz");
    expect(inferDefaultSnowflakeCast("timestamp_tz")).toBe("timestamp_tz");
  });
});
