import type { LockWait } from "../../ipc/generated";

export interface LockSummary {
  /** Waiting session -> sessions it waits on. */
  waitingOn: Map<string, string[]>;
  /** Blocking session -> sessions that wait on it directly. */
  blocks: Map<string, string[]>;
  /** Blockers that are not waiting themselves, most sessions blocked first. */
  roots: string[];
  /** Sessions blocked directly or indirectly by each blocker. */
  blockedCount: Map<string, number>;
  /** Sessions that wait on themselves through a chain (deadlock candidates). */
  cycle: string[];
}

export function summarizeLocks(waits: readonly LockWait[]): LockSummary {
  const waitingOn = new Map<string, string[]>();
  const blocks = new Map<string, string[]>();
  for (const wait of waits) {
    waitingOn.set(wait.waitingId, [...wait.blockingIds]);
    for (const blocker of wait.blockingIds) {
      const list = blocks.get(blocker) ?? [];
      if (!list.includes(wait.waitingId)) list.push(wait.waitingId);
      blocks.set(blocker, list);
    }
  }
  const reach = (start: string) => {
    const seen = new Set<string>();
    const queue = [...(blocks.get(start) ?? [])];
    while (queue.length) {
      const next = queue.shift()!;
      if (seen.has(next)) continue;
      seen.add(next);
      queue.push(...(blocks.get(next) ?? []));
    }
    return seen;
  };
  const blockedCount = new Map<string, number>();
  const cycle: string[] = [];
  for (const blocker of blocks.keys()) {
    const reached = reach(blocker);
    reached.delete(blocker);
    blockedCount.set(blocker, reached.size);
  }
  for (const waiting of waitingOn.keys()) {
    if (reach(waiting).has(waiting)) cycle.push(waiting);
  }
  const roots = [...blocks.keys()]
    .filter((id) => !waitingOn.has(id))
    .sort(
      (a, b) =>
        (blockedCount.get(b) ?? 0) - (blockedCount.get(a) ?? 0) ||
        a.localeCompare(b, undefined, { numeric: true }),
    );
  cycle.sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
  return { waitingOn, blocks, roots, blockedCount, cycle };
}
