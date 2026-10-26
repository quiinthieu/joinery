import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ConnectionDialog } from "./ConnectionDialog";
import { client } from "../../ipc/client";
import type { ConnectionProfile } from "../../ipc/generated";
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

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.resetAllMocks();
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
