import type { EngineKind } from "../ipc/generated";
import { identifierValue, type Identifier } from "./sql-identifier";

export interface CompletionItem {
  label: string;
  type: string;
  detail?: string;
  apply?: string;
  /** Identifier segments, kept separate so an embedded dot is never a qualifier. */
  path?: string[];
}
export class CompletionIndex {
  private entries: { key: string; item: CompletionItem }[] = [];
  replace(items: CompletionItem[]) {
    this.entries = items
      .map((item) => ({ key: (item.path?.join("\0") ?? item.label).toLowerCase(), item }))
      .sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  }
  prefix(prefix: string, limit = 100, accept: (item: CompletionItem) => boolean = () => true) {
    const key = prefix.toLowerCase();
    let low = 0,
      high = this.entries.length;
    while (low < high) {
      const mid = (low + high) >>> 1;
      if (this.entries[mid].key < key) low = mid + 1;
      else high = mid;
    }
    const output: CompletionItem[] = [];
    for (
      let i = low;
      i < this.entries.length && this.entries[i].key.startsWith(key) && output.length < limit;
      i++
    )
      if (accept(this.entries[i].item)) output.push(this.entries[i].item);
    return output;
  }

  search(parts: Identifier[], engine: EngineKind, limit = 100) {
    return this.prefix(parts.map((p) => p.value).join("\0"), limit, (item) => {
      if (!item.path) return parts.length === 1 && !parts[0].quoted;
      return parts.every((part, i) => {
        const value = item.path![i];
        if (value === undefined) return false;
        if (i === parts.length - 1)
          return part.quoted
            ? value.startsWith(part.value)
            : value.toLowerCase().startsWith(part.value.toLowerCase());
        return engine === "mysql" && !part.quoted
          ? value.toLowerCase() === part.value.toLowerCase()
          : value === identifierValue(part, engine);
      });
    });
  }
}
