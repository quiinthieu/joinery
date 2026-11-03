import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { LockWaitSnapshot, ProcessInfo, SessionInfo } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";
import { ProcessesView } from "./UtilityViews";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (e: unknown) => String(e instanceof Error ? e.message : e),
}));

const session: SessionInfo = {
  id: "session-1",
  connectionId: "profile",
  engine: "postgres",
  serverVersion: "fixture",
  latencyMs: 0,
  encrypted: false,
  capabilities: {
    sql: true,
    transactions: true,
    cancellation: true,
    tableBrowsing: true,
    tableEditing: true,
    tableFiltering: true,
    processes: true,
    objectExplorer: true,
  },
};
const process = (id: string, state: string, query: string): ProcessInfo => ({
  id,
  identity: `${id}|start`,
  user: id === "40" ? "batch" : "app",
  database: "joinery",
  state,
  durationSeconds: 3,
  query,
  ownSession: false,
});
const processes = [
  process("10", "idle in transaction", "UPDATE accounts SET v = 1"),
  process("20", "active", "UPDATE accounts SET v = 2"),
  process("30", "active", "UPDATE accounts SET v = 3"),
  process("40", "idle", "SELECT 1"),
];
const locks: LockWaitSnapshot = {
  waits: [
    {
      waitingId: "30",
      user: "app",
      waitSeconds: 4,
      lockType: "transactionid",
      lockMode: "ShareLock",
      object: { schema: "public", name: "accounts" },
      waitEvent: "Lock:transactionid",
      query: "UPDATE accounts SET v = 3",
      blockingIds: ["20"],
      ownSession: false,
    },
    {
      waitingId: "20",
      user: "app",
      waitSeconds: 2,
      lockType: "transactionid",
      lockMode: "ShareLock",
      object: null,
      waitEvent: "Lock:transactionid",
      query: "UPDATE accounts SET v = 2",
      blockingIds: ["10"],
      ownSession: false,
    },
  ],
  truncated: false,
};
const tab = { id: "processes", kind: "processes", connectionId: "profile" } as StudioTab;

function respond(lockResult: LockWaitSnapshot | Error) {
  vi.mocked(client.call).mockImplementation(async (command: string) => {
    if (command === "processes") return processes;
    if (lockResult instanceof Error) throw lockResult;
    return lockResult;
  });
}

beforeEach(() => {
  useStudio.setState({ sessions: { profile: session } });
  vi.mocked(client.call).mockReset();
});
afterEach(() => cleanup());

it("shows lock waits, marks the root blocker and filters processes", async () => {
  respond(locks);
  render(<ProcessesView tab={tab} onCancel={vi.fn()} />);
  const waits = await screen.findByRole("region", { name: "Lock waits" });
  expect(within(waits).getAllByRole("row")).toHaveLength(3);
  expect(within(waits).getByText("public.accounts")).toBeTruthy();
  expect(within(waits).getAllByText("transactionid · ShareLock", { selector: "td" })).toHaveLength(
    2,
  );
  const root = document.querySelector('[data-process="10"]')!;
  expect(root.textContent).toContain("Blocking 2");
  expect(document.querySelector('[data-process="30"]')!.textContent).toContain("Waiting on 20");
  expect(document.querySelector('[data-process="20"]')!.textContent).not.toContain("Blocking");

  fireEvent.click(screen.getByLabelText("Hide idle"));
  expect(document.querySelector('[data-process="40"]')).toBeNull();
  expect(document.querySelector('[data-process="10"]')).not.toBeNull();
  fireEvent.click(screen.getByLabelText("Hide idle"));
  fireEvent.change(screen.getByLabelText("Filter processes"), { target: { value: "batch" } });
  expect(document.querySelectorAll("[data-process]")).toHaveLength(1);
});

it("warns about deadlock cycles and explains missing privileges", async () => {
  respond({
    waits: [
      { ...locks.waits[0], waitingId: "1", blockingIds: ["2"] },
      { ...locks.waits[1], waitingId: "2", blockingIds: ["1"] },
    ],
    truncated: true,
  });
  render(<ProcessesView tab={tab} onCancel={vi.fn()} />);
  expect(await screen.findByText(/Sessions 1, 2 wait on each other/)).toBeTruthy();
  expect(screen.getByText("More sessions are waiting than shown.")).toBeTruthy();
  cleanup();

  respond(new Error("Lock inspection needs the PROCESS privilege."));
  render(<ProcessesView tab={tab} onCancel={vi.fn()} />);
  expect(await screen.findByText("Lock inspection needs the PROCESS privilege.")).toBeTruthy();
  expect(document.querySelectorAll("[data-process]")).toHaveLength(4);
});

it("hides the lock section for engines without lock inspection", async () => {
  respond(new Error("Lock inspection is unavailable for this engine"));
  render(<ProcessesView tab={tab} onCancel={vi.fn()} />);
  await screen.findAllByText("app");
  expect(screen.queryByRole("region", { name: "Lock waits" })).toBeNull();
});

it("skips lock_waits RPC and hides Lock waits section when lockInspection capability is false", async () => {
  useStudio.setState({
    sessions: {
      profile: {
        ...session,
        capabilities: {
          ...session.capabilities,
          lockInspection: false,
        },
      },
    },
  });
  vi.mocked(client.call).mockImplementation(async (command: string) => {
    if (command === "processes") return processes;
    throw new Error(`Unexpected command: ${command}`);
  });

  render(<ProcessesView tab={tab} onCancel={vi.fn()} />);
  await screen.findAllByText("app");
  expect(client.call).toHaveBeenCalledWith("processes", { sessionId: "session-1" });
  expect(client.call).not.toHaveBeenCalledWith("lock_waits", expect.anything());
  expect(screen.queryByRole("region", { name: "Lock waits" })).toBeNull();
});
