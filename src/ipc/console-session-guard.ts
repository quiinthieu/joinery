import type { SessionInfo } from "./generated";

// A physical console session can never become an observer connection again.
// Kept outside Zustand so delayed handlers remain fenced after UI retirement.
const sealed = new Set<string>();
const consoleSessionCommands = new Set([
  "prepare_snowflake_console",
  "start_snowflake_console",
  "disconnect",
]);
export function sealConsoleSession(info: Pick<SessionInfo, "id" | "consoleMode">): void {
  if (info.consoleMode === true) sealed.add(info.id);
}

/** Final scheduling fence. Native admission independently enforces the seal. */
export function assertConsoleCommandAllowed(command: string, args?: Record<string, unknown>): void {
  if (consoleSessionCommands.has(command)) return;
  const request = args?.request;
  const nested =
    request && typeof request === "object" && !Array.isArray(request)
      ? (request as Record<string, unknown>).sessionId
      : undefined;
  if ([args?.sessionId, nested].some((id) => typeof id === "string" && sealed.has(id)))
    throw new Error(
      "This physical session belongs to the reviewed SQL console. Automatic SQL and generated operations are disabled; use a separate metadata profile.",
    );
}
