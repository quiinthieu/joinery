import { useSyncExternalStore } from "react";

// Volatile only: a local clear must fence every mounted history reader and opener.
let epoch = 0;
let clears = 0;
const listeners = new Set<() => void>();

export const historyAccessEpoch = () => epoch;
export const historyClearPending = () => clears > 0;

function publish() {
  epoch += 1;
  for (const listener of listeners) listener();
}

export function useHistoryAccessEpoch() {
  return useSyncExternalStore((listener) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  }, historyAccessEpoch);
}

export function beginHistoryClear() {
  clears += 1;
  publish();
  let finished = false;
  return () => {
    if (finished) return;
    finished = true;
    clears -= 1;
    publish();
  };
}

export function historySearchError(search: string): string | null {
  return new TextEncoder().encode(search).length > 512
    ? "History searches must be at most 512 UTF-8 bytes."
    : null;
}
