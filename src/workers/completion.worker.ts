import { CompletionIndex, type CompletionItem } from "../data/completion-index";
const index = new CompletionIndex();
self.onmessage = ({ data }: MessageEvent<{ type: "replace"; items: CompletionItem[]; revision: number } | { type: "prefix"; id: number; prefix: string; revision: number }>) => {
  if (data.type === "replace") { index.replace(data.items); self.postMessage({ type: "ready", revision: data.revision }); }
  else self.postMessage({ type: "suggestions", id: data.id, revision: data.revision, items: index.prefix(data.prefix) });
};
