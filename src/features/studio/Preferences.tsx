import { useState, type FormEvent } from "react";
import { Check, LockKeyhole, Monitor, Moon, Sun, Loader2 } from "lucide-react";
import { Modal, InlineError } from "../../components/ui/Modal";
import { client, desktopAvailable, messageOf } from "../../ipc/client";
import { useStudio } from "../../state/studio";

export function Preferences({ onClose }: { onClose(): void }) {
  const state = useStudio(), [password, setPassword] = useState(""), [previous, setPrevious] = useState(""), [busy, setBusy] = useState(false), [error, setError] = useState<string | null>(null);
  async function save(event: FormEvent) {
    event.preventDefault(); setBusy(true); setError(null);
    try { await client.call("set_passphrase", { passphrase: password, previous: previous || null }); await state.initialize(); state.notify("Write passphrase updated"); setPassword(""); setPrevious(""); }
    catch (error) { setError(messageOf(error)); } finally { setBusy(false); }
  }
  return <Modal title="Preferences" subtitle="Make this space your own." onClose={onClose} busy={busy}>
    <div className="modal-body preferences"><h3>Appearance</h3><div className="theme-options">{([{ id: "dark", label: "Forest dark", icon: Moon }, { id: "light", label: "Paper light", icon: Sun }, { id: "contrast", label: "High contrast", icon: Monitor }] as const).map(({ id, label, icon: Icon }) => <button key={id} className={`theme-option ${state.theme === id ? "selected" : ""}`} onClick={() => state.setTheme(id)}><Icon size={19} /><span>{label}</span>{state.theme === id && <Check size={13} />}</button>)}</div>
      <div className="preference-divider" /><h3><LockKeyhole size={15} />Write authorization</h3><p className="form-note">Production writes require this application passphrase and approval of the exact query batch.</p>
      <form onSubmit={(e) => void save(e)} className="passphrase-form">
        {state.bootstrap?.passphraseConfigured && <label>Current passphrase<input type="password" value={previous} onChange={(e) => setPrevious(e.target.value)} autoComplete="current-password" required /></label>}
        <label>{state.bootstrap?.passphraseConfigured ? "New passphrase" : "Set a write passphrase"}<input type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" minLength={12} maxLength={1024} placeholder="At least 12 characters" required /></label>
        <InlineError message={error} /><button className="button primary" type="submit" disabled={busy || !desktopAvailable}>{busy && <Loader2 size={14} className="spin" />}{state.bootstrap?.passphraseConfigured ? "Update passphrase" : "Set passphrase"}</button>
      </form><div className="preference-divider" /><h3>Data privacy</h3><p className="form-note">Result rows stay in memory. Passwords use your system credential store. Query text is saved only when enabled for a connection.</p>
    </div><footer className="modal-footer"><button className="button ghost" onClick={onClose} disabled={busy}>Done</button></footer>
  </Modal>;
}
