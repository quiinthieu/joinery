import type {
  SnowflakeConsolePlan,
  SnowflakeConsoleRunView,
  SnowflakeConsoleRequestLookup,
  SnowflakeConsoleReceiptSummary,
  SnowflakeConsoleStorage,
  ErrorCode,
} from "../../ipc/generated";

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
  decisionConflict?: boolean;
  inventoried?: boolean;
  nativeSettled?: boolean;
  lastAccessed?: number;
  /** Derived only from a validated durable terminal view, never browser storage. */
  settledProof?: { revision: string; recordedAt: string };
}

const RECEIPT_KEY = "joinery.snowflakeConsoleReceipts.v1";
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
let receiptAccess = 0;
export function consoleReceiptAccess(): number {
  return (receiptAccess = Math.max(Date.now(), receiptAccess + 1));
}

function storedCorrelations(): { ids: string[]; problem: string | null } {
  try {
    const text = localStorage.getItem(RECEIPT_KEY) ?? "[]";
    if (text.length > 400_002) throw new Error("Receipt storage exceeds its bound");
    const raw: unknown = JSON.parse(text);
    if (!Array.isArray(raw) || raw.length > 10_000) throw new Error("Invalid receipt storage");
    const ids = raw.filter((id): id is string => typeof id === "string" && uuid.test(id));
    return {
      ids: [...new Set(ids)],
      problem:
        ids.length !== raw.length || new Set(ids).size !== ids.length
          ? "Browser receipt storage is malformed. Existing correlations are preserved; new console starts are blocked. Native inventory cannot prove that an unrecorded request was never submitted."
          : null,
    };
  } catch {
    return {
      ids: [],
      problem:
        "Browser receipt storage is unavailable or malformed. New console starts are blocked; do not overwrite it or resend SQL to recover an outcome.",
    };
  }
}

export function consoleReceiptStorageProblem(): string | null {
  return storedCorrelations().problem;
}

/** Only opaque correlation IDs survive reload. Never SQL, parameters or results. */
export function restoredConsoleReceipts(): Record<string, SnowflakeConsoleReceipt> {
  return Object.fromEntries(
    storedCorrelations().ids.map((clientRequestId) => [
      clientRequestId,
      { clientRequestId, lookupPending: true },
    ]),
  );
}

export function rememberConsoleReceipts(receipts: Record<string, SnowflakeConsoleReceipt>): void {
  // Capacity fails closed. Do not silently evict receipt correlation IDs.
  const problem = consoleReceiptStorageProblem();
  if (problem) throw new Error(problem);
  const ids = Object.values(receipts)
    .filter((receipt) => !receipt.inventoried)
    .map((receipt) => receipt.clientRequestId);
  if (ids.length > 10_000)
    throw new Error("Console receipt capacity reached; existing receipts were preserved.");
  localStorage.setItem(RECEIPT_KEY, JSON.stringify(ids));
}

export function consoleDecimal(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^(0|[1-9][0-9]{0,18})$/.test(value) &&
    BigInt(value) <= 9223372036854775807n
  );
}

export function validateConsoleStorage(storage: SnowflakeConsoleStorage): void {
  if (
    !storage ||
    !uuid.test(storage.inventoryEpoch) ||
    ![storage.positiveCount, storage.refusalCount].every(consoleDecimal) ||
    ![
      storage.chargedBytes,
      storage.reservedBytes,
      storage.admissionBudgetBytes,
      storage.freeBytes,
    ].every((value) => value === null || consoleDecimal(value)) ||
    typeof storage.retentionEnabled !== "boolean"
  )
    throw new Error("Native receipt storage accounting is unavailable.");
}

export function validateConsoleSummary(summary: SnowflakeConsoleReceiptSummary): void {
  const text = (value: unknown) =>
    typeof value === "string" && value.length > 0 && value.length <= 128;
  const states = ["prepared", "running", "terminal"];
  const reasons = [
    "completed",
    "resultLimit",
    "cancelled",
    "failed",
    "outcomeUnknown",
    "interrupted",
    "recordingFailed",
  ];
  const refusalCodes = [
    "invalidInput",
    "connectionFailed",
    "connectionClosed",
    "database",
    "permissionDenied",
    "unsupported",
    "policyBlocked",
    "approvalRequired",
    "approvalExpired",
    "passphraseRequired",
    "invalidPassphrase",
    "conflict",
    "resultLimitReached",
    "notFound",
    "cancelled",
    "outcomeUnknown",
    "credentialStore",
    "internal",
    "sshHostKeyUnknown",
    "sshHostKeyChanged",
  ];
  if (
    !summary ||
    !uuid.test(summary.inventoryEpoch) ||
    !uuid.test(summary.clientRequestId) ||
    !text(summary.sessionId) ||
    !/^[a-f0-9]{64}$/.test(summary.planDigest) ||
    ![
      summary.sequence,
      summary.recordedAt,
      summary.originalRevision,
      summary.browsingRevision,
    ].every(consoleDecimal) ||
    summary.sequence === "0" ||
    (summary.kind !== "positive" && summary.kind !== "refusal") ||
    typeof summary.archived !== "boolean" ||
    typeof summary.archiveEligible !== "boolean" ||
    typeof summary.unknown !== "boolean" ||
    (summary.archiveBlockReason !== null &&
      (typeof summary.archiveBlockReason !== "string" ||
        summary.archiveBlockReason.length > 512)) ||
    (summary.kind === "positive" &&
      (!summary.runId ||
        !uuid.test(summary.runId) ||
        summary.refusalCode !== null ||
        !text(summary.connectionId) ||
        !states.includes(summary.state ?? "") ||
        (summary.state === "terminal"
          ? !reasons.includes(summary.stopReason ?? "")
          : summary.stopReason !== null) ||
        (summary.archiveEligible && summary.state !== "terminal"))) ||
    (summary.kind === "refusal" &&
      (summary.runId !== null ||
        !refusalCodes.includes(summary.refusalCode ?? "") ||
        summary.state !== null ||
        summary.connectionId !== null ||
        summary.stopReason !== null ||
        summary.unknown ||
        summary.originalRevision !== "0"))
  )
    throw new Error("Malformed native receipt inventory; previous correlations were preserved.");
}

export function validateConsoleHandoff(
  receipt: SnowflakeConsoleReceipt,
  summary: SnowflakeConsoleReceiptSummary,
): void {
  validateConsoleSummary(summary);
  if (
    receipt.clientRequestId !== summary.clientRequestId ||
    receipt.decisionConflict ||
    consoleReceiptHeld(receipt) ||
    !summary.archiveEligible ||
    receipt.sessionId !== summary.sessionId ||
    receipt.planDigest !== summary.planDigest ||
    (receipt.notAdmitted
      ? summary.kind !== "refusal" || receipt.notAdmitted !== summary.refusalCode
      : summary.kind !== "positive" ||
        receipt.runId !== summary.runId ||
        receipt.connectionId !== summary.connectionId ||
        (receipt.view
          ? !Number.isSafeInteger(receipt.view.run.metadata.createdAt) ||
            String(receipt.view.run.metadata.createdAt) !== summary.recordedAt ||
            !Number.isSafeInteger(receipt.view.run.revision) ||
            String(receipt.view.run.revision) !== summary.originalRevision
          : !receipt.settledProof ||
            receipt.settledProof.recordedAt !== summary.recordedAt ||
            receipt.settledProof.revision !== summary.originalRevision))
  )
    throw new Error(
      "Native inventory has not confirmed this exact settled receipt; its browser correlation remains preserved.",
    );
}

/** Opaque correlations are never lost. Settled detail payloads are bounded even
 * if native handoff or browser persistence fails. Pending recording is retained. */
export function compactConsoleReceipts(
  receipts: Record<string, SnowflakeConsoleReceipt>,
): Record<string, SnowflakeConsoleReceipt> {
  const next = { ...receipts };
  const cache = Object.values(next)
    .filter(
      (receipt) =>
        ((receipt.view?.run.state === "terminal" && !receipt.view.pendingRecording) ||
          (receipt.notAdmitted && !receipt.lookupPending)) &&
        !receipt.decisionConflict,
    )
    .sort((a, b) => (b.lastAccessed ?? 0) - (a.lastAccessed ?? 0));
  let bytes = 0;
  cache.forEach((receipt, index) => {
    const charge = JSON.stringify(receipt).length * 2 + 256;
    if (index < 16 && bytes + charge <= 16 * 1024 * 1024) bytes += charge;
    else if (
      receipt.inventoried &&
      receipt.nativeSettled &&
      !receipt.startPending &&
      !consoleReceiptHeld(receipt)
    )
      delete next[receipt.clientRequestId];
    else {
      const { view: _view, ...identity } = receipt;
      const proof =
        _view &&
        Number.isSafeInteger(_view.run.revision) &&
        Number.isSafeInteger(_view.run.metadata.createdAt) &&
        _view.run.revision >= 0 &&
        _view.run.metadata.createdAt >= 0
          ? {
              revision: String(_view.run.revision),
              recordedAt: String(_view.run.metadata.createdAt),
            }
          : undefined;
      next[receipt.clientRequestId] = {
        ...identity,
        settledProof: proof,
        lookupPending: proof ? receipt.lookupPending : true,
        error: receipt.error,
      };
    }
  });
  for (const receipt of Object.values(next))
    if (
      !receipt.view &&
      !receipt.notAdmitted &&
      !receipt.startPending &&
      receipt.inventoried &&
      receipt.nativeSettled &&
      !consoleReceiptHeld(receipt)
    )
      delete next[receipt.clientRequestId];
  return next;
}

export function consoleReceiptHeld(receipt: SnowflakeConsoleReceipt): boolean {
  if (receipt.decisionConflict) return true;
  if (receipt.notAdmitted) return receipt.lookupPending || !!receipt.runId || !!receipt.view;
  return (
    (receipt.startPending === true && !receipt.view && !receipt.settledProof) ||
    receipt.lookupPending ||
    (!receipt.view && !receipt.settledProof) ||
    (receipt.view !== undefined &&
      (receipt.view.pendingRecording || receipt.view.run.state !== "terminal"))
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
    !Number.isInteger(plan.timeoutSeconds) ||
    plan.timeoutSeconds < 1 ||
    plan.timeoutSeconds > 86_400 ||
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
    receipt.decisionConflict === true ||
    receipt.notAdmitted !== undefined ||
    metadata.clientRequestId !== receipt.clientRequestId ||
    (receipt.sessionId !== undefined && metadata.sessionId !== receipt.sessionId) ||
    (receipt.connectionId !== undefined && metadata.connectionId !== receipt.connectionId) ||
    (receipt.planDigest !== undefined && metadata.planDigest !== receipt.planDigest) ||
    (receipt.stepCount !== undefined && metadata.stepCount !== receipt.stepCount) ||
    (receipt.runId !== undefined && metadata.runId !== receipt.runId) ||
    (receipt.view !== undefined && view.run.revision < receipt.view.run.revision) ||
    (receipt.settledProof !== undefined &&
      (!Number.isSafeInteger(view.run.revision) ||
        !Number.isSafeInteger(metadata.createdAt) ||
        String(metadata.createdAt) !== receipt.settledProof.recordedAt ||
        BigInt(view.run.revision) < BigInt(receipt.settledProof.revision) ||
        view.run.state !== "terminal")) ||
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

/** Only the same request identity can establish an opposite native decision. */
export function consoleDecisionConflicts(
  receipt: SnowflakeConsoleReceipt,
  decision: SnowflakeConsoleRequestLookup,
): boolean {
  const identity = decision.kind === "started" ? decision.view.run.metadata : decision;
  return (
    identity.clientRequestId === receipt.clientRequestId &&
    (receipt.sessionId === undefined || receipt.sessionId === identity.sessionId) &&
    (receipt.planDigest === undefined || receipt.planDigest === identity.planDigest) &&
    (decision.kind === "started"
      ? receipt.notAdmitted !== undefined
      : receipt.runId !== undefined || receipt.view !== undefined)
  );
}

/** A native durable refusal is proof only for its exact retained identity. */
export function adoptConsoleNotAdmitted(
  receipt: SnowflakeConsoleReceipt,
  decision: Extract<SnowflakeConsoleRequestLookup, { kind: "notAdmitted" }>,
): SnowflakeConsoleReceipt {
  if (
    receipt.decisionConflict === true ||
    decision.clientRequestId !== receipt.clientRequestId ||
    !uuid.test(decision.clientRequestId) ||
    !decision.sessionId ||
    !/^[a-f0-9]{64}$/.test(decision.planDigest) ||
    (receipt.sessionId !== undefined && decision.sessionId !== receipt.sessionId) ||
    (receipt.planDigest !== undefined && decision.planDigest !== receipt.planDigest) ||
    (receipt.notAdmitted !== undefined && receipt.notAdmitted !== decision.code) ||
    receipt.runId !== undefined ||
    receipt.view !== undefined
  )
    throw new Error(
      "Pre-admission receipt conflicts with retained evidence; the original evidence was preserved.",
    );
  return {
    ...receipt,
    sessionId: decision.sessionId,
    planDigest: decision.planDigest,
    notAdmitted: decision.code,
    lastAccessed: consoleReceiptAccess(),
    startPending: false,
    lookupPending: false,
    error: undefined,
  };
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
