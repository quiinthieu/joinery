export const DEFAULT_HEX_MAX_BYTES = 64 * 1024; // 64 KB

export type ImageFormat = "png" | "jpeg" | "gif" | "webp" | "svg";

export interface ImageFormatInfo {
  format: ImageFormat;
  mime: string;
  label: string;
}

export interface BinaryParseResult {
  bytes: Uint8Array;
  totalBytes: number;
  truncated: boolean;
  encoding: "hex" | "base64" | "escaped" | "text" | "raw";
}

export interface HexDumpLine {
  offset: number;
  offsetHex: string; // e.g. "00000000:"
  bytes: (number | null)[]; // exactly 16 entries (null for padded padding)
  hexGroup1: string; // 8 bytes hex
  hexGroup2: string; // 8 bytes hex
  ascii: string; // 16 printable characters
}

export interface ImageDimensions {
  width: number;
  height: number;
}

/**
 * Robust binary parser supporting:
 * - \x... and 0x... pure hex
 * - Escaped string literals with \xHH, \r, \n, \t, \0, \\, \ooo
 * - Base64 strings (with or without data URI prefix)
 * - Plain hex strings without prefix
 * - SVG XML text
 */
export function parseBinaryBytes(
  raw: string | null | Uint8Array,
  maxBytes = DEFAULT_HEX_MAX_BYTES,
): BinaryParseResult {
  if (raw === null || raw === undefined) {
    return { bytes: new Uint8Array(0), totalBytes: 0, truncated: false, encoding: "raw" };
  }

  if (raw instanceof Uint8Array) {
    const totalBytes = raw.length;
    const truncated = totalBytes > maxBytes;
    return {
      bytes: truncated ? raw.slice(0, maxBytes) : raw,
      totalBytes,
      truncated,
      encoding: "raw",
    };
  }

  let str = raw.trim();

  // Strip data URL scheme if present (e.g. data:image/png;base64,...)
  if (str.startsWith("data:") && str.includes(";base64,")) {
    str = str.slice(str.indexOf(";base64,") + 8).trim();
  }

  // 1. Check for standard pure hex prefix (\x, 0x)
  if (
    str.startsWith("\\x") ||
    str.startsWith("\\X") ||
    str.startsWith("0x") ||
    str.startsWith("0X")
  ) {
    const body = str.slice(2);
    // If entirely hex digits and even length
    if (body.length % 2 === 0 && /^[0-9a-fA-F]*$/.test(body)) {
      const totalBytes = body.length / 2;
      const previewLen = Math.min(totalBytes, maxBytes);
      const bytes = new Uint8Array(previewLen);
      for (let i = 0; i < previewLen; i++) {
        bytes[i] = Number.parseInt(body.slice(i * 2, i * 2 + 2), 16);
      }
      return { bytes, totalBytes, truncated: totalBytes > maxBytes, encoding: "hex" };
    }
    // Otherwise fallback to escape parser below (handles \x89PNG\r\n...)
  }

  // 2. Check for pure hex without prefix (even length >= 4, pure hex characters)
  if (str.length >= 4 && str.length % 2 === 0 && /^[0-9a-fA-F]+$/.test(str)) {
    const totalBytes = str.length / 2;
    const previewLen = Math.min(totalBytes, maxBytes);
    const bytes = new Uint8Array(previewLen);
    for (let i = 0; i < previewLen; i++) {
      bytes[i] = Number.parseInt(str.slice(i * 2, i * 2 + 2), 16);
    }
    return { bytes, totalBytes, truncated: totalBytes > maxBytes, encoding: "hex" };
  }

  // 3. Check for SVG XML
  if (/^\s*(<\?xml[^>]*\?>\s*)?(<!--[\s\S]*?-->\s*)*<svg/i.test(str)) {
    const encoded = new TextEncoder().encode(str);
    const totalBytes = encoded.length;
    return {
      bytes: totalBytes > maxBytes ? encoded.slice(0, maxBytes) : encoded,
      totalBytes,
      truncated: totalBytes > maxBytes,
      encoding: "text",
    };
  }

  // 4. Check for Base64 (length multiple of 4, valid chars, decodes cleanly)
  if (str.length >= 8 && /^[A-Za-z0-9+/]+={0,2}$/.test(str.replace(/\s+/g, ""))) {
    try {
      const clean = str.replace(/\s+/g, "");
      const binaryString = atob(clean);
      const totalBytes = binaryString.length;
      const previewLen = Math.min(totalBytes, maxBytes);
      const bytes = new Uint8Array(previewLen);
      for (let i = 0; i < previewLen; i++) {
        bytes[i] = binaryString.charCodeAt(i) & 0xff;
      }
      return { bytes, totalBytes, truncated: totalBytes > maxBytes, encoding: "base64" };
    } catch {
      // not base64, proceed to escape tokenizer
    }
  }

  // 5. C / PostgreSQL Escape Tokenizer (handles \x89PNG\r\n\x1a\n...)
  const preview: number[] = [];
  let totalBytes = 0;
  let i = 0;

  while (i < str.length) {
    if (str[i] === "\\" && i + 1 < str.length) {
      const next = str[i + 1];
      // \xHH hex byte
      if (
        (next === "x" || next === "X") &&
        i + 3 < str.length &&
        /^[0-9a-fA-F]{2}$/.test(str.slice(i + 2, i + 4))
      ) {
        const byte = Number.parseInt(str.slice(i + 2, i + 4), 16);
        if (preview.length < maxBytes) preview.push(byte);
        totalBytes++;
        i += 4;
        continue;
      }
      // Standard escape characters
      if (next === "r") {
        if (preview.length < maxBytes) preview.push(0x0d);
        totalBytes++;
        i += 2;
        continue;
      }
      if (next === "n") {
        if (preview.length < maxBytes) preview.push(0x0a);
        totalBytes++;
        i += 2;
        continue;
      }
      if (next === "t") {
        if (preview.length < maxBytes) preview.push(0x09);
        totalBytes++;
        i += 2;
        continue;
      }
      if (next === "0" && (i + 2 >= str.length || !/^[0-7]$/.test(str[i + 2]))) {
        if (preview.length < maxBytes) preview.push(0x00);
        totalBytes++;
        i += 2;
        continue;
      }
      if (next === "\\") {
        if (preview.length < maxBytes) preview.push(0x5c);
        totalBytes++;
        i += 2;
        continue;
      }
      // \ooo octal byte
      if (
        /^[0-3]$/.test(next) &&
        i + 3 < str.length &&
        /^[0-7]{2}$/.test(str.slice(i + 2, i + 4))
      ) {
        const byte = Number.parseInt(str.slice(i + 1, i + 4), 8);
        if (preview.length < maxBytes) preview.push(byte);
        totalBytes++;
        i += 4;
        continue;
      }
    }

    const code = str.charCodeAt(i);
    // In Latin-1 / byte strings, characters 0..255 represent raw bytes
    if (code <= 255) {
      if (preview.length < maxBytes) preview.push(code);
      totalBytes++;
      i++;
    } else {
      // UTF-8 code point
      const ch = String.fromCodePoint(str.codePointAt(i)!);
      const encoded = new TextEncoder().encode(ch);
      for (const b of encoded) {
        if (preview.length < maxBytes) preview.push(b);
        totalBytes++;
      }
      i += ch.length;
    }
  }

  return {
    bytes: Uint8Array.from(preview),
    totalBytes,
    truncated: totalBytes > maxBytes,
    encoding: "escaped",
  };
}

/**
 * Detects image format by checking magic bytes.
 */
export function detectImageFormat(bytes: Uint8Array): ImageFormatInfo | null {
  if (bytes.length >= 8) {
    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if (
      bytes[0] === 0x89 &&
      bytes[1] === 0x50 &&
      bytes[2] === 0x4e &&
      bytes[3] === 0x47 &&
      bytes[4] === 0x0d &&
      bytes[5] === 0x0a &&
      bytes[6] === 0x1a &&
      bytes[7] === 0x0a
    ) {
      return { format: "png", mime: "image/png", label: "PNG Image" };
    }
  }

  if (bytes.length >= 3) {
    // JPEG: FF D8 FF
    if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) {
      return { format: "jpeg", mime: "image/jpeg", label: "JPEG Image" };
    }
  }

  if (bytes.length >= 6) {
    // GIF: GIF87a or GIF89a
    const sig = String.fromCharCode(...bytes.slice(0, 6));
    if (sig === "GIF87a" || sig === "GIF89a") {
      return { format: "gif", mime: "image/gif", label: "GIF Image" };
    }
  }

  if (bytes.length >= 12) {
    // WebP: RIFF....WEBP
    const riff = String.fromCharCode(...bytes.slice(0, 4));
    const webp = String.fromCharCode(...bytes.slice(8, 12));
    if (riff === "RIFF" && webp === "WEBP") {
      return { format: "webp", mime: "image/webp", label: "WebP Image" };
    }
  }

  // SVG check
  if (bytes.length > 0) {
    const textSample = new TextDecoder("utf-8", { fatal: false }).decode(
      bytes.slice(0, Math.min(bytes.length, 1024)),
    );
    if (/^\s*(<\?xml[^>]*\?>\s*)?(<!--[\s\S]*?-->\s*)*<svg/i.test(textSample)) {
      return { format: "svg", mime: "image/svg+xml", label: "SVG Vector Image" };
    }
  }

  return null;
}

/**
 * Extracts width and height directly from image headers if available.
 */
export function extractImageDimensions(
  bytes: Uint8Array,
  format: ImageFormat,
): ImageDimensions | null {
  try {
    if (format === "png" && bytes.length >= 24) {
      // PNG: IHDR chunk width (16..19) and height (20..23) big-endian
      const width = (bytes[16] << 24) | (bytes[17] << 16) | (bytes[18] << 8) | bytes[19];
      const height = (bytes[20] << 24) | (bytes[21] << 16) | (bytes[22] << 8) | bytes[23];
      if (width > 0 && height > 0) return { width, height };
    }

    if (format === "gif" && bytes.length >= 10) {
      // GIF: Logical screen width (6..7) and height (8..9) little-endian
      const width = bytes[6] | (bytes[7] << 8);
      const height = bytes[8] | (bytes[9] << 8);
      if (width > 0 && height > 0) return { width, height };
    }

    if (format === "svg") {
      const text = new TextDecoder("utf-8", { fatal: false }).decode(bytes.slice(0, 2048));
      const wMatch = /width=["'](\d+(?:\.\d+)?)/i.exec(text);
      const hMatch = /height=["'](\d+(?:\.\d+)?)/i.exec(text);
      if (wMatch && hMatch) {
        return { width: Math.round(Number(wMatch[1])), height: Math.round(Number(hMatch[1])) };
      }
      const vbMatch = /viewBox=["']\s*\d+\s+\d+\s+(\d+(?:\.\d+)?)\s+(\d+(?:\.\d+)?)/i.exec(text);
      if (vbMatch) {
        return { width: Math.round(Number(vbMatch[1])), height: Math.round(Number(vbMatch[2])) };
      }
    }
  } catch {
    // fallback to image element onload
  }
  return null;
}

/**
 * Formats bytes into classic 3-column hex lines.
 */
export function formatHexDumpLines(bytes: Uint8Array, colon = true): HexDumpLine[] {
  if (bytes.length === 0) return [];
  const lines: HexDumpLine[] = [];

  for (let offset = 0; offset < bytes.length; offset += 16) {
    const rowEnd = Math.min(offset + 16, bytes.length);
    const offsetHex = `${offset.toString(16).padStart(8, "0")}${colon ? ":" : " "}`;

    const lineBytes: (number | null)[] = [];
    const hexCells: string[] = [];
    let ascii = "";

    for (let i = 0; i < 16; i++) {
      const idx = offset + i;
      if (idx < rowEnd) {
        const b = bytes[idx];
        lineBytes.push(b);
        hexCells.push(b.toString(16).padStart(2, "0"));
        ascii += b >= 0x20 && b <= 0x7e ? String.fromCharCode(b) : ".";
      } else {
        lineBytes.push(null);
        hexCells.push("  ");
        ascii += " ";
      }
    }

    const hexGroup1 = hexCells.slice(0, 8).join(" ");
    const hexGroup2 = hexCells.slice(8, 16).join(" ");

    lines.push({
      offset,
      offsetHex,
      bytes: lineBytes,
      hexGroup1,
      hexGroup2,
      ascii,
    });
  }

  return lines;
}

/**
 * Converts a byte buffer to a browser Data URL for <img> rendering.
 */
export function bytesToDataUrl(bytes: Uint8Array, mime: string): string {
  let binary = "";
  const len = bytes.byteLength;
  for (let i = 0; i < len; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return `data:${mime};base64,${btoa(binary)}`;
}

export function formatByteSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  if (bytes === 1) return "1 byte";
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024)
    return `${(bytes / 1024).toFixed(1)} KB (${bytes.toLocaleString()} bytes)`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB (${bytes.toLocaleString()} bytes)`;
}
