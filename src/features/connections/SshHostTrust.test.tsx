import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { useSshTrust } from "./SshHostTrust";
import { client } from "../../ipc/client";
import type { ConnectionProfile } from "../../ipc/generated";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (error: unknown) => String(error),
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
function Harness() {
  const trust = useSshTrust(),
    [result, setResult] = useState("");
  return (
    <>
      {trust.dialog ?? (
        <button
          onClick={() =>
            void trust
              .verify({ id: "fixture", ssh: {} } as ConnectionProfile)
              .then(() => setResult("verified"))
              .catch(() => setResult("blocked"))
          }
        >
          Verify
        </button>
      )}
      <output>{result}</output>
    </>
  );
}
test("an unknown host needs explicit approval of the displayed fingerprint", async () => {
  vi.mocked(client.call)
    .mockResolvedValueOnce({
      host: "bastion.test",
      port: 22,
      fingerprint: "SHA256:fixture",
      previousFingerprint: null,
      challengeId: "challenge",
    })
    .mockResolvedValueOnce(null);
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: /^Verify$/ }));
  expect(await screen.findByText("SHA256:fixture")).toBeTruthy();
  expect(client.call).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button", { name: "Trust this host key" }));
  expect(await screen.findByText("verified")).toBeTruthy();
  expect(client.call).toHaveBeenLastCalledWith("trust_ssh_key", {
    challengeId: "challenge",
    fingerprint: "SHA256:fixture",
  });
});
test("a changed host is blocked without offering a trust override", async () => {
  vi.mocked(client.call).mockResolvedValueOnce({
    host: "bastion.test",
    port: 22,
    fingerprint: "SHA256:new",
    previousFingerprint: "SHA256:known",
    challengeId: null,
  });
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: /^Verify$/ }));
  expect(await screen.findByText("blocked")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Trust this host key" })).toBeNull();
  expect(client.call).toHaveBeenCalledTimes(1);
});
