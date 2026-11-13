"""Exercise real v6/v7 harness processes without accounts or database execution.

First preserve the v6 harness outside target/debug. This runner never builds,
copies binaries, edits user state, or manufactures a schema version. A probe is
available before the new harness exists; the full run requires both binaries.
"""

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import secrets
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


DEADLINE_SECONDS = 15
OWNERSHIP_ERROR = "History ownership is busy"
NEWER_SCHEMA_ERROR = "newer or unsupported Joinery version"
METADATA_TABLES = ("profiles", "history", "audit")
LOCAL_HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def sha256(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def binary_inventory(path):
    path = path.resolve(strict=True)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f"Harness is not executable: {path}")
    stat = path.stat()
    return {"path": str(path), "sha256": sha256(path),
            "sizeBytes": stat.st_size, "mtimeNs": stat.st_mtime_ns}


def read_connection(root):
    database = root / "joinery.sqlite"
    return sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=1)


def snapshot(root):
    """Read only fresh runner-owned metadata, including its physical schema."""
    with closing(read_connection(root)) as database:
        version = database.execute("PRAGMA user_version").fetchone()[0]
        schema = database.execute(
            "SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name"
        ).fetchall()
        tables = [row[1] for row in schema if row[0] == "table"]
        rows = {}
        for table in tables:
            quoted = '"' + table.replace('"', '""') + '"'
            values = database.execute(f"SELECT * FROM {quoted}").fetchall()
            rows[table] = sorted(values, key=repr)
    canonical = json.dumps({"schema": schema, "rows": rows}, sort_keys=True,
                           default=lambda value: {"bytesHex": value.hex()})
    return {"userVersion": version, "tables": tables,
            "rowCounts": {table: len(values) for table, values in rows.items()},
            "logicalSha256": hashlib.sha256(canonical.encode()).hexdigest(),
            "ownerFiles": sorted(path.name for path in (root / "history-owners").glob("*.lock"))}


def assert_version(state, version):
    assert state["userVersion"] == version, state
    for table in METADATA_TABLES:
        assert f"{table}_v{version}" in state["tables"], state
        other = 7 if version == 6 else 6
        assert f"{table}_v{other}" not in state["tables"], state
    if version == 7:
        assert "outcome_operations_v7" in state["tables"], state


class Harness:
    def __init__(self, binary, root, logs, label):
        self.label = label
        self.token = secrets.token_urlsafe(32)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            self.port = reservation.getsockname()[1]
        self.url = f"http://127.0.0.1:{self.port}"
        # No inherited Joinery fixture mode, account or synthetic SDK opt-in.
        env = {key: value for key, value in os.environ.items()
               if not key.startswith("JOINERY_")}
        env.update(JOINERY_TEST_BRIDGE="1", JOINERY_TEST_TOKEN=self.token,
                   JOINERY_TEST_DATA_ROOT=str(root), JOINERY_TEST_PORT=str(self.port))
        self.stdout_path = logs / f"{label}-stdout.log"
        self.stderr_path = logs / f"{label}-stderr.log"
        self.stdout = self.stdout_path.open("xb")
        self.stderr = self.stderr_path.open("xb")
        try:
            self.process = subprocess.Popen([str(binary)], env=env,
                                            stdout=self.stdout, stderr=self.stderr)
        except BaseException:
            self.stdout.close()
            self.stderr.close()
            raise
        self.pid = self.process.pid

    def bootstrap(self):
        request = urllib.request.Request(
            self.url + "/rpc/bootstrap", data=b"{}", method="POST",
            headers={"Authorization": "Bearer " + self.token,
                     "Content-Type": "application/json"})
        with LOCAL_HTTP.open(request, timeout=0.5) as response:
            assert response.status == 200
            envelope = json.load(response)
            assert isinstance(envelope, dict) and "error" not in envelope, envelope
            bootstrap = envelope["result"]
            assert isinstance(bootstrap, dict) and isinstance(bootstrap["supportedEngines"], list)
            return bootstrap

    def ready(self):
        deadline = time.monotonic() + DEADLINE_SECONDS
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise AssertionError(f"{self.label} exited before readiness: {self.stderr_tail()}")
            try:
                bootstrap = self.bootstrap()
                assert self.process.poll() is None
                assert bootstrap["profiles"] == [], "Fresh fixture unexpectedly has profiles"
                return
            except (urllib.error.URLError, TimeoutError, ConnectionError):
                time.sleep(0.025)
        raise AssertionError(f"{self.label} did not become ready")

    def rejected(self, expected, code):
        self.process.wait(timeout=DEADLINE_SECONDS)
        error = self.stderr_tail()
        assert self.process.returncode != 0, f"{self.label} unexpectedly exited successfully"
        assert expected in error, f"{self.label} failed for another reason: {error}"
        assert code in error, f"{self.label} did not report {code}: {error}"
        return {"pid": self.pid, "returnCode": self.process.returncode,
                "expectedDiagnostic": expected, "expectedCode": code, "stderrTail": error}

    def stderr_tail(self):
        self.stderr.flush()
        with self.stderr_path.open("rb") as source:
            source.seek(max(0, self.stderr_path.stat().st_size - 4096))
            return source.read().decode(errors="replace")

    def stop(self):
        # The only PID signalled is the Popen child owned by this instance.
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=5)
        self.stdout.close()
        self.stderr.close()


def run_case(old, new, directory):
    root = directory / "metadata"
    root.mkdir(mode=0o700)
    children = []
    result = {"directory": str(directory), "passed": False}
    try:
        old_child = Harness(old, root, directory, "old-idle")
        children.append(old_child)
        old_child.ready()
        before = snapshot(root)
        assert_version(before, 6)
        assert before["ownerFiles"], "The old Studio did not create an owner lease"
        assert before["rowCounts"]["history_owners_v6"] == 0
        assert before["rowCounts"]["history_v6"] == 0
        assert before["rowCounts"]["audit_v6"] == 0
        result.update(oldPid=old_child.pid, oldIdle=before)
        if new is None:
            result["passed"] = True
            return result

        # Hold a genuine writer without changing data. The migration must reject
        # the live idle owner before waiting for/acquiring this writer lock.
        with closing(sqlite3.connect(root / "joinery.sqlite", timeout=1)) as writer:
            writer.execute("BEGIN IMMEDIATE")
            candidate = Harness(new, root, directory, "new-blocked")
            children.append(candidate)
            result["blockedMigration"] = candidate.rejected(OWNERSHIP_ERROR, "Conflict")
            assert old_child.process.poll() is None
            after_blocked = snapshot(root)
            assert after_blocked == before, "Rejected migration changed old metadata"
            result["unchangedWhileWriterHeld"] = after_blocked
            writer.rollback()
        old_child.bootstrap()  # Authenticated old Studio still works.
        old_child.stop()
        result["oldOwnedChildKilled"] = old_child.process.returncode

        upgraded = Harness(new, root, directory, "new-migrated")
        children.append(upgraded)
        upgraded.ready()
        after = snapshot(root)
        assert_version(after, 7)
        result["newPid"] = upgraded.pid
        result["migrated"] = after
        upgraded.stop()
        before_old_reopen = snapshot(root)

        stale = Harness(old, root, directory, "old-rejected")
        children.append(stale)
        result["oldAgainstNew"] = stale.rejected(NEWER_SCHEMA_ERROR, "Unsupported")
        final = snapshot(root)
        assert final == before_old_reopen, "Old startup changed v7 metadata"
        assert_version(final, 7)
        result["unchangedAfterOldRejection"] = final
        result["passed"] = True
    except Exception as error:
        result["error"] = str(error)
    finally:
        for child in reversed(children):
            child.stop()
        result["ownedChildren"] = [
            {"label": child.label, "pid": child.pid, "returnCode": child.process.returncode}
            for child in children
        ]
        (directory / "receipt.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--old-harness", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--new-harness", type=Path)
    mode.add_argument("--probe-old", action="store_true")
    parser.add_argument("--runs", type=int, choices=range(1, 4), default=2)
    parser.add_argument("--output-dir", type=Path,
                        default=Path(".cache/verification/history-migration-v6"))
    args = parser.parse_args()
    old = binary_inventory(args.old_harness)
    new = binary_inventory(args.new_harness) if args.new_harness else None
    if new and old["sha256"] == new["sha256"]:
        parser.error("Old and new harnesses must be distinct preserved builds")
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    runs = []
    for _ in range(args.runs):
        directory = Path(tempfile.mkdtemp(prefix="probe-" if args.probe_old else "migration-", dir=output))
        row = run_case(Path(old["path"]), Path(new["path"]) if new else None, directory)
        runs.append(row)
        print(json.dumps({"passed": row["passed"], "directory": str(directory),
                          "error": row.get("error")}), flush=True)
        if not row["passed"]:
            break
    assert binary_inventory(args.old_harness) == old, "Preserved old harness changed"
    if new:
        assert binary_inventory(args.new_harness) == new, "New harness changed during verification"
    receipt = {
        "passed": len(runs) == args.runs and all(row["passed"] for row in runs),
        "mode": "old-schema-probe" if args.probe_old else "actual-v6-v7-migration",
        "oldHarness": old, "newHarness": new, "runs": runs,
        "runnerSha256": sha256(Path(__file__).resolve()),
        "scope": "Actual isolated harness processes; metadata startup/bootstrap only. No engine connection, authentication, database SQL dispatch or account access.",
        "delayedStartupBeforeOwnerClaim": {
            "status": "not-proved",
            "reason": "The preserved v6 harness has no exact post-Persistence/pre-owner-claim pause control. This runner does not manufacture PRAGMA versions or claim that race was exercised."},
    }
    path = output / ("old-probe.json" if args.probe_old else "migration.json")
    path.write_text(json.dumps(receipt, indent=2) + "\n")
    print(f"Evidence: {path}")
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
