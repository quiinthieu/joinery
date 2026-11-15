"""Exercise actual native close/reopen using only the hidden isolated test build."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time


VERIFY_CHECKS = (
    "recoveredLatestDraft", "preferencesRestored", "temporaryResultStorageRestored", "noRestoredSessions",
    "noRestoredResults", "noRestoredTransactions", "noSqlReplay",
    "rolledBack", "tableDropped",
)


def acceptance_errors(report, stage, return_code, timed_out, exit_epoch_ms):
    """Require process and workflow evidence, not merely a passing JS report."""
    errors = []
    if timed_out or return_code != 0:
        errors.append("Native process failed or exceeded its deadline.")
    if report.get("passed") is not True or report.get("recoveryStage") != stage:
        errors.append("Missing passing report for the requested recovery stage.")
    recovery = report.get("recovery", {})
    if not isinstance(recovery, dict):
        return errors + ["Malformed recovery evidence."]
    if stage == "setup":
        for key in ("baselinePersisted", "cancelKeptSession", "cancelKeptTransaction"):
            if recovery.get(key) is not True:
                errors.append(f"Missing setup check: {key}.")
        mutation = recovery.get("draftMutationEpochMs")
        # The writer debounces at 400 ms. Requiring observed process exit before
        # that deadline conservatively proves the pending write used close flush.
        if (isinstance(mutation, bool) or not isinstance(mutation, (float, int))
                or not 0 <= exit_epoch_ms - mutation < 400):
            errors.append("Process did not exit within the 400 ms pending-save window.")
    else:
        for key in VERIFY_CHECKS:
            if recovery.get(key) is not True:
                errors.append(f"Missing reopen check: {key}.")
        counts = recovery.get("invocationsBeforeObserver", {})
        if not isinstance(counts, dict) or any(
            type(counts.get(key)) is not int or counts[key] != 0
            for key in ("connect", "prepareExecution", "execute")
        ):
            errors.append("Reopened process invoked connection/execution before the observer.")
    return errors


def run_stage(binary, directory, stage, port):
    (directory / "options.json").write_text(json.dumps({
        "cleanupCycles": 10, "idleMs": 1000,
        "recoveryStage": stage, "fixturePort": port,
    }) + "\n")
    report_path = directory / "report.json"
    if report_path.exists():
        report_path.unlink()  # Only this runner's previous-stage output.
    env = os.environ.copy()
    env["JOINERY_NATIVE_SMOKE_ROOT"] = str(directory)
    started = time.monotonic()
    with (directory / f"{stage}-stdout.log").open("wb") as stdout:
        with (directory / f"{stage}-stderr.log").open("wb") as stderr:
            process = subprocess.Popen([str(binary)], env=env, stdout=stdout, stderr=stderr)
            try:
                # The recovery feature has a 90-second application watchdog;
                # leave room for its failure report and bounded process exit.
                while process.poll() is None and time.monotonic() - started < 95:
                    time.sleep(0.01)
                exit_epoch_ms = time.time() * 1000
                timed_out = process.poll() is None
            finally:
                if process.poll() is None:
                    process.kill()  # Only the child launched above.
                process.wait(timeout=5)
    try:
        report = json.loads(report_path.read_text())
        if not isinstance(report, dict):
            raise ValueError("report must be an object")
    except (OSError, ValueError) as error:
        report = {"passed": False, "error": f"Missing or invalid report: {error}"}
    errors = acceptance_errors(report, stage, process.returncode, timed_out, exit_epoch_ms)
    evidence = {
        "stage": stage, "report": report, "passed": not errors, "errors": errors,
        "returnCode": process.returncode, "runnerTimedOut": timed_out,
        "observedExitEpochMs": exit_epoch_ms,
        "processElapsedMs": round((time.monotonic() - started) * 1000),
    }
    mutation = report.get("recovery", {}).get("draftMutationEpochMs") if isinstance(report.get("recovery"), dict) else None
    if isinstance(mutation, (float, int)) and not isinstance(mutation, bool):
        evidence["pendingCloseElapsedMs"] = round(exit_epoch_ms - mutation, 2)
    (directory / f"runner-{stage}.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=int, default=2, choices=range(1, 4))
    parser.add_argument("--fixture-port", type=int, default=55432, choices=range(1024, 65536),
                        metavar="PORT", help="Synthetic local PostgreSQL fixture port")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent / ".cache" / "native-smoke"
    build = json.loads((root / "build.json").read_text())
    binary = Path(build["binary"]).resolve()
    if binary.parent != root.resolve() or hashlib.sha256(binary.read_bytes()).hexdigest() != build["sha256"]:
        raise SystemExit("Native test binary changed. Run pnpm desktop:smoke:build again.")
    runs = []
    for _ in range(args.runs):
        directory = Path(tempfile.mkdtemp(prefix="recovery-", dir=root))
        setup = run_stage(binary, directory, "setup", args.fixture_port)
        verify = None
        # Even a slow close should be followed by reopen/cleanup when setup
        # produced valid state. Keep the timing failure in the final result.
        if setup["report"].get("passed") is True and (directory / "report-setup.json").is_file():
            verify = run_stage(binary, directory, "verify", args.fixture_port)
        run = {"directory": str(directory), "setup": setup, "verify": verify,
               "passed": setup["passed"] and verify is not None and verify["passed"]}
        runs.append(run)
        print(json.dumps({"passed": run["passed"], "directory": str(directory),
                          "pendingCloseElapsedMs": setup.get("pendingCloseElapsedMs"),
                          "errors": setup["errors"] + (verify["errors"] if verify else ["Reopen not run."])}), flush=True)
        if verify is None:
            break  # Retain the isolated root and logs for diagnosis.
    evidence = {
        "build": build, "platform": platform.platform(),
        "scope": "Hidden native WebView, real window close confirmation/cancel, isolated workspace and in-memory secret store, and synthetic loopback PostgreSQL. Two fresh processes share only their owned workspace. Does not qualify normal installed bundle, OS credential persistence, other engines, or user workflows.",
        "runs": runs,
    }
    path = root / "recovery-runs.json"
    path.write_text(json.dumps(evidence, indent=2) + "\n")
    print(f"Detailed native recovery evidence: {path}")
    if any(not row["passed"] for row in runs):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
