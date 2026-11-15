import test from "node:test";
import assert from "node:assert/strict";
import { Readable } from "node:stream";
import { createQualificationDecoder, FRAMING_LIMITS } from "./snowflake-qualification-framing.mjs";
import {
  PROTOCOL_LIMITS,
  createQualificationProtocol,
} from "./snowflake-qualification-protocol.mjs";
import { compileManifest, WORKFLOW, SOURCE_PATHS } from "./snowflake-qualification-manifest.mjs";
import { Helper, LIMITS } from "../snowflake-helper/src/helper.mjs";
import { encode } from "../snowflake-helper/src/framing.mjs";

function bytes(body) {
  const content = typeof body === "string" ? Buffer.from(body) : body;
  const header = Buffer.alloc(4);
  header.writeUInt32BE(content.length);
  return Buffer.concat([header, content]);
}
function setup() {
  const received = [];
  const decoder = createQualificationDecoder({
    onFrame: (entry) => {
      received.push(entry);
    },
  });
  return { decoder, received };
}
function rejects(action, code) {
  assert.throws(action, (error) => {
    assert.equal(error.safeCode, code);
    assert.equal(error.message, code);
    assert.deepEqual(Object.keys(error), ["safeCode"]);
    return true;
  });
}
function rejectsBody(body, code) {
  const { decoder, received } = setup();
  rejects(() => decoder.push(bytes(body)), code);
  assert.equal(received.length, 0);
  assert.equal(decoder.snapshot().failure, code);
  rejects(() => decoder.push(encode({})), code);
  rejects(() => decoder.end(), code);
}

test("physical limits match protocol; callback is required without accessors or extra options", () => {
  for (const key of ["frameBytes", "inboundBytes", "inboundFrames"])
    assert.equal(FRAMING_LIMITS[key], PROTOCOL_LIMITS[key]);
  assert.equal(Object.isFrozen(FRAMING_LIMITS), true);
  for (const options of [undefined, null, [], {}, { onFrame: null }, { onFrame() {}, other: 1 }])
    rejects(() => createQualificationDecoder(options), "decoderConfiguration");
  let touched = false;
  rejects(
    () =>
      createQualificationDecoder({
        get onFrame() {
          touched = true;
        },
      }),
    "decoderConfiguration",
  );
  assert.equal(touched, false);
});

test("every byte boundary preserves frame and exact raw whitespace/escaped body length", () => {
  const body = ' \r\n\t{"v":1,"type":"hello","unicode":"é😀","\\u006b":"value"}  ';
  const encoded = bytes(body);
  for (let split = 0; split <= encoded.length; split++) {
    const { decoder, received } = setup();
    const first = decoder.push(encoded.subarray(0, split));
    const second = decoder.push(encoded.subarray(split));
    assert.equal(first + second, 1);
    decoder.end();
    assert.deepEqual(received, [{ frame: JSON.parse(body), bodyBytes: Buffer.byteLength(body) }]);
    assert.equal(decoder.snapshot().inboundBytes, encoded.length);
    assert.equal(decoder.snapshot().processedBytes, encoded.length);
    assert.equal(decoder.snapshot().bodyAllocations, 1);
    assert.equal(decoder.snapshot().bodyBytesCopied, encoded.length - 4);
    assert.equal(decoder.snapshot().ended, true);
  }
});

test("coalesced frames deliver synchronously before later malformed data", () => {
  const good = { v: 1, type: "phase", phase: "acknowledged" };
  const { decoder, received } = setup();
  const joined = Buffer.concat([encode(good), bytes('{"bad":')]);
  rejects(() => decoder.push(joined), "decoderJson");
  assert.deepEqual(received, [{ frame: good, bodyBytes: encode(good).length - 4 }]);
  assert.equal(decoder.snapshot().frames, 1);
  assert.equal(decoder.snapshot().inboundBytes, joined.length);
  rejects(() => decoder.push(encode(good)), "decoderJson");
  assert.equal(received.length, 1);
});

test("known ACK is delivered before later truncated header/body EOF and remains retained", () => {
  const acknowledged = { type: "phase", phase: "acknowledged" };
  for (const suffix of [Buffer.from([0, 0]), bytes('{"later":1}').subarray(0, 9)]) {
    const { decoder, received } = setup();
    assert.equal(decoder.push(Buffer.concat([encode(acknowledged), suffix])), 1);
    assert.equal(Object.isFrozen(received[0]), true);
    rejects(() => decoder.end(), "decoderTruncated");
    assert.deepEqual(
      received.map((entry) => entry.frame),
      [acknowledged],
    );
    assert.equal(decoder.snapshot().frames, 1);
  }
});

test("zero/oversized big-endian headers fail before body allocation", () => {
  for (const size of [0, FRAMING_LIMITS.frameBytes + 1, 0xffffffff]) {
    const { decoder } = setup();
    const header = Buffer.alloc(4);
    header.writeUInt32BE(size);
    decoder.push(header.subarray(0, 3));
    rejects(() => decoder.push(header.subarray(3)), "decoderFrameLimit");
    assert.equal(decoder.snapshot().bodyAllocations, 0);
    assert.equal(decoder.snapshot().bodyBytesCopied, 0);
    assert.equal(decoder.snapshot().processedBytes, 4);
  }
});

test("exact maximum raw frame accepts padding and one-byte splits copy each body byte once", () => {
  const prefix = Buffer.from('{"text":"9007199254740993"}');
  const body = Buffer.concat([prefix, Buffer.alloc(FRAMING_LIMITS.frameBytes - prefix.length, 32)]);
  const encoded = bytes(body);
  const { decoder, received } = setup();
  for (let index = 0; index < encoded.length; index++)
    decoder.push(encoded.subarray(index, index + 1));
  decoder.end();
  assert.equal(received[0].bodyBytes, FRAMING_LIMITS.frameBytes);
  assert.deepEqual(received[0].frame, { text: "9007199254740993" });
  assert.equal(decoder.snapshot().bodyAllocations, 1);
  assert.equal(decoder.snapshot().bodyBytesCopied, FRAMING_LIMITS.frameBytes);
});

test("8MiB physical limit includes headers, padding and incomplete bytes; preserves valid prefix", () => {
  const { decoder, received } = setup();
  const large = bytes(
    Buffer.concat([Buffer.from("{}"), Buffer.alloc(FRAMING_LIMITS.frameBytes - 2, 32)]),
  );
  for (let index = 0; index < 7; index++) decoder.push(large);
  const remainder = FRAMING_LIMITS.inboundBytes - 7 * large.length;
  const last = bytes(Buffer.concat([Buffer.from("{}"), Buffer.alloc(remainder - 6, 32)]));
  assert.equal(last.length, remainder);
  rejects(() => decoder.push(Buffer.concat([last, Buffer.from([0])])), "decoderByteLimit");
  assert.equal(received.length, 8);
  assert.equal(decoder.snapshot().processedBytes, FRAMING_LIMITS.inboundBytes);
  assert.equal(decoder.snapshot().inboundBytes, FRAMING_LIMITS.inboundBytes + 1);

  const partial = setup();
  for (let index = 0; index < 7; index++) partial.decoder.push(large);
  const header = Buffer.alloc(4);
  header.writeUInt32BE(FRAMING_LIMITS.frameBytes);
  partial.decoder.push(Buffer.concat([header, Buffer.alloc(remainder - 4, 32)]));
  rejects(() => partial.decoder.push(Buffer.from([32])), "decoderByteLimit");
  assert.equal(partial.received.length, 7);
});

test("4096 frame limit rejects next header without allocating body and preserves prior frames", () => {
  const { decoder, received } = setup();
  const one = encode({});
  rejects(
    () => decoder.push(Buffer.concat(Array.from({ length: 4097 }, () => one))),
    "decoderFrameCount",
  );
  assert.equal(received.length, 4096);
  assert.equal(decoder.snapshot().bodyAllocations, 4096);
  assert.equal(decoder.snapshot().processedBytes, 4096 * one.length + 4);
});

test("fatal UTF-8 rejects replacement, overlong, surrogate, truncated and BOM sequences", () => {
  for (const raw of [
    [0xff],
    [0xc0, 0xaf],
    [0xed, 0xa0, 0x80],
    [0xf4, 0x90, 0x80, 0x80],
    [0xe2, 0x82],
  ])
    rejectsBody(
      Buffer.concat([Buffer.from('{"value":"'), Buffer.from(raw), Buffer.from('"}')]),
      "decoderUtf8",
    );
  rejectsBody(Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from("{}")]), "decoderBom");
  // A real U+FFFD or U+FEFF within a JSON string is valid data, not decoder repair.
  const { decoder, received } = setup();
  decoder.push(encode({ value: "�\ufeff" }));
  decoder.end();
  assert.equal(received[0].frame.value, "�\ufeff");
});

test("nested and Unicode-escaped duplicate property names reject before any frame delivery", () => {
  for (const body of [
    '{"a":1,"a":2}',
    '{"a":1,"\\u0061":2}',
    '{"nested":{"x":1,"\\u0078":2}}',
    '{"array":[{"😀":1,"\\ud83d\\ude00":2}]}',
    '{"\\u005f_proto__":1,"__proto__":2}',
    '{"quote\\\"":1,"quote\\u0022":2}',
  ])
    rejectsBody(body, "decoderDuplicateKey");
  const { decoder, received } = setup();
  decoder.push(bytes('{"a":{"same":1},"b":{"same":2},"__proto__":{"safe":true}}'));
  assert.equal(Object.hasOwn(received[0].frame, "__proto__"), true);
  assert.equal(Object.hasOwn({}, "safe"), false);
});

test("strict JSON rejects malformed tokens, controls, trailing content and non-object top levels", () => {
  for (const body of ["", " ", "null", "true", "1", '"text"', "[]"])
    rejectsBody(body, body === "" ? "decoderFrameLimit" : "decoderTopLevel");
  for (const body of [
    "{",
    '{"a"}',
    '{"a":}',
    '{"a":1,}',
    '{"a":[1,]}',
    "{a:1}",
    '{"a":undefined}',
    '{"a":True}',
    '{"a":null}{}',
    "{}x",
    '{"a":"\n"}',
    '{"a":"\\q"}',
    '{"a":"\\u12x4"}',
    '{"a":"unterminated}',
    '{"a":/*x*/1}',
    '{"a":01}',
    '{"a":+1}',
    '{"a":.1}',
    '{"a":1.}',
    '{"a":1e}',
    '{"a":1e+}',
    '{"a":NaN}',
    '{"a":Infinity}',
    '{"a":-}',
    '{"a":--1}',
  ])
    rejectsBody(body, "decoderJson");
});

test("helper number subset rejects rounding, negative zero and fractions; wire decimal strings survive", () => {
  for (const value of [
    "9007199254740993",
    "1e999",
    "1e-3",
    "1.5",
    "-0",
    "-0e0",
    "1e-999",
    "1.00000000000000001",
    "9007199254740990.5",
  ])
    rejectsBody(`{"number":${value}}`, "decoderNumber");
  const { decoder, received } = setup();
  decoder.push(
    bytes(
      '{"max":9007199254740991,"min":-9007199254740991,"exponent":1e2,"decimal":2.0,"wire":"9007199254740993"}',
    ),
  );
  assert.deepEqual(received[0].frame, {
    max: Number.MAX_SAFE_INTEGER,
    min: Number.MIN_SAFE_INTEGER,
    exponent: 100,
    decimal: 2,
    wire: "9007199254740993",
  });
  const shifted = setup();
  shifted.decoder.push(bytes('{"zero":0e999999,"exact":90071992547409910e-1}'));
  assert.deepEqual(shifted.received[0].frame, { zero: 0, exact: Number.MAX_SAFE_INTEGER });
});

test("Unicode surrogate escapes require paired scalars without rejecting valid escapes", () => {
  for (const value of [
    '"\\ud800"',
    '"\\udc00"',
    '"\\ud800x"',
    '"\\ud800\\u0041"',
    '"\\ud800\\ud800"',
  ])
    rejectsBody(`{"value":${value}}`, "decoderJson");
  const { decoder, received } = setup();
  decoder.push(bytes('{"value":"\\ud83d\\ude00\\u0000\\b\\f\\n\\r\\t\\/\\\\\\\""}'));
  assert.equal(received[0].frame.value, '😀\0\b\f\n\r\t/\\"');
});

test("depth, total value nodes and per-container entries are independently bounded", () => {
  const nested = (depth) => '{"x":'.repeat(depth) + "0" + "}".repeat(depth);
  const { decoder } = setup();
  decoder.push(bytes(nested(16)));
  rejectsBody(nested(17), "decoderResourceLimit");
  const array = (length) => Array(length).fill(0);
  const atEntries = setup();
  atEntries.decoder.push(encode({ rows: array(4096) }));
  rejectsBody(JSON.stringify({ rows: array(4097) }), "decoderResourceLimit");
  const atNodes = setup();
  atNodes.decoder.push(encode({ a: array(4096), b: array(4096), c: array(1804) }));
  // Root + three arrays +9996 scalars =10,000; one additional scalar fails.
  rejectsBody(
    JSON.stringify({ a: array(4096), b: array(4096), c: array(1805) }),
    "decoderResourceLimit",
  );
  const members = Object.fromEntries(Array.from({ length: 4096 }, (_, index) => [`k${index}`, 0]));
  const atMembers = setup();
  atMembers.decoder.push(encode(members));
  rejectsBody(JSON.stringify({ ...members, extra: 0 }), "decoderResourceLimit");
});

test("every incomplete header/body suffix fails EOF permanently; completed EOF is idempotent", () => {
  const encoded = encode({ type: "terminal", ok: true });
  for (let cut = 1; cut < encoded.length; cut++) {
    const { decoder, received } = setup();
    decoder.push(encoded.subarray(0, cut));
    rejects(() => decoder.end(), "decoderTruncated");
    rejects(() => decoder.push(encoded.subarray(cut)), "decoderTruncated");
    assert.deepEqual(received, []);
  }
  const { decoder } = setup();
  decoder.end();
  decoder.end();
  rejects(() => decoder.push(new Uint8Array()), "decoderClosed");
  assert.equal(Object.isFrozen(decoder.snapshot()), true);
});

test("consumer throws/returns/async/reentry fail safely without replaying or delivering later frames", async () => {
  for (const behavior of [
    () => {
      throw new Error("PRIVATE_RAW_ERROR");
    },
    () => 1,
    async () => {
      throw new Error("PRIVATE_ASYNC_ERROR");
    },
    () => ({
      get then() {
        throw new Error("must never read then");
      },
    }),
  ]) {
    let called = 0;
    const decoder = createQualificationDecoder({
      onFrame() {
        called++;
        return behavior();
      },
    });
    rejects(() => decoder.push(Buffer.concat([encode({}), encode({})])), "decoderConsumerFailed");
    assert.equal(called, 1);
    assert.equal(decoder.snapshot().frames, 1);
    rejects(() => decoder.push(encode({})), "decoderConsumerFailed");
  }
  for (const action of ["push", "end"]) {
    let decoder;
    decoder = createQualificationDecoder({
      onFrame() {
        decoder[action](encode({}));
      },
    });
    rejects(() => decoder.push(encode({})), "decoderReentrant");
    assert.equal(decoder.snapshot().frames, 1);
  }
  await new Promise((resolve) => setImmediate(resolve));
});

test("only branded Uint8Array/Buffer inputs accepted, with no caller property getter reads", () => {
  for (const input of [
    null,
    "bytes",
    [],
    new DataView(new ArrayBuffer(8)),
    new Uint16Array(8),
    Object.create(Uint8Array.prototype),
  ]) {
    const { decoder } = setup();
    rejects(() => decoder.push(input), "decoderInput");
  }
  const { decoder, received } = setup();
  const view = new Uint8Array(encode({ valid: true }));
  Object.defineProperty(view, "byteLength", {
    get() {
      throw new Error("not read");
    },
  });
  Object.defineProperty(view, "subarray", {
    get() {
      throw new Error("not read");
    },
  });
  decoder.push(view);
  assert.deepEqual(received[0].frame, { valid: true });
});

test("swallowed consumer reentry cannot resume outer delivery", () => {
  for (const action of ["push", "end"]) {
    let decoder;
    let calls = 0;
    decoder = createQualificationDecoder({
      onFrame() {
        calls++;
        try {
          decoder[action](encode({}));
        } catch {}
      },
    });
    rejects(() => decoder.push(Buffer.concat([encode({}), encode({})])), "decoderReentrant");
    assert.equal(calls, 1);
    assert.equal(decoder.snapshot().frames, 1);
    rejects(() => decoder.push(encode({})), "decoderReentrant");
  }
});

test("input constructor and species hooks are never used to copy bytes", () => {
  for (const hook of ["constructor", "species"]) {
    const { decoder, received } = setup();
    const view = new Uint8Array(encode({ valid: true }));
    let touched = false;
    const getter = () => {
      touched = true;
      throw new Error("PRIVATE_RAW_ERROR");
    };
    if (hook === "constructor") Object.defineProperty(view, "constructor", { get: getter });
    else
      Object.defineProperty(view, "constructor", {
        value: {
          get [Symbol.species]() {
            return getter();
          },
        },
      });
    assert.equal(decoder.push(view), 1);
    assert.equal(touched, false);
    assert.deepEqual(received[0].frame, { valid: true });
    assert.equal(decoder.snapshot().failure, null);
  }
});

test("input detachment during a callback fails safely after preserving the delivered prefix", () => {
  const input = new Uint8Array(
    Buffer.concat([encode({ acknowledged: true }), encode({ later: true })]),
  );
  const received = [];
  const decoder = createQualificationDecoder({
    onFrame(entry) {
      received.push(entry);
      structuredClone(input.buffer, { transfer: [input.buffer] });
    },
  });
  rejects(() => decoder.push(input), "decoderInput");
  assert.deepEqual(
    received.map((entry) => entry.frame),
    [{ acknowledged: true }],
  );
  assert.equal(decoder.snapshot().failure, "decoderInput");
  rejects(() => decoder.end(), "decoderInput");
});

test("composed protocol retains known SQL ACK before malformed or truncated coalesced suffix", () => {
  const generation = "12345678-1234-4234-8234-123456789abc";
  const currentInput = {
    workflow: WORKFLOW,
    route: "helper-direct",
    runId: "A1".repeat(16),
    window: { notBefore: "2026-11-10T10:00:00.000Z", expiresAt: "2026-11-10T10:10:00.000Z" },
    profile: {
      account: "synthetic-account",
      host: "synthetic-account.snowflakecomputing.com",
      username: "login@example.test",
      database: "DB",
      schema: "PUBLIC",
      warehouse: "WH",
      role: "ROLE",
    },
    sources: Object.fromEntries(
      SOURCE_PATHS.map((path, index) => [path, index.toString(16).padStart(64, "0")]),
    ),
    runtime: {
      format: "joinery-snowflake-runtime-v1",
      target: "darwin-arm64",
      nodeVersion: "24.22.2",
      nodeSha256: "b6fb419f0f43bb2fbca273700294dd2f3b685a64b60e8417c295f47646a70e4a",
      sdkVersion: "3.4.1",
      manifestSha256: "a".repeat(64),
    },
  };
  for (const malformed of [true, false]) {
    const protocol = createQualificationProtocol({
      manifest: compileManifest(currentInput),
      currentInput,
      generation,
    });
    const decoder = createQualificationDecoder({
      onFrame(entry) {
        protocol.receive(entry.frame, { bodyBytes: entry.bodyBytes });
      },
    });
    const receive = (frame) => decoder.push(encode(frame));
    const reply = (request, type, rest = {}) => ({
      v: 1,
      generation,
      id: request.id,
      type,
      ...rest,
    });
    receive({
      v: 1,
      type: "hello",
      protocol: "joinery-snowflake",
      sdkVersion: "3.4.1",
      limits: { ...LIMITS },
    });
    const connect = protocol.reserveNext();
    receive(reply(connect, "accepted"));
    receive(reply(connect, "terminal", { ok: true, result: { connected: true } }));
    const context = protocol.reserveNext();
    const contextIds = { sdkRequestId: "00000000-0000-4000-8000-000000000001" };
    receive(reply(context, "accepted"));
    receive(reply(context, "phase", { phase: "dispatching" }));
    receive(reply(context, "statement", contextIds));
    receive(reply(context, "phase", { phase: "acknowledged", ...contextIds }));
    receive(
      reply(context, "terminal", {
        ok: true,
        ...contextIds,
        result: {
          user: "SERVER_NAME",
          role: "ROLE",
          database: "DB",
          schema: "PUBLIC",
          warehouse: "WH",
          session: "opaque-session",
          ...contextIds,
        },
      }),
    );
    const version = protocol.reserveNext();
    const versionIds = { sdkRequestId: "00000000-0000-4000-8000-000000000002" };
    receive(reply(version, "accepted"));
    receive(reply(version, "phase", { phase: "dispatching" }));
    receive(reply(version, "statement", versionIds));
    const acknowledged = encode(reply(version, "phase", { phase: "acknowledged", ...versionIds }));
    const suffix = malformed ? bytes('{"invalid":') : Buffer.from([0, 0]);
    if (malformed)
      rejects(() => decoder.push(Buffer.concat([acknowledged, suffix])), "decoderJson");
    else {
      assert.equal(decoder.push(Buffer.concat([acknowledged, suffix])), 1);
      rejects(() => decoder.end(), "decoderTruncated");
    }
    assert.deepEqual(protocol.outcomes().at(-1), {
      commandId: "startup.version",
      op: "query",
      requestStatus: "acknowledged",
      remoteOutcome: "succeeded",
      validationStatus: "pending",
    });
    // Framing failure does not rewrite protocol evidence. A future runner must
    // gate ALL subsequent admission on both independent failure latches.
    assert.equal(protocol.snapshot().failure, null);
    const runnerAdmission = () => decoder.snapshot().failure === null && protocol.reserveNext();
    assert.equal(runnerAdmission(), false);
  }
});

test("actual production Helper emits frames accepted across every boundary without SDK/account/network", async () => {
  const emitted = [];
  let retired = false;
  const sdk = {
    createConnection() {
      return {
        connectAsync(callback) {
          queueMicrotask(() => callback(null));
          return Promise.resolve();
        },
        destroy(callback) {
          queueMicrotask(() => callback(null));
        },
        execute(options) {
          const statement = {
            getRequestId: () => "12345678-1234-4234-8234-123456789abc",
            getQueryId: () => "12345678-1234-4234-8234-123456789abd",
            getColumns: () => [{ getName: () => "EXACT", getType: () => "FIXED" }],
            streamRows: () => Readable.from([["9007199254740993"]]),
          };
          queueMicrotask(() => options.complete(null, statement));
          return statement;
        },
      };
    },
  };
  const helper = new Helper(
    sdk,
    (value) => emitted.push(value),
    () => {
      retired = true;
    },
    { clientConfigFile: "unused-synthetic" },
  );
  const request = (id, op, payload) => ({
    v: 1,
    generation: "12345678-1234-4234-8234-123456789abc",
    id,
    op,
    payload,
  });
  helper.receive(
    request("1", "connect", {
      account: "synthetic",
      host: "synthetic.snowflakecomputing.com",
      username: "synthetic@example.test",
      database: "DB",
      schema: "PUBLIC",
      warehouse: "WH",
      role: "ROLE",
    }),
  );
  for (
    let turn = 0;
    turn < 100 && !emitted.some((entry) => entry.id === "1" && entry.type === "terminal");
    turn++
  )
    await new Promise((resolve) => setImmediate(resolve));
  helper.receive(
    request("2", "query", {
      sql: "SELECT 1",
      binds: [],
      classification: "read",
      credits: 1,
      maxRows: 10,
      maxBytes: 1024,
    }),
  );
  for (
    let turn = 0;
    turn < 100 && !emitted.some((entry) => entry.id === "2" && entry.type === "terminal");
    turn++
  )
    await new Promise((resolve) => setImmediate(resolve));
  helper.receive(request("3", "close", {}));
  for (let turn = 0; turn < 100 && !retired; turn++)
    await new Promise((resolve) => setImmediate(resolve));
  assert.equal(retired, true);
  assert.equal(
    emitted.some((entry) => entry.type === "rows" && entry.rows[0][0] === "9007199254740993"),
    true,
  );
  for (const original of emitted) {
    const encoded = encode(original);
    for (let split = 0; split <= encoded.length; split++) {
      const { decoder, received } = setup();
      decoder.push(encoded.subarray(0, split));
      decoder.push(encoded.subarray(split));
      decoder.end();
      assert.deepEqual(received, [{ frame: original, bodyBytes: encoded.length - 4 }]);
    }
  }
});
