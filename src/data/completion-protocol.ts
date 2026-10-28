import type { EngineKind } from "../ipc/generated";
import type { Identifier } from "./sql-identifier";
import { CompletionIndex, type CompletionItem } from "./completion-index";

export interface CompletionStamp {
  document: number;
  context: number;
  position: number;
}
export type CompletionRequest =
  | { type: "replace"; section: "catalog" | "columns"; revision: number; items: CompletionItem[] }
  | {
      type: "prefix";
      id: number;
      revision: number;
      stamp: CompletionStamp;
      parts: Identifier[];
      engine: EngineKind;
      columnsOnly: boolean;
    };
export type CompletionResponse =
  | { type: "ready"; revision: number }
  | {
      type: "suggestions";
      id: number;
      revision: number;
      stamp: CompletionStamp;
      items: CompletionItem[];
    };

export class CompletionWorkerIndex {
  private catalog = new CompletionIndex();
  private columns = new CompletionIndex();
  private revision = 0;

  handle(request: CompletionRequest): CompletionResponse {
    if (request.type === "replace") {
      if (request.revision > this.revision) {
        this[request.section].replace(request.items);
        if (request.section === "catalog") this.columns.replace([]);
        this.revision = request.revision;
      }
      return { type: "ready", revision: this.revision };
    }
    const columns =
      request.revision === this.revision ? this.columns.search(request.parts, request.engine) : [];
    const items =
      request.revision !== this.revision || request.columnsOnly
        ? columns
        : [...columns, ...this.catalog.search(request.parts, request.engine, 100 - columns.length)];
    return {
      type: "suggestions",
      id: request.id,
      stamp: request.stamp,
      revision: this.revision,
      items,
    };
  }
}
