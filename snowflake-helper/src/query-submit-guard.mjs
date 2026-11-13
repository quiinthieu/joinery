// Original, process-local admission around the reviewed native HTTP/1 transport.
// Install before importing the SDK/follow-redirects. This is neither a network
// sandbox nor proof of a remote effect/rollback. No URL, ID or error is logged.
import http from "node:http";
import https from "node:https";
import { syncBuiltinESMExports } from "node:module";
import { URL, urlToHttpOptions } from "node:url";

const ENDPOINT = "/queries/v1/query-request";
const ID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const MAX_IDS = 65_536;
const MAX_TARGET = 16_384;
let installed;

function denied() {
  const error = new Error("Snowflake query submission admission is closed");
  error.code = "JOINERY_QUERY_SUBMIT_GUARD";
  return error;
}

function shape() {
  throw denied();
}

function record(value) {
  if (!value || typeof value !== "object") shape();
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) shape();
  // Inspecting a getter and then letting native request inspect it again would
  // permit a different target at admission and dispatch. Pinned options are data.
  const descriptors = Object.getOwnPropertyDescriptors(value);
  if (Object.values(descriptors).some((d) => d.get || d.set)) shape();
  return value;
}

function text(value, max = MAX_TARGET) {
  if (typeof value !== "string" || value.length > max || /[\u0000-\u0020\u007f]/.test(value))
    shape();
  return value;
}

function authority(protocol, hostname, port) {
  if (protocol !== "http:" && protocol !== "https:") shape();
  text(hostname, 253);
  if (!hostname || /[^a-z0-9.:[\]-]/i.test(hostname)) shape();
  const defaultPort = protocol === "https:" ? 443 : 80;
  if (port === undefined || port === "") port = defaultPort;
  if (typeof port === "string") {
    if (!/^[1-9][0-9]{0,4}$/.test(port)) shape();
    port = Number(port);
  }
  if (!Number.isInteger(port) || port < 1 || port > 65535) shape();
  const bracketed =
    hostname.includes(":") && !hostname.startsWith("[") ? `[${hostname}]` : hostname;
  const url = new URL(`${protocol}//${bracketed}:${port}/`);
  if (url.username || url.password || url.pathname !== "/" || url.search || url.hash) shape();
  return { protocol, hostname: url.hostname.toLowerCase(), port };
}

function dataProperty(object, key) {
  for (
    let depth = 0;
    object != null && depth < 16;
    depth++, object = Object.getPrototypeOf(object)
  ) {
    const descriptor = Object.getOwnPropertyDescriptor(object, key);
    if (descriptor) {
      if (descriptor.get || descriptor.set) shape();
      return descriptor.value;
    }
  }
  if (object != null) shape();
}

function effectivePort(options, protocol) {
  // Match pinned Node's native port selection, including agent defaults. Reading
  // a getter twice would make the checked and dispatched authorities differ.
  if (options._defaultAgent !== undefined) shape();
  if (options.port) return options.port;
  if (options.defaultPort) return options.defaultPort;
  let agent = options.agent;
  if (agent === false) return protocol === "https:" ? 443 : 80;
  if (agent == null && typeof options.createConnection !== "function")
    agent = protocol === "https:" ? https.globalAgent : http.globalAgent;
  return (agent && dataProperty(agent, "defaultPort")) || 80;
}

function equalAuthority(a, b) {
  return a.protocol === b.protocol && a.hostname === b.hostname && a.port === b.port;
}

function normalizedArguments(args, protocol) {
  if (args.length < 1 || args.length > 3) shape();
  const first = args[0];
  let options, mutable, rawUrl;
  if (typeof first === "string" || first instanceof URL) {
    if (
      typeof first !== "string" &&
      (Object.getPrototypeOf(first) !== URL.prototype || Reflect.ownKeys(first).length !== 0)
    )
      shape();
    // Pinned Node reads URL public properties and spreads own properties again.
    // A data override on an otherwise genuine URL can change its dispatched
    // authority; only the unmodified built-in URL shape is reviewed.
    rawUrl = typeof first === "string" ? text(first) : URL.prototype.toString.call(first);
    const url = new URL(rawUrl);
    options = urlToHttpOptions(url);
    if (typeof args[1] === "object" && args[1] !== null) {
      mutable = record(args[1]);
      options = { ...options, ...mutable };
      if (args.length === 3 && args[2] !== undefined && typeof args[2] !== "function") shape();
    } else {
      if (args[1] !== undefined && typeof args[1] !== "function") shape();
      if (
        args.length === 3 &&
        (args[1] !== undefined || (args[2] !== undefined && typeof args[2] !== "function"))
      )
        shape();
    }
  } else {
    mutable = record(first);
    options = mutable;
    if (args.length > 2 || (args[1] !== undefined && typeof args[1] !== "function")) shape();
  }
  const callbackIndex = args.findIndex((arg, index) => index > 0 && typeof arg === "function");
  const path = options.path === undefined ? "/" : text(options.path);
  return { options, mutable, path, rawUrl, callbackIndex, protocol };
}

function originalUrlPath(rawUrl) {
  return /^[a-z][a-z0-9+.-]*:\/\/[^/?#]*(\/[^?#]*)?/i.exec(rawUrl)?.[1] ?? "/";
}

function queryCandidate(path, rawUrl) {
  // Split only literal delimiters BEFORE decoding. Query values are opaque and
  // may legitimately contain encoded percent signs (including signed results).
  // Keep decoded ?/# inside the candidate text: they are ambiguous path aliases,
  // not permission to hide an encoded endpoint beyond a newly decoded delimiter.
  const paths = [path.split(/[?#]/, 1)[0]];
  if (rawUrl !== undefined) paths.push(originalUrlPath(rawUrl));
  for (const initial of paths) {
    let value = initial;
    for (let i = 0; i < 8; i++) {
      if (value.replaceAll("\\", "/").toLowerCase().includes("query-request")) return true;
      if (!value.includes("%")) break;
      let decoded;
      try {
        decoded = decodeURIComponent(value);
      } catch {
        shape();
      }
      if (decoded === value) break;
      value = decoded;
      // Do not accept arbitrarily deep encodings outside the reviewed shapes.
      if (i === 7 && value.includes("%")) shape();
    }
  }
  return false;
}

function requestIdentity(description) {
  const { options, path, protocol, rawUrl } = description;
  if (rawUrl !== undefined) {
    // WHATWG parsing removes dot segments. Require the original URL spelling as
    // well as the effective native path; accepting its normalized alias would
    // obscure which endpoint was admitted.
    const rawPath = originalUrlPath(rawUrl);
    if (rawPath !== ENDPOINT || rawUrl.includes("#")) shape();
  }
  if (options.protocol !== undefined && options.protocol !== protocol) shape();
  if (options.socketPath !== undefined || options.auth !== undefined || options.hash) shape();
  const host = options.hostname ?? options.host ?? "localhost";
  const port = effectivePort(options, protocol);
  const target = authority(protocol, host, port);
  if (options.hostname !== undefined && options.host !== undefined) {
    if (!equalAuthority(target, authority(protocol, options.host, port))) shape();
  }
  if (options.servername !== undefined && options.servername.toLowerCase() !== target.hostname)
    shape();
  if (options.method !== "POST") shape();
  const question = path.indexOf("?");
  if (question < 0 || path.slice(0, question) !== ENDPOINT || path.includes("#")) shape();
  const query = path.slice(question + 1);
  if (!query || query.length > 2048) shape();
  const parameters = new Map();
  for (const pair of query.split("&")) {
    const match = /^([A-Za-z][A-Za-z0-9_]*)=([A-Za-z0-9-]+)$/.exec(pair);
    if (!match || parameters.has(match[1])) shape();
    const [, key, value] = match;
    if (key === "requestId" || key === "request_guid") {
      if (!ID.test(value)) shape();
    } else if (["clientStartTime", "retryCount", "retryReason"].includes(key)) {
      if (!/^[0-9]{1,20}$/.test(value)) shape();
    } else shape();
    parameters.set(key, value);
  }
  const id = parameters.get("requestId");
  if (!id) shape();
  const headers = options.headers;
  let hostHeaders = [];
  if (Array.isArray(headers)) {
    if (
      Object.getPrototypeOf(headers) !== Array.prototype ||
      Object.values(Object.getOwnPropertyDescriptors(headers)).some((d) => d.get || d.set)
    )
      shape();
    if (headers.length % 2) shape();
    for (let i = 0; i < headers.length; i += 2) {
      if (typeof headers[i] !== "string") shape();
      if (headers[i].toLowerCase() === "host") hostHeaders.push(headers[i + 1]);
    }
  } else if (headers !== undefined) {
    record(headers);
    hostHeaders = Object.entries(headers)
      .filter(([key]) => key.toLowerCase() === "host")
      .map(([, value]) => value);
  }
  if (hostHeaders.length > 1) shape();
  if (hostHeaders.length) {
    const header = text(hostHeaders[0], 300);
    const url = new URL(`${protocol}//${header}/`);
    if (
      url.username ||
      url.password ||
      url.pathname !== "/" ||
      url.search ||
      url.hash ||
      !equalAuthority(target, authority(protocol, url.hostname, url.port))
    )
      shape();
  }
  return { target, id };
}

/**
 * Install once, before SDK/dependency import. Every execute, including private
 * reads and each individual context statement, needs a ticket before execute()
 * and its exact SDK-generated ID registered synchronously after execute returns.
 * Keep the ticket through original acknowledgement, drain and terminal decision.
 * Lifecycle handles never contain request IDs or caller-selected generations.
 * 65,536 distinct registrations is a fail-closed helper lifetime ceiling.
 * Capacity leases reserve future registrations before an owned transaction
 * starts. Unused slots can be released; spent IDs can never be returned.
 */
export function installQuerySubmitGuard({ onViolation } = {}) {
  if (installed) return installed.rejectInstallation();
  if (typeof onViolation !== "function") throw denied();
  const originals = { "http:": http.request, "https:": https.request };
  let expected, operation, statement, failure;
  let registered = 0,
    admitted = 0,
    reserved = 0;
  const ids = new Set();
  const reservations = new WeakMap();
  const snapshot = () => Object.freeze({ failed: failure !== undefined, registered, admitted });
  const fail = (category) => {
    if (failure === undefined) {
      failure = category;
      try {
        onViolation(Object.freeze({ category, ...snapshot() }));
      } catch {
        // Caller exceptions cannot reopen admission or become diagnostics.
      }
    }
  };
  const reject = (category) => {
    fail(category);
    throw denied();
  };
  const healthy = () => {
    if (failure !== undefined) throw denied();
  };
  function wrap(protocol) {
    return function (...args) {
      let description, candidate;
      try {
        description = normalizedArguments(args, protocol);
        candidate = queryCandidate(description.path, description.rawUrl);
      } catch {
        return reject("requestShape");
      }
      if (!candidate) return Reflect.apply(originals[protocol], this, args);
      healthy();
      let identity;
      try {
        identity = requestIdentity(description);
      } catch {
        return reject("queryShape");
      }
      if (!expected || !equalAuthority(expected, identity.target)) return reject("authority");
      if (!operation || !statement || statement.id !== identity.id) return reject("unowned");
      if (statement.spent) return reject("duplicate");
      // Consumption precedes even option mutation and a throwing native call.
      statement.spent = true;
      admitted++;
      try {
        if (description.mutable) {
          // Pinned follow-redirects 1.16.3 retains this exact object. A clone
          // would leave its redirect allowance intact. Verify the write stuck.
          description.mutable.maxRedirects = 0;
          if (description.mutable.maxRedirects !== 0) shape();
        }
      } catch {
        return reject("redirectControl");
      }
      const forwarded = args.slice();
      const callback = description.callbackIndex < 0 ? undefined : args[description.callbackIndex];
      const observe = function (response, ...rest) {
        if (response?.statusCode >= 300 && response.statusCode < 400) fail("redirect");
        if (callback) return Reflect.apply(callback, this, [response, ...rest]);
      };
      if (description.callbackIndex < 0) {
        // Replace a trailing undefined callback slot rather than appending an
        // ignored third argument to request(options, undefined).
        if (forwarded.length > 1 && forwarded.at(-1) === undefined)
          forwarded[forwarded.length - 1] = observe;
        else forwarded.push(observe);
      } else forwarded[description.callbackIndex] = observe;
      try {
        return Reflect.apply(originals[protocol], this, forwarded);
      } catch {
        return reject("nativeThrow");
      }
    };
  }
  const guard = Object.freeze({
    setAuthority(value) {
      healthy();
      if (expected || operation) return reject("authoritySetup");
      try {
        record(value);
        if (Object.keys(value).some((key) => !["protocol", "hostname", "port"].includes(key)))
          shape();
        expected = authority(value.protocol, value.hostname, value.port);
      } catch {
        return reject("authoritySetup");
      }
    },
    beginOperation() {
      healthy();
      if (!expected || operation) return reject("lifecycle");
      operation = Object.freeze({});
      return operation;
    },
    reserveCapacity(count) {
      healthy();
      if (!expected || statement || !Number.isSafeInteger(count) || count < 1 || count > MAX_IDS)
        return reject("reservation");
      // Ordinary capacity refusal is admission failure, not a guard violation:
      // an existing manual owner must retain its reserved rollback capacity.
      if (count > MAX_IDS - registered - reserved) return null;
      const lease = Object.freeze({});
      reservations.set(lease, count);
      reserved += count;
      return lease;
    },
    releaseCapacity(lease) {
      // Cleanup is permitted after retirement, but cannot revoke a live ticket.
      if (!reservations.has(lease) || (statement && statement.lease === lease))
        return reject("reservation");
      reserved -= reservations.get(lease);
      reservations.delete(lease);
    },
    beginStatement(owner, lease) {
      healthy();
      if (!operation || operation !== owner || statement) return reject("lifecycle");
      if (lease !== undefined && (!reservations.has(lease) || reservations.get(lease) < 1))
        return reject("reservation");
      const ticket = Object.freeze({});
      statement = { ticket, id: undefined, spent: false, lease };
      return ticket;
    },
    registerRequestId(ticket, id) {
      healthy();
      if (!statement || statement.ticket !== ticket || statement.id !== undefined)
        return reject("lifecycle");
      if (typeof id !== "string" || !ID.test(id)) return reject("requestId");
      if (ids.has(id.toLowerCase())) return reject("reusedId");
      if (statement.lease === undefined) {
        if (registered + reserved === MAX_IDS) return reject("capacity");
      } else {
        const remaining = reservations.get(statement.lease);
        if (!remaining) return reject("reservation");
        reservations.set(statement.lease, remaining - 1);
        reserved--;
      }
      ids.add(id.toLowerCase());
      statement.id = id;
      registered++;
    },
    endStatement(ticket) {
      if (!statement || statement.ticket !== ticket) return reject("lifecycle");
      statement = undefined;
    },
    endOperation(owner) {
      if (!operation || operation !== owner || statement) return reject("lifecycle");
      operation = undefined;
    },
    snapshot,
  });
  installed = { rejectInstallation: () => reject("installation") };
  http.request = wrap("http:");
  https.request = wrap("https:");
  syncBuiltinESMExports();
  return guard;
}
