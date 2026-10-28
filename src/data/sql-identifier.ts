import type { EngineKind } from "../ipc/generated";

export interface Identifier {
  value: string;
  quoted: boolean;
}

export function identifierValue(part: Identifier, engine: EngineKind): string {
  if (part.quoted || engine === "mysql") return part.value;
  return engine === "snowflake" ? part.value.toUpperCase() : part.value.toLowerCase();
}
