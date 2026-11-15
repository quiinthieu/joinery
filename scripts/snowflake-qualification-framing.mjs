// Original, pure framing for the fixed qualification workflow, not a general
// JSON parser or an executable runner. No I/O or execution authority is present.
export const FRAMING_LIMITS = Object.freeze({
  frameBytes: 1048576,
  inboundBytes: 8 * 1024 * 1024,
  inboundFrames: 4096,
  maximumDepth: 16,
  maximumNodes: 10000,
  maximumEntries: 4096,
});

const fail = (safeCode) => Object.assign(new Error(safeCode), { safeCode });
const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
const byteLength = Object.getOwnPropertyDescriptor(typedArrayPrototype, "byteLength").get;
const byteOffset = Object.getOwnPropertyDescriptor(typedArrayPrototype, "byteOffset").get;
const buffer = Object.getOwnPropertyDescriptor(typedArrayPrototype, "buffer").get;
const digit = (code) => code >= 48 && code <= 57;
const highSurrogate = (code) => code >= 0xd800 && code <= 0xdbff;
const lowSurrogate = (code) => code >= 0xdc00 && code <= 0xdfff;

// Validate once, then JSON.parse once. Each value/character is visited a bounded
// number of times; decoded object keys use sets to detect nested/escaped duplicate
// names. Only key substrings are separately parsed, with total length <= body.
// The helper workflow subset requires safe integer numbers (excluding -0),
// root depth zero, <=10,000 value nodes and <=4096 entries per container. Raw
// number spelling and JSON whitespace need not be canonical. SDK decimal values
// remain strings, including integers larger than JavaScript's exact range.
function validateJson(text) {
  let index = 0;
  let nodes = 0;
  const invalid = () => {
    throw fail("decoderJson");
  };
  function whitespace() {
    while (true) {
      const code = text.charCodeAt(index);
      if (code !== 32 && code !== 9 && code !== 10 && code !== 13) return;
      index++;
    }
  }
  function hex(start) {
    let value = 0;
    for (let offset = 0; offset < 4; offset++) {
      const code = text.charCodeAt(start + offset);
      const nibble =
        code >= 48 && code <= 57
          ? code - 48
          : code >= 65 && code <= 70
            ? code - 55
            : code >= 97 && code <= 102
              ? code - 87
              : -1;
      if (nibble < 0) invalid();
      value = value * 16 + nibble;
    }
    return value;
  }
  function string(key = false) {
    if (text[index] !== '"') invalid();
    const start = index++;
    while (index < text.length) {
      const code = text.charCodeAt(index);
      if (code === 34) {
        index++;
        return key ? JSON.parse(text.slice(start, index)) : undefined;
      }
      if (code === 92) {
        index++;
        const escape = text[index];
        if (escape === "u") {
          const point = hex(index + 1);
          index += 5;
          if (highSurrogate(point)) {
            if (text[index] !== "\\" || text[index + 1] !== "u") invalid();
            if (!lowSurrogate(hex(index + 2))) invalid();
            index += 6;
          } else if (lowSurrogate(point)) invalid();
        } else if ('"\\/bfnrt'.includes(escape ?? "") && escape !== undefined) index++;
        else invalid();
      } else {
        if (code < 32 || lowSurrogate(code)) invalid();
        if (highSurrogate(code)) {
          if (!lowSurrogate(text.charCodeAt(index + 1))) invalid();
          index += 2;
        } else index++;
      }
    }
    invalid();
  }
  function number() {
    const start = index;
    if (text[index] === "-") index++;
    const integerStart = index;
    if (text[index] === "0") {
      index++;
      if (digit(text.charCodeAt(index))) invalid();
    } else {
      const first = text.charCodeAt(index);
      if (first < 49 || first > 57 || !Number.isInteger(first)) invalid();
      while (digit(text.charCodeAt(index))) index++;
    }
    const integerEnd = index;
    let fractionStart = index,
      fractionEnd = index,
      exponentStart = null;
    if (text[index] === ".") {
      index++;
      fractionStart = index;
      if (!digit(text.charCodeAt(index))) invalid();
      while (digit(text.charCodeAt(index))) index++;
      fractionEnd = index;
    }
    if (text[index] === "e" || text[index] === "E") {
      index++;
      exponentStart = index;
      if (text[index] === "+" || text[index] === "-") index++;
      if (!digit(text.charCodeAt(index))) invalid();
      while (digit(text.charCodeAt(index))) index++;
    }
    const value = Number(text.slice(start, index));
    if (!Number.isSafeInteger(value) || Object.is(value, -0)) throw fail("decoderNumber");
    // Number() alone rounds e.g. 1.00000000000000001 and underflows 1e-999
    // into safe integers. Verify the exact decimal value without an unbounded
    // BigInt or exponent-sized allocation. Every digit is inspected O(1) times.
    let digits = (
      text.slice(integerStart, integerEnd) + text.slice(fractionStart, fractionEnd)
    ).replace(/^0+/, "");
    if (!digits.length) return;
    const exponent = exponentStart === null ? 0 : Number(text.slice(exponentStart, index));
    const shift = exponent - (fractionEnd - fractionStart);
    if (!Number.isSafeInteger(shift)) throw fail("decoderNumber");
    if (shift < 0) {
      if (-shift >= digits.length) throw fail("decoderNumber");
      for (let offset = digits.length + shift; offset < digits.length; offset++)
        if (digits[offset] !== "0") throw fail("decoderNumber");
      digits = digits.slice(0, digits.length + shift);
    } else {
      if (digits.length + shift > 16) throw fail("decoderNumber");
      digits += "0".repeat(shift);
    }
    if (digits !== String(Math.abs(value))) throw fail("decoderNumber");
  }
  function value(depth) {
    if (++nodes > FRAMING_LIMITS.maximumNodes || depth > FRAMING_LIMITS.maximumDepth)
      throw fail("decoderResourceLimit");
    whitespace();
    const token = text[index];
    if (token === '"') string();
    else if (token === "{" || token === "[") {
      const object = token === "{";
      const end = object ? "}" : "]";
      const keys = object ? new Set() : null;
      let entries = 0;
      index++;
      whitespace();
      if (text[index] === end) {
        index++;
        return;
      }
      while (true) {
        if (++entries > FRAMING_LIMITS.maximumEntries) throw fail("decoderResourceLimit");
        whitespace();
        if (object) {
          const key = string(true);
          if (keys.has(key)) throw fail("decoderDuplicateKey");
          keys.add(key);
          whitespace();
          if (text[index++] !== ":") invalid();
        }
        value(depth + 1);
        whitespace();
        if (text[index] === end) {
          index++;
          return;
        }
        if (text[index++] !== ",") invalid();
      }
    } else if (text.startsWith("true", index)) index += 4;
    else if (text.startsWith("false", index)) index += 5;
    else if (text.startsWith("null", index)) index += 4;
    else number();
  }
  whitespace();
  if (text[index] !== "{") throw fail("decoderTopLevel");
  value(0);
  whitespace();
  if (index !== text.length) invalid();
}

/**
 * Required synchronous onFrame({frame, bodyBytes}) delivers each validated frame
 * exactly once BEFORE examining later bytes. push returns the delivered count,
 * not an array: a later malformed frame must not erase an earlier success ACK.
 * The callback must return undefined. Its failure permanently stops decoding;
 * the caller separately retains its original protocol failure. No async consumer
 * or recovery/replay is supported. Input views must remain stable during push.
 *
 * Physical limits include partial headers/bodies and noncanonical whitespace.
 * inboundBytes records all offered bytes, processedBytes the admitted prefix.
 * One checked body allocation/frame and direct
 * copies avoid concatenation or reparsing on every adversarial split. UTF-8 is
 * decoded fatally, with BOM preservation followed by explicit prefix rejection.
 */
export function createQualificationDecoder(options) {
  if (!options || typeof options !== "object" || Array.isArray(options))
    throw fail("decoderConfiguration");
  const descriptor = Object.getOwnPropertyDescriptor(options, "onFrame");
  if (
    Reflect.ownKeys(options).length !== 1 ||
    !descriptor ||
    !Object.hasOwn(descriptor, "value") ||
    typeof descriptor.value !== "function"
  )
    throw fail("decoderConfiguration");
  const onFrame = descriptor.value;
  const header = new Uint8Array(4);
  const utf8 = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
  let headerBytes = 0,
    body = null,
    bodyBytes = 0,
    inboundBytes = 0,
    processedBytes = 0,
    frames = 0,
    bodyAllocations = 0,
    bodyBytesCopied = 0,
    ended = false,
    pushing = false,
    failure = null;

  function stop(code) {
    failure ??= code;
    body = null;
    bodyBytes = 0;
    headerBytes = 0;
    throw fail(failure);
  }
  function open() {
    if (failure) throw fail(failure);
    if (ended) stop("decoderClosed");
  }
  function push(chunk) {
    open();
    if (pushing) stop("decoderReentrant");
    pushing = true;
    try {
      return consume(chunk);
    } catch {
      // A detached/resized input or other copy failure must not escape with a
      // caller-controlled message or leave decoding admission open.
      stop(failure ?? "decoderInput");
    } finally {
      pushing = false;
    }
  }
  function consume(chunk) {
    let length, inputBuffer, inputOffset;
    try {
      if (!(chunk instanceof Uint8Array)) return stop("decoderInput");
      length = byteLength.call(chunk);
      inputBuffer = buffer.call(chunk);
      inputOffset = byteOffset.call(chunk);
    } catch {
      return stop("decoderInput");
    }
    inboundBytes += length;
    // Do not pre-reject an entire coalesced chunk: it may contain an ACK before
    // the byte-budget boundary. Charge/copy only bounded pieces and deliver the
    // valid prefix before rejecting its first byte beyond the physical budget.
    let offset = 0;
    const before = frames;
    while (offset < length) {
      if (processedBytes === FRAMING_LIMITS.inboundBytes) stop("decoderByteLimit");
      const available = Math.min(length - offset, FRAMING_LIMITS.inboundBytes - processedBytes);
      if (!body) {
        const take = Math.min(4 - headerBytes, available);
        // Intrinsic subarray still invokes caller constructor/@@species hooks.
        // Fresh plain views use only branded buffer/offset getters instead.
        header.set(new Uint8Array(inputBuffer, inputOffset + offset, take), headerBytes);
        headerBytes += take;
        processedBytes += take;
        offset += take;
        if (headerBytes !== 4) continue;
        const size = header[0] * 16777216 + header[1] * 65536 + header[2] * 256 + header[3];
        if (size === 0 || size > FRAMING_LIMITS.frameBytes) stop("decoderFrameLimit");
        if (frames === FRAMING_LIMITS.inboundFrames) stop("decoderFrameCount");
        body = new Uint8Array(size);
        bodyAllocations++;
        bodyBytes = 0;
        headerBytes = 0;
      } else {
        const take = Math.min(body.length - bodyBytes, available);
        body.set(new Uint8Array(inputBuffer, inputOffset + offset, take), bodyBytes);
        bodyBytes += take;
        bodyBytesCopied += take;
        processedBytes += take;
        offset += take;
        if (bodyBytes !== body.length) continue;
        const size = body.length;
        let text;
        try {
          text = utf8.decode(body);
        } catch {
          stop("decoderUtf8");
        }
        if (text.charCodeAt(0) === 0xfeff) stop("decoderBom");
        let frame;
        try {
          validateJson(text);
          frame = JSON.parse(text);
        } catch (error) {
          stop(
            [
              "decoderJson",
              "decoderTopLevel",
              "decoderNumber",
              "decoderResourceLimit",
              "decoderDuplicateKey",
            ].includes(error?.safeCode)
              ? error.safeCode
              : "decoderJson",
          );
        }
        body = null;
        bodyBytes = 0;
        // Receipt precedes consumer execution. A throwing consumer is never
        // retried, and later frames in this chunk remain undelivered.
        frames++;
        try {
          const result = onFrame(Object.freeze({ frame, bodyBytes: size }));
          if (result !== undefined) {
            // Attach a rejection handler to a native Promise without invoking
            // an arbitrary thenable. Async callbacks still fail closed.
            try {
              Promise.prototype.then.call(result, undefined, () => {});
            } catch {}
            stop("decoderConsumerFailed");
          }
        } catch {
          stop("decoderConsumerFailed");
        }
        // The consumer may swallow a reentrant push/end exception. The latch
        // still forbids delivery of any later frame from this outer chunk.
        if (failure) throw fail(failure);
      }
    }
    return frames - before;
  }
  function end() {
    if (failure) throw fail(failure);
    if (pushing) stop("decoderReentrant");
    if (ended) return;
    if (headerBytes || body) stop("decoderTruncated");
    ended = true;
  }
  function snapshot() {
    return Object.freeze({
      inboundBytes,
      processedBytes,
      frames,
      headerBytes,
      bodyBytesBuffered: bodyBytes,
      expectedBodyBytes: body?.length ?? 0,
      bodyAllocations,
      bodyBytesCopied,
      ended,
      failure,
    });
  }
  return Object.freeze({ push, end, snapshot });
}
