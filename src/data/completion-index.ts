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

interface IndexedEntry {
  key: string;
  item: CompletionItem;
}

interface LeafEntry {
  key: string;
  depth: number;
  subword: boolean;
  item: CompletionItem;
}

function collectFromSorted(
  entries: readonly IndexedEntry[],
  prefix: string,
  limit: number,
  output: CompletionItem[],
  seen: Set<CompletionItem>,
  accept: (item: CompletionItem) => boolean,
) {
  if (output.length >= limit || entries.length === 0) return;
  const key = prefix.toLowerCase();
  let low = 0,
    high = entries.length;
  while (low < high) {
    const mid = (low + high) >>> 1;
    if (entries[mid].key < key) low = mid + 1;
    else high = mid;
  }
  for (
    let i = low;
    i < entries.length && entries[i].key.startsWith(key) && output.length < limit;
    i++
  ) {
    const item = entries[i].item;
    if (!seen.has(item) && accept(item)) {
      seen.add(item);
      output.push(item);
    }
  }
}

export class CompletionIndex {
  private entries: IndexedEntry[] = [];
  private byDepth = new Map<number, IndexedEntry[]>();
  private keywords: IndexedEntry[] = [];
  private leafEntries: LeafEntry[] | null = null;

  replace(items: CompletionItem[]) {
    this.entries = items
      .map((item) => ({ key: (item.path?.join("\0") ?? item.label).toLowerCase(), item }))
      .sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
    this.byDepth.clear();
    this.keywords = [];
    this.leafEntries = null;
    for (const entry of this.entries) {
      if (entry.item.type === "keyword") {
        this.keywords.push(entry);
      } else {
        const depth = entry.item.path?.length ?? 1;
        let bucket = this.byDepth.get(depth);
        if (!bucket) {
          bucket = [];
          this.byDepth.set(depth, bucket);
        }
        bucket.push(entry);
      }
    }
  }

  private getLeafEntries(): LeafEntry[] {
    if (this.leafEntries) return this.leafEntries;
    const list: LeafEntry[] = [];
    for (const entry of this.entries) {
      const { item } = entry;
      if (item.type === "keyword") continue;
      const depth = item.path?.length ?? 1;
      const leaf = (item.path?.at(-1) ?? item.label).toLowerCase();
      if (depth > 1) {
        list.push({ key: leaf, depth, subword: false, item });
      }
      let pos = leaf.indexOf("_");
      let count = 0;
      while (pos >= 0 && pos + 1 < leaf.length && count < 6) {
        const suffix = leaf.slice(pos + 1);
        if (suffix.length >= 2) {
          list.push({ key: suffix, depth, subword: true, item });
          count++;
        }
        pos = leaf.indexOf("_", pos + 1);
      }
    }
    list.sort((a, b) =>
      a.key < b.key
        ? -1
        : a.key > b.key
          ? 1
          : a.subword !== b.subword
            ? a.subword
              ? 1
              : -1
            : a.depth - b.depth,
    );
    this.leafEntries = list;
    return list;
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
    if (limit <= 0) return [];
    const prefixKey = parts.map((p) => p.value).join("\0");
    const matchesQualifier = (item: CompletionItem) => {
      if (!item.path || item.path.length < parts.length) return false;
      for (let i = 0; i < parts.length - 1; i++) {
        const part = parts[i];
        const value = item.path[i];
        const ok =
          engine === "mysql" && !part.quoted
            ? value.toLowerCase() === part.value.toLowerCase()
            : value === identifierValue(part, engine);
        if (!ok) return false;
      }
      return true;
    };
    const matchesPath = (item: CompletionItem) => {
      if (!item.path) return parts.length === 1 && !parts[0].quoted;
      if (!matchesQualifier(item)) return false;
      const lastPart = parts[parts.length - 1];
      const value = item.path[parts.length - 1];
      if (value === undefined) return false;
      return lastPart.quoted
        ? value.startsWith(lastPart.value)
        : value.toLowerCase().startsWith(lastPart.value.toLowerCase());
    };

    const output: CompletionItem[] = [];
    const seen = new Set<CompletionItem>();

    // 1. Exact-depth non-keyword prefix matches first.
    const exactBucket = this.byDepth.get(parts.length);
    if (exactBucket) {
      collectFromSorted(exactBucket, prefixKey, limit, output, seen, matchesPath);
    }

    // 2. Qualified underscore-subword matches within the exact qualifier scope.
    const lastPart = parts[parts.length - 1];
    if (
      output.length < limit &&
      parts.length > 1 &&
      exactBucket &&
      !lastPart.quoted &&
      lastPart.value.length >= 2
    ) {
      const qualPrefix =
        parts
          .slice(0, -1)
          .map((p) => p.value)
          .join("\0") + "\0";
      const needle = "_" + lastPart.value.toLowerCase();
      collectFromSorted(
        exactBucket,
        qualPrefix,
        limit,
        output,
        seen,
        (item) =>
          matchesQualifier(item) &&
          Boolean(item.path?.[parts.length - 1]?.toLowerCase().includes(needle)),
      );
    }

    // 3. Deeper-path non-keyword prefix matches ordered by depth (2, 3, 4) so
    //    2-part schemas (`DWH.PROD`) precede 3-part tables (`DWH.ANALYTICS.T1`).
    const hasBareColumns = this.byDepth.get(1)?.some((e) => e.item.type === "property") ?? false;
    for (let depth = parts.length + 1; depth <= 4 && output.length < limit; depth++) {
      const bucket = this.byDepth.get(depth);
      if (!bucket) continue;
      collectFromSorted(bucket, prefixKey, limit, output, seen, (item) => {
        if (parts.length === 1 && item.type === "property" && hasBareColumns) return false;
        return matchesPath(item);
      });
    }

    // 4. Unqualified leaf-name and underscore-subword matches for 1-part queries.
    if (
      output.length < limit &&
      parts.length === 1 &&
      !lastPart.quoted &&
      lastPart.value.length > 0
    ) {
      const leafKey = lastPart.value.toLowerCase();
      const leaves = this.getLeafEntries();
      let low = 0,
        high = leaves.length;
      while (low < high) {
        const mid = (low + high) >>> 1;
        if (leaves[mid].key < leafKey) low = mid + 1;
        else high = mid;
      }
      for (const allowSubword of [false, true]) {
        if (allowSubword && leafKey.length < 2) break;
        for (
          let i = low;
          i < leaves.length && leaves[i].key.startsWith(leafKey) && output.length < limit;
          i++
        ) {
          const entry = leaves[i];
          if (entry.subword !== allowSubword) continue;
          const item = entry.item;
          if (seen.has(item)) continue;
          if (item.type === "property" && entry.depth > 1 && hasBareColumns) continue;
          seen.add(item);
          output.push(item);
        }
      }
    }

    // 5. SQL keywords last for 1-part unquoted queries.
    if (output.length < limit && parts.length === 1 && !lastPart.quoted) {
      collectFromSorted(this.keywords, prefixKey, limit, output, seen, () => true);
    }

    return output;
  }
}
