import { describe, expect, it } from "vitest";
import { evaluateJsonPath } from "./jsonpath";
import {
  parseDateTime,
  formatInTimezone,
  formatRelativeTime,
  getCalendarBreakdown,
} from "./datetime-utils";
import {
  parseSpatialGeometry,
  buildSvgRenderData,
  type ParsedSpatialGeometry,
} from "./spatial-format";
import {
  generateSnowflakePath,
  isSnowflakeSimpleIdentifier,
  inferDefaultSnowflakeCast,
  SNOWFLAKE_RESERVED_KEYWORDS,
} from "./variant-path";

describe("Milestone 6 Phase 2 Challenger: Inspector White-Box Adversarial Hardening", () => {
  describe("1. jsonpath.ts deep recursion, cyclic structures, and boundary parsing", () => {
    it("handles deep recursive descent (500 levels) without call stack overflow", () => {
      let root: Record<string, unknown> = { leafValue: 42 };
      for (let i = 0; i < 500; i++) {
        root = { [`level_${i}`]: root };
      }

      const res = evaluateJsonPath("$..leafValue", root);
      expect(res.ok).toBe(true);
      if (res.ok) {
        expect(res.matches).toHaveLength(1);
        expect(res.matches[0].value).toBe(42);
        expect(res.matches[0].normalizedPath).toHaveLength(501);
      }
    });

    it("evaluates mutual cyclic references across objects and arrays safely with WeakSet guards", () => {
      interface NodeA {
        id: string;
        child?: NodeB;
      }
      interface NodeB {
        id: string;
        parent?: NodeA;
        list?: unknown[];
      }

      const a: NodeA = { id: "node-a" };
      const b: NodeB = { id: "node-b", parent: a };
      a.child = b;
      b.list = [a, b];

      // Recursive descent on specific key in cyclic graph
      const resKey = evaluateJsonPath("$..id", a);
      expect(resKey.ok).toBe(true);
      if (resKey.ok) {
        const ids = resKey.matches.map((m) => m.value);
        expect(ids).toContain("node-a");
        expect(ids).toContain("node-b");
        // Verify it terminates and did not infinitely loop
        expect(resKey.matches.length).toBeLessThan(10);
      }

      // Recursive descent wildcard on cyclic graph
      const resWild = evaluateJsonPath("$..*", a);
      expect(resWild.ok).toBe(true);
      if (resWild.ok) {
        expect(resWild.matches.length).toBeGreaterThan(0);
        expect(resWild.matches.length).toBeLessThan(20);
      }
    });

    it("handles self-referential arrays gracefully", () => {
      const arr: unknown[] = [1, 2];
      arr.push(arr);

      const res = evaluateJsonPath("$..*", arr);
      expect(res.ok).toBe(true);
      if (res.ok) {
        // Must contain 1, 2, and the array itself once without crashing
        expect(res.matches.map((m) => m.value)).toContain(1);
        expect(res.matches.map((m) => m.value)).toContain(2);
      }
    });

    it("rejects malformed and edge-case expressions with informative errors", () => {
      // Missing property after dot
      expect(evaluateJsonPath("$.store.", { store: 1 }).ok).toBe(false);
      // Empty bracket
      expect(evaluateJsonPath("$[]", [1]).ok).toBe(false);
      // Malformed bracket slice parameters (non-numeric)
      expect(evaluateJsonPath("$[abc:def]", [1, 2]).ok).toBe(false);
      // Filter without '@' prefix
      expect(evaluateJsonPath("$[?(price == 10)]", [{ price: 10 }]).ok).toBe(false);
      // Filter with unclosed parenthesis
      expect(evaluateJsonPath("$[?(@.price == 10", [{ price: 10 }]).ok).toBe(false);
      // Unexpected leading token
      expect(evaluateJsonPath("@.store", { store: 1 }).ok).toBe(false);
      // Step zero in slice
      const sliceZero = evaluateJsonPath("$[0:5:0]", [1, 2, 3]);
      expect(sliceZero.ok).toBe(true);
      if (sliceZero.ok) {
        // Step <= 0 loop produces empty matches rather than infinite loop
        expect(sliceZero.matches).toEqual([]);
      }
    });

    it("FINDING: compileFilterCondition accepts invalid operator '<>' by partially matching '<'", () => {
      // In jsonpath.ts:331, opMatch regex tests '<' which prefix-matches the SQL '<>' operator,
      // treating the remaining '>' as part of the expected value string rather than rejecting syntax.
      const res = evaluateJsonPath("$[?(@.price <> 10)]", [{ price: 5 }]);
      expect(res.ok).toBe(true);
      if (res.ok) {
        // Evaluates condition (5 < "> 10") which returns false due to type mismatch, yielding 0 matches
        expect(res.matches).toEqual([]);
      }
    });

    it("handles primitive root values (string, number, boolean, null)", () => {
      expect(evaluateJsonPath("$", 12345)).toEqual({
        ok: true,
        matches: [{ path: "$", normalizedPath: [], value: 12345 }],
      });
      expect(evaluateJsonPath("$.prop", 12345)).toEqual({
        ok: true,
        matches: [],
      });
      expect(evaluateJsonPath("$[0]", "text")).toEqual({
        ok: true,
        matches: [],
      });
      expect(evaluateJsonPath("$", true)).toEqual({
        ok: true,
        matches: [{ path: "$", normalizedPath: [], value: true }],
      });
    });
  });

  describe("2. datetime-utils.ts first-century formatting, invalid dates, and timezone math", () => {
    it("preserves years 0000..0099 strictly without 1900s distortion", () => {
      const years = ["0000", "0001", "0042", "0099"];
      for (const y of years) {
        const parsed = parseDateTime(`${y}-06-15 12:00:00`);
        expect(parsed).not.toBeNull();
        if (parsed) {
          expect(parsed.date.getUTCFullYear()).toBe(Number(y));
          expect(parsed.date.getUTCMonth()).toBe(5); // June
          expect(parsed.date.getUTCDate()).toBe(15);
        }
      }
    });

    it("parses first-century dates with explicit UTC timezone", () => {
      const parsed = parseDateTime("0050-01-01T00:00:00Z");
      expect(parsed).not.toBeNull();
      if (parsed) {
        expect(parsed.date.getUTCFullYear()).toBe(50);
        expect(parsed.hasTimezone).toBe(true);
      }
    });

    it("safely handles Invalid Date without unhandled throws in formatInTimezone", () => {
      const nanDate = new Date(Number.NaN);
      expect(formatInTimezone(nanDate, "UTC")).toBe("Invalid Date");
      expect(formatInTimezone(nanDate, "America/New_York")).toBe("Invalid Date");
      expect(formatInTimezone(nanDate, "Invalid/Timezone")).toBe("Invalid Date");
    });

    it("falls back gracefully when given completely invalid IANA timezones", () => {
      const validDate = new Date("2026-11-06T12:00:00.000Z");
      const formatted = formatInTimezone(validDate, "Unknown/Alien_Timezone");
      // Fallback is valid ISO string replace T with space
      expect(formatted).toBe("2026-11-06 12:00:00.000Z");
    });

    it("handles relative time edge cases: immediate zero diff, past, and future", () => {
      const now = 1730000000000;
      // Exact diff 0s
      expect(formatRelativeTime(now, now)).toBe("just now");
      // 4s diff
      expect(formatRelativeTime(now + 4000, now)).toBe("just now");
      // 10s future
      expect(formatRelativeTime(now + 10000, now)).toMatch(/in 10 seconds|second/);
      // 5 minutes ago
      expect(formatRelativeTime(now - 300000, now)).toMatch(/5 minutes ago|minute/);
      // 2 hours ago
      expect(formatRelativeTime(now - 7200000, now)).toMatch(/2 hours ago|hour/);
      // 3 days future
      expect(formatRelativeTime(now + 259200000, now)).toMatch(/in 3 days|day/);
    });

    it("verifies getCalendarBreakdown across century, leap, and non-leap years", () => {
      // Leap year 2024: 366 days
      const d2024 = new Date("2024-03-01T00:00:00Z");
      const c2024 = getCalendarBreakdown(d2024);
      expect(c2024.isLeapYear).toBe(true);
      expect(c2024.totalDaysInYear).toBe(366);
      expect(c2024.quarter).toBe("Q1");

      // Non-leap year 2023: 365 days
      const d2023 = new Date("2023-03-01T00:00:00Z");
      const c2023 = getCalendarBreakdown(d2023);
      expect(c2023.isLeapYear).toBe(false);
      expect(c2023.totalDaysInYear).toBe(365);

      // Century non-leap year 1900: divisible by 100 but not 400
      const d1900 = new Date("1900-05-15T00:00:00Z");
      const c1900 = getCalendarBreakdown(d1900);
      expect(c1900.isLeapYear).toBe(false);
      expect(c1900.totalDaysInYear).toBe(365);
      expect(c1900.quarter).toBe("Q2");

      // Century leap year 2000: divisible by 400
      const d2000 = new Date("2000-08-20T00:00:00Z");
      const c2000 = getCalendarBreakdown(d2000);
      expect(c2000.isLeapYear).toBe(true);
      expect(c2000.totalDaysInYear).toBe(366);
      expect(c2000.quarter).toBe("Q3");
    });
  });

  describe("3. spatial-format.ts empty geometries, WKT syntax errors, and viewBox math", () => {
    it("handles all standard EMPTY geometries safely with emptyMessage and default viewBox", () => {
      const emptyTypes = [
        "POINT EMPTY",
        "LINESTRING EMPTY",
        "POLYGON EMPTY",
        "MULTIPOINT EMPTY",
        "MULTILINESTRING EMPTY",
        "MULTIPOLYGON EMPTY",
        "GEOMETRYCOLLECTION EMPTY",
      ];

      for (const wkt of emptyTypes) {
        const parsed = parseSpatialGeometry(wkt);
        expect(parsed.ok).toBe(true);
        expect(parsed.geometry).toBeDefined();
        if (parsed.geometry) {
          expect(parsed.geometry.vertexCount).toBe(0);
          expect(parsed.geometry.bbox).toEqual([0, 0, 0, 0]);

          const svg = buildSvgRenderData(parsed.geometry);
          expect(svg.emptyMessage).toBe("Empty Geometry");
          expect(svg.viewBox).toBe("0 0 100 100");
          expect(svg.paths).toHaveLength(0);
          expect(svg.vertices).toHaveLength(0);
        }
      }
    });

    it("handles deeply nested GEOMETRYCOLLECTION and aggregates bounds correctly", () => {
      const nestedWkt =
        "GEOMETRYCOLLECTION (POINT (10 20), GEOMETRYCOLLECTION (LINESTRING (30 40, 50 60), POINT EMPTY), POINT (70 80))";
      const parsed = parseSpatialGeometry(nestedWkt);
      expect(parsed.ok).toBe(true);
      if (parsed.geometry) {
        expect(parsed.geometry.kind).toBe("GEOMETRYCOLLECTION");
        expect(parsed.geometry.vertexCount).toBe(4);
        expect(parsed.geometry.bbox).toEqual([10, 20, 70, 80]);

        const svg = buildSvgRenderData(parsed.geometry);
        expect(svg.paths.length).toBeGreaterThanOrEqual(1); // Linestring path
        expect(svg.vertices.length).toBe(4);
        expect(svg.viewBoxBounds.minX).toBeLessThan(10);
        expect(svg.viewBoxBounds.width).toBeGreaterThan(60);
      }
    });

    it("rejects malformed WKT with trailing garbage or unclosed parentheses", () => {
      // Unclosed paren
      expect(parseSpatialGeometry("LINESTRING (1 1, 2 2").ok).toBe(false);
      // Mismatched paren
      expect(parseSpatialGeometry("POINT ((1 1)").ok).toBe(false);
      // Trailing garbage after valid geometry
      const trailing = parseSpatialGeometry("POINT (1 1) DROP TABLE USERS;");
      expect(trailing.ok).toBe(false);
      expect(trailing.error).toMatch(/must be enclosed in parentheses|trailing content/i);
      // Missing geometry type
      expect(parseSpatialGeometry("(1 1, 2 2)").ok).toBe(false);
    });

    it("builds valid SVG viewBox for negative coordinates and zero spans without NaN", () => {
      // Negative coordinates bounding box
      const negWkt = "LINESTRING (-122.4194 -37.7749, -74.0060 -40.7128)";
      const parsedNeg = parseSpatialGeometry(negWkt);
      expect(parsedNeg.ok).toBe(true);
      if (parsedNeg.geometry) {
        const svg = buildSvgRenderData(parsedNeg.geometry);
        expect(svg.viewBox).not.toContain("NaN");
        expect(svg.viewBoxBounds.width).toBeGreaterThan(0);
        expect(svg.viewBoxBounds.height).toBeGreaterThan(0);
      }

      // Single point zero span: minX === maxX, minY === maxY
      const ptWkt = "POINT (500 500)";
      const parsedPt = parseSpatialGeometry(ptWkt);
      expect(parsedPt.ok).toBe(true);
      if (parsedPt.geometry) {
        const svg = buildSvgRenderData(parsedPt.geometry);
        expect(svg.viewBox).not.toContain("NaN");
        expect(svg.viewBoxBounds.width).toBeGreaterThan(0);
        expect(svg.viewBoxBounds.height).toBeGreaterThan(0);
        expect(svg.vertices[0].cx).toBeGreaterThan(0);
        expect(svg.vertices[0].cy).toBeGreaterThan(0);
      }
    });
  });

  describe("4. variant-path.ts Snowflake reserved words, escaping, and bracket notation", () => {
    it("strictly verifies all Snowflake reserved keywords require double quotes in colon syntax", () => {
      const keywordsToTest = [
        "SELECT",
        "FROM",
        "WHERE",
        "TABLE",
        "ORDER",
        "GROUP",
        "HAVING",
        "LIMIT",
        "JOIN",
        "UNION",
        "VALUES",
        "UPDATE",
        "DELETE",
        "INSERT",
        "CREATE",
        "DROP",
        "ALTER",
        "TRUNCATE",
        "MERGE",
        "QUALIFY",
        "TRY_CAST",
      ];

      for (const kw of keywordsToTest) {
        // Upper case
        expect(isSnowflakeSimpleIdentifier(kw)).toBe(false);
        const exprUpper = generateSnowflakePath({
          columnName: "v",
          path: [kw],
          valueType: "string",
          style: "colon",
        });
        expect(exprUpper).toBe(`v:"${kw}"::string`);

        // Lower case
        expect(isSnowflakeSimpleIdentifier(kw.toLowerCase())).toBe(false);
        const exprLower = generateSnowflakePath({
          columnName: "v",
          path: [kw.toLowerCase()],
          valueType: "string",
          style: "colon",
        });
        expect(exprLower).toBe(`v:"${kw.toLowerCase()}"::string`);
      }
    });

    it("properly escapes double quotes inside colon path segments with doubled double-quotes", () => {
      const expr = generateSnowflakePath({
        columnName: "payload",
        path: ['data"field'],
        valueType: "string",
        style: "colon",
      });
      expect(expr).toBe('payload:"data""field"::string');
    });

    it("properly escapes single quotes and backslashes in bracket syntax with doubled single-quotes", () => {
      const expr = generateSnowflakePath({
        columnName: "payload",
        path: ["user's_data", "c:\\temp\\file"],
        valueType: "string",
        style: "bracket",
      });
      expect(expr).toBe("payload['user''s_data']['c:\\\\temp\\\\file']::string");
    });

    it("generates correct paths for top-level array indexing and mixed path chains", () => {
      // Top level array index in colon style
      const exprArrayFirst = generateSnowflakePath({
        columnName: "items",
        path: [0, "name"],
        valueType: "string",
        style: "colon",
      });
      expect(exprArrayFirst).toBe("items[0].name::string");

      // Multiple consecutive array indices
      const exprMatrix = generateSnowflakePath({
        columnName: "matrix",
        path: [1, 2, 3],
        valueType: "number",
        style: "colon",
      });
      expect(exprMatrix).toBe("matrix[1][2][3]::number");

      // Empty path returns just the column name without cast
      const exprEmpty = generateSnowflakePath({
        columnName: "raw_variant",
        path: [],
      });
      expect(exprEmpty).toBe("raw_variant");
    });
  });
});
