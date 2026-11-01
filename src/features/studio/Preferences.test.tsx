import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { client } from "../../ipc/client";
import type { SqlFormatSettings, SqlSnippet, SqlSnippetInput } from "../../ipc/generated";
import { DEFAULT_SQL_FORMAT_SETTINGS } from "../../data/sql-format";
import { useStudio } from "../../state/studio";
import { Preferences } from "./Preferences";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  desktopAvailable: true,
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));

afterEach(cleanup);

beforeEach(() => {
  vi.clearAllMocks();
  useStudio.setState({
    bootstrap: {
      profiles: [],
      groups: [{ id: "local", name: "Local", parentId: null, environment: "local" }],
      passphraseConfigured: false,
      supportedEngines: ["postgres", "mysql", "snowflake", "redis"],
      resultBudgetBytes: 64 * 1024 * 1024,
      rowLimit: 100000,
      workspace: null,
      savedQueries: [],
      bookmarks: [],
      sqlFormatSettings: DEFAULT_SQL_FORMAT_SETTINGS,
      sqlSnippets: [],
    },
    sqlFormatSettings: DEFAULT_SQL_FORMAT_SETTINGS,
    sqlSnippets: [],
  });
});

test("Preferences updates SQL formatting settings and manages custom SQL snippets", async () => {
  vi.mocked(client.call).mockImplementation((async (
    command: string,
    args?: Record<string, unknown>,
  ) => {
    if (command === "save_sql_format_settings") {
      return args?.settings as SqlFormatSettings;
    }
    if (command === "save_sql_snippet") {
      const input = args?.input as SqlSnippetInput;
      const saved: SqlSnippet = {
        id: input.id ?? "snippet-1",
        trigger: input.trigger,
        title: input.title,
        body: input.body,
        engine: input.engine,
        updatedAtMs: 100,
      };
      return saved;
    }
    if (command === "delete_sql_snippet") {
      return null;
    }
    if (command === "metadata_backups") return [];
    throw new Error(`Unexpected command ${command}`);
  }) as typeof client.call);

  render(<Preferences onClose={vi.fn()} />);

  fireEvent.change(screen.getByLabelText("SQL keyword case"), {
    target: { value: "lower" },
  });
  await waitFor(() => {
    expect(useStudio.getState().sqlFormatSettings.keywordCase).toBe("lower");
  });
  expect(client.call).toHaveBeenCalledWith("save_sql_format_settings", {
    settings: {
      keywordCase: "lower",
      indentWidth: 2,
      linesBetweenQueries: 1,
    },
  });

  fireEvent.change(screen.getByLabelText("SQL indent width"), {
    target: { value: "4" },
  });
  await waitFor(() => {
    expect(useStudio.getState().sqlFormatSettings.indentWidth).toBe(4);
  });

  fireEvent.change(screen.getByLabelText("Snippet trigger"), {
    target: { value: "recent_orders" },
  });
  fireEvent.change(screen.getByLabelText("Snippet engine"), {
    target: { value: "postgres" },
  });
  fireEvent.change(screen.getByLabelText("Snippet title"), {
    target: { value: "Recent orders" },
  });
  fireEvent.change(screen.getByLabelText("Snippet template"), {
    target: { value: "SELECT * FROM orders LIMIT ${1:25};" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Add snippet" }));

  await waitFor(() => {
    expect(useStudio.getState().sqlSnippets).toHaveLength(1);
  });
  expect(screen.getByText("recent_orders")).toBeTruthy();
  expect(screen.getByText("Recent orders")).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Edit snippet recent_orders" }));
  fireEvent.change(screen.getByLabelText("Snippet title"), {
    target: { value: "Latest orders" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Update snippet" }));

  await waitFor(() => {
    expect(useStudio.getState().sqlSnippets[0]?.title).toBe("Latest orders");
  });

  fireEvent.click(screen.getByRole("button", { name: "Delete snippet recent_orders" }));
  await waitFor(() => {
    expect(useStudio.getState().sqlSnippets).toHaveLength(0);
  });
});

test("rapid preference changes compose and persist in order, with actionable save errors", async () => {
  const releases: ((value: unknown) => void)[] = [];
  vi.mocked(client.call).mockImplementation((command) =>
    command === "save_sql_format_settings"
      ? new Promise((resolve) => releases.push(resolve))
      : Promise.resolve([]),
  );
  render(<Preferences onClose={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("SQL keyword case"), { target: { value: "lower" } });
  fireEvent.change(screen.getByLabelText("SQL indent width"), { target: { value: "4" } });
  await waitFor(() => expect(releases).toHaveLength(1));
  expect(screen.getByRole("button", { name: "Create metadata backup" }).matches(":disabled")).toBe(
    true,
  );
  expect(useStudio.getState().sqlFormatSettings).toEqual({
    keywordCase: "lower",
    indentWidth: 4,
    linesBetweenQueries: 1,
  });
  releases[0]({ keywordCase: "lower", indentWidth: 2, linesBetweenQueries: 1 });
  await waitFor(() => expect(releases).toHaveLength(2));
  expect(client.call).toHaveBeenLastCalledWith("save_sql_format_settings", {
    settings: { keywordCase: "lower", indentWidth: 4, linesBetweenQueries: 1 },
  });
  releases[1]({ keywordCase: "lower", indentWidth: 4, linesBetweenQueries: 1 });
  await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  expect(screen.getByRole("button", { name: "Create metadata backup" }).matches(":disabled")).toBe(
    false,
  );
  expect(useStudio.getState().bootstrap?.sqlFormatSettings.indentWidth).toBe(4);

  vi.mocked(client.call).mockRejectedValueOnce(new Error("Could not save metadata"));
  fireEvent.change(screen.getByLabelText("SQL keyword case"), { target: { value: "upper" } });
  await screen.findByText("Could not save metadata");
  expect(useStudio.getState().sqlFormatSettings.keywordCase).toBe("lower");
});
