#!/usr/bin/env python3
"""Actual clipboard/public-client acceptance. Never compiles or uses SSH.

Each Native host uses its own X server and actual X11 CLIPBOARD selection.
Independent xclip readers observe system results; no in-memory clipboard stub,
shared WSLg bridge, clipboard plaintext or clipboard digest enters receipts.
"""
import argparse
import hashlib
import json
import marshal
import os
from pathlib import Path
import queue
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import types


class Desktop:
    def __init__(self, runner, name):
        self.runner, self.name = runner, name
        self.owner, self.process, self.display_lease = None, None, None
        self.log = (runner.output / (name + "-xserver.log")).open("w")
        runner.desktops.append(self)
        # Never use :0/:1: after an abstract X server disappears, Xlib may try
        # the same display's named socket (including the user's WSLg bridge).
        # A unique high display and private cookie prevent that fallback.
        for _ in range(32):
            display = 2000 + secrets.randbelow(50000)
            lease = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                lease.bind("\0miopunch-m117-xdisplay-" + str(display))
                probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                try:
                    probe.bind("\0/tmp/.X11-unix/X" + str(display))
                finally:
                    probe.close()
                if Path("/tmp/.X11-unix/X" + str(display)).exists():
                    lease.close()
                    continue
                self.display_lease = lease
                break
            except OSError:
                lease.close()
        assert self.display_lease is not None, "no unique owned X11 display available"
        authority = runner.private / (name + ".Xauthority")
        authority.touch(mode=0o600)
        runtime = runner.private / (name + "-runtime")
        runtime.mkdir(mode=0o700)
        self.env = dict(os.environ, DISPLAY=":" + str(display), XAUTHORITY=str(authority),
                        XDG_RUNTIME_DIR=str(runtime), XDG_SESSION_TYPE="x11",
                        GDK_BACKEND="x11", QT_QPA_PLATFORM="xcb", SDL_VIDEODRIVER="x11",
                        CLUTTER_BACKEND="x11", MOZ_ENABLE_WAYLAND="0",
                        DBUS_SESSION_BUS_ADDRESS="unix:path=" + str(runtime / "no-session-bus"))
        for key in ["WAYLAND_DISPLAY", "WAYLAND_SOCKET", "MIR_SOCKET", "XAUTHLOCALHOSTNAME"]:
            self.env.pop(key, None)
        self.env["PATH"] = str(runner.tools) + os.pathsep + self.env["PATH"]
        cookie = secrets.token_hex(16)
        auth = subprocess.run(["xauth", "-f", str(authority), "source", "-"],
                              input=f"add :{display} MIT-MAGIC-COOKIE-1 {cookie}\n",
                              text=True, capture_output=True, timeout=5, env=self.env)
        assert auth.returncode == 0, "private X11 authentication setup failed"
        self.process = subprocess.Popen(
            ["Xvfb", ":" + str(display), "-auth", str(authority), "-screen", "0", "800x600x24",
             "-nolisten", "tcp", "-nolisten", "unix", "-listen", "local", "-noreset"],
            stdout=self.log, stderr=self.log)
        connection_probe = """import ctypes,ctypes.util
x=ctypes.CDLL(ctypes.util.find_library('X11'))
x.XOpenDisplay.argtypes=[ctypes.c_char_p];x.XOpenDisplay.restype=ctypes.c_void_p
x.XCloseDisplay.argtypes=[ctypes.c_void_p]
d=x.XOpenDisplay(None)
if d:x.XCloseDisplay(d)
raise SystemExit(0 if d else 1)
"""
        def ready():
            assert self.process.poll() is None, "owned X11 server exited during startup"
            result = subprocess.run([sys.executable, "-c", connection_probe], env=self.env,
                                    capture_output=True, timeout=3)
            return result.returncode == 0 and self.process.poll() is None
        runner.until(ready, "private authenticated X11 startup", 10)
        runner.record(name, "independent_system_session", {"backend": "Xvfb/X11", "display": ":" + str(display),
                    "private_authority": True, "abstract_only": True, "global_display_fallback_excluded": True,
                    "private_runtime_directory": True, "alternate_display_contexts_cleared": True})

    def read(self):
        result = subprocess.run(
            [str(self.runner.tools / "xclip"), "-selection", "clipboard", "-out", "-target", "UTF8_STRING"],
            env=self.env, capture_output=True, timeout=3)
        return result.stdout.decode("utf-8") if result.returncode == 0 else None

    def copy(self, text):
        old = self.owner
        self.owner = subprocess.Popen(
            [str(self.runner.tools / "xclip"), "-selection", "clipboard", "-in", "-target", "UTF8_STRING", "-quiet"],
            env=self.env, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.owner.stdin.write(text.encode())
        self.owner.stdin.close()
        self.runner.until(lambda: self.read() == text, "external system copy")
        if old is not None:
            if old.poll() is None:
                old.terminate()
            old.wait(timeout=5)

    def close(self):
        failures = []
        for process in [self.owner, self.process]:
            if process is not None:
                try:
                    if process.poll() is None:
                        process.terminate()
                    process.wait(timeout=5)
                except BaseException as failure:
                    failures.append(type(failure).__name__)
        try:
            self.log.close()
        except BaseException as failure:
            failures.append(type(failure).__name__)
        if failures:
            raise RuntimeError("owned desktop cleanup failed: " + ",".join(failures))

    def terminal(self):
        return all(process is None or process.poll() is not None for process in [self.owner, self.process])

    def release_lease(self):
        if self.display_lease is not None:
            self.display_lease.close()
            self.display_lease = None

    def input_focus(self, restore=False):
        # A normal X11 focus operation on this test's own isolated server.
        # No browser API/prototype is replaced and no global desktop changes.
        code = """import ctypes,ctypes.util,json,sys
x=ctypes.CDLL(ctypes.util.find_library('X11'))
x.XOpenDisplay.argtypes=[ctypes.c_char_p];x.XOpenDisplay.restype=ctypes.c_void_p
x.XGetInputFocus.argtypes=[ctypes.c_void_p,ctypes.POINTER(ctypes.c_ulong),ctypes.POINTER(ctypes.c_int)]
x.XSetInputFocus.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.c_int,ctypes.c_ulong]
x.XSync.argtypes=[ctypes.c_void_p,ctypes.c_int];x.XCloseDisplay.argtypes=[ctypes.c_void_p]
d=x.XOpenDisplay(None);assert d,'private authenticated X server unavailable'
try:
 p,r=ctypes.c_ulong(),ctypes.c_int();x.XGetInputFocus(d,ctypes.byref(p),ctypes.byref(r))
 if sys.argv[1]=='restore':
  assert p.value==0,'focus changed after owned blur'
  x.XSetInputFocus(d,int(sys.argv[2]),int(sys.argv[3]),0)
 else:x.XSetInputFocus(d,0,0,0)
 x.XSync(d,0);print(json.dumps({'focus':p.value,'revert':r.value}))
finally:x.XCloseDisplay(d)
"""
        result = subprocess.run([sys.executable, "-c", code, "restore" if restore else "blur",
                                 str(getattr(self, "saved_focus", 0)), str(getattr(self, "saved_revert", 0))],
                                env=self.env, capture_output=True, text=True, timeout=5)
        assert result.returncode == 0, "owned authenticated X11 focus operation failed"
        previous = json.loads(result.stdout)
        if not restore:
            self.saved_focus, self.saved_revert = previous["focus"], previous["revert"]


class Host:
    def __init__(self, runner, label, desktop, action="create", invitation=None, directory=None):
        self.runner, self.label, self.desktop = runner, label, desktop
        self.directory = directory or runner.private / (label + "-state")
        args = [str(runner.binary), action, str(self.directory)]
        if invitation:
            args.append(str(invitation))
        self.log = (runner.output / (label + ".stderr")).open("w")
        self.process = subprocess.Popen(args, env=desktop.env, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=self.log, text=True)
        self.lines = queue.Queue()
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()
        runner.hosts.append(self)
        ready = self.receive()
        assert ready.get("event") == "ready", ready
        self.node, self.address = ready["status"]["node"], ready["address"]
        runner.record(label, "ready", ready)
        if action == "create":
            self.call("export_recovery_card", path=str(runner.private / (label + "-card")))
            self.call("acknowledge_recovery_saved")

    def _read(self):
        for line in self.process.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def receive(self):
        deadline = time.monotonic() + 90
        while True:
            line = self.lines.get(timeout=max(0, deadline - time.monotonic()))
            assert line is not None, "owned client exited before response"
            value = json.loads(line)
            if value.get("event") and value["event"] != "ready":
                self.runner.record(self.label, "sdk_event", value)
            else:
                return value

    def call(self, operation, ok=True, **fields):
        self.runner.sequence += 1
        identifier = self.runner.sequence
        self.process.stdin.write(json.dumps(dict(id=identifier, op=operation, **fields)) + "\n")
        self.process.stdin.flush()
        answer = self.receive()
        self.runner.record(self.label, operation, answer)
        assert answer.get("id") == identifier and answer.get("ok") is ok, answer
        return answer.get("result")

    def close(self):
        failures = []
        if self.process.poll() is None:
            try:
                self.call("stop")
            except BaseException as failure:
                failures.append(type(failure).__name__)
        try:
            self.process.stdin.close()
        except BaseException as failure:
            failures.append(type(failure).__name__)
        try:
            if self.process.wait(timeout=20) != 0:
                failures.append("NonzeroExit")
        except BaseException as failure:
            failures.append(type(failure).__name__)
        self.reader.join(timeout=3)
        if self.reader.is_alive():
            failures.append("ReaderNotJoined")
        else:
            self.process.stdout.close()
        try:
            self.log.close()
        except BaseException as failure:
            failures.append(type(failure).__name__)
        self.cleanup_result = {"client": self.label, "exit": self.process.poll(),
                               "reader_joined": not self.reader.is_alive(), "errors": failures}
        if failures:
            raise RuntimeError("owned native host cleanup failed")
        return self.cleanup_result

    def terminal(self):
        return self.process.poll() is not None and not self.reader.is_alive()


class Runner:
    def __init__(self, args):
        self.output = args.output.resolve()
        self.output.mkdir(parents=True, exist_ok=False)
        self.binary, self.tools = args.linux.resolve(), args.tools.resolve()
        self.case = args.case
        assert (self.tools / "xclip").is_file(), "private xclip tool precondition failed"
        assert hashlib.sha256(self.binary.read_bytes()).hexdigest() == args.linux_sha
        self.private = Path(tempfile.mkdtemp(prefix="miopunch-m117-clipboard-"))
        (self.private / ".owner").write_text(str(self.output))
        self.hosts, self.desktops, self.sequence = [], [], 0
        self.controller_sources, self.loaded_code = {}, {}
        root = Path(__file__).resolve().parents[1]
        for module in list(sys.modules.values()):
            origin = getattr(module, "__file__", None)
            if not origin:
                continue
            path = Path(origin).resolve()
            if path.parent == root / "scripts" and path.name.startswith("check_clipboard") and path.suffix == ".py":
                self.freeze_source(path)
                for value in vars(module).values():
                    candidates = list(vars(value).values()) if isinstance(value, type) and value.__module__ == module.__name__ else [value]
                    for candidate in candidates:
                        if isinstance(candidate, types.FunctionType) and candidate.__module__ == module.__name__:
                            self.loaded_code[module.__name__ + ":" + candidate.__qualname__] = hashlib.sha256(marshal.dumps(candidate.__code__)).hexdigest()
        (self.output / "loaded-python-code.json").write_text(json.dumps(self.loaded_code, indent=2))
        self.record("runner", "pins", {"linux": str(self.binary), "sha256": args.linux_sha,
                                      "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()})

    def record(self, host, operation, result):
        with (self.output / "events.jsonl").open("a") as output:
            output.write(json.dumps(dict(host=host, operation=operation, result=result,
                                         monotonic=time.monotonic(), wall_time=time.time(),
                                         boottime=time.clock_gettime(time.CLOCK_BOOTTIME))) + "\n")

    def freeze_source(self, path):
        path = Path(path).resolve()
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if str(path) in self.controller_sources:
            assert self.controller_sources[str(path)] == digest, "controller changed after it was loaded"
        self.controller_sources[str(path)] = digest
        (self.output / "controller-start.json").write_text(json.dumps(self.controller_sources, indent=2))

    def until(self, condition, operation, seconds=15):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if condition():
                return
            time.sleep(.1)
        raise AssertionError(operation + " did not complete")

    def core(self):
        desktop = Desktop(self, "source")
        desktop.copy("M117 owned initial source marker")
        source = Host(self, "source", desktop)
        # The first tracer assertion is the real public operation with an
        # available system clipboard. The old host rejects this operation.
        state = source.call("clipboard_enable")
        assert state["enabled"] and state["capability"]["read"] and state["capability"]["write"]
        self.record("source", "actual_system_enable", {"passed": True})
        target_desktop = Desktop(self, "target")
        target_desktop.copy("M117 owned independent target marker")
        assert target_desktop.read() != desktop.read(), "independent sessions unexpectedly share clipboard"
        invitation = self.private / "target-invitation"
        source.call("issue_precode", path=str(invitation), seconds=3600, max_uses=1)
        target = Host(self, "target", target_desktop, "join", invitation)
        assert target.call("request", path=str(invitation))["stage"] == "Complete"
        target.call("clipboard_enable")
        source.call("clipboard_start", address=target.address)
        assert target.call("clipboard_status")["peer"] == source.node
        value = "M117 owned automatic forward 剪贴板\nwithout per-change send"
        desktop.copy(value)
        self.until(lambda: target_desktop.read() == value, "actual automatic target system write")
        self.until(lambda: source.call("clipboard_status")["acknowledged"] == 1, "actual write acknowledgment")
        assert target.call("clipboard_status")["written"] == 1
        self.record("target", "automatic_system_write", {"independent_system_read_matches": True})
        reverse = "M117 owned reverse 值\n"
        target_desktop.copy(reverse)
        self.until(lambda: desktop.read() == reverse, "actual reverse system write")
        self.until(lambda: target.call("clipboard_status")["acknowledged"] == 1, "reverse write acknowledgment")
        time.sleep(1)
        assert source.call("clipboard_status")["sent"] == 1
        assert target.call("clipboard_status")["sent"] == 1
        source.call("clipboard_stop")
        self.until(lambda: target.call("clipboard_status")["peer"] is None, "peer observes pair stop")
        desktop.copy("M117 owned post-stop value")
        time.sleep(1)
        assert target_desktop.read() == reverse
        self.record("pair", "duplex_stop_no_echo", {"reverse_matches": True, "one_change_each": True, "stopped_no_write": True})

    def member(self, admin, label, desktop):
        invitation = self.private / (label + "-invitation")
        admin.call("issue_precode", path=str(invitation), seconds=3600, max_uses=1)
        member = Host(self, label, desktop, "join", invitation)
        assert member.call("request", path=str(invitation))["stage"] == "Complete"
        return member

    def guards(self):
        source_desktop = Desktop(self, "guard-source")
        target_desktop = Desktop(self, "guard-target")
        third_desktop = Desktop(self, "guard-third")
        source_desktop.copy("M117 guard source initial")
        target_desktop.copy("M117 guard target initial")
        third_value = "M117 unrelated third system clipboard"
        third_desktop.copy(third_value)
        assert len({source_desktop.read(), target_desktop.read(), third_desktop.read()}) == 3
        source = Host(self, "guard-source", source_desktop)
        target = self.member(source, "guard-target", target_desktop)
        third = self.member(source, "guard-third", third_desktop)
        source.call("service_sync", address=target.address)
        third.call("service_sync", address=source.address)
        source.call("clipboard_enable")
        # Effective member access and a real clipboard do not start a service.
        source.call("clipboard_start", ok=False, address=target.address)
        assert source.call("clipboard_status")["peer"] is None
        assert target_desktop.read() == "M117 guard target initial"
        target.call("clipboard_enable")
        third.call("clipboard_enable")
        source.call("clipboard_start", address=target.address)
        value = "M117 positive before third-node exclusion"
        source_desktop.copy(value)
        self.until(lambda: target_desktop.read() == value, "guard positive real transport prerequisite")
        self.until(lambda: source.call("clipboard_status")["acknowledged"] == 1, "guard initial written acknowledgment")
        third.call("clipboard_start", ok=False, address=target.address)
        assert target.call("clipboard_status")["peer"] == source.node
        assert third.call("clipboard_status")["peer"] is None
        assert third_desktop.read() == third_value
        value = "M117 selected pair after third request"
        source_desktop.copy(value)
        self.until(lambda: target_desktop.read() == value, "selected pair continues")
        assert third_desktop.read() == third_value
        self.record("guard-third", "only_selected_pair", {"current_member": True, "third_has_real_clipboard": True,
                    "third_pair_rejected": True, "third_system_clipboard_unchanged": True, "selected_pair_continues": True})
        target.call("clipboard_disable")
        self.until(lambda: source.call("clipboard_status")["peer"] is None, "disable stops source pair")
        source_desktop.copy("M117 owned change after provider disable")
        time.sleep(.8)
        assert target_desktop.read() == value
        assert not target.call("clipboard_status")["enabled"]
        target.call("clipboard_enable")
        assert target.call("clipboard_status")["peer"] is None
        source.call("clipboard_start", address=target.address)
        value = "M117 restored explicit pair before ACL change"
        source_desktop.copy(value)
        self.until(lambda: target_desktop.read() == value, "positive before current ACL change")
        deny = dict(members=True, guests=True, allowed=[], denied=[source.node])
        revision = target.call("service_rule", provider=target.node, service="clipboard", previous=None, rule=deny)["revision"]
        self.until(lambda: target.call("clipboard_status")["peer"] is None, "current effective ACL stops target")
        self.until(lambda: source.call("clipboard_status")["peer"] is None, "current ACL closes actual connection")
        source_desktop.copy("M117 owned change after effective denial")
        time.sleep(.8)
        assert target_desktop.read() == value
        source.call("clipboard_start", ok=False, address=target.address)
        assert target_desktop.read() == value
        # Keep an independent external selection owner alive across close/open,
        # so a startup write could not be hidden by resetting the marker later.
        target_desktop.copy(value)
        target.close()
        reopened = Host(self, "guard-target-reopen", target_desktop, "open", directory=target.directory)
        assert reopened.node == target.node
        assert target_desktop.read() == value
        assert not reopened.call("clipboard_status")["enabled"]
        rule = next(p for p in reopened.call("service_status")["policies"] if p["provider"] == target.node and p["service"] == "clipboard")
        assert rule["revision"] == revision and rule["rule"] == deny
        self.record("guard-target-reopen", "independent_owner_spans_reopen", {"marker_established_before_close": True,
                    "same_system_marker_after_open": True, "reopened_written_zero": True})
        reopened.call("clipboard_enable")
        source.call("clipboard_start", ok=False, address=reopened.address)
        assert target_desktop.read() == value
        # The same peer identity has now had two real failed contacts. The
        # public SDK case observes this existing second retry-after <=2s;
        # respect it once before the independently allowed policy protocol.
        time.sleep(2.05)
        # An authorized administrator changes that same saved ACL remotely.
        source.call("service_sync", address=reopened.address)
        default = dict(members=True, guests=True, allowed=[], denied=[])
        new_revision = source.call("service_rule", provider=target.node, service="clipboard", previous=revision, rule=default)["revision"]
        source.call("service_sync", address=reopened.address)
        rule = next(p for p in reopened.call("service_status")["policies"] if p["provider"] == target.node and p["service"] == "clipboard")
        assert rule["revision"] == new_revision and rule["rule"] == default
        source.call("clipboard_start", address=reopened.address)
        restored = "M117 same automatic system operation after legitimate ACL restoration"
        source_desktop.copy(restored)
        self.until(lambda: target_desktop.read() == restored, "same operation after authorized ACL restoration")
        assert third_desktop.read() == third_value
        self.record("guards", "disable_current_acl_and_reopen", {"disabled_stops_actual_write": True,
                    "current_acl_stops_actual_write": True, "reopen_preserves_acl": True,
                    "enable_preserves_restriction": True, "local_and_remote_same_acl": True,
                    "same_core_after_restoration": True, "third_unchanged": True})

    def retention(self):
        source_desktop, target_desktop = Desktop(self, "retain-source"), Desktop(self, "retain-target")
        source_desktop.copy("M117 retain source initial")
        target_desktop.copy("M117 retain target initial")
        source = Host(self, "retain-source", source_desktop)
        target = self.member(source, "retain-target", target_desktop)
        source.call("clipboard_enable")
        target.call("clipboard_enable")
        source.call("clipboard_start", address=target.address)
        value = "M117 received system value survives disable and enable"
        source_desktop.copy(value)
        self.until(lambda: target_desktop.read() == value, "actual value before disable/enable")
        self.until(lambda: source.call("clipboard_status")["acknowledged"] == 1, "actual written receipt before disable/enable")
        target.call("clipboard_disable")
        assert target_desktop.read() == value
        target.call("clipboard_enable")
        assert target_desktop.read() == value, "enabling the service destroyed its current actual system clipboard selection"
        self.record("retain-target", "disable_enable_preserves_system_clipboard", {"independent_system_value_retained": True})

    def scenario(self):
        if self.case in {"core", "all"}:
            self.core()
        if self.case in {"guards", "all"}:
            self.guards()
        if self.case in {"retention", "all"}:
            self.retention()

    def run(self):
        started = time.monotonic()
        report = {"status": "failed"}
        primary_failure = None
        try:
            self.scenario()
            report["status"] = "pass"
        except BaseException as failure:
            primary_failure = failure
            report.update(failure_type=type(failure).__name__, failure=str(failure))
        cleanup, cleanup_failures = [], []
        # Browser scenarios may already close themselves, but the registered
        # owner is still an obligation on every exceptional return path.
        owners = ([self.browser] if hasattr(self, "browser") else []) + list(reversed(self.hosts))
        for owner in owners:
            try:
                entry = owner.close()
                cleanup.append(entry)
            except BaseException as failure:
                cleanup.append(getattr(owner, "cleanup_result", None) or {"owner": getattr(owner, "label", "browser"),
                               "cleanup_error": type(failure).__name__})
                cleanup_failures.append(type(failure).__name__)
        for desktop in reversed(self.desktops):
            failures = []
            try:
                desktop.close()
            except BaseException as failure:
                failures.append(type(failure).__name__)
            if desktop.terminal():
                try:
                    desktop.release_lease()
                except BaseException as failure:
                    failures.append(type(failure).__name__)
            cleanup.append({"desktop": desktop.name, "exit": desktop.process.poll() if desktop.process else None,
                            "terminal": desktop.terminal(), "errors": failures})
            cleanup_failures.extend(failures)
        terminal = all(owner.terminal() for owner in owners) and all(desktop.terminal() for desktop in self.desktops)
        if terminal:
            try:
                assert (self.private / ".owner").read_text() == str(self.output), "private owner mismatch"
                shutil.rmtree(self.private)
            except BaseException as failure:
                cleanup_failures.append(type(failure).__name__)
        report.update(seconds=time.monotonic() - started, cleanup=cleanup, cleanup_failures=cleanup_failures,
                      all_owned_processes_terminal=terminal, private_state_removed=not self.private.exists())
        end = {}
        for path in self.controller_sources:
            try:
                end[path] = hashlib.sha256(Path(path).read_bytes()).hexdigest()
            except BaseException as failure:
                end[path] = None
                cleanup_failures.append(type(failure).__name__)
        report["controller_source_drift"] = [path for path, digest in self.controller_sources.items() if end[path] != digest]
        if cleanup_failures or not terminal or not report["private_state_removed"] or report["controller_source_drift"]:
            report["status"] = "failed"
        (self.output / "controller-end.json").write_text(json.dumps(end, indent=2))
        (self.output / "result.json").write_text(json.dumps(report, indent=2))
        if primary_failure is not None:
            raise primary_failure
        assert report["status"] == "pass", "clipboard acceptance cleanup or controller validation failed; raw receipt retained"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", choices=["core", "guards", "retention", "all"], default="all")
    Runner(parser.parse_args()).run()


if __name__ == "__main__":
    main()
