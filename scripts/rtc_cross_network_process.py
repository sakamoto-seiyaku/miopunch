#!/usr/bin/env python3
"""Bounded owned-process pipe; invitation material is never written to logs."""
import json
import os
import re
import selectors
import signal
import subprocess
import time


def public_event(value, key=""):
    """Preserve safe diagnostics; ICE credentials/descriptions stay private."""
    if isinstance(value, str) and (re.search(r"sdp|ufrag|username|password|icepwd|fingerprint|certificate", key, re.I)):
        return None
    if isinstance(value, dict):
        return {name: public for name, item in value.items() if name not in ("invitation", "code")
                if (public := public_event(item, name)) is not None}
    if isinstance(value, list):
        return [public_event(item) for item in value]
    return value


class OwnedProcess:
    def __init__(self, lab, role, argv, name, *, environment=()):
        self.lab, self.name, self.role = lab, name, role
        command = ["rtk", "proxy", "docker", "exec", "--user", f"{os.getuid()}:{os.getgid()}", "-i"]
        for key, value in environment:
            command += ["--env", key+"="+str(value)]
        wrapped = argv
        if name in ("browser-driver", "mqtt-services", "stun", "relay", "broker"):
            wrapped = ["python3", "/repo/scripts/rtc_cross_network_cost.py", "/evidence/cost-"+name+".json",
                "browser" if name == "browser-driver" else "broker" if name == "broker" else "service", *argv]
        command += [lab.containers[role], "rtk", "proxy", *wrapped]
        self.error_log = (lab.output/(name+"-stderr.log")).open("w")
        self.public_log = (lab.output/(name+"-events.jsonl")).open("w")
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=self.error_log, start_new_session=True)
        os.set_blocking(self.process.stdout.fileno(), False)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.buffer = b""
        self.pending, self.entries = [], []
        self.started = time.monotonic()
        self.argv = command
        self.stopped = None

    def read(self, timeout=0):
        if not self.selector.select(timeout):
            return
        block = os.read(self.process.stdout.fileno(), 65536)
        if not block:
            if self.process.poll() is None:
                return
            self.selector.unregister(self.process.stdout)
            return
        self.buffer += block
        if len(self.buffer) > 2 * 1024 * 1024:
            raise RuntimeError("owned process JSON pipe exceeds bound")
        while b"\n" in self.buffer:
            raw, self.buffer = self.buffer.split(b"\n", 1)
            line = raw.decode()
            if line.startswith("STUN_UDP_"):
                label, body = line.split(" ", 1)
                entry = {"event": label, **json.loads(body)}
            elif line.startswith("RELAY "):
                entry = {"event": "relay-ready", "url": line[6:]}
            else:
                try:
                    entry = json.loads(line)
                except json.JSONDecodeError:
                    continue
            self.pending.append(entry)
            self.entries.append(entry)
            # Full offer/answer/credential content never comes from these fixture events.
            public = public_event(entry)
            self.public_log.write(json.dumps(public)+"\n")
            self.public_log.flush()

    def wait(self, event, *, budget=12):
        started = time.monotonic()
        while True:
            for index, entry in enumerate(self.pending):
                if entry.get("event") == event:
                    return self.pending.pop(index)
            if self.process.poll() is not None:
                self.read()
                for index, entry in enumerate(self.pending):
                    if entry.get("event") == event:
                        return self.pending.pop(index)
                raise RuntimeError(f"{self.name} exited{self.process.returncode} before {event}")
            remaining = min(self.lab.remaining(), budget-(time.monotonic()-started))
            if remaining <= 0:
                raise TimeoutError(f"{self.name} {event} deadline")
            self.read(min(0.05, remaining))

    def send(self, value):
        self.process.stdin.write((value+"\n").encode())
        self.process.stdin.flush()

    def command(self, value, event):
        self.read()
        self.pending = [entry for entry in self.pending if entry.get("event") != event]
        self.send(value)
        return self.wait(event)

    def stop(self):
        if self.stopped is not None:
            return self.stopped
        if self.process.poll() is None:
            try:
                budget = self.lab.stop_budget()
                stop_deadline = time.monotonic()+budget
                if budget <= 0.2:
                    raise TimeoutError("preserve terminal owned-resource removal reserve")
                if self.name in ("stun", "broker"):
                    label = "STUN_UDP_READY" if self.name == "stun" else "owned-broker-started"
                    pid = next(entry["pid"] for entry in self.entries if entry.get("event") == label)
                    self.lab.exec(self.role, ["python3", "-c", f"import os,signal;os.kill({pid},signal.SIGTERM)"],
                        user=f"{os.getuid()}:{os.getgid()}", name="owned-"+self.name+"-stop", cleanup=True, budget=min(0.5, budget/2))
                else:
                    self.send("stop")
                    self.process.stdin.close()
                self.process.wait(timeout=max(0.001, stop_deadline-time.monotonic()-0.2))
            except BaseException:
                try:
                    os.killpg(self.process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    self.process.wait(timeout=0.1)
                except subprocess.TimeoutExpired:
                    try:
                        os.killpg(self.process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    self.process.wait(timeout=0.1)
        self.read()
        self.public_log.close()
        self.error_log.close()
        self.selector.close()
        self.stopped = {"name": self.name, "argv": self.argv, "exit": self.process.returncode,
                        "seconds": time.monotonic()-self.started}
        return self.stopped
