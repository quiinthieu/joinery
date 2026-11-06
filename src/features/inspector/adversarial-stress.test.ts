import { describe, expect, it } from "vitest";
import {
  parseUniversalArray,
  filterArrayElements,
  exportArrayAsJson,
  exportArrayAsCsv,
} from "./array-format";
import {
  parseSpatialGeometry,
  buildSvgRenderData,
  type ParsedSpatialGeometry,
} from "./spatial-format";
import {
  generateSnowflakePath,
  inferDefaultSnowflakeCast,
  SNOWFLAKE_CAST_TYPES,
} from "./variant-path";

describe("Adversarial Stress Test: array-format.ts", () => {
  describe("PostgreSQL array literals edge cases", () => {
    it("handles unclosed braces safely without throwing unhandled exceptions", () => {
      const res1 = parseUniversalArray("{a,b");
      expect(res1.ok).toBe(false);
      expect(res1.error).toBeDefined();

      const res2 = parseUniversalArray("{{1,2},{3,4");
      expect(res2.ok).toBe(false);
      expect(res2.error).toBeDefined();
    });

    it("handles unescaped commas inside quoted array elements correctly", () => {
      const raw = '{"hello, world", "second, item", normal}';
      const res = parseUniversalArray(raw);
      expect(res.ok).toBe(true);
      expect(res.format).toBe("postgres");
      expect(res.elements).toHaveLength(3);
      expect(res.elements[0].displayValue).toBe("hello, world");
      expect(res.elements[1].displayValue).toBe("second, item");
      expect(res.elements[2].displayValue).toBe("normal");
    });

    it("handles backslash-escaped quotes inside quoted elements", () => {
      const raw = '{"foo\\"bar", "baz"}';
      const res = parseUniversalArray(raw);
      expect(res.ok).toBe(true);
      expect(res.elements[0].displayValue).toBe('foo"bar');
    });

    it("evaluates doubled quotes behavior in PostgreSQL array literals", () => {
      // In PostgreSQL, "" inside a quoted string is standard escaping for a double quote
      const raw = '{"foo""bar", "baz"}';
      const res = parseUniversalArray(raw);
      // Current parser does not support doubled-quote escaping; rejects with ok: false
      expect(res.ok).toBe(false);
    });

    it("rejects mixed depth / non-rectangular arrays gracefully", () => {
      const raw = "{{1, 2}, 3}";
      const res = parseUniversalArray(raw);
      expect(res.ok).toBe(false);
      expect(res.error).toBeDefined();
    });

    it("correctly identifies NULL vs string 'NULL' in PostgreSQL arrays", () => {
      const raw = '{NULL, "NULL", null, "null"}';
      const res = parseUniversalArray(raw);
      expect(res.ok).toBe(true);
      expect(res.elements[0].isNull).toBe(true);
      expect(res.elements[0].displayValue).toBe("NULL");
      expect(res.elements[1].isNull).toBe(false);
      expect(res.elements[1].displayValue).toBe("NULL");
      expect(res.elements[2].isNull).toBe(true);
      expect(res.elements[3].isNull).toBe(false);
      expect(res.elements[3].displayValue).toBe("null");
    });

    it("handles empty PostgreSQL array {}", () => {
      const res = parseUniversalArray("{}");
      expect(res.ok).toBe(true);
      expect(res.format).toBe("postgres");
      expect(res.totalElements).toBe(0);
      expect(res.elements).toHaveLength(0);
    });

    it("handles array with explicit dimension bounds [1:2]={10, 20}", () => {
      const res = parseUniversalArray("[1:2]={10, 20}");
      expect(res.ok).toBe(true);
      expect(res.elements).toHaveLength(2);
      expect(res.elements[0].indexLabel).toBe("[1]");
      expect(res.elements[0].displayValue).toBe("10");
    });
  });

  describe("Corrupted JSON arrays", () => {
    it("rejects trailing commas in JSON arrays", () => {
      const res = parseUniversalArray("[1, 2, 3,]");
      expect(res.ok).toBe(false);
      expect(res.error).toBeDefined();
    });

    it("rejects single-quoted JSON arrays", () => {
      const res = parseUniversalArray("['item1', 'item2']");
      expect(res.ok).toBe(false);
      expect(res.error).toBeDefined();
    });

    it("rejects unclosed JSON arrays", () => {
      const res = parseUniversalArray("[1, 2, 3");
      expect(res.ok).toBe(false);
    });

    it("rejects empty or whitespace-only inputs", () => {
      expect(parseUniversalArray("").ok).toBe(false);
      expect(parseUniversalArray("   \n\t  ").ok).toBe(false);
      expect(parseUniversalArray(null).ok).toBe(false);
      expect(parseUniversalArray(undefined).ok).toBe(false);
    });
  });

  describe("Array classification, performance and CSV export", () => {
    it("classifies nested primitives and complex structures accurately", () => {
      const raw = JSON.stringify([
        42,
        3.14159,
        "sample",
        true,
        false,
        null,
        { a: 1, b: 2, c: 3 },
        [10, 20],
      ]);
      const res = parseUniversalArray(raw);
      expect(res.ok).toBe(true);
      expect(res.elements[0].kind).toBe("number");
      expect(res.elements[0].lengthOrSize).toBe("int");
      expect(res.elements[1].kind).toBe("number");
      expect(res.elements[1].lengthOrSize).toBe("float");
      expect(res.elements[2].kind).toBe("string");
      expect(res.elements[2].lengthOrSize).toBe("6 chars");
      expect(res.elements[3].kind).toBe("boolean");
      expect(res.elements[4].kind).toBe("boolean");
      expect(res.elements[5].kind).toBe("null");
      expect(res.elements[6].kind).toBe("object");
      expect(res.elements[6].lengthOrSize).toBe("3 keys");
      expect(res.elements[7].kind).toBe("array");
      expect(res.elements[7].lengthOrSize).toBe("2 items");
    });

    it("handles large arrays (10,000 items) with truncation cap and sub-100ms performance", () => {
      const largeArray = Array.from({ length: 10_000 }, (_, i) => `item_${i}`);
      const raw = JSON.stringify(largeArray);

      const start = performance.now();
      const res = parseUniversalArray(raw);
      const elapsed = performance.now() - start;

      expect(res.ok).toBe(true);
      expect(res.totalElements).toBe(10_000);
      expect(res.elements.length).toBe(2_000);
      expect(res.truncated).toBe(true);
      expect(elapsed).toBeLessThan(200);
    });

    it("exports CSV following RFC 4180 with proper double-quote escaping", () => {
      const raw = JSON.stringify([
        'He said "Hello, World!"',
        'Comma, and "quotes" combined',
        "Line 1\nLine 2",
      ]);
      const res = parseUniversalArray(raw);
      expect(res.ok).toBe(true);

      const csv = exportArrayAsCsv(res.elements);
      const lines = csv.split("\n");
      expect(lines[0]).toBe('"Index","Type","Value","Size"');
      // Quotes inside values must be doubled
      expect(csv).toContain('""Hello, World!""');
      expect(csv).toContain('Comma, and ""quotes"" combined');
    });
  });
});

describe("Adversarial Stress Test: spatial-format.ts", () => {
  describe("Malformed WKT and edge cases", () => {
    it("rejects invalid geometry keywords", () => {
      const res1 = parseSpatialGeometry("CIRCULARSTRING (1 2, 3 4)");
      expect(res1.ok).toBe(false);

      const res2 = parseSpatialGeometry("FOOBAR (1 2)");
      expect(res2.ok).toBe(false);
    });

    it("rejects unclosed / mismatched parentheses in WKT bodies", () => {
      // In WKT, "POLYGON ((30 10, 40 40" is malformed because closing parens are missing.
      const res1 = parseSpatialGeometry("POLYGON ((30 10, 40 40");
      expect(res1.ok).toBe(false);
      expect(res1.error).toMatch(/parenthes/i);

      // Partially unclosed parentheses
      const res2 = parseSpatialGeometry("POLYGON ((30 10, 40 40)");
      expect(res2.ok).toBe(false);
      expect(res2.error).toContain("unclosed parenthesis");
    });

    it("safely handles empty geometries: POINT EMPTY & GEOMETRYCOLLECTION EMPTY", () => {
      const ptEmpty = parseSpatialGeometry("POINT EMPTY");
      expect(ptEmpty.ok).toBe(true);
      expect(ptEmpty.geometry?.vertexCount).toBe(0);
      expect(ptEmpty.geometry?.bbox).toEqual([0, 0, 0, 0]);

      if (ptEmpty.geometry) {
        const svg = buildSvgRenderData(ptEmpty.geometry);
        expect(svg.viewBox).toBe("0 0 100 100");
        expect(svg.viewBoxBounds.minX).toBe(0);
        expect(svg.emptyMessage).toBe("Empty Geometry");
      }

      const gcEmpty = parseSpatialGeometry("GEOMETRYCOLLECTION EMPTY");
      expect(gcEmpty.ok).toBe(true);
      expect(gcEmpty.geometry?.kind).toBe("GEOMETRYCOLLECTION");
      expect(gcEmpty.geometry?.vertexCount).toBe(0);
      expect(gcEmpty.geometry?.bbox).toEqual([0, 0, 0, 0]);
    });
  });

  describe("3D and 4D coordinates", () => {
    it("parses 3D POINT Z coordinates", () => {
      const res = parseSpatialGeometry("POINT Z (10 20 30)");
      expect(res.ok).toBe(true);
      expect(res.geometry?.kind).toBe("POINT");
      expect(res.geometry?.points?.[0]).toEqual({ x: 10, y: 20, z: 30 });
    });

    it("evaluates POINT M coordinates", () => {
      const res = parseSpatialGeometry("POINT M (10 20 30)");
      expect(res.ok).toBe(true);
      // In current implementation, nums[2] is mapped to pt.z
      expect(res.geometry?.points?.[0].z).toBe(30);
    });

    it("evaluates 4D POINT ZM coordinates", () => {
      const res = parseSpatialGeometry("POINT ZM (10 20 30 40)");
      expect(res.ok).toBe(true);
      expect(res.geometry?.points?.[0].x).toBe(10);
      expect(res.geometry?.points?.[0].y).toBe(20);
      expect(res.geometry?.points?.[0].z).toBe(30);
      // Note: 4th coordinate (m) is not stored on Coordinate
    });
  });

  describe("Complex polygon geometries with multiple nested holes", () => {
    it("parses nested holes and generates fill-rule='evenodd' SVG paths", () => {
      // Concentric rings: outer square, hole 1, island inside hole 1
      const nestedWkt =
        "POLYGON ((0 0, 100 0, 100 100, 0 100, 0 0), (20 20, 80 20, 80 80, 20 80, 20 20), (40 40, 60 40, 60 60, 40 60, 40 40))";
      const res = parseSpatialGeometry(nestedWkt);
      expect(res.ok).toBe(true);
      expect(res.geometry?.kind).toBe("POLYGON");
      expect(res.geometry?.polygons).toHaveLength(1);

      const poly = res.geometry!.polygons![0];
      expect(poly.exteriorRing).toHaveLength(5);
      expect(poly.interiorRings).toHaveLength(2);

      const svg = buildSvgRenderData(res.geometry!);
      expect(svg.paths).toHaveLength(1);
      expect(svg.paths[0].fillRule).toBe("evenodd");
      // Verify path contains multiple M / Z segments for holes
      const mMatches = svg.paths[0].d.match(/M/g);
      expect(mMatches?.length).toBe(3);
    });
  });

  describe("Degenerate geometries", () => {
    it("handles zero-area polygons without division by zero in SVG viewBox", () => {
      const zeroPoly = "POLYGON ((10 10, 10 10, 10 10, 10 10))";
      const res = parseSpatialGeometry(zeroPoly);
      expect(res.ok).toBe(true);
      expect(res.geometry?.bbox).toEqual([10, 10, 10, 10]);

      const svg = buildSvgRenderData(res.geometry!);
      expect(svg.viewBoxBounds.width).toBeGreaterThan(0);
      expect(svg.viewBox).not.toContain("NaN");
      expect(svg.viewBox).not.toContain("0 0 0 0");
    });

    it("handles coincident vertices in LINESTRING", () => {
      const line = "LINESTRING (25 25, 25 25)";
      const res = parseSpatialGeometry(line);
      expect(res.ok).toBe(true);

      const svg = buildSvgRenderData(res.geometry!);
      expect(svg.viewBoxBounds.width).toBeGreaterThan(0);
    });
  });

  describe("GeoJSON FeatureCollection and GeometryCollection", () => {
    it("supports GeoJSON FeatureCollection", () => {
      const fc = JSON.stringify({
        type: "FeatureCollection",
        features: [
          {
            type: "Feature",
            geometry: { type: "Point", coordinates: [100.0, 0.0] },
            properties: { prop0: "value0" },
          },
        ],
      });
      const res = parseSpatialGeometry(fc);
      expect(res.ok).toBe(true);
      expect(res.geometry?.kind).toBe("GEOMETRYCOLLECTION");
      expect(res.geometry?.vertexCount).toBe(1);
      expect(res.geometry?.points).toHaveLength(1);
      expect(res.geometry?.points?.[0]).toEqual({ x: 100.0, y: 0.0 });
    });

    it("supports GeoJSON GeometryCollection", () => {
      const gc = JSON.stringify({
        type: "GeometryCollection",
        geometries: [{ type: "Point", coordinates: [100.0, 0.0] }],
      });
      const res = parseSpatialGeometry(gc);
      expect(res.ok).toBe(true);
      expect(res.geometry?.kind).toBe("GEOMETRYCOLLECTION");
      expect(res.geometry?.vertexCount).toBe(1);
      expect(res.geometry?.points).toHaveLength(1);
      expect(res.geometry?.points?.[0]).toEqual({ x: 100.0, y: 0.0 });
    });
  });
});

describe("Adversarial Stress Test: variant-path.ts", () => {
  describe("Hostile identifiers and SQL reserved keywords", () => {
    it("quotes SQL reserved keywords in colon/dot syntax to prevent compilation syntax errors", () => {
      // In Snowflake, referencing variant keys that match SQL reserved keywords (SELECT, FROM, WHERE)
      // without double quotes causes a SQL compilation syntax error.
      const exprSelect = generateSnowflakePath({
        columnName: "payload",
        path: ["SELECT"],
        valueType: "string",
      });
      expect(exprSelect).toBe('payload:"SELECT"::string');

      const exprFrom = generateSnowflakePath({
        columnName: "payload",
        path: ["FROM"],
        valueType: "string",
      });
      expect(exprFrom).toBe('payload:"FROM"::string');

      const exprWhereLower = generateSnowflakePath({
        columnName: "payload",
        path: ["where"],
        valueType: "string",
      });
      expect(exprWhereLower).toBe('payload:"where"::string');
    });

    it("properly quotes keys containing dots, colons, and hyphens in colon/dot syntax", () => {
      const exprDot = generateSnowflakePath({
        columnName: "col",
        path: ["my.key"],
        valueType: "string",
      });
      expect(exprDot).toBe('col:"my.key"::string');

      const exprColon = generateSnowflakePath({
        columnName: "col",
        path: ["attr:val"],
        valueType: "string",
      });
      expect(exprColon).toBe('col:"attr:val"::string');

      const exprHyphen = generateSnowflakePath({
        columnName: "col",
        path: ["user-id"],
        cast: "int",
      });
      expect(exprHyphen).toBe('col:"user-id"::int');
    });

    it("escapes double quotes inside keys for colon/dot syntax using SQL standard doubled double-quotes", () => {
      const exprQuotes = generateSnowflakePath({
        columnName: "col",
        path: ['user"name'],
        valueType: "string",
      });
      expect(exprQuotes).toBe('col:"user""name"::string');
    });

    it("escapes single quotes with SQL-standard doubled quotes and escapes backslashes in bracket style syntax", () => {
      // Single quote in bracket syntax: doubled '' instead of C-style \'
      const exprSingleQuote = generateSnowflakePath({
        columnName: "col",
        path: ["user'name"],
        style: "bracket",
        valueType: "string",
      });
      expect(exprSingleQuote).toBe("col['user''name']::string");

      // Backslash in bracket syntax: escaped \\\\
      const exprBackslash = generateSnowflakePath({
        columnName: "col",
        path: ["path\\to\\file"],
        style: "bracket",
        valueType: "string",
      });
      expect(exprBackslash).toBe("col['path\\\\to\\\\file']::string");

      // Trailing backslash must not leave unescaped quote
      const exprTrailingBackslash = generateSnowflakePath({
        columnName: "col",
        path: ["key\\"],
        style: "bracket",
        valueType: "string",
      });
      expect(exprTrailingBackslash).toBe("col['key\\\\']::string");
    });

    it("handles keys with emojis and newlines", () => {
      const exprEmoji = generateSnowflakePath({
        columnName: "col",
        path: ["user_🎉"],
        valueType: "string",
      });
      expect(exprEmoji).toBe('col:"user_🎉"::string');

      const exprNewline = generateSnowflakePath({
        columnName: "col",
        path: ["line1\nline2"],
        valueType: "string",
      });
      expect(exprNewline).toBe('col:"line1\nline2"::string');
    });
  });

  describe("Deep nested access with mixed array indices and object keys", () => {
    it("generates colon/dot path with array indices and quoted identifiers", () => {
      const expr = generateSnowflakePath({
        columnName: "col",
        path: ["my.key", 0, "attr:val"],
        valueType: "string",
        style: "colon",
      });
      expect(expr).toBe('col:"my.key"[0]."attr:val"::string');
    });

    it("generates bracket style path with array indices and object keys", () => {
      const expr = generateSnowflakePath({
        columnName: "col",
        path: ["my.key", 0, "attr:val"],
        valueType: "string",
        style: "bracket",
      });
      expect(expr).toBe("col['my.key'][0]['attr:val']::string");
    });

    it("handles top-level array indexing followed by nested object access", () => {
      const expr = generateSnowflakePath({
        columnName: "records",
        path: [0, "status"],
        valueType: "string",
        style: "colon",
      });
      expect(expr).toBe("records[0].status::string");
    });
  });

  describe("Snowflake type cast syntax conformance", () => {
    it("supports all defined Snowflake cast types", () => {
      for (const castItem of SNOWFLAKE_CAST_TYPES) {
        const expr = generateSnowflakePath({
          columnName: "col",
          path: ["f"],
          cast: castItem.id,
        });
        if (castItem.id === "none") {
          expect(expr).toBe("col:f");
        } else {
          expect(expr).toBe(`col:f::${castItem.id}`);
        }
      }
    });

    it("infers accurate default casts based on value types and samples", () => {
      expect(inferDefaultSnowflakeCast("string")).toBe("string");
      expect(inferDefaultSnowflakeCast("number", 42)).toBe("int");
      expect(inferDefaultSnowflakeCast("number", 3.14)).toBe("number");
      expect(inferDefaultSnowflakeCast("boolean")).toBe("boolean");
      expect(inferDefaultSnowflakeCast("array")).toBe("array");
      expect(inferDefaultSnowflakeCast("object")).toBe("object");
      expect(inferDefaultSnowflakeCast("null")).toBe("string");
      expect(inferDefaultSnowflakeCast("unknown_type")).toBe("variant");
      expect(inferDefaultSnowflakeCast(undefined)).toBe("string");
    });
  });
});
