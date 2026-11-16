import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectionProfile, LocalHistoryEntry } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import { computeLineDiff } from "./local-diff";
import { LocalHistoryModal, localHistoryTabKey } from "./LocalHistoryModal";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (e: unknown) => String(e instanceof Error ? e.message : e),
}));

const profile: ConnectionProfile = {
  id: "11111111-1111-4111-8111-111111111111",
  name: "Local PG",
  groupId: "local",
  engine: "postgres",
  host: "127.0.0.1",
  port: 5432,
  database: "joinery",
  username: "postgres",
  environment: "local",
  safeMode: "confirm",
  tlsMode: "disable",
  caCertPath: null,
  ssh: null,
  rememberPassword: false,
  storeQueryText: true,
  storeDrafts: true,
  snowflake: null,
};

describe("computeLineDiff and LocalHistoryModal", () => {
  beforeEach(() => {
    vi.mocked(client.call).mockReset();
    useStudio.setState({
      bootstrap: {
        profiles: [profile],
        groups: [],
        passphraseConfigured: false,
        supportedEngines: ["postgres", "mysql", "snowflake", "redis"],
        resultBudgetBytes: 64 * 1024 * 1024,
        rowLimit: 50_000,
        workspace: null,
        savedQueries: [],
        bookmarks: [],
        sqlFormatSettings: { keywordCase: "upper", indentWidth: 2, linesBetweenQueries: 1 },
        sqlSnippets: [],
        virtualForeignKeys: [],
      },
      selectedConnectionId: profile.id,
      tabs: [],
      activeTabId: null,
    });
  });

  afterEach(() => cleanup());

  it("computes line diffs with added, removed, and context lines", () => {
    const diff = computeLineDiff(
      "SELECT id,\n  old_col\nFROM orders;",
      "SELECT id,\n  new_col,\n  created_at\nFROM orders;",
    );
    expect(diff.identical).toBe(false);
    expect(diff.addedCount).toBe(2);
    expect(diff.removedCount).toBe(1);
    expect(diff.lines.some((l) => l.kind === "removed" && l.text.includes("old_col"))).toBe(true);
    expect(diff.lines.some((l) => l.kind === "added" && l.text.includes("created_at"))).toBe(true);
  });

  it("loads revisions, renders diff vs active editor, saves snapshots, and restores SQL", async () => {
    useStudio.getState().openQuery(profile.id, "SELECT id, email\nFROM users\nLIMIT 25;");
    const tab = useStudio.getState().tabs[0]!;
    const key = localHistoryTabKey(tab);

    const initialEntries: LocalHistoryEntry[] = [
      {
        id: "rev-1",
        tabKey: key,
        title: tab.title,
        connectionId: profile.id,
        trigger: "executed",
        sql: "SELECT id\nFROM users\nLIMIT 10;",
        createdAtMs: 1_700_000_000_000,
      },
    ];
    const snapEntry: LocalHistoryEntry = {
      id: "rev-2",
      tabKey: key,
      title: tab.title,
      connectionId: profile.id,
      trigger: "snapshot",
      sql: tab.sql,
      createdAtMs: 1_700_000_005_000,
    };

    vi.mocked(client.call)
      .mockResolvedValueOnce(initialEntries)
      .mockResolvedValueOnce(snapEntry)
      .mockResolvedValueOnce([snapEntry, ...initialEntries]);

    const onClose = vi.fn();
    render(<LocalHistoryModal tab={tab} onClose={onClose} />);

    await waitFor(() => {
      expect(client.call).toHaveBeenCalledWith("local_history_entries", { tabKey: key });
    });

    expect(screen.getByText("+2 / -2 vs active editor")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /Save snapshot/i }));
    await waitFor(() => {
      expect(client.call).toHaveBeenCalledWith("record_local_history", {
        input: {
          tabKey: key,
          title: tab.title,
          connectionId: profile.id,
          trigger: "snapshot",
          sql: tab.sql,
        },
      });
    });

    // Select the older rev-1 entry and restore it into the editor.
    const revButtons = screen.getAllByRole("option");
    fireEvent.click(revButtons[1]!.querySelector("button")!);

    fireEvent.click(screen.getByRole("button", { name: /Restore to editor/i }));
    expect(useStudio.getState().tabs[0]?.sql).toBe("SELECT id\nFROM users\nLIMIT 10;");
    expect(onClose).toHaveBeenCalled();
  });

  it("disables snapshot capture when connection privacy disables both query-text and draft storage", async () => {
    const strictProfile: ConnectionProfile = {
      ...profile,
      id: "99999999-9999-4999-8999-999999999999",
      name: "Strict Prod",
      storeQueryText: false,
      storeDrafts: false,
    };
    useStudio.setState((s) => ({
      bootstrap: s.bootstrap ? { ...s.bootstrap, profiles: [strictProfile] } : null,
    }));
    useStudio.getState().openQuery(strictProfile.id, "SELECT 1;");
    const tab = useStudio.getState().tabs[0]!;
    vi.mocked(client.call).mockResolvedValueOnce([]);

    render(<LocalHistoryModal tab={tab} onClose={vi.fn()} />);
    await waitFor(() => {
      expect(client.call).toHaveBeenCalledWith("local_history_entries", {
        tabKey: localHistoryTabKey(tab),
      });
    });

    expect(
      (screen.getByRole("button", { name: /Save snapshot/i }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(screen.getByText(/disables both query-text and draft storage/i)).toBeTruthy();
  });
});
