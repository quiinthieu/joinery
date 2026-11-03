import type { IndexInfo, TableStorageStats } from "../../ipc/generated";

export function formatStorageBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes) || bytes < 0) {
    return "—";
  }
  if (bytes < 1024) {
    return `${Math.round(bytes).toLocaleString()} B`;
  }
  const kib = bytes / 1024;
  if (kib < 1024) {
    return `${kib.toFixed(1)} KiB`;
  }
  const mib = kib / 1024;
  if (mib < 1024) {
    return `${mib.toFixed(1)} MiB`;
  }
  const gib = mib / 1024;
  return `${gib.toFixed(2)} GiB`;
}

export function formatStatCount(count: number | null | undefined): string {
  if (count === null || count === undefined || !Number.isFinite(count) || count < 0) {
    return "—";
  }
  return Math.round(count).toLocaleString();
}

export interface IndexUsageSummary {
  label: string;
  unused: boolean;
  detail: string | null;
}

export function describeIndexUsage(
  index: Pick<IndexInfo, "scans" | "tuplesRead" | "primary" | "unique">,
): IndexUsageSummary {
  const scans = index.scans;
  if (scans === null || scans === undefined || !Number.isFinite(scans) || scans < 0) {
    return { label: "—", unused: false, detail: null };
  }
  const rounded = Math.round(scans);
  if (rounded === 0) {
    if (index.primary || index.unique) {
      return { label: "0 scans", unused: false, detail: "enforces constraint" };
    }
    return { label: "Unused (0 scans)", unused: true, detail: null };
  }
  const read = index.tuplesRead;
  const detail =
    read !== null && read !== undefined && Number.isFinite(read) && read >= 0
      ? `${Math.round(read).toLocaleString()} read`
      : null;
  return {
    label: `${rounded.toLocaleString()} scan${rounded === 1 ? "" : "s"}`,
    unused: false,
    detail,
  };
}

export function deadTupleSummary(
  storage: Pick<TableStorageStats, "liveTuples" | "deadTuples">,
): { label: string; bloated: boolean } | null {
  const live = storage.liveTuples;
  const dead = storage.deadTuples;
  if (
    (live === null || live === undefined || !Number.isFinite(live)) &&
    (dead === null || dead === undefined || !Number.isFinite(dead))
  ) {
    return null;
  }
  const liveCount =
    live !== null && live !== undefined && Number.isFinite(live)
      ? Math.max(0, Math.round(live))
      : 0;
  const deadCount =
    dead !== null && dead !== undefined && Number.isFinite(dead)
      ? Math.max(0, Math.round(dead))
      : 0;
  const total = liveCount + deadCount;
  const ratio = total > 0 ? deadCount / total : 0;
  const pct = total > 0 ? ` (${(ratio * 100).toFixed(1)}% dead)` : "";
  return {
    label: `${liveCount.toLocaleString()} live · ${deadCount.toLocaleString()} dead${pct}`,
    bloated: deadCount >= 100 && ratio >= 0.2,
  };
}
