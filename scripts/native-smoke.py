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
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent / ".cache" / "native-smoke"
    build = json.loads((root / "build.json").read_text())
    binary = Path(build["binary"]).resolve()
    if binary.parent != root.resolve() or hashlib.sha256(binary.read_bytes()).hexdigest() != build["sha256"]:
        raise SystemExit("Native test binary changed. Run pnpm desktop:smoke:build again.")
    results = []
    for _ in range(args.runs):
        directory = Path(tempfile.mkdtemp(prefix="run-", dir=root))
        env = os.environ.copy()
        env["JOINERY_NATIVE_SMOKE_ROOT"] = str(directory)
        started = time.monotonic()
        process = subprocess.Popen([str(binary)], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        peak_rss = peak_footprint = 0
        phase_memory = {}
        while process.poll() is None and time.monotonic() - started < 30:
            rss, footprint = main_memory(process.pid)
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
            time.sleep(0.05)
        if process.poll() is None:
            process.kill()
        _, stderr = process.communicate(timeout=5)
        report_file = directory / "report.json"
        report = json.loads(report_file.read_text()) if report_file.exists() else {"passed": False, "error": stderr.decode(errors="replace")[-2000:]}
        report.update(processElapsedMs=round((time.monotonic() - started) * 1000), mainProcessPeakRssMiB=round(peak_rss / 1048576, 2), mainProcessPeakPhysicalFootprintMiB=round(peak_footprint / 1048576, 2), returnCode=process.returncode)
        report["mainProcessMemoryAvailable"] = peak_rss > 0
        report["phaseMemory"] = phase_memory
        results.append(report)
    evidence = {"build": build, "binarySha256": build["sha256"], "platform": platform.platform(), "scope": "Hidden native integration fixture, including ten tab/result cleanup cycles with in-flight Arrow requests. Phase memory is sampled every 50 ms for the main process only; WebView helpers are excluded. Shell checkpoint measures bootstrap completion, not visible paint. OS services may be warm. Does not qualify total idle RAM, fully cold startup, or display FPS.", "acceptance": {"totalIdleMemory": "unqualified", "coldVisibleStartup": "unqualified", "displayFps": "unqualified"}, "runs": results}
    (root / "runs.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence, indent=2))
    if any(not row.get("passed") or row.get("returnCode") != 0 for row in results):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
