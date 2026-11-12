"""Run actual invoke-ACL encrypted recovery in the isolated hidden macOS app."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import stat
import subprocess
import tempfile
import time
import uuid


SETUP_CHECKS = (
    "closedReadEnrolled", "inventoryEnrolled", "deleteEnrolled", "lookupEnrolled",
    "cancelEnrolled", "reconcileEnrolled", "globalFenceEnrolled",
    "receiptInventoryEnrolled", "receiptSummaryEnrolled", "receiptPendingEnrolled",
    "receiptStorageEnrolled", "receiptArchiveEnrolled", "receiptReleaseEnrolled",
    "csvReleaseEnrolled", "createFromPreferences", "wrongKeyPreservedMetadata",
    "wrongKeyPreservedInventory", "restoreFromPreferences", "undoFromPreferences",
    "exactDeleteSucceeded", "closedAdmissionRejectsNewStart", "noSqlOrSessions",
)
VERIFY_CHECKS = (
    "groupRestored", "encryptedInventoryRetained", "freshProcessAdmissionOpen",
    "noRestoredSessionsOrResults", "noSqlReplay",
)
STAGE_TIMEOUTS = {"encryptedSetup": 930, "encryptedVerify": 150}


def canonical_uuid(value):
    try:
        return isinstance(value, str) and str(uuid.UUID(value)) == value
    except ValueError:
        return False


def acceptance_errors(report, stage, return_code, timed_out):
    errors = []
    if stage not in STAGE_TIMEOUTS:
        return ["Unknown encrypted recovery stage."]
    if timed_out or return_code != 0:
        errors.append("Native process failed or exceeded its bounded KDF deadline.")
    if not isinstance(report, dict):
        return errors + ["Malformed native report."]
    if report.get("passed") is not True or report.get("recoveryStage") != stage:
        errors.append("Missing passing report for the requested encrypted stage.")
    evidence = report.get("encryptedRecovery")
    if not isinstance(evidence, dict):
        return errors + ["Missing encrypted recovery evidence."]
    checks = evidence.get("checks")
    if not isinstance(checks, dict):
        return errors + ["Missing per-contract native checks."]
    for key in SETUP_CHECKS if stage == "encryptedSetup" else VERIFY_CHECKS:
        if checks.get(key) is not True:
            errors.append(f"Missing native check: {key}.")
    counts = evidence.get("invocations")
    if not isinstance(counts, dict) or any(
        type(counts.get(key)) is not int or counts[key] != 0
        for key in ("connect", "prepareExecution", "execute")
    ):
        errors.append("Encrypted recovery connected or dispatched SQL, or counts are missing.")
    if not canonical_uuid(evidence.get("groupId")):
        errors.append("Missing exact local group identity.")
    if stage == "encryptedSetup":
        ids = evidence.get("operationIds")
        if not isinstance(ids, list) or len(ids) != 4 or not all(map(canonical_uuid, ids)) or len(set(ids)) != 4:
            errors.append("Create/wrong-key/restore/undo must have four distinct original IDs.")
    backups = evidence.get("backups")
    if not isinstance(backups, list) or len(backups) != 2:
        errors.append("Missing exact retained original and undo copies.")
    else:
        seen = set()
        for backup in backups:
            metadata = backup.get("metadata") if isinstance(backup, dict) else None
            if (not isinstance(metadata, dict) or backup.get("format") != "encryptedV1"
                    or not canonical_uuid(metadata.get("id"))
                    or type(metadata.get("sizeBytes")) is not int or metadata["sizeBytes"] <= 0
                    or not isinstance(metadata.get("digest"), str)
                    or re.fullmatch(r"[a-f0-9]{64}", metadata["digest"]) is None):
                errors.append("Malformed encrypted artifact identity.")
                continue
            if metadata["id"] in seen:
                errors.append("Duplicate encrypted artifact identity.")
            seen.add(metadata["id"])
    return errors


def artifact_evidence(directory, backups):
    """Independently inspect only ciphertext files inside this runner's root."""
    errors, artifacts = [], []
    if not isinstance(backups, list):
        return ["No encrypted artifacts were reported."], artifacts
    for backup in backups:
        metadata = backup.get("metadata", {}) if isinstance(backup, dict) else {}
        identity = metadata.get("id")
        if not canonical_uuid(identity):
            errors.append("Invalid ciphertext file identity.")
            continue
        path = directory / "backups" / f"{identity}.age"
        try:
            info = path.lstat()
            if not stat.S_ISREG(info.st_mode):
                raise ValueError("ciphertext must be a regular file, without symlinks")
            digest = hashlib.sha256()
            with path.open("rb") as source:
                header = source.read(8192)
                digest.update(header)
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(chunk)
            parts = header.split(b"\n--- ", 1)
            if (not header.startswith(b"age-encryption.org/v1\n") or len(parts) != 2
                    or len(re.findall(br"(?m)^-> ", parts[0])) != 1
                    or len(re.findall(br"(?m)^-> scrypt [A-Za-z0-9+/]+ 18$", parts[0])) != 1):
                raise ValueError("ciphertext does not expose the production single-scrypt N18 header")
            observed = {"id": identity, "sizeBytes": info.st_size, "digest": digest.hexdigest()}
            if observed["sizeBytes"] != metadata.get("sizeBytes") or observed["digest"] != metadata.get("digest"):
                raise ValueError("ciphertext bytes differ from native immutable metadata")
            artifacts.append(observed)
        except (OSError, ValueError) as error:
            errors.append(f"Ciphertext verification failed for {identity}: {error}")
    return errors, artifacts


def run_stage(binary, directory, stage):
    (directory / "options.json").write_text(json.dumps({
        "cleanupCycles": 10, "idleMs": 1000, "recoveryStage": stage,
    }) + "\n")
    report_path = directory / "report.json"
    if report_path.exists():
        report_path.unlink()  # Only this runner's own preceding-stage output.
    env = os.environ.copy()
    env["JOINERY_NATIVE_SMOKE_ROOT"] = str(directory)
    started = time.monotonic()
    with (directory / f"{stage}-stdout.log").open("wb") as stdout:
        with (directory / f"{stage}-stderr.log").open("wb") as stderr:
            process = subprocess.Popen([str(binary)], env=env, stdout=stdout, stderr=stderr)
            try:
                while process.poll() is None and time.monotonic() - started < STAGE_TIMEOUTS[stage]:
                    time.sleep(0.05)
                timed_out = process.poll() is None
            finally:
                if process.poll() is None:
                    process.kill()  # Only this runner's own child.
                process.wait(timeout=5)
    try:
        report = json.loads(report_path.read_text())
    except (OSError, ValueError) as error:
        report = {"passed": False, "error": f"Missing native report: {error}"}
    errors = acceptance_errors(report, stage, process.returncode, timed_out)
    artifacts = []
    if not errors:
        errors, artifacts = artifact_evidence(directory, report["encryptedRecovery"]["backups"])
    evidence = {
        "stage": stage, "passed": not errors, "errors": errors, "report": report,
        "returnCode": process.returncode, "runnerTimedOut": timed_out,
        "processElapsedMs": round((time.monotonic() - started) * 1000),
        "ciphertextArtifacts": artifacts,
    }
    (directory / f"runner-{stage}.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=int, default=1, choices=range(1, 4))
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("This actual hidden WebView check requires macOS")
    root = Path(__file__).resolve().parent.parent / ".cache" / "native-smoke"
    build = json.loads((root / "build.json").read_text())
    binary = Path(build["binary"]).resolve()
    if binary.parent != root.resolve() or hashlib.sha256(binary.read_bytes()).hexdigest() != build["sha256"]:
        raise SystemExit("Native test binary changed. Run pnpm desktop:smoke:build again.")
    runs = []
    for _ in range(args.runs):
        directory = Path(tempfile.mkdtemp(prefix="encrypted-recovery-", dir=root))
        setup = run_stage(binary, directory, "encryptedSetup")
        verify = None
        if setup["passed"] and (directory / "report-encrypted-setup.json").is_file():
            verify = run_stage(binary, directory, "encryptedVerify")
        errors = setup["errors"] + (verify["errors"] if verify else ["Native reopen was not run."])
        if verify and setup["ciphertextArtifacts"] != verify["ciphertextArtifacts"]:
            errors.append("Close/reopen changed ciphertext bytes.")
        run = {"directory": str(directory), "setup": setup, "verify": verify,
               "passed": not errors, "errors": errors}
        runs.append(run)
        print(json.dumps({"passed": run["passed"], "directory": str(directory), "errors": errors}), flush=True)
        if not run["passed"]:
            break
    evidence = {
        "build": build, "platform": platform.platform(), "runs": runs,
        "scope": "Two real hidden macOS main WebViews with normal invoke ACL, separate isolated root and in-memory secrets. Actual Preferences encrypted create/wrong-key/restore/undo, production N18 header and ciphertext identity, full process close/reopen and zero native connect/SQL counters. Import/export picker byte flow is covered separately by the browser harness. Does not qualify the normal installed app, OS keychain, external SSO/accounts, or visible interaction.",
    }
    path = root / "encrypted-recovery-runs.json"
    path.write_text(json.dumps(evidence, indent=2) + "\n")
    print(f"Detailed encrypted native recovery evidence: {path}")
    if any(not run["passed"] for run in runs):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
