import { afterEach, expect, test, vi } from "vitest";
import { Table, Utf8, vectorFromArray, tableToIPC } from "apache-arrow";
import { client } from "../ipc/client";
import { decodeArrow } from "./columnar";
import { ResultCache } from "./result-cache";

vi.mock("../ipc/client", () => ({
  client: { range: vi.fn() },
  messageOf: (error: unknown) => String(error),
}));
afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});
test("adapts wide result pages and fetches every requested row at the smaller size", async () => {
  class Decoder {
    onmessage?: (message: unknown) => void;
    postMessage({ id, buffer }: { id: number; buffer: ArrayBuffer }) {
      queueMicrotask(() => this.onmessage?.({ data: { id, page: decodeArrow(buffer) } }));
    }
    terminate() {}
  }
  vi.stubGlobal("Worker", Decoder);
  vi.stubGlobal("requestAnimationFrame", () => 1);
  vi.mocked(client.range).mockImplementation(async (id, start, count) => {
    if (count > 8) throw { code: "resultLimitReached", message: "wide page" };
    const table = new Table({
      c0: vectorFromArray(
        Array.from({ length: count }, (_, i) => `row ${start + i}`),
        new Utf8(),
      ),
    });
    table.schema.metadata.set("joinery.protocol", "1");
    table.schema.metadata.set("joinery.resultId", id);
    table.schema.metadata.set("joinery.offset", String(start));
    const bytes = tableToIPC(table, "stream");
    return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
  });
  const cache = new ResultCache();
  await cache.ensure("wide", 5, 1000);
  expect(cache.read("wide", "c0", 5)?.value).toBe("row 5");
  await cache.ensure("wide", 60, 1000);
  expect(cache.read("wide", "c0", 60)?.value).toBe("row 60");
  expect(client.range).toHaveBeenCalledWith("wide", 56, 8);
  cache.release("wide");
  expect(cache.retainedBytes).toBe(0);
});
