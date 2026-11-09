import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { client } from "../../ipc/client";
import type {
  ChangePreview,
  ConnectionProfile,
  CsvImportPreview,
  SessionInfo,
} from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import { ImportCsvDialog } from "./ImportCsvDialog";

vi.mock("../../ipc/client", async (original) => ({
  ...(await original<typeof import("../../ipc/client")>()),
  desktopAvailable: true,
}));

const profile: ConnectionProfile = {
  id: "import-profile",
  name: "Import fixture",
  groupId: "local",
  engine: "postgres",
  host: "127.0.0.1",
  port: 55432,
  database: "joinery",
  username: "joinery",
  environment: "local",
  safeMode: "confirm",
  tlsMode: "disable",
  caCertPath: null,
  ssh: null,
  rememberPassword: false,
  storeQueryText: false,
  storeDrafts: false,
  snowflake: null,
};
const session: SessionInfo = {
  id: "import-session",
  connectionId: profile.id,
  engine: "postgres",
  serverVersion: "fixture",
  latencyMs: 0,
  encrypted: false,
  capabilities: {
    sql: true,
    transactions: true,
    cancellation: true,
    tableBrowsing: true,
    tableEditing: true,
    tableFiltering: true,
    processes: true,
  },
};
const preview: CsvImportPreview = {
  fileName: "fixture.csv",
  headers: ["id"],
  mappings: [{ csvIndex: 0, targetColumn: "id" }],
  unmappedDefaults: [],
  unmappedNulls: [],
  sampleRows: [["1"]],
  totalRows: 501,
  batchSize: 500,
  batchCount: 2,
  issues: [],
  totalIssues: 0,
  canPrepare: true,
};
const changes: ChangePreview = {
  plan: {
    id: "import-plan",
    sessionId: session.id,
    requiresApproval: true,
    requiresPassphrase: false,
    confirmationText: null,
    expiresAt: Date.now() + 60000,
    warnings: [],
    statements: [{ sql: "INSERT INTO fixture VALUES ($1)", kind: "write", missingWhere: false }],
  },
  rowCount: 500,
  sql: ["INSERT INTO fixture VALUES ($1)"],
  bindings: [["1"]],
};
const initialState = useStudio.getInitialState();
beforeEach(() => {
  vi.restoreAllMocks();
  useStudio.setState(initialState, true);
});
afterEach(cleanup);

// These are UI event-contract regressions. Real COMMIT/rollback behavior is
// qualified independently through the protocol proxy and browser fixture.
test.each(["primary", "pooled", "writer-only"] as const)(
  "uncertain CSV batch reports truthfully and retires only explicit %s session closure",
  async (ownership) => {
    const healthy = { ...session, id: "healthy-session" };
    useStudio.setState({
      sessions: { [profile.id]: ownership === "pooled" ? healthy : session },
      sessionPool:
        ownership === "pooled"
          ? { [profile.id]: [{ info: session, label: "Isolated import", primary: false }] }
          : {},
      sessionStates: {
        [session.id]: {
          sessionId: session.id,
          transactionMode: "autoCommit",
          transactionState: "idle",
          contextKind: "schema",
          activeContext: "public",
          database: "joinery",
          role: "joinery",
          searchPath: ["public"],
        },
      },
    });
    const tabId = useStudio.getState().openQuery(profile.id);
    useStudio.getState().bindQuerySession(tabId, session, "Import fixture");
    const call = vi.spyOn(client, "call").mockImplementation(async (command) => {
      if (command === "pick_import_source")
        return { token: "fixture-token", fileName: "fixture.csv", displayLabel: "fixture.csv" };
      if (command === "preview_csv_import") return preview;
      if (command === "prepare_csv_import_batch") return changes;
      if (command === "authorize_plan") return "one-use-approval";
      throw new Error(`Unexpected command: ${command}`);
    });
    const execute = vi
      .spyOn(client, "execute")
      .mockImplementation(async (_id, _approval, onEvent) => {
        onEvent({
          type: "error",
          statementIndex: 0,
          error: {
            code: "outcomeUnknown",
            message: "COMMIT acknowledgement lost. Verify the table before retrying.",
          },
        });
        if (ownership !== "writer-only")
          onEvent({
            type: "sessionClosed",
            sessionId: session.id,
            reason: "The editor connection was closed after an interrupted write.",
          });
        onEvent({ type: "finished", runId: "import-run", status: "outcomeUnknown", elapsedMs: 10 });
        return "import-run";
      });
    const onImported = vi.fn();
    render(
      <ImportCsvDialog
        table={{ schema: "public", name: "fixture" }}
        schema={null}
        profile={profile}
        session={session}
        onImported={onImported}
        onClose={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Choose CSV file…" }));
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Review batch 1 of 2…" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole("button", { name: "Review batch 1 of 2…" }));
    await screen.findByRole("dialog", { name: "Review pending changes" });
    fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain("commit outcome unknown"),
    );
    expect.soft(screen.getByRole("alert").textContent).toContain("This batch may have committed");
    expect
      .soft(screen.getByRole("alert").textContent)
      .not.toContain("No rows from this batch were committed");
    expect(
      (screen.getByRole("button", { name: "Review batch 1 of 2…" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(execute).toHaveBeenCalledTimes(1);
    expect(
      call.mock.calls.filter(([command]) => command === "prepare_csv_import_batch"),
    ).toHaveLength(1);
    expect(onImported).not.toHaveBeenCalled();
    const state = useStudio.getState();
    if (ownership === "writer-only") {
      expect(state.sessions[profile.id]?.id).toBe(session.id);
      expect(state.sessionStates[session.id]).toBeDefined();
      expect(state.tabs.find((tab) => tab.id === tabId)?.querySessionClosed).toBe(false);
    } else {
      expect
        .soft(Object.values(state.sessions).some((entry) => entry.id === session.id))
        .toBe(false);
      expect.soft(state.sessionPool[profile.id]).toBeUndefined();
      expect.soft(state.sessionStates[session.id]).toBeUndefined();
      expect.soft(state.tabs.find((tab) => tab.id === tabId)?.querySessionClosed).toBe(true);
      if (ownership === "pooled") expect(state.sessions[profile.id]?.id).toBe(healthy.id);
    }
  },
);
