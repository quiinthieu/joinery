import type { SnowflakeConsolePlan, SnowflakeConsoleRunView, ErrorCode } from "../../ipc/generated";

export interface SnowflakeConsoleReceipt {
  clientRequestId: string;
  sessionId?: string;
  connectionId?: string;
  planDigest?: string;
  stepCount?: number;
  tabId?: string;
  tabRevision?: number;
  runId?: string;
  view?: SnowflakeConsoleRunView;
  lookupPending: boolean;
  startPending?: boolean;
  cancellationRequested?: boolean;
  error?: string;
  notAdmitted?: ErrorCode;
}

const RECEIPT_KEY = "joinery.snowflakeConsoleReceipts.v1";
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Only opaque correlation IDs survive reload. Never SQL, parameters or results. */
export function restoredConsoleReceipts(): Record<string, SnowflakeConsoleReceipt> {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(RECEIPT_KEY) ?? "[]");
    if (!Array.isArray(raw) || raw.length > 10_000) return {};
    return Object.fromEntries(
      raw
        .filter((id): id is string => typeof id === "string" && uuid.test(id))
        .map((clientRequestId) => [clientRequestId, { clientRequestId, lookupPending: true }]),
    );
  } catch {
    return {};
  }
}

export function rememberConsoleReceipts(receipts: Record<string, SnowflakeConsoleReceipt>): void {
  // Capacity fails closed. Do not silently evict unresolved correlation IDs.
  const ids = Object.values(receipts)
    .filter((receipt) => !receipt.notAdmitted)
    .map((receipt) => receipt.clientRequestId);
  if (ids.length > 10_000)
    throw new Error("Console receipt capacity reached; existing receipts were preserved.");
  localStorage.setItem(RECEIPT_KEY, JSON.stringify(ids));
}

export function consoleReceiptHeld(receipt: SnowflakeConsoleReceipt): boolean {
  if (receipt.notAdmitted) return false;
  return (
    (receipt.startPending === true && !receipt.view) ||
    receipt.lookupPending ||
    !receipt.view ||
    receipt.view.pendingRecording ||
    receipt.view.run.state !== "terminal"
  );
}

/** Until lookup establishes scope, an opaque restored receipt holds every console. */
export function consoleReceiptBlocksSession(
  receipt: SnowflakeConsoleReceipt,
  sessionId: string,
): boolean {
  return consoleReceiptHeld(receipt) && (!receipt.sessionId || receipt.sessionId === sessionId);
}

export function validateConsolePlan(plan: SnowflakeConsolePlan, sessionId: string): void {
  if (
    plan.review.sessionId !== sessionId ||
    !/^[a-f0-9]{64}$/.test(plan.digest) ||
    (plan.mode !== "parsedStatements" && plan.mode !== "selectedUnit") ||
    plan.review.statements.length === 0 ||
    plan.review.statements.length > 100 ||
    (plan.mode === "selectedUnit" &&
      (plan.review.statements.length !== 1 || plan.validations[0] !== "serverAtExecution")) ||
    plan.validations.length !== plan.review.statements.length ||
    plan.validations.some((value) => value !== "localSyntax" && value !== "serverAtExecution")
  )
    throw new Error("Invalid reviewed console plan; no SQL was started.");
}

export function validateConsoleReceipt(
  receipt: SnowflakeConsoleReceipt,
  view: SnowflakeConsoleRunView,
): void {
  const { metadata } = view.run;
  if (
    !uuid.test(metadata.runId) ||
    !Number.isInteger(metadata.stepCount) ||
    metadata.stepCount < 1 ||
    metadata.stepCount > 100 ||
    view.run.steps.length > metadata.stepCount ||
    view.run.steps.some((step, index) => step.ordinal !== index) ||
    (view.run.state === "terminal") !== (view.run.terminal !== null)
  )
    throw new Error("Malformed console receipt; previous evidence was preserved.");
  if (
    metadata.clientRequestId !== receipt.clientRequestId ||
    (receipt.sessionId !== undefined && metadata.sessionId !== receipt.sessionId) ||
    (receipt.connectionId !== undefined && metadata.connectionId !== receipt.connectionId) ||
    (receipt.planDigest !== undefined && metadata.planDigest !== receipt.planDigest) ||
    (receipt.stepCount !== undefined && metadata.stepCount !== receipt.stepCount) ||
    (receipt.runId !== undefined && metadata.runId !== receipt.runId) ||
    (receipt.view !== undefined && view.run.revision < receipt.view.run.revision) ||
    view.results.some(
      (result) =>
        result.runId !== metadata.runId ||
        !Number.isInteger(result.statementIndex) ||
        result.statementIndex < 0 ||
        result.statementIndex >= metadata.stepCount,
    )
  )
    throw new Error("Receipt identity changed; the previous evidence was preserved.");
}

export function consoleRunStatus(view: SnowflakeConsoleRunView) {
  if (view.pendingRecording || view.run.state !== "terminal") return "running" as const;
  switch (view.run.terminal?.reason) {
    case "completed":
      return "completed" as const;
    case "resultLimit":
      return "stopped" as const;
    case "cancelled":
      return "cancelled" as const;
    case "outcomeUnknown":
    case "interrupted":
      return "outcomeUnknown" as const;
    default:
      return "failed" as const;
  }
}
