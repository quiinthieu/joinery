import { describe, expect, expectTypeOf, it, vi } from "vitest";
import type { StudioClient } from "./client";
import type {
  JoineryCommandMap,
  JoineryCommandName,
  NoArgCommandName,
  RequiredArgCommandName,
} from "./command-map";
import type { Bootstrap, PreparedPlan, SchemaDdlPlan, TableSchema } from "./generated";

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

    await Promise.all([
      bootstrapPromise,
      schemaPromise,
      explicitGenericPromise,
      migrationPrepPromise,
    ]);
    expect(callMock).toHaveBeenCalledTimes(4);
  });

  it("rejects unknown command names and invalid argument payloads at compile time", () => {
    const verifyCompileTimeConstraints = (c: StudioClient) => {
      // @ts-expect-error — unknown command name is rejected
      void c.call("non_existent_command");

      // @ts-expect-error — unknown command name with explicit type arg is also rejected
      void c.call<unknown>("non_existent_command", {});

      // @ts-expect-error — missing required arguments for table_schema
      void c.call("table_schema");

      // @ts-expect-error — wrong argument shape for connect
      void c.call("connect", { id: 123 });

      // @ts-expect-error — wrong argument shape for generate_schema_ddl
      void c.call("generate_schema_ddl", { snapshot: "invalid" });
    };

    expect(typeof verifyCompileTimeConstraints).toBe("function");
    expectTypeOf<"bootstrap">().toExtend<NoArgCommandName>();
    expectTypeOf<"table_schema">().toExtend<RequiredArgCommandName>();
    expectTypeOf<JoineryCommandName>().toEqualTypeOf<keyof JoineryCommandMap>();
  });
});
