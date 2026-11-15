import { consoleReceiptHeld, type SnowflakeConsoleReceipt } from "./SnowflakeConsole";

export interface ConsoleReceiptReconcilerOptions {
  getReceipts(): Record<string, SnowflakeConsoleReceipt>;
  refresh(clientRequestId: string): Promise<void>;
  handoff(clientRequestId: string): Promise<void>;
  /** Register bounded pending correlations only. Receipt lookups use this
   * scheduler's four-task lane, rather than running inside discovery. */
  discoverPending(): Promise<void>;
  onError(message: string): void;
}

interface Attempt {
  due: number;
  failures: number;
  error?: string;
}

// A disposed component cannot abort a lookup already owned by the client. Keep
// these slots across remounts so the replacement cannot overlap those requests.
const activeReceipts = new Set<string>();
const receiptSlotListeners = new Set<() => void>();
let pendingDiscovery = false;

function needsReconciliation(receipt: SnowflakeConsoleReceipt): boolean {
  return consoleReceiptHeld(receipt) || !receipt.inventoried || !receipt.nativeSettled;
}

function errorMessage(error: unknown): string {
  if (
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }
  return typeof error === "string" ? error : "Could not reconcile console receipts";
}

/** SQL-free callback scheduling only. Disposal does not abort an already-owned
 * lookup, but prevents every subsequent refresh, handoff and discovery call. */
export function startConsoleReceiptReconciler({
  getReceipts,
  refresh,
  handoff,
  discoverPending,
  onError,
}: ConsoleReceiptReconcilerOptions): () => void {
  const attempts = new Map<string, Attempt>();
  let disposed = false;
  let workTimer: ReturnType<typeof setTimeout> | undefined;
  let discoveryTimer: ReturnType<typeof setTimeout> | undefined;
  let workDue = Infinity;
  let scanAt = 0;
  let queue: string[] = [];
  let queueOffset = 0;

  function scheduleWork(delay: number) {
    if (disposed) return;
    const due = Date.now() + delay;
    if (workTimer !== undefined && workDue <= due) return;
    if (workTimer !== undefined) clearTimeout(workTimer);
    workDue = due;
    workTimer = setTimeout(pump, delay);
  }

  function failure(id: string, attempt: Attempt, message: string) {
    attempt.failures += 1;
    attempt.due = Date.now() + Math.min(60_000, 1_000 * 2 ** Math.min(attempt.failures - 1, 6));
    if (attempt.error !== message) {
      attempt.error = message;
      onError(message);
    }
    attempts.set(id, attempt);
  }

  async function reconcile(id: string, attempt: Attempt) {
    try {
      await refresh(id);
      if (disposed) return;
      let current = getReceipts()[id];
      if (!current || !needsReconciliation(current)) {
        attempts.delete(id);
        return;
      }
      // Refresh records native lookup failures in state instead of rejecting.
      // Inspect that fresh state; null/error never counts as settled evidence.
      if (current.error || current.decisionConflict) {
        failure(id, attempt, current.error ?? "Conflicting native console decisions remain held");
        return;
      }
      if (!consoleReceiptHeld(current)) {
        await handoff(id);
        if (disposed) return;
        current = getReceipts()[id];
      }
      if (!current || !needsReconciliation(current)) {
        attempts.delete(id);
        return;
      }
      if (current.error || current.decisionConflict) {
        failure(id, attempt, current.error ?? "Conflicting native console decisions remain held");
        return;
      }
      // A live or still-recording run is not an error. Keep its one-second
      // cadence without accelerating failed/null lookups elsewhere.
      attempts.set(id, { due: Date.now() + 1_000, failures: 0 });
    } catch (error) {
      if (!disposed) failure(id, attempt, errorMessage(error));
    } finally {
      activeReceipts.delete(id);
      // A timer turn between batches keeps even 10,000 immediately-resolved
      // legacy correlations from forming an uninterrupted microtask chain.
      for (const wake of receiptSlotListeners) wake();
    }
  }

  function pump() {
    workTimer = undefined;
    workDue = Infinity;
    if (disposed) return;
    const now = Date.now();
    const scan = now >= scanAt;
    // Even a failing state reader retries on the bounded heartbeat.
    if (scan) scanAt = now + 1_000;
    try {
      const receipts = getReceipts();
      if (scan) {
        for (const id of attempts.keys()) {
          if (!activeReceipts.has(id) && (!receipts[id] || !needsReconciliation(receipts[id]))) {
            attempts.delete(id);
          }
        }
        queue = [];
        queueOffset = 0;
        for (const [id, receipt] of Object.entries(receipts)) {
          if (
            needsReconciliation(receipt) &&
            !activeReceipts.has(id) &&
            (attempts.get(id)?.due ?? 0) <= now
          ) {
            queue.push(id);
          }
        }
      }
      while (activeReceipts.size < 4 && queueOffset < queue.length) {
        const id = queue[queueOffset++];
        const receipt = getReceipts()[id];
        if (!receipt || !needsReconciliation(receipt) || activeReceipts.has(id)) continue;
        const attempt = attempts.get(id) ?? { due: 0, failures: 0 };
        if (attempt.due > now) continue;
        attempts.set(id, attempt);
        activeReceipts.add(id);
        void reconcile(id, attempt);
      }
      if (queueOffset === queue.length) {
        queue = [];
        queueOffset = 0;
      }
    } catch (error) {
      onError(errorMessage(error));
    }
    scheduleWork(Math.max(0, scanAt - Date.now()));
  }

  async function discover() {
    discoveryTimer = undefined;
    if (disposed) return;
    if (pendingDiscovery) {
      discoveryTimer = setTimeout(() => void discover(), 1_000);
      return;
    }
    pendingDiscovery = true;
    try {
      await discoverPending();
    } catch (error) {
      if (!disposed) onError(errorMessage(error));
    } finally {
      pendingDiscovery = false;
      if (!disposed) {
        scheduleWork(0);
        discoveryTimer = setTimeout(() => void discover(), 10_000);
      }
    }
  }

  const wakeForReceiptSlot = () => scheduleWork(0);
  receiptSlotListeners.add(wakeForReceiptSlot);
  scheduleWork(0);
  discoveryTimer = setTimeout(() => void discover(), 0);
  return () => {
    disposed = true;
    if (workTimer !== undefined) clearTimeout(workTimer);
    if (discoveryTimer !== undefined) clearTimeout(discoveryTimer);
    receiptSlotListeners.delete(wakeForReceiptSlot);
    queue = [];
    attempts.clear();
  };
}
