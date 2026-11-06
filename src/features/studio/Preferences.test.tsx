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
      scratchpads: [],
    },
    sqlFormatSettings: DEFAULT_SQL_FORMAT_SETTINGS,
    sqlSnippets: [],
    shortcuts: {},
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
        place: input.place,
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
  fireEvent.change(screen.getByLabelText("Snippet completion site"), {
    target: { value: "statement" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Add snippet" }));

  await waitFor(() => {
    expect(useStudio.getState().sqlSnippets).toHaveLength(1);
  });
  expect(screen.getByText("recent_orders")).toBeTruthy();
  expect(screen.getByText("Recent orders")).toBeTruthy();
  expect(client.call).toHaveBeenCalledWith("save_sql_snippet", {
    input: {
      id: null,
      trigger: "recent_orders",
      title: "Recent orders",
      body: "SELECT * FROM orders LIMIT ${1:25};",
      engine: "postgres",
      place: "statement",
    },
  });

  fireEvent.click(screen.getByRole("button", { name: "Edit snippet recent_orders" }));
  expect((screen.getByLabelText("Snippet completion site") as HTMLSelectElement).value).toBe(
    "statement",
  );
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

test("engine styles inherit, stay independent, persist across scope switches and reset", async () => {
  vi.mocked(client.call).mockImplementation((command, args) =>
    Promise.resolve(command === "save_sql_format_settings" ? args?.settings : []),
  );
  render(<Preferences onClose={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("Formatting style"), { target: { value: "postgres" } });
  expect(screen.queryByRole("button", { name: "Use global defaults" })).toBeNull();
  fireEvent.change(screen.getByLabelText("SQL keyword case"), { target: { value: "lower" } });
  fireEvent.change(screen.getByLabelText("SQL indent width"), { target: { value: "4" } });
  fireEvent.change(screen.getByLabelText("Formatting style"), { target: { value: "mysql" } });
  expect((screen.getByLabelText("SQL keyword case") as HTMLSelectElement).value).toBe("upper");
  fireEvent.change(screen.getByLabelText("SQL keyword case"), { target: { value: "preserve" } });
  fireEvent.change(screen.getByLabelText("Formatting style"), { target: { value: "global" } });
  fireEvent.change(screen.getByLabelText("Lines between SQL queries"), { target: { value: "2" } });
  await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  expect(useStudio.getState().sqlFormatSettings).toEqual({
    ...DEFAULT_SQL_FORMAT_SETTINGS,
    linesBetweenQueries: 2,
    dialects: {
      postgres: { keywordCase: "lower", indentWidth: 4, linesBetweenQueries: 1 },
      mysql: { keywordCase: "preserve", indentWidth: 2, linesBetweenQueries: 1 },
    },
  });
  fireEvent.change(screen.getByLabelText("Formatting style"), { target: { value: "postgres" } });
  expect((screen.getByLabelText("Lines between SQL queries") as HTMLSelectElement).value).toBe("1");
  vi.mocked(client.call).mockRejectedValueOnce(new Error("Style reset could not be saved"));
  fireEvent.click(screen.getByRole("button", { name: "Use global defaults" }));
  await screen.findByText("Style reset could not be saved");
  expect((screen.getByLabelText("SQL keyword case") as HTMLSelectElement).value).toBe("lower");
  fireEvent.click(screen.getByRole("button", { name: "Use global defaults" }));
  await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  expect((screen.getByLabelText("Lines between SQL queries") as HTMLSelectElement).value).toBe("2");
  expect(useStudio.getState().sqlFormatSettings.dialects?.postgres).toBeUndefined();
  expect(useStudio.getState().sqlFormatSettings.dialects?.mysql?.keywordCase).toBe("preserve");
  fireEvent.change(screen.getByLabelText("Formatting style"), { target: { value: "mysql" } });
  fireEvent.click(screen.getByRole("button", { name: "Use global defaults" }));
  await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  expect(useStudio.getState().sqlFormatSettings.dialects).toBeUndefined();
});

test("edits keyboard shortcuts, flags conflicts and reserved chords, and resets bindings", () => {
  vi.mocked(client.call).mockResolvedValue([]);
  render(<Preferences onClose={vi.fn()} />);

  const paletteInput = screen.getByLabelText(
    "Shortcut for Open command palette",
  ) as HTMLInputElement;
  expect(paletteInput.value).toBe("Mod+K");

  // Rebind Open command palette to Mod+J
  fireEvent.change(paletteInput, { target: { value: "Mod+J" } });
  fireEvent.keyDown(paletteInput, { key: "Enter" });
  expect(useStudio.getState().shortcuts.openPalette).toBe("Mod+J");
  expect(screen.getByLabelText("Current badge for Open command palette").textContent).toBe("⌘ J");

  // Attempt to bind New query tab to Mod+J (conflict with Open command palette)
  const newQueryInput = screen.getByLabelText("Shortcut for New query tab") as HTMLInputElement;
  fireEvent.change(newQueryInput, { target: { value: "Mod+J" } });
  fireEvent.keyDown(newQueryInput, { key: "Enter" });
  expect(screen.getByText('⌘ J is already assigned to "Open command palette".')).toBeTruthy();
  expect(useStudio.getState().shortcuts.newQuery).toBeUndefined();

  // Attempt to bind to reserved Mod+C
  fireEvent.change(newQueryInput, { target: { value: "Mod+C" } });
  fireEvent.keyDown(newQueryInput, { key: "Enter" });
  expect(screen.getByText(/reserved for text and grid copying/i)).toBeTruthy();

  // Reset all shortcuts
  fireEvent.click(screen.getByRole("button", { name: "Reset all shortcuts" }));
  expect(useStudio.getState().shortcuts).toEqual({});
  expect(screen.getByLabelText("Current badge for Open command palette").textContent).toBe("⌘ K");
});

test("Preferences switches appearance mode and color palette presets", () => {
  vi.mocked(client.call).mockResolvedValue([]);
  useStudio.setState({ theme: "dark", palette: "slate" });
  render(<Preferences onClose={vi.fn()} />);

  fireEvent.click(screen.getByRole("button", { name: "Paper light" }));
  expect(useStudio.getState().theme).toBe("light");
  expect(document.documentElement.dataset.theme).toBe("light");

  fireEvent.click(screen.getByRole("button", { name: /Obsidian & Indigo/i }));
  expect(useStudio.getState().palette).toBe("obsidian");
  expect(document.documentElement.dataset.palette).toBe("obsidian");

  fireEvent.click(screen.getByRole("button", { name: /Graphite & Amber/i }));
  expect(useStudio.getState().palette).toBe("graphite");
  expect(document.documentElement.dataset.palette).toBe("graphite");

  fireEvent.click(screen.getByRole("button", { name: /Forest & Sage/i }));
  expect(useStudio.getState().palette).toBe("sage");
  expect(document.documentElement.dataset.palette).toBe("sage");

  fireEvent.click(screen.getByRole("button", { name: /Slate & Electric Blue/i }));
  expect(useStudio.getState().palette).toBe("slate");
  expect(document.documentElement.dataset.palette).toBe("slate");
});
