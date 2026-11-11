import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { client } from "../../ipc/client";
import type { ConnectionProfile, PreparedPlan } from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import { SnowflakeDdlDialog } from "./SnowflakeDdlDialog";
vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));
const profile = {
  id: "profile",
  name: "Guarded account",
  database: "DB",
  environment: "local",
  safeMode: "guard",
} as ConnectionProfile;
function plan(drop = false): PreparedPlan {
  return {
    id: "ddl-plan",
    sessionId: "isolated-console",
    reviewedSnowflakeDdl: true,
    statements: [
      {
        sql: drop
          ? 'DROP TABLE "DB"."S"."T" RESTRICT'
          : 'CREATE TABLE "DB"."S"."NEW_TABLE" ("ID" NUMBER(38, 0))',
        kind: "ddl",
        missingWhere: false,
      },
    ],
    requiresApproval: true,
    requiresPassphrase: false,
    confirmationText: "Guarded account",
    targetConfirmationText: drop ? '"DB"."S"."T"' : undefined,
    warnings: ["DDL cannot be rolled back"],
    expiresAt: Date.now() + 60000,
  };
}
beforeEach(() => {
  vi.resetAllMocks();
  useStudio.setState({
    sessionStates: {},
    sessionTransactionPending: {},
    sessionMetadataRefreshRequired: {},
    sessionDdlCatalogRefreshRequired: {},
    sessionContextSwitching: {},
    sessionContextAuthorities: {},
  });
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "preflight_snowflake_ddl")
      return { revision: "grant", sql: plan().statements[0].sql };
    if (command === "prepare_snowflake_ddl") return plan();
    if (command === "authorize_snowflake_ddl") return "approval";
    throw new Error(`Unexpected ${command}`);
  });
});
afterEach(cleanup);
it("preflights and prepares CREATE on the exact isolated console and preserves quoted name spelling", async () => {
  render(
    <SnowflakeDdlDialog
      sessionId="isolated-console"
      ownerTabId="owner"
      profile={profile}
      request={{ kind: "createTable", database: "DB", schema: "S" }}
      onClose={vi.fn()}
    />,
  );
  fireEvent.change(screen.getByLabelText("Table name"), { target: { value: "Quoted. 雪" } });
  fireEvent.click(screen.getByRole("button", { name: "Generate & review SQL" }));
  await screen.findByRole("dialog", { name: "Review this operation" });
  expect(client.call).toHaveBeenNthCalledWith(1, "preflight_snowflake_ddl", {
    request: {
      sessionId: "isolated-console",
      action: {
        kind: "createTable",
        database: "DB",
        schema: "S",
        name: "Quoted. 雪",
        columns: [{ name: "ID", dataType: "NUMBER(38,0)", nullable: true }],
      },
    },
  });
  expect(client.call).toHaveBeenNthCalledWith(2, "prepare_snowflake_ddl", {
    sessionId: "isolated-console",
    request: expect.objectContaining({ revision: "grant" }),
  });
  expect(client.call).not.toHaveBeenCalledWith("prepare_table_object_ddl", expect.anything());
});
it("DROP needs independent exact target and connection confirmations before typed authorization", async () => {
  vi.mocked(client.call).mockImplementation(async (command) =>
    command === "preflight_snowflake_ddl"
      ? { revision: "grant", sql: plan(true).statements[0].sql }
      : command === "prepare_snowflake_ddl"
        ? plan(true)
        : "approval",
  );
  const execute = vi
    .spyOn(useStudio.getState(), "executeSnowflakeDdl")
    .mockResolvedValue({} as never);
  render(
    <SnowflakeDdlDialog
      sessionId="isolated-console"
      ownerTabId="owner"
      profile={profile}
      request={{
        kind: "dropTable",
        browseRevision: "browse",
        targetId: "opaque",
        qualifiedTarget: '"DB"."S"."T"',
      }}
      onClose={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Generate & review SQL" }));
  await screen.findByLabelText("Exact table name to confirm DROP");
  const button = screen.getByRole("button", { name: "Authorize and run" });
  expect((button as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(screen.getByLabelText(/Type Guarded account/), {
    target: { value: "Guarded account" },
  });
  expect((button as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(screen.getByLabelText("Exact table name to confirm DROP"), {
    target: { value: '"DB"."S"."T"' },
  });
  fireEvent.click(button);
  await waitFor(() =>
    expect(execute).toHaveBeenCalledWith(
      expect.objectContaining({ id: "ddl-plan", reviewedSnowflakeDdl: true }),
      "approval",
      "owner",
    ),
  );
  expect(client.call).toHaveBeenLastCalledWith("authorize_snowflake_ddl", {
    planId: "ddl-plan",
    confirmation: "Guarded account",
    passphrase: null,
    targetConfirmation: '"DB"."S"."T"',
  });
  execute.mockRestore();
});
it("unmount during authorization never dispatches a late approved DDL", async () => {
  let authorize!: (value: string) => void;
  vi.mocked(client.call).mockImplementation(async (command) =>
    command === "preflight_snowflake_ddl"
      ? { revision: "grant", sql: plan().statements[0].sql }
      : command === "prepare_snowflake_ddl"
        ? plan()
        : new Promise<string>((resolve) => {
            authorize = resolve;
          }),
  );
  const execute = vi.spyOn(useStudio.getState(), "executeSnowflakeDdl");
  const view = render(
    <SnowflakeDdlDialog
      sessionId="isolated-console"
      profile={profile}
      request={{ kind: "createTable", database: "DB", schema: "S" }}
      onClose={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Generate & review SQL" }));
  await screen.findByRole("dialog", { name: "Review this operation" });
  fireEvent.change(screen.getByLabelText(/Type Guarded account/), {
    target: { value: "Guarded account" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Authorize and run" }));
  await waitFor(() => expect(authorize).toBeDefined());
  view.unmount();
  await act(async () => {
    authorize("late");
  });
  expect(execute).not.toHaveBeenCalled();
  execute.mockRestore();
});
