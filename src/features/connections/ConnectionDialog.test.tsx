import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ConnectionDialog, ConnectDialog } from "./ConnectionDialog";
import { client } from "../../ipc/client";
import type { ConnectionProfile, SessionInfo } from "../../ipc/generated";
import { useStudio } from "../../state/studio";

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

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.resetAllMocks();
  useStudio.setState({ ...initialStudio }, true);
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
