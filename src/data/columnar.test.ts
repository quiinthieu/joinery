import { Table, Utf8, vectorFromArray, tableToIPC } from "apache-arrow";
import { describe, expect, it } from "vitest";
import { decodeArrow, readValue } from "./columnar";

describe("columnar decoding", () => {
  it("reads nullable sliced UTF8 values without losing integer precision", () => {
    const table = new Table({
      c0: vectorFromArray([null, "", "9223372036854775807", "héllo 世界"], new Utf8()),
    });
    table.schema.metadata.set("joinery.protocol", "1");
    table.schema.metadata.set("joinery.resultId", "r");
    table.schema.metadata.set("joinery.offset", "20");
    const bytes = tableToIPC(table, "stream");
    const page = decodeArrow(
      bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer,
    );
    expect(readValue(page, "c0", 20)?.value).toBeNull();
    expect(readValue(page, "c0", 21)?.value).toBe("");
    expect(readValue(page, "c0", 22)?.value).toBe("9223372036854775807");
    expect(readValue(page, "c0", 23)?.value).toBe("héllo 世界");
    expect(readValue(page, "c0", 24)).toBeUndefined();
  });
});
