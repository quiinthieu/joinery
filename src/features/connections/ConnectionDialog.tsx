import { useState, type FormEvent } from "react";
import { Database, Loader2, ShieldCheck } from "lucide-react";
import { Modal, InlineError } from "../../components/ui/Modal";
import { client, desktopAvailable, messageOf } from "../../ipc/client";
import type {
  ConnectionProfile,
  SafeMode,
  SessionInfo,
  SnowflakeAuthentication,
} from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import { useSshTrust } from "./SshHostTrust";
import { orderedGroups } from "./GroupManager";
import { useConnectionSetup } from "./connectionSetup";

function snowflakeAuthentication(config: ConnectionProfile["snowflake"]): SnowflakeAuthentication {
  return config?.authentication ?? "localApplicationOauth";
}

function withSnowflakeAuthentication(
  config: Omit<NonNullable<ConnectionProfile["snowflake"]>, "authentication">,
  authentication: SnowflakeAuthentication,
): NonNullable<ConnectionProfile["snowflake"]> {
  return { ...config, authentication };
}

function completeConnection(profile: ConnectionProfile, session: SessionInfo, onClose: () => void) {
  const state = useStudio.getState();
  state.setSession(session);
  state.notify(`Connected to ${profile.name}`);
  onClose();
  void state.refreshCatalog(profile.id).catch((error) => {
    const latest = useStudio.getState();
    if (latest.sessions[profile.id]?.id === session.id)
      latest.notify(`Connected to ${profile.name}. Could not load tables: ${messageOf(error)}`);
  });
}

export function ConnectionDialog({
  profile: initialProfile,
  groupId = "local",
  onClose,
}: {
  profile?: ConnectionProfile;
  groupId?: string;
  onClose(): void;
}) {
  const bootstrap = useStudio((s) => s.bootstrap);
  const [profile, setProfile] = useState(initialProfile);
  const initialGroup = bootstrap?.groups.find((g) => g.id === (profile?.groupId ?? groupId));
  const [draft, setDraft] = useState<ConnectionProfile>(() => {
    if (profile) {
      if (profile.engine === "snowflake" && profile.snowflake) {
        const authentication = snowflakeAuthentication(profile.snowflake);
        const environment = initialGroup?.environment ?? profile.environment;
        return {
          ...profile,
          safeMode:
            authentication !== "externalBrowser"
              ? "readOnly"
              : environment === "production" && profile.safeMode !== "readOnly"
                ? "passwordGated"
                : profile.safeMode,
          snowflake: withSnowflakeAuthentication(profile.snowflake, authentication),
        };
      }
      return profile;
    }
    return {
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
      storeDrafts: false,
      snowflake: null,
    };
  });
  const [password, setPassword] = useState(""),
    [passphrase, setPassphrase] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null);
  const [sshPassword, setSshPassword] = useState(""),
    [keyPassphrase, setKeyPassphrase] = useState("");
  const trust = useSshTrust();
  const setup = useConnectionSetup();
  function close() {
    setup.cancel();
    onClose();
  }
  const [deleting, setDeleting] = useState(false);
  const production =
    bootstrap?.groups.find((g) => g.id === draft.groupId)?.environment === "production";
  function change<K extends keyof ConnectionProfile>(key: K, value: ConnectionProfile[K]) {
    setDraft((draft) => ({ ...draft, [key]: value }));
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    const attempt = setup.begin();
    if (!attempt) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await client.call<ConnectionProfile>("save_profile", {
        profile: draft,
        passphrase: passphrase || null,
      });
      if (!attempt.active) return;
      setProfile(saved);
      useStudio.getState().resetConnectionResults(saved.id);
      useStudio.getState().updateProfile(saved);
      const attemptId = await attempt.prepare(saved.id);
      if (!attemptId) return;
      await trust.verify(saved, attemptId, () => attempt.active);
      await attempt.connect(
        saved.id,
        {
          password: password || null,
          sshPassword: sshPassword || null,
          keyPassphrase: keyPassphrase || null,
        },
        (session) => completeConnection(saved, session, onClose),
      );
    } catch (error) {
      if (attempt.active) setError(messageOf(error));
    } finally {
      if (attempt.active) {
        setPassword("");
        setPassphrase("");
        setSshPassword("");
        setKeyPassphrase("");
        setBusy(false);
      }
      setup.finish(attempt);
    }
  }
  if (trust.dialog) return trust.dialog;
  if (deleting && profile)
    return (
      <Modal
        key="delete"
        title="Delete connection?"
        subtitle={profile.name}
        onClose={onClose}
        busy={busy}
      >
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (busy) return;
            setBusy(true);
            setError(null);
            void client
              .call("delete_profile", { id: profile.id })
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
          <div className="modal-body">
            <p className="form-note">
              Removing this saved connection also deletes its saved password and query drafts, and
              closes its workspaces. This cannot be undone.
            </p>
            <InlineError message={error} />
          </div>
          <footer className="modal-footer">
            <button
              type="button"
              className="button ghost"
              disabled={busy}
              onClick={() => {
                setDeleting(false);
                setError(null);
                setPassphrase("");
              }}
            >
              Cancel
            </button>
            <button type="submit" className="button danger" disabled={busy}>
              {busy ? "Removing…" : "Remove saved connection"}
            </button>
          </footer>
        </form>
      </Modal>
    );
  return (
    <Modal
      key="edit"
      title={profile ? "Edit connection" : "New connection"}
      subtitle="A direct line to your data."
      onClose={close}
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
                    engine === "snowflake"
                      ? current.engine === "snowflake" && current.snowflake
                        ? current.snowflake
                        : withSnowflakeAuthentication(
                            { role: "", warehouse: "", schema: "" },
                            "externalBrowser",
                          )
                      : null,
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
                        : engine === "snowflake"
                          ? "Snowflake"
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
                      ? snowflakeAuthentication(draft.snowflake) !== "externalBrowser" ||
                        draft.safeMode === "readOnly"
                        ? "readOnly"
                        : group?.environment === "production"
                          ? "passwordGated"
                          : draft.safeMode
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
              <label className="span-2">
                Snowflake authentication
                <select
                  aria-label="Snowflake authentication"
                  value={snowflakeAuthentication(draft.snowflake)}
                  onChange={(event) => {
                    const authentication = event.target.value as SnowflakeAuthentication;
                    setDraft((current) => ({
                      ...current,
                      safeMode:
                        authentication === "externalBrowser" &&
                        snowflakeAuthentication(current.snowflake) === "externalBrowser"
                          ? current.safeMode
                          : "readOnly",
                      snowflake: current.snowflake
                        ? withSnowflakeAuthentication(current.snowflake, authentication)
                        : null,
                    }));
                  }}
                >
                  <option value="externalBrowser">External browser SSO (Snowflake SDK)</option>
                  <option value="localApplicationOauth">Local application OAuth (SQL API)</option>
                </select>
              </label>
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
                {snowflakeAuthentication(draft.snowflake) === "externalBrowser" ? (
                  <>
                    External browser SSO uses the Snowflake SDK. SQL access is read-only; metadata
                    browsing is temporarily unavailable for this sign-in mode. Joinery will not fall
                    back to local application OAuth.
                  </>
                ) : (
                  <>
                    Local application OAuth uses Snowflake SQL API. SQL access is read-only, and
                    bounded table previews require the account’s OAuth integration. Joinery will not
                    fall back to external browser SSO.
                  </>
                )}
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
              disabled={
                draft.engine === "snowflake" &&
                snowflakeAuthentication(draft.snowflake) !== "externalBrowser"
              }
              onChange={(e) => change("safeMode", e.target.value as SafeMode)}
            >
              {draft.engine === "snowflake" &&
              snowflakeAuthentication(draft.snowflake) === "externalBrowser" ? (
                <>
                  {!production && (
                    <>
                      <option value="silent">Silent · no boundary confirmation</option>
                      <option value="confirm">Confirm · review explicit boundaries</option>
                      <option value="guard">Guard · review explicit boundaries</option>
                    </>
                  )}
                  <option value="passwordGated">Password gated · authorize each boundary</option>
                </>
              ) : null}
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
          {draft.engine === "snowflake" && (
            <p className="form-note span-2">
              {snowflakeAuthentication(draft.snowflake) === "externalBrowser"
                ? "Safe Mode applies only to separately reviewed explicit transaction boundaries. SQL writes and DDL remain blocked."
                : "This authentication route supports read-only SQL."}
            </p>
          )}
          <label className="checkbox-label span-2">
            <input
              type="checkbox"
              checked={draft.storeQueryText}
              onChange={(e) => change("storeQueryText", e.target.checked)}
            />
            Save SQL text in local history{" "}
            <span className="optional">may contain sensitive literals</span>
          </label>
          <label className="checkbox-label span-2">
            <input
              type="checkbox"
              checked={draft.storeDrafts ?? false}
              onChange={(event) => change("storeDrafts", event.target.checked)}
            />
            Restore open query drafts after restart{" "}
            <span className="optional">saves text locally; may contain sensitive literals</span>
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
                setError(null);
                setPassphrase("");
                setDeleting(true);
              }}
            >
              Delete connection
            </button>
          )}
          <button type="button" className="button ghost" onClick={close}>
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
  onConnected,
}: {
  profile: ConnectionProfile;
  onClose(): void;
  onConnected?(session: SessionInfo): void;
}) {
  const [password, setPassword] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null);
  const [sshPassword, setSshPassword] = useState(""),
    [keyPassphrase, setKeyPassphrase] = useState("");
  const trust = useSshTrust();
  const setup = useConnectionSetup();
  function close() {
    setup.cancel();
    onClose();
  }
  async function connect(event: FormEvent) {
    event.preventDefault();
    const attempt = setup.begin();
    if (!attempt) return;
    setBusy(true);
    setError(null);
    try {
      const attemptId = await attempt.prepare(profile.id);
      if (!attemptId) return;
      await trust.verify(profile, attemptId, () => attempt.active);
      await attempt.connect(
        profile.id,
        {
          password: password || null,
          sshPassword: sshPassword || null,
          keyPassphrase: keyPassphrase || null,
        },
        (session) => {
          if (onConnected) {
            onConnected(session);
            onClose();
          } else completeConnection(profile, session, onClose);
        },
      );
    } catch (error) {
      if (attempt.active) setError(messageOf(error));
    } finally {
      if (attempt.active) {
        setPassword("");
        setSshPassword("");
        setKeyPassphrase("");
        setBusy(false);
      }
      setup.finish(attempt);
    }
  }
  if (trust.dialog) return trust.dialog;
  return (
    <Modal
      title={onConnected ? `New SQL session for ${profile.name}` : `Connect to ${profile.name}`}
      subtitle={`${profile.username}@${profile.host}:${profile.port}/${profile.database}`}
      onClose={close}
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
          <button type="button" className="button ghost" onClick={close}>
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
