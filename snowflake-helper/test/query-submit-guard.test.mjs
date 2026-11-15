import test from "node:test";
import assert from "node:assert/strict";
import http, { request as namedRequest } from "node:http";
import https from "node:https";
import { EventEmitter, once } from "node:events";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { installQuerySubmitGuard } from "../src/query-submit-guard.mjs";

const A = "11111111-1111-4111-8111-111111111111";
const B = "22222222-2222-4222-8222-222222222222";
const endpoint = `/queries/v1/query-request?requestId=${A}`;
const ownFile = fileURLToPath(import.meta.url);
const require = createRequire(import.meta.url);

function configure(onViolation = () => {}, port = 80) {
  const guard = installQuerySubmitGuard({ onViolation });
  guard.setAuthority({ protocol: "http:", hostname: "127.0.0.1", port });
  const owner = guard.beginOperation();
  const ticket = guard.beginStatement(owner);
  return { guard, owner, ticket };
}
function options(path = endpoint) {
  return { protocol: "http:", hostname: "127.0.0.1", method: "POST", path };
}
function fixedError(action) {
  assert.throws(action, (error) => {
    assert.equal(error.code, "JOINERY_QUERY_SUBMIT_GUARD");
    assert.equal(error.message, "Snowflake query submission admission is closed");
    assert.equal(error.cause, undefined);
    return true;
  });
}
function fakeNative() {
  const calls = [];
  const native = function (...args) {
    calls.push({ receiver: this, args });
    return new EventEmitter();
  };
  http.request = native;
  https.request = native;
  return calls;
}
async function nativeResponse(request) {
  return new Promise((resolve, reject) => {
    request.once("error", reject);
    request.once("response", (response) => {
      response.resume();
      response.once("end", resolve);
    });
    request.end();
  });
}

async function scenario(name, variant) {
  if (name === "passthrough-and-forms") {
    const calls = fakeNative();
    const reports = [];
    const { guard, owner, ticket } = configure((r) => reports.push(r));
    const receiver = {};
    const agent = {};
    const headers = { host: "127.0.0.1" };
    const opts = { ...options(), agent, headers, maxRedirects: 21 };
    let callbackThis, callbackResponse;
    const callback = function (response) {
      callbackThis = this;
      callbackResponse = response;
    };
    guard.registerRequestId(ticket, A);
    const forms = {
      options: [opts, callback],
      "options-undefined": [opts, undefined],
      string: [`http://127.0.0.1${endpoint}`, opts, callback],
      url: [new URL(`http://127.0.0.1${endpoint}`), opts, callback],
      "url-options-undefined": [new URL(`http://127.0.0.1${endpoint}`), opts, undefined],
      "url-undefined-callback": [new URL(`http://127.0.0.1${endpoint}`), undefined, callback],
    };
    // URL-only forms cannot submit POST; an options object supplying method is
    // required. The undefined URL form is tested below as a rejected GET.
    if (variant === "url-undefined-callback") {
      fixedError(() => http.request(...forms[variant]));
      assert.equal(calls.length, 0);
      return;
    }
    assert.equal(namedRequest, http.request);
    const result = http.request.apply(receiver, forms[variant]);
    assert.ok(result instanceof EventEmitter);
    assert.equal(calls.length, 1);
    assert.equal(calls[0].receiver, receiver);
    assert.equal(opts.maxRedirects, 0);
    assert.equal(calls[0].args.includes(opts), true);
    assert.equal(opts.agent, agent);
    assert.equal(opts.headers, headers);
    const actualCallback = calls[0].args.at(-1);
    assert.equal(typeof actualCallback, "function");
    const response = { statusCode: 200 };
    const responseThis = {};
    actualCallback.call(responseThis, response);
    if (!variant.endsWith("undefined")) {
      assert.equal(callbackThis, responseThis);
      assert.equal(callbackResponse, response);
    }
    assert.deepEqual(reports, []);
    guard.endStatement(ticket);
    guard.endOperation(owner);
    const login = { ...options("/session/v1/login-request"), method: "POST" };
    http.request(login, callback);
    assert.equal(calls[1].args[0], login);
    assert.equal(calls[1].args[1], callback);
    assert.equal(login.maxRedirects, undefined);
    return;
  }
  if (name === "sdk-parameters") {
    const calls = fakeNative();
    const { guard, ticket } = configure();
    guard.registerRequestId(ticket, A);
    http.request(
      options(`${endpoint}&request_guid=${B}&clientStartTime=123&retryCount=0&retryReason=0`),
    );
    assert.equal(calls.length, 1);
    assert.equal(guard.snapshot().failed, false);
    return;
  }
  if (name === "negative") {
    const calls = fakeNative();
    const reports = [];
    const { guard, ticket } = configure((report) => reports.push(report));
    guard.registerRequestId(ticket, A);
    const opts = options();
    const variants = {
      encodedPath: { path: endpoint.replace("query-request", "query%2drequest") },
      encodedQuestionPath: { path: endpoint.replace("/query-request", "/%3f%71uery-request") },
      encodedHashPath: { path: endpoint.replace("/query-request", "/%23%71uery-request") },
      twiceEncodedPath: { path: endpoint.replace("query-request", "query%252drequest") },
      casePath: { path: endpoint.replace("queries", "Queries") },
      slashPath: { path: endpoint.replace("/v1/", "//v1/") },
      dotPath: { path: endpoint.replace("/v1/", "/v1/../v1/") },
      duplicateId: { path: `${endpoint}&requestId=${A}` },
      encodedIdKey: { path: endpoint.replace("requestId", "request%49d") },
      encodedId: { path: endpoint.replace(A, `%31${A.slice(1)}`) },
      staleId: { path: endpoint.replace(A, B) },
      badId: { path: endpoint.replace(A, "not-a-uuid") },
      foreignHost: { hostname: "elsewhere.invalid" },
      hostConflict: { host: "elsewhere.invalid" },
      hostHeader: { headers: { Host: "elsewhere.invalid" } },
      duplicateHostHeader: { headers: ["Host", "127.0.0.1", "host", "127.0.0.1"] },
      wrongPort: { port: 81 },
      defaultPort: { defaultPort: 81 },
      agentPort: { agent: { defaultPort: 81 } },
      defaultAgent: { _defaultAgent: { defaultPort: 81 } },
      protocol: { protocol: "https:" },
      method: { method: "GET" },
      socket: { socketPath: "/not-a-real-socket" },
      auth: { auth: "synthetic-secret" },
      extraParameter: { path: `${endpoint}&unreviewed=1` },
      fragment: { path: `${endpoint}#hidden` },
    };
    let action;
    if (variant === "urlOwnPort") {
      const url = new URL(`http://127.0.0.1${endpoint}`);
      Object.defineProperty(url, "port", { value: "81" });
      action = () => http.request(url, { method: "POST" });
    } else if (variant === "rawUrlDot") {
      action = () =>
        http.request(`http://127.0.0.1/queries/v1/x/../query-request?requestId=${A}`, {
          method: "POST",
        });
    } else if (variant === "accessorHeaders") {
      opts.headers = ["Host", "127.0.0.1"];
      Object.defineProperty(opts.headers, "1", { get: () => "127.0.0.1" });
      action = () => http.request(opts);
    } else if (variant === "accessor") {
      Object.defineProperty(opts, "path", { get: () => endpoint });
      action = () => http.request(opts);
    } else if (variant === "frozen") {
      Object.freeze(opts);
      action = () => http.request(opts);
    } else {
      Object.assign(opts, variants[variant]);
      action = () => http.request(opts);
    }
    fixedError(action);
    assert.equal(calls.length, 0);
    assert.equal(reports.length, 1);
    assert.deepEqual(Object.keys(reports[0]).sort(), [
      "admitted",
      "category",
      "failed",
      "registered",
    ]);
    assert.equal(reports[0].failed, true);
    assert.equal(JSON.stringify(reports).includes(A), false);
    assert.equal(JSON.stringify(reports).includes("synthetic-secret"), false);
    fixedError(() => guard.beginOperation());
    fixedError(() => http.request(options()));
    assert.equal(reports.length, 1);
    // Cleanup/result paths remain usable after the irreversible submission latch.
    http.request(options("/queries/synthetic/result?disableOfflineChunks=false"));
    http.request(options("/queries/v1/abort-request"));
    assert.equal(calls.length, 2);
    return;
  }
  if (name === "lifecycle") {
    const calls = fakeNative();
    const reports = [];
    const { guard, owner, ticket } = configure((r) => reports.push(r));
    if (variant === "unregistered") {
      fixedError(() => http.request(options()));
    } else if (variant === "invalidId") {
      fixedError(() => guard.registerRequestId(ticket, "secret-invalid"));
    } else {
      guard.registerRequestId(ticket, A);
      http.request(options());
      assert.equal(calls.length, 1);
      if (variant === "duplicate") fixedError(() => http.request(options()));
      else {
        guard.endStatement(ticket);
        const next = guard.beginStatement(owner);
        if (variant === "reuse") fixedError(() => guard.registerRequestId(next, A.toUpperCase()));
        else {
          guard.registerRequestId(next, B);
          await new Promise((resolve) => setImmediate(resolve));
          fixedError(() => http.request(options()));
        }
      }
    }
    assert.equal(reports.length, 1);
    assert.equal(guard.snapshot().failed, true);
    assert.equal(calls.length, variant === "unregistered" || variant === "invalidId" ? 0 : 1);
    return;
  }
  if (name === "native-throw") {
    let calls = 0,
      reports = 0;
    http.request = () => {
      calls++;
      throw new Error("synthetic URL/token must not escape");
    };
    const { guard, ticket } = configure(() => {
      reports++;
      throw new Error("caller must not reopen");
    });
    guard.registerRequestId(ticket, A);
    fixedError(() => http.request(options()));
    fixedError(() => http.request(options()));
    assert.equal(calls, 1);
    assert.equal(reports, 1);
    assert.deepEqual(guard.snapshot(), { failed: true, registered: 1, admitted: 1 });
    return;
  }
  if (name === "capacity") {
    fakeNative();
    const reports = [];
    const { guard, owner, ticket } = configure((r) => reports.push(r));
    guard.endStatement(ticket);
    for (let i = 0; i < 65_536; i++) {
      const next = guard.beginStatement(owner);
      guard.registerRequestId(
        next,
        `${i.toString(16).padStart(8, "0")}-0000-4000-8000-000000000000`,
      );
      guard.endStatement(next);
    }
    const next = guard.beginStatement(owner);
    fixedError(() => guard.registerRequestId(next, "ffffffff-0000-4000-8000-000000000000"));
    assert.equal(reports[0].category, "capacity");
    assert.equal(guard.snapshot().registered, 65_536);
    fixedError(() => guard.beginOperation());
    return;
  }
  if (name === "reservation") {
    fakeNative();
    const reports = [];
    const { guard, owner, ticket } = configure((r) => reports.push(r));
    guard.endStatement(ticket);
    const finish = guard.reserveCapacity(2);
    const batch = guard.reserveCapacity(65_534);
    assert.ok(finish);
    assert.ok(batch);
    assert.equal(guard.reserveCapacity(1), null);
    assert.equal(guard.snapshot().failed, false);
    if (variant === "legacy") {
      const unreserved = guard.beginStatement(owner);
      fixedError(() => guard.registerRequestId(unreserved, A));
      assert.equal(reports[0].category, "capacity");
      assert.equal(guard.snapshot().registered, 0);
      return;
    }
    if (variant === "invalidLease") {
      fixedError(() => guard.beginStatement(owner, {}));
      return;
    }
    const reserved = guard.beginStatement(owner, batch);
    if (variant === "releaseActive") {
      fixedError(() => guard.releaseCapacity(batch));
      return;
    }
    // A ticket abandoned before registration must not spend its reservation.
    guard.endStatement(reserved);
    const used = guard.beginStatement(owner, batch);
    guard.registerRequestId(used, A);
    if (variant === "retired") {
      fixedError(() => guard.registerRequestId(used, B));
      guard.endStatement(used);
      guard.releaseCapacity(batch);
      guard.releaseCapacity(finish);
      assert.deepEqual(guard.snapshot(), { failed: true, registered: 1, admitted: 0 });
      return;
    }
    guard.endStatement(used);
    guard.releaseCapacity(batch);
    assert.equal(guard.reserveCapacity(65_534), null, "spent IDs cannot be returned");
    const rest = guard.reserveCapacity(65_533);
    assert.ok(rest, "only unused reserved slots were returned");
    guard.endOperation(owner);
    const nextOwner = guard.beginOperation();
    const finishTicket = guard.beginStatement(nextOwner, finish);
    guard.registerRequestId(finishTicket, B);
    guard.endStatement(finishTicket);
    const finalTicket = guard.beginStatement(nextOwner, finish);
    guard.registerRequestId(finalTicket, "33333333-3333-4333-8333-333333333333");
    guard.endStatement(finalTicket);
    if (variant === "empty") {
      fixedError(() => guard.beginStatement(nextOwner, finish));
      return;
    }
    guard.releaseCapacity(finish);
    guard.releaseCapacity(rest);
    guard.endOperation(nextOwner);
    assert.deepEqual(guard.snapshot(), { failed: false, registered: 3, admitted: 0 });
    assert.equal(reports.length, 0);
    return;
  }
  if (name === "https-wrapper" || name === "callback-throw" || name === "immutable-authority") {
    const calls = fakeNative();
    const reports = [];
    const guard = installQuerySubmitGuard({ onViolation: (r) => reports.push(r) });
    const protocol = name === "https-wrapper" ? "https:" : "http:";
    guard.setAuthority({ protocol, hostname: "127.0.0.1", port: protocol === "https:" ? 443 : 80 });
    if (name === "immutable-authority") {
      fixedError(() => guard.setAuthority({ protocol, hostname: "127.0.0.1", port: 81 }));
      fixedError(() => installQuerySubmitGuard({ onViolation() {} }));
      assert.equal(reports.length, 1);
      return;
    }
    const owner = guard.beginOperation();
    const ticket = guard.beginStatement(owner);
    guard.registerRequestId(ticket, A);
    const original = new Error("synthetic callback exception");
    const request = name === "https-wrapper" ? https.request : http.request;
    request({ ...options(), protocol }, () => {
      throw original;
    });
    assert.throws(
      () => calls[0].args.at(-1)({ statusCode: 200 }),
      (error) => error === original,
    );
    assert.equal(guard.snapshot().failed, false);
    assert.equal(calls.length, 1);
    return;
  }
  if (name === "encoded-nonquery") {
    const paths = [];
    const server = http.createServer((request, response) => {
      paths.push(request.url);
      response.end("synthetic");
    });
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    const reports = [];
    const { guard } = configure((r) => reports.push(r), server.address().port);
    const path = "/results/chunk?token=100%25&opaque=%2525&other=%71uery-request";
    try {
      if (variant === "options")
        await nativeResponse(
          http.request({ ...options(path), method: "GET", port: server.address().port }),
        );
      else
        await nativeResponse(
          http.request(new URL(`http://127.0.0.1:${server.address().port}${path}`)),
        );
      assert.deepEqual(paths, [path]);
      assert.equal(guard.snapshot().failed, false);
      assert.equal(guard.snapshot().admitted, 0);
      assert.deepEqual(reports, []);
    } finally {
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    }
    return;
  }
  if (name === "loopback" || name === "redirect" || name === "native-undefined-redirect") {
    let received = 0;
    const server = http.createServer((request, response) => {
      received++;
      if (name !== "loopback") {
        const [status, target] = (variant ?? "302").split("-");
        let location = `http://127.0.0.1:${server.address().port}/changed`;
        if (target === "authority")
          location = `http://127.0.0.1:${destination.address().port}${endpoint}`;
        if (target === "protocol")
          location = `https://127.0.0.1:${destination.address().port}${endpoint}`;
        response.writeHead(Number(status), { Location: location });
      }
      response.end("synthetic");
    });
    let destinationReceived = 0;
    const destination = http.createServer((request, response) => {
      destinationReceived++;
      response.end();
    });
    destination.listen(0, "127.0.0.1");
    await once(destination, "listening");
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    const reports = [];
    const { guard, ticket } = configure((r) => reports.push(r), server.address().port);
    guard.registerRequestId(ticket, A);
    try {
      if (name === "loopback") {
        const opts = { ...options(), port: server.address().port };
        await nativeResponse(http.request(opts));
        assert.equal(received, 1);
        fixedError(() => http.request(opts));
        assert.equal(received, 1);
      } else if (name === "native-undefined-redirect") {
        await nativeResponse(
          http.request({ ...options(), port: server.address().port }, undefined),
        );
        assert.equal(reports[0].category, "redirect");
        assert.equal(guard.snapshot().failed, true);
      } else {
        // Import only after installing the native wrappers, as production must.
        const redirects = require("follow-redirects");
        const opts = { ...options(), port: server.address().port };
        let callbacks = 0,
          latchObserved = false;
        const request = redirects.http.request(opts, () => {
          callbacks++;
        });
        // A deliberately prepended listener can run first. The guard must
        // still latch before delegating to follow-redirects' original callback.
        request._currentRequest.prependListener("response", () => {
          latchObserved = guard.snapshot().failed;
        });
        const error = once(request, "error");
        request.end();
        const [failure] = await error;
        assert.equal(failure.code, "ERR_FR_TOO_MANY_REDIRECTS");
        assert.equal(request._options.maxRedirects, 0);
        assert.equal(received, 1);
        assert.equal(callbacks, 0);
        // prependListener can precede a callback: check its explicit ordering
        // below, rather than making an unsupported event-emitter priority claim.
        assert.equal(latchObserved, false);
        assert.equal(reports[0].category, "redirect");
        assert.equal(guard.snapshot().failed, true);
      }
    } finally {
      assert.equal(destinationReceived, 0);
      server.closeAllConnections();
      destination.closeAllConnections();
      await Promise.all([
        new Promise((resolve) => server.close(resolve)),
        new Promise((resolve) => destination.close(resolve)),
      ]);
    }
    return;
  }
  if (name === "redirect-callback-order") {
    const calls = fakeNative();
    let reentrantRejected = false;
    const { guard, ticket } = configure(() => {
      fixedError(() => http.request(options()));
      reentrantRejected = true;
    });
    guard.registerRequestId(ticket, A);
    let observed;
    http.request(options(), () => {
      observed = guard.snapshot().failed;
    });
    calls[0].args.at(-1)({ statusCode: 302, headers: { location: "synthetic-secret" } });
    assert.equal(observed, true);
    assert.equal(reentrantRejected, true);
    assert.equal(calls.length, 1);
    fixedError(() => http.request(options()));
    return;
  }
  throw new Error("Unknown synthetic scenario");
}

if (process.argv[2] === "guard-child") {
  await scenario(process.argv[3], process.argv[4]);
} else {
  const cases = [
    ...[
      "options",
      "options-undefined",
      "string",
      "url",
      "url-options-undefined",
      "url-undefined-callback",
    ].map((v) => ["passthrough-and-forms", v]),
    ...[
      "encodedPath",
      "encodedQuestionPath",
      "encodedHashPath",
      "twiceEncodedPath",
      "casePath",
      "slashPath",
      "dotPath",
      "duplicateId",
      "encodedIdKey",
      "encodedId",
      "staleId",
      "badId",
      "foreignHost",
      "hostConflict",
      "hostHeader",
      "duplicateHostHeader",
      "wrongPort",
      "defaultPort",
      "agentPort",
      "defaultAgent",
      "protocol",
      "method",
      "socket",
      "auth",
      "extraParameter",
      "fragment",
      "rawUrlDot",
      "urlOwnPort",
      "accessor",
      "accessorHeaders",
      "frozen",
    ].map((v) => ["negative", v]),
    ...["unregistered", "invalidId", "duplicate", "reuse", "stale"].map((v) => ["lifecycle", v]),
    ["sdk-parameters"],
    ["native-throw"],
    ["capacity"],
    ...["retained", "legacy", "invalidLease", "releaseActive", "empty", "retired"].map((v) => [
      "reservation",
      v,
    ]),
    ["loopback"],
    ["encoded-nonquery", "options"],
    ["encoded-nonquery", "url"],
    ["redirect-callback-order"],
    ["native-undefined-redirect"],
    ["https-wrapper"],
    ["callback-throw"],
    ["immutable-authority"],
    ["redirect", "302-authority"],
    ["redirect", "307-protocol"],
    ...[301, 302, 303, 307, 308].map((v) => ["redirect", String(v)]),
  ];
  for (const [name, variant] of cases) {
    test(
      `submission guard: ${name}${variant ? ` / ${variant}` : ""}`,
      { timeout: 15_000 },
      async () => {
        const child = spawn(
          process.execPath,
          [ownFile, "guard-child", name, ...(variant ? [variant] : [])],
          { stdio: ["ignore", "pipe", "pipe"] },
        );
        let output = "",
          exceeded = false;
        for (const stream of [child.stdout, child.stderr])
          stream.on("data", (data) => {
            if (output.length + data.length > 32_768) {
              exceeded = true;
              child.kill("SIGKILL");
            } else output += data;
          });
        const timer = setTimeout(() => {
          exceeded = true;
          child.kill("SIGKILL");
        }, 10_000);
        try {
          const [code, signal] = await once(child, "close");
          assert.equal(exceeded, false, "bounded synthetic child");
          assert.equal(signal, null, output);
          assert.equal(code, 0, output);
        } finally {
          clearTimeout(timer);
          if (child.exitCode === null) child.kill("SIGKILL");
        }
      },
    );
  }
}
