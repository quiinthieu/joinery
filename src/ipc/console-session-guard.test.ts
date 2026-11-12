import { expect, test } from "vitest";
import { assertConsoleCommandAllowed, sealConsoleSession } from "./console-session-guard";
import { session } from "../features/studio/SnowflakeConsole.test-fixtures";
test("authoritative console seal fences delayed direct and nested commands permanently", () => {
  const id = "sealed-transport-session";
  sealConsoleSession({ ...session, id });
  for (const command of [
    "health",
    "session_state",
    "session_contexts",
    "completion_catalog",
    "completion_schema",
    "object_details",
    "table_schema",
    "processes",
    "execute_transaction_control",
    "switch_session_schema",
    "unknown_future_metadata",
  ]) {
    expect(() => assertConsoleCommandAllowed(command, { sessionId: id })).toThrow(
      /reviewed SQL console/,
    );
    expect(() => assertConsoleCommandAllowed(command, { request: { sessionId: id } })).toThrow(
      /reviewed SQL console/,
    );
  }
  sealConsoleSession({ ...session, id, consoleMode: false });
  expect(() => assertConsoleCommandAllowed("health", { sessionId: id })).toThrow();
});
test("reviewed execution, disconnect and SQL-free receipt/cancel/result/local operations stay available", () => {
  const id = "sealed-transport-allowed";
  sealConsoleSession({ ...session, id });
  for (const command of ["prepare_snowflake_console", "start_snowflake_console", "disconnect"])
    expect(() =>
      assertConsoleCommandAllowed(command, { sessionId: id, request: { sessionId: id } }),
    ).not.toThrow();
  for (const command of [
    "snowflake_console_run",
    "cancel_run",
    "release_result",
    "authorize_plan",
    "extract_query_params",
    "format_sql",
    "resource_usage",
  ])
    expect(() =>
      assertConsoleCommandAllowed(command, { clientRequestId: "opaque", runId: "opaque" }),
    ).not.toThrow();
});
test("profile consent and ordinary sessions cannot seal a connection", () => {
  const id = "ordinary-transport";
  sealConsoleSession({ ...session, id, consoleMode: undefined });
  expect(() => assertConsoleCommandAllowed("completion_catalog", { sessionId: id })).not.toThrow();
  expect(() => assertConsoleCommandAllowed("health", { sessionId: "other" })).not.toThrow();
  expect(() => assertConsoleCommandAllowed("health", { request: null })).not.toThrow();
});
