import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  previewToken: "preview-generation-1",
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
      if (command === "release_csv_import_preview") return;
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function renderImport(onClose = vi.fn()) {
  return render(
    <ImportCsvDialog
      table={{ schema: "public", name: "fixture" }}
      schema={null}
      profile={profile}
      session={session}
      onImported={vi.fn()}
      onClose={onClose}
    />,
  );
}

function mockPreview(load: () => Promise<CsvImportPreview>) {
  return vi.spyOn(client, "call").mockImplementation(async (command) => {
    if (command === "pick_import_source")
      return { token: "fixture-token", fileName: "fixture.csv", displayLabel: "fixture.csv" };
    if (command === "preview_csv_import") return load();
    if (command === "prepare_csv_import_batch") return changes;
    if (command === "release_csv_import_preview") return;
    throw new Error(`Unexpected command: ${command}`);
  });
}

test("unmount releases the accepted CSV preview generation", async () => {
  const call = mockPreview(async () => preview);
  const view = renderImport();
  fireEvent.click(screen.getByRole("button", { name: "Choose CSV file…" }));
  await screen.findByRole("button", { name: "Review batch 1 of 2…" });
  await waitFor(() =>
    expect(
      (screen.getByRole("button", { name: "Review batch 1 of 2…" }) as HTMLButtonElement).disabled,
    ).toBe(false),
  );
  view.unmount();
  expect(call).toHaveBeenCalledWith("release_csv_import_preview", {
    sessionId: session.id,
    sourceToken: "fixture-token",
    previewToken: "preview-generation-1",
  });
});

test("a preview response arriving after unmount is released immediately", async () => {
  const pending = deferred<CsvImportPreview>();
  const call = mockPreview(() => pending.promise);
  const view = renderImport();
  fireEvent.click(screen.getByRole("button", { name: "Choose CSV file…" }));
  await waitFor(() => expect(call).toHaveBeenCalledWith("preview_csv_import", expect.anything()));
  view.unmount();
  await act(async () => pending.resolve(preview));
  expect(call).toHaveBeenCalledWith("release_csv_import_preview", {
    sessionId: session.id,
    sourceToken: "fixture-token",
    previewToken: "preview-generation-1",
  });
});

async function choosePreview() {
  fireEvent.click(screen.getByRole("button", { name: "Choose CSV file…" }));
  await waitFor(() =>
    expect(
      (screen.getByRole("button", { name: "Review batch 1 of 2…" }) as HTMLButtonElement).disabled,
    ).toBe(false),
  );
}

test("close releases the accepted preview only once even before unmount", async () => {
  const call = mockPreview(async () => preview);
  const onClose = vi.fn();
  const view = renderImport(onClose);
  await choosePreview();
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  expect(onClose).toHaveBeenCalledOnce();
  view.unmount();
  expect(
    call.mock.calls.filter(([command]) => command === "release_csv_import_preview"),
  ).toHaveLength(1);
});

test("replacement releases only the previous token and prepares the accepted generation", async () => {
  const replacement = deferred<CsvImportPreview>();
  let loads = 0;
  const call = mockPreview(() => (++loads === 1 ? Promise.resolve(preview) : replacement.promise));
  const view = renderImport();
  await choosePreview();
  fireEvent.change(screen.getByLabelText("NULL token"), { target: { value: "NULL" } });
  await waitFor(() => expect(loads).toBe(2));
  expect(
    call.mock.calls.filter(([command]) => command === "release_csv_import_preview"),
  ).toHaveLength(0);
  await act(async () => replacement.resolve({ ...preview, previewToken: "preview-generation-2" }));
  expect(call).toHaveBeenCalledWith("release_csv_import_preview", {
    sessionId: session.id,
    sourceToken: "fixture-token",
    previewToken: "preview-generation-1",
  });
  fireEvent.click(screen.getByRole("button", { name: "Review batch 1 of 2…" }));
  await waitFor(() =>
    expect(call).toHaveBeenCalledWith(
      "prepare_csv_import_batch",
      expect.objectContaining({ previewToken: "preview-generation-2" }),
    ),
  );
  view.unmount();
  expect(call).toHaveBeenCalledWith("release_csv_import_preview", {
    sessionId: session.id,
    sourceToken: "fixture-token",
    previewToken: "preview-generation-2",
  });
});

test("a stale response from a closed dialog cannot release the reopened dialog's preview", async () => {
  const stale = deferred<CsvImportPreview>();
  let loads = 0;
  const call = mockPreview(() =>
    ++loads === 1
      ? stale.promise
      : Promise.resolve({ ...preview, previewToken: "new-dialog-generation" }),
  );
  const first = renderImport();
  fireEvent.click(screen.getByRole("button", { name: "Choose CSV file…" }));
  await waitFor(() => expect(loads).toBe(1));
  first.unmount();
  const second = renderImport();
  await choosePreview();
  await act(async () => stale.resolve(preview));
  const releases = call.mock.calls.filter(([command]) => command === "release_csv_import_preview");
  expect(releases).toEqual([
    [
      "release_csv_import_preview",
      {
        sessionId: session.id,
        sourceToken: "fixture-token",
        previewToken: "preview-generation-1",
      },
    ],
  ]);
  fireEvent.click(screen.getByRole("button", { name: "Review batch 1 of 2…" }));
  await waitFor(() =>
    expect(call).toHaveBeenCalledWith(
      "prepare_csv_import_batch",
      expect.objectContaining({ previewToken: "new-dialog-generation" }),
    ),
  );
  second.unmount();
});

test("failed replacement releases the previous preview rather than retaining an invisible resource", async () => {
  const replacement = deferred<CsvImportPreview>();
  let loads = 0;
  const call = mockPreview(() => (++loads === 1 ? Promise.resolve(preview) : replacement.promise));
  const view = renderImport();
  await choosePreview();
  fireEvent.change(screen.getByLabelText("NULL token"), { target: { value: "NULL" } });
  await act(async () => replacement.reject(new Error("CSV validation failed")));
  await screen.findByText("CSV validation failed");
  expect(call).toHaveBeenCalledWith(
    "release_csv_import_preview",
    expect.objectContaining({ previewToken: "preview-generation-1" }),
  );
  view.unmount();
  expect(
    call.mock.calls.filter(([command]) => command === "release_csv_import_preview"),
  ).toHaveLength(1);
});

test("a file picker finishing after unmount never acquires a preview", async () => {
  const picked = deferred<{ token: string; fileName: string; displayLabel: string }>();
  const call = vi.spyOn(client, "call").mockImplementation(async (command) => {
    if (command === "pick_import_source") return picked.promise;
    throw new Error(`Unexpected command: ${command}`);
  });
  const view = renderImport();
  fireEvent.click(screen.getByRole("button", { name: "Choose CSV file…" }));
  view.unmount();
  await act(async () =>
    picked.resolve({
      token: "fixture-token",
      fileName: "fixture.csv",
      displayLabel: "fixture.csv",
    }),
  );
  expect(call).toHaveBeenCalledTimes(1);
});

test("completed batches share the original token and release it after the final commit", async () => {
  const call = vi.spyOn(client, "call").mockImplementation(async (command, args) => {
    if (command === "pick_import_source")
      return { token: "fixture-token", fileName: "fixture.csv", displayLabel: "fixture.csv" };
    if (command === "preview_csv_import") return preview;
    if (command === "prepare_csv_import_batch") {
      const batch = (args as { batchIndex: number }).batchIndex;
      return {
        ...changes,
        plan: { ...changes.plan, id: `import-plan-${batch}` },
        rowCount: batch === 0 ? 500 : 1,
      };
    }
    if (command === "authorize_plan") return "one-use-approval";
    if (command === "release_csv_import_preview") return;
    throw new Error(`Unexpected command: ${command}`);
  });
  const execute = vi
    .spyOn(client, "execute")
    .mockImplementation(async (_id, _approval, onEvent) => {
      onEvent({ type: "finished", runId: "import-run", status: "completed", elapsedMs: 1 });
      return "import-run";
    });
  const view = renderImport();
  await choosePreview();
  fireEvent.click(screen.getByRole("button", { name: "Review batch 1 of 2…" }));
  await screen.findByRole("dialog", { name: "Review pending changes" });
  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));
  await waitFor(() =>
    expect(
      call.mock.calls.filter(([command]) => command === "prepare_csv_import_batch"),
    ).toHaveLength(2),
  );
  expect(
    call.mock.calls.filter(([command]) => command === "release_csv_import_preview"),
  ).toHaveLength(0);
  await waitFor(() =>
    expect(
      (screen.getByRole("button", { name: "Apply changes" }) as HTMLButtonElement).disabled,
    ).toBe(false),
  );
  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));
  await screen.findByRole("button", { name: "Done" });
  expect(execute).toHaveBeenCalledTimes(2);
  for (const [, args] of call.mock.calls.filter(
    ([command]) => command === "prepare_csv_import_batch",
  )) {
    expect(args).toMatchObject({ previewToken: "preview-generation-1" });
  }
  expect(
    call.mock.calls.filter(([command]) => command === "release_csv_import_preview"),
  ).toHaveLength(1);
  view.unmount();
  expect(
    call.mock.calls.filter(([command]) => command === "release_csv_import_preview"),
  ).toHaveLength(1);
});

test("session retirement releases the preview without hiding the submitted batch's uncertain outcome", async () => {
  const call = mockPreview(async () => preview);
  call.mockImplementation(async (command) => {
    if (command === "pick_import_source")
      return { token: "fixture-token", fileName: "fixture.csv", displayLabel: "fixture.csv" };
    if (command === "preview_csv_import") return preview;
    if (command === "prepare_csv_import_batch") return changes;
    if (command === "authorize_plan") return "one-use-approval";
    if (command === "release_csv_import_preview") return;
    throw new Error(`Unexpected command: ${command}`);
  });
  let emit!: Parameters<typeof client.execute>[2];
  const execute = vi
    .spyOn(client, "execute")
    .mockImplementation(async (_id, _approval, onEvent) => {
      emit = onEvent;
      return "import-run";
    });
  const view = renderImport();
  await choosePreview();
  fireEvent.click(screen.getByRole("button", { name: "Review batch 1 of 2…" }));
  await screen.findByRole("dialog", { name: "Review pending changes" });
  fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));
  await waitFor(() => expect(execute).toHaveBeenCalledOnce());
  view.rerender(
    <ImportCsvDialog
      table={{ schema: "public", name: "fixture" }}
      schema={null}
      profile={profile}
      session={undefined}
      onImported={vi.fn()}
      onClose={vi.fn()}
    />,
  );
  await act(async () => {
    emit({
      type: "error",
      statementIndex: 0,
      error: { code: "outcomeUnknown", message: "Verify the table before retrying." },
    });
    emit({ type: "finished", runId: "import-run", status: "outcomeUnknown", elapsedMs: 1 });
  });
  expect((await screen.findByRole("alert")).textContent).toContain("This batch may have committed");
  expect(
    call.mock.calls.filter(([command]) => command === "release_csv_import_preview"),
  ).toHaveLength(1);
  expect(
    call.mock.calls.filter(([command]) => command === "prepare_csv_import_batch"),
  ).toHaveLength(1);
});

test("a preview without target ownership cannot prepare an import batch", async () => {
  const call = mockPreview(async () => ({ ...preview, previewToken: null }));
  renderImport();
  fireEvent.click(screen.getByRole("button", { name: "Choose CSV file…" }));
  const review = await screen.findByRole("button", { name: "Review batch 1 of 2…" });
  await waitFor(() => expect((review as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(review);
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Preview this CSV again before reviewing an import batch.",
  );
  expect(call.mock.calls.filter(([command]) => command === "prepare_csv_import_batch")).toEqual([]);
});
