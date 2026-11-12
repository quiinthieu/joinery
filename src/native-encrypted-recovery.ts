import { getCurrentWindow } from "@tauri-apps/api/window";
import { appErrorOf, client, messageOf } from "./ipc/client";
import type {
  Bootstrap,
  RecoveryBackup,
  RecoveryOperation,
  ResourceUsage,
  SnowflakeConsoleReceiptPage,
  SnowflakeConsolePending,
  SnowflakeConsoleStorage,
} from "./ipc/generated";
import { useStudio } from "./state/studio";

type Stage = "encryptedSetup" | "encryptedVerify";
type Counts = { connect: number; prepareExecution: number; execute: number };
interface SetupEvidence {
  groupId: string;
  groupName: string;
  backups: RecoveryBackup[];
  operationIds: string[];
  checks: Record<string, boolean>;
  invocations: Counts;
}
interface Checkpoint {
  options: { recoveryStage: string | null };
  invocations: Counts;
  usage: ResourceUsage;
  recoverySetup?: { passed: boolean; recoveryStage: Stage; encryptedRecovery: SetupEvidence };
}

export function sameRecoveryInventory(actual: RecoveryBackup[], expected: RecoveryBackup[]) {
  // Setup passes through serde_json::Value, whose object key order differs
  // from fresh typed IPC. Preserve exact array order and every backup field.
  return (
    actual.length === expected.length &&
    actual.every((backup, index) => {
      const prior = expected[index];
      return (
        backup.format === prior.format &&
        backup.metadata.id === prior.metadata.id &&
        backup.metadata.createdAt === prior.metadata.createdAt &&
        backup.metadata.sizeBytes === prior.metadata.sizeBytes &&
        backup.metadata.digest === prior.metadata.digest
      );
    })
  );
}

// These are disposable fixture keys, used only in the separate native-smoke entry.
const key = "native isolated encrypted recovery passphrase";
const wrongKey = "wrong native isolated recovery passphrase";
const marker = "joinery.metadataRecoveryOperation.v1";
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
async function until(check: () => boolean | Promise<boolean>, message: string, timeout = 10000) {
  const deadline = performance.now() + timeout;
  while (!(await check())) {
    assert(performance.now() < deadline, message);
    await delay(25);
  }
}
function recovery() {
  const node = document.querySelector<HTMLElement>('[aria-label="Local metadata recovery"]');
  assert(node, "Native Preferences recovery form is missing");
  return node;
}
function button(label: string, root: ParentNode = document): HTMLButtonElement {
  const node = [...root.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) => item.textContent?.trim() === label || item.getAttribute("aria-label") === label,
  );
  assert(node, `Native UI button is missing: ${label}`);
  return node;
}
function control(label: string, root: ParentNode = document): HTMLInputElement | HTMLSelectElement {
  const explicit = [
    ...root.querySelectorAll<HTMLInputElement | HTMLSelectElement>("input,select"),
  ].find((item) => item.getAttribute("aria-label") === label);
  const implicit = [...root.querySelectorAll("label")]
    .find((item) => item.textContent?.trim().startsWith(label))
    ?.querySelector<HTMLInputElement | HTMLSelectElement>("input,select");
  const node = explicit ?? implicit;
  assert(node, `Native UI control is missing: ${label}`);
  return node;
}
function fill(label: string, value: string, root: ParentNode = document) {
  const node = control(label, root);
  assert(!node.disabled, `Native UI control is disabled: ${label}`);
  const prototype =
    node instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
  assert(setter, "Native control value setter is unavailable");
  setter.call(node, value);
  node.dispatchEvent(
    new Event(node instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }),
  );
}
async function clickReady(label: string, root: ParentNode = document) {
  await until(() => !button(label, root).disabled, `Native UI action stayed disabled: ${label}`);
  button(label, root).click();
}
async function openPreferences() {
  const preferences = [...document.querySelectorAll<HTMLButtonElement>("button")].find((item) =>
    [item.textContent?.trim(), item.getAttribute("aria-label"), item.getAttribute("title")].some(
      (name) => name?.startsWith("Preferences"),
    ),
  );
  assert(preferences && !preferences.disabled, "Native Preferences entry is unavailable");
  preferences.click();
  await until(
    () => Boolean(document.querySelector('[aria-label="Local metadata recovery"]')),
    "Native Preferences did not open",
  );
}
async function runForm(kind: "create" | "restore", selected?: RecoveryBackup, secret = key) {
  const form = recovery();
  if (selected) {
    fill("Metadata backup", selected.metadata.id, form);
    await until(
      () => control("Metadata backup", recovery()).value === selected.metadata.id,
      "Backup selection did not commit",
    );
  }
  fill("Backup passphrase", secret, form);
  fill("Confirm backup passphrase", secret, form);
  if (kind === "restore") fill("Type RESTORE to replace metadata", "RESTORE", form);
  const action = kind === "create" ? "Create metadata backup" : "Restore metadata";
  await clickReady(action, form);
  let id: string | null = null;
  await until(() => {
    id = sessionStorage.getItem(marker);
    return !!id && uuid.test(id);
  }, "Native UI did not persist its original recovery ID");
  assert(id, "Native original operation identity is missing");
  const originalId: string = id;
  let terminal: RecoveryOperation | null = null;
  await until(
    async () => {
      let value: RecoveryOperation;
      try {
        value = await client.call<RecoveryOperation>("metadata_recovery_operation", {
          id: originalId,
        });
      } catch (error) {
        // The UI retains its UUID before the one original start reaches native admission.
        // Only the exact domain absence is not-ready; ACL/transport errors still fail.
        if (appErrorOf(error)?.code === "notFound") return false;
        throw error;
      }
      assert(value.id === originalId && value.kind === kind, "Native operation identity changed");
      if (value.state === "running") return false;
      terminal = value;
      return true;
    },
    "Native encrypted operation did not settle within its production KDF budget",
    180000,
  );
  await until(
    () => sessionStorage.getItem(marker) === null,
    "Native UI did not finish terminal metadata refresh",
    15000,
  );
  assert(
    control("Backup passphrase", recovery()).value === "" &&
      control("Confirm backup passphrase", recovery()).value === "",
    "Native UI retained a backup key after completion",
  );
  assert(terminal, "Native terminal status is missing");
  return terminal as RecoveryOperation;
}
async function expectCode(
  command:
    | "delete_recovery_backup"
    | "metadata_recovery_operation"
    | "cancel_metadata_recovery"
    | "reconcile_metadata_recovery"
    | "start_metadata_recovery"
    | "set_snowflake_console_receipt_archive"
    | "release_snowflake_console_results",
  args: Record<string, unknown>,
  code: string,
) {
  try {
    await client.call<unknown>(command, args);
  } catch (error) {
    const actual = appErrorOf(error);
    assert(
      actual?.code === code,
      `${command} returned ${actual?.code ?? "non-AppError (possible invoke ACL denial)"}; expected ${code}`,
    );
    return;
  }
  throw new Error(`${command} unexpectedly succeeded`);
}
function emptyResources(usage: ResourceUsage) {
  return (
    usage.sessionCount === 0 &&
    usage.resultCount === 0 &&
    usage.activeRuns === 0 &&
    usage.retainedBytes === 0 &&
    usage.temporaryResultBytes === 0
  );
}
function zeroSql(counts: Counts) {
  return counts.connect === 0 && counts.prepareExecution === 0 && counts.execute === 0;
}
async function checkpoint() {
  return client.call<Checkpoint>("native_smoke_checkpoint", { phase: "options", cycle: null });
}

/** Isolated hidden main-WebView integration; never imported by the normal entry. */
export async function runNativeEncryptedRecovery(): Promise<boolean> {
  const initial = await checkpoint();
  const stage = initial.options.recoveryStage;
  if (stage !== "encryptedSetup" && stage !== "encryptedVerify") return false;
  const started = performance.now();
  const report: Record<string, unknown> = { passed: false, recoveryStage: stage, checks: [] };
  try {
    await until(
      () =>
        Boolean(
          useStudio.getState().bootstrap && document.querySelector('[aria-label="New query tab"]'),
        ),
      "Native App did not initialize",
    );
    const bootstrap = await client.call<Bootstrap>("bootstrap");
    assert(
      !bootstrap.passphraseConfigured,
      "Disposable native recovery unexpectedly requires an owner write key",
    );
    assert(
      emptyResources(initial.usage) && zeroSql(initial.invocations),
      "Native startup created SQL resources",
    );
    assert(Object.keys(useStudio.getState().sessions).length === 0, "Native UI restored sessions");
    const checks: Record<string, boolean> = {};
    await openPreferences();
    if (stage === "encryptedSetup") {
      assert(
        (await client.call<boolean>("metadata_recovery_admission_closed")) === false,
        "Fresh native process has closed recovery admission",
      );
      assert(
        (await client.call<RecoveryBackup[]>("recovery_backups")).length === 0,
        "Isolated native root is not empty",
      );
      const missing = crypto.randomUUID();
      await expectCode(
        "delete_recovery_backup",
        { id: missing, digest: "0".repeat(64), confirmation: "WRONG" },
        "invalidInput",
      );
      await expectCode("metadata_recovery_operation", { id: missing }, "notFound");
      await expectCode("cancel_metadata_recovery", { id: missing }, "notFound");
      await expectCode("reconcile_metadata_recovery", { id: "not-a-uuid" }, "invalidInput");
      checks.closedReadEnrolled = true;
      checks.inventoryEnrolled = true;
      checks.deleteEnrolled = true;
      checks.lookupEnrolled = true;
      checks.cancelEnrolled = true;
      checks.reconcileEnrolled = true;

      const receipts = await client.call<SnowflakeConsoleReceiptPage>(
        "snowflake_console_receipts",
        {
          request: {
            filter: { visibility: "all", kind: null, connectionId: null },
            cursor: null,
            limit: 1,
          },
        },
      );
      assert(
        uuid.test(receipts.inventoryEpoch) &&
          receipts.receipts.length === 0 &&
          receipts.nextCursor === null,
        "Native empty receipt inventory is invalid",
      );
      assert(
        (await client.call("snowflake_console_receipt_summary", { clientRequestId: missing })) ===
          null,
        "Native missing receipt summary was fabricated",
      );
      const pending = await client.call<SnowflakeConsolePending>("snowflake_console_pending");
      assert(
        pending.clientRequestIds.length === 0 && pending.recordingHold === false,
        "Native empty journal has pending work",
      );
      const storage = await client.call<SnowflakeConsoleStorage>("snowflake_console_storage");
      assert(
        storage.inventoryEpoch === receipts.inventoryEpoch &&
          storage.positiveCount === "0" &&
          storage.refusalCount === "0",
        "Native empty receipt storage is invalid",
      );
      await expectCode(
        "set_snowflake_console_receipt_archive",
        { request: { inventoryEpoch: receipts.inventoryEpoch, items: [], archived: true } },
        "invalidInput",
      );
      await expectCode(
        "release_snowflake_console_results",
        { clientRequestId: missing },
        "notFound",
      );
      // This disposal API intentionally returns unit for missing tokens.
      assert(
        (await client.call("release_csv_import_preview", {
          sessionId: missing,
          sourceToken: "missing-native-source",
          previewToken: "missing-native-preview",
        })) === null,
        "Native CSV preview release returned an unexpected value",
      );
      Object.assign(checks, {
        receiptInventoryEnrolled: true,
        receiptSummaryEnrolled: true,
        receiptPendingEnrolled: true,
        receiptStorageEnrolled: true,
        receiptArchiveEnrolled: true,
        receiptReleaseEnrolled: true,
        csvReleaseEnrolled: true,
      });

      const created = await runForm("create");
      assert(
        created.state === "succeeded" && created.backup?.format === "encryptedV1",
        "Native production encrypted create failed",
      );
      const original = created.backup;
      checks.createFromPreferences = true;
      const groupName = `Native encrypted recovery ${crypto.randomUUID()}`;
      fill("Group name", groupName);
      fill("Parent group", "local");
      await clickReady("Add group");
      let groupId = "";
      await until(async () => {
        const value = await client.call<Bootstrap>("bootstrap");
        groupId = value.groups.find((group) => group.name === groupName)?.id ?? "";
        return (
          uuid.test(groupId) &&
          useStudio.getState().bootstrap?.groups.some((group) => group.id === groupId) === true
        );
      }, "Native local group mutation was not committed");
      const beforeWrong = await client.call<Bootstrap>("bootstrap");
      const inventoryBeforeWrong = await client.call<RecoveryBackup[]>("recovery_backups");
      const rejected = await runForm("restore", original, wrongKey);
      assert(
        rejected.state === "failed" && rejected.backup === null,
        "Wrong key fabricated a successful restore or undo",
      );
      const afterWrong = await client.call<Bootstrap>("bootstrap");
      assert(
        JSON.stringify(beforeWrong.groups) === JSON.stringify(afterWrong.groups) &&
          JSON.stringify(beforeWrong.profiles) === JSON.stringify(afterWrong.profiles),
        "Wrong key changed live metadata",
      );
      assert(
        JSON.stringify(inventoryBeforeWrong) ===
          JSON.stringify(await client.call<RecoveryBackup[]>("recovery_backups")),
        "Wrong key changed retained backups",
      );
      assert(
        recovery().textContent?.includes("Recovery operation failed; no operation was retried."),
        "Preferences did not report the actual wrong-key failure",
      );
      checks.wrongKeyPreservedMetadata = true;
      checks.wrongKeyPreservedInventory = true;

      const restored = await runForm("restore", original);
      assert(
        restored.state === "succeeded" && restored.backup?.format === "encryptedV1",
        "Native restore did not retain encrypted undo evidence",
      );
      assert(
        !(await client.call<Bootstrap>("bootstrap")).groups.some((group) => group.id === groupId),
        "Native restore did not replace the post-snapshot group",
      );
      assert(
        !useStudio.getState().bootstrap?.groups.some((group) => group.id === groupId),
        "Native UI did not reload restored metadata",
      );
      checks.restoreFromPreferences = true;
      const undo = restored.backup;
      assert(undo.metadata.id !== original.metadata.id, "Native restore reused the source as undo");
      const undone = await runForm("restore", undo);
      assert(
        undone.state === "succeeded" && undone.backup?.format === "encryptedV1",
        "Native encrypted undo failed",
      );
      assert(
        (await client.call<Bootstrap>("bootstrap")).groups.some(
          (group) => group.id === groupId && group.name === groupName,
        ),
        "Native undo lost the original group",
      );
      assert(
        useStudio.getState().bootstrap?.groups.some((group) => group.id === groupId),
        "Preferences did not reload encrypted undo",
      );
      checks.undoFromPreferences = true;

      // Delete only this test's redundant undo-of-undo through the real ACL.
      await client.call("delete_recovery_backup", {
        id: undone.backup.metadata.id,
        digest: undone.backup.metadata.digest,
        confirmation: "DELETE",
      });
      const backups = await client.call<RecoveryBackup[]>("recovery_backups");
      assert(
        backups.length === 2 &&
          backups.every((entry) => entry.format === "encryptedV1") &&
          backups.some((entry) => entry.metadata.id === original.metadata.id) &&
          backups.some((entry) => entry.metadata.id === undo.metadata.id),
        "Native delete changed unrelated recovery artifacts",
      );
      checks.exactDeleteSucceeded = true;

      const fenced = await client.call<RecoveryOperation>("reconcile_unknown_metadata_recovery");
      assert(
        uuid.test(fenced.id) &&
          fenced.kind === null &&
          fenced.state === "failed" &&
          fenced.error?.code === "outcomeUnknown",
        "Native global fence fabricated an original operation outcome",
      );
      assert(
        (await client.call<boolean>("metadata_recovery_admission_closed")) === true,
        "Native global fence did not close admission",
      );
      await expectCode(
        "start_metadata_recovery",
        {
          id: crypto.randomUUID(),
          request: { kind: "create", backupId: null, digest: null, confirmation: "" },
          backupPassphrase: key,
          backupConfirmation: key,
          writePassphrase: null,
        },
        "conflict",
      );
      await clickReady("Refresh backup inventory", recovery());
      await until(
        () =>
          recovery().textContent?.includes("Recovery admission is closed for this app process") ===
          true,
        "Preferences did not read native closure",
      );
      assert(
        button("Create metadata backup", recovery()).disabled &&
          button("Import encrypted backup…", recovery()).disabled,
        "Preferences reopened closed native recovery admission",
      );
      checks.globalFenceEnrolled = true;
      checks.closedAdmissionRejectsNewStart = true;
      const final = await checkpoint();
      assert(
        emptyResources(final.usage) && zeroSql(final.invocations),
        "Encrypted metadata workflow dispatched SQL",
      );
      checks.noSqlOrSessions = true;
      report.encryptedRecovery = {
        groupId,
        groupName,
        backups,
        operationIds: [created.id, rejected.id, restored.id, undone.id],
        checks,
        invocations: final.invocations,
      } satisfies SetupEvidence;
      report.passed = true;
      report.frontendElapsedMs = performance.now() - started;
      await client.call("native_smoke_report", { report });
      // Real App close-request handling flushes local state; no test app.exit.
      button("Done").click();
      await getCurrentWindow().close();
      await delay(5000);
      throw new Error("Native encrypted setup did not close its actual window");
    }

    const setup = initial.recoverySetup;
    assert(
      setup?.passed && setup.recoveryStage === "encryptedSetup",
      "Missing successful encrypted setup evidence",
    );
    const evidence = setup.encryptedRecovery;
    assert(evidence && uuid.test(evidence.groupId), "Invalid encrypted setup identity");
    assert(
      bootstrap.groups.some(
        (group) => group.id === evidence.groupId && group.name === evidence.groupName,
      ),
      "Native reopen did not retain the undone group",
    );
    const reopenedBackups = await client.call<RecoveryBackup[]>("recovery_backups");
    report.reopenedBackups = reopenedBackups;
    assert(
      sameRecoveryInventory(reopenedBackups, evidence.backups),
      "Native reopen changed encrypted backup identities",
    );
    assert(
      (await client.call<boolean>("metadata_recovery_admission_closed")) === false,
      "Native restart did not reset the process-only admission fence",
    );
    assert(
      sessionStorage.getItem(marker) === null,
      "Native reopen restored a pending operation marker",
    );
    assert(
      useStudio
        .getState()
        .tabs.every((tab) => tab.querySession === null && tab.results.length === 0),
      "Native reopen restored live SQL resources",
    );
    const final = await checkpoint();
    assert(
      emptyResources(final.usage) && zeroSql(final.invocations),
      "Native reopen replayed a connection or SQL operation",
    );
    Object.assign(checks, {
      groupRestored: true,
      encryptedInventoryRetained: true,
      freshProcessAdmissionOpen: true,
      noRestoredSessionsOrResults: true,
      noSqlReplay: true,
    });
    report.encryptedRecovery = {
      checks,
      invocations: final.invocations,
      groupId: evidence.groupId,
      backups: reopenedBackups,
    };
    report.passed = true;
  } catch (error) {
    report.passed = false;
    report.error = messageOf(error);
  }
  report.frontendElapsedMs = performance.now() - started;
  await client.call("native_smoke_report", { report });
  return true;
}
