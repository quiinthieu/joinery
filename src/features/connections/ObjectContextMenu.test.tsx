import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ObjectContextMenu } from "./ObjectContextMenu";
import type { ConnectionProfile, SessionInfo } from "../../ipc/generated";
import type { ExplorerItem } from "./explorer-tree";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn().mockResolvedValue({}) },
  messageOf: (error: unknown) => (error as Error).message,
}));

const mockProfile: ConnectionProfile = {
  id: "sf-test",
  name: "Snowflake Test",
  groupId: "local",
  engine: "snowflake",
  host: "test.snowflakecomputing.com",
  port: 443,
  database: "PROD",
  username: "ADMIN",
  environment: "local",
  safeMode: "confirm",
  tlsMode: "verifyFull",
  caCertPath: null,
  ssh: null,
  rememberPassword: false,
  storeQueryText: true,
  storeDrafts: true,
  snowflake: null,
};

const mockSession: SessionInfo = {
  id: "session-sf",
  connectionId: "sf-test",
  engine: "snowflake",
  serverVersion: "8.42.0",
  latencyMs: 5,
  encrypted: true,
  capabilities: {
    sql: true,
    transactions: false,
    cancellation: true,
    tableBrowsing: true,
    tableEditing: false,
    tableFiltering: true,
    processes: false,
    objectExplorer: true,
  },
};

const tableItem: ExplorerItem = {
  key: "obj:PUBLIC:table:LINEITEM",
  object: { schema: "PUBLIC", name: "LINEITEM", kind: "table", identity: null },
  category: "tables",
  primaryLabel: "LINEITEM",
  secondaryLabel: null,
  tooltip: "PUBLIC.LINEITEM",
  ownerTable: null,
  signature: null,
  estimatedRows: 5000,
  totalBytes: 250000,
};

const stageItem: ExplorerItem = {
  key: "obj:PUBLIC:stage:S3_STAGE",
  object: { schema: "PUBLIC", name: "S3_STAGE", kind: "stage", identity: null },
  category: "stages",
  primaryLabel: "S3_STAGE",
  secondaryLabel: "stage",
  tooltip: "PUBLIC.S3_STAGE",
  signature: null,
  ownerTable: null,
  estimatedRows: null,
  totalBytes: null,
};

const taskItem: ExplorerItem = {
  key: "obj:PUBLIC:task:AGG_TASK",
  object: { schema: "PUBLIC", name: "AGG_TASK", kind: "task", identity: null },
  category: "tasks",
  primaryLabel: "AGG_TASK",
  secondaryLabel: "task",
  tooltip: "PUBLIC.AGG_TASK",
  signature: "()",
  ownerTable: null,
  estimatedRows: null,
  totalBytes: null,
};

const pipeItem: ExplorerItem = {
  key: "obj:PUBLIC:pipe:EVENTS_PIPE",
  object: { schema: "PUBLIC", name: "EVENTS_PIPE", kind: "pipe", identity: null },
  category: "pipes",
  primaryLabel: "EVENTS_PIPE",
  secondaryLabel: "pipe",
  tooltip: "PUBLIC.EVENTS_PIPE",
  signature: null,
  ownerTable: null,
  estimatedRows: null,
  totalBytes: null,
};

describe("ObjectContextMenu", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders table actions and copies name to clipboard", () => {
    const onClose = vi.fn();
    render(
      <ObjectContextMenu
        x={100}
        y={100}
        item={tableItem}
        profile={mockProfile}
        session={mockSession}
        onClose={onClose}
      />,
    );

    expect(screen.getByRole("menu")).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Select from Table" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Browse data" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Inspect structure" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Copy DDL" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Copy object name" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Copy qualified name" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Truncate table…" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Drop table…" })).toBeTruthy();
  });

  it("disables destructive actions when safeMode is readOnly", () => {
    const readOnlyProfile: ConnectionProfile = {
      ...mockProfile,
      safeMode: "readOnly",
    };
    render(
      <ObjectContextMenu
        x={100}
        y={100}
        item={tableItem}
        profile={readOnlyProfile}
        session={mockSession}
        onClose={vi.fn()}
      />,
    );

    const truncateItem = screen.getByRole("menuitem", {
      name: "Truncate table…",
    }) as HTMLButtonElement;
    expect(truncateItem.disabled).toBe(true);

    const dropItem = screen.getByRole("menuitem", { name: "Drop table…" }) as HTMLButtonElement;
    expect(dropItem.disabled).toBe(true);
  });

  it("renders Snowflake specific action for stages (LIST @stage)", () => {
    render(
      <ObjectContextMenu
        x={100}
        y={100}
        item={stageItem}
        profile={mockProfile}
        session={mockSession}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByRole("menuitem", { name: "List stage files" })).toBeTruthy();
  });

  it("renders Snowflake specific action for tasks (TASK_HISTORY())", () => {
    render(
      <ObjectContextMenu
        x={100}
        y={100}
        item={taskItem}
        profile={mockProfile}
        session={mockSession}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByRole("menuitem", { name: "View task history" })).toBeTruthy();
  });

  it("renders Snowflake specific action for pipes (PIPE_STATUS())", () => {
    render(
      <ObjectContextMenu
        x={100}
        y={100}
        item={pipeItem}
        profile={mockProfile}
        session={mockSession}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByRole("menuitem", { name: "Check pipe status" })).toBeTruthy();
  });

  it("supports keyboard navigation: Escape closes menu", () => {
    const onClose = vi.fn();
    render(
      <ObjectContextMenu
        x={100}
        y={100}
        item={tableItem}
        profile={mockProfile}
        session={mockSession}
        onClose={onClose}
      />,
    );

    const menu = screen.getByRole("menu");
    fireEvent.keyDown(menu, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
  });
});
