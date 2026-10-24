export interface CompletionItem {
  label: string;
  type: string;
  detail?: string;
}
export class CompletionIndex {
  private entries: { key: string; item: CompletionItem }[] = [];
  replace(items: CompletionItem[]) {
    this.entries = items
      .map((item) => ({ key: item.label.toLowerCase(), item }))
      .sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  }
  prefix(prefix: string, limit = 100) {
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
      output.push(this.entries[i].item);
    return output;
  }
}
