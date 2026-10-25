import { useEffect, useState } from "react";
import type { MetadataBackup } from "../../ipc/generated";
import { client, desktopAvailable, messageOf } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import { InlineError } from "../../components/ui/Modal";

export function MetadataRecovery({ onBusy }: { onBusy(busy: boolean): void }) {
  const state = useStudio();
  const [backups, setBackups] = useState<MetadataBackup[]>([]);
  const [selected, setSelected] = useState<MetadataBackup | null>(null);
  const [confirmation, setConfirmation] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const blocked = state.tabs.length > 0 || Object.keys(state.sessions).length > 0;
  useEffect(() => {
    if (!desktopAvailable) return;
    let active = true;
    void client
      .call<MetadataBackup[]>("metadata_backups")
      .then((rows) => {
        if (active) setBackups(rows);
      })
      .catch((error) => {
        if (active) setError(messageOf(error));
      });
    return () => {
      active = false;
    };
  }, []);

  async function perform(action: () => Promise<void>) {
    setBusy(true);
    onBusy(true);
    setError(null);
    setNotice(null);
    try {
      await action();
    } catch (error) {
      setError(messageOf(error));
    } finally {
      setBusy(false);
      onBusy(false);
      setPassphrase("");
    }
  }

  return (
    <section aria-label="Local metadata recovery">
      <h3>Backup and recovery</h3>
      <p className="form-note">
        Snapshots contain connection profiles, groups, local settings, SSH trust and permitted query
        history. Passwords, passphrases and result rows are excluded. Backups stay in this device’s
        application data; keep a separate copy for disk-loss recovery.
      </p>
      <button
        className="button ghost"
        disabled={busy || !desktopAvailable}
        onClick={() =>
          void perform(async () => {
            await client.call<MetadataBackup>("create_metadata_backup");
            setBackups(await client.call("metadata_backups"));
            setNotice("Local metadata snapshot created.");
          })
        }
      >
        Create metadata backup
      </button>
      <label>
        Metadata backup
        <select
          aria-label="Metadata backup"
          value={selected?.id ?? ""}
          disabled={busy}
          onChange={(event) => {
            setSelected(backups.find((backup) => backup.id === event.target.value) ?? null);
            setConfirmation("");
            setPassphrase("");
            setError(null);
          }}
        >
          <option value="">Choose a snapshot ({backups.length}/20)</option>
          {backups.map((backup) => (
            <option key={backup.id} value={backup.id}>
              {new Date(backup.createdAt).toLocaleString()} · {Math.ceil(backup.sizeBytes / 1024)}{" "}
              KiB · {backup.id.slice(0, 8)}
            </option>
          ))}
        </select>
      </label>
      {selected && (
        <>
          <p className="form-note">
            Restore replaces the local profiles, groups, settings, SSH trust and history. The
            current write passphrase and system credentials are retained. An undo snapshot is
            created first. No database writes are undone, and nothing reconnects or runs
            automatically.
          </p>
          {blocked && (
            <p className="form-note" role="status">
              Disconnect every connection and close all workspace tabs before restoring.
            </p>
          )}
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
                value={passphrase}
                disabled={busy}
                onChange={(event) => setPassphrase(event.target.value)}
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
                confirmation !== "RESTORE" ||
                Boolean(state.bootstrap?.passphraseConfigured && !passphrase)
              }
              onClick={() =>
                void perform(async () => {
                  const undo = await client.call<MetadataBackup>("restore_metadata_backup", {
                    id: selected.id,
                    digest: selected.digest,
                    confirmation,
                    passphrase: passphrase || null,
                  });
                  useStudio.setState({
                    selectedConnectionId: null,
                    catalog: {},
                    tabs: [],
                    activeTabId: null,
                  });
                  await state.initialize();
                  setBackups(await client.call("metadata_backups"));
                  setSelected(null);
                  setConfirmation("");
                  setNotice(
                    `Metadata restored. Undo snapshot: ${undo.id.slice(0, 8)}. Reconnect manually when ready.`,
                  );
                })
              }
            >
              Restore metadata
            </button>
            <button
              className="button ghost"
              disabled={busy || confirmation !== "DELETE"}
              onClick={() =>
                void perform(async () => {
                  await client.call("delete_metadata_backup", {
                    id: selected.id,
                    digest: selected.digest,
                  });
                  setSelected(null);
                  setConfirmation("");
                  setBackups(await client.call("metadata_backups"));
                  setNotice("Snapshot deleted.");
                })
              }
            >
              Delete snapshot
            </button>
          </div>
          <p className="form-note">
            To remove this snapshot instead, type DELETE above. Keep fewer than 20 snapshots so
            restore has room for its undo copy. Each snapshot is limited to 64 MiB.
          </p>
        </>
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
