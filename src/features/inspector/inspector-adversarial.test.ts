import { describe, expect, it } from "vitest";
import { evaluateJsonPath } from "./jsonpath";
import {
  parseBinaryBytes,
  detectImageFormat,
  extractImageDimensions,
  formatHexDumpLines,
  formatByteSize,
} from "./binary-utils";
import {
  parseDateTime,
  formatInTimezone,
  formatRelativeTime,
  getCalendarBreakdown,
} from "./datetime-utils";

describe("Milestone 4 Adversarial Stress Harness: jsonpath.ts", () => {
  it("handles malformed query strings without unhandled exceptions", () => {
    // Unclosed bracket
    const unclosed = evaluateJsonPath("$.store[0", { store: [1, 2] });
    expect(unclosed.ok).toBe(false);
    if (!unclosed.ok) {
      expect(unclosed.error).toContain("Unclosed '['");
    }

    // Trailing dot
    const trailingDot = evaluateJsonPath("$.store.", { store: { a: 1 } });
    expect(trailingDot.ok).toBe(false);
    if (!trailingDot.ok) {
      expect(trailingDot.error).toContain("Trailing '.'");
    }

    // Trailing double dot
    const trailingDoubleDot = evaluateJsonPath("$.store..", { store: { a: 1 } });
    expect(trailingDoubleDot.ok).toBe(false);
    if (!trailingDoubleDot.ok) {
      expect(trailingDoubleDot.error).toContain("Trailing '..'");
    }

    // Malformed bracket token
    const malformedBracket = evaluateJsonPath("$.store[invalidToken]", { store: [1] });
    expect(malformedBracket.ok).toBe(false);
    if (!malformedBracket.ok) {
      expect(malformedBracket.error).toContain("Invalid bracket token");
    }
  });

  it("evaluateJsonPath returns ok: false on null or undefined query instead of throwing TypeError", () => {
    const resNull = evaluateJsonPath(null as unknown as string, { a: 1 });
    expect(resNull.ok).toBe(false);
    if (!resNull.ok) {
      expect(resNull.error).toContain("must be a string");
      expect(resNull.matches).toEqual([]);
    }

    const resUndef = evaluateJsonPath(undefined as unknown as string, { a: 1 });
    expect(resUndef.ok).toBe(false);
    if (!resUndef.ok) {
      expect(resUndef.error).toContain("must be a string");
      expect(resUndef.matches).toEqual([]);
    }
  });

  it("handles null and undefined root values safely", () => {
    const resNull = evaluateJsonPath("$.a.b", null);
    expect(resNull.ok).toBe(true);
    if (resNull.ok) {
      expect(resNull.matches).toEqual([]);
    }

    const resUndef = evaluateJsonPath("$.a.b", undefined);
    expect(resUndef.ok).toBe(true);
    if (resUndef.ok) {
      expect(resUndef.matches).toEqual([]);
    }

    const resRootNull = evaluateJsonPath("$", null);
    expect(resRootNull.ok).toBe(true);
    if (resRootNull.ok) {
      expect(resRootNull.matches).toHaveLength(1);
      expect(resRootNull.matches[0].value).toBeNull();
    }
  });

  it("evaluateJsonPath recursive descent terminates safely on cyclic structures without stack overflow", () => {
    interface CyclicNode {
      name: string;
      self?: CyclicNode;
    }
    const cyclic: CyclicNode = { name: "root" };
    cyclic.self = cyclic;

    // Direct property lookup succeeds without loop
    const directRes = evaluateJsonPath("$.name", cyclic);
    expect(directRes.ok).toBe(true);
    if (directRes.ok) {
      expect(directRes.matches[0].value).toBe("root");
    }

    // Recursive descent tracks visited objects via WeakSet, preventing RangeError
    const recRes = evaluateJsonPath("$..name", cyclic);
    expect(recRes.ok).toBe(true);
    if (recRes.ok) {
      expect(recRes.matches).toHaveLength(1);
      expect(recRes.matches[0].value).toBe("root");
    }
  });

  it("handles deep nesting (100 levels) without stack overflow", () => {
    let deep: Record<string, unknown> = { leaf: "found_it" };
    for (let i = 0; i < 100; i++) {
      deep = { child: deep };
    }

    const res = evaluateJsonPath("$..leaf", deep);
    expect(res.ok).toBe(true);
    if (res.ok) {
      expect(res.matches).toHaveLength(1);
      expect(res.matches[0].value).toBe("found_it");
    }
  });

  it("FINDING: array slice with negative step [::-1] returns empty matches instead of reversing", () => {
    const list = [10, 20, 30, 40, 50];

    // Forward slice works
    const fwd = evaluateJsonPath("$[0:3]", list);
    expect(fwd.ok).toBe(true);
    if (fwd.ok) {
      expect(fwd.matches.map((m) => m.value)).toEqual([10, 20, 30]);
    }

    // Negative step [::-1] is currently ignored because `if (step > 0)` in jsonpath.ts:108
    const rev = evaluateJsonPath("$[::-1]", list);
    expect(rev.ok).toBe(true);
    if (rev.ok) {
      // Demonstrating that negative step yields empty array
      expect(rev.matches).toEqual([]);
    }

    // Step 2 works
    const step2 = evaluateJsonPath("$[::2]", list);
    expect(step2.ok).toBe(true);
    if (step2.ok) {
      expect(step2.matches.map((m) => m.value)).toEqual([10, 30, 50]);
    }

    // Out of bound slice returns empty gracefully
    const oob = evaluateJsonPath("$[100:200]", list);
    expect(oob.ok).toBe(true);
    if (oob.ok) {
      expect(oob.matches).toEqual([]);
    }
  });

  it("evaluates filter expressions with missing fields, type coercion, and unicode keys", () => {
    const data = [
      { id: 1, title: "Book A", score: 95, active: true },
      { id: 2, title: "Book B", score: 40, active: false },
      { id: 3, title: "", active: true }, // missing score
      { id: 4, score: null, active: null },
      null,
      "not an object",
    ];

    // Filter missing fields: elements without 'score' should be excluded
    const highScores = evaluateJsonPath("$[?(@.score >= 90)].title", data);
    expect(highScores.ok).toBe(true);
    if (highScores.ok) {
      expect(highScores.matches).toHaveLength(1);
      expect(highScores.matches[0].value).toBe("Book A");
    }

    // Filter with null check
    const nullScore = evaluateJsonPath("$[?(@.score == null)].id", data);
    expect(nullScore.ok).toBe(true);
    if (nullScore.ok) {
      expect(nullScore.matches).toHaveLength(1);
      expect(nullScore.matches[0].value).toBe(4);
    }

    // Filter truthy
    const truthyTitle = evaluateJsonPath("$[?(@.title)].id", data);
    expect(truthyTitle.ok).toBe(true);
    if (truthyTitle.ok) {
      expect(truthyTitle.matches.map((m) => m.value)).toEqual([1, 2]);
    }

    // Unicode bracket property lookup
    const unicodeObj = { "🚀 launch_code": "SATURN_V", π_value: 3.14159 };
    const uniRes = evaluateJsonPath("$['🚀 launch_code']", unicodeObj);
    expect(uniRes.ok).toBe(true);
    if (uniRes.ok) {
      expect(uniRes.matches[0].value).toBe("SATURN_V");
    }
  });
});

describe("Milestone 4 Adversarial Stress Harness: binary-utils.ts", () => {
  it("handles truncated and corrupted image headers gracefully", () => {
    // 1-byte buffer
    expect(detectImageFormat(new Uint8Array([0x89]))).toBeNull();

    // 7-byte truncated PNG header
    const png7 = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a]);
    expect(detectImageFormat(png7)).toBeNull();

    // 8-byte PNG signature but truncated before IHDR chunk
    const png8 = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    expect(detectImageFormat(png8)?.format).toBe("png");
    expect(extractImageDimensions(png8, "png")).toBeNull();

    // 23-byte PNG (1 byte short of the 24 bytes required for IHDR dimensions)
    const png23 = new Uint8Array(23);
    png23.set(png8, 0);
    expect(extractImageDimensions(png23, "png")).toBeNull();
  });

  it("FINDING: extractImageDimensions does not validate IHDR chunk type and accepts corrupted non-IHDR chunks", () => {
    const pngCorruptChunk = new Uint8Array(24);
    pngCorruptChunk.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], 0);
    pngCorruptChunk.set([0x00, 0x00, 0x00, 0x0d], 8); // chunk len
    pngCorruptChunk.set([0x58, 0x58, 0x58, 0x58], 12); // "XXXX" invalid chunk type instead of "IHDR"
    pngCorruptChunk.set([0x00, 0x00, 0x01, 0x00], 16); // width 256
    pngCorruptChunk.set([0x00, 0x00, 0x01, 0x00], 20); // height 256

    // The function assumes bytes 16..23 are dimensions without checking chunk type "IHDR"
    const dims = extractImageDimensions(pngCorruptChunk, "png");
    expect(dims).toEqual({ width: 256, height: 256 });
  });

  it("FINDING: signed 32-bit overflow in extractImageDimensions causes high-bit dimensions to return null", () => {
    const pngHighBit = new Uint8Array(24);
    pngHighBit.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], 0);
    pngHighBit.set([0x00, 0x00, 0x00, 0x0d], 8);
    pngHighBit.set([0x49, 0x48, 0x44, 0x52], 12); // "IHDR"
    pngHighBit.set([0x80, 0x00, 0x00, 0x01], 16); // 0x80000001 (high bit set)
    pngHighBit.set([0x00, 0x00, 0x00, 0x10], 20); // 16

    // In JS bitwise OR, (0x80 << 24) is negative (-2147483648), failing `width > 0`
    const dims = extractImageDimensions(pngHighBit, "png");
    expect(dims).toBeNull();
  });

  it("robustly parses mixed hex, octal, and escaped ASCII sequences", () => {
    // Postgres escape format \x89PNG\r\n\x1a\n
    const sample = "\\x89PNG\\r\\n\\x1a\\n";
    const res = parseBinaryBytes(sample);
    expect(res.encoding).toBe("escaped");
    expect(Array.from(res.bytes)).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);

    // Octal byte sequences
    const octal = "\\000\\101\\102"; // \0, 'A', 'B'
    const octRes = parseBinaryBytes(octal);
    expect(Array.from(octRes.bytes)).toEqual([0, 65, 66]);

    // Non-hex characters after \x fall back to literal characters
    const broken = "\\xG1\\x41";
    const brokenRes = parseBinaryBytes(broken);
    expect(brokenRes.encoding).toBe("escaped");
    expect(Array.from(brokenRes.bytes)).toEqual([92, 120, 71, 49, 65]);
  });

  it("handles odd-length hex and invalid base64 padding without throwing", () => {
    // Odd-length hex strings fall back cleanly to escaped encoding
    const resOdd0x = parseBinaryBytes("0x123");
    expect(resOdd0x.encoding).toBe("escaped");
    expect(resOdd0x.bytes).toHaveLength(5); // "0x123" ASCII

    const resOddSlash = parseBinaryBytes("\\x123");
    expect(resOddSlash.encoding).toBe("escaped");
    expect(resOddSlash.bytes).toHaveLength(2); // 0x12, '3'

    const resOddPlain = parseBinaryBytes("12345");
    expect(resOddPlain.encoding).toBe("escaped");
    expect(resOddPlain.bytes).toHaveLength(5);

    // Invalid base64 padding falls back to escaped characters
    const badBase64 = parseBinaryBytes("AAAAA===");
    expect(badBase64.encoding).toBe("escaped");
    expect(badBase64.bytes).toHaveLength(8);
  });

  it("truncates massive input buffers at maxBytes limit", () => {
    const huge = new Uint8Array(200 * 1024); // 200 KB
    huge.fill(0xaa);

    const parsed = parseBinaryBytes(huge, 1024);
    expect(parsed.totalBytes).toBe(200 * 1024);
    expect(parsed.bytes.length).toBe(1024);
    expect(parsed.truncated).toBe(true);

    const lines = formatHexDumpLines(parsed.bytes.slice(0, 32));
    expect(lines).toHaveLength(2);
    expect(lines[0].offsetHex).toBe("00000000:");
    expect(lines[1].offsetHex).toBe("00000010:");
  });
});

describe("Milestone 4 Adversarial Stress Harness: datetime-utils.ts", () => {
  it("parseDateTime accurately preserves year 0000..0099 without timezone using setUTCFullYear", () => {
    // With setUTCFullYear adjustment, year 0 without timezone preserves year 0 accurately
    const y0NoTz = parseDateTime("0000-01-01");
    expect(y0NoTz).not.toBeNull();
    if (y0NoTz) {
      expect(y0NoTz.date.getUTCFullYear()).toBe(0);
      expect(y0NoTz.epochMs).toBe(-62167219200000); // 0000-01-01 ms
    }

    // With explicit timezone, year 0 is also preserved
    const y0WithTz = parseDateTime("0000-01-01T00:00:00Z");
    expect(y0WithTz).not.toBeNull();
    if (y0WithTz) {
      expect(y0WithTz.date.getUTCFullYear()).toBe(0);
      expect(y0WithTz.epochMs).toBe(-62167219200000); // 0000-01-01 ms
    }
  });

  it("parses year 9999 boundary correctly", () => {
    const y9999 = parseDateTime("9999-12-31T23:59:59.999Z");
    expect(y9999).not.toBeNull();
    if (y9999) {
      expect(y9999.date.getUTCFullYear()).toBe(9999);
      expect(y9999.epochMs).toBe(253402300799999);
    }
  });

  it("FINDING: negative epoch ms before year 1653 is misclassified as microseconds in parseEpochNumber", () => {
    // Year 0 in milliseconds is -62167219200000.
    // Because Math.abs(-62167219200000) > 1e13, parseEpochNumber classifies it as 'us' (microseconds)
    // and divides by 1000, turning it into -62167219200 ms (year 1968)!
    const res = parseDateTime(-62167219200000);
    expect(res).not.toBeNull();
    if (res) {
      expect(res.originalUnit).toBe("us");
      expect(res.date.getUTCFullYear()).toBe(1968);
    }
  });

  it("FINDING: leap seconds (second 60) return null due to V8 Date parser rejection", () => {
    // 2016-12-31 had an official UTC leap second at 23:59:60
    const leapSec = parseDateTime("2016-12-31T23:59:60Z");
    // Standard JS Date fails on 23:59:60 and returns NaN, causing parseDateTime to return null
    expect(leapSec).toBeNull();
  });

  it("handles leap day 2024-02-29 and non-leap 2023-02-29 rollovers", () => {
    const leap2024 = parseDateTime("2024-02-29");
    expect(leap2024).not.toBeNull();
    if (leap2024) {
      expect(leap2024.date.toISOString()).toBe("2024-02-29T00:00:00.000Z");
      const cal = getCalendarBreakdown(leap2024.date);
      expect(cal.isLeapYear).toBe(true);
      expect(cal.totalDaysInYear).toBe(366);
      expect(cal.dayOfYear).toBe(60);
    }

    // 2023 is not a leap year: Date.UTC rolls 2023-02-29 over to 2023-03-01
    const nonLeap = parseDateTime("2023-02-29");
    expect(nonLeap).not.toBeNull();
    if (nonLeap) {
      expect(nonLeap.date.toISOString()).toBe("2023-03-01T00:00:00.000Z");
    }
  });

  it("preserves nanosecond fractional precision string", () => {
    const nanoWithZ = parseDateTime("2026-11-05T15:30:00.123456789Z");
    expect(nanoWithZ).not.toBeNull();
    if (nanoWithZ) {
      expect(nanoWithZ.nanos).toBe("123456789");
      expect(nanoWithZ.epochMs).toBe(1793892600123);
    }

    const nanoNoZ = parseDateTime("2026-11-05 15:30:00.987654321");
    expect(nanoNoZ).not.toBeNull();
    if (nanoNoZ) {
      expect(nanoNoZ.nanos).toBe("987654321");
      expect(nanoNoZ.epochMs).toBe(1793892600987);
    }
  });

  it("formats extreme timezones: UTC+14, UTC-12, and non-hour offsets", () => {
    const date = new Date("2026-11-05T15:30:00Z");

    // UTC+14 Line Islands
    const lineIslands = formatInTimezone(date, "Pacific/Kiritimati");
    expect(lineIslands).toContain("GMT+14");
    expect(lineIslands).toContain("2026-11-06");

    // UTC-12 Baker Island
    const baker = formatInTimezone(date, "Etc/GMT+12");
    expect(baker).toContain("GMT-12");
    expect(baker).toContain("03:30:00");

    // UTC+05:45 Nepal
    const nepal = formatInTimezone(date, "Asia/Kathmandu");
    expect(nepal).toContain("GMT+5:45");
    expect(nepal).toContain("21:15:00");

    // UTC+12:45 / +13:45 Chatham
    const chatham = formatInTimezone(date, "Pacific/Chatham");
    expect(chatham).toContain("GMT+13:45");
  });

  it("formatInTimezone safely formats Invalid Date returning 'Invalid Date' without throwing RangeError", () => {
    const invalidDate = new Date(NaN);
    expect(formatInTimezone(invalidDate, "UTC")).toBe("Invalid Date");
    expect(formatInTimezone(invalidDate, "Invalid/Timezone")).toBe("Invalid Date");
  });
});
