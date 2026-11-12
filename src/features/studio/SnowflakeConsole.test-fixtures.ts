import type {
  SessionInfo,
  SnowflakeConsolePlan,
  SnowflakeConsoleRunView,
  ConsoleObservations,
} from "../../ipc/generated";
export const clientId = "11111111-1111-4111-8111-111111111111";
export const runId = "22222222-2222-4222-8222-222222222222";
export const session: SessionInfo = {
  id: "console-session",
  connectionId: "profile",
  engine: "snowflake",
  serverVersion: "fixture",
  latencyMs: 0,
  encrypted: true,
  consoleMode: true,
  capabilities: {
    sql: true,
    transactions: false,
    cancellation: true,
    tableBrowsing: true,
    tableEditing: false,
    tableFiltering: false,
    processes: false,
    relationBrowsing: true,
    objectExplorer: true,
  },
};
export const observations: ConsoleObservations = {
  delivered: false,
  deliveryFailed: false,
  cancellationObserved: false,
  retired: false,
  truncated: false,
  recordingFailed: false,
  firstError: null,
};
export const plan: SnowflakeConsolePlan = {
  digest: "a".repeat(64),
  mode: "parsedStatements",
  validations: ["localSyntax", "localSyntax"],
  review: {
    id: "plan",
    expiresAt: 60000,
    sessionId: session.id,
    statements: [
      {
        sql: "SELECT :value",
        kind: "read",
        missingWhere: false,
        span: { startUtf16: 0, endUtf16: 13, line: 1, column: 1 },
      },
      {
        sql: "COMMIT",
        kind: "transaction",
        missingWhere: false,
        span: { startUtf16: 15, endUtf16: 21, line: 1, column: 16 },
      },
    ],
    warnings: [],
    requiresApproval: true,
    requiresPassphrase: false,
    confirmationText: null,
    boundParameters: [{ key: "value", kind: "text", value: "immutable" }],
  },
};
export function receiptView(
  overrides: Partial<SnowflakeConsoleRunView> = {},
): SnowflakeConsoleRunView {
  return {
    run: {
      metadata: {
        runId,
        clientRequestId: clientId,
        sessionId: session.id,
        connectionId: "profile",
        planDigest: plan.digest,
        stepCount: 2,
        createdAt: 1,
      },
      revision: 4,
      state: "terminal",
      observations: { ...observations, delivered: true, truncated: true },
      steps: [
        {
          ordinal: 0,
          intendedAt: 2,
          outcome: {
            kind: "acknowledged",
            evidence: {
              statement: { sdkRequestId: "sdk-original", queryId: "original-query" },
              context: { database: "DB", schema: "SCHEMA", role: null, warehouse: null },
            },
          },
          observations: { ...observations, truncated: true },
        },
      ],
      terminal: {
        reason: "resultLimit",
        finishedAt: 3,
        observations: { ...observations, delivered: true, truncated: true },
        accounting: {
          planned: 2,
          intended: 1,
          acknowledged: 1,
          rejected: 0,
          unknown: 0,
          notDispatched: 0,
          notIntended: 1,
          notAttempted: 1,
        },
      },
    },
    results: [],
    pendingRecording: false,
    ...overrides,
  };
}
