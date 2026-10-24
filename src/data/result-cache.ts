import { client, messageOf } from "../ipc/client";
import { readValue, type ColumnarPage } from "./columnar";

const PAGE_ROWS = 128;
const CACHE_BUDGET = 32 * 1024 * 1024;

export class ResultCache {
  private pages = new Map<string, ColumnarPage>();
  private pending = new Map<string, Promise<void>>();
  private errors = new Map<string, string>();
  private queue: (() => void)[] = [];
  private active = 0;
  private bytes = 0;
  private listeners = new Set<() => void>();
  private worker: Worker | null = null;
  private sequence = 0;
  private decoding = new Map<
    number,
    { resolve: (page: ColumnarPage) => void; reject: (error: Error) => void }
  >();
  private generations = new Map<string, number>();
  private notifyFrame = 0;
  private pageSizes = new Map<string, number>();

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private notify() {
    if (!this.notifyFrame)
      this.notifyFrame = requestAnimationFrame(() => {
        this.notifyFrame = 0;
        this.listeners.forEach((listener) => listener());
      });
  }
  private key(id: string, row: number) {
    const size = this.pageSizes.get(id) ?? PAGE_ROWS;
    return `${id}:${Math.floor(row / size) * size}`;
  }

  read(id: string, column: string, row: number, full = false) {
    const key = this.key(id, row);
    const page = this.pages.get(key);
    if (!page) return;
    this.pages.delete(key);
    this.pages.set(key, page);
    return readValue(page, column, row, !full);
  }
  error(id: string, row: number) {
    return this.errors.get(this.key(id, row));
  }

  ensure(id: string, row: number, rowCount: number): Promise<void> {
    const key = this.key(id, row);
    if (this.pages.has(key)) return Promise.resolve();
    const pending = this.pending.get(key);
    if (pending) return pending;
    if (this.errors.has(key)) return Promise.resolve();
    const generation = this.generations.get(id) ?? 0;
    const task = new Promise<void>((resolve) => {
      const run = async () => {
        this.active++;
        try {
          let size = this.pageSizes.get(id) ?? PAGE_ROWS,
            start = Math.floor(row / size) * size,
            buffer: ArrayBuffer;
          while (true) {
            try {
              buffer = await client.range(id, start, Math.min(size, rowCount - start));
              break;
            } catch (error) {
              if (
                size === 1 ||
                !error ||
                typeof error !== "object" ||
                !("code" in error) ||
                error.code !== "resultLimitReached"
              )
                throw error;
              size = Math.min(
                this.pageSizes.get(id) ?? PAGE_ROWS,
                Math.max(1, Math.floor(size / 4)),
              );
              this.pageSizes.set(id, size);
              this.dropPages(id);
              start = Math.floor(row / size) * size;
            }
          }
          const page = await this.decode(buffer);
          if (page.resultId !== id || page.start !== start)
            throw new Error("Received data for a different result");
          if ((this.generations.get(id) ?? 0) !== generation) return;
          if (size !== (this.pageSizes.get(id) ?? PAGE_ROWS)) return;
          if (page.byteLength > CACHE_BUDGET)
            throw new Error("This result page exceeds the display cache budget");
          while (
            (this.bytes + page.byteLength > CACHE_BUDGET || this.pages.size >= 512) &&
            this.pages.size
          ) {
            const oldest = this.pages.entries().next().value!;
            this.pages.delete(oldest[0]);
            this.bytes -= oldest[1].byteLength;
          }
          const actualKey = this.key(id, row);
          const previous = this.pages.get(actualKey);
          if (previous) this.bytes -= previous.byteLength;
          this.pages.set(actualKey, page);
          this.bytes += page.byteLength;
        } catch (error) {
          if ((this.generations.get(id) ?? 0) === generation)
            this.errors.set(this.key(id, row), messageOf(error));
        } finally {
          this.pending.delete(key);
          this.active--;
          if (![...this.pending.keys()].some((key) => key.startsWith(`${id}:`)))
            this.generations.delete(id);
          if (!this.pending.size && !this.pages.size) {
            this.worker?.terminate();
            this.worker = null;
          }
          this.notify();
          resolve();
          this.queue.shift()?.();
        }
      };
      if (this.active < 4) void run();
      else this.queue.push(() => void run());
    });
    this.pending.set(key, task);
    return task;
  }

  private decode(buffer: ArrayBuffer): Promise<ColumnarPage> {
    if (!this.worker) {
      this.worker = new Worker(new URL("../workers/arrow.worker.ts", import.meta.url), {
        type: "module",
      });
      this.worker.onmessage = ({
        data,
      }: MessageEvent<{ id: number; page?: ColumnarPage; error?: string }>) => {
        const task = this.decoding.get(data.id);
        this.decoding.delete(data.id);
        if (data.page) task?.resolve(data.page);
        else task?.reject(new Error(data.error ?? "Could not decode result"));
      };
      this.worker.onerror = () => {
        for (const task of this.decoding.values())
          task.reject(new Error("The result decoder stopped"));
        this.decoding.clear();
        this.worker?.terminate();
        this.worker = null;
      };
    }
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      this.decoding.set(id, { resolve, reject });
      this.worker!.postMessage({ id, buffer }, [buffer]);
    });
  }

  release(id: string) {
    this.generations.set(id, (this.generations.get(id) ?? 0) + 1);
    this.dropPages(id);
    this.pageSizes.delete(id);
    for (const key of this.errors.keys()) if (key.startsWith(`${id}:`)) this.errors.delete(key);
    if (![...this.pending.keys()].some((key) => key.startsWith(`${id}:`)))
      this.generations.delete(id);
    if (!this.pending.size && !this.pages.size) {
      this.worker?.terminate();
      this.worker = null;
    }
    this.notify();
  }
  private dropPages(id: string) {
    for (const [key, page] of this.pages)
      if (page.resultId === id) {
        this.pages.delete(key);
        this.bytes -= page.byteLength;
      }
  }
  get retainedBytes() {
    return this.bytes;
  }
}

export const resultCache = new ResultCache();
