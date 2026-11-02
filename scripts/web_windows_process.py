#!/usr/bin/env python3
"""Original Windows child handles and bounded JSON pipes for M119 fixtures."""
import hashlib
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time


class WindowsChild:
    def __init__(self, argv, environment, output, label, response_timeout=10, owners=None):
        if os.name != "nt":
            raise RuntimeError("actual Windows process handles required")
        self.output, self.label = Path(output), label
        self.write_lock, self.call_lock = threading.Lock(), threading.Lock()
        self.events, self.errors = [], []
        self.window_premises = []
        self.queue = queue.Queue(maxsize=128)
        self.sequence = 0
        self.response_timeout = response_timeout
        self.closed = None
        self.argv = list(argv)
        self.readers = []
        self.record("launch_intent", argv=self.argv,
                    executable_sha256=hashlib.sha256(Path(argv[0]).read_bytes()).hexdigest())
        self.process = subprocess.Popen(self.argv, env=environment, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                        text=True, encoding="utf-8", errors="strict", bufsize=1)
        if owners is not None:
            owners.append(self)
        self.handle = int(self.process._handle)
        self.record("created_original_handle", pid=self.process.pid, native_handle=self.handle)
        self.readers = [threading.Thread(target=self.read, args=(name,), daemon=True)
                        for name in ["stdout", "stderr"]]
        for reader in self.readers:
            reader.start()

    def record(self, phase, **fields):
        with (self.output / (self.label + ".owner.jsonl")).open("a", encoding="utf-8") as log:
            log.write(json.dumps(dict(phase=phase, monotonic=time.monotonic(), **fields)) + "\n")
            log.flush()
            os.fsync(log.fileno())

    def close_input(self):
        with self.write_lock:
            if not self.process.stdin.closed:
                self.process.stdin.close()
                self.record("original_stdin_closed")

    def read(self, name):
        stream = getattr(self.process, name)
        total = 0
        try:
            with (self.output / (self.label + "." + name)).open("w", encoding="utf-8") as log:
                while True:
                    line = stream.readline(131073)
                    if not line:
                        break
                    total += len(line.encode("utf-8"))
                    if len(line) > 131072 or total > 1024 * 1024:
                        self.errors.append(name + "_bound")
                        continue
                    log.write(line)
                    log.flush()
                    if name == "stdout":
                        value = json.loads(line)
                        if value.get("event"):
                            self.events.append(value)
                        if value.get("event") == "closing":
                            self.close_input()
                        self.queue.put(value, timeout=2)
        except BaseException as failure:
            self.errors.append(name + ":" + type(failure).__name__)
        finally:
            stream.close()
            if name == "stdout":
                try:
                    self.queue.put(None, timeout=2)
                except queue.Full:
                    self.errors.append("stdout_queue_full_at_eof")

    def receive(self, timeout):
        value = self.queue.get(timeout=timeout)
        if value is None:
            raise RuntimeError("original child stdout ended before expected response")
        return value

    def wait_ready(self, timeout=15):
        deadline = time.monotonic() + timeout
        while True:
            value = self.receive(max(0, deadline - time.monotonic()))
            if value.get("event") == "own_window_precondition":
                if (value.get("valid") is not True or value.get("process") != self.process.pid or
                        value.get("window_thread") != value.get("caller_thread") or
                        value.get("phase") not in {"before_environment", "after_environment"}):
                    raise RuntimeError("fresh own-window/thread premise failed: " + str(value))
                self.window_premises.append(value)
                continue
            if value.get("event") != "ready":
                raise RuntimeError("original host failed before readiness: " + str(value))
            if self.window_premises:
                if ([item["phase"] for item in self.window_premises] != ["before_environment", "after_environment"] or
                        self.window_premises[0]["window_thread"] != self.window_premises[1]["window_thread"]):
                    raise RuntimeError("own-window/thread observations did not match across environment initialization")
            return value

    def call(self, operation, expected_ok=True, **arguments):
        with self.call_lock:
            self.sequence += 1
            identifier = self.sequence
            raw = json.dumps(dict(id=identifier, op=operation, **arguments), ensure_ascii=True) + "\n"
            if len(raw) > 65536:
                raise ValueError("private command exceeds existing host bound")
            with self.write_lock:
                self.process.stdin.write(raw)
                self.process.stdin.flush()
            deadline = time.monotonic() + self.response_timeout
            while True:
                value = self.receive(max(0, deadline - time.monotonic()))
                if value.get("event"):
                    continue
                self.last_response = value
                if value.get("id") != identifier or value.get("ok") is not expected_ok:
                    raise RuntimeError("private command failed or correlation changed")
                return value.get("result")

    def join(self, expected_exit=0, timeout=10):
        self.close_input()
        code = self.process.wait(timeout=timeout)
        for reader in self.readers:
            if reader.ident is not None:
                reader.join(timeout=5)
        alive = sum(reader.is_alive() for reader in self.readers)
        self.closed = dict(owner=self.label, pid=self.process.pid, exit=code,
                           original_handle=self.handle, readers_alive=alive, errors=list(self.errors))
        self.record("joined", **self.closed)
        if code != expected_exit or alive or self.errors:
            raise RuntimeError("owned Windows child did not settle cleanly")
        return self.closed

    def cleanup(self):
        failures = []
        try:
            self.close_input()
            self.process.wait(timeout=10)
        except BaseException as failure:
            failures.append(type(failure).__name__)
        if self.process.poll() is None:
            # Popen.terminate on Windows uses this ORIGINAL native handle,
            # never a PID lookup/reopened process or another process tree.
            if int(self.process._handle) != self.handle:
                failures.append("original_handle_changed_no_termination")
            else:
                self.record("original_handle_terminate_intent", native_handle=self.handle)
                self.process.terminate()
                self.process.wait(timeout=5)
                failures.append("forced_termination_not_clean_exit")
        for reader in self.readers:
            if reader.ident is not None:
                reader.join(timeout=5)
        result = dict(owner=self.label, exit=self.process.poll(),
                      readers_alive=sum(reader.is_alive() for reader in self.readers),
                      errors=failures + self.errors)
        self.record("cleanup", **result)
        if result["exit"] is None or result["readers_alive"] or result["errors"]:
            raise RuntimeError("Windows child cleanup failed")
        return result
