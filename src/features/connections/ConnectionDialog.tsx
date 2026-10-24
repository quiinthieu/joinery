import { useState, type FormEvent } from "react";
import { Database, Loader2, ShieldCheck } from "lucide-react";
import { Modal, InlineError } from "../../components/ui/Modal";
import { client, desktopAvailable, messageOf } from "../../ipc/client";
import type { ConnectionProfile, SafeMode, SessionInfo } from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import { useSshTrust } from "./SshHostTrust";
import { orderedGroups } from "./GroupManager";

export function ConnectionDialog({
  profile,
  groupId = "local",
  onClose,
}: {
  profile?: ConnectionProfile;
  groupId?: string;
  onClose(): void;
}) {
  const bootstrap = useStudio((s) => s.bootstrap);
  const initialGroup = bootstrap?.groups.find((g) => g.id === (profile?.groupId ?? groupId));
  const [draft, setDraft] = useState<ConnectionProfile>(
    () =>
      profile ?? {
        id: crypto.randomUUID(),
        name: "",
        groupId,
        engine: "postgres",
        host: "localhost",
        port: 5432,
        database: "postgres",
        username: "postgres",
        environment: initialGroup?.environment ?? "local",
        safeMode:
          initialGroup?.environment === "production"
            ? "passwordGated"
            : initialGroup?.environment === "staging"
              ? "confirm"
              : "silent",
        tlsMode: "verifyFull",
        caCertPath: null,
        ssh: null,
        rememberPassword: false,
        storeQueryText: false,
        snowflake: null,
      },
  );
  const [password, setPassword] = useState(""),
    [passphrase, setPassphrase] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null);
  const [sshPassword, setSshPassword] = useState(""),
    [keyPassphrase, setKeyPassphrase] = useState("");
  const trust = useSshTrust();
  const [deleting, setDeleting] = useState(false);
  const production =
    bootstrap?.groups.find((g) => g.id === draft.groupId)?.environment === "production";
  function change<K extends keyof ConnectionProfile>(key: K, value: ConnectionProfile[K]) {
    setDraft((draft) => ({ ...draft, [key]: value }));
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const saved = await client.call<ConnectionProfile>("save_profile", {
        profile: draft,
        passphrase: passphrase || null,
      });
      useStudio.getState().resetConnectionResults(saved.id);
      useStudio.getState().updateProfile(saved);
      await trust.verify(saved);
      const session = await client.call<SessionInfo>("connect", {
        id: saved.id,
        secrets: {
          password: password || null,
          sshPassword: sshPassword || null,
          keyPassphrase: keyPassphrase || null,
        },
      });
      useStudio.getState().setSession(session);
      await useStudio.getState().refreshCatalog(saved.id);
      useStudio.getState().notify(`Connected to ${saved.name}`);
      onClose();
    } catch (error) {
      setError(messageOf(error));
    } finally {
      setPassword("");
      setPassphrase("");
      setSshPassword("");
      setKeyPassphrase("");
      setBusy(false);
    }
  }
  if (trust.dialog) return trust.dialog;
  return (
    <Modal
      title={profile ? "Edit connection" : "New connection"}
      subtitle="A direct line to your data."
      onClose={onClose}
      busy={busy}
    >
      <form onSubmit={(event) => void submit(event)}>
        <div className="modal-body form-grid">
          <label className="span-2">
            Connection name
            <input
              autoFocus
              value={draft.name}
              onChange={(e) => change("name", e.target.value)}
              placeholder="e.g. Local development"
              required
              maxLength={100}
            />
          </label>
          <label>
            Database engine
            <select
              value={draft.engine}
              onChange={(e) => {
                const engine = e.target.value as ConnectionProfile["engine"];
                setDraft((current) => ({
                  ...current,
                  engine,
                  host: engine === "snowflake" ? "" : "localhost",
                  port:
                    engine === "snowflake"
                      ? 443
                      : engine === "redis"
                        ? 6379
                        : engine === "mysql"
                          ? 3306
                          : 5432,
                  database: engine === "redis" ? "0" : engine === "postgres" ? "postgres" : "",
                  username:
                    engine === "redis" ? "default" : engine === "postgres" ? "postgres" : "",
                  safeMode: engine === "snowflake" ? "readOnly" : current.safeMode,
                  tlsMode: "verifyFull",
                  caCertPath: null,
                  ssh: null,
                  rememberPassword: false,
                  snowflake:
                    engine === "snowflake" ? { role: "", warehouse: "", schema: "" } : null,
                }));
              }}
            >
              {bootstrap?.supportedEngines.map((engine) => (
                <option key={engine} value={engine}>
                  {engine === "postgres"
                    ? "PostgreSQL"
                    : engine === "mysql"
                      ? "MySQL / MariaDB"
                      : engine === "redis"
                        ? "Redis"
                        : engine}
                </option>
              ))}
            </select>
          </label>
          <label>
            Group
            <select
              aria-label="Connection group"
              value={draft.groupId}
              onChange={(e) => {
                const group = bootstrap?.groups.find((g) => g.id === e.target.value);
                setDraft((draft) => ({
                  ...draft,
                  groupId: e.target.value,
                  environment: group?.environment ?? "local",
                  safeMode:
                    draft.engine === "snowflake"
                      ? "readOnly"
                      : group?.environment === "production"
                        ? "passwordGated"
                        : group?.environment === "staging"
                          ? "confirm"
                          : "silent",
                  tlsMode: group?.environment === "production" ? "verifyFull" : draft.tlsMode,
                }));
              }}
            >
              {orderedGroups(bootstrap?.groups ?? []).map(({ group, path }) => (
                <option value={group.id} key={group.id}>
                  {path}
                </option>
              ))}
            </select>
          </label>
          <label>
            {draft.engine === "snowflake" ? "Snowflake account hostname" : "Host"}
            <input
              value={draft.host}
              onChange={(e) => change("host", e.target.value)}
              required
              spellCheck={false}
              placeholder={
                draft.engine === "snowflake" ? "org-account.snowflakecomputing.com" : undefined
              }
            />
          </label>
          {draft.engine === "snowflake" && draft.snowflake && (
            <>
              <label>
                Role
                <input
                  value={draft.snowflake.role}
                  onChange={(e) =>
                    change("snowflake", { ...draft.snowflake!, role: e.target.value })
                  }
                  required
                  spellCheck={false}
                  placeholder="READ_ONLY_ROLE"
                />
              </label>
              <label>
                Warehouse
                <input
                  value={draft.snowflake.warehouse}
                  onChange={(e) =>
                    change("snowflake", { ...draft.snowflake!, warehouse: e.target.value })
                  }
                  required
                  spellCheck={false}
                />
              </label>
              <label className="span-2">
                Schema
                <input
                  value={draft.snowflake.schema}
                  onChange={(e) =>
                    change("snowflake", { ...draft.snowflake!, schema: e.target.value })
                  }
                  required
                  spellCheck={false}
                />
              </label>
              <p className="form-note span-2">
                Joinery opens your system browser for Snowflake sign-in. Read-only SQL and bounded
                table previews need the account’s local-application OAuth integration.
              </p>
            </>
          )}
          <label>
            Port
            <input
              type="number"
              min={1}
              max={65535}
              value={draft.port}
              onChange={(e) => change("port", Number(e.target.value))}
              required
            />
          </label>
          <label>
            {draft.engine === "redis" ? "Database index" : "Database"}
            <input
              value={draft.database}
              onChange={(e) => change("database", e.target.value)}
              required
              spellCheck={false}
            />
          </label>
          <label>
            Username
            <input
              value={draft.username}
              onChange={(e) => change("username", e.target.value)}
              required
              autoComplete="username"
              spellCheck={false}
            />
          </label>
          {draft.engine !== "snowflake" && (
            <label className="span-2">
              Password
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="current-password"
                placeholder="Enter for this session"
              />
            </label>
          )}
          {draft.engine !== "snowflake" && (
            <label className="checkbox-label span-2">
              <input
                type="checkbox"
                checked={draft.rememberPassword}
                onChange={(e) => change("rememberPassword", e.target.checked)}
              />
              Remember password in the system credential store
            </label>
          )}
          {draft.engine !== "snowflake" && (
            <label className="checkbox-label span-2">
              <input
                type="checkbox"
                checked={!!draft.ssh}
                onChange={(e) =>
                  change(
                    "ssh",
                    e.target.checked
                      ? {
                          host: "",
                          port: 22,
                          username: draft.username,
                          authentication: "agent",
                          privateKeyPath: null,
                          agentSocketPath: null,
                        }
                      : null,
                  )
                }
              />
              Connect through an SSH bastion
            </label>
          )}
          {draft.ssh && (
            <>
              <label>
                SSH host
                <input
                  value={draft.ssh.host}
                  onChange={(e) => change("ssh", { ...draft.ssh!, host: e.target.value })}
                  required
                  spellCheck={false}
                />
              </label>
              <label>
                SSH port
                <input
                  type="number"
                  min={1}
                  max={65535}
                  value={draft.ssh.port}
                  onChange={(e) => change("ssh", { ...draft.ssh!, port: Number(e.target.value) })}
                  required
                />
              </label>
              <label>
                SSH username
                <input
                  value={draft.ssh.username}
                  onChange={(e) => change("ssh", { ...draft.ssh!, username: e.target.value })}
                  required
                  spellCheck={false}
                />
              </label>
              <label>
                SSH authentication
                <select
                  aria-label="SSH authentication"
                  value={draft.ssh.authentication}
                  onChange={(e) =>
                    change("ssh", {
                      ...draft.ssh!,
                      authentication: e.target.value as NonNullable<
                        ConnectionProfile["ssh"]
                      >["authentication"],
                    })
                  }
                >
                  <option value="agent">SSH agent</option>
                  <option value="privateKey">Private key</option>
                  <option value="password">Password</option>
                </select>
              </label>
              {draft.ssh.authentication === "password" && (
                <label className="span-2">
                  SSH password
                  <input
                    type="password"
                    value={sshPassword}
                    onChange={(e) => setSshPassword(e.target.value)}
                    autoComplete="off"
                  />
                </label>
              )}
              {draft.ssh.authentication === "agent" && (
                <label className="span-2">
                  Agent socket path <span className="optional">optional</span>
                  <input
                    value={draft.ssh.agentSocketPath ?? ""}
                    onChange={(e) =>
                      change("ssh", { ...draft.ssh!, agentSocketPath: e.target.value || null })
                    }
                    spellCheck={false}
                    placeholder="Use SSH_AUTH_SOCK or the Windows OpenSSH agent"
                  />
                </label>
              )}
              {draft.ssh.authentication === "privateKey" && (
                <>
                  <label className="span-2">
                    SSH private key path
                    <input
                      value={draft.ssh.privateKeyPath ?? ""}
                      onChange={(e) =>
                        change("ssh", { ...draft.ssh!, privateKeyPath: e.target.value })
                      }
                      required
                      spellCheck={false}
                      placeholder="/Users/you/.ssh/id_ed25519"
                    />
                  </label>
                  <label className="span-2">
                    Key passphrase
                    <input
                      type="password"
                      value={keyPassphrase}
                      onChange={(e) => setKeyPassphrase(e.target.value)}
                      autoComplete="off"
                    />
                  </label>
                </>
              )}
              <p className="form-note span-2">
                The database host is resolved from the bastion. SSH credentials stay in this
                session; agent forwarding is disabled.
              </p>
            </>
          )}
          <div className="form-divider span-2">
            <ShieldCheck size={14} /> CONNECTION SAFETY
          </div>
          <label className="checkbox-label span-2">
            <input
              type="checkbox"
              checked={draft.tlsMode === "verifyFull"}
              disabled={production || draft.engine === "snowflake"}
              onChange={(e) => change("tlsMode", e.target.checked ? "verifyFull" : "disable")}
            />
            Require TLS and verify the server certificate
          </label>
          {draft.tlsMode === "verifyFull" && draft.engine !== "snowflake" && (
            <label className="span-2">
              CA certificate path <span className="optional">optional</span>
              <input
                value={draft.caCertPath ?? ""}
                onChange={(e) => change("caCertPath", e.target.value || null)}
                placeholder="Use system trust, or provide a PEM certificate path"
                spellCheck={false}
              />
            </label>
          )}
          <label className="span-2">
            Safe Mode
            <select
              aria-label="Safe Mode"
              value={draft.safeMode}
              disabled={draft.engine === "snowflake"}
              onChange={(e) => change("safeMode", e.target.value as SafeMode)}
            >
              {!production && draft.engine !== "snowflake" && (
                <>
                  <option value="silent">Silent · run without confirmations</option>
                  <option value="confirm">Confirm · review writes</option>
                  <option value="guard">Guard · stronger checks for unrestricted writes</option>
                </>
              )}
              {draft.engine !== "snowflake" && (
                <option value="passwordGated">Password gated · authorize each write batch</option>
              )}
              <option value="readOnly">Read only · block writes</option>
            </select>
          </label>
          <label className="checkbox-label span-2">
            <input
              type="checkbox"
              checked={draft.storeQueryText}
              onChange={(e) => change("storeQueryText", e.target.checked)}
            />
            Save SQL text in local history{" "}
            <span className="optional">may contain sensitive literals</span>
          </label>
          {profile?.environment === "production" && (
            <label className="span-2">
              Application write passphrase
              <input
                type="password"
                value={passphrase}
                onChange={(e) => setPassphrase(e.target.value)}
                autoComplete="current-password"
                placeholder="Required to change this protected profile"
              />
            </label>
          )}
          {profile && (
            <p className="form-note span-2">
              Saving reconnects this profile and closes its existing results.
            </p>
          )}
          {deleting && (
            <p className="form-note span-2">
              Removing this saved connection also deletes its saved password and closes its
              workspaces.
            </p>
          )}
          <div className="span-2">
            <InlineError
              message={
                error ?? (!desktopAvailable ? "Open the desktop app to save connections." : null)
              }
            />
          </div>
        </div>
        <footer className="modal-footer">
          {profile && (
            <button
              type="button"
              className="button danger-text delete-profile"
              disabled={busy}
              onClick={() => {
                if (!deleting) {
                  setDeleting(true);
                  return;
                }
                setBusy(true);
                void client
                  .call("delete_profile", { id: profile.id, passphrase: passphrase || null })
                  .then(() => {
                    useStudio.getState().removeConnection(profile.id);
                    onClose();
                  })
                  .catch((error) => setError(messageOf(error)))
                  .finally(() => {
                    setPassphrase("");
                    setBusy(false);
                  });
              }}
            >
              {deleting ? "Remove saved connection" : "Delete connection"}
            </button>
          )}
          <button type="button" className="button ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button type="submit" className="button primary" disabled={busy || !desktopAvailable}>
            {busy ? <Loader2 size={15} className="spin" /> : <Database size={15} />}{" "}
            {busy ? "Connecting…" : "Save and connect"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}

export function ConnectDialog({
  profile,
  onClose,
}: {
  profile: ConnectionProfile;
  onClose(): void;
}) {
  const [password, setPassword] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null);
  const [sshPassword, setSshPassword] = useState(""),
    [keyPassphrase, setKeyPassphrase] = useState("");
  const trust = useSshTrust();
  async function connect(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await trust.verify(profile);
      const session = await client.call<SessionInfo>("connect", {
        id: profile.id,
        secrets: {
          password: password || null,
          sshPassword: sshPassword || null,
          keyPassphrase: keyPassphrase || null,
        },
      });
      useStudio.getState().setSession(session);
      await useStudio.getState().refreshCatalog(profile.id);
      onClose();
    } catch (error) {
      setError(messageOf(error));
    } finally {
      setPassword("");
      setSshPassword("");
      setKeyPassphrase("");
      setBusy(false);
    }
  }
  if (trust.dialog) return trust.dialog;
  return (
    <Modal
      title={`Connect to ${profile.name}`}
      subtitle={`${profile.username}@${profile.host}:${profile.port}/${profile.database}`}
      onClose={onClose}
      busy={busy}
    >
      <form onSubmit={(e) => void connect(e)}>
        <div className="modal-body">
          {profile.engine !== "snowflake" ? (
            <label>
              Password
              <input
                autoFocus
                type="password"
                autoComplete="current-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={
                  profile.rememberPassword
                    ? "Leave blank to use the saved password"
                    : "Enter for this session"
                }
              />
            </label>
          ) : (
            <p className="form-note">Your system browser will open for Snowflake sign-in.</p>
          )}
          {profile.ssh?.authentication === "password" && (
            <label>
              SSH password
              <input
                type="password"
                value={sshPassword}
                onChange={(e) => setSshPassword(e.target.value)}
                autoComplete="off"
              />
            </label>
          )}
          {profile.ssh?.authentication === "privateKey" && (
            <label>
              Key passphrase
              <input
                type="password"
                value={keyPassphrase}
                onChange={(e) => setKeyPassphrase(e.target.value)}
                autoComplete="off"
              />
            </label>
          )}
          <InlineError message={error} />
        </div>
        <footer className="modal-footer">
          <button type="button" className="button ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="button primary" type="submit" disabled={busy}>
            {busy && <Loader2 size={14} className="spin" />}Connect
          </button>
        </footer>
      </form>
    </Modal>
  );
}
