#!/usr/bin/env python3
"""Qualify a fresh Popen launch before registering its signal identity.

An empty /proc cmdline during exec is not an identity. This helper never
adopts another PID, refreshes an existing owner, or changes a signal guard.
"""
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time

from forwarding_local_resources import OwnedProcess, identity


def wait_future(future, timeout):
    """Wait for an owned future using its complete monotonic budget."""
    deadline = time.monotonic() + timeout
    while not future.done():
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("owned future exceeded monotonic wait budget")
        time.sleep(min(.02, remaining))
    return future.result()


def register_process(process, receipt):
    assert isinstance(process.args, list) and all(isinstance(arg, str) for arg in process.args)
    expected = b"\0".join(arg.encode() for arg in process.args) + b"\0"
    expected_hash = hashlib.sha256(expected).hexdigest()
    first = {"start_ticks": None}
    def journal(phase, **details):
        with Path(receipt).open("a") as output:
            output.write(json.dumps(dict(phase=phase, pid=process.pid, start_ticks=first["start_ticks"],
                expected_cmdline_sha256=expected_hash, monotonic=time.monotonic(), **details)) + "\n")
            output.flush()
            os.fsync(output.fileno())
    journal("popen_created")
    try:
        first = identity(process.pid)
    except (FileNotFoundError, ProcessLookupError):
        journal("launch_identity_absent", exit=process.poll())
        raise RuntimeError("owned child identity absent before launch qualification") from None
    journal("launch_observed")
    deadline = time.monotonic() + 2
    stable = None
    while time.monotonic() < deadline:
        if process.poll() is not None:
            journal("launch_exited", exit=process.returncode)
            raise RuntimeError("owned child exited before launch identity qualified")
        current = identity(process.pid)
        if current["start_ticks"] != first["start_ticks"]:
            raise RuntimeError("owned child start identity changed before registration")
        raw = (Path("/proc") / str(process.pid) / "cmdline").read_bytes()
        if raw:
            if raw != expected or current["cmdline_sha256"] != expected_hash:
                journal("launch_not_yet_exact", observed_cmdline_sha256=current["cmdline_sha256"])
                if stable is not None:
                    raise RuntimeError("owned child command changed during launch qualification")
                # A newly forked child can still show its parent's argv before
                # exec. It is not a signal owner until its exact Popen launch
                # has been observed twice with the original start identity.
                time.sleep(0.01)
                continue
            if stable == current:
                owner = OwnedProcess(process, receipt)
                if owner.expected != current:
                    if owner.pidfd is not None:
                        os.close(owner.pidfd)
                        owner.pidfd = None
                    journal("registration_identity_changed", observed=owner.expected)
                    raise RuntimeError("owned child changed while registering signal identity")
                namespaces = {}
                for name in ["net", "pid"]:
                    try:
                        namespaces[name] = os.readlink("/proc/%d/ns/%s" % (process.pid, name))
                    except (FileNotFoundError, PermissionError):
                        namespaces[name] = None
                journal("qualified_namespaces", namespaces=namespaces,
                        controller_network_namespace=os.readlink("/proc/self/ns/net"))
                return owner
            stable = current
        time.sleep(0.01)
    raise RuntimeError("owned child launch identity never became stable")


class OrdinaryChild:
    def __init__(self, process, receipt):
        self.process, self.receipt, self.owner = process, Path(receipt), None
        self.cleanup = None
        self.stdout, self.stderr = None, None
        self.persisted = {"stdout": b"", "stderr": b""}
        self.created = set()

    def retain_output(self, stdout=None, stderr=None):
        # TimeoutExpired may carry bytes even for a text-mode Popen. A later
        # communicate returns the complete stream, replacing its partial copy.
        for name, value in [("stdout", stdout), ("stderr", stderr)]:
            if value is not None:
                if isinstance(value, str):
                    value = value.encode()
                setattr(self, name, value)
            retained = getattr(self, name)
            if retained is not None:
                path = self.receipt.with_suffix("."+name)
                previous = self.persisted[name]
                if not retained.startswith(previous):
                    raise RuntimeError("owned child output changed after capture")
                if name not in self.created:
                    mode = "xb"
                elif len(retained) > len(previous):
                    mode = "ab"
                else:
                    continue
                with path.open(mode) as output:
                    if mode == "xb":
                        path.chmod(0o600)
                    self.created.add(name)
                    output.write(retained[len(previous):])
                    output.flush()
                    os.fsync(output.fileno())
                self.persisted[name] = retained

    def communicate(self, **arguments):
        try:
            output, errors = self.process.communicate(**arguments)
        except subprocess.TimeoutExpired as failure:
            self.retain_output(failure.output, failure.stderr)
            raise
        self.retain_output(output, errors)
        return output, errors

    def close(self):
        if self.cleanup is not None:
            return self.cleanup
        failures = []
        try:
            if self.process.stdin and not self.process.stdin.closed:
                self.process.stdin.close()
                self.process.stdin = None
            self.communicate(timeout=5)
        except BaseException as failure:
            failures.append(type(failure).__name__)
        if self.process.poll() is None and self.owner:
            try:
                self.owner.send(signal.SIGTERM)
                self.communicate(timeout=5)
            except BaseException as failure:
                failures.append(type(failure).__name__)
        if self.process.poll() is not None and self.owner:
            self.owner.close()
        self.cleanup = {"owner": "ordinary-helper", "pid": self.process.pid, "exit": self.process.poll(),
                        "errors": failures, "registered": self.owner is not None,
                        "stdout": str(self.receipt.with_suffix(".stdout")) if self.stdout is not None else None,
                        "stderr": str(self.receipt.with_suffix(".stderr")) if self.stderr is not None else None}
        with self.receipt.open("a") as output:
            output.write(json.dumps(dict(phase="ordinary_cleanup", **self.cleanup)) + "\n")
            output.flush()
            os.fsync(output.fileno())
        if failures or self.process.poll() is None:
            raise RuntimeError("ordinary owned child cleanup failed")
        return self.cleanup

    def terminal(self):
        return self.process.poll() is not None and (self.owner is None or self.owner.pidfd is None)


def run_process(arguments, *, env, receipt, timeout, owners, input=None):
    """A bounded ordinary child; even timeout signals use the same guard."""
    process = subprocess.Popen(arguments, env=env, stdin=subprocess.PIPE if input is not None else subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    child = OrdinaryChild(process, receipt)
    owners.append(child)
    primary = None
    output, errors = "", ""
    try:
        child.owner = register_process(process, receipt)
        output, errors = child.communicate(input=input, timeout=timeout)
    except BaseException as failure:
        primary = failure
    finally:
        try:
            child.close()
        except BaseException:
            if primary is None:
                raise
    if primary:
        raise primary
    if not child.terminal():
        raise RuntimeError("ordinary owned child cleanup failed")
    return subprocess.CompletedProcess(arguments, process.returncode, output, errors)
