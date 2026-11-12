import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const entry = fileURLToPath(new URL("./query-submit-guard-sdk-probe.mjs", import.meta.url));
const cases = [];
for (const transport of ["http", "tls", "proxy"]) {
  for (const scenario of [
    "success",
    "lost-ack",
    "renewal",
    "cancel-before",
    "cancel-during",
    "cancel-after",
    "stale-owner",
    "redirect-302",
    "redirect-307",
    "helper-success",
    "helper-lost-ack",
    "helper-ack-drain",
    "helper-database",
    "helper-transaction-begin",
    "helper-transaction-commit",
    "helper-transaction-rollback",
    "helper-transaction-lost-ack",
    "helper-transaction-ack-guard",
    "helper-transaction-database",
    "helper-dml-insert",
    "helper-dml-update",
    "helper-dml-delete",
    "helper-dml-lost-ack",
    "helper-dml-ack-guard",
    "helper-dml-database",
    "helper-dml-ack-drain",
    "helper-dml-ack-cancel",
    "helper-dml-ack-eof",
    "helper-ddl-create",
    "helper-ddl-add-column",
    "helper-ddl-drop",
    "helper-ddl-lost-ack",
    "helper-ddl-ack-guard",
    "helper-ddl-database",
    "helper-ddl-ack-drain",
    "helper-ddl-ack-cancel",
    "helper-ddl-ack-eof",
    "helper-ddl-close-before-ack",
    "helper-ddl-close-after-ack",
    "helper-switch-success",
    "helper-switch-lost-use",
    "helper-switch-readback-denied",
    "helper-switch-lost-readback",
    "helper-setup-database",
    "helper-setup-schema",
    "helper-setup-table",
    "helper-setup-read-context",
    "helper-setup-read-parameter",
    "helper-setup-lost-ack",
    "helper-setup-rejected",
    "helper-setup-ack-drain",
    "helper-setup-ack-guard",
    "helper-setup-ack-cancel",
    "helper-setup-ack-eof",
    "helper-setup-close-before-ack",
    "helper-setup-close-after-ack",
  ])
    cases.push([scenario, transport]);
}
for (const scenario of [
  "reset",
  "delayed-loss",
  "status-503",
  "status-429",
  "redirect-301",
  "redirect-303",
  "redirect-308",
  "mutation-no-guard",
])
  cases.push([scenario, "http"]);
cases.push(["untrusted-tls", "tls"], ["untrusted-tls", "proxy"]);
cases.push(["fixture-boundary", "http"]);
for (const transport of ["http", "tls", "proxy"])
  for (const scenario of [
    "reviewed-success",
    "reviewed-timeout-policy",
    "reviewed-context-partial",
    "reviewed-ack-drain",
    "reviewed-file-veto",
  ])
    cases.push([scenario, transport]);

async function bounded(promise, milliseconds) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("Owned child did not close")), milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
async function certificate(root) {
  const config = join(root, "openssl.cnf");
  await writeFile(
    config,
    "[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n[dn]\nCN=joinery-owned-loopback\n[ext]\nsubjectAltName=IP:127.0.0.1,DNS:localhost\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,digitalSignature,keyEncipherment,keyCertSign\nextendedKeyUsage=serverAuth\n",
    { mode: 0o600 },
  );
  const child = spawn(
    "/usr/bin/openssl",
    [
      "req",
      "-x509",
      "-newkey",
      "rsa:2048",
      "-nodes",
      "-days",
      "1",
      "-config",
      config,
      "-keyout",
      join(root, "tls-key.pem"),
      "-out",
      join(root, "tls-cert.pem"),
    ],
    { stdio: "ignore", env: { PATH: "/usr/bin:/bin" } },
  );
  const closed = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code) => resolve(code));
  });
  try {
    assert.equal(await bounded(closed, 8000), 0);
  } finally {
    if (child.exitCode === null) {
      child.kill("SIGKILL");
      await bounded(closed, 3000);
    }
  }
  await chmod(join(root, "tls-key.pem"), 0o600);
}

for (const [scenario, transport] of cases) {
  test(
    `actual SDK submission guard: ${transport} / ${scenario}`,
    { timeout: 40_000 },
    async (t) => {
      const root = await mkdtemp(join(tmpdir(), "joinery-guard-sdk-"));
      let child,
        closed,
        observedClose = false,
        certificateSettled = transport === "http",
        deadline,
        exceeded = false,
        overflow = false;
      let stdout = "",
        stderr = "",
        spawnError;
      try {
        await chmod(root, 0o700);
        if (transport !== "http") await certificate(root);
        certificateSettled = true;
        child = spawn(process.execPath, [entry, scenario, transport], {
          cwd: root,
          stdio: ["ignore", "pipe", "pipe"],
          env: {
            JOINERY_GUARD_ROOT: root,
            SNOWFLAKE_HOME: join(root, "disabled-home"),
            SNOWFLAKE_DISABLE_PLATFORM_DETECTION: "true",
            SF_OCSP_RESPONSE_CACHE_DIR: join(root, "ocsp"),
            SNOWFLAKE_CRL_ON_DISK_CACHE_DIR: join(root, "crls"),
            ...(transport !== "http" && scenario !== "untrusted-tls"
              ? { NODE_EXTRA_CA_CERTS: join(root, "tls-cert.pem") }
              : {}),
          },
        });
        closed = new Promise((resolve) => {
          child.once("error", (error) => {
            spawnError = error;
          });
          child.once("close", (code, signal) => {
            observedClose = true;
            resolve({ code, signal });
          });
        });
        for (const [stream, name] of [
          [child.stdout, "stdout"],
          [child.stderr, "stderr"],
        ])
          stream.on("data", (chunk) => {
            if ((name === "stdout" ? stdout.length : stderr.length) + chunk.length > 32_768) {
              overflow = true;
              child.kill("SIGKILL");
            } else if (name === "stdout") stdout += chunk.toString();
            else stderr += chunk.toString();
          });
        deadline = setTimeout(() => {
          exceeded = true;
          child.kill("SIGKILL");
        }, 23_000);
        const status = await bounded(closed, 26_000);
        clearTimeout(deadline);
        assert.equal(spawnError, undefined);
        assert.equal(exceeded, false, "bounded owned SDK child");
        assert.equal(overflow, false, "bounded child output");
        assert.equal(stderr, "");
        const result = JSON.parse(stdout);
        assert.equal(status.signal, null);
        assert.equal(status.code, 0, JSON.stringify(result));
        assert.equal(result.sourcePins, true);
        assert.equal(result.listenersClosed, true);
        if (scenario === "fixture-boundary") {
          assert.equal(result.boundaryVerified, true);
          assert.equal(result.dnsDenied, result.boundaryDNSChecks);
          assert(result.boundaryDNSChecks >= 60);
          assert.equal(result.networkDenied, 7);
          assert.equal(result.connected, false);
          assert.equal(result.queryPosts, 0);
          return;
        }
        assert.equal(result.networkDenied, 0);
        assert.equal(result.dnsDenied, 0);
        assert.equal(result.destinationRequests, 0);
        assert.equal(result.destinationConnections, 0);
        if (scenario === "untrusted-tls") {
          assert.equal(result.connected, false);
          assert.equal(result.outcome, "tls-untrusted-rejected");
          assert.equal(result.tlsUntrustedRejected, true);
          assert.equal(result.tlsVerified, false);
          assert.equal(result.queryPosts, 0);
          return;
        }
        assert.equal(result.connected, true);
        assert.equal(result.intendedSQLOnly, true);
        assert.equal(result.scalarBindsExact, true);
        if (scenario.startsWith("reviewed-")) {
          assert.equal(result.helperFixturePreconnected, true);
          assert.equal(result.externalBrowserConnectSkipped, true);
          assert.equal(result.violations, 0);
          if (scenario === "reviewed-file-veto") {
            assert.equal(result.queryPosts, 0);
            assert.equal(result.guardAdmitted, 0);
            assert.equal(result.vetoCases, 4);
            assert.equal(result.helperTerminals, 4);
          } else {
            if (scenario === "reviewed-timeout-policy") {
              assert.equal(result.queryTimeoutObservationLayer, "follow-redirects-request");
              assert.deepEqual(result.observedQueryTimeouts, [125000]);
            }
            assert.equal(result.queryPosts, 1);
            assert.equal(result.guardAdmitted, 1);
            assert.equal(result.singleStatementParameter, true);
            assert.equal(result.originalAck, true);
            assert.equal(result.ackBeforeTerminal, true);
            assert.equal(result.ackIdentityRegistered, true);
            assert.equal(result.helperTerminals, 1);
            assert.deepEqual(result.passiveContext, {
              database: "RESPONSE_DB",
              schema: "RESPONSE_SCHEMA",
              role: scenario === "reviewed-context-partial" ? null : "RESPONSE_ROLE",
              warehouse: scenario === "reviewed-context-partial" ? null : "RESPONSE_WH",
            });
            assert.equal(
              result.helperTerminal,
              scenario === "reviewed-ack-drain" ? "connectionClosed" : "success",
            );
            assert.equal(result.rows, scenario === "reviewed-ack-drain" ? 0 : 1);
          }
          return;
        }
        if (scenario.startsWith("helper-setup-")) {
          assert.equal(result.helperFixturePreconnected, true);
          assert.equal(result.externalBrowserConnectSkipped, true);
          assert.equal(result.setupCursorFixtureSeeded, true);
          assert.equal(result.queryPosts, 1);
          const readStep = scenario.startsWith("helper-setup-read-");
          assert.equal(result.effects, readStep || scenario === "helper-setup-rejected" ? 0 : 1);
          assert.equal(result.guardAdmitted, 1);
          assert.equal(result.helperRetired, 1);
          assert.equal(result.rows, 0);
          assert.equal(result.publicColumns, 0);
          const expected =
            {
              "helper-setup-lost-ack": "outcomeUnknown",
              "helper-setup-rejected": "database",
              "helper-setup-ack-drain": "connectionClosed",
              "helper-setup-ack-guard": "connectionClosed",
              "helper-setup-ack-cancel": "connectionClosed",
              "helper-setup-ack-eof": "localEOF",
              "helper-setup-close-before-ack": "outcomeUnknown",
              "helper-setup-close-after-ack": "connectionClosed",
            }[scenario] ?? "success";
          assert.equal(result.helperTerminal, expected);
          assert.equal(result.helperTerminals, scenario === "helper-setup-ack-eof" ? 0 : 1);
          assert.equal(
            result.helperSetupAcknowledged,
            !readStep &&
              ![
                "helper-setup-lost-ack",
                "helper-setup-rejected",
                "helper-setup-close-before-ack",
              ].includes(scenario),
          );
          if (result.helperSetupAcknowledged) {
            assert.equal(result.setupAckBeforeTerminal, true);
            assert.equal(result.setupAckRegistered, true);
          }
          if (expected === "success") assert.equal(result.privateSetupResultExact, true);
          if (scenario === "helper-setup-ack-drain") assert.equal(result.resultChunkRequests, 1);
          if (scenario === "helper-setup-ack-cancel")
            assert.equal(result.helperCancelRequested, true);
          if (scenario.startsWith("helper-setup-close-"))
            assert.equal(result.helperExplicitCloseRequested, true);
          assert.equal(
            result.violations,
            ["helper-setup-lost-ack", "helper-setup-ack-guard"].includes(scenario) ? 1 : 0,
          );
          if (transport !== "http") assert.equal(result.tlsVerified, true);
          if (transport === "proxy" && scenario === "helper-setup-lost-ack")
            assert.equal(result.proxyConnectsAtViolation, result.firstQueryProxyConnects);
          t.diagnostic(
            JSON.stringify({
              scenario,
              transport,
              posts: result.queryPosts,
              effects: result.effects,
              outcome: result.outcome,
            }),
          );
          return;
        }
        const switchScenario = scenario.startsWith("helper-switch-");
        const transactionScenario = scenario.startsWith("helper-transaction-");
        const dmlScenario = scenario.startsWith("helper-dml-");
        const ddlScenario = scenario.startsWith("helper-ddl-");
        assert.equal(
          result.queryPosts,
          switchScenario && scenario !== "helper-switch-lost-use"
            ? 2
            : scenario === "cancel-before"
              ? 0
              : scenario === "mutation-no-guard"
                ? 2
                : 1,
        );
        const effectful = new Set([
          "success",
          "lost-ack",
          "delayed-loss",
          "cancel-during",
          "cancel-after",
          "stale-owner",
          "helper-success",
          "helper-lost-ack",
          "helper-ack-drain",
        ]);
        assert.equal(
          result.effects,
          dmlScenario
            ? scenario === "helper-dml-database"
              ? 0
              : 1
            : ddlScenario
              ? scenario === "helper-ddl-database"
                ? 0
                : 1
              : transactionScenario
                ? scenario === "helper-transaction-database"
                  ? 0
                  : 1
                : switchScenario
                  ? 1
                  : scenario === "mutation-no-guard"
                    ? 2
                    : effectful.has(scenario)
                      ? 1
                      : 0,
        );
        if (scenario.startsWith("helper-")) {
          const expected = {
            "helper-success": "success",
            "helper-lost-ack": "outcomeUnknown",
            "helper-ack-drain": "connectionClosed",
            "helper-database": "database",
            "helper-transaction-begin": "success",
            "helper-transaction-commit": "success",
            "helper-transaction-rollback": "success",
            "helper-transaction-lost-ack": "outcomeUnknown",
            "helper-transaction-ack-guard": "connectionClosed",
            "helper-transaction-database": "database",
            "helper-dml-insert": "success",
            "helper-dml-update": "success",
            "helper-dml-delete": "success",
            "helper-dml-lost-ack": "outcomeUnknown",
            "helper-dml-ack-guard": "connectionClosed",
            "helper-dml-database": "database",
            "helper-dml-ack-drain": "connectionClosed",
            "helper-dml-ack-cancel": "connectionClosed",
            "helper-dml-ack-eof": "localEOF",
            "helper-ddl-create": "success",
            "helper-ddl-add-column": "success",
            "helper-ddl-drop": "success",
            "helper-ddl-lost-ack": "outcomeUnknown",
            "helper-ddl-ack-guard": "connectionClosed",
            "helper-ddl-database": "database",
            "helper-ddl-ack-drain": "connectionClosed",
            "helper-ddl-ack-cancel": "connectionClosed",
            "helper-ddl-ack-eof": "localEOF",
            "helper-ddl-close-before-ack": "outcomeUnknown",
            "helper-ddl-close-after-ack": "connectionClosed",
            "helper-switch-success": "success",
            "helper-switch-lost-use": "outcomeUnknown",
            "helper-switch-readback-denied": "connectionClosed",
            "helper-switch-lost-readback": "connectionClosed",
          }[scenario];
          assert.equal(result.helperFixturePreconnected, true);
          assert.equal(result.externalBrowserConnectSkipped, true);
          assert.equal(result.helperTerminal, expected);
          assert.equal(
            result.helperTerminals,
            ["helper-dml-ack-eof", "helper-ddl-ack-eof"].includes(scenario) ? 0 : 1,
          );
          assert.equal(result.helperRetired, 1);
          assert.equal(result.outcome, `helper-${expected}`);
          const violation = dmlScenario
            ? ["helper-dml-lost-ack", "helper-dml-ack-guard", "helper-dml-database"].includes(
                scenario,
              )
            : ddlScenario
              ? ["helper-ddl-lost-ack", "helper-ddl-ack-guard", "helper-ddl-database"].includes(
                  scenario,
                )
              : transactionScenario
                ? ["helper-transaction-lost-ack", "helper-transaction-ack-guard"].includes(scenario)
                : switchScenario
                  ? scenario.includes("lost-")
                  : scenario !== "helper-success";
          assert.equal(result.violations, violation ? 1 : 0);
          if (violation) assert.equal(result.helperSettledSynchronously, true);
          if (transactionScenario)
            assert.equal(
              result.helperBoundaryAcknowledged === true,
              !["helper-transaction-lost-ack", "helper-transaction-database"].includes(scenario),
            );
          if (dmlScenario) {
            assert.equal(
              result.helperDmlAcknowledged,
              !["helper-dml-lost-ack", "helper-dml-database"].includes(scenario),
            );
            assert.equal(result.rows, 0);
            assert.equal(result.publicColumns, 0);
            assert.equal(result.guardAdmitted, 1);
            if (result.helperDmlAcknowledged) {
              assert.equal(result.dmlAckBeforeTerminal, true);
              assert.equal(result.dmlAckRegistered, true);
            }
            if (expected === "success") assert.equal(result.privateDmlResultExact, true);
            if (scenario === "helper-dml-ack-drain") assert.equal(result.resultChunkRequests, 1);
            if (scenario === "helper-dml-ack-cancel")
              assert.equal(result.helperCancelRequested, true);
            if (scenario === "helper-dml-database")
              assert.equal(result.helperReentrantFailure, true);
          }
          if (ddlScenario) {
            assert.equal(
              result.helperDdlAcknowledged,
              ![
                "helper-ddl-lost-ack",
                "helper-ddl-database",
                "helper-ddl-close-before-ack",
              ].includes(scenario),
            );
            assert.equal(result.rows, 0);
            assert.equal(result.publicColumns, 0);
            assert.equal(result.guardAdmitted, 1);
            if (result.helperDdlAcknowledged) {
              assert.equal(result.ddlAckBeforeTerminal, true);
              assert.equal(result.ddlAckRegistered, true);
            }
            if (expected === "success") assert.equal(result.privateDdlResultExact, true);
            if (scenario === "helper-ddl-ack-drain") assert.equal(result.resultChunkRequests, 1);
            if (scenario === "helper-ddl-ack-cancel")
              assert.equal(result.helperCancelRequested, true);
            if (scenario === "helper-ddl-database")
              assert.equal(result.helperReentrantFailure, true);
            if (scenario.startsWith("helper-ddl-close-"))
              assert.equal(result.helperExplicitCloseRequested, true);
          }
          if (switchScenario) {
            assert.equal(result.helperUseAcknowledged, scenario !== "helper-switch-lost-use");
            assert.equal(result.helperAcknowledged, expected === "success");
            assert.equal(result.switchIdsDistinct, true);
            assert.equal(result.guardAdmitted, result.queryPosts);
            if (expected === "success") {
              assert.equal(result.switchSchemaMatches, true);
              assert.equal(result.switchRoleNull, true);
            }
          }
          if (scenario === "helper-ack-drain") assert.equal(result.helperAcknowledged, true);
          if (scenario === "helper-database") assert.equal(result.helperReentrantFailure, true);
        } else if (scenario === "success" || scenario === "mutation-no-guard") {
          assert.equal(result.outcome, "success");
          assert.equal(result.rows, 1);
          assert.equal(result.violations, 0);
          assert.equal(result.effects, scenario === "mutation-no-guard" ? 2 : 1);
          if (scenario === "mutation-no-guard") assert.equal(result.queryIdsSame, true);
        } else {
          assert.equal(result.violations, 1);
          assert.equal(result.guardFailed, true);
          assert.equal(
            result.outcome,
            scenario === "cancel-after" ? "success" : "harness-OutcomeUnknown",
          );
          assert.equal(
            result.violationCategory,
            scenario.startsWith("redirect-")
              ? "redirect"
              : ["cancel-before", "cancel-after", "stale-owner"].includes(scenario)
                ? "unowned"
                : "duplicate",
          );
          assert(result.violationLatencyMs < 12_000);
        }
        if (["cancel-after", "stale-owner"].includes(scenario))
          assert.equal(result.knownSuccesses, 1);
        if (scenario.startsWith("cancel-")) assert.equal(result.cancelAcknowledged, true);
        if (scenario === "renewal") assert.equal(result.renewalRequests, 1);
        if (transport === "proxy") {
          assert(result.proxyConnects >= 1);
          if (
            ["lost-ack", "helper-lost-ack", "helper-dml-lost-ack", "helper-ddl-lost-ack"].includes(
              scenario,
            ) ||
            scenario.startsWith("redirect-")
          )
            assert.equal(result.proxyConnectsAtViolation, result.firstQueryProxyConnects);
        }
        if (transport !== "http") assert.equal(result.tlsVerified, true);
        // Persist only the same bounded counters/categories while root is owned;
        // it is removed only after this child has actually closed.
        await writeFile(join(root, "evidence.json"), JSON.stringify(result), { mode: 0o600 });
        t.diagnostic(
          JSON.stringify({
            scenario,
            transport,
            posts: result.queryPosts,
            effects: result.effects,
            violations: result.violations,
            outcome: result.outcome,
          }),
        );
      } finally {
        clearTimeout(deadline);
        if (child && !observedClose) {
          child.kill("SIGKILL");
          await bounded(closed, 4000);
        }
        // Retain the root if either owned subprocess lacks a cleanup receipt.
        if (certificateSettled && (!child || observedClose))
          await rm(root, { recursive: true, force: true });
      }
    },
  );
}
