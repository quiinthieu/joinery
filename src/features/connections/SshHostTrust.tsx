import { useEffect, useRef, useState } from "react";
import { ShieldCheck } from "lucide-react";
import { Modal, InlineError } from "../../components/ui/Modal";
import { client, messageOf } from "../../ipc/client";
import type { ConnectionProfile, SshHostKey } from "../../ipc/generated";

export function useSshTrust() {
  const [prompt, setPrompt] = useState<SshHostKey | null>(null),
    [error, setError] = useState<string | null>(null),
    [busy, setBusy] = useState(false);
  const pending = useRef<{ resolve(): void; reject(error: Error): void } | null>(null);
  useEffect(
    () => () => {
      pending.current?.reject(new Error("SSH host verification cancelled"));
    },
    [],
  );
  async function verify(profile: ConnectionProfile) {
    if (!profile.ssh) return;
    const key = await client.call<SshHostKey>("probe_ssh", { profileId: profile.id });
    if (key.previousFingerprint && key.previousFingerprint !== key.fingerprint)
      throw new Error(
        `SSH host key changed. Connection blocked. Expected ${key.previousFingerprint}; received ${key.fingerprint}. Verify the server identity with its administrator.`,
      );
    if (!key.challengeId) return;
    setError(null);
    setPrompt(key);
    await new Promise<void>((resolve, reject) => {
      pending.current = { resolve, reject };
    });
  }
  function cancel() {
    pending.current?.reject(new Error("SSH host verification cancelled"));
    pending.current = null;
    setPrompt(null);
  }
  const dialog = prompt ? (
    <Modal
      key="ssh-host-trust"
      title="Verify SSH host"
      subtitle={`${prompt.host}:${prompt.port}`}
      onClose={cancel}
      busy={busy}
    >
      <div className="modal-body">
        <p className="confirmation-description">
          This is the first connection to this SSH host. Compare its fingerprint with one from the
          server administrator before trusting it.
        </p>
        <pre className="host-fingerprint">{prompt.fingerprint}</pre>
        <InlineError message={error} />
      </div>
      <footer className="modal-footer">
        <button className="button ghost" onClick={cancel} disabled={busy}>
          Cancel
        </button>
        <button
          className="button primary"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            void client
              .call("trust_ssh_key", {
                challengeId: prompt.challengeId,
                fingerprint: prompt.fingerprint,
              })
              .then(() => {
                pending.current?.resolve();
                pending.current = null;
                setPrompt(null);
              })
              .catch((error) => setError(messageOf(error)))
              .finally(() => setBusy(false));
          }}
        >
          <ShieldCheck size={14} />
          Trust this host key
        </button>
      </footer>
    </Modal>
  ) : null;
  return { verify, dialog };
}
