#!/usr/bin/env python3
"""Fresh authenticated desktops and exact child ownership for M119 hosts.

No clipboard operation is present. Every display, bus and control pipe belongs
to this run; inherited global desktop and bus contexts are removed.
"""
import json
import hashlib
import os
from pathlib import Path
import queue
import secrets
import signal
import socket
import subprocess
import shutil
import sys
import threading
import time

from check_forwarding import isolated_environment
from web_process_owner import register_process, run_process
from forwarding_local_resources import identity

PROBE = '''import ctypes,ctypes.util,json,os
x=ctypes.CDLL(ctypes.util.find_library('X11'))
x.XOpenDisplay.argtypes=[ctypes.c_char_p];x.XOpenDisplay.restype=ctypes.c_void_p
x.XCloseDisplay.argtypes=[ctypes.c_void_p]
d=x.XOpenDisplay(None)
if d:x.XCloseDisplay(d)
print(json.dumps({'connected':bool(d),'mount_namespace':os.readlink('/proc/self/ns/mnt'),'network_namespace':os.readlink('/proc/self/ns/net')}))
raise SystemExit(0 if d else 1)
'''


class Descendants:
    """Read only descendants of this exact fresh Popen; never signals a PID."""
    def __init__(self, runner, process, label):
        self.runner, self.process, self.label = runner, process, label
        self.root_identity = identity(process.pid)
        self.known, self.errors = {}, []
        self.stop = threading.Event()
        self.settled = False
        self.thread = threading.Thread(target=self.observe, daemon=True)
        self.thread.start()

    def observe(self):
        while not self.stop.is_set():
            pending = [(self.process.pid, self.root_identity["start_ticks"])]
            visited = set()
            while pending:
                pid, expected_start = pending.pop()
                if pid in visited:
                    continue
                visited.add(pid)
                try:
                    if identity(pid)["start_ticks"] != expected_start:
                        continue
                    path = Path("/proc") / str(pid) / "task" / str(pid) / "children"
                    children = [int(value) for value in path.read_text().split()]
                    for child in children:
                        value = identity(child)
                        key = (child, value["start_ticks"])
                        if key not in self.known:
                            if len(self.known) >= 64:
                                raise RuntimeError("owned descendant observation bound exceeded")
                            self.known[key] = value
                        pending.append((child, value["start_ticks"]))
                except (FileNotFoundError, ProcessLookupError):
                    pass
                except BaseException as failure:
                    self.errors.append(type(failure).__name__)
            self.stop.wait(0.02)

    @staticmethod
    def absent(value):
        try:
            return identity(value["pid"])["start_ticks"] != value["start_ticks"]
        except (FileNotFoundError, ProcessLookupError):
            return True

    def close(self):
        if self.settled:
            return
        self.stop.set()
        self.thread.join(timeout=3)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and not all(self.absent(value) for value in self.known.values()):
            time.sleep(0.05)
        rows = [dict(value, original_identity_absent=self.absent(value)) for value in self.known.values()]
        self.settled = not self.thread.is_alive() and not self.errors and all(row["original_identity_absent"] for row in rows)
        self.runner.record(self.label, "known_descendant_termination", {"observed_descendants": rows,
                           "observer_joined": not self.thread.is_alive(), "errors": self.errors, "all_known_absent": self.settled,
                           "signals_to_descendants": False, "scope": "only this fresh Popen subtree observed during its lifetime"})
        assert self.settled, "a known owned descendant has not been proved terminal"


class PrivateDesktop:
    def __init__(self, runner, label):
        self.runner, self.label = runner, label
        self.process, self.owner, self.lease = None, None, None
        self.descendants = None
        self.errors = []
        self.probe_sequence = 0
        runner.desktops.append(self)
        runner.nodes.append(self)
        for _ in range(32):
            number = 2000 + secrets.randbelow(50000)
            lease = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                lease.bind("\0miopunch-m119-display-" + str(number))
                probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                try:
                    probe.bind("\0/tmp/.X11-unix/X" + str(number))
                finally:
                    probe.close()
                if Path("/tmp/.X11-unix/X" + str(number)).exists():
                    lease.close()
                    continue
                self.lease = lease
                break
            except OSError:
                lease.close()
        assert self.lease, "no fresh private display available"
        self.root = runner.private / label
        self.root.mkdir(mode=0o700)
        self.x11_directory = self.root / "x11"
        self.x11_directory.mkdir(mode=0o700)
        authority = self.root / "authority"
        authority.touch(mode=0o600)
        self.env = isolated_environment()
        self.env.update(DISPLAY=":" + str(number), XAUTHORITY=str(authority), GDK_BACKEND="x11",
                        XDG_RUNTIME_DIR=str(self.root), XDG_SESSION_TYPE="x11", MOZ_ENABLE_WAYLAND="0",
                        DBUS_SESSION_BUS_ADDRESS="unix:path=" + str(self.root / "no-bus"),
                        DBUS_SYSTEM_BUS_ADDRESS="unix:path=" + str(self.root / "no-system-bus"))
        self.env.pop("XAUTHLOCALHOSTNAME", None)
        self.env["NO_AT_BRIDGE"] = "1"
        for name in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"]:
            path = self.root / name.lower()
            path.mkdir(mode=0o700)
            self.env[name] = str(path)
        result = run_process(["xauth", "-f", str(authority), "source", "-"], input=
                                "add :%d MIT-MAGIC-COOKIE-1 %s\n" % (number, secrets.token_hex(16)),
                                env=self.env, receipt=runner.output / (label + "-xauth-owner.jsonl"), timeout=5, owners=runner.nodes)
        assert result.returncode == 0, "private display authentication setup failed"
        self.log = (runner.output / (label + ".stderr")).open("w")
        self.process = subprocess.Popen(self.namespace_command() + ["/usr/bin/Xvfb", ":" + str(number), "-auth", str(authority), "-screen", "0", "800x600x24",
                                         "-nolisten", "tcp", "-noreset"],
                                        env=self.env, stdout=self.log, stderr=self.log)
        self.owner = register_process(self.process, runner.output / (label + "-owner.jsonl"))
        self.descendants = Descendants(runner, self.process, label)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            assert self.process.poll() is None, "private display exited"
            if self.probe(self.env):
                break
            time.sleep(0.05)
        else:
            raise AssertionError("private authenticated display did not start")
        assert (self.x11_directory / ("X" + str(number))).is_socket(), "owned named X11 socket was not created"
        assert not Path("/tmp/.X11-unix/X" + str(number)).exists(), "owned display appeared in global X11 directory"
        self.number = number
        runner.record("desktop", "authenticated_private_display", {"display": ":" + str(number), "private_auth": True,
                       "private_named_socket": True, "global_selected_path_absent": True, "global_context_removed": True})

    def namespace_command(self):
        # Every mount destination below is in bwrap's NEW mount namespace.
        # The host /tmp/.X11-unix and user's home are never mounted or changed.
        # WebKitGTK 2.50.6's BubblewrapLauncher requires these five read-only
        # sysfs trees even for its DBus proxy. Keep its nested sandbox enabled.
        # https://github.com/WebKit/WebKit/blob/webkitgtk-2.50.6/Source/WebKit/UIProcess/Launcher/glib/BubblewrapLauncher.cpp
        required = ["/sys/block", "/sys/bus", "/sys/class", "/sys/dev", "/sys/devices"]
        assert all(Path(path).is_dir() for path in required), "WebKit sandbox system paths unavailable"
        system = [item for path in required for item in ["--ro-bind", path, path]]
        return ["/usr/bin/bwrap", "--unshare-pid", "--die-with-parent", "--ro-bind", "/usr", "/usr",
                "--ro-bind", "/etc", "/etc", "--symlink", "usr/bin", "/bin", "--symlink", "usr/lib", "/lib",
                "--symlink", "usr/lib64", "/lib64", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp",
                "--dir", "/run", "--dir", "/var", "--dir", "/var/cache", "--dir", "/var/tmp",
                "--bind", str(self.root), str(self.root),
                "--bind", str(self.x11_directory), "/tmp/.X11-unix", *system, "--chdir", "/"]

    def probe(self, env, private_network=False):
        self.probe_sequence += 1
        command = self.namespace_command() + (["--unshare-net"] if private_network else []) + ["/usr/bin/python3", "-c", PROBE]
        result = run_process(command, env=env, receipt=self.runner.output / (self.label + "-probe-%d-owner.jsonl" % self.probe_sequence), timeout=3,
                             owners=self.runner.nodes)
        if result.returncode == 0:
            observed = json.loads(result.stdout)
            assert observed["mount_namespace"] != os.readlink("/proc/self/ns/mnt")
            if private_network:
                assert observed["network_namespace"] != os.readlink("/proc/self/ns/net")
                self.runner.record("desktop", "sandbox_like_named_socket_precondition", {"authenticated_connected": True,
                                   "different_private_mount_namespace": True, "different_private_network_namespace": True,
                                   "host_selected_socket_absent": not Path("/tmp/.X11-unix/X" + str(self.number)).exists()})
        return result.returncode == 0

    def close(self):
        errors = []
        if self.process:
            try:
                if self.process.poll() is None:
                    self.owner.send(signal.SIGTERM)
                self.process.wait(timeout=5)
            except BaseException as failure:
                errors.append(type(failure).__name__)
            if self.process.poll() is not None and self.owner:
                self.owner.close()
            if self.process.poll() is not None and self.descendants:
                try:
                    self.descendants.close()
                except BaseException as failure:
                    errors.append(type(failure).__name__)
        if hasattr(self, "log"):
            self.log.close()
        if self.lease and self.terminal():
            self.lease.close()
            self.lease = None
        if hasattr(self, "number"):
            assert not Path("/tmp/.X11-unix/X" + str(self.number)).exists(), "global selected X11 path changed"
        assert not errors and self.terminal(), errors
        return {"owner": self.label, "exit": self.process.poll() if self.process else None, "errors": errors}

    def terminal(self):
        return self.process is None or (self.process.poll() is not None and (self.owner is None or self.owner.pidfd is None)
                and (self.descendants is None or self.descendants.settled))


class PrivateBus:
    def __init__(self, runner, desktop):
        self.stderr = (runner.output / "private-bus.stderr").open("w")
        configuration = desktop.root / "bus.conf"
        configuration.write_text('<busconfig><type>session</type><listen>unix:path=' + str(desktop.root / "bus") +
            '</listen><auth>EXTERNAL</auth><policy context="default"><allow own="*"/><allow send_destination="*"/>'
            '<allow receive_sender="*"/></policy></busconfig>')
        configuration.chmod(0o600)
        # No system/session service directories are imported: this own bus
        # cannot auto-activate the user's portal, keyring or desktop services.
        self.process = subprocess.Popen(["dbus-daemon", "--config-file=" + str(configuration), "--nofork", "--print-address=1"],
                                       env=desktop.env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=self.stderr, text=True)
        self.owner, self.descendants = None, None
        self.queue = queue.Queue(maxsize=16)
        self.error = None
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader_started = False
        runner.buses.append(self)
        runner.nodes.append(self)
        self.owner = register_process(self.process, runner.output / "private-bus-owner.jsonl")
        self.descendants = Descendants(runner, self.process, "private-bus")
        self.reader.start()
        self.reader_started = True
        address = self.queue.get(timeout=5)
        assert address.startswith("unix:path=" + str(desktop.root / "bus"))
        desktop.env["DBUS_SESSION_BUS_ADDRESS"] = address

    def read(self):
        try:
            for line in self.process.stdout:
                self.queue.put(line.strip(), timeout=2)
        except BaseException as failure:
            self.error = type(failure).__name__

    def close(self):
        errors = []
        try:
            if self.process.poll() is None:
                if self.owner is None:
                    raise RuntimeError("private bus launch unqualified; no signal sent")
                self.owner.send(signal.SIGTERM)
        except BaseException as failure:
            errors.append(type(failure).__name__)
        try:
            self.process.wait(timeout=5)
        except BaseException as failure:
            errors.append(type(failure).__name__)
        if self.reader_started:
            self.reader.join(timeout=3)
        self.stderr.close()
        if self.owner and self.process.poll() is not None:
            self.owner.close()
        if self.descendants:
            try:
                self.descendants.close()
            except BaseException as failure:
                errors.append(type(failure).__name__)
        self.cleanup = {"owner": "private-bus", "exit": self.process.returncode,
                        "reader_joined": not self.reader.is_alive(), "errors": errors}
        assert self.terminal() and not self.error and not errors, self.cleanup
        return self.cleanup

    def terminal(self):
        return self.process.poll() is not None and not self.reader.is_alive() and (self.owner is None or self.owner.pidfd is None) and (self.descendants is None or self.descendants.settled)


class WebKitHost:
    platform = "Linux/WebKit"
    def __init__(self, runner, desktop, gateway):
        self.runner, self.sequence = runner, 0
        self.stderr = (runner.output / "webkit-host.stderr").open("w")
        script = Path(__file__).with_name("web_content_host.py")
        runner.freeze_source(script)
        private_script = desktop.root / script.name
        shutil.copyfile(script, private_script)
        assert hashlib.sha256(private_script.read_bytes()).hexdigest() == runner.controller_sources[str(script.resolve())]
        public_certificate = desktop.root / "gateway-public-certificate.pem"
        shutil.copyfile(gateway.certificate, public_certificate)
        self.process = subprocess.Popen(desktop.namespace_command() + ["/usr/bin/python3", str(private_script), gateway.bootstrap,
                                       str(public_certificate), gateway.proxy_address], env=desktop.env,
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr, text=True)
        self.owner, self.descendants = None, None
        self.queue, self.error = queue.Queue(maxsize=32), None
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader_started = False
        runner.hosts.append(self)
        runner.nodes.append(self)
        self.owner = register_process(self.process, runner.output / "webkit-host-owner.jsonl")
        self.descendants = Descendants(runner, self.process, "webkit-host")
        self.reader.start()
        self.reader_started = True
        value = self.queue.get(timeout=20)
        assert value and value.get("event") == "ready", "WebKit host did not start"
        assert value["ephemeral_context"] and value["sandbox_enabled"] and not value["native_management_handlers"]
        runner.record("host", "actual_webkit_ready", value)

    def read(self):
        try:
            for line in self.process.stdout:
                value = json.loads(line)
                if value.get("event") and value["event"] != "ready":
                    self.runner.record("host", "actual_content_event", value)
                    continue
                self.queue.put(value, timeout=2)
        except BaseException as failure:
            self.error = type(failure).__name__
        finally:
            self.queue.put(None, timeout=2)

    def call(self, operation, **arguments):
        self.sequence += 1
        self.process.stdin.write(json.dumps(dict(id=self.sequence, op=operation, **arguments)) + "\n")
        self.process.stdin.flush()
        value = self.queue.get(timeout=20)
        assert value and value.get("id") == self.sequence and value.get("ok"), "WebKit control operation failed"
        return value["result"]

    def evaluate(self, expression):
        return self.call("evaluate", expression=expression)

    def close(self):
        errors = []
        if self.process.poll() is None and self.reader_started:
            try:
                self.call("close")
            except BaseException as failure:
                errors.append(type(failure).__name__)
        try:
            self.process.stdin.close()
            self.process.wait(timeout=10)
        except BaseException as failure:
            errors.append(type(failure).__name__)
        if self.process.poll() is None and self.owner:
            try:
                self.owner.send(signal.SIGTERM)
                self.process.wait(timeout=5)
            except BaseException as failure:
                errors.append(type(failure).__name__)
        if self.reader_started:
            self.reader.join(timeout=3)
        self.stderr.close()
        if self.process.poll() is not None:
            if self.owner:
                self.owner.close()
            if self.descendants:
                try:
                    self.descendants.close()
                except BaseException as failure:
                    errors.append(type(failure).__name__)
        assert self.process.returncode == 0 and self.terminal() and not errors and not self.error, errors
        return {"owner": "webkit-host", "exit": self.process.returncode, "reader_joined": True, "errors": []}

    def terminal(self):
        return self.process.poll() is not None and not self.reader.is_alive() and (self.owner is None or self.owner.pidfd is None) and (self.descendants is None or self.descendants.settled)
