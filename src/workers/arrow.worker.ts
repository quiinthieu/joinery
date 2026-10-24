import { decodeArrow, transferableBuffers } from "../data/columnar";

self.onmessage = ({ data }: MessageEvent<{ id: number; buffer: ArrayBuffer }>) => {
  try {
    const page = decodeArrow(data.buffer);
    self.postMessage({ id: data.id, page }, { transfer: transferableBuffers(page) });
  } catch (error) {
    self.postMessage({
      id: data.id,
      error: error instanceof Error ? error.message : "Invalid Arrow data",
    });
  }
};
