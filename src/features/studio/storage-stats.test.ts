import { describe, expect, it } from "vitest";
import {
  deadTupleSummary,
  describeIndexUsage,
  formatStatCount,
  formatStorageBytes,
} from "./storage-stats";

describe("storage-stats helpers", () => {
  it("formats byte sizes across B, KiB, MiB, GiB, and unavailable values", () => {
    expect(formatStorageBytes(null)).toBe("—");
    expect(formatStorageBytes(undefined)).toBe("—");
    expect(formatStorageBytes(-1)).toBe("—");
    expect(formatStorageBytes(0)).toBe("0 B");
    expect(formatStorageBytes(512)).toBe("512 B");
    expect(formatStorageBytes(16384)).toBe("16.0 KiB");
    expect(formatStorageBytes(2.5 * 1024 * 1024)).toBe("2.5 MiB");
    expect(formatStorageBytes(1.25 * 1024 * 1024 * 1024)).toBe("1.25 GiB");
    expect(formatStatCount(1250)).toBe("1,250");
    expect(formatStatCount(null)).toBe("—");
  });

  it("distinguishes unused non-unique indexes from zero-scan constraint indexes and active indexes", () => {
    expect(describeIndexUsage({ scans: 0, tuplesRead: 0, primary: false, unique: false })).toEqual({
      label: "Unused (0 scans)",
      unused: true,
      detail: null,
    });
    expect(describeIndexUsage({ scans: 0, tuplesRead: 0, primary: true, unique: true })).toEqual({
      label: "0 scans",
      unused: false,
      detail: "enforces constraint",
    });
    expect(
      describeIndexUsage({ scans: 42, tuplesRead: 120, primary: false, unique: false }),
    ).toEqual({
      label: "42 scans",
      unused: false,
      detail: "120 read",
    });
    expect(
      describeIndexUsage({
        scans: undefined,
        tuplesRead: undefined,
        primary: false,
        unique: false,
      }),
    ).toEqual({
      label: "—",
      unused: false,
      detail: null,
    });
  });

  it("computes dead tuple percentages and flags high dead tuple bloat", () => {
    expect(deadTupleSummary({ liveTuples: null, deadTuples: null })).toBeNull();
    expect(deadTupleSummary({ liveTuples: 900, deadTuples: 100 })).toEqual({
      label: "900 live · 100 dead (10.0% dead)",
      bloated: false,
    });
    expect(deadTupleSummary({ liveTuples: 300, deadTuples: 200 })).toEqual({
      label: "300 live · 200 dead (40.0% dead)",
      bloated: true,
    });
  });
});
