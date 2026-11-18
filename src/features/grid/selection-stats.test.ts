import { describe, expect, it } from "vitest";
import { computeSelectionStats } from "./selection-stats";

describe("computeSelectionStats", () => {
  it("returns null for empty or single-cell selections", () => {
    expect(computeSelectionStats([])).toBeNull();
    expect(computeSelectionStats(["42"])).toBeNull();
  });

  it("computes lossless >53-bit integer and scaled decimal Sum, Avg, Min, Max without IEEE-754 precision loss", () => {
    // 9007199254740993 + 9007199254740995 === 18014398509481988 (Number() would lose the last bit!)
    const bigIntStats = computeSelectionStats(["9007199254740993", "9007199254740995", null]);
    expect(bigIntStats).toEqual({
      cellCount: 3,
      nonNullCount: 2,
      distinctCount: 2,
      numericCount: 2,
      sum: "18014398509481988",
      avg: "9007199254740994",
      min: "9007199254740993",
      max: "9007199254740995",
      overLimit: false,
    });

    // Mixed scale decimals including negatives
    const decStats = computeSelectionStats(["-10.25", "20.5", "0.125", "20.5"]);
    expect(decStats).toEqual({
      cellCount: 4,
      nonNullCount: 4,
      distinctCount: 3,
      numericCount: 4,
      sum: "30.875",
      avg: "7.7188",
      min: "-10.25",
      max: "20.5",
      overLimit: false,
    });
  });

  it("handles text-only selections and enforces the cell count cap", () => {
    const textStats = computeSelectionStats(["alpha", null, "beta", "alpha"]);
    expect(textStats).toEqual({
      cellCount: 4,
      nonNullCount: 3,
      distinctCount: 2,
      numericCount: 0,
      sum: null,
      avg: null,
      min: null,
      max: null,
      overLimit: false,
    });

    const capped = computeSelectionStats(["1", "2", "3"], 2);
    expect(capped?.overLimit).toBe(true);
    expect(capped?.cellCount).toBe(3);
  });
});
