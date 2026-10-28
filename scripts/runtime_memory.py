"""Best-effort attribution for the isolated macOS hidden fixture, never by name alone.

Uses libproc rusage_info_v0 and the dynamically resolved macOS responsibility
API. API absence, shared responsibility and failed reads remain explicit gaps.
See Chromium base/process/process_info_mac.mm for the responsibility signature.
RSS sums double-count shared pages; physical footprint is reported separately.
"""
import ctypes
import platform
import subprocess


def select_processes(main_pid, rows, responsible):
    """Pure attribution step; an inherited responsibility must never match peers."""
    own = responsible(main_pid)
    if own != main_pid:
        return [(main_pid, "main")], "shared_or_unavailable_responsibility"
    selected = [(main_pid, "main")]
    for pid, name in rows:
        if pid != main_pid and responsible(pid) == main_pid:
            selected.append((pid, "webkit" if "WebKit" in name else "helper"))
    return selected, "responsibility_attributed"


class MacMemory:
    def __init__(self):
        self.library = ctypes.CDLL("/usr/lib/libproc.dylib")
        self.library.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        self.library.proc_pid_rusage.restype = ctypes.c_int
        try:
            self.responsible = ctypes.CDLL(None).responsibility_get_pid_responsible_for_pid
            self.responsible.argtypes = [ctypes.c_int]
            self.responsible.restype = ctypes.c_int
        except AttributeError:
            self.responsible = lambda _: -1

    def memory(self, pid):
        info = (ctypes.c_uint64 * 12)()
        if self.library.proc_pid_rusage(pid, 0, ctypes.byref(info)) != 0:
            return None
        return {"rssBytes": info[8], "physicalFootprintBytes": info[9]}

    def snapshot(self, pid):
        try:
            # Executable names only: never inspect command arguments/environments.
            listing = subprocess.check_output(
                ["ps", "-axo", "pid=,comm="], text=True, stderr=subprocess.DEVNULL,
                timeout=2,
            )
            rows = []
            for line in listing.splitlines():
                parts = line.strip().split(None, 1)
                if len(parts) == 2 and parts[0].isdigit():
                    rows.append((int(parts[0]), parts[1]))
            selected, status = select_processes(pid, rows, self.responsible)
        except (OSError, subprocess.SubprocessError):
            selected, status = [(pid, "main")], "process_enumeration_unavailable"
        processes = []
        unreadable = []
        for process_id, role in selected:
            memory = self.memory(process_id)
            if memory is None:
                unreadable.append(process_id)
            else:
                processes.append({"pid": process_id, "role": role, **memory})
        return summarize(processes, unreadable, status)


def summarize(processes, unreadable, status):
    # These are simultaneous sums, never a sum of unrelated per-process peaks.
    attributed = status == "responsibility_attributed"
    has_webkit = any(process["role"] == "webkit" for process in processes)
    return {
        "attribution": status,
        "processes": processes,
        "unreadablePids": unreadable,
        "helpersObserved": has_webkit,
        "sampleComplete": attributed and has_webkit and not unreadable,
        "summedRssBytes": sum(process["rssBytes"] for process in processes),
        "summedPhysicalFootprintBytes": sum(process["physicalFootprintBytes"] for process in processes),
    }


def sampler():
    if platform.system() == "Darwin":
        return MacMemory()
    return None
