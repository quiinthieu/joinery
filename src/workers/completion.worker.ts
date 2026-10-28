import { CompletionWorkerIndex, type CompletionRequest } from "../data/completion-protocol";
const index = new CompletionWorkerIndex();
self.onmessage = ({ data }: MessageEvent<CompletionRequest>) =>
  self.postMessage(index.handle(data));
