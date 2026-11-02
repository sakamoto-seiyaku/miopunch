#!/usr/bin/env python3
"""Own Android test processes/pages; ADB is control only, never a data-path pass."""
import argparse
import hashlib
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import re
import shlex
import signal
import subprocess
import threading
import time
import urllib.request
import uuid
import xml.etree.ElementTree as ET

from wan_environment_preflight import bounded_process

ROOT = Path(__file__).resolve().parents[1]
ADB = Path.home() / "Android/Sdk/platform-tools/adb"


def browser_cleanup_succeeded(cleanup):
    for key in ("sdkRequired", "tabRequired", "forwardRequired", "reverseRequired"):
        if not isinstance(cleanup.get(key), bool):
            return False
    if cleanup["sdkRequired"]:
        sdk = cleanup.get("sdk", {})
        observed = sdk.get("result", {})
        if (sdk.get("ok") is not True or observed.get("cleanupSucceeded") is not True
                or observed.get("networkClosed") is not True or observed.get("networkFreed") is not True
                or observed.get("ownedMemberHandles") != 0):
            return False
        storage = observed.get("storage", {})
        if not isinstance(storage.get("required"), bool):
            return False
        if storage["required"] and (storage.get("ownedDatabaseClaimed") is not True
                                   or storage.get("exclusiveStorageLock") is not True
                                   or storage.get("deleted") is not True or storage.get("outcome") != "success"
                                   or storage.get("terminal") is not True or storage.get("pending") is not False):
            return False
    if cleanup["tabRequired"] and cleanup.get("tab", {}).get("ownedTargetAbsent") is not True:
        return False
    for kind in ("forward", "reverse"):
        if cleanup[kind + "Required"] and (cleanup.get(kind + "RuleAbsent") is not True
                                          or cleanup.get(kind + "RemoveExit") != 0):
            return False
    return cleanup.get("localListenerClosed") is True and not cleanup.get("errors")


class Device:
    def __init__(self, model, transport, adb=ADB):
        self.adb, self.model, self.transport = str(adb), model, transport
        code, stdout, _, _ = bounded_process(["rtk", "proxy", self.adb, "devices", "-l"])
        matches = [line.split()[0] for line in stdout.splitlines()
                   if "model:" + model in line.split() and len(line.split()) > 1
                   and line.split()[1] == "device"
                   and ((":" in line.split()[0]) == (transport == "network"))]
        if code or len(matches) != 1:
            raise RuntimeError("expected exactly one authorized device for model/transport")
        self._serial = matches[0]  # Never part of a public receipt or error.
        self.commands = []

    def run(self, *args, required=True):
        started = time.monotonic()
        command = ["rtk", "proxy", self.adb, "-s", self._serial, *map(str, args)]
        code, stdout, stderr, cleanup = bounded_process(command)
        record = {"command": ["rtk", "proxy", self.adb, "-s", "<model-selected-device>", *map(str, args)],
                  "exit": code, "elapsedMs": round((time.monotonic() - started) * 1000, 3),
                  "stderr": stderr.replace(self._serial, "<model-selected-device>"), "cleanup": cleanup}
        self.commands.append(record)
        if required and (code or cleanup.get("forced")):
            raise RuntimeError("owned ADB operation failed; see sanitized command receipt")
        return stdout, code

    def shell(self, *args, required=True):
        return self.run("shell", shlex.join(list(map(str, args))), required=required)


class NativeActor:
    """Private JSON-line events and exact own-PID cleanup; never logs raw events."""
    def __init__(self, device):
        self.device = device
        self.directory = "/data/local/tmp/miopunch-pair-" + uuid.uuid4().hex
        device.shell("mkdir", "-m", "700", self.directory)
        self.process = None
        self.events = []
        self.condition = threading.Condition()
        self.stderr = []  # Private memory only; not a public evidence field.
        self.threads = []
        self.runs = []
        self.sequence = 0
        self.remote_pid = None
        self.binary = None
        self.stop_command = "stop"
        self.artifacts = {}

    def deploy(self, source, name, *, executable=False, artifact=False):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+", name):
            raise ValueError("owned input name must be a single safe path component")
        source = Path(source).resolve()
        remote = self.directory + "/" + name
        self.device.run("push", str(source), remote)
        self.device.shell("chmod", "700" if executable else "600", remote)
        actual, _ = self.device.shell("sha256sum", remote)
        expected = hashlib.sha256(source.read_bytes()).hexdigest()
        if not actual.startswith(expected + " "):
            raise RuntimeError("owned deployed artifact hash does not match")
        if executable:
            self.binary = remote
        if executable or artifact:
            self.artifacts[name] = expected
        return remote

    def start(self, args, *, stop_command="stop", environment=None, launcher=None):
        if not self.binary and not launcher:
            raise RuntimeError("deploy an executable before native startup")
        if self.process and self.process.poll() is None:
            raise RuntimeError("an owned native process is already active")
        if any(thread.is_alive() for thread in self.threads):
            raise RuntimeError("close the prior owned native readers before restart")
        self.events.clear()
        self.stderr.clear()
        self.remote_pid = None
        self.stop_command = stop_command
        argv = [*(launcher or [self.binary]), *map(str, args)]
        if environment:
            argv = ["env", *(str(key) + "=" + str(value) for key, value in environment.items()), *argv]
        remote = ("umask 077; echo $$ > " + shlex.quote(self.directory + "/actor.pid")
                  + "; exec " + shlex.join(argv))
        command = ["rtk", "proxy", self.device.adb, "-s", self.device._serial, "shell", remote]
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True, start_new_session=True)
        run = {"command": ["rtk", "proxy", self.device.adb, "-s", "<model-selected-device>", "shell", remote],
               "generation": uuid.uuid4().hex, "startedMonotonic": time.monotonic()}
        self.runs.append(run)

        def read_events():
            for line in self.process.stdout:
                if len(line) > 1024 * 1024:
                    with self.condition:
                        self.events.append({"event": "local-reader-failure", "failure": "native event exceeded bound"})
                        self.condition.notify_all()
                    break
                try:
                    value = json.loads(line)
                except ValueError:
                    continue
                with self.condition:
                    if len(self.events) >= 512:
                        self.events.pop(0)
                    self.events.append(value)
                    self.condition.notify_all()
            with self.condition:
                self.condition.notify_all()

        def read_errors():
            for line in self.process.stderr:
                if len(self.stderr) < 64:
                    self.stderr.append(line[:4096])

        self.threads = [threading.Thread(target=reader, daemon=True)
                        for reader in (read_events, read_errors)]
        for thread in self.threads:
            thread.start()
        return self

    def send(self, value):
        if not self.process or self.process.poll() is not None:
            raise RuntimeError("owned native process is not active")
        line = value if isinstance(value, str) else json.dumps(value)
        if "\n" in line:
            raise ValueError("one native command per line is required")
        self.process.stdin.write(line + "\n")
        self.process.stdin.flush()

    def wait_event(self, expected, timeout=20):
        predicate = expected if callable(expected) else lambda event: event.get("event") == expected
        deadline = time.monotonic() + timeout
        with self.condition:
            while True:
                for index, value in enumerate(self.events):
                    if predicate(value):
                        return self.events.pop(index)
                if self.process and self.process.poll() is not None:
                    raise RuntimeError("owned native process exited before the expected event")
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("local native event wait expired; no SDK deadline extension")
                self.condition.wait(min(remaining, 0.2))

    def command(self, value, expected=None, timeout=20):
        if isinstance(value, dict):
            self.sequence += 1
            value = {"id": self.sequence, **value}
            if expected is None:
                ident = value["id"]
                expected = lambda event: event.get("id") == ident
        elif expected is None:
            expected = value
        self.send(value)
        return self.wait_event(expected, timeout)

    def _owned_remote_process(self):
        value, code = self.device.shell("cat", self.directory + "/actor.pid", required=False)
        if code or not value.strip().isdigit():
            return None
        self.remote_pid = int(value.strip())
        current, code = self.device.shell("cat", f"/proc/{self.remote_pid}/cmdline", required=False)
        if code:
            absent, absent_code = self.device.shell("test", "!", "-e", f"/proc/{self.remote_pid}", required=False)
            return False if absent_code == 0 else None
        return self.directory in current

    def close(self, *, remove_directory=True, graceful_seconds=5):
        started = time.monotonic()
        result = {"forced": False}
        try:
            if self.process and self.process.poll() is None:
                if self.stop_command is not None:
                    try:
                        self.send(self.stop_command)
                    except (OSError, RuntimeError):
                        result["stopWriteFailed"] = True
                self.process.stdin.close()
                try:
                    self.process.wait(timeout=graceful_seconds)
                except subprocess.TimeoutExpired:
                    owned = self._owned_remote_process()
                    result["ownedPidConfirmedBeforeSignal"] = owned
                    if owned:
                        result["forced"] = True
                        self.device.shell("kill", "-TERM", self.remote_pid, required=False)
                        try:
                            self.process.wait(timeout=1)
                        except subprocess.TimeoutExpired:
                            if self._owned_remote_process():
                                self.device.shell("kill", "-KILL", self.remote_pid, required=False)
                                result["remoteKillSent"] = True
                    if self.process.poll() is None:
                        result["forced"] = True
                        try:
                            os.killpg(self.process.pid, signal.SIGTERM)
                        except ProcessLookupError:
                            pass
                        try:
                            self.process.wait(timeout=0.3)
                        except subprocess.TimeoutExpired:
                            os.killpg(self.process.pid, signal.SIGKILL)
                            self.process.wait()
            if self.process:
                result["commandExit"] = self.process.poll()
                result["leaderWaitedFor"] = self.process.returncode is not None
            owned = self._owned_remote_process() if self.process else False
            result["ownedRemoteProcessAbsent"] = owned is False
            # Independent late observation; never equate local ADB exit with remote exit.
            if self.process:
                time.sleep(0.1)
                result["ownedRemoteProcessAbsentLate"] = self._owned_remote_process() is False
            for thread in self.threads:
                thread.join(timeout=1)
            result["localReadersEnded"] = all(not thread.is_alive() for thread in self.threads)
        finally:
            if remove_directory:
                # Refuse a modified/foreign path before destructive cleanup.
                if not re.fullmatch(r"/data/local/tmp/miopunch-pair-[0-9a-f]{32}", self.directory):
                    raise RuntimeError("refusing cleanup of an unowned device directory")
                if result.get("ownedRemoteProcessAbsent", not self.process):
                    _, code = self.device.shell("rm", "-rf", "--", self.directory, required=False)
                    _, absent_code = self.device.shell("test", "!", "-e", self.directory, required=False)
                    result["ownedDirectoryRemoved"] = code == 0 and absent_code == 0
                else:
                    result["ownedDirectoryRemoved"] = False
            result["elapsedMs"] = round((time.monotonic() - started) * 1000, 3)
            if self.runs:
                self.runs[-1]["cleanup"] = result
        return result


class BrowserPage:
    """A dedicated local origin, private in-memory RPC and self-owned ADB rules."""
    def __init__(self, device, package=ROOT / "browser-probe/pkg", *, prelude=None,
                 environment_rtc_probe=True):
        self.device, self.package = device, Path(package).resolve()
        self.prelude = prelude or ""
        self.actor = uuid.uuid4().hex
        self.requests = queue.Queue()
        self.condition = threading.Condition()
        self.responses = {}
        self.ready = None
        self.terminal = None
        self.sequence = 0
        self.reverse = None
        self.forward = None
        self.browser_version = None
        self.cdp_failure = None
        self.closed = False
        self.opened = False
        self.forward_attempted = False
        self.cleanup = {}
        page = self

        class Handler(SimpleHTTPRequestHandler):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, directory=str(page.package.parent), **kwargs)

            def log_message(self, *_):
                pass  # No raw configuration, invitations, node identity or URLs.

            def end_headers(self):
                self.send_header("Cache-Control", "no-store")
                self.send_header("Cross-Origin-Opener-Policy", "same-origin")
                self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
                super().end_headers()

            def send_bytes(self, body, kind):
                self.send_response(200)
                self.send_header("Content-Type", kind)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                if self.path == page.page_path:
                    self.send_bytes(b'<!doctype html><title>miopunch owned Android SDK check</title>'
                        b'<meta charset="utf-8"><link rel="icon" href="data:,">'
                        b'<p>Owned miopunch SDK test page</p>'
                        b'<script src="/owned-prelude.js"></script>'
                        b'<script type="module" src="/android-pair-page.js"></script>', "text/html; charset=utf-8")
                elif self.path == "/owned-prelude.js":
                    self.send_bytes(page.prelude.encode(), "text/javascript; charset=utf-8")
                elif self.path == "/android-pair-page.js":
                    self.send_bytes((ROOT / "browser-probe/harness/android-pair-page.js").read_bytes(),
                                    "text/javascript; charset=utf-8")
                elif self.path == f"/control?actor={page.actor}":
                    try:
                        value = page.requests.get(timeout=0.5)
                    except queue.Empty:
                        value = None
                    self.send_bytes(json.dumps(value).encode(), "application/json")
                elif self.path.startswith("/pkg/"):
                    # Remap to the chosen package; do not serve the rest of the repo.
                    relative = Path(self.path.split("?", 1)[0].removeprefix("/pkg/"))
                    destination = (page.package / relative).resolve()
                    if not destination.is_relative_to(page.package) or not destination.is_file():
                        self.send_error(404)
                    else:
                        kind = "application/wasm" if destination.suffix == ".wasm" else "text/javascript"
                        self.send_bytes(destination.read_bytes(), kind)
                else:
                    self.send_error(404)

            def do_POST(self):
                if self.path != f"/result?actor={page.actor}":
                    self.send_error(404)
                    return
                length = int(self.headers.get("Content-Length", "0"))
                if not 0 < length <= 1024 * 1024:
                    self.send_error(413)
                    return
                try:
                    value = json.loads(self.rfile.read(length))
                    if not isinstance(value, dict):
                        raise ValueError("result must be an object")
                except ValueError:
                    self.send_error(400)
                    return
                with page.condition:
                    if value.get("event") == "ready":
                        page.ready = value
                    elif value.get("event") == "terminal":
                        page.terminal = value
                    elif "id" in value:
                        page.responses[value["id"]] = value
                    page.condition.notify_all()
                self.send_bytes(b"{}", "application/json")

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.port = self.server.server_address[1]
        self.page_path = f"/android-pair?actor={self.actor}&environmentRtcProbe={str(environment_rtc_probe).lower()}"
        self.url = f"http://127.0.0.1:{self.port}" + self.page_path

    def start(self, timeout=25):
        self.device.run("reverse", "--no-rebind", f"tcp:{self.port}", f"tcp:{self.port}")
        self.reverse = f"tcp:{self.port}"
        self.device.shell("am", "start", "--user", "0", "-a", "android.intent.action.VIEW",
                          "-d", self.url, "--ez", "create_new_tab", "true", "com.android.chrome")
        self.opened = True
        self._probe_cdp()
        deadline = time.monotonic() + timeout
        with self.condition:
            while self.ready is None and self.terminal is None:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("owned browser page did not load/report before its setup deadline")
                self.condition.wait(remaining)
        if self.terminal:
            raise RuntimeError("owned browser page reported a terminal setup error")
        return self.ready

    def _probe_cdp(self):
        self.forward_attempted = True
        port, _ = self.device.run("forward", "tcp:0", "localabstract:chrome_devtools_remote")
        if not port.strip().isdigit():
            self.cdp_failure = "forward returned no numeric port"
            return
        self.forward = f"tcp:{port.strip()}"
        self.cdp_base = f"http://127.0.0.1:{port.strip()}"
        try:
            with urllib.request.urlopen(self.cdp_base + "/json/version", timeout=3) as response:
                self.browser_version = json.load(response).get("Browser")
        except Exception as error:
            self.cdp_failure = type(error).__name__

    def call(self, op, args=None, timeout=20):
        if self.closed:
            raise RuntimeError("owned page is already closed")
        self.sequence += 1
        ident = self.sequence
        self.requests.put({"id": ident, "op": op, "args": args or {}})
        deadline = time.monotonic() + timeout
        with self.condition:
            while ident not in self.responses:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("local control wait expired; this does not extend the SDK deadline")
                self.condition.wait(remaining)
            return self.responses.pop(ident)

    def _close_owned_tab(self):
        if self.browser_version:
            try:
                with urllib.request.urlopen(self.cdp_base + "/json/list", timeout=3) as response:
                    targets = json.load(response)
                # Only act on the exact URL opened by this runner; never page-evaluate others.
                ids = [target["id"] for target in targets if target.get("url") == self.url]
                for ident in ids:
                    with urllib.request.urlopen(self.cdp_base + "/json/close/" + ident, timeout=3) as response:
                        response.read()
                with urllib.request.urlopen(self.cdp_base + "/json/list", timeout=3) as response:
                    absent = all(target.get("url") != self.url for target in json.load(response))
                return {"method": "exact owned URL CDP close", "ownedTargetAbsent": absent}
            except Exception as error:
                self.cleanup["cdpCloseFailure"] = type(error).__name__
        xml = f"/data/local/tmp/mio82-ui-{self.actor}.xml"
        try:
            self.device.shell("uiautomator", "dump", xml)
            content, _ = self.device.shell("cat", xml)
            root = ET.fromstring(content)
            def address_bars(tree):
                return [node.get("text", "") for node in tree.iter("node")
                        if node.get("package") == "com.android.chrome"
                        and node.get("resource-id") == "com.android.chrome:id/url_bar"]
            def owned_address_bar(tree):
                return any(value in (self.url, self.url.removeprefix("http://")) for value in address_bars(tree))
            visible = owned_address_bar(root)
            if not visible:
                return {"method": "UI", "ownedPageCurrentlyVisible": False,
                        "exactOwnedAddressBarConfirmed": False, "ownedTargetAbsent": False}
            self.device.shell("input", "keycombination", "KEYCODE_CTRL_LEFT", "KEYCODE_W")
            self.device.shell("uiautomator", "dump", xml)
            content, _ = self.device.shell("cat", xml)
            root = ET.fromstring(content)
            # A missing or shortened same-origin bar is not proof of tab absence.
            after_bars = address_bars(root)
            absent = bool(after_bars) and all(value and f"127.0.0.1:{self.port}" not in value
                                              for value in after_bars)
            return {"method": "exact owned Chrome address bar then Ctrl-W",
                    "exactOwnedAddressBarConfirmed": True, "ownedTargetAbsent": absent}
        except Exception as error:
            return {"method": "UI", "ownedTargetAbsent": False, "failureClass": type(error).__name__}
        finally:
            self.device.shell("rm", "-f", xml, required=False)

    def close(self):
        if self.closed:
            return self.cleanup
        started = time.monotonic()
        self.cleanup.update(sdkRequired=self.ready is not None, tabRequired=self.opened,
                            forwardRequired=self.forward_attempted, reverseRequired=self.reverse is not None,
                            errors=[])
        def failed(operation, error):
            self.cleanup["errors"].append({"operation": operation, "failureClass": type(error).__name__})
        try:
            if self.ready is not None:
                try:
                    self.cleanup["sdk"] = self.call("shutdown", timeout=8)
                except Exception as error:
                    self.cleanup["sdk"] = {"ok": False, "failureClass": type(error).__name__}
                    failed("sdk shutdown", error)
        finally:
            try:
                self.cleanup["tab"] = self._close_owned_tab() if self.opened else {
                    "openedByRunner": False, "ownedTargetAbsent": True}
            except Exception as error:
                self.cleanup["tab"] = {"ownedTargetAbsent": False, "failureClass": type(error).__name__}
                failed("owned tab close", error)
            storage = self.cleanup.get("sdk", {}).get("result", {}).get("storage", {})
            if storage.get("pending") is True:
                storage["ownedPageTerminationObserved"] = self.cleanup["tab"].get("ownedTargetAbsent") is True
            for kind, port in (("forward", self.forward), ("reverse", self.reverse)):
                if port:
                    try:
                        _, code = self.device.run(kind, "--remove", port, required=False)
                        self.cleanup[kind + "RemoveExit"] = code
                    except Exception as error:
                        self.cleanup[kind + "RemoveExit"] = None
                        failed(kind + " remove", error)
                    try:
                        rules, list_code = self.device.run(kind, "--list", required=False)
                        self.cleanup[kind + "RuleAbsent"] = list_code == 0 and not any(
                            port in line.split() for line in rules.splitlines())
                    except Exception as error:
                        self.cleanup[kind + "RuleAbsent"] = False
                        failed(kind + " absence check", error)
            for operation, callback in (("listener shutdown", self.server.shutdown),
                                        ("listener close", self.server.server_close),
                                        ("listener thread join", lambda: self.thread.join(timeout=2))):
                try:
                    callback()
                except Exception as error:
                    failed(operation, error)
            self.closed = True
            self.cleanup["localListenerClosed"] = not self.thread.is_alive() and self.server.socket.fileno() == -1
            self.cleanup["elapsedMs"] = round((time.monotonic() - started) * 1000, 3)
            self.cleanup["cleanupSucceeded"] = browser_cleanup_succeeded(self.cleanup)
        return self.cleanup


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new public capability receipt")
    parser.add_argument("--adb", type=Path, default=ADB)
    parser.add_argument("--model", action="append", choices=("Pixel_6a", "TB322FC"))
    parser.add_argument("--package", type=Path, default=ROOT / "browser-probe/pkg")
    parser.add_argument("--prelude", type=Path, help="only this owned page; runs before its WASM module")
    parser.add_argument("--skip-environment-rtc-probe", action="store_true")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("receipt exists; preserve the earlier result")
    report = {"scope": "physical Android browser formal-WASM load/control prerequisites only",
              "hostRole": "build/ADB/code delivery/control/evidence; not a business peer",
              "physicalWanQualified": False, "endpoints": [],
              "sourceSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "pageSha256": hashlib.sha256((ROOT / "browser-probe/harness/android-pair-page.js").read_bytes()).hexdigest(),
              "preludeSha256": hashlib.sha256(args.prelude.read_bytes()).hexdigest() if args.prelude else None,
              "environmentRtcProbeSkipped": args.skip_environment_rtc_probe,
              "wasmSha256": hashlib.sha256((args.package / "miopunch_browser_probe_bg.wasm").read_bytes()).hexdigest()}
    failed = False
    for model in args.model or ("Pixel_6a", "TB322FC"):
        item = {"model": model}
        device = None
        page = None
        try:
            device = Device(model, "usb" if model == "Pixel_6a" else "network", args.adb)
            page = BrowserPage(device, args.package,
                prelude=args.prelude.read_text() if args.prelude else None,
                environment_rtc_probe=not args.skip_environment_rtc_probe)
            item["ready"] = page.start()
            item["cdpAvailableAfterOwnedPageStart"] = bool(page.browser_version)
            item["browserVersion"] = page.browser_version
            item["cdpFailure"] = page.cdp_failure
            rejection = page.call("unsupported-probe-operation")
            item["unsupportedOperationRejected"] = rejection.get("ok") is False
            failed |= not item["unsupportedOperationRejected"]
        except Exception as error:
            item["failureClass"] = type(error).__name__
            failed = True
        finally:
            if page:
                item["cleanup"] = page.close()
                failed |= not browser_cleanup_succeeded(item["cleanup"])
            if device:
                item["commands"] = device.commands
            report["endpoints"].append(item)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    print(json.dumps({"receipt": str(args.output), "failed": failed, "physicalWanQualified": False}))
    return 2 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
