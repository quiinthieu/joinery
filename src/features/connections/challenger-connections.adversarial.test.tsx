import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ObjectContextMenu } from "./ObjectContextMenu";
import {
  categoryLabel,
  categoryMatchesFilter,
  objectKindCategory,
  EXPLORER_CATEGORY_ORDER,
  EXPLORER_FILTER_CHIPS,
  type ExplorerItem,
} from "./explorer-tree";
import { buildSearchEverywhereResults } from "../studio/search-everywhere";
import type { ConnectionProfile, DatabaseObjectSummary, SessionInfo } from "../../ipc/generated";

afterEach(cleanup);

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn().mockResolvedValue({}) },
  messageOf: (error: unknown) => (error as Error).message,
}));

const mockSession: SessionInfo = {
  id: "sf-sess-1",
  connectionId: "sf-conn-1",
  engine: "snowflake",
  serverVersion: "8.0",
  latencyMs: 10,
  encrypted: true,
  capabilities: {
    sql: true,
    transactions: true,
    cancellation: true,
    tableBrowsing: true,
    tableEditing: false,
    tableFiltering: true,
    processes: true,
  },
};

const mockProfile: ConnectionProfile = {
  id: "sf-conn-1",
  name: "Snowflake Analytics",
  groupId: "local",
  engine: "snowflake",
  host: "test.snowflakecomputing.com",
  port: 443,
  database: "DB",
  username: "ADMIN",
  environment: "production",
  safeMode: "confirm",
  tlsMode: "verifyFull",
  caCertPath: null,
  ssh: null,
  rememberPassword: false,
  storeQueryText: true,
  storeDrafts: true,
  snowflake: null,
};

const readOnlyProfile: ConnectionProfile = {
  ...mockProfile,
  safeMode: "readOnly",
};

describe("Milestone 6 Phase 2 Challenger: Connections & Object Tooling Adversarial Suite", () => {
  describe("1. explorer-tree.ts Extended Kinds Categorization & Filter Chips", () => {
    it("maps extended kinds (stages, tasks, streams, pipes, fileFormats) to designated categories", () => {
      expect(objectKindCategory("stage")).toBe("stages");
      expect(objectKindCategory("task")).toBe("tasks");
      expect(objectKindCategory("stream")).toBe("streams");
      expect(objectKindCategory("pipe")).toBe("pipes");
      expect(objectKindCategory("fileFormat")).toBe("fileFormats");

      expect(categoryLabel("stages")).toBe("Stages");
      expect(categoryLabel("tasks")).toBe("Tasks");
      expect(categoryLabel("streams")).toBe("Streams");
      expect(categoryLabel("pipes")).toBe("Pipes");
      expect(categoryLabel("fileFormats")).toBe("File formats");
    });

    it("evaluates categoryMatchesFilter for dedicated chips and 'other' fallback", () => {
      // Dedicated chips
      expect(categoryMatchesFilter("stages", "stages")).toBe(true);
      expect(categoryMatchesFilter("tasks", "tasks")).toBe(true);
      expect(categoryMatchesFilter("streams", "streams")).toBe(true);
      expect(categoryMatchesFilter("pipes", "pipes")).toBe(true);

      // 'other' chip covers fileFormats, sequences, triggers, indexes
      expect(categoryMatchesFilter("fileFormats", "other")).toBe(true);
      expect(categoryMatchesFilter("sequences", "other")).toBe(true);
      expect(categoryMatchesFilter("triggers", "other")).toBe(true);
      expect(categoryMatchesFilter("indexes", "other")).toBe(true);

      // Negative checks
      expect(categoryMatchesFilter("stages", "tables")).toBe(false);
      expect(categoryMatchesFilter("fileFormats", "stages")).toBe(false);
      expect(categoryMatchesFilter("tasks", "other")).toBe(false);

      // 'all' chip matches all categories
      expect(categoryMatchesFilter("stages", "all")).toBe(true);
      expect(categoryMatchesFilter("fileFormats", "all")).toBe(true);
    });

    it("verifies EXPLORER_CATEGORY_ORDER includes all 12 categories in logical hierarchy", () => {
      expect(EXPLORER_CATEGORY_ORDER).toEqual([
        "tables",
        "views",
        "materializedViews",
        "stages",
        "tasks",
        "streams",
        "pipes",
        "fileFormats",
        "routines",
        "sequences",
        "triggers",
        "indexes",
      ]);
    });
  });

  describe("2. ObjectContextMenu WAI-ARIA, Roving Tabindex, and SafeMode Guards", () => {
    const tableItem: ExplorerItem = {
      key: "obj:PUBLIC:table:ORDERS",
      object: { schema: "PUBLIC", name: "ORDERS", kind: "table", identity: null },
      category: "tables",
      primaryLabel: "ORDERS",
      secondaryLabel: null,
      tooltip: "PUBLIC.ORDERS",
      ownerTable: null,
      signature: null,
      estimatedRows: 1000,
      totalBytes: 50000,
    };

    it("complies with WAI-ARIA role='menu' and role='menuitem' semantics", () => {
      render(
        <ObjectContextMenu
          x={100}
          y={100}
          item={tableItem}
          profile={mockProfile}
          session={mockSession}
          onClose={vi.fn()}
        />,
      );

      const menu = screen.getByRole("menu");
      expect(menu.getAttribute("aria-label")).toBe("ORDERS context actions");

      const menuItems = screen.getAllByRole("menuitem");
      expect(menuItems.length).toBeGreaterThan(0);
      menuItems.forEach((btn) => {
        expect(btn.tagName).toBe("BUTTON");
      });
    });

    it("implements roving tabindex and handles keyboard navigation (ArrowDown, ArrowUp, Home, End, Escape)", () => {
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
      const menuItems = screen.getAllByRole("menuitem");

      // Initial focus: item 0 has tabIndex 0, item 1 has tabIndex -1
      expect(menuItems[0].getAttribute("tabindex")).toBe("0");
      expect(menuItems[1].getAttribute("tabindex")).toBe("-1");

      // ArrowDown moves focus to item 1
      fireEvent.keyDown(menu, { key: "ArrowDown" });
      expect(menuItems[0].getAttribute("tabindex")).toBe("-1");
      expect(menuItems[1].getAttribute("tabindex")).toBe("0");

      // End moves to last item
      fireEvent.keyDown(menu, { key: "End" });
      expect(menuItems[menuItems.length - 1].getAttribute("tabindex")).toBe("0");

      // Home moves back to item 0
      fireEvent.keyDown(menu, { key: "Home" });
      expect(menuItems[0].getAttribute("tabindex")).toBe("0");

      // ArrowUp wraps around to last item
      fireEvent.keyDown(menu, { key: "ArrowUp" });
      expect(menuItems[menuItems.length - 1].getAttribute("tabindex")).toBe("0");

      // Escape triggers onClose
      fireEvent.keyDown(menu, { key: "Escape" });
      expect(onClose).toHaveBeenCalled();
    });

    it("enforces SafeMode readOnly guards on destructive actions (Truncate & Drop)", () => {
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

      // Truncate Table button
      const truncateBtn = screen.getByRole("menuitem", {
        name: /Truncate table…/i,
      }) as HTMLButtonElement;
      expect(truncateBtn.disabled).toBe(true);
      expect(truncateBtn.getAttribute("title")).toBe("Connection is in read-only safe mode");

      // Drop Table button
      const dropBtn = screen.getByRole("menuitem", { name: /Drop table…/i }) as HTMLButtonElement;
      expect(dropBtn.disabled).toBe(true);
      expect(dropBtn.getAttribute("title")).toBe("Connection is in read-only safe mode");

      // Non-destructive actions (Browse data, Inspect structure, Copy DDL) must remain enabled
      const browseBtn = screen.getByRole("menuitem", { name: /Browse data/i }) as HTMLButtonElement;
      expect(browseBtn.disabled).toBe(false);

      const copyDdlBtn = screen.getByRole("menuitem", { name: /Copy DDL/i }) as HTMLButtonElement;
      expect(copyDdlBtn.disabled).toBe(false);
    });
  });

  describe("3. search-everywhere.ts Priority 6 Ranking & Filter Chips", () => {
    it("assigns priority 6 to Snowflake extended objects (stage, task, stream, pipe)", () => {
      const summaries: DatabaseObjectSummary[] = [
        {
          object: { schema: "PUBLIC", name: "RAW_STREAM", kind: "stream", identity: null },
          signature: null,
          ownerTable: "ORDERS",
          estimatedRows: null,
          totalBytes: null,
        },
        {
          object: { schema: "PUBLIC", name: "CLEAN_TASK", kind: "task", identity: null },
          signature: "()",
          ownerTable: null,
          estimatedRows: null,
          totalBytes: null,
        },
        {
          object: { schema: "PUBLIC", name: "S3_STAGE", kind: "stage", identity: null },
          signature: null,
          ownerTable: null,
          estimatedRows: null,
          totalBytes: null,
        },
        {
          object: { schema: "PUBLIC", name: "AUTO_PIPE", kind: "pipe", identity: null },
          signature: null,
          ownerTable: null,
          estimatedRows: null,
          totalBytes: null,
        },
        {
          object: { schema: "PUBLIC", name: "CSV_FORMAT", kind: "fileFormat", identity: null },
          signature: null,
          ownerTable: null,
          estimatedRows: null,
          totalBytes: null,
        },
      ];

      const res = buildSearchEverywhereResults({
        query: "PUBLIC",
        scope: "all",
        commands: [],
        tabs: [],
        activeTabId: null,
        preferredConnectionId: "sf-conn-1",
        profiles: [mockProfile],
        sessions: { "sf-conn-1": mockSession },
        catalog: {},
        objects: { "sf-conn-1": summaries },
        history: [],
      });

      expect(res.items.length).toBeGreaterThanOrEqual(5);

      const streamItem = res.items.find((i) => i.label === "RAW_STREAM");
      expect(streamItem).toBeDefined();
      expect(streamItem?.shortcut).toBe("Stream");

      const taskItem = res.items.find((i) => i.label.startsWith("CLEAN_TASK"));
      expect(taskItem).toBeDefined();
      expect(taskItem?.shortcut).toBe("Task");

      const stageItem = res.items.find((i) => i.label === "S3_STAGE");
      expect(stageItem).toBeDefined();
      expect(stageItem?.shortcut).toBe("Stage");

      const pipeItem = res.items.find((i) => i.label === "AUTO_PIPE");
      expect(pipeItem).toBeDefined();
      expect(pipeItem?.shortcut).toBe("Pipe");

      const fileFormatItem = res.items.find((i) => i.label === "CSV_FORMAT");
      expect(fileFormatItem).toBeDefined();
      expect(fileFormatItem?.shortcut).toBe("File format");
    });
  });
});
