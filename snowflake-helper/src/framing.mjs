export const FRAME_BYTES = 1048576;
export function encode(value) {
  const body = Buffer.from(JSON.stringify(value));
  if (!body.length || body.length > FRAME_BYTES) throw new Error("frame");
  const size = Buffer.alloc(4);
  size.writeUInt32BE(body.length);
  return Buffer.concat([size, body]);
}
export class Decoder {
  buffer = Buffer.alloc(0);
  push(chunk) {
    const frames = [];
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (this.buffer.length >= 4) {
      const size = this.buffer.readUInt32BE();
      if (!size || size > FRAME_BYTES) throw new Error("frame");
      if (this.buffer.length < size + 4) break;
      const bytes = this.buffer.subarray(4, size + 4);
      // Buffer.toString alone replaces malformed UTF-8, which must be rejected.
      const value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
      frames.push(value);
      this.buffer = this.buffer.subarray(size + 4);
    }
    if (this.buffer.length > FRAME_BYTES + 4) throw new Error("frame");
    return frames;
  }
  end() {
    if (this.buffer.length) throw new Error("truncated");
  }
}
export function writer(write, fail) {
  let queued = 0,
    chain = Promise.resolve();
  const send = (value) => {
    const frame = encode(value);
    queued += frame.length;
    if (queued > 4 * FRAME_BYTES) {
      fail();
      return;
    }
    chain = chain
      .then(
        () =>
          new Promise((resolve, reject) => {
            write(frame, (error) => (error ? reject(error) : resolve()));
          }),
      )
      .then(() => {
        queued -= frame.length;
      })
      .catch(fail);
    return chain;
  };
  send.flush = () => chain;
  return send;
}
