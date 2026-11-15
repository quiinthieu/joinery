import { describe, expect, it } from "vitest";
import {
  buildJsonTree,
  decodeBinaryPreview,
  formatHexDump,
  formatJsonLossless,
  isBinaryColumn,
  isJsonColumn,
  isPostgresArrayColumn,
  isRawHexBinaryType,
  isSpatialColumn,
  opensInValueDialog,
  parsePostgresArray,
  parseSpatialPreview,
} from "./value-format";

describe("column viewer classification", () => {
  it("classifies binary, PostgreSQL array, and JSON columns accurately", () => {
    expect(isBinaryColumn({ kind: "binary", databaseType: "bytea" })).toBe(true);
    expect(isPostgresArrayColumn({ kind: "unknown", databaseType: "text[]" })).toBe(true);
    expect(
      isPostgresArrayColumn({ kind: "unknown", databaseType: "character varying(32)[]" }),
    ).toBe(true);
    expect(isPostgresArrayColumn({ kind: "unknown", databaseType: "_int4" })).toBe(true);
    expect(isPostgresArrayColumn({ kind: "text", databaseType: "text" })).toBe(false);

    // `jsonb[]` is a PostgreSQL array of JSON documents, not a scalar JSON column.
    expect(isPostgresArrayColumn({ kind: "unknown", databaseType: "jsonb[]" })).toBe(true);
    expect(isJsonColumn({ kind: "json", databaseType: "jsonb[]" })).toBe(false);
    expect(isJsonColumn({ kind: "json", databaseType: "jsonb" })).toBe(true);

    expect(opensInValueDialog({ kind: "binary", databaseType: "bytea" })).toBe(true);
    expect(opensInValueDialog({ kind: "unknown", databaseType: "integer[]" })).toBe(true);
    expect(opensInValueDialog({ kind: "json", databaseType: "jsonb" })).toBe(true);
    expect(opensInValueDialog({ kind: "text", databaseType: "text" })).toBe(false);
  });
});

describe("decodeBinaryPreview and formatHexDump", () => {
  it("decodes PostgreSQL \\x hex, MySQL 0x hex, and empty binary values", () => {
    const emptyPg = decodeBinaryPreview("\\x");
    expect(emptyPg.totalBytes).toBe(0);
    expect(emptyPg.bytes).toHaveLength(0);
    expect(emptyPg.encoding).toBe("hex");
    expect(formatHexDump(emptyPg.bytes)).toBe("");

    const mysql = decodeBinaryPreview("0x4a6f696e657279");
    expect(mysql.totalBytes).toBe(7);
    expect(mysql.truncated).toBe(false);
    expect(formatHexDump(mysql.bytes)).toBe(
      "00000000  4a 6f 69 6e 65 72 79                              |Joinery|",
    );

    const pngHeaderAndText = decodeBinaryPreview(
      "\\x89504e470d0a1a0a48656c6c6f00ff214a6f696e657279",
    );
    expect(pngHeaderAndText.totalBytes).toBe(23);
    expect(formatHexDump(pngHeaderAndText.bytes)).toBe(
      [
        "00000000  89 50 4e 47 0d 0a 1a 0a  48 65 6c 6c 6f 00 ff 21  |.PNG....Hello..!|",
        "00000010  4a 6f 69 6e 65 72 79                              |Joinery|",
      ].join("\n"),
    );
  });

  it("decodes PostgreSQL escape bytea and bounds preview allocation while keeping exact byte count", () => {
    const escaped = decodeBinaryPreview("Hi\\000\\\\\\377!");
    expect(escaped.encoding).toBe("escaped");
    expect(escaped.totalBytes).toBe(6);
    expect(Array.from(escaped.bytes)).toEqual([0x48, 0x69, 0x00, 0x5c, 0xff, 0x21]);

    const bounded = decodeBinaryPreview(`\\x${"ab".repeat(40)}`, 16);
    expect(bounded.totalBytes).toBe(40);
    expect(bounded.bytes).toHaveLength(16);
    expect(bounded.truncated).toBe(true);
  });
});

describe("parsePostgresArray", () => {
  it("parses empty, 1D, multi-dimensional, and custom-lower-bound arrays with NULL distinction", () => {
    expect(parsePostgresArray("{}")).toEqual({
      ok: true,
      elements: [],
      totalElements: 0,
      dimensions: 0,
      truncated: false,
    });

    const oneDim = parsePostgresArray(
      '{alpha,"beta, gamma",NULL,"NULL","","quote\\"back\\\\slash"}',
    );
    expect(oneDim.ok).toBe(true);
    if (!oneDim.ok) return;
    expect(oneDim.dimensions).toBe(1);
    expect(oneDim.totalElements).toBe(6);
    expect(oneDim.elements).toEqual([
      { path: [1], indexLabel: "[1]", value: "alpha", isNull: false },
      { path: [2], indexLabel: "[2]", value: "beta, gamma", isNull: false },
      { path: [3], indexLabel: "[3]", value: null, isNull: true },
      { path: [4], indexLabel: "[4]", value: "NULL", isNull: false },
      { path: [5], indexLabel: "[5]", value: "", isNull: false },
      { path: [6], indexLabel: "[6]", value: 'quote"back\\slash', isNull: false },
    ]);

    const twoDim = parsePostgresArray("{{10,20},{30,NULL}}");
    expect(twoDim.ok).toBe(true);
    if (!twoDim.ok) return;
    expect(twoDim.dimensions).toBe(2);
    expect(twoDim.elements.map((e) => [e.indexLabel, e.value, e.isNull])).toEqual([
      ["[1][1]", "10", false],
      ["[1][2]", "20", false],
      ["[2][1]", "30", false],
      ["[2][2]", null, true],
    ]);

    const customBounds = parsePostgresArray("[0:1][-1:0]={{a,b},{c,d}}");
    expect(customBounds.ok).toBe(true);
    if (!customBounds.ok) return;
    expect(customBounds.elements.map((e) => [e.indexLabel, e.value])).toEqual([
      ["[0][-1]", "a"],
      ["[0][0]", "b"],
      ["[1][-1]", "c"],
      ["[1][0]", "d"],
    ]);
  });

  it("truncates preview past maxElements and rejects malformed array literals", () => {
    const truncated = parsePostgresArray("{a,b,c,d}", 2);
    expect(truncated.ok).toBe(true);
    if (truncated.ok) {
      expect(truncated.totalElements).toBe(4);
      expect(truncated.elements).toHaveLength(2);
      expect(truncated.truncated).toBe(true);
    }

    expect(parsePostgresArray("not-an-array").ok).toBe(false);
    expect(parsePostgresArray("{1,2").ok).toBe(false);
    expect(parsePostgresArray("{1,}").ok).toBe(false);
    expect(parsePostgresArray("{1,{2,3}}").ok).toBe(false);
    expect(parsePostgresArray("{1,2} trailing").ok).toBe(false);
  });
});

describe("formatJsonLossless", () => {
  it("pretty-prints JSON without losing 64-bit integer precision or reordering integer keys", () => {
    const formatted = formatJsonLossless(
      '{"2":"second","1":"first","maxBigint":9223372036854775807,"decimal":12.34000000,"emptyObj":{},"emptyArr":[],"nested":{"flags":[true,false,null],"note":"a:b, {c}"}}',
    );
    expect(formatted).toEqual({
      ok: true,
      formatted: [
        "{",
        '  "2": "second",',
        '  "1": "first",',
        '  "maxBigint": 9223372036854775807,',
        '  "decimal": 12.34000000,',
        '  "emptyObj": {},',
        '  "emptyArr": [],',
        '  "nested": {',
        '    "flags": [',
        "      true,",
        "      false,",
        "      null",
        "    ],",
        '    "note": "a:b, {c}"',
        "  }",
        "}",
      ].join("\n"),
    });
  });

  it("rejects invalid JSON syntax", () => {
    const invalid = formatJsonLossless('{"broken": }');
    expect(invalid.ok).toBe(false);
  });
});

describe("Snowflake semi-structured, binary, and spatial value inspectors", () => {
  it("classifies Snowflake VARIANT/OBJECT/ARRAY, BINARY/VARBINARY, and GEOGRAPHY/GEOMETRY columns", () => {
    expect(isJsonColumn({ kind: "text", databaseType: "VARIANT" })).toBe(true);
    expect(isJsonColumn({ kind: "text", databaseType: "OBJECT" })).toBe(true);
    expect(isJsonColumn({ kind: "text", databaseType: "ARRAY" })).toBe(true);
    expect(isBinaryColumn({ kind: "text", databaseType: "BINARY(16)" })).toBe(true);
    expect(isRawHexBinaryType("VARBINARY")).toBe(true);
    expect(isRawHexBinaryType("bytea")).toBe(false);
    expect(isSpatialColumn({ kind: "text", databaseType: "GEOGRAPHY" })).toBe(true);
    expect(isSpatialColumn({ kind: "text", databaseType: "GEOMETRY" })).toBe(true);
    expect(opensInValueDialog({ kind: "text", databaseType: "GEOGRAPHY" })).toBe(true);
  });

  it("decodes Snowflake raw even-length hex when allowRawHex is enabled", () => {
    const sfHex = decodeBinaryPreview("4A6F696E657279", 64, true);
    expect(sfHex.encoding).toBe("hex");
    expect(sfHex.totalBytes).toBe(7);
    expect(formatHexDump(sfHex.bytes)).toBe(
      "00000000  4a 6f 69 6e 65 72 79                              |Joinery|",
    );
  });

  it("builds lossless Snowflake :path trees and bounds large documents", () => {
    const tree = buildJsonTree(
      '{"customer":{"orders":[{"id":9223372036854775807}],"odd.key":"ok"}}',
      "PAYLOAD",
    );
    expect(tree.ok).toBe(true);
    if (!tree.ok) return;
    expect(tree.truncated).toBe(false);
    expect(tree.nodes.map((n) => [n.path, n.kind, n.summary])).toEqual([
      ["PAYLOAD", "object", "{1 key}"],
      ["PAYLOAD:customer", "object", "{2 keys}"],
      ["PAYLOAD:customer.orders", "array", "[1 item]"],
      ["PAYLOAD:customer.orders[0]", "object", "{1 key}"],
      ["PAYLOAD:customer.orders[0].id", "number", "9223372036854775807"],
      ["PAYLOAD:customer['odd.key']", "string", '"ok"'],
    ]);

    const bounded = buildJsonTree("[1,2,3,4,5]", "ARR_COL", 3);
    expect(bounded.ok).toBe(true);
    if (bounded.ok) {
      expect(bounded.totalNodes).toBe(6);
      expect(bounded.nodes).toHaveLength(3);
      expect(bounded.truncated).toBe(true);
      expect(bounded.nodes[0].summary).toBe("[5 items]");
    }
  });

  it("parses GeoJSON and EWKT/WKT spatial values into normalized 2D SVG shapes", () => {
    const geojson = parseSpatialPreview(
      '{"type":"Polygon","crs":{"type":"name","properties":{"name":"EPSG:4326"}},"coordinates":[[[-122.5,37.7],[-122.3,37.7],[-122.3,37.8],[-122.5,37.8],[-122.5,37.7]]]}',
    );
    expect(geojson.ok).toBe(true);
    if (geojson.ok) {
      expect(geojson.format).toBe("GeoJSON");
      expect(geojson.geometryType).toBe("Polygon");
      expect(geojson.srid).toBe(4326);
      expect(geojson.coordinateCount).toBe(5);
      expect(geojson.bbox).toEqual([-122.5, 37.7, -122.3, 37.8]);
      expect(geojson.shapes).toHaveLength(1);
      expect(geojson.shapes[0].kind).toBe("polygon");
      expect(geojson.shapes[0].d).toContain("Z");
    }

    const ewkt = parseSpatialPreview("SRID=3857;LINESTRING(0 0, 10 20, 30 15)");
    expect(ewkt.ok).toBe(true);
    if (ewkt.ok) {
      expect(ewkt.format).toBe("EWKT");
      expect(ewkt.geometryType).toBe("LINESTRING");
      expect(ewkt.srid).toBe(3857);
      expect(ewkt.coordinateCount).toBe(3);
      expect(ewkt.bbox).toEqual([0, 0, 30, 20]);
      expect(ewkt.shapes[0].kind).toBe("polyline");
    }
  });
});
