import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ConnectionDialog, ConnectDialog } from "./ConnectionDialog";
import { client } from "../../ipc/client";
import type {
  Bootstrap,
  ConnectionProfile,
  SessionInfo,
  SnowflakeAuthentication,
} from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import { cancelPendingConnections, hasPendingConnections } from "./connectionSetup";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  desktopAvailable: true,
  messageOf: (error: unknown) => (error as Error).message,
}));

const profile: ConnectionProfile = {
  id: "delete-fixture",
  name: "Delete fixture",
  groupId: "local",
  engine: "postgres",
  host: "localhost",
  port: 5432,
  database: "postgres",
  username: "postgres",
  environment: "local",
  safeMode: "readOnly",
  tlsMode: "disable",
  caCertPath: null,
  ssh: null,
  rememberPassword: false,
  storeQueryText: false,
  storeDrafts: false,
  snowflake: null,
};
const initialStudio = { ...useStudio.getState() };
const sshProfile: ConnectionProfile = {
  ...profile,
  ssh: {
    host: "bastion.fixture",
    port: 22,
    username: "fixture",
    authentication: "agent",
    privateKeyPath: null,
    agentSocketPath: null,
  },
};

function snowflakeProfile(authentication?: SnowflakeAuthentication) {
  const snowflake = {
    role: "READ_ONLY",
    warehouse: "COMPUTE",
    schema: "PUBLIC",
    ...(authentication ? { authentication } : {}),
  };
  // The undefined case deliberately models a persisted profile from before this field existed.
  return {
    ...profile,
    id: "snowflake-fixture",
    name: "Snowflake fixture",
    engine: "snowflake",
    host: "org-account.snowflakecomputing.com",
    port: 443,
    database: "ANALYTICS",
    username: "BILLIE",
    snowflake,
  } as unknown as ConnectionProfile;
}

function enableSnowflakeEngine() {
  useStudio.setState({
    bootstrap: {
      groups: [{ id: "local", name: "Local", environment: "local", parentId: null }],
      supportedEngines: ["postgres", "snowflake"],
    } as Bootstrap,
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

for (const mode of ["save", "connect"] as const) {
  test(`${mode}: unmount cancels a late reservation without dispatching authentication`, async () => {
    const reserved = deferred<string>();
    vi.mocked(client.call).mockImplementation(async (command) => {
      if (command === "save_profile") return profile;
      if (command === "prepare_connection") return reserved.promise;
      return null;
    });
    const onClose = vi.fn();
    const view = render(
      mode === "save" ? (
        <ConnectionDialog profile={profile} onClose={onClose} />
      ) : (
        <ConnectDialog profile={profile} onClose={onClose} />
      ),
    );
    fireEvent.submit(
      screen
        .getByRole("button", { name: mode === "save" ? "Save and connect" : "Connect" })
        .closest("form")!,
    );
    await waitFor(() =>
      expect(client.call).toHaveBeenCalledWith("prepare_connection", { id: profile.id }),
    );
    expect(hasPendingConnections()).toBe(true);
    view.unmount();
    reserved.resolve("late-reservation");
    await waitFor(() =>
      expect(client.call).toHaveBeenCalledWith("cancel_connection_attempt", {
        attemptId: "late-reservation",
      }),
    );
    await waitFor(() => expect(hasPendingConnections()).toBe(false));
    expect(
      vi.mocked(client.call).mock.calls.some(([command]) => command === "connect_prepared"),
    ).toBe(false);
    expect(onClose).not.toHaveBeenCalled();
  });

  test(`${mode}: unmount retires only the late session and never refreshes its catalog`, async () => {
    const connected = deferred<SessionInfo>();
    const setSession = vi.spyOn(useStudio.getState(), "setSession");
    const catalog = vi.spyOn(useStudio.getState(), "refreshCatalog");
    vi.mocked(client.call).mockImplementation(async (command) => {
      if (command === "save_profile") return profile;
      if (command === "prepare_connection") return "late-publish";
      if (command === "connect_prepared") return connected.promise;
      return null;
    });
    const view = render(
      mode === "save" ? (
        <ConnectionDialog profile={profile} onClose={vi.fn()} />
      ) : (
        <ConnectDialog profile={profile} onClose={vi.fn()} />
      ),
    );
    fireEvent.submit(
      screen
        .getByRole("button", { name: mode === "save" ? "Save and connect" : "Connect" })
        .closest("form")!,
    );
    await waitFor(() =>
      expect(
        vi.mocked(client.call).mock.calls.some(([command]) => command === "connect_prepared"),
      ).toBe(true),
    );
    view.unmount();
    connected.resolve({ id: "only-this-session", connectionId: profile.id } as SessionInfo);
    await waitFor(() =>
      expect(client.call).toHaveBeenCalledWith("disconnect", { sessionId: "only-this-session" }),
    );
    await waitFor(() => expect(hasPendingConnections()).toBe(false));
    expect(setSession).not.toHaveBeenCalled();
    expect(catalog).not.toHaveBeenCalled();
  });
}

test("cancelling a save prevents later sign-in and completion callbacks", async () => {
  const saved = deferred<ConnectionProfile>();
  vi.mocked(client.call).mockImplementation(async () => saved.promise);
  const onClose = vi.fn();
  render(<ConnectionDialog profile={profile} onClose={onClose} />);
  fireEvent.submit(screen.getByRole("button", { name: "Save and connect" }).closest("form")!);
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  saved.resolve(profile);
  await waitFor(() => expect(hasPendingConnections()).toBe(false));
  expect(onClose).toHaveBeenCalledOnce();
  expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).toEqual(["save_profile"]);
});

test("a late SSH probe after unmount cannot open a trust prompt or start authentication", async () => {
  const probed = deferred<unknown>();
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "prepare_connection") return "preflight-attempt";
    if (command === "probe_ssh") return probed.promise;
    return null;
  });
  const view = render(<ConnectDialog profile={sshProfile} onClose={vi.fn()} />);
  fireEvent.submit(screen.getByRole("button", { name: "Connect" }).closest("form")!);
  await waitFor(() =>
    expect(client.call).toHaveBeenCalledWith("probe_ssh", {
      profileId: profile.id,
      attemptId: "preflight-attempt",
    }),
  );
  view.unmount();
  // Cancellation must reach the backend while the probe response is still held.
  await waitFor(() =>
    expect(client.call).toHaveBeenCalledWith("cancel_connection_attempt", {
      attemptId: "preflight-attempt",
    }),
  );
  probed.resolve({ challengeId: "late-probe", fingerprint: "fixture" });
  await waitFor(() => expect(hasPendingConnections()).toBe(false));
  expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).toEqual([
    "prepare_connection",
    "probe_ssh",
    "cancel_connection_attempt",
  ]);
});

test("declining SSH host trust retires the reservation without starting authentication", async () => {
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "prepare_connection") return "declined-host";
    if (command === "probe_ssh")
      return { challengeId: "untrusted-host", fingerprint: "fixture", host: "bastion", port: 22 };
    return null;
  });
  render(<ConnectDialog profile={sshProfile} onClose={vi.fn()} />);
  fireEvent.submit(screen.getByRole("button", { name: "Connect" }).closest("form")!);
  await screen.findByRole("dialog", { name: "Verify SSH host" });
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await waitFor(() =>
    expect(client.call).toHaveBeenCalledWith("cancel_connection_attempt", {
      attemptId: "declined-host",
    }),
  );
  await waitFor(() => expect(hasPendingConnections()).toBe(false));
  expect(
    vi.mocked(client.call).mock.calls.some(([command]) => command === "connect_prepared"),
  ).toBe(false);
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "SSH host verification cancelled",
  );
});

test("verified SSH preflight and authentication reuse the exact reservation", async () => {
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "prepare_connection") return "one-ssh-attempt";
    if (command === "probe_ssh") return { challengeId: null, fingerprint: "fixture" };
    if (command === "connect_prepared") throw new Error("fixture authentication rejected");
    return null;
  });
  render(<ConnectDialog profile={sshProfile} onClose={vi.fn()} />);
  fireEvent.submit(screen.getByRole("button", { name: "Connect" }).closest("form")!);
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "fixture authentication rejected",
  );
  expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).toEqual([
    "prepare_connection",
    "probe_ssh",
    "connect_prepared",
    "cancel_connection_attempt",
  ]);
  expect(client.call).toHaveBeenCalledWith("connect_prepared", {
    attemptId: "one-ssh-attempt",
    secrets: { password: null, sshPassword: null, keyPassphrase: null },
  });
  await waitFor(() => expect(hasPendingConnections()).toBe(false));
});

test("cancel remains available during sign-in and late success cannot install a session", async () => {
  const onClose = vi.fn(),
    onConnected = vi.fn();
  let complete!: (session: SessionInfo) => void;
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "prepare_connection") return "pending-sign-in";
    if (command === "connect" || command === "connect_prepared")
      return new Promise<SessionInfo>((resolve) => {
        complete = resolve;
      });
    return null;
  });
  render(<ConnectDialog profile={profile} onClose={onClose} onConnected={onConnected} />);
  fireEvent.submit(screen.getByRole("button", { name: "Connect" }).closest("form")!);
  await waitFor(() => expect(complete).toBeTypeOf("function"));
  const cancel = screen.getByRole("button", { name: "Cancel" }) as HTMLButtonElement;
  expect(cancel.disabled).toBe(false);
  fireEvent.click(cancel);
  complete({ id: "late-session", connectionId: profile.id } as SessionInfo);
  await waitFor(() =>
    expect(client.call).toHaveBeenCalledWith("disconnect", { sessionId: "late-session" }),
  );
  expect(onConnected).not.toHaveBeenCalled();
  expect(onClose).toHaveBeenCalledOnce();
});

test("failed late-session cleanup stays visible and retries the exact session", async () => {
  const connected = deferred<SessionInfo>(),
    cancelled = deferred<void>();
  const notify = vi.spyOn(useStudio.getState(), "notify").mockImplementation(() => {});
  let disconnects = 0;
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "prepare_connection") return "cleanup-failure";
    if (command === "connect_prepared") return connected.promise;
    if (command === "cancel_connection_attempt") return cancelled.promise;
    if (command === "disconnect" && ++disconnects === 1) throw new Error("Close was interrupted");
    return null;
  });
  const view = render(<ConnectDialog profile={profile} onClose={vi.fn()} />);
  fireEvent.submit(screen.getByRole("button", { name: "Connect" }).closest("form")!);
  await waitFor(() =>
    expect(
      vi.mocked(client.call).mock.calls.some(([command]) => command === "connect_prepared"),
    ).toBe(true),
  );
  view.unmount();
  connected.resolve({ id: "failed-close-session", connectionId: profile.id } as SessionInfo);
  await waitFor(() =>
    expect(notify).toHaveBeenCalledWith("Connection cleanup failed: Close was interrupted"),
  );
  cancelled.resolve();
  await Promise.resolve();
  expect(hasPendingConnections()).toBe(true);
  await cancelPendingConnections();
  await waitFor(() => expect(hasPendingConnections()).toBe(false));
  expect(vi.mocked(client.call).mock.calls.filter(([command]) => command === "disconnect")).toEqual(
    [
      ["disconnect", { sessionId: "failed-close-session" }],
      ["disconnect", { sessionId: "failed-close-session" }],
    ],
  );
});

test("an isolated connection is handed to its tab without replacing the shared session", async () => {
  const setSession = vi.spyOn(useStudio.getState(), "setSession");
  const refresh = vi.spyOn(useStudio.getState(), "refreshCatalog");
  const onConnected = vi.fn(),
    onClose = vi.fn();
  const session = { id: "isolated", connectionId: profile.id } as SessionInfo;
  vi.mocked(client.call).mockResolvedValue(session);
  render(<ConnectDialog profile={profile} onClose={onClose} onConnected={onConnected} />);
  fireEvent.change(screen.getByLabelText("Password", { exact: true }), {
    target: { value: "one-use-fixture" },
  });
  fireEvent.submit(screen.getByRole("button", { name: "Connect" }).closest("form")!);
  await waitFor(() => expect(onConnected).toHaveBeenCalledWith(session));
  expect(setSession).not.toHaveBeenCalled();
  expect(refresh).not.toHaveBeenCalled();
  expect(onClose).toHaveBeenCalledOnce();
  await waitFor(() =>
    expect((screen.getByLabelText("Password", { exact: true }) as HTMLInputElement).value).toBe(""),
  );
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.resetAllMocks();
  useStudio.setState({ ...initialStudio }, true);
});

test("new Snowflake profiles default to external browser SSO and explain both modes", () => {
  enableSnowflakeEngine();
  render(<ConnectionDialog onClose={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("Database engine"), {
    target: { value: "snowflake" },
  });

  const authentication = screen.getByLabelText("Snowflake authentication") as HTMLSelectElement;
  expect(authentication.value).toBe("externalBrowser");
  const safeMode = screen.getByLabelText("Safe Mode") as HTMLSelectElement;
  expect(safeMode.value).toBe("readOnly");
  expect(safeMode.disabled).toBe(false);
  expect(screen.queryByLabelText(/allow separately reviewed single INSERT/i)).toBeNull();
  expect(screen.getByText(/Writable SQL requires separate consent/i)).toBeTruthy();
  expect(screen.getByText(/will not fall back to local application OAuth/i)).toBeTruthy();

  fireEvent.change(safeMode, { target: { value: "confirm" } });
  const reviewedDml = screen.getByLabelText(
    /allow separately reviewed single INSERT, UPDATE, and DELETE statements/i,
  ) as HTMLInputElement;
  expect(reviewedDml.checked).toBe(false);
  expect(screen.getAllByText(/may incur warehouse charges/i).length).toBeGreaterThan(0);
  expect(screen.getByText(/does not automatically retry submitted SQL/i)).toBeTruthy();
  expect(screen.getByText(/Safe Mode controls approval for opted-in operations/i)).toBeTruthy();

  fireEvent.change(authentication, { target: { value: "localApplicationOauth" } });
  expect(authentication.value).toBe("localApplicationOauth");
  expect(
    screen.getByText(/bounded table previews require the account’s OAuth integration/i),
  ).toBeTruthy();
  expect(screen.getByText(/will not fall back to external browser SSO/i)).toBeTruthy();
});

test("Snowflake OAuth transitions reset policy and never restore an earlier elevation", () => {
  enableSnowflakeEngine();
  render(<ConnectionDialog onClose={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("Database engine"), {
    target: { value: "snowflake" },
  });

  const authentication = screen.getByLabelText("Snowflake authentication") as HTMLSelectElement;
  const safeMode = screen.getByLabelText("Safe Mode") as HTMLSelectElement;
  fireEvent.change(safeMode, { target: { value: "confirm" } });
  expect(safeMode.value).toBe("confirm");
  const reviewedDmlLabel =
    /allow separately reviewed single INSERT, UPDATE, and DELETE statements/i;
  fireEvent.click(screen.getByLabelText(reviewedDmlLabel));
  expect((screen.getByLabelText(reviewedDmlLabel) as HTMLInputElement).checked).toBe(true);

  fireEvent.change(authentication, { target: { value: "localApplicationOauth" } });
  expect(safeMode.value).toBe("readOnly");
  expect(safeMode.disabled).toBe(true);
  expect(screen.queryByLabelText(reviewedDmlLabel)).toBeNull();

  fireEvent.change(authentication, { target: { value: "externalBrowser" } });
  expect(safeMode.value).toBe("readOnly");
  expect(safeMode.disabled).toBe(false);
  fireEvent.change(safeMode, { target: { value: "confirm" } });
  expect((screen.getByLabelText(reviewedDmlLabel) as HTMLInputElement).checked).toBe(false);
  expect(screen.getByText(/Safe Mode controls approval for opted-in operations/i)).toBeTruthy();
});

test("ReadOnly clears reviewed DML consent and does not restore it when policy is elevated", () => {
  enableSnowflakeEngine();
  render(<ConnectionDialog onClose={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("Database engine"), {
    target: { value: "snowflake" },
  });
  fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "confirm" } });
  const reviewedDmlLabel =
    /allow separately reviewed single INSERT, UPDATE, and DELETE statements/i;
  fireEvent.click(screen.getByLabelText(reviewedDmlLabel));
  fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "readOnly" } });
  expect(screen.queryByLabelText(reviewedDmlLabel)).toBeNull();
  fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "confirm" } });
  expect((screen.getByLabelText(reviewedDmlLabel) as HTMLInputElement).checked).toBe(false);

  fireEvent.click(screen.getByLabelText(reviewedDmlLabel));
  fireEvent.change(screen.getByLabelText("Database engine"), { target: { value: "postgres" } });
  fireEvent.change(screen.getByLabelText("Database engine"), { target: { value: "snowflake" } });
  fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "confirm" } });
  expect((screen.getByLabelText(reviewedDmlLabel) as HTMLInputElement).checked).toBe(false);
});

test("saving a legacy Snowflake profile submits reviewedDml false until explicitly opted in", async () => {
  const existing = snowflakeProfile("externalBrowser");
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "save_profile") return existing;
    if (command === "prepare_connection") throw new Error("Stop after profile save");
    return null;
  });
  render(<ConnectionDialog profile={existing} onClose={vi.fn()} />);

  fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "confirm" } });
  const reviewedDmlLabel =
    /allow separately reviewed single INSERT, UPDATE, and DELETE statements/i;
  expect((screen.getByLabelText(reviewedDmlLabel) as HTMLInputElement).checked).toBe(false);
  fireEvent.submit(screen.getByRole("button", { name: "Save and connect" }).closest("form")!);

  await waitFor(() => expect(client.call).toHaveBeenCalledWith("save_profile", expect.anything()));
  let save = vi.mocked(client.call).mock.calls.find(([command]) => command === "save_profile");
  expect(save?.[1]).toMatchObject({ profile: { snowflake: { reviewedDml: false } } });
  await screen.findByRole("alert");

  vi.mocked(client.call).mockClear();
  fireEvent.click(screen.getByLabelText(reviewedDmlLabel));
  fireEvent.submit(screen.getByRole("button", { name: "Save and connect" }).closest("form")!);

  await waitFor(() => expect(client.call).toHaveBeenCalledWith("save_profile", expect.anything()));
  save = vi.mocked(client.call).mock.calls.find(([command]) => command === "save_profile");
  expect(save?.[1]).toMatchObject({ profile: { snowflake: { reviewedDml: true } } });
});

test("Snowflake group changes preserve read-only and nonproduction choices, and normalize production", () => {
  enableSnowflakeEngine();
  const bootstrap = useStudio.getState().bootstrap!;
  useStudio.setState({
    bootstrap: {
      ...bootstrap,
      groups: [
        { id: "local", name: "Local", environment: "local", parentId: null },
        { id: "staging", name: "Staging", environment: "staging", parentId: null },
        { id: "production", name: "Production", environment: "production", parentId: null },
      ],
    },
  });
  render(<ConnectionDialog onClose={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("Database engine"), {
    target: { value: "snowflake" },
  });

  const group = screen.getByLabelText("Connection group");
  const safeMode = screen.getByLabelText("Safe Mode") as HTMLSelectElement;
  fireEvent.change(safeMode, { target: { value: "guard" } });
  fireEvent.change(group, { target: { value: "staging" } });
  expect(safeMode.value).toBe("guard");
  fireEvent.change(group, { target: { value: "local" } });
  expect(safeMode.value).toBe("guard");

  fireEvent.change(safeMode, { target: { value: "readOnly" } });
  fireEvent.change(group, { target: { value: "production" } });
  expect(safeMode.value).toBe("readOnly");
  expect(Array.from(safeMode.options, (option) => option.value)).toEqual([
    "passwordGated",
    "readOnly",
  ]);

  fireEvent.change(group, { target: { value: "local" } });
  expect(safeMode.value).toBe("readOnly");
  fireEvent.change(safeMode, { target: { value: "confirm" } });
  expect(safeMode.value).toBe("confirm");
  fireEvent.change(group, { target: { value: "production" } });
  expect(safeMode.value).toBe("passwordGated");
});

test.each([
  ["local application OAuth", "localApplicationOauth" as const],
  ["legacy missing authentication", undefined],
])("editing a Snowflake %s profile stays read-only", (_label, authentication) => {
  const existing = {
    ...snowflakeProfile(authentication),
    safeMode: "confirm" as const,
  };
  render(<ConnectionDialog profile={existing} onClose={vi.fn()} />);

  const safeMode = screen.getByLabelText("Safe Mode") as HTMLSelectElement;
  expect(safeMode.value).toBe("readOnly");
  expect(safeMode.disabled).toBe(true);
});

for (const [name, configured, expected] of [
  ["external browser SSO", "externalBrowser", "externalBrowser"],
  ["local application OAuth", "localApplicationOauth", "localApplicationOauth"],
  ["legacy missing authentication", undefined, "localApplicationOauth"],
] as const) {
  test(`editing a Snowflake profile preserves ${name}`, async () => {
    const existing = snowflakeProfile(configured);
    vi.mocked(client.call).mockImplementation(async (command) => {
      if (command === "save_profile") return existing;
      if (command === "prepare_connection") throw new Error("Stop after profile save");
      return null;
    });
    render(<ConnectionDialog profile={existing} onClose={vi.fn()} />);

    const authentication = screen.getByLabelText("Snowflake authentication") as HTMLSelectElement;
    expect(authentication.value).toBe(expected);
    fireEvent.change(screen.getByLabelText("Connection name", { exact: true }), {
      target: { value: "Updated Snowflake name" },
    });
    fireEvent.submit(screen.getByRole("button", { name: "Save and connect" }).closest("form")!);

    await waitFor(() =>
      expect(
        vi.mocked(client.call).mock.calls.some(([command]) => command === "save_profile"),
      ).toBe(true),
    );
    const save = vi.mocked(client.call).mock.calls.find(([command]) => command === "save_profile");
    expect(save?.[1]).toMatchObject({
      profile: { snowflake: { authentication: expected } },
    });
  });
}

test("external-browser Snowflake sessions skip automatic catalog requests", async () => {
  const snowflake = snowflakeProfile("externalBrowser");
  const session = {
    id: "snowflake-sdk-session",
    connectionId: snowflake.id,
    engine: "snowflake",
    serverVersion: "fixture",
    latencyMs: 0,
    encrypted: true,
    capabilities: {
      sql: true,
      transactions: false,
      cancellation: false,
      tableBrowsing: false,
      tableEditing: false,
      tableFiltering: false,
      processes: false,
    },
  } as SessionInfo;
  const onClose = vi.fn();
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "prepare_connection") return "snowflake-attempt";
    if (command === "connect_prepared") return session;
    return null;
  });
  render(<ConnectDialog profile={snowflake} onClose={onClose} />);
  fireEvent.submit(screen.getByRole("button", { name: "Connect" }).closest("form")!);

  await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
  expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).not.toContain("catalog");
});

test("submitting deletion confirmation deletes instead of saving or connecting", async () => {
  vi.mocked(client.call).mockRejectedValue(new Error("Fixture error"));
  render(<ConnectionDialog profile={profile} onClose={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Delete connection" }));
  expect(client.call).not.toHaveBeenCalled();
  const confirm = screen.getByRole("button", { name: "Remove saved connection" });
  fireEvent.submit(confirm.closest("form")!);
  await waitFor(() =>
    expect(client.call).toHaveBeenCalledWith("delete_profile", {
      id: profile.id,
    }),
  );
  expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).toEqual(["delete_profile"]);
});

test("cancelling confirmation preserves unsaved edits and never calls deletion", () => {
  render(<ConnectionDialog profile={profile} onClose={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("Connection name", { exact: true }), {
    target: { value: "Unsaved name" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Delete connection" }));
  expect(screen.getByRole("dialog", { name: "Delete connection?" })).toBeTruthy();
  expect(screen.getByText(profile.name, { exact: true })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(
    (screen.getByLabelText("Connection name", { exact: true }) as HTMLInputElement).value,
  ).toBe("Unsaved name");
  expect(client.call).not.toHaveBeenCalled();
});

test("production deletion needs only confirmation and preserves the profile on failure", async () => {
  const remove = vi.spyOn(useStudio.getState(), "removeConnection").mockImplementation(() => {});
  const onClose = vi.fn();
  vi.mocked(client.call).mockRejectedValueOnce(new Error("Unlock the credential store"));
  render(
    <ConnectionDialog profile={{ ...profile, environment: "production" }} onClose={onClose} />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Delete connection" }));
  expect(screen.queryByLabelText("Application write passphrase")).toBeNull();
  expect(screen.queryByRole("button", { name: "Save and connect" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Remove saved connection" }));
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Unlock the credential store",
  );
  expect(remove).not.toHaveBeenCalled();
  expect(onClose).not.toHaveBeenCalled();
  vi.mocked(client.call).mockResolvedValueOnce(null);
  fireEvent.click(screen.getByRole("button", { name: "Remove saved connection" }));
  await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
  expect(remove).toHaveBeenCalledExactlyOnceWith(profile.id);
  expect(client.call).toHaveBeenLastCalledWith("delete_profile", { id: profile.id });
});

for (const mode of ["save", "connect"] as const)
  test(`${mode}: SSO success closes the dialog without waiting for table discovery`, async () => {
    const onClose = vi.fn();
    const setSession = vi.spyOn(useStudio.getState(), "setSession");
    const notify = vi.spyOn(useStudio.getState(), "notify").mockImplementation(() => {});
    let rejectCatalog!: (error: Error) => void;
    vi.spyOn(useStudio.getState(), "refreshCatalog").mockImplementation(
      () =>
        new Promise((_, reject) => {
          rejectCatalog = reject;
        }),
    );
    const session = { id: "sso-session", connectionId: profile.id } as SessionInfo;
    vi.mocked(client.call).mockImplementation(async (command) =>
      command === "save_profile" ? profile : session,
    );
    render(
      mode === "save" ? (
        <ConnectionDialog profile={profile} onClose={onClose} />
      ) : (
        <ConnectDialog profile={profile} onClose={onClose} />
      ),
    );
    const button = screen.getByRole("button", {
      name: mode === "save" ? "Save and connect" : "Connect",
    });
    fireEvent.submit(button.closest("form")!);
    await waitFor(() => expect(setSession).toHaveBeenCalledWith(session));
    expect(onClose).toHaveBeenCalledOnce();
    rejectCatalog(new Error("Catalog unavailable"));
    await waitFor(() =>
      expect(notify).toHaveBeenCalledWith(expect.stringContaining("Catalog unavailable")),
    );
    expect(screen.queryByRole("alert")).toBeNull();
  });

test("table discovery cannot report a disconnected session as connected", async () => {
  let rejectCatalog!: (error: Error) => void;
  vi.spyOn(useStudio.getState(), "refreshCatalog").mockImplementation(
    () =>
      new Promise((_, reject) => {
        rejectCatalog = reject;
      }),
  );
  const notify = vi.spyOn(useStudio.getState(), "notify").mockImplementation(() => {});
  vi.mocked(client.call).mockResolvedValue({
    id: "sso-session",
    connectionId: profile.id,
  } as SessionInfo);
  const onClose = vi.fn();
  render(<ConnectDialog profile={profile} onClose={onClose} />);
  fireEvent.submit(screen.getByRole("button", { name: "Connect" }).closest("form")!);
  await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
  useStudio.getState().disconnected(profile.id);
  rejectCatalog(new Error("Late catalog error"));
  await Promise.resolve();
  expect(notify).toHaveBeenCalledExactlyOnceWith(`Connected to ${profile.name}`);
});

test("a newly saved production profile can authorize changed settings after a failed connection", async () => {
  useStudio.setState({
    bootstrap: {
      profiles: [],
      groups: [{ id: "production", name: "Production", environment: "production", parentId: null }],
      supportedEngines: ["postgres", "snowflake"],
      passphraseConfigured: false,
      resultBudgetBytes: 64 * 1024 * 1024,
      rowLimit: 100000,
      savedQueries: [],
      bookmarks: [],
      sqlFormatSettings: {
        keywordCase: "upper",
        indentWidth: 2,
        linesBetweenQueries: 1,
      },
      sqlSnippets: [],
      workspace: null,
    },
  });
  vi.mocked(client.call).mockImplementation(async (command, args) => {
    if (command === "save_profile") return args?.profile;
    throw new Error("Sign-in was not completed");
  });
  const onClose = vi.fn();
  render(<ConnectionDialog groupId="production" onClose={onClose} />);
  expect(screen.queryByLabelText("Application write passphrase")).toBeNull();
  fireEvent.change(screen.getByLabelText("Connection name", { exact: true }), {
    target: { value: "Production fixture" },
  });
  fireEvent.submit(screen.getByRole("button", { name: "Save and connect" }).closest("form")!);
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Sign-in was not completed",
  );
  expect(screen.getByRole("dialog", { name: "Edit connection" })).toBeTruthy();
  expect(screen.getByLabelText("Application write passphrase")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Delete connection" })).toBeTruthy();
  expect(onClose).not.toHaveBeenCalled();
});

test("reviewed Snowflake DDL is independent and revoked by ReadOnly and OAuth transitions", () => {
  enableSnowflakeEngine();
  render(<ConnectionDialog onClose={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("Database engine"), { target: { value: "snowflake" } });
  fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "confirm" } });
  const ddl = /Allow separately reviewed CREATE TABLE, nullable ADD COLUMN, and DROP TABLE/i;
  const dml = /Allow separately reviewed single INSERT, UPDATE, and DELETE statements/i;
  expect((screen.getByLabelText(ddl) as HTMLInputElement).checked).toBe(false);
  fireEvent.click(screen.getByLabelText(dml));
  expect((screen.getByLabelText(ddl) as HTMLInputElement).checked).toBe(false);
  fireEvent.click(screen.getByLabelText(ddl));
  fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "readOnly" } });
  expect(screen.queryByLabelText(ddl)).toBeNull();
  fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "confirm" } });
  expect((screen.getByLabelText(ddl) as HTMLInputElement).checked).toBe(false);
  fireEvent.click(screen.getByLabelText(ddl));
  fireEvent.change(screen.getByLabelText("Snowflake authentication"), {
    target: { value: "localApplicationOauth" },
  });
  fireEvent.change(screen.getByLabelText("Snowflake authentication"), {
    target: { value: "externalBrowser" },
  });
  fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "confirm" } });
  expect((screen.getByLabelText(ddl) as HTMLInputElement).checked).toBe(false);
});

for (const reset of ["readOnly", "oauth"] as const) {
  test(`reviewed console consent is independent, defaults off, and clears on ${reset}`, () => {
    enableSnowflakeEngine();
    render(
      <ConnectionDialog
        profile={{ ...snowflakeProfile("externalBrowser"), safeMode: "confirm" }}
        onClose={vi.fn()}
      />,
    );
    const consoleOption = screen.getByRole("checkbox", {
      name: "Enable reviewed Snowflake SQL console",
    }) as HTMLInputElement;
    expect(consoleOption.checked).toBe(false);
    fireEvent.click(consoleOption);
    expect(consoleOption.checked).toBe(true);
    expect(
      (screen.getByLabelText(/allow separately reviewed single INSERT/i) as HTMLInputElement)
        .checked,
    ).toBe(false);
    expect(screen.getByText(/fixed 65-second per-unit delivery deadline/)).toBeTruthy();
    if (reset === "readOnly")
      fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "readOnly" } });
    else {
      fireEvent.change(screen.getByLabelText("Snowflake authentication"), {
        target: { value: "localApplicationOauth" },
      });
      fireEvent.change(screen.getByLabelText("Snowflake authentication"), {
        target: { value: "externalBrowser" },
      });
    }
    expect(
      screen.queryByRole("checkbox", { name: "Enable reviewed Snowflake SQL console" }),
    ).toBeNull();
    fireEvent.change(screen.getByLabelText("Safe Mode"), { target: { value: "confirm" } });
    expect(
      (
        screen.getByRole("checkbox", {
          name: "Enable reviewed Snowflake SQL console",
        }) as HTMLInputElement
      ).checked,
    ).toBe(false);
  });
}
