import { useEffect, useRef, useState } from "react";
import type { RecoveryBackup, RecoveryOperation, RecoveryOperationKind } from "../../ipc/generated";
import { client, desktopAvailable, messageOf } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import { InlineError } from "../../components/ui/Modal";
import {
  RECOVERY_OPERATION_KEY,
  discardRecoveryWorkspaceSaves,
  flushWorkspace,
  pauseWorkspaceForRecovery,
  resumeWorkspaceAfterRecovery,
  workspaceRecoveryStartupHeld,
} from "../../state/workspace";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const phases = [
  "staging",
  "validating",
  "encrypting",
  "decrypting",
  "merging",
  "creatingUndo",
  "replacing",
  "finalizing",
];
const kinds = ["create", "restore", "import", "export"];

function rememberedOperation(): string | null {
  try {
    return (
      sessionStorage.getItem(RECOVERY_OPERATION_KEY) ??
      (workspaceRecoveryStartupHeld() ? "unavailable" : null)
    );
  } catch {
    return "unavailable";
  }
}

export function validBackupPassphrase(value: string): boolean {
  const count = Array.from(value).length;
  return (
    count >= 12 &&
    count <= 1024 &&
    !/[\uD800-\uDFFF]/u.test(value) &&
    new TextEncoder().encode(value).length <= 4096
  );
}

function validBackup(value: RecoveryBackup): boolean {
  const metadata = value?.metadata;
  return (
    !!metadata &&
    UUID.test(metadata.id) &&
    Number.isSafeInteger(metadata.createdAt) &&
    metadata.createdAt >= 0 &&
    Number.isSafeInteger(metadata.sizeBytes) &&
    metadata.sizeBytes >= 0 &&
    /^[a-f0-9]{64}$/.test(metadata.digest) &&
    (value.format === "encryptedV1" || value.format === "legacySqlite")
  );
}

function validOperation(
  value: RecoveryOperation,
  id: string,
  prior: RecoveryOperation | null,
): boolean {
  return (
    !!value &&
    value.id === id &&
    (value.kind === null
      ? value.state === "failed" && value.error?.code === "outcomeUnknown"
      : kinds.includes(value.kind)) &&
    (!prior || prior.kind === value.kind) &&
    (!prior || prior.state === "running" || prior.state === value.state) &&
    ["running", "succeeded", "failed", "cancelled"].includes(value.state) &&
    !!value.progress &&
    phases.includes(value.progress.phase) &&
    Number.isSafeInteger(value.progress.completedBytes) &&
    value.progress.completedBytes >= 0 &&
    (value.progress.totalBytes === null ||
      (Number.isSafeInteger(value.progress.totalBytes) &&
        value.progress.totalBytes >= value.progress.completedBytes)) &&
    typeof value.cancellationRequested === "boolean" &&
    (value.backup === null || validBackup(value.backup))
  );
}

function notFound(error: unknown) {
  return !!error && typeof error === "object" && "code" in error && error.code === "notFound";
}

export function MetadataRecovery({ onBusy }: { onBusy(busy: boolean): void }) {
  const state = useStudio();
  const [backups, setBackups] = useState<RecoveryBackup[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [backupPassphrase, setBackupPassphrase] = useState("");
  const [backupConfirmation, setBackupConfirmation] = useState("");
  const [writePassphrase, setWritePassphrase] = useState("");
  const [pendingId, setPendingId] = useState(rememberedOperation);
  const [operation, setOperation] = useState<RecoveryOperation | null>(null);
  const [working, setWorking] = useState(false);
  const [unknown, setUnknown] = useState(false);
  const [admissionClosed, setAdmissionClosed] = useState(false);
  const [admissionKnown, setAdmissionKnown] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const idRef = useRef(pendingId);
  const operationRef = useRef<RecoveryOperation | null>(null);
  const expectedKind = useRef<RecoveryOperationKind | null>(null);
  const workingRef = useRef(false);
  const startPending = useRef(false);
  const settling = useRef(false);
  const mounted = useRef(true);
  const selected = backups.find((backup) => backup.metadata.id === selectedId) ?? null;
  const busy = working || pendingId !== null;
  const blocked = state.tabs.length > 0 || Object.keys(state.sessions).length > 0;
  const secretValid =
    validBackupPassphrase(backupPassphrase) && backupPassphrase === backupConfirmation;

  function clearSecrets() {
    setBackupPassphrase("");
    setBackupConfirmation("");
    setWritePassphrase("");
  }

  async function reloadBackups() {
    setAdmissionKnown(false);
    const [rows, closed] = await Promise.all([
      client.call<RecoveryBackup[]>("recovery_backups"),
      client.call<boolean>("metadata_recovery_admission_closed"),
    ]);
    if (typeof closed !== "boolean")
      throw new Error("Native recovery admission is unknown; no recovery action was taken.");
    if (
      !Array.isArray(rows) ||
      rows.length > 20 ||
      !rows.every(validBackup) ||
      new Set(rows.map((row) => row.metadata.id)).size !== rows.length
    ) {
      throw new Error("The native backup inventory is invalid; no recovery action was taken.");
    }
    if (mounted.current) {
      setBackups(rows);
      // Native closure is permanent for this process, including across remounts.
      setAdmissionClosed((previous) => previous || closed);
      setAdmissionKnown(true);
    }
  }

  async function reloadMetadata() {
    // Clear queued old snapshots before initialize publishes the restored one.
    discardRecoveryWorkspaceSaves();
    useStudio.setState({
      bootstrap: null,
      selectedConnectionId: null,
      catalog: {},
      objects: {},
      sessions: {},
      sessionPool: {},
      sessionStates: {},
      sessionContexts: {},
      sessionContextRevision: {},
      sessionContextAuthorities: {},
      sessionContextSwitching: {},
      sessionTransactionPending: {},
      sessionMetadataRefreshRequired: {},
      sessionDdlCatalogRefreshRequired: {},
      tabs: [],
      activeTabId: null,
      closedTabs: [],
      tableLayouts: [],
    });
    await useStudio.getState().initialize();
    if (!useStudio.getState().bootstrap)
      throw new Error("Metadata could not be reloaded. Workspace saving remains paused.");
  }

  function forgetOperation(discard: boolean) {
    sessionStorage.removeItem(RECOVERY_OPERATION_KEY);
    if (sessionStorage.getItem(RECOVERY_OPERATION_KEY) !== null)
      throw new Error("The recovery marker could not be cleared. Workspace saving remains paused.");
    idRef.current = null;
    if (mounted.current) setPendingId(null);
    resumeWorkspaceAfterRecovery(discard);
  }

  async function settle(value: RecoveryOperation) {
    if (
      settling.current ||
      value.state === "running" ||
      idRef.current !== value.id ||
      !mounted.current
    )
      return;
    settling.current = true;
    try {
      // A failed finalization can follow a committed replacement. Refresh every
      // terminal restore before any old workspace snapshot can be saved again.
      if (value.kind === "restore" || value.kind === null) await reloadMetadata();
      await reloadBackups();
      if (!mounted.current || idRef.current !== value.id) return;
      forgetOperation(value.kind === "restore" || value.kind === null);
      setSelectedId("");
      setConfirmation("");
      clearSecrets();
      setNotice(
        value.state === "succeeded"
          ? value.kind === "restore"
            ? `Metadata restored. Encrypted undo backup: ${value.backup?.metadata.id ?? "see backup inventory"}. Reconnect manually when ready.`
            : `${value.kind === "create" ? "Encrypted backup created" : value.kind === "import" ? "Encrypted backup imported" : "Encrypted backup exported"}.`
          : value.state === "cancelled"
            ? "Recovery operation cancelled; no operation was retried."
            : "Recovery operation failed; no operation was retried.",
      );
      setError(
        value.state === "failed"
          ? (value.error?.message ?? "The native recovery operation failed.")
          : null,
      );
    } catch (failure) {
      if (mounted.current) setError(messageOf(failure));
    } finally {
      settling.current = false;
    }
  }

  async function accept(value: RecoveryOperation, id: string) {
    if (!mounted.current || idRef.current !== id) return;
    if (
      (expectedKind.current !== null &&
        value.kind !== null &&
        value.kind !== expectedKind.current) ||
      !validOperation(value, id, operationRef.current)
    )
      throw new Error(
        "Recovery operation identity or status changed unexpectedly. Workspace saving remains paused.",
      );
    operationRef.current = value;
    setOperation(value);
    setUnknown(false);
    setError(null);
    await settle(value);
  }

  useEffect(() => {
    onBusy(busy);
  }, [busy, onBusy]);
  useEffect(() => {
    mounted.current = true;
    if (pendingId !== null) {
      void pauseWorkspaceForRecovery().catch((failure) => {
        if (mounted.current) setError(messageOf(failure));
      });
    } else if (desktopAvailable) {
      void reloadBackups().catch((failure) => {
        if (mounted.current) setError(messageOf(failure));
      });
    }
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (!pendingId || !desktopAvailable) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function lookup() {
      if (!active || idRef.current !== pendingId) return;
      if (!UUID.test(pendingId!)) {
        setUnknown(true);
        setError(
          "The saved recovery identifier is unavailable or malformed. Before reloading metadata, close all further recovery admission for this app process. The prior outcome remains unknown.",
        );
        return;
      }
      try {
        const value = await client.call<RecoveryOperation>("metadata_recovery_operation", {
          id: pendingId,
        });
        if (active) await accept(value, pendingId!);
      } catch (failure) {
        if (active && idRef.current === pendingId) {
          setUnknown(notFound(failure));
          setError(
            `Recovery status is unconfirmed: ${messageOf(failure)} No operation will be submitted again.`,
          );
        }
      }
      if (active && idRef.current === pendingId) timer = setTimeout(() => void lookup(), 250);
    }
    void lookup();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [pendingId]);

  async function start(kind: RecoveryOperationKind) {
    if (
      workingRef.current ||
      idRef.current ||
      blocked ||
      admissionClosed ||
      !admissionKnown ||
      !desktopAvailable
    )
      return;
    const secret = kind === "create" || kind === "restore";
    if (secret && !secretValid) return;
    if ((kind === "restore" || kind === "export") && !selected) return;
    if (
      kind === "restore" &&
      (confirmation !== "RESTORE" || (state.bootstrap?.passphraseConfigured && !writePassphrase))
    )
      return;
    if (kind === "export" && selected?.format !== "encryptedV1") return;
    const args = {
      id: crypto.randomUUID(),
      request: {
        kind,
        backupId: kind === "restore" || kind === "export" ? selected!.metadata.id : null,
        digest: kind === "restore" || kind === "export" ? selected!.metadata.digest : null,
        confirmation: kind === "restore" ? confirmation : "",
      },
      backupPassphrase: secret ? backupPassphrase : "",
      backupConfirmation: secret ? backupConfirmation : "",
      writePassphrase: kind === "restore" ? writePassphrase || null : null,
    };
    clearSecrets();
    setError(null);
    setNotice(null);
    setUnknown(false);
    workingRef.current = true;
    setWorking(true);
    let transmitted = false;
    let ownsMarker = false;
    try {
      await flushWorkspace();
      if (!mounted.current) return;
      if (sessionStorage.getItem(RECOVERY_OPERATION_KEY) !== null)
        throw new Error("Another recovery identifier is already retained; nothing was started.");
      sessionStorage.setItem(RECOVERY_OPERATION_KEY, args.id);
      ownsMarker = true;
      if (sessionStorage.getItem(RECOVERY_OPERATION_KEY) !== args.id)
        throw new Error("The operation identifier could not be retained; nothing was started.");
      await pauseWorkspaceForRecovery();
      if (!mounted.current) return;
      idRef.current = args.id;
      operationRef.current = null;
      expectedKind.current = kind;
      setOperation(null);
      setPendingId(args.id);
      startPending.current = true;
      transmitted = true;
      await accept(await client.call<RecoveryOperation>("start_metadata_recovery", args), args.id);
    } catch (failure) {
      if (mounted.current)
        setError(
          transmitted
            ? `The start response is unconfirmed: ${messageOf(failure)} Reading the same operation ID; nothing will be resubmitted.`
            : messageOf(failure),
        );
      if (!transmitted && ownsMarker) {
        try {
          forgetOperation(false);
        } catch (cleanupError) {
          if (mounted.current) setError(messageOf(cleanupError));
        }
      }
    } finally {
      // These bounded copies exist only for the original invocation.
      args.backupPassphrase = "";
      args.backupConfirmation = "";
      args.writePassphrase = null;
      startPending.current = false;
      workingRef.current = false;
      if (mounted.current) {
        setWorking(false);
        clearSecrets();
      }
    }
  }

  async function cancel() {
    const id = idRef.current;
    if (!id || !UUID.test(id) || workingRef.current) return;
    workingRef.current = true;
    setWorking(true);
    try {
      await accept(await client.call<RecoveryOperation>("cancel_metadata_recovery", { id }), id);
    } catch (failure) {
      if (mounted.current)
        setError(
          `Cancellation is unconfirmed: ${messageOf(failure)} Continue reading the actual operation status.`,
        );
    } finally {
      workingRef.current = false;
      if (mounted.current) setWorking(false);
    }
  }

  async function reconcileUnknown() {
    if (
      !unknown ||
      startPending.current ||
      workingRef.current ||
      settling.current ||
      !idRef.current
    )
      return;
    const retainedId = idRef.current;
    settling.current = true;
    workingRef.current = true;
    setWorking(true);
    try {
      const knownId = UUID.test(retainedId);
      // A per-ID tombstone can fence only that exact original ID. When it is
      // missing, the native process must close ALL future recovery admission.
      const reconciled = knownId
        ? await client.call<RecoveryOperation>("reconcile_metadata_recovery", { id: retainedId })
        : await client.call<RecoveryOperation>("reconcile_unknown_metadata_recovery");
      const id = knownId ? retainedId : reconciled?.id;
      if (
        typeof id !== "string" ||
        !UUID.test(id) ||
        !validOperation(reconciled, id, null) ||
        reconciled.state === "running" ||
        (!knownId &&
          (reconciled.kind !== null ||
            reconciled.state !== "failed" ||
            reconciled.error?.code !== "outcomeUnknown"))
      )
        throw new Error(
          "Native recovery reconciliation is incomplete. Workspace saving remains paused.",
        );
      if (!knownId) setAdmissionClosed(true);
      await reloadBackups();
      if (!mounted.current || idRef.current !== retainedId) return;
      await reloadMetadata();
      if (!mounted.current || idRef.current !== retainedId) return;
      forgetOperation(true);
      setOperation(null);
      operationRef.current = null;
      expectedKind.current = null;
      setUnknown(false);
      setError(null);
      setNotice(
        knownId
          ? "Native metadata reloaded. The prior operation outcome remains unknown; inspect the backup inventory. Nothing was retried."
          : "Native metadata reloaded. The prior operation outcome remains unknown; inspect the backup inventory. Nothing was retried. Further recovery requires quitting and restarting the full Joinery app; reloading this window is insufficient.",
      );
    } catch (failure) {
      if (mounted.current) setError(messageOf(failure));
    } finally {
      settling.current = false;
      workingRef.current = false;
      if (mounted.current) setWorking(false);
    }
  }

  async function remove() {
    if (!selected || busy || confirmation !== "DELETE") return;
    workingRef.current = true;
    setWorking(true);
    clearSecrets();
    setError(null);
    try {
      await client.call("delete_recovery_backup", {
        id: selected.metadata.id,
        digest: selected.metadata.digest,
        confirmation: "DELETE",
      });
      await reloadBackups();
      if (mounted.current) {
        setSelectedId("");
        setConfirmation("");
        setNotice("Local backup deleted.");
      }
    } catch (failure) {
      if (mounted.current) setError(messageOf(failure));
    } finally {
      workingRef.current = false;
      if (mounted.current) setWorking(false);
    }
  }

  return (
    <section aria-label="Local metadata recovery">
      <h3>Backup and recovery</h3>
      <p className="form-note">
        Encrypted backups contain profiles, groups, settings, SSH trust, permitted history and
        durable console receipts. Database passwords, write passphrases and result rows are
        excluded. Live metadata and temporary staging remain plaintext on this device. New encrypted
        backups cannot be read by older Joinery versions.
      </p>
      <p className="form-note">
        Keep a separate exported copy for disk-loss recovery. Limits: 20 local copies, 64 GiB
        aggregate encrypted storage and 16 GiB plaintext per snapshot. Restore requires room for its
        mandatory encrypted undo backup.
      </p>
      {admissionClosed && (
        <p className="form-note" role="status">
          Recovery admission is closed for this app process. Quit and restart the full Joinery app
          before another create, restore, import or export; reloading this window is insufficient.
        </p>
      )}
      {blocked && (
        <p className="form-note" role="status">
          Disconnect every connection and close all workspace tabs before creating, restoring,
          importing or exporting a backup. Native checks also require no connection setup, jobs,
          exports, retained results or pending recordings.
        </p>
      )}
      <fieldset disabled={busy || !desktopAvailable}>
        <legend>Separate backup passphrase</legend>
        <p className="form-note">
          Use 12–1024 Unicode characters (at most 4096 UTF-8 bytes). This is separate from your
          current write passphrase and is never saved. For an encrypted restore, enter the backup’s
          key; it also encrypts the undo copy. For a legacy restore, choose a new undo key.
        </p>
        <label>
          Backup passphrase
          <input
            aria-label="Backup passphrase"
            type="password"
            maxLength={2048}
            value={backupPassphrase}
            onChange={(event) => setBackupPassphrase(event.target.value)}
            autoComplete="new-password"
          />
        </label>
        <label>
          Confirm backup passphrase
          <input
            aria-label="Confirm backup passphrase"
            type="password"
            maxLength={2048}
            value={backupConfirmation}
            onChange={(event) => setBackupConfirmation(event.target.value)}
            autoComplete="new-password"
          />
        </label>
        {(backupPassphrase || backupConfirmation) && !secretValid && (
          <p className="form-note">
            Enter matching backup passphrases within the character and byte limits.
          </p>
        )}
      </fieldset>
      <div className="utility-actions">
        <button
          className="button ghost"
          disabled={
            busy ||
            blocked ||
            admissionClosed ||
            !admissionKnown ||
            !desktopAvailable ||
            !secretValid
          }
          onClick={() => void start("create")}
        >
          Create metadata backup
        </button>
        <button
          className="button ghost"
          disabled={busy || blocked || admissionClosed || !admissionKnown || !desktopAvailable}
          onClick={() => void start("import")}
        >
          Import encrypted backup…
        </button>
        <button
          className="button ghost"
          disabled={busy || !desktopAvailable}
          onClick={() => void reloadBackups().catch((failure) => setError(messageOf(failure)))}
        >
          Refresh backup inventory
        </button>
      </div>
      <label>
        Metadata backup
        <select
          aria-label="Metadata backup"
          value={selectedId}
          disabled={busy}
          onChange={(event) => {
            setSelectedId(event.target.value);
            setConfirmation("");
            clearSecrets();
            setError(null);
          }}
        >
          <option value="">Choose a backup ({backups.length}/20)</option>
          {backups.map((backup) => (
            <option key={backup.metadata.id} value={backup.metadata.id}>
              {new Date(backup.metadata.createdAt).toLocaleString()} ·{" "}
              {Math.ceil(backup.metadata.sizeBytes / 1024)} KiB ·{" "}
              {backup.format === "legacySqlite" ? "Legacy unencrypted backup" : "Encrypted backup"}{" "}
              · {backup.metadata.id.slice(0, 8)}
            </option>
          ))}
        </select>
      </label>
      {selected && (
        <>
          {selected.format === "legacySqlite" && (
            <p className="form-note">
              Legacy unencrypted backup: its original file remains unchanged. Restore requires a new
              confirmed passphrase for the mandatory encrypted undo copy. Only encrypted backups can
              be imported or exported here.
            </p>
          )}
          <p className="form-note">
            Restore replaces local metadata, retains the current write passphrase and system
            credentials, and first creates an encrypted undo backup. Remote database writes are not
            undone. Nothing reconnects or executes automatically.
          </p>
          <p className="form-note">Type DELETE instead to remove the selected local backup.</p>
          <label>
            Type RESTORE to replace metadata
            <input
              value={confirmation}
              disabled={busy}
              onChange={(event) => setConfirmation(event.target.value)}
              autoComplete="off"
              spellCheck={false}
            />
          </label>
          {state.bootstrap?.passphraseConfigured && (
            <label>
              Current write passphrase
              <input
                type="password"
                value={writePassphrase}
                disabled={busy}
                onChange={(event) => setWritePassphrase(event.target.value)}
                autoComplete="current-password"
                maxLength={1024}
              />
            </label>
          )}
          <div className="utility-actions">
            <button
              className="button danger"
              disabled={
                busy ||
                blocked ||
                admissionClosed ||
                !admissionKnown ||
                !desktopAvailable ||
                confirmation !== "RESTORE" ||
                !secretValid ||
                Boolean(state.bootstrap?.passphraseConfigured && !writePassphrase)
              }
              onClick={() => void start("restore")}
            >
              Restore metadata
            </button>
            <button
              className="button ghost"
              disabled={
                busy ||
                blocked ||
                admissionClosed ||
                !admissionKnown ||
                !desktopAvailable ||
                selected.format !== "encryptedV1"
              }
              onClick={() => void start("export")}
            >
              Export encrypted backup…
            </button>
            <button
              className="button ghost"
              disabled={busy || !desktopAvailable || confirmation !== "DELETE"}
              onClick={() => void remove()}
            >
              Delete snapshot
            </button>
          </div>
        </>
      )}
      {pendingId && (
        <div role="status" aria-label="Recovery operation status">
          <p>Workspace saving is paused. Operation {pendingId}.</p>
          {operation ? (
            <p>
              {operation.kind ?? "Unknown operation"}: {operation.state} ·{" "}
              {operation.progress.phase} · {operation.progress.completedBytes.toLocaleString()}
              {operation.progress.totalBytes === null
                ? " bytes"
                : ` / ${operation.progress.totalBytes.toLocaleString()} bytes`}
            </p>
          ) : (
            <p>Reading native operation status; no request will be repeated.</p>
          )}
          {operation?.state === "running" && (
            <>
              <p>
                Cancellation is a request. A bounded key-derivation step may need to finish;
                replacement and final publication cannot be cancelled.
              </p>
              <button
                className="button ghost"
                disabled={
                  working ||
                  operation.cancellationRequested ||
                  ["replacing", "finalizing"].includes(operation.progress.phase)
                }
                onClick={() => void cancel()}
              >
                {operation.cancellationRequested
                  ? "Cancellation requested"
                  : "Cancel recovery operation"}
              </button>
            </>
          )}
          {operation && operation.state !== "running" && (
            <button
              className="button ghost"
              disabled={working}
              onClick={() => void settle(operation)}
            >
              Finish metadata refresh
            </button>
          )}
          {unknown && (
            <button
              className="button ghost"
              disabled={working || startPending.current}
              onClick={() => void reconcileUnknown()}
            >
              Reload native metadata without retrying
            </button>
          )}
        </div>
      )}
      <InlineError message={error} />
      {notice && (
        <p role="status" className="form-note">
          {notice}
        </p>
      )}
    </section>
  );
}
