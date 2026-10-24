#!/usr/bin/env python3
"""Real WASM/system-clipboard acceptance under actual Chromium permissions.

Runs separate Native and headed Chromium X11 system sessions. An independent
xclip client observes the browser's OS clipboard, rather than a JS text buffer.
"""
import argparse
import hashlib
import json
from pathlib import Path
import queue
import subprocess
import threading
import time

from check_clipboard import Runner, Desktop, Host

ROOT = Path(__file__).resolve().parents[1]


class Browser:
    def __init__(self, runner, args, desktop):
        self.runner, self.sequence = runner, 0
        self.cleanup_result = None
        self.process, self.reader, self.log = None, None, None
        runner.browser = self
        for path in [ROOT / "browser-probe/harness/clipboard.mjs",
                     ROOT / "browser-probe/harness/node_modules/playwright-core/package.json",
                     ROOT / "browser-probe/harness/node_modules/playwright-core/lib/coreBundle.js"]:
            runner.freeze_source(path)
        runner.record("browser", "actual_chromium_binary", {"path": str(args.chromium.resolve()),
                      "sha256": hashlib.sha256(args.chromium.read_bytes()).hexdigest()})
        self.log = (runner.output / "browser.stderr").open("w")
        self.process = subprocess.Popen(
            ["node", str(ROOT / "browser-probe/harness/clipboard.mjs"), str(args.package.resolve()),
             str(args.chromium.resolve()), str(runner.private / "browser-profiles")],
            cwd=ROOT, env=desktop.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=self.log, text=True, start_new_session=True)
        self.lines = queue.Queue()
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()
        assert self.receive()["event"] == "ready"

    def read(self):
        for line in self.process.stdout:
            self.lines.put(json.loads(line))
        self.lines.put(None)

    def receive(self):
        response = self.lines.get(timeout=90)
        assert response is not None, "browser host exited before response"
        return response

    def call(self, method, *args, ok=True, actor="wasm"):
        self.sequence += 1
        identifier = self.sequence
        self.process.stdin.write(json.dumps(dict(id=identifier, actor=actor, op=method, args=args)) + "\n")
        self.process.stdin.flush()
        response = self.receive()
        public = {key: value for key, value in response.items() if key != "privateError"}
        if method in {"export_recovery_card", "invitation", "precode_invitation", "issue_guest", "issue_service_guest"}:
            public.pop("result", None)
            public["privateReturn"] = response["ok"]
        self.runner.record(actor, method, public)
        if not response["ok"]:
            # The product errors contain safe reason codes, never clipboard text.
            with (self.runner.output / "browser-errors.jsonl").open("a") as out:
                out.write(json.dumps(response) + "\n")
        assert response.get("id") == identifier and response["ok"] is ok, public
        return response.get("result")

    def close(self):
        if self.cleanup_result is not None:
            if self.cleanup_result["errors"]:
                raise RuntimeError("owned browser cleanup previously failed")
            return self.cleanup_result
        failures, actors = [], []
        accounted = self.process is None
        if self.process is None:
            self.cleanup_result = {"client": "browser", "exit": None, "reader_joined": True,
                                   "actors": [], "all_actor_processes_terminal": True, "errors": []}
            if self.log is not None:
                self.log.close()
            return self.cleanup_result
        if self.process.poll() is None:
            try:
                result = self.call("close", actor="runner")
                actors = result["cleanup"]
                accounted = True
                if not all(item["closed"] for item in actors) or not result["server_closed"]:
                    failures.append("BrowserActorCleanup")
            except BaseException as failure:
                failures.append(type(failure).__name__)
            try:
                self.process.stdin.close()
            except BaseException as failure:
                failures.append(type(failure).__name__)
        else:
            failures.append("BrowserHostExitedBeforeCleanup")
        try:
            if self.process.wait(timeout=25) != 0:
                failures.append("NonzeroExit")
        except BaseException as failure:
            failures.append(type(failure).__name__)
        self.reader.join(timeout=3)
        if self.reader.is_alive():
            failures.append("ReaderNotJoined")
        try:
            self.log.close()
        except BaseException as failure:
            failures.append(type(failure).__name__)
        self.cleanup_result = {"client": "browser", "exit": self.process.poll(),
                               "reader_joined": not self.reader.is_alive(), "actors": actors,
                               "all_actor_processes_terminal": accounted and all(item["terminal"] for item in actors),
                               "errors": failures}
        self.runner.record("browser", "cleanup", self.cleanup_result)
        if failures:
            raise RuntimeError("owned browser cleanup failed")
        return self.cleanup_result

    def terminal(self):
        return (self.process is None or self.process.poll() is not None) \
            and (self.reader is None or not self.reader.is_alive()) and self.cleanup_result is not None \
            and self.cleanup_result["all_actor_processes_terminal"]


class BrowserRunner(Runner):
    def __init__(self, args):
        # Reject a missing/mismatched package before allocating private state.
        build = json.loads(args.build_receipt.read_text())
        assert build["exit"] == build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"]
        actual = {path.relative_to(args.package).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                  for path in args.package.rglob("*") if path.is_file()}
        assert actual == build["artifacts"], "immutable browser package precondition failed"
        assert args.chromium.is_file(), "pinned Chromium precondition failed"
        super().__init__(args)
        self.args = args
        self.record("runner", "browser_package", {"package_sha256": build["packageSHA256"],
                    "build_receipt": str(args.build_receipt), "headed": True,
                    "harness_sha256": hashlib.sha256((ROOT / "browser-probe/harness/clipboard.mjs").read_bytes()).hexdigest()})

    def scenario(self):
        if self.args.case == "platform":
            desktop = Desktop(self, "browser-platform")
            try:
                browser = Browser(self, self.args, desktop)
                browser.call("open")
                assert browser.call("document_context")["focused"]
                desktop.input_focus()
                self.until(lambda: not browser.call("document_context")["focused"], "real OS blur prerequisite")
                desktop.input_focus(restore=True)
                self.until(lambda: browser.call("document_context")["focused"], "real OS focus restore prerequisite")
                self.record("browser-platform", "actual_focus_preflight", {"actual_blur_and_restore": True,
                            "automation_focus_emulation_removed": True, "no_clipboard_or_network_effect": True})
            finally:
                if hasattr(self, "browser"):
                    self.browser.close()
            return
        native_desktop, browser_desktop = Desktop(self, "native"), Desktop(self, "browser")
        native_desktop.copy("M117 owned native initial")
        browser_desktop.copy("M117 owned browser initial")
        assert native_desktop.read() != browser_desktop.read()
        native = Host(self, "native", native_desktop)
        native.call("clipboard_enable")
        invitation = self.private / "browser-invitation"
        native.call("issue_precode", path=str(invitation), seconds=3600, max_uses=1)
        try:
            browser = Browser(self, self.args, browser_desktop)
            browser.call("open")
            configuration = json.dumps({"rtc_enabled": False, "relays": ["https://aps1-1.relay.n0.iroh.link./"]})
            original = browser.call("start", "join", configuration, invitation.read_text())
            assert browser.call("request_join", False) == "Complete"
            available = browser.call("clipboard_status")
            assert not available["capability"]["read"] and available["capability"]["reason"]
            assert not next(item for item in browser.call("service_status")["services"] if item["service"] == "clipboard")["capable"]
            browser.call("enable_clipboard", ok=False)
            browser.call("grant_clipboard")
            available = browser.call("enable_clipboard")
            assert available["enabled"] and available["capability"]["read"] and available["capability"]["write"]
            wasm_address = dict(id=original["id"], addrs=[{"Relay": "https://aps1-1.relay.n0.iroh.link./"}])
            native.call("clipboard_start", address=wasm_address)
            first = "M117 actual Native→WASM system write\n真实剪贴板"
            native_desktop.copy(first)
            self.until(lambda: browser_desktop.read() == first, "Native to actual browser system clipboard", 30)
            self.until(lambda: native.call("clipboard_status")["acknowledged"] == 1, "browser actual-write acknowledgment")
            reverse = "M117 actual WASM→Native automatically采集"
            browser_desktop.copy(reverse)
            self.until(lambda: native_desktop.read() == reverse, "browser automatic capture to native system", 30)
            self.until(lambda: browser.call("clipboard_status")["acknowledged"] == 1, "native actual-write acknowledgment")
            time.sleep(.8)
            assert native.call("clipboard_status")["sent"] == 1
            assert browser.call("clipboard_status")["sent"] == 1
            self.record("pair", "real_wasm_system_duplex", {"independent_system_reads": True, "one_change_each": True,
                        "native_initiator": True, "actual_context_permissions": True})
            browser.call("revoke_clipboard")
            self.until(lambda: browser.call("clipboard_status")["peer"] is None, "actual permission loss stops browser")
            self.until(lambda: native.call("clipboard_status")["peer"] is None, "permission loss closes pair")
            unavailable = browser.call("clipboard_status")
            assert not unavailable["capability"]["read"] and unavailable["capability"]["reason"]
            assert not next(item for item in browser.call("service_status")["services"] if item["service"] == "clipboard")["capable"]
            native_desktop.copy("M117 owned after actual browser permission loss")
            time.sleep(.8)
            assert browser_desktop.read() == reverse
            browser.call("grant_clipboard")
            assert browser.call("clipboard_status")["peer"] is None
            browser.call("start_clipboard", native.node)
            resumed = "M117 real browser initiator after permission restored"
            native_desktop.copy(resumed)
            self.until(lambda: browser_desktop.read() == resumed, "same system operation after permission restore", 30)
            browser_desktop.input_focus()
            self.until(lambda: browser.call("document_context")["focused"] is False, "actual document focus-loss prerequisite")
            self.until(lambda: browser.call("clipboard_status")["peer"] is None, "unfocused document stops automatic clipboard")
            native_desktop.copy("M117 owned after browser context loss")
            time.sleep(.8)
            assert browser_desktop.read() == resumed
            assert not browser.call("clipboard_status")["capability"]["read"]
            assert not next(item for item in browser.call("service_status")["services"] if item["service"] == "clipboard")["capable"]
            browser_desktop.input_focus(restore=True)
            self.until(lambda: browser.call("document_context")["focused"] is True, "actual document focus-restoration prerequisite")
            browser.call("start_clipboard", native.node)
            browser.call("disable_clipboard")
            self.until(lambda: native.call("clipboard_status")["peer"] is None, "browser disable stops native")
            native_desktop.copy("M117 owned after browser disable")
            time.sleep(.8)
            assert browser_desktop.read() == resumed
            self.record("browser", "permission_focus_disable", {"actual_permission_loss_stops_effects": True,
                        "actual_focus_loss_stops_effects": True, "disabled_stops_effects": True,
                        "explicit_repair_resumes_same_core": True, "browser_initiator": True})
            browser.call("enable_clipboard")
            browser.call("start_clipboard", native.node)
            positive = "M117 actual positive before WASM effective ACL change"
            native_desktop.copy(positive)
            self.until(lambda: browser_desktop.read() == positive, "browser ACL prerequisite", 30)
            deny = dict(members=True, guests=True, allowed=[], denied=[native.node])
            revision = browser.call("set_service_rule", original["id"], "clipboard", None, json.dumps(deny))
            self.until(lambda: native.call("clipboard_status")["peer"] is None, "browser ACL terminates actual pair")
            native_desktop.copy("M117 owned after WASM effective ACL change")
            time.sleep(.8)
            assert browser_desktop.read() == positive
            browser.call("reload")
            reopened = browser.call("start", "start", "{}", None)
            assert reopened["id"] == original["id"]
            assert not browser.call("clipboard_status")["enabled"]
            policy = next(p for p in browser.call("service_status")["policies"]
                          if p["provider"] == original["id"] and p["service"] == "clipboard")
            assert policy["revision"] == revision and policy["rule"] == deny
            self.record("browser", "effective_acl_and_reopen", {"actual_write_stopped": True,
                        "identity_and_acl_saved_in_indexeddb": True, "no_automatic_pair_on_reopen": True})
        finally:
            if hasattr(self, "browser"):
                self.browser.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    parser.add_argument("--case", choices=["all", "platform"], default="all")
    args = parser.parse_args()
    BrowserRunner(args).run()


if __name__ == "__main__":
    main()
