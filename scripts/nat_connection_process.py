#!/usr/bin/env python3
"""Measure one actual native node child inside its network namespace.

The wrapper's Python, docker client, allocator and broker CPU are excluded.
No timeout intermediary sits between wait4 and the measured binary.
"""
import argparse
import hashlib
import json
import os
import pathlib
import signal
import subprocess
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--limit", type=float, default=75)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command or args.output.exists():
        parser.error("require command and a new accounting path")
    executable = pathlib.Path(command[0]).resolve()
    # A whole debug-binary read before fork can contaminate wait4 ru_maxrss
    # with the launcher's inherited high-water mark. Hash without that peak.
    with executable.open("rb") as binary:
        expected = hashlib.file_digest(binary, "sha256").hexdigest()
    started = time.monotonic()
    child = subprocess.Popen(command)
    maximum_fd = 0
    exec_hwm = 0
    exec_samples = 0
    timed_out = False
    while True:
        try:
            process = pathlib.Path(f"/proc/{child.pid}")
            if (process / "exe").resolve() == executable:
                exec_samples += 1
                maximum_fd = max(maximum_fd, len(list((process / "fd").iterdir())))
                for line in (process / "status").read_text().splitlines():
                    if line.startswith("VmHWM:"):
                        exec_hwm = max(exec_hwm, int(line.split()[1]))
        except (FileNotFoundError, ProcessLookupError):
            pass
        pid, status, usage = os.wait4(child.pid, os.WNOHANG)
        if pid:
            break
        if time.monotonic() - started > args.limit and not timed_out:
            timed_out = True
            child.send_signal(signal.SIGKILL)
        time.sleep(0.01)
    child.returncode = os.waitstatus_to_exitcode(status)
    record = dict(schema=1, scope="actual native node process", collector="namespace-local wait4",
                  pid=pid, exit=child.returncode, timed_out=timed_out,
                  executable=str(executable), executable_sha256=expected,
                  user_cpu_seconds=usage.ru_utime, system_cpu_seconds=usage.ru_stime,
                  wall_seconds=time.monotonic() - started, max_rss_kib=usage.ru_maxrss,
                  max_rss_scope="wait4 child fork/exec lifetime; streamed launcher hash",
                  sampled_exec_vm_hwm_kib=exec_hwm, exec_samples=exec_samples,
                  sampled_maximum_fd=maximum_fd, voluntary_context_switches=usage.ru_nvcsw,
                  involuntary_context_switches=usage.ru_nivcsw)
    args.output.write_text(json.dumps(record, indent=2) + "\n")
    raise SystemExit(child.returncode)


if __name__ == "__main__":
    main()
