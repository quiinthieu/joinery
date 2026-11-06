import { describe, expect, it } from "vitest";
import {
  parseBinaryBytes,
  detectImageFormat,
  extractImageDimensions,
  formatHexDumpLines,
  formatByteSize,
} from "./binary-utils";

describe("binary-utils", () => {
  it("parses pure hex strings with \\x and 0x prefix", () => {
    const res1 = parseBinaryBytes("\\x4a6f696e657279");
    expect(res1.encoding).toBe("hex");
    expect(new TextDecoder().decode(res1.bytes)).toBe("Joinery");

    const res2 = parseBinaryBytes("0x4a6f696e657279");
    expect(res2.encoding).toBe("hex");
    expect(new TextDecoder().decode(res2.bytes)).toBe("Joinery");
  });

  it("parses escaped hex strings with mixed ASCII like \\x89PNG\\r\\n...", () => {
    const sample =
      "\\x89PNG\\r\\n\\x1a\\n\\x00\\x00\\x00\\rIHDR\\x00\\x00\\x00\\x01\\x00\\x00\\x00\\x01";
    const res = parseBinaryBytes(sample);
    expect(res.encoding).toBe("escaped");
    expect(res.bytes[0]).toBe(0x89);
    expect(res.bytes[1]).toBe(0x50); // P
    expect(res.bytes[2]).toBe(0x4e); // N
    expect(res.bytes[3]).toBe(0x47); // G
    expect(res.bytes[4]).toBe(0x0d); // \r
    expect(res.bytes[5]).toBe(0x0a); // \n
    expect(res.bytes[6]).toBe(0x1a); // \x1a
    expect(res.bytes[7]).toBe(0x0a); // \n

    const format = detectImageFormat(res.bytes);
    expect(format?.format).toBe("png");
    expect(format?.mime).toBe("image/png");

    const dims = extractImageDimensions(res.bytes, "png");
    expect(dims).toEqual({ width: 1, height: 1 });
  });

  it("detects JPEG, GIF, WebP, and SVG magic headers", () => {
    // JPEG: FF D8 FF
    const jpegBytes = new Uint8Array([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]);
    expect(detectImageFormat(jpegBytes)?.format).toBe("jpeg");

    // GIF: GIF89a
    const gifBytes = new TextEncoder().encode("GIF89a\x0a\x00\x0a\x00");
    const gifFormat = detectImageFormat(gifBytes);
    expect(gifFormat?.format).toBe("gif");
    const gifDims = extractImageDimensions(gifBytes, "gif");
    expect(gifDims).toEqual({ width: 10, height: 10 });

    // WebP: RIFF....WEBP
    const webpBytes = new TextEncoder().encode("RIFF\x20\x00\x00\x00WEBPVP8 ");
    expect(detectImageFormat(webpBytes)?.format).toBe("webp");

    // SVG: <svg
    const svgStr = '<svg width="200" height="150" viewBox="0 0 200 150"></svg>';
    const svgRes = parseBinaryBytes(svgStr);
    expect(svgRes.encoding).toBe("text");
    expect(detectImageFormat(svgRes.bytes)?.format).toBe("svg");
    const svgDims = extractImageDimensions(svgRes.bytes, "svg");
    expect(svgDims).toEqual({ width: 200, height: 150 });
  });

  it("parses base64 encoded strings", () => {
    // "Joinery" in base64: Sm9pbmVyeQ==
    const res = parseBinaryBytes("Sm9pbmVyeQ==");
    expect(res.encoding).toBe("base64");
    expect(new TextDecoder().decode(res.bytes)).toBe("Joinery");
  });

  it("formats classic 3-column hex lines", () => {
    const bytes = new TextEncoder().encode("Hello Joinery!");
    const lines = formatHexDumpLines(bytes, true);
    expect(lines).toHaveLength(1);
    expect(lines[0].offsetHex).toBe("00000000:");
    expect(lines[0].hexGroup1).toBe("48 65 6c 6c 6f 20 4a 6f");
    expect(lines[0].ascii).toBe("Hello Joinery!  ");
  });

  it("formats byte size accurately", () => {
    expect(formatByteSize(0)).toBe("0 B");
    expect(formatByteSize(512)).toBe("512 bytes");
    expect(formatByteSize(2048)).toContain("2.0 KB");
  });
});
