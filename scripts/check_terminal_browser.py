#!/usr/bin/env python3
"""Independent sandboxed Browser/WASM terminal client, on fresh owned actors."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import time

from check_forwarding import Runner
from check_terminal_service import TerminalRunner, WorkspaceObserver
from check_web_service import HttpTarget, OwnedNode
from forwarding_local_resources import configure_local_resources
from web_browser_runtime import WebBrowser
from web_content_gateway import ContentGateway
from web_process_owner import register_process
from terminal_browser_access import BrowserAccess


class BrowserTerminalNode:
    def __init__(self, browser, identity, actor="wasm"):
        self.browser, self.identity, self.actor = browser, identity, actor

    def call(self, operation, ok=True, **arguments):
        target = arguments.get("address", {}).get("id")
        if operation in {"terminal_create", "terminal_list", "terminal_end"}:
            request = {"operation": operation.removeprefix("terminal_")}
            if operation == "terminal_end":
                request["workspace"] = arguments["workspace"]
            return self.browser.call("request_terminal", target, json.dumps(request), ok=ok, actor=self.actor)
        if operation == "terminal_attach":
            return self.browser.call("attach_terminal", target, arguments["workspace"], arguments.get("columns",80), arguments.get("rows",24), ok=ok, actor=self.actor)
        if operation == "terminal_send":
            return self.browser.call("send_terminal", arguments["attachment"], json.dumps(arguments["message"]), ok=ok, actor=self.actor)
        if operation == "terminal_read":
            return self.browser.call("read_terminal", arguments["attachment"], arguments.get("timeout_ms",100), ok=ok, actor=self.actor)
        if operation in {"terminal_detach", "terminal_cancel"}:
            method = "detach_terminal" if operation == "terminal_detach" else "cancel_terminal"
            return self.browser.call(method, arguments["attachment"], ok=ok, actor=self.actor)
        if operation == "terminal_status":
            return self.browser.call("terminal_status", ok=ok, actor=self.actor)
        raise ValueError("unknown terminal adapter operation")


class BrowserRunner(TerminalRunner):
    def __init__(self, args):
        self.reused_sources = {}
        self.source_reuse = getattr(args, "reuse_controller_sources", None)
        self.source_reuse_manifest = {}
        if self.source_reuse:
            self.source_reuse = self.source_reuse.resolve()
            previous = json.loads((self.source_reuse / "result.json").read_text())
            assert previous["all_owned_terminal"] and not previous["controller_drift"]
            self.source_reuse_manifest = json.loads((self.source_reuse / "controller-start.json").read_text())
        super().__init__(args)
        # Freeze every imported local controller before scenario effects.
        # This prospective custody does not qualify held helper changes.
        scripts = Path(__file__).resolve().parent
        imported = {}
        for module in list(sys.modules.values()):
            source = getattr(module, "__file__", None)
            if source:
                path = Path(source).resolve()
                if path.suffix == ".py" and path.is_relative_to(scripts):
                    self.freeze_source(path)
                    imported[str(path)] = self.controller_sources[str(path)]
        self.record("controller", "imported_local_controllers_frozen_before_effects", imported)

    def freeze_source(self, path):
        path = Path(path).resolve()
        root = Path(__file__).resolve().parents[1]
        expected = self.source_reuse_manifest.get(str(path))
        if expected and path.is_relative_to(root) and hashlib.sha256(path.read_bytes()).hexdigest() == expected:
            archived = self.source_reuse / "controller-sources" / path.relative_to(root)
            if archived.is_file() and hashlib.sha256(archived.read_bytes()).hexdigest() == expected:
                Runner.freeze_source(self, path)
                self.reused_sources[str(path)] = {"path": str(archived), "sha256": expected}
                (self.output / "controller-source-reuse.json").write_text(json.dumps(self.reused_sources, indent=2) + "\n")
                return
        super().freeze_source(path)

    def browser(self):
        target = HttpTarget(self)
        gateway = ContentGateway(target.url, lambda _: None, self.private, self.nodes, browser_origins=True)
        gateway.record = lambda operation, result: self.record("controller", operation, result)
        browser = WebBrowser(self, self.args, gateway)
        assert not browser.call("open")["browserSandboxDisabledByHarness"]
        return browser

    def guests(self):
        """Ordinary small-command flows on immutable SDK3/WASM1 versions."""
        configure_local_resources(self, self.args, owner_factory=register_process)
        provider = OwnedNode(self, "provider")
        directory = self.private / "browser-terminal-guests"
        provider.call("terminal_enable", directory=str(directory))
        browser = self.browser()
        invitation = self.private / "browser-member-invitation"
        provider.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        status = browser.call("start", "join", self.configuration, invitation.read_text())
        assert status["id"] != provider.identity
        assert browser.call("request_join", False) == "Complete"
        browser.call("sync_services", provider.identity)
        operation = provider.call("genesis_propose", action="appoint-administrator", target=status["id"])["operation"]["id"]
        provider.call("genesis_confirm", operation=operation)
        browser.call("sync_services", provider.identity)
        administrator = BrowserTerminalNode(browser, status["id"])
        observer = WorkspaceObserver(self, directory)
        workspace = administrator.call("terminal_create", address=provider.address)["workspace"]
        observer.observe_server()

        def denied(node):
            before = sorted(path.name for path in directory.glob("mio-*"))
            node.call("terminal_create", address=provider.address, ok=False)
            node.call("terminal_attach", address=provider.address, workspace=workspace, ok=False)
            assert sorted(path.name for path in directory.glob("mio-*")) == before
            time.sleep(2.05)  # Preserve the existing refused-contact backoff.

        def attach_effect(node, filename, value):
            identifier = node.call("terminal_attach", address=provider.address, workspace=workspace)["id"]
            self.send(node, identifier, "stty -echo; printf 'GUEST_'; printf 'READY\\n'\n")
            self.read_until(node, identifier, b"GUEST_READY")
            self.send(node, identifier, "printf '%s' " + value + " > " + filename + "; printf 'SMALL_'; printf 'COMPLETE\\n'\n")
            self.read_until(node, identifier, b"SMALL_COMPLETE")
            assert (directory/workspace/filename).read_bytes() == value.encode()
            return identifier

        def provider_settled():
            self.until(lambda: provider.call("terminal_status")["active_operations"] == 0
                       and provider.call("terminal_status")["attachment_process_owners"] == 0,
                       "normal guest access end retained provider attachment owners", seconds=10)

        invitation = self.private / "browser-issued-terminal-invitation"
        invitation.write_text(browser.call("issue_service_guest", json.dumps(provider.address), "terminal", int(time.time())+180))
        invitation.chmod(0o600)
        native = OwnedNode(self, "native-guest", "join-guest", invitation)
        denied(native)
        scope = native.call("guest_redeem")
        assert scope["stage"] == "GuestActive" and scope["scope"]["service"] == "terminal"
        assert scope["scope"]["provider"] == provider.identity and scope["scope"]["guest"] == native.identity
        browser.call("sync_services", provider.identity)
        identifier = attach_effect(native, "browser-issued", "owned-browser-issued")
        browser.call("revoke_guest", scope["scope"]["invitation"])
        browser.call("sync_services", provider.identity)
        provider_settled()
        outcome = native.call("terminal_read", attachment=identifier, timeout_ms=100)
        assert not outcome["attached"] and outcome.get("failed"), outcome
        native.call("terminal_send", attachment=identifier,
                    message={"type":"bytes","data":list(b"printf forbidden > forbidden\n")}, ok=False)
        assert not (directory/workspace/"forbidden").exists()
        native.call("terminal_cancel", attachment=identifier)
        denied(native)
        self.record("controller", "ordinary_browser_issuer_native_terminal_guest", {
            "small_command_independent_file":True,"pending_no_effect":True,"issuer_revoke_closes_live_attachment":True,
            "new_operations_and_input_denied_after":True,"version":"NativeSDK3+WASM1"})

        invitation = self.private / "native-issued-terminal-invitation"
        provider.call("guest_issue", provider=provider.address, service="terminal", seconds=180, path=str(invitation))
        assert not browser.call("open", actor="guest")["browserSandboxDisabledByHarness"]
        status = browser.call("start", "join_guest", self.configuration, invitation.read_text(), actor="guest")
        guest = BrowserTerminalNode(browser, status["id"], actor="guest")
        denied(guest)
        scope = browser.call("redeem_guest", actor="guest")
        assert scope["stage"] == "GuestActive" and scope["scope"]["service"] == "terminal"
        assert scope["scope"]["guest"] == status["id"] and scope["scope"]["provider"] == provider.identity
        original_deadline = scope["scope"]["expires"]
        identifier = attach_effect(guest, "browser-consumed", "owned-browser-consumed")
        assert not guest.call("terminal_detach", attachment=identifier)["failed"]
        browser.call("reload", actor="guest")
        reopened = browser.call("start", "start", self.configuration, None, actor="guest")
        retained = browser.call("redeem_guest", actor="guest")
        assert reopened["id"] == status["id"] and retained["scope"]["expires"] == original_deadline
        assert time.time() < original_deadline
        identifier = attach_effect(guest, "browser-reopened", "owned-browser-reopened")
        ended = browser.call("end_guest", actor="guest")
        assert ended["ended_locally"] and all(row["saved"] for row in ended["delivery"])
        provider_settled()
        guest.call("terminal_read", attachment=identifier, timeout_ms=100, ok=False)
        guest.call("terminal_send", attachment=identifier,
                   message={"type":"bytes","data":list(b"printf forbidden > forbidden\n")}, ok=False)
        assert not (directory/workspace/"forbidden").exists()
        denied(guest)
        assert workspace in administrator.call("terminal_list", address=provider.address)["workspaces"]
        assert administrator.call("terminal_end", address=provider.address, workspace=workspace)["ended"]
        provider_settled()
        self.record("controller", "ordinary_native_issuer_browser_terminal_guest", {
            "small_commands_independent_files":True,"pending_no_effect":True,"guest_end_closes_live_attachment":True,
            "same_saved_identity_original_deadline_after_reopen":True,"new_operations_and_input_denied_after":True,
            "retained_workspace_publicly_ended":True,"version":"NativeSDK3+WASM1"})

    def bounds(self, provider, consumer, directory, browser):
        self.input_boundary(provider, consumer, directory)
        observer = WorkspaceObserver(self, directory)
        workspace = consumer.call("terminal_create", address=provider.address)["workspace"]
        observer.observe_server()
        identifiers = [consumer.call("terminal_attach", address=provider.address, workspace=workspace)["id"]
                       for _ in range(8)]
        assert consumer.call("terminal_status")["attachment_handles"] == 8
        assert provider.call("terminal_status")["attachment_process_owners"] == 8
        consumer.call("terminal_attach", address=provider.address, workspace=workspace, ok=False)
        assert provider.call("terminal_status")["attachment_process_owners"] == 8
        for identifier in identifiers:
            consumer.call("terminal_cancel", attachment=identifier)
        self.until(lambda: provider.call("terminal_status")["attachment_process_owners"] == 0,
                   "cancelled browser attachments retained processes", seconds=7)
        consumer.call("terminal_end", address=provider.address, workspace=workspace)
        self.pressure(provider, consumer, directory, browser, mode="burst")
        self.pressure(provider, consumer, directory, browser, mode="queued")
        self.record("controller", "actual_browser_terminal_bounds", {"eight_attachments_ninth_no_effect": True,
                    "max_input_16384_exact": True, "burst_and_queued_cancellation_bounded": True})

    def pressure(self, provider, consumer, directory, browser, mode="burst"):
        observer = WorkspaceObserver(self, directory)
        workspace = consumer.call("terminal_create", address=provider.address)["workspace"]
        observer.observe_server()
        identifier = consumer.call("terminal_attach", address=provider.address, workspace=workspace)["id"]
        self.send(consumer, identifier, "stty -echo; printf 'BOUND_'; printf 'READY\\n'\n")
        self.read_until(consumer, identifier, b"BOUND_READY")
        consumer.call("terminal_send", attachment=identifier, message={"type":"bytes","data":[81]*16385}, ok=False)
        self.send(consumer, identifier, "stty -icanon; printf 'started' > pressure-start; sleep 20\n")
        self.until((directory/workspace/"pressure-start").exists, "real finite input stall did not start")
        result = browser.call("terminal_input_pressure", identifier, mode)
        before = result["before"]
        assert result["seconds"] < 6.25, result
        assert before["pending"] > 0 and before["rejected"] == 0, result
        if mode == "queued":
            assert before["accepted"] >= 16, result
        assert before["terminal"]["active_operations"] == 1, result
        assert result["after"]["accepted"] + result["after"]["rejected"] == 1024, result
        assert result["terminal"]["active_operations"] == result["terminal"]["attachment_handles"] == 0
        self.until(lambda: provider.call("terminal_status")["attachment_process_owners"] == 0,
                   "input pressure retained browser provider attachment", seconds=7)
        assert workspace in consumer.call("terminal_list", address=provider.address)["workspaces"]
        consumer.call("terminal_end", address=provider.address, workspace=workspace)
        self.record("controller", "actual_browser_terminal_input_pressure", {"mode": mode,
            "oversized_input_rejected": True, "actual_pending_callers": before["pending"],
            "accepted_before_close": before["accepted"], "all_callers_settled": True,
            "detach_seconds": result["seconds"], "detach_result": result["result"],
            "workspace_retained_then_publicly_ended": True})

    def output_pressure(self, provider, consumer, directory):
        observer = WorkspaceObserver(self, directory)
        workspace = consumer.call("terminal_create", address=provider.address)["workspace"]
        observer.observe_server()
        identifier = consumer.call("terminal_attach", address=provider.address, workspace=workspace)["id"]
        self.send(consumer, identifier, "stty -echo; printf 'OUTPUT_'; printf 'READY\\n'\n")
        self.read_until(consumer, identifier, b"OUTPUT_READY")
        failures = provider.call("terminal_status")["failed_operations"]
        self.send(consumer, identifier, 'python3 -c \'import os; open("output-start","w").write("started"); [os.write(1,b"P"*16384) for _ in range(1024)]; open("output-done","w").write("completed")\'\n')
        self.until((directory/workspace/"output-start").exists, "real output producer did not start")
        self.until(lambda: consumer.call("terminal_status")["active_operations"] == 0,
                   "unread browser output retained an active owner", seconds=10)
        status = provider.call("terminal_status")
        assert status["enabled"] and status["failed_operations"] > failures
        outcome = consumer.call("terminal_read", attachment=identifier, timeout_ms=100)
        assert not outcome["attached"] and outcome.get("failed"), outcome
        started = time.monotonic()
        detached = consumer.call("terminal_detach", attachment=identifier)
        elapsed = time.monotonic() - started
        assert elapsed < 6.25 and detached["failed"] and not detached["workspace_ended"]
        self.until(lambda: provider.call("terminal_status")["attachment_process_owners"] == 0,
                   "unread browser output retained a provider process", seconds=7)
        self.until((directory/workspace/"output-done").exists, "finite output job did not finish after detach")
        assert (directory/workspace/"output-done").read_bytes() == b"completed"
        assert workspace in consumer.call("terminal_list", address=provider.address)["workspaces"]
        consumer.call("terminal_end", address=provider.address, workspace=workspace)
        self.record("controller", "actual_browser_terminal_output_pressure", {"produced_bytes": 16777216,
                    "reader_consumed": False, "failure_truthful": True, "detach_seconds": elapsed,
                    "retained_job_completed": True, "workspace_publicly_ended": True})

    def scenario(self):
        if self.case == "browser-open":
            self.browser()
            self.record("controller", "actual_browser_launch_entry", {"owned_chromium_opened": True, "sdk_started": False})
            return
        if self.case in {"access-policy", "access-scope", "access-expiry"}:
            BrowserAccess(self, BrowserTerminalNode).run()
            return
        if self.case == "guests":
            self.guests()
            return
        configure_local_resources(self, self.args, owner_factory=register_process)
        provider = OwnedNode(self, "provider")
        directory = self.private / "browser-terminal"
        provider.call("terminal_enable", directory=str(directory))
        browser = self.browser()
        invitation = self.private / "browser-invitation"
        provider.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        status = browser.call("start", "join", self.configuration, invitation.read_text())
        assert status["id"] != provider.identity
        assert browser.call("request_join", False) == "Complete"
        browser.call("sync_services", provider.identity)
        consumer = BrowserTerminalNode(browser, status["id"])
        self.record("controller", "independent_terminal_wasm_admission", {"distinct_identity":True,"real_admission":True,"service_sync":True})
        if self.case == "output-pressure":
            self.output_pressure(provider, consumer, directory)
            return
        if self.case in {"pressure", "pressure-burst", "pressure-queued"}:
            self.pressure(provider, consumer, directory, browser, mode="queued" if self.case == "pressure-queued" else "burst")
            return
        if self.case == "bounds":
            self.bounds(provider, consumer, directory, browser)
            return
        self.workspaces(provider, consumer, directory)
        if self.case == "entry":
            return
        observer = WorkspaceObserver(self, directory)
        workspace = consumer.call("terminal_create", address=provider.address)["workspace"]
        observer.observe_server()
        attachment = consumer.call("terminal_attach", address=provider.address, workspace=workspace)["id"]
        self.send(consumer, attachment, "stty -echo; printf 'TAB_'; printf 'READY\\n'\n")
        self.read_until(consumer, attachment, b"TAB_READY")
        self.send(consumer, attachment, "printf 'started' > tab-start; (sleep 1; printf 'retained-tab-work' > tab-done) &\n")
        self.until((directory/workspace/"tab-start").exists, "actual tab-owned job did not start")
        assert browser.call("close_tab")["closedWithoutSdkClose"]
        self.until((directory/workspace/"tab-done").exists, "actual tab close ended retained work")
        assert (directory/workspace/"tab-done").read_bytes() == b"retained-tab-work"
        reopened = browser.call("start", "start", self.configuration, None)
        assert reopened["id"] == status["id"]
        consumer = BrowserTerminalNode(browser, reopened["id"])
        assert workspace in consumer.call("terminal_list", address=provider.address)["workspaces"]
        attachment = consumer.call("terminal_attach", address=provider.address, workspace=workspace)["id"]
        self.send(consumer, attachment, "printf 'REATTACHED_'; printf 'TAB\\n'; cat tab-done\n")
        self.read_until(consumer, attachment, b"retained-tab-work")
        detached = consumer.call("terminal_detach", attachment=attachment)
        assert detached == {"attached":False,"workspace_ended":False,"failed":False}
        assert consumer.call("terminal_end", address=provider.address, workspace=workspace)["ended"]
        self.until(lambda: provider.call("terminal_status")["active_operations"] == 0
                   and provider.call("terminal_status")["attachment_process_owners"] == 0,
                   "old and new tab terminal owners did not settle", seconds=6.25)
        self.record("controller", "actual_closed_tab_retains_workspace", {"no_awaited_sdk_close":True,"same_saved_identity":True,
                    "same_workspace_reattached":True,"background_job_retained":True,"all_provider_owners_settled":True})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--browser-receipt", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    parser.add_argument("--case", default="tab")
    parser.add_argument("--reuse-controller-sources", type=Path, help="Reuse verified snapshots from a settled prior run")
    parser.add_argument("--current-builds", action="store_true", help="Verify selected sources of supplied current build receipts")
    BrowserRunner(parser.parse_args()).run()
