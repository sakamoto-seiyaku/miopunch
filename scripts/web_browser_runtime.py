#!/usr/bin/env python3
"""Private-pipe host for independent WASM nodes and actual isolated content.

The ordinary browser uses a new profile. Only the newly owned content
certificate gets an exact SPKI exception; upstream TLS is verified by provider.
"""
import hashlib
import json
from pathlib import Path
import queue
import shutil
import signal
import subprocess
import threading

from check_forwarding import isolated_environment
from web_content_runtime import Descendants
from web_process_owner import register_process
from web_network_namespace import validate_namespace

ROOT = Path(__file__).resolve().parents[1]


class BrowserWebRequests:
    """Browser Adapter for the gateway's owned HTTP request Interface."""

    def __init__(self, peer):
        self.peer, self.browser = peer, None
        self.callbacks = {"begin": self.begin, "result": self.result, "cancel": self.cancel}

    def bind(self, browser):
        assert self.browser is None, "Web request Adapter already bound"
        self.browser = browser

    def request(self, arguments):
        return self.browser.call("request_web", self.peer, json.dumps(arguments))

    def begin(self, arguments):
        return self.browser.call("begin_web_request", self.peer, json.dumps(arguments))

    def result(self, identifier):
        return self.browser.call("web_request_result", identifier)

    def cancel(self, identifier):
        return self.browser.call("cancel_web_request", identifier)


class WebBrowser:
    def __init__(self, runner, args, gateway=None):
        boundary = validate_namespace()
        runner.freeze_source(Path(__file__).with_name("web_network_namespace.py"))
        runner.record("controller", "browser_loopback_only_boundary", boundary)
        self.runner, self.next = runner, 0
        self.pending, self.pending_lock = {}, threading.Lock()
        self.ready = queue.Queue(maxsize=1)
        self.error, self.accounted = None, False
        build = json.loads(args.browser_receipt.read_text())
        assert build["exit"] == build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"]
        actual = {path.relative_to(args.package).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                  for path in args.package.rglob("*") if path.is_file()}
        assert actual == build["artifacts"], "WASM package changed"
        harness = ROOT / "browser-probe/harness/forwarding.mjs"
        for source in [Path(__file__), harness, ROOT / "browser-probe/harness/node_modules/playwright-core/package.json",
                       ROOT / "browser-probe/harness/node_modules/playwright-core/lib/coreBundle.js"]:
            runner.freeze_source(source)
        entry_site = getattr(args, "entry_site", None)
        assert gateway is not None or entry_site is not None, "a declared browser entry is required"
        options = runner.private / "browser-web-options.json"
        if gateway is not None:
            options.write_text(json.dumps({"contentOrigin": gateway.address, "contentSPKI": gateway.spki,
                              "managementHost": gateway.management_host, "certificate": str(gateway.certificate), "key": str(gateway.key_path)}))
        self.stderr = (runner.output / "browser-host.stderr").open("w")
        environment = isolated_environment()
        runtime = runner.private / "browser-runtime"
        runtime.mkdir(mode=0o700)
        environment.update(DBUS_SESSION_BUS_ADDRESS="unix:path=" + str(runtime / "no-session-bus"),
                           DBUS_SYSTEM_BUS_ADDRESS="unix:path=" + str(runtime / "no-system-bus"),
                           XDG_RUNTIME_DIR=str(runtime))
        environment.pop("DEBUG", None)
        environment.pop("SSLKEYLOGFILE", None)
        for name in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"]:
            directory = runtime / name.lower()
            directory.mkdir(mode=0o700)
            environment[name] = str(directory)
        command = [shutil.which("node"), str(harness), str(args.package.resolve()),
                   str(args.chromium.resolve()), str(runner.private / "browser-profiles"), str(options) if gateway is not None else ""]
        if entry_site is not None:
            command.append(str(entry_site.resolve()))
        self.process = subprocess.Popen(command,
                                       env=environment, cwd=ROOT, stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=self.stderr, text=True)
        self.owner = None
        self.descendants = None
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader_started = False
        runner.nodes.append(self)
        self.owner = register_process(self.process, runner.output / "browser-host-owner.jsonl")
        self.descendants = Descendants(runner, self.process, "browser-host")
        self.reader.start()
        self.reader_started = True
        assert self.ready.get(timeout=10)["event"] == "ready"
        runner.record("controller", "browser_package", {"package_sha256": build["packageSHA256"],
                      "source_receipt": str(args.browser_receipt), "chromium_sha256": hashlib.sha256(args.chromium.read_bytes()).hexdigest(),
                      "own_content_certificate_spki_pinned": gateway is not None,
                      "main_storage_origin_host": gateway.management_host if gateway else "owned localhost platform entry",
                      "content_origin_host": gateway.content_host if gateway else None,
                      "same_site_separate_https_origins": gateway is not None,
                      "packaged_platform_entry": str(entry_site) if entry_site else None})

    def read(self):
        try:
            while line := self.process.stdout.readline(12 * 1024 * 1024 + 1):
                if len(line) > 12 * 1024 * 1024:
                    raise RuntimeError("browser RPC response exceeded module bound")
                value = json.loads(line)
                if value.get("event") == "ready":
                    self.ready.put(value, timeout=2)
                    continue
                with self.pending_lock:
                    receiver = self.pending.get(value["id"])
                if receiver is None:
                    raise RuntimeError("unexpected browser response identity")
                receiver.put(value, timeout=2)
        except BaseException as failure:
            self.error = type(failure).__name__
        finally:
            with self.pending_lock:
                for receiver in self.pending.values():
                    try:
                        receiver.put_nowait(None)
                    except queue.Full:
                        pass

    def call(self, operation, *arguments, ok=True, actor="wasm"):
        receiver = queue.Queue(maxsize=1)
        with self.pending_lock:
            assert len(self.pending) < 16, "browser controller pending bound"
            self.next += 1
            identifier = self.next
            self.pending[identifier] = receiver
            self.process.stdin.write(json.dumps(dict(id=identifier, actor=actor, op=operation, args=arguments)) + "\n")
            self.process.stdin.flush()
        try:
            value = receiver.get(timeout=90)
            assert value is not None, "browser host ended before public response"
            public = {key: result for key, result in value.items() if key != "privateError"}
            if operation in {"export_recovery_card", "invitation", "precode_invitation", "issue_service_guest"}:
                public.pop("result", None)
                public["private_return"] = value["ok"]
            self.runner.record(actor, operation, public)
            assert value["ok"] == ok, {"operation": operation, "failure_class": value.get("failureClass"), "ok": value["ok"]}
            return value.get("result")
        finally:
            with self.pending_lock:
                self.pending.pop(identifier, None)

    def close(self):
        errors, actors = [], []
        if self.process.poll() is None and self.reader_started:
            try:
                result = self.call("close", actor="runner")
                actors = result["cleanup"]
                self.accounted = result["server_closed"] and all(item["terminal"] for item in actors)
                if not all(item["closed"] for item in actors):
                    errors.append("BrowserActorCleanup")
            except BaseException as failure:
                errors.append(type(failure).__name__)
        try:
            self.process.stdin.close()
            self.process.wait(timeout=20)
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
        for source in (self.runner.private / "browser-profiles").glob("*/m119-process-owner.jsonl"):
            destination = self.runner.output / ("browser-actor-" + source.parent.name + "-owner.jsonl")
            destination.write_bytes(source.read_bytes())
        self.cleanup = {"owner": "browser-host", "exit": self.process.poll(), "reader_joined": not self.reader.is_alive(),
                        "actors": actors, "actors_accounted": self.accounted, "errors": errors, "reader_error": self.error}
        assert self.process.returncode == 0 and self.terminal() and not errors and not self.error, self.cleanup
        return self.cleanup

    def terminal(self):
        return self.process.poll() is not None and not self.reader.is_alive() and self.accounted and (
            self.owner is None or self.owner.pidfd is None) and (self.descendants is None or self.descendants.settled)
