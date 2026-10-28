"""Run the separately built hidden WebView test; never launch the normal app."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time
from runtime_memory import sampler


def main_memory(pid):
    """Main-process metrics only; WebKit helpers are intentionally not attributed."""
    if platform.system() == "Darwin":
        library = ctypes.CDLL("/usr/lib/libproc.dylib")
        library.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        library.proc_pid_rusage.restype = ctypes.c_int
        # SDK sys/resource.h rusage_info_v0: UUID[16], then ten uint64 fields.
        info = (ctypes.c_uint64 * 12)()
        if library.proc_pid_rusage(pid, 0, ctypes.byref(info)) == 0:
            return info[8], info[9]  # resident_size, phys_footprint
    if platform.system() == "Linux":
        try:
            for line in Path(f"/proc/{pid}/status").read_text().splitlines():
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) * 1024, 0
        except OSError:
            pass
    return 0, 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", type=int, default=3, choices=range(1, 11))
    parser.add_argument("--cycles", type=int, default=10, choices=range(10, 1001))
    parser.add_argument("--idle-seconds", type=int, default=1, choices=range(1, 61))
    parser.add_argument("--attributed-memory", action="store_true",
                        help="Launch an isolated hidden macOS bundle and sample attributable helpers")
    args = parser.parse_args()
    if args.attributed_memory and platform.system() != "Darwin":
        parser.error("--attributed-memory requires macOS")
    root = Path(__file__).resolve().parent.parent / ".cache" / "native-smoke"
    build = json.loads((root / "build.json").read_text())
    binary = Path(build["binary"]).resolve()
    if binary.parent != root.resolve() or hashlib.sha256(binary.read_bytes()).hexdigest() != build["sha256"]:
        raise SystemExit("Native test binary changed. Run pnpm desktop:smoke:build again.")
    bundle = Path(build.get("bundle", ""))
    if args.attributed_memory:
        bundle_binary = bundle / "Contents" / "MacOS" / "joinery-smoke"
        if bundle.parent.resolve() != root.resolve() or not bundle.is_dir():
            raise SystemExit("Isolated bundle unavailable. Run pnpm desktop:smoke:build again.")
        if hashlib.sha256(bundle_binary.read_bytes()).hexdigest() != build.get("bundleBinarySha256"):
            raise SystemExit("Isolated bundle changed. Run pnpm desktop:smoke:build again.")
        subprocess.run(["codesign", "--verify", "--strict", str(bundle)], check=True)
    memory = sampler() if args.attributed_memory else None
    timeout_seconds = 35 + args.cycles + 2 * args.idle_seconds
    results = []
    for _ in range(args.runs):
        directory = Path(tempfile.mkdtemp(prefix="run-", dir=root))
        env = os.environ.copy()
        env["JOINERY_NATIVE_SMOKE_ROOT"] = str(directory)
        (directory / "options.json").write_text(json.dumps({
            "cleanupCycles": args.cycles, "idleMs": args.idle_seconds * 1000,
        }))
        started = time.monotonic()
        command = [str(binary)]
        if args.attributed_memory:
            command = ["open", "-g", "-j", "-n", "-W", "--env",
                       f"JOINERY_NATIVE_SMOKE_ROOT={directory}", str(bundle)]
        # File-backed output cannot block the fixture when pipe buffers fill.
        stdout_file = (directory / "stdout.log").open("wb")
        stderr_file = (directory / "stderr.log").open("wb")
        process = subprocess.Popen(command, env=env, stdout=stdout_file, stderr=stderr_file)
        fixture_pid = None if args.attributed_memory else process.pid
        attributed_samples = []
        peak_rss = peak_footprint = 0
        phase_memory = {}
        while process.poll() is None and time.monotonic() - started < timeout_seconds:
            if fixture_pid is None:
                try:
                    fixture_pid = int(json.loads((directory / "startup.json").read_text())["pid"])
                except (OSError, ValueError, KeyError):
                    time.sleep(0.05)
                    continue
            rss, footprint = main_memory(fixture_pid)
            peak_rss = max(peak_rss, rss)
            peak_footprint = max(peak_footprint, footprint)
            try:
                checkpoint = json.loads((directory / "checkpoint.json").read_text())
                phase = checkpoint["phase"]
                if phase not in phase_memory:
                    phase_memory[phase] = {"samples": 0, "rssBytes": [], "physicalFootprintBytes": []}
                sample = phase_memory[phase]
                if rss:
                    sample["samples"] += 1
                    sample["rssBytes"].append(rss)
                    if footprint:
                        sample["physicalFootprintBytes"].append(footprint)
            except (OSError, ValueError, KeyError):
                pass  # A checkpoint can be replaced between polling reads.
            if memory:
                sample = memory.snapshot(fixture_pid)
                sample["elapsedMs"] = round((time.monotonic() - started) * 1000)
                try:
                    current = json.loads((directory / "checkpoint.json").read_text())
                    sample["phase"] = current["phase"]
                    sample["cycle"] = current.get("cycle")
                except (OSError, ValueError, KeyError):
                    sample["phase"] = "starting"
                attributed_samples.append(sample)
            time.sleep(0.2 if memory else 0.05)
        timed_out = process.poll() is None
        if process.poll() is None:
            process.kill()
        process.wait(timeout=5)
        stdout_file.close()
        stderr_file.close()
        report_file = directory / "report.json"
        report = json.loads(report_file.read_text()) if report_file.exists() else {"passed": False, "error": (directory / "stderr.log").read_text(errors="replace")[-2000:]}
        report.update(processElapsedMs=round((time.monotonic() - started) * 1000), mainProcessPeakRssMiB=round(peak_rss / 1048576, 2), mainProcessPeakPhysicalFootprintMiB=round(peak_footprint / 1048576, 2), returnCode=process.returncode)
        if timed_out:
            report["passed"] = False
            report["error"] = "Runner deadline exceeded; the fixture has its own bounded watchdog."
        report["runnerTimedOut"] = timed_out
        report["attributedMemorySamples"] = attributed_samples
        report["mainProcessMemoryAvailable"] = peak_rss > 0
        report["phaseMemory"] = phase_memory
        results.append(report)
    evidence = {"build": build, "binarySha256": build["sha256"], "platform": platform.platform(), "scope": "Hidden native synthetic integration fixture with configurable cleanup cycles and empty/released idle intervals. Main memory uses libproc on macOS. Optional isolated-bundle samples enumerate processes and attribute helpers only when responsibility points to the fixture itself. Missing API, inherited responsibility, absent WebKit helpers and unreadable processes remain explicit gaps. RSS sums can double-count shared pages; physical footprint is separate. Sampling is sequential and best-effort, not an atomic OS snapshot. Shell checkpoint is DOM readiness, not visible paint. Does not qualify fully cold startup, display FPS or representative production workloads.", "acceptance": {"totalIdleMemory": "unqualified", "coldVisibleStartup": "unqualified", "displayFps": "unqualified"}, "runs": results}
    (root / "runs.json").write_text(json.dumps(evidence, indent=2) + "\n")
    for row in results:
        print(json.dumps({key: row.get(key) for key in ["passed", "cleanupCycles", "nativeElapsedMs", "mainProcessPeakRssMiB", "returnCode"]}))
    print(f"Detailed native evidence: {root / 'runs.json'}")
    if any(not row.get("passed") or row.get("returnCode") != 0 for row in results):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
