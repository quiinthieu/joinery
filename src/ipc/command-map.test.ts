import { describe, expect, expectTypeOf, it, vi } from "vitest";
import type { StudioClient } from "./client";
import type {
  JoineryCommandMap,
  JoineryCommandName,
  NoArgCommandName,
  RequiredArgCommandName,
} from "./command-map";
import type {
  Bootstrap,
  PreparedPlan,
  RelationDefinition,
  SchemaDdlPlan,
  TableSchema,
} from "./generated";

describe("JoineryCommandMap closed typed IPC contract (I03)", () => {
  it("infers exact return types and argument shapes for zero-arg and required-arg commands", async () => {
    const callMock = vi.fn().mockResolvedValue(undefined);
    const typedClient: StudioClient = {
      call: callMock as unknown as StudioClient["call"],
      execute: vi.fn(),
      exportData: vi.fn(),
      range: vi.fn(),
    };

    const bootstrapPromise = typedClient.call("bootstrap");
    expectTypeOf(bootstrapPromise).toEqualTypeOf<Promise<Bootstrap>>();

    const schemaPromise = typedClient.call("table_schema", {
      sessionId: "session-1",
      table: { schema: "public", name: "orders" },
    });
    expectTypeOf(schemaPromise).toEqualTypeOf<Promise<TableSchema>>();

    const definitionPromise = typedClient.call("define_relation", {
      sessionId: "session-1",
      request: { revision: "catalog-1", targetId: "opaque-target" },
    });
    expectTypeOf(definitionPromise).toEqualTypeOf<Promise<RelationDefinition>>();

    const explicitGenericPromise = typedClient.call<TableSchema>("table_schema", {
      sessionId: "session-1",
      table: { schema: "public", name: "orders" },
    });
    expectTypeOf(explicitGenericPromise).toEqualTypeOf<Promise<TableSchema>>();

    const migrationPrepPromise = typedClient.call("prepare_migration_ddl", {
      sessionId: "session-1",
      expectedSnapshot: {
        engine: "postgres",
        serverVersion: "17.2",
        database: "joinery",
        schema: "public",
        role: "joinery",
        tables: [],
        routines: [],
        sequences: [],
        unsupportedObjects: [],
        truncated: false,
        restricted: false,
      },
      statements: [
        {
          sql: 'DROP TABLE "public"."legacy";',
          description: "Drop table legacy",
          objectKind: "table",
          objectName: "legacy",
          destructive: true,
        },
      ],
    });
    expectTypeOf(migrationPrepPromise).toEqualTypeOf<Promise<PreparedPlan>>();

    const releasePreviewPromise = typedClient.call("release_csv_import_preview", {
      sessionId: "session-1",
      sourceToken: "source-1",
      previewToken: "preview-1",
    });
    expectTypeOf(releasePreviewPromise).toEqualTypeOf<Promise<void>>();

    await Promise.all([
      bootstrapPromise,
      definitionPromise,
      schemaPromise,
      explicitGenericPromise,
      migrationPrepPromise,
      releasePreviewPromise,
    ]);
    expect(callMock).toHaveBeenCalledTimes(6);
  });

  it("rejects unknown command names and invalid argument payloads at compile time", () => {
    const verifyCompileTimeConstraints = (c: StudioClient) => {
      // @ts-expect-error — unknown command name is rejected
      void c.call("non_existent_command");

      // @ts-expect-error — unknown command name with explicit type arg is also rejected
      void c.call<unknown>("non_existent_command", {});

      // @ts-expect-error — missing required arguments for table_schema
      void c.call("table_schema");

      void c.call("define_relation", {
        sessionId: "session-1",
        // @ts-expect-error — definitions resolve only retained targets, never caller SQL
        request: { revision: "catalog-1", targetId: "opaque-target", sql: "SELECT 1" },
      });
      void c.call("define_relation", {
        sessionId: "session-1",
        // @ts-expect-error — a retained target requires its catalog revision
        request: { targetId: "opaque-target" },
      });

      // @ts-expect-error — wrong argument shape for connect
      void c.call("connect", { id: 123 });

      void c.call("prepare_transaction_control", {
        sessionId: "session-1",
        request: { action: "rollback", revision: "one-use" },
      });
      void c.call("prepare_transaction_control", {
        sessionId: "session-1",
        // @ts-expect-error — explicit controls cannot carry arbitrary SQL
        request: { action: "begin", revision: "one-use", sql: "DROP TABLE x" },
      });
      void c.call("prepare_transaction_control", {
        sessionId: "session-1",
        // @ts-expect-error — action is a closed typed enumeration
        request: { action: "autocommit", revision: "one-use" },
      });
      // @ts-expect-error — executing a boundary requires its prepared plan
      void c.call("execute_transaction_control", { sessionId: "session-1", action: "commit" });

      // @ts-expect-error — wrong argument shape for generate_schema_ddl
      void c.call("generate_schema_ddl", { snapshot: "invalid" });

      // @ts-expect-error — preparing a batch requires its exact preview generation
      void c.call("prepare_csv_import_batch", {
        sessionId: "session-1",
        sourceToken: "source-1",
        table: { schema: "public", name: "orders" },
        options: { delimiter: "comma", hasHeader: true, nullToken: "\\N" },
        mappings: [],
        batchIndex: 0,
      });

      // @ts-expect-error — releasing a preview requires its exact generation
      void c.call("release_csv_import_preview", {
        sessionId: "session-1",
        sourceToken: "source-1",
      });
    };

    expect(typeof verifyCompileTimeConstraints).toBe("function");
    expectTypeOf<"bootstrap">().toExtend<NoArgCommandName>();
    expectTypeOf<"table_schema">().toExtend<RequiredArgCommandName>();
    expectTypeOf<JoineryCommandName>().toEqualTypeOf<keyof JoineryCommandMap>();
  });
});
