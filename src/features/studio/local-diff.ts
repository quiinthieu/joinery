export type DiffLineKind = "context" | "added" | "removed";

export interface DiffLine {
  kind: DiffLineKind;
  oldLineNo: number | null;
  newLineNo: number | null;
  text: string;
}

export interface LineDiffResult {
  identical: boolean;
  addedCount: number;
  removedCount: number;
  truncated: boolean;
  lines: DiffLine[];
}

const MAX_LCS_LINES = 600;
const MAX_RENDER_LINES = 1200;

/**
 * Computes a readable unified line diff from `older` (snapshot) -> `current` (editor).
 * Bounded in O(N*M) up to 600 lines per side with a linear prefix/suffix fast-path
 * and graceful fallback for larger SQL scripts.
 */
export function computeLineDiff(older: string, current: string): LineDiffResult {
  if (older === current) {
    const split = older.split("\n");
    const capped = split.slice(0, MAX_RENDER_LINES);
    return {
      identical: true,
      addedCount: 0,
      removedCount: 0,
      truncated: split.length > MAX_RENDER_LINES,
      lines: capped.map((text, idx) => ({
        kind: "context",
        oldLineNo: idx + 1,
        newLineNo: idx + 1,
        text,
      })),
    };
  }

  const oldLines = older.split("\n");
  const newLines = current.split("\n");

  let prefix = 0;
  while (
    prefix < oldLines.length &&
    prefix < newLines.length &&
    oldLines[prefix] === newLines[prefix]
  ) {
    prefix++;
  }

  let oldEnd = oldLines.length - 1;
  let newEnd = newLines.length - 1;
  while (oldEnd >= prefix && newEnd >= prefix && oldLines[oldEnd] === newLines[newEnd]) {
    oldEnd--;
    newEnd--;
  }

  const midOld = oldLines.slice(prefix, oldEnd + 1);
  const midNew = newLines.slice(prefix, newEnd + 1);

  const out: DiffLine[] = [];
  for (let i = 0; i < prefix; i++) {
    out.push({
      kind: "context",
      oldLineNo: i + 1,
      newLineNo: i + 1,
      text: oldLines[i]!,
    });
  }

  let addedCount = 0;
  let removedCount = 0;

  if (midOld.length <= MAX_LCS_LINES && midNew.length <= MAX_LCS_LINES) {
    const m = midOld.length;
    const n = midNew.length;
    const dp = Array.from({ length: m + 1 }, () => new Uint16Array(n + 1));
    for (let i = m - 1; i >= 0; i--) {
      const row = dp[i]!;
      const nextRow = dp[i + 1]!;
      const oldText = midOld[i]!;
      for (let j = n - 1; j >= 0; j--) {
        row[j] = oldText === midNew[j] ? nextRow[j + 1]! + 1 : Math.max(nextRow[j]!, row[j + 1]!);
      }
    }
    let i = 0;
    let j = 0;
    while (i < m && j < n) {
      if (midOld[i] === midNew[j]) {
        out.push({
          kind: "context",
          oldLineNo: prefix + i + 1,
          newLineNo: prefix + j + 1,
          text: midOld[i]!,
        });
        i++;
        j++;
      } else if (dp[i + 1]![j]! >= dp[i]![j + 1]!) {
        out.push({
          kind: "removed",
          oldLineNo: prefix + i + 1,
          newLineNo: null,
          text: midOld[i]!,
        });
        removedCount++;
        i++;
      } else {
        out.push({
          kind: "added",
          oldLineNo: null,
          newLineNo: prefix + j + 1,
          text: midNew[j]!,
        });
        addedCount++;
        j++;
      }
    }
    while (i < m) {
      out.push({
        kind: "removed",
        oldLineNo: prefix + i + 1,
        newLineNo: null,
        text: midOld[i]!,
      });
      removedCount++;
      i++;
    }
    while (j < n) {
      out.push({
        kind: "added",
        oldLineNo: null,
        newLineNo: prefix + j + 1,
        text: midNew[j]!,
      });
      addedCount++;
      j++;
    }
  } else {
    for (let i = 0; i < midOld.length; i++) {
      out.push({
        kind: "removed",
        oldLineNo: prefix + i + 1,
        newLineNo: null,
        text: midOld[i]!,
      });
      removedCount++;
    }
    for (let j = 0; j < midNew.length; j++) {
      out.push({
        kind: "added",
        oldLineNo: null,
        newLineNo: prefix + j + 1,
        text: midNew[j]!,
      });
      addedCount++;
    }
  }

  const suffixCount = oldLines.length - 1 - oldEnd;
  for (let k = 0; k < suffixCount; k++) {
    out.push({
      kind: "context",
      oldLineNo: oldEnd + 1 + k + 1,
      newLineNo: newEnd + 1 + k + 1,
      text: oldLines[oldEnd + 1 + k]!,
    });
  }

  const truncated = out.length > MAX_RENDER_LINES;
  return {
    identical: false,
    addedCount,
    removedCount,
    truncated,
    lines: truncated ? out.slice(0, MAX_RENDER_LINES) : out,
  };
}
