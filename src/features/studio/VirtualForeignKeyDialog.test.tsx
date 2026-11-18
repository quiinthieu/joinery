import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectionProfile, VirtualForeignKey } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import { VirtualForeignKeyDialog } from "./VirtualForeignKeyDialog";
import { LibraryModal } from "./LibraryModal";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (e: unknown) => String(e instanceof Error ? e.message : e),
}));

const profile: ConnectionProfile = {
  id: "11111111-1111-4111-8111-111111111111",
  name: "Analytics Warehouse",
  groupId: "local",
  engine: "snowflake",
  host: "acct.snowflakecomputing.com",
  port: 443,
  database: "ANALYTICS",
  username: "analyst",
  environment: "local",
  safeMode: "confirm",
  tlsMode: "verifyFull",
  caCertPath: null,
  ssh: null,
  snowflake: {
    role: "ANALYST",
    warehouse: "WH",
    schema: "PUBLIC",
    authentication: "externalBrowser",
    reviewedDml: false,
    reviewedDdl: false,
    reviewedConsole: false,
  },
  rememberPassword: false,
  storeQueryText: true,
  storeDrafts: true,
};

describe("VirtualForeignKeyDialog and LibraryModal virtual relationships", () => {
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
      virtualForeignKeys: [],
      catalog: {
        [profile.id]: [
          {
            table: { schema: "PUBLIC", name: "ORDERS" },
            kind: "TABLE",
            estimatedRows: 100,
            totalBytes: 1024,
          },
          {
            table: { schema: "PUBLIC", name: "CUSTOMERS" },
            kind: "TABLE",
            estimatedRows: 20,
            totalBytes: 512,
          },
        ],
      },
    });
  });

  afterEach(() => cleanup());

  it("saves composite virtual foreign keys and updates Studio state", async () => {
    const savedVfk: VirtualForeignKey = {
      id: "22222222-2222-4222-8222-222222222222",
      connectionId: profile.id,
      name: "vfk_ORDERS_CUSTOMERS",
      source: { schema: "PUBLIC", name: "ORDERS" },
      columns: ["TENANT_ID", "CUSTOMER_ID"],
      target: { schema: "PUBLIC", name: "CUSTOMERS" },
      targetColumns: ["TENANT_ID", "ID"],
      updatedAtMs: 1000,
    };
    vi.mocked(client.call).mockResolvedValueOnce(savedVfk);
    const onSaved = vi.fn();
    const onClose = vi.fn();

    render(
      <VirtualForeignKeyDialog
        connectionId={profile.id}
        initialSource={{ schema: "PUBLIC", name: "ORDERS" }}
        sourceColumns={["ID", "TENANT_ID", "CUSTOMER_ID"]}
        knownTables={[
          { schema: "PUBLIC", name: "ORDERS" },
          { schema: "PUBLIC", name: "CUSTOMERS" },
        ]}
        onSaved={onSaved}
        onClose={onClose}
      />,
    );

    fireEvent.change(screen.getByLabelText("Source column 1"), {
      target: { value: "TENANT_ID" },
    });
    fireEvent.change(screen.getByLabelText("Referenced column 1"), {
      target: { value: "TENANT_ID" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Add column pair/i }));
    fireEvent.change(screen.getByLabelText("Source column 2"), {
      target: { value: "CUSTOMER_ID" },
    });
    fireEvent.change(screen.getByLabelText("Referenced column 2"), {
      target: { value: "ID" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Save virtual FK" }));

    await waitFor(() => {
      expect(client.call).toHaveBeenCalledWith("save_virtual_foreign_key", {
        input: {
          id: null,
          connectionId: profile.id,
          name: "vfk_ORDERS_CUSTOMERS",
          source: { schema: "PUBLIC", name: "ORDERS" },
          columns: ["TENANT_ID", "CUSTOMER_ID"],
          target: { schema: "PUBLIC", name: "CUSTOMERS" },
          targetColumns: ["TENANT_ID", "ID"],
        },
      });
    });
    expect(useStudio.getState().virtualForeignKeys).toEqual([savedVfk]);
    expect(onSaved).toHaveBeenCalledWith(savedVfk);
    expect(onClose).toHaveBeenCalled();
  });

  it("lists and deletes virtual foreign keys in LibraryModal", async () => {
    const existing: VirtualForeignKey = {
      id: "33333333-3333-4333-8333-333333333333",
      connectionId: profile.id,
      name: "vfk_orders_cust",
      source: { schema: "PUBLIC", name: "ORDERS" },
      columns: ["CUSTOMER_ID"],
      target: { schema: "PUBLIC", name: "CUSTOMERS" },
      targetColumns: ["ID"],
      updatedAtMs: 2000,
    };
    useStudio.setState({ virtualForeignKeys: [existing] });
    vi.mocked(client.call).mockResolvedValueOnce(undefined);

    render(<LibraryModal initialTab="relationships" onNeedsConnect={vi.fn()} onClose={vi.fn()} />);

    expect(screen.getByText("PUBLIC.ORDERS(CUSTOMER_ID) → PUBLIC.CUSTOMERS(ID)")).toBeTruthy();

    fireEvent.click(
      screen.getByRole("button", { name: "Delete virtual foreign key vfk_orders_cust" }),
    );
    await waitFor(() => {
      expect(client.call).toHaveBeenCalledWith("delete_virtual_foreign_key", {
        id: existing.id,
      });
    });
    expect(useStudio.getState().virtualForeignKeys).toHaveLength(0);
  });

  it("reconciles open table tabs immediately when saving and deleting virtual foreign keys", async () => {
    const saved: VirtualForeignKey = {
      id: "44444444-4444-4444-8444-444444444444",
      connectionId: profile.id,
      name: "vfk_orders_cust",
      source: { schema: "PUBLIC", name: "ORDERS" },
      columns: ["CUSTOMER_ID"],
      target: { schema: "PUBLIC", name: "CUSTOMERS" },
      targetColumns: ["ID"],
      updatedAtMs: 3000,
    };
    useStudio.setState({
      tabs: [
        {
          id: "tab-orders",
          kind: "table",
          title: "ORDERS",
          connectionId: profile.id,
          sql: "",
          table: { schema: "PUBLIC", name: "ORDERS" },
          view: "data",
          results: [],
          activeResultId: null,
          pages: [],
          pageKeys: {},
          loadedRows: 0,
          hasMore: false,
          loadingPage: false,
          schema: {
            table: { schema: "PUBLIC", name: "ORDERS" },
            columns: [
              {
                id: "c0",
                name: "ID",
                databaseType: "NUMBER",
                kind: "integer",
                nullable: false,
                primaryKey: true,
                foreignKey: false,
                editable: true,
                insertable: true,
                hasDefault: false,
              },
              {
                id: "c1",
                name: "CUSTOMER_ID",
                databaseType: "NUMBER",
                kind: "integer",
                nullable: true,
                primaryKey: false,
                foreignKey: false,
                editable: true,
                insertable: true,
                hasDefault: false,
              },
            ],
            primaryKey: ["ID"],
            foreignKeys: [],
            inboundForeignKeys: [],
            editable: true,
          },
          nextKey: null,
          query: { sort: [], filters: [], whereClause: null, orderClause: null },
          status: "idle",
          runId: null,
          revision: 1,
          dataSessionId: null,
          error: null,
          elapsedMs: 0,
          metrics: {},
          fileToken: null,
          fileName: null,
          savedSql: null,
          savedQueryId: null,
          object: null,
          querySession: null,
          querySessionClosed: false,
        },
      ],
    });

    vi.mocked(client.call).mockResolvedValueOnce(saved).mockResolvedValueOnce(undefined);
    await useStudio.getState().saveVirtualForeignKey({
      id: null,
      connectionId: profile.id,
      name: saved.name,
      source: saved.source,
      columns: saved.columns,
      target: saved.target,
      targetColumns: saved.targetColumns,
    });

    const afterSave = useStudio.getState().tabs[0]!.schema!;
    expect(afterSave.foreignKeys).toHaveLength(1);
    expect(afterSave.foreignKeys[0]?.virtualRelationship).toBe(true);
    expect(afterSave.columns.find((c) => c.name === "CUSTOMER_ID")?.foreignKey).toBe(true);

    await useStudio.getState().deleteVirtualForeignKey(saved.id);
    const afterDelete = useStudio.getState().tabs[0]!.schema!;
    expect(afterDelete.foreignKeys).toHaveLength(0);
    expect(afterDelete.columns.find((c) => c.name === "CUSTOMER_ID")?.foreignKey).toBe(false);
  });
});
