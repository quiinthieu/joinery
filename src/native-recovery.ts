import { getCurrentWindow } from "@tauri-apps/api/window";
import { client, messageOf } from "./ipc/client";
import { decodeArrow, readValue } from "./data/columnar";
import { useStudio } from "./state/studio";
import type {
  Bootstrap,
  ConnectionProfile,
  PreparedPlan,
  QueryEvent,
  ResourceUsage,
  SessionInfo,
} from "./ipc/generated";

type Stage = "setup" | "verify";
type Preferences = {
  theme: "dark";
  density: "compact";
  sidebarVisible: false;
  inspectorVisible: true;
};
interface RecoveryEvidence {
  table: string;
  profileId: string;
  tabId: string;
  expectedDraft: string;
  expectedPreferences: Preferences;
  baselineDraft: string;
  baselinePersisted: boolean;
  cancelKeptSession: boolean;
  cancelKeptTransaction: boolean;
  draftMutationEpochMs: number;
  reportEpochMs: number;
  transactionSessionId: string;
}
interface Checkpoint {
  options: { recoveryStage: Stage | null; fixturePort: number | null };
  usage: ResourceUsage;
  invocations: { connect: number; prepareExecution: number; execute: number };
  recoverySetup?: { passed: boolean; recoveryStage: Stage; recovery: RecoveryEvidence };
}
function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(check: () => boolean | Promise<boolean>, message: string) {
  const deadline = performance.now() + 10000;
  while (!(await check())) {
    assert(performance.now() < deadline, message);
    await delay(20);
  }
}
function dialog() {
  return document.querySelector<HTMLElement>('[role="dialog"][aria-label="Close Joinery?"]');
}
function dialogButton(label: string) {
  const button = [...(dialog()?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find(
    (candidate) => candidate.textContent?.trim() === label,
  );
  assert(button && !button.disabled, `Native close confirmation is missing ${label}`);
  return button;
}
async function connect(profileId: string) {
  return client.call("connect", {
    id: profileId,
    secrets: { password: "joinery-local-only", sshPassword: null, keyPassphrase: null },
  });
}
async function query(session: SessionInfo, sql: string, tabId?: string) {
  const plan = await client.call<PreparedPlan>("prepare_execution", { sessionId: session.id, sql });
  assert(!plan.requiresApproval, "Disposable native recovery SQL unexpectedly requires approval");
  const events: QueryEvent[] = [];
  const revision = tabId
    ? useStudio.getState().tabs.find((tab) => tab.id === tabId)?.revision
    : undefined;
  let done!: () => void;
  const finished = new Promise<void>((resolve) => (done = resolve));
  const timer = setTimeout(done, 15000);
  try {
    await client.execute(plan.id, null, (event) => {
      events.push(event);
      if (tabId && revision !== undefined)
        useStudio.getState().applyEvents(tabId, [event], revision);
      if (event.type === "finished") done();
    });
    await finished;
    const last = events.find((event) => event.type === "finished");
    const failure = events.find((event) => event.type === "error");
    assert(
      last?.type === "finished" && last.status === "completed",
      `Native recovery query failed: ${failure?.type === "error" ? failure.error.message : "no successful completion"}`,
    );
    return events;
  } finally {
    clearTimeout(timer);
  }
}
async function scalar(session: SessionInfo, sql: string) {
  const events = await query(session, sql);
  const result = events.find((event) => event.type === "result");
  assert(result?.type === "result", "Native PostgreSQL query produced no result");
  const page = decodeArrow(await client.range(result.result.id, 0, 1));
  const value = readValue(page, result.result.columns[0].id, 0, false)?.value;
  assert(value !== undefined, "Native PostgreSQL result could not be decoded");
  await client.call("release_result", { resultId: result.result.id });
  return value;
}
function ownTable(table: string) {
  assert(/^native_recovery_[a-f0-9]{32}$/.test(table), "Invalid owned native recovery table");
  return `public.${table}`;
}
function assertFixtureProfile(profile: ConnectionProfile, port: number) {
  assert(
    profile.engine === "postgres" &&
      profile.host === "127.0.0.1" &&
      profile.port === port &&
      profile.database === "joinery" &&
      profile.username === "joinery" &&
      profile.ssh === null &&
      profile.tlsMode === "disable" &&
      profile.caCertPath === null &&
      !profile.rememberPassword,
    "Recovery profile is not the disposable PostgreSQL fixture",
  );
}

/** Used only by the separately built native-smoke entry point. */
export async function runNativeRecovery(): Promise<boolean> {
  let checkpoint: Checkpoint;
  try {
    checkpoint = await client.call<Checkpoint>("native_smoke_checkpoint", {
      phase: "options",
      cycle: null,
    });
  } catch (error) {
    await client.call("native_smoke_report", {
      report: { passed: false, error: messageOf(error), checks: [] },
    });
    return true;
  }
  const stage = checkpoint.options.recoveryStage;
  if (!stage) return false;
  const started = performance.now();
  const report: Record<string, unknown> = { passed: false, recoveryStage: stage, checks: [] };
  let profileId: string | undefined;
  let table: string | undefined;
  let ownedSession: SessionInfo | undefined;
  let tableCreated = false;
  let fixtureValidated = false;
  try {
    await until(
      () =>
        Boolean(
          useStudio.getState().bootstrap && document.querySelector('[aria-label="New query tab"]'),
        ),
      "Native App did not initialize",
    );
    if (stage === "setup") {
      const profile: ConnectionProfile = {
        id: crypto.randomUUID(),
        name: "Disposable native recovery",
        groupId: "local",
        engine: "postgres",
        host: "127.0.0.1",
        port: checkpoint.options.fixturePort ?? 55432,
        database: "joinery",
        username: "joinery",
        environment: "local",
        safeMode: "silent",
        tlsMode: "disable",
        caCertPath: null,
        ssh: null,
        rememberPassword: false,
        storeQueryText: false,
        storeDrafts: true,
        snowflake: null,
      };
      const saved = await client.call("save_profile", { profile });
      assertFixtureProfile(saved, checkpoint.options.fixturePort ?? 55432);
      fixtureValidated = true;
      profileId = saved.id;
      useStudio.getState().updateProfile(saved);
      const tabId = useStudio.getState().openQuery(saved.id);
      ownedSession = await connect(saved.id);
      useStudio.getState().bindQuerySession(tabId, ownedSession, "Native recovery transaction");
      table = `native_recovery_${crypto.randomUUID().replaceAll("-", "")}`;
      await query(ownedSession, `CREATE TABLE ${ownTable(table)} (id integer PRIMARY KEY)`, tabId);
      tableCreated = true;
      await useStudio.getState().setTransactionMode(ownedSession.id, "manual");
      await query(ownedSession, `INSERT INTO ${ownTable(table)} VALUES (1)`, tabId);
      assert(
        useStudio.getState().sessionStates[ownedSession.id]?.transactionState === "inTransaction",
        "Native manual transaction did not open",
      );

      await getCurrentWindow().close();
      await until(() => Boolean(dialog()), "Native close did not request confirmation");
      dialogButton("Keep working").click();
      await until(() => !dialog(), "Native close cancellation did not dismiss confirmation");
      const afterCancel = await client.call("session_contexts", { sessionId: ownedSession.id });
      const usage = await client.call("resource_usage");
      assert(usage.sessionCount === 1, "Cancelling native close lost or duplicated the session");
      assert(
        afterCancel.state.transactionState === "inTransaction",
        "Cancelling native close ended the transaction",
      );
      assert(
        (await scalar(ownedSession, `SELECT count(*) FROM ${ownTable(table)}`)) === "1",
        "Cancelled native close lost the pending write",
      );

      const baselineDraft = `-- native recovery baseline ${table}\nSELECT 0;`;
      useStudio.getState().updateTab(tabId, { sql: baselineDraft });
      useStudio.getState().setTheme("light");
      useStudio.setState({
        density: "comfortable",
        sidebarVisible: true,
        sidebarCollapsed: false,
        inspectorVisible: false,
      });
      await until(async () => {
        const bootstrap = await client.call<Bootstrap>("bootstrap");
        return (
          bootstrap.workspace?.drafts.some(
            (draft) => draft.id === tabId && draft.sql === baselineDraft,
          ) === true &&
          bootstrap.workspace.theme === "light" &&
          bootstrap.workspace.density === "comfortable"
        );
      }, "Native baseline draft was not persisted by the normal workspace watcher");

      await getCurrentWindow().close();
      await until(() => Boolean(dialog()), "Native second close did not request confirmation");
      const confirm = dialogButton("Continue");
      const baseline = (await client.call<Bootstrap>("bootstrap")).workspace;
      assert(
        baseline?.drafts.some((draft) => draft.id === tabId && draft.sql === baselineDraft),
        "Persisted baseline changed before the pending mutation",
      );
      const expectedDraft = `-- native recovery latest héllo;世界 ${table}\nINSERT INTO ${ownTable(table)} VALUES (999);\nSELECT 'draft must not replay';`;
      const expectedPreferences: Preferences = {
        theme: "dark",
        density: "compact",
        sidebarVisible: false,
        inspectorVisible: true,
      };
      const draftMutationEpochMs = Date.now();
      useStudio.getState().updateTab(tabId, { sql: expectedDraft });
      useStudio.getState().setTheme(expectedPreferences.theme);
      useStudio.setState({
        density: expectedPreferences.density,
        sidebarVisible: false,
        sidebarCollapsed: true,
        inspectorVisible: true,
      });
      const recovery: RecoveryEvidence = {
        table,
        profileId: saved.id,
        tabId,
        expectedDraft,
        expectedPreferences,
        baselineDraft,
        baselinePersisted: true,
        cancelKeptSession: true,
        cancelKeptTransaction: true,
        draftMutationEpochMs,
        reportEpochMs: Date.now(),
        transactionSessionId: ownedSession.id,
      };
      report.recovery = recovery;
      report.passed = true;
      report.checks = [
        "real-close-confirmation-cancel",
        "live-manual-postgres-write",
        "persisted-baseline-before-pending-mutation",
        "real-close-confirmation-continue",
      ];
      await client.call("native_smoke_report", { report });
      assert(
        Date.now() - draftMutationEpochMs < 400,
        "Draft debounce elapsed before native close confirmation",
      );
      confirm.click();
      // A destroyed WebView never reaches this fallback; failures still report
      // and force a bounded test exit rather than hanging a hidden process.
      await delay(4000);
      throw new Error("The native App window did not close after confirmation");
    }

    const setup = checkpoint.recoverySetup;
    assert(
      setup?.passed && setup.recoveryStage === "setup",
      "Missing successful native setup evidence",
    );
    const evidence = setup.recovery;
    ownTable(evidence.table);
    table = evidence.table;
    profileId = evidence.profileId;
    const initial = await client.call<Bootstrap>("bootstrap");
    const state = useStudio.getState();
    const persistedProfile = initial.profiles.find((profile) => profile.id === profileId);
    assert(
      persistedProfile?.storeDrafts && !persistedProfile.rememberPassword,
      "The opted-in native draft profile was not retained",
    );
    assertFixtureProfile(persistedProfile, checkpoint.options.fixturePort ?? 55432);
    fixtureValidated = true;
    tableCreated = true;
    const draft = state.tabs.find((tab) => tab.id === evidence.tabId);
    assert(
      draft?.connectionId === profileId && draft.sql === evidence.expectedDraft,
      "Latest pending native draft was not recovered exactly",
    );
    assert(
      initial.workspace?.drafts.some(
        (item) => item.id === evidence.tabId && item.sql === evidence.expectedDraft,
      ),
      "Latest draft was not persisted at native close",
    );
    for (const key of ["theme", "density", "sidebarVisible", "inspectorVisible"] as const) {
      assert(
        state[key] === evidence.expectedPreferences[key] &&
          initial.workspace?.[key] === evidence.expectedPreferences[key],
        `Native preference ${key} was not restored`,
      );
    }
    const beforeConnect = await client.call("resource_usage");
    const beforeObserver = await client.call<Checkpoint>("native_smoke_checkpoint", {
      phase: "options",
      cycle: null,
    });
    assert(
      beforeObserver.invocations.connect === 0 &&
        beforeObserver.invocations.prepareExecution === 0 &&
        beforeObserver.invocations.execute === 0,
      "Native restart invoked a connection or SQL execution before the explicit observer",
    );
    assert(
      checkpoint.usage.sessionCount === 0 &&
        beforeConnect.sessionCount === 0 &&
        Object.keys(state.sessions).length === 0 &&
        Object.keys(state.sessionPool).length === 0 &&
        state.tabs.every((tab) => tab.querySession === null),
      "Native restart restored a live session",
    );
    assert(
      checkpoint.usage.resultCount === 0 &&
        beforeConnect.resultCount === 0 &&
        beforeConnect.retainedBytes === 0 &&
        state.tabs.every((tab) => tab.results.length === 0),
      "Native restart restored result rows",
    );
    assert(
      Object.keys(state.sessionStates).length === 0 &&
        beforeConnect.activeRuns === 0 &&
        !document.querySelector(".tx-badge"),
      "Native restart restored a transaction or running SQL",
    );

    // This is the first explicit connection in the restarted process. Never
    // execute the recovered draft: its INSERT is deliberately left as text.
    ownedSession = await connect(profileId);
    assert(
      (await scalar(ownedSession, `SELECT count(*) FROM ${ownTable(table)}`)) === "0",
      "Native close did not roll back, or restored draft SQL was replayed",
    );
    await query(ownedSession, `DROP TABLE ${ownTable(table)}`);
    tableCreated = false;
    await client.call("disconnect", { sessionId: ownedSession.id });
    ownedSession = undefined;
    const after = await client.call("resource_usage");
    assert(
      after.sessionCount === 0 && after.resultCount === 0,
      "Native recovery verification leaked its observer",
    );
    report.recovery = {
      recoveredLatestDraft: true,
      preferencesRestored: true,
      noRestoredSessions: true,
      noRestoredResults: true,
      noRestoredTransactions: true,
      noSqlReplay: true,
      rolledBack: true,
      tableDropped: true,
      invocationsBeforeObserver: beforeObserver.invocations,
      table,
      profileId,
    };
    report.checks = [
      "same-root-persisted-draft-and-preferences",
      "empty-native-session-and-result-state-before-connect",
      "actual-postgres-rollback-and-no-draft-replay",
      "owned-table-cleanup",
    ];
    report.passed = true;
  } catch (error) {
    report.passed = false;
    report.error = messageOf(error);
    try {
      if (ownedSession) await client.call("disconnect", { sessionId: ownedSession.id });
      if (fixtureValidated && tableCreated && profileId && table) {
        const cleanup = await connect(profileId);
        try {
          await query(cleanup, `DROP TABLE IF EXISTS ${ownTable(table)}`);
        } finally {
          await client.call("disconnect", { sessionId: cleanup.id });
        }
      }
    } catch (cleanupError) {
      report.cleanupError = messageOf(cleanupError);
    }
  }
  report.frontendElapsedMs = performance.now() - started;
  await client.call("native_smoke_report", { report });
  return true;
}
