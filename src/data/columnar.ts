import { tableFromIPC, Type } from "apache-arrow";

export interface Utf8Segment {
  length: number;
  offset: number;
  offsets: Int32Array;
  values: Uint8Array;
  validity: Uint8Array;
  nullCount: number;
}
export interface ColumnarPage {
  resultId: string;
  start: number;
  length: number;
  columns: { id: string; segments: Utf8Segment[] }[];
  byteLength: number;
}

export function decodeArrow(buffer: ArrayBuffer): ColumnarPage {
  const table = tableFromIPC(buffer);
  const resultId = table.schema.metadata.get("joinery.resultId");
  const start = Number(table.schema.metadata.get("joinery.offset"));
  if (table.schema.metadata.get("joinery.protocol") !== "1" || !resultId || !Number.isSafeInteger(start) || start < 0) throw new Error("Unsupported result stream");
  const columns = table.schema.fields.map((field, index) => {
    if (field.type.typeId !== Type.Utf8) throw new Error("Unsupported physical result type");
    const vector = table.getChildAt(index)!;
    return { id: field.name, segments: vector.data.map((data) => ({ length: data.length, offset: data.offset, offsets: data.valueOffsets as Int32Array, values: data.values as Uint8Array, validity: data.nullBitmap, nullCount: data.nullCount })) };
  });
  return { resultId, start, length: table.numRows, columns, byteLength: buffer.byteLength };
}

const decoder = new TextDecoder();
export function readValue(page: ColumnarPage, columnId: string, row: number, preview = true): { value: string | null; large: boolean } | undefined {
  const column = page.columns.find((column) => column.id === columnId);
  if (!column || row < page.start || row >= page.start + page.length) return;
  let index = row - page.start;
  for (const segment of column.segments) {
    if (index >= segment.length) { index -= segment.length; continue; }
    const bit = index + segment.offset;
    if (segment.nullCount > 0 && !(segment.validity[bit >> 3] & (1 << (bit & 7)))) return { value: null, large: false };
    const from = segment.offsets[index], to = segment.offsets[index + 1];
    const large = to - from > 4096;
    const value = decoder.decode(segment.values.subarray(from, preview && large ? Math.min(to, from + 4096) : to), { stream: false });
    return { value: preview && large ? `${value}…` : value, large };
  }
}

export function transferableBuffers(page: ColumnarPage): ArrayBuffer[] {
  const buffers = new Set<ArrayBuffer>();
  for (const column of page.columns) for (const segment of column.segments) {
    for (const array of [segment.offsets, segment.values, segment.validity]) if (array.buffer instanceof ArrayBuffer) buffers.add(array.buffer);
  }
  return [...buffers];
}
