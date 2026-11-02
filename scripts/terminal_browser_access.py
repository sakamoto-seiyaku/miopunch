"""Ordinary Browser terminal access checks against verified Native/WASM builds.

This controller does not build artifacts or exercise input boundaries/pressure.
Every command and observed file belongs to its fresh scenario.
"""
import base64
import hashlib
import json
from pathlib import Path
import time

from check_terminal_service import WorkspaceObserver
from check_web_service import HttpTarget, OwnedNode
from forwarding_local_resources import configure_local_resources
from web_process_owner import register_process


ROOT = Path(__file__).resolve().parents[1]
ORIGIN_INVENTORY = ROOT / ".artifacts/runs/m120-high-20261025/renewed-prep-1/fresh-inventory.json"
ORIGIN_INVENTORY_SHA256 = "598236e1ddd52b3b2e931f27a723bcf241e31873489ae9e65d96d125bd29e003"
NATIVE_SHA256 = "cd94fdc5c263262380864154a38bee96b2a2369d89ca98045b51c2a7f5ad54a7"
NATIVE_LIBRARY_SHA256 = "550e2fd69ece02275429889c84e6cb2c5123846d2d0a43f9fffc771e1280972b"
RAW_WASM_SHA256 = "b2fcc16bc897584bfe3c5f0b962a429104b431a977e7f40953781f97fcc59e9b"
ROOT_LOCK_SHA256 = "7f77e9317f1c17dd78b45ab54ce980b783bd191735fe2652f6cfb7aaf40c44f7"


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class BrowserAccess:
    def __init__(self, runner, adapter):
        self.runner, self.adapter = runner, adapter

    def verify_versions(self):
        runner = self.runner
        if getattr(runner.args, "current_builds", False):
            return self.verify_current_versions()
        self.version = "archived SDK3+WASM1"
        assert digest(ORIGIN_INVENTORY) == ORIGIN_INVENTORY_SHA256
        inventory = json.loads(ORIGIN_INVENTORY.read_text())
        native = ROOT / ".artifacts/runs/m120-linux-terminal-3"
        wasm = ROOT / ".artifacts/runs/m120-wasm-terminal-1"
        assert runner.args.binary.resolve() == native / "miopunch-client"
        assert runner.args.source_receipt.resolve() == native / "build.json"
        assert runner.args.sha256 == NATIVE_SHA256
        assert runner.args.package.resolve() == wasm / "pkg"
        assert runner.args.browser_receipt.resolve() == wasm / "build.json"
        for directory in [native, wasm]:
            for name, expected in inventory["frozenProvenance"][directory.name].items():
                assert digest(directory / name) == expected, "archived SDK provenance changed"
        assert digest(runner.args.binary) == NATIVE_SHA256
        assert digest(native / "sdk/libmiopunch_native_probe.rlib") == NATIVE_LIBRARY_SHA256
        assert digest(wasm / "compiled.wasm") == RAW_WASM_SHA256
        assert digest(ROOT / "Cargo.lock") == ROOT_LOCK_SHA256
        selected = json.loads((wasm / "selected-dependency-inputs.json").read_text())["selectedRepositoryInputs"]
        assert len(selected) == 229
        for name, expected in selected.items():
            assert digest(ROOT / name) == expected, "selected WASM input changed"
        build = json.loads((wasm / "build.json").read_text())
        actual = {path.relative_to(wasm / "pkg").as_posix(): digest(path)
                  for path in (wasm / "pkg").rglob("*") if path.is_file()}
        assert actual == build["artifacts"]
        versions = {
            "native_build_receipt": str(native / "build.json"),
            "wasm_build_receipt": str(wasm / "build.json"),
            "archived_provenance": {directory.name: inventory["frozenProvenance"][directory.name]
                                    for directory in [native, wasm]},
            "native_library_sha256": NATIVE_LIBRARY_SHA256,
            "raw_wasm_sha256": RAW_WASM_SHA256, "root_lock_sha256": ROOT_LOCK_SHA256,
            "wasm_package_files": actual,
            "native_version": "archived SDK3", "native_sha256": NATIVE_SHA256,
            "wasm_version": "WASM1", "package_sha256": build["packageSHA256"],
            "selected_wasm_inputs": len(selected), "all_selected_match": True,
            "origin_inventory_sha256": ORIGIN_INVENTORY_SHA256,
            "current_held_native_source_qualified": False, "build_invoked": False}
        (runner.output / "ordinary-version-reuse.json").write_text(json.dumps(versions, indent=2) + "\n")
        runner.record("controller", "ordinary_browser_access_versions", versions)

    def verify_current_versions(self):
        """Require current selected sources, successful receipts and immutable bytes."""
        runner = self.runner
        native_receipt = runner.args.source_receipt.resolve()
        wasm_receipt = runner.args.browser_receipt.resolve()
        native = json.loads(native_receipt.read_text())
        wasm = json.loads(wasm_receipt.read_text())
        assert native["exit"] == wasm["exit"] == wasm["bindgenExit"] == 0
        assert wasm["sourceFrozenDuringBuild"]
        assert Path(native["pin"]).resolve() == runner.args.binary.resolve()
        assert native["sha256"] == runner.args.sha256 == digest(runner.args.binary)
        library = native["sdkLibrary"]
        assert digest(library["pin"]) == library["sha256"]
        selected_counts = {}
        for receipt in [native_receipt, wasm_receipt]:
            selected = json.loads(receipt.with_name("selected-dependency-inputs.json").read_text())["selectedRepositoryInputs"]
            assert selected
            frozen = json.loads(receipt.with_name("inputs.json").read_text())
            assert "Cargo.lock" in frozen
            assert digest(ROOT / "Cargo.lock") == frozen["Cargo.lock"]
            for name, expected in selected.items():
                path = (ROOT / name).resolve()
                assert path.is_relative_to(ROOT)
                assert digest(path) == expected, "selected current source changed: " + name
            selected_counts[str(receipt)] = len(selected)
        actual = {path.relative_to(runner.args.package).as_posix(): digest(path)
                  for path in runner.args.package.rglob("*") if path.is_file()}
        assert actual == wasm["artifacts"]
        raw_wasm = Path(wasm["compiledPin"])
        raw_hash = digest(raw_wasm)
        assert any(raw_wasm.samefile(path) for path in (ROOT / ".artifacts/sha256" / raw_hash[:2]).glob(raw_hash + "-*")
                   if path.is_file()), "compiled WASM is not linked to its immutable store object"
        self.version = "current verified Native/WASM builds"
        versions = {"native_build_receipt": str(native_receipt), "native_sha256": native["sha256"],
                    "native_library_sha256": library["sha256"], "wasm_build_receipt": str(wasm_receipt),
                    "raw_wasm_sha256": raw_hash, "package_sha256": wasm["packageSHA256"],
                    "wasm_package_files": actual, "selected_current_inputs": selected_counts,
                    "all_selected_match": True, "build_invoked": False}
        (runner.output / "ordinary-version-reuse.json").write_text(json.dumps(versions, indent=2) + "\n")
        runner.record("controller", "ordinary_browser_access_versions", versions)

    def setup(self):
        runner = self.runner
        self.verify_versions()
        configure_local_resources(runner, runner.args, owner_factory=register_process)
        self.provider = OwnedNode(runner, "provider")
        self.control = runner.member(self.provider, "control")
        self.genesis("appoint-administrator", self.control.identity)
        self.provider.call("service_sync", address=self.control.address)
        runner.record("controller", "ordinary_current_admin_control", {
            "native_identity": self.control.identity, "appointed_and_received": True})
        self.browser = runner.browser()
        invitation = runner.private / "browser-member-invitation"
        self.provider.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        status = self.browser.call("start", "join", runner.configuration, invitation.read_text())
        assert status["id"] not in {self.provider.identity, self.control.identity}
        assert self.browser.call("request_join", False) == "Complete"
        self.browser.call("sync_services", self.provider.identity)
        self.member = self.adapter(self.browser, status["id"])
        self.directory = runner.private / "ordinary-browser-access"
        self.observer = WorkspaceObserver(runner, self.directory)

    def enable(self):
        status = self.provider.call("terminal_enable", directory=str(self.directory))
        assert status["enabled"] and status["capable"]
        self.browser.call("sync_services", self.provider.identity)
        self.workspace = self.member.call("terminal_create", address=self.provider.address)["workspace"]
        self.observer.observe_server()

    def effect(self, node, filename, value, *, provider=None, workspace=None, directory=None):
        provider = provider or self.provider
        workspace = workspace or self.workspace
        directory = directory or self.directory
        identifier = node.call("terminal_attach", address=provider.address, workspace=workspace)["id"]
        self.runner.send(node, identifier, "stty -echo; printf 'ACCESS_'; printf 'READY\\n'\n")
        self.runner.read_until(node, identifier, b"ACCESS_READY")
        self.runner.send(node, identifier, "printf '%s' " + value + " > " + filename + "; printf 'SMALL_'; printf 'COMPLETE\\n'\n")
        self.runner.read_until(node, identifier, b"SMALL_COMPLETE")
        assert (directory / workspace / filename).read_bytes() == value.encode()
        return identifier

    def denied(self, node, *, provider=None, workspace=None, directory=None):
        provider = provider or self.provider
        workspace = workspace or self.workspace
        directory = directory or self.directory
        before = sorted(path.name for path in directory.glob("mio-*"))
        node.call("terminal_create", address=provider.address, ok=False)
        node.call("terminal_attach", address=provider.address, workspace=workspace, ok=False)
        assert sorted(path.name for path in directory.glob("mio-*")) == before
        time.sleep(2.05)  # Preserve the existing refused-contact backoff.
        assert sorted(path.name for path in directory.glob("mio-*")) == before
        assert not (directory / workspace / "forbidden").exists()
        self.runner.record("controller", "ordinary_terminal_denied_no_later_effect", {
            "provider": provider.identity, "node": node.identity,
            "workspace_names_unchanged": True, "forbidden_file_absent": True,
            "original_refused_contact_backoff_seconds": 2.05})

    def settled(self):
        self.runner.until(lambda: self.provider.call("terminal_status")["active_operations"] == 0
                          and self.provider.call("terminal_status")["attachment_process_owners"] == 0,
                          "current Browser access change retained provider owners", seconds=10)

    def closed(self, node, identifier):
        self.settled()
        outcome = node.call("terminal_read", attachment=identifier, timeout_ms=100)
        assert not outcome["attached"] and outcome.get("failed"), outcome
        node.call("terminal_send", attachment=identifier,
                  message={"type": "bytes", "data": list(b"printf forbidden > forbidden\n")}, ok=False)
        assert not (self.directory / self.workspace / "forbidden").exists()
        node.call("terminal_cancel", attachment=identifier)

    def genesis(self, action, target, *, execute=False):
        operation = self.provider.call("genesis_propose", action=action, target=target)["operation"]["id"]
        if execute:
            self.provider.call("device_execute", operation=operation)
        else:
            self.provider.call("genesis_confirm", operation=operation)

    def policy(self):
        assert not self.provider.call("terminal_status")["enabled"]
        self.workspace = "mio-" + "0" * 32
        self.denied(self.member)
        assert not self.directory.exists()
        self.enable()
        identifier = self.effect(self.member, "default-member", "owned-default-member")
        self.provider.call("terminal_disable")
        self.browser.call("sync_services", self.provider.identity)
        self.closed(self.member, identifier)
        self.denied(self.member)
        self.provider.call("terminal_enable", directory=str(self.directory))
        self.browser.call("sync_services", self.provider.identity)
        rule = {"members": False, "guests": True, "allowed": [], "denied": []}
        revision = self.provider.call("service_rule", provider=self.provider.identity, service="terminal",
                                      previous=None, rule=rule)["revision"]
        self.browser.call("sync_services", self.provider.identity)
        self.denied(self.member)
        rule["allowed"] = [self.member.identity]
        revision = self.provider.call("service_rule", provider=self.provider.identity, service="terminal",
                                      previous=revision, rule=rule)["revision"]
        self.browser.call("sync_services", self.provider.identity)
        identifier = self.effect(self.member, "explicit-allow", "owned-explicit-allow")
        rule["denied"] = [self.member.identity]
        revision = self.provider.call("service_rule", provider=self.provider.identity, service="terminal",
                                      previous=revision, rule=rule)["revision"]
        self.browser.call("sync_services", self.provider.identity)
        self.closed(self.member, identifier)
        self.denied(self.member)
        rule = {"members": True, "guests": True, "allowed": [], "denied": []}
        revision = self.provider.call("service_rule", provider=self.provider.identity, service="terminal",
                                      previous=revision, rule=rule)["revision"]
        self.genesis("appoint-administrator", self.member.identity)
        self.browser.call("sync_services", self.provider.identity)
        identifier = self.effect(self.control, "before-browser-admin", "owned-before-browser-admin")
        denied_rule = dict(rule, denied=[self.control.identity])
        revision = self.browser.call("set_service_rule", self.provider.identity, "terminal", revision,
                                     json.dumps(denied_rule))
        self.browser.call("sync_services", self.provider.identity)
        self.closed(self.control, identifier)
        self.denied(self.control)
        revision = self.browser.call("set_service_rule", self.provider.identity, "terminal", revision,
                                     json.dumps(rule))
        self.browser.call("sync_services", self.provider.identity)
        identifier = self.effect(self.control, "restored-browser-admin", "owned-restored-browser-admin")
        assert not self.control.call("terminal_detach", attachment=identifier)["failed"]
        self.genesis("demote-administrator", self.member.identity)
        self.browser.call("sync_services", self.provider.identity)
        self.browser.call("set_service_rule", self.provider.identity, "terminal", revision,
                          json.dumps(denied_rule), ok=False)
        identifier = self.effect(self.member, "demoted-member", "owned-demoted-member")
        self.genesis("remove-member", self.member.identity, execute=True)
        self.closed(self.member, identifier)
        self.denied(self.member)
        assert self.workspace in self.control.call("terminal_list", address=self.provider.address)["workspaces"]
        assert self.control.call("terminal_end", address=self.provider.address, workspace=self.workspace)["ended"]
        self.runner.record("controller", "ordinary_browser_terminal_current_policy", {
            "service_off_live_closure_and_no_effect": True, "default_member_real_command": True,
            "explicit_allow_deny_live_closure": True, "browser_admin_received_rule_deny_restore": True,
            "received_demotion_denies_management_retains_membership": True,
            "received_member_removal_closes_browser_attachment": True,
            "retained_workspace_publicly_ended": True, "version": self.version})

    def new_guest(self, actor, service, seconds=180):
        invitation = self.runner.private / (actor + "-invitation")
        self.provider.call("guest_issue", provider=self.provider.address, service=service,
                           seconds=seconds, path=str(invitation))
        assert not self.browser.call("open", actor=actor)["browserSandboxDisabledByHarness"]
        status = self.browser.call("start", "join_guest", self.runner.configuration, invitation.read_text(), actor=actor)
        scope = self.browser.call("redeem_guest", actor=actor)
        assert scope["stage"] == "GuestActive" and scope["scope"]["service"] == service
        assert scope["scope"]["provider"] == self.provider.identity
        assert scope["scope"]["guest"] == status["id"]
        assert time.time() < scope["scope"]["expires"]
        return self.adapter(self.browser, status["id"], actor=actor), scope

    def scope(self):
        self.enable()
        second = self.runner.member(self.provider, "second-provider")
        second_directory = self.runner.private / "second-browser-provider"
        second.call("terminal_enable", directory=str(second_directory))
        second_observer = WorkspaceObserver(self.runner, second_directory)
        self.provider.call("service_sync", address=second.address)
        self.control.call("service_sync", address=self.provider.address)
        self.control.call("service_sync", address=second.address)
        self.browser.call("sync_services", self.provider.identity)
        self.browser.call("sync_services", second.identity)
        second_workspace = self.member.call("terminal_create", address=second.address)["workspace"]
        second_observer.observe_server()
        identifier = self.effect(self.member, "second-positive", "owned-second-provider",
                                 provider=second, workspace=second_workspace, directory=second_directory)
        assert not self.member.call("terminal_detach", attachment=identifier)["failed"]
        assert second_workspace in self.control.call("terminal_list", address=second.address)["workspaces"]
        self.runner.record("controller", "ordinary_second_provider_current_admin_premise", {
            "records_received_from_creator_and_second": True,
            "native_current_admin_listed_exact_retained_workspace": True})
        guest, scope = self.new_guest("terminal-guest", "terminal")
        identifier = self.effect(guest, "scoped-positive", "owned-scoped-positive")
        self.denied(guest, provider=second, workspace=second_workspace, directory=second_directory)
        assert second.call("terminal_status")["attachment_process_owners"] == 0
        assert time.time() < scope["scope"]["expires"]
        target = HttpTarget(self.runner)
        self.provider.call("web_enable", targets=[target.url])
        positive = self.control.call("web_open", address=self.provider.address, url=target.url)
        assert positive["status"] == 200 and base64.b64decode(positive["body"]) == b"owned M119 web target"
        request = json.dumps({"url": target.url, "method": "GET", "headers": [], "body": ""})
        before = len(target.requests)
        self.browser.call("request_web", self.provider.identity, request, actor="terminal-guest", ok=False)
        assert len(target.requests) == before
        time.sleep(2.05)  # Preserve the existing refused-contact backoff.
        assert len(target.requests) == before
        web_guest, web_scope = self.new_guest("web-guest", "web")
        positive = self.browser.call("request_web", self.provider.identity, request, actor="web-guest")
        assert positive["status"] == 200 and base64.b64decode(positive["body"]) == b"owned M119 web target"
        self.denied(web_guest)
        assert time.time() < web_scope["scope"]["expires"]
        assert not guest.call("terminal_detach", attachment=identifier)["failed"]
        assert self.control.call("terminal_end", address=self.provider.address, workspace=self.workspace)["ended"]
        assert self.control.call("terminal_end", address=second.address, workspace=second_workspace)["ended"]
        self.settled()
        self.runner.record("controller", "ordinary_browser_terminal_scoped_denial", {
            "reachable_second_provider_real_browser_pty_command": True,
            "terminal_guest_real_first_provider_command": True,
            "wrong_provider_create_attach_denied_no_workspace_effect": True,
            "terminal_guest_web_denied_no_target_effect": True,
            "web_only_browser_guest_actual_http_200_before_terminal_denial": True,
            "web_only_guest_terminal_denied_no_workspace_effect": True,
            "original_deadlines_unchanged_and_current": True,
            "version": self.version})

    def expiry(self):
        self.enable()
        guest, scope = self.new_guest("expiry-guest", "terminal", seconds=25)
        expires = scope["scope"]["expires"]
        identifier = self.effect(guest, "before-expiry", "owned-before-expiry")
        assert time.time() < expires
        assert self.provider.call("terminal_status")["attachment_process_owners"] == 1
        window_started, invalid_windows = None, 0

        def ended_in_current_window():
            nonlocal window_started, invalid_windows
            status = self.provider.call("terminal_status")
            guest_status = self.browser.call("guest_status", actor="expiry-guest")
            wall, now = time.time(), time.monotonic()
            self.runner.record("controller", "original_browser_deadline_observation", {
                "original_deadline": expires, "wall": wall, "monotonic": now,
                "guest_stage": guest_status["stage"],
                "active_operations": status["active_operations"],
                "attachment_process_owners": status["attachment_process_owners"]})
            if wall <= expires:
                if window_started is not None:
                    invalid_windows += 1
                window_started = None
                return False
            if window_started is None:
                window_started = now
            assert now - window_started < 10, "ten continuously expired seconds retained Browser terminal owners"
            return (guest_status["stage"] == "GuestExpired"
                    and status["active_operations"] == 0
                    and status["attachment_process_owners"] == 0)

        self.runner.until(ended_in_current_window, "natural original Browser guest expiry did not stop access", seconds=100)
        elapsed_expired = time.monotonic() - window_started
        assert time.time() > expires
        self.browser.call("read_terminal", identifier, 100, actor="expiry-guest", ok=False)
        guest.call("terminal_send", attachment=identifier,
                   message={"type": "bytes", "data": list(b"printf forbidden > forbidden\n")}, ok=False)
        assert not (self.directory / self.workspace / "forbidden").exists()
        self.denied(guest)
        assert time.time() > expires
        assert self.browser.call("guest_status", actor="expiry-guest")["stage"] == "GuestExpired"
        assert not (self.directory / self.workspace / "forbidden").exists()
        assert self.control.call("terminal_end", address=self.provider.address, workspace=self.workspace)["ended"]
        self.runner.record("controller", "ordinary_browser_terminal_natural_expiry", {
            "original_deadline": expires, "continuous_expired_window_seconds": elapsed_expired,
            "invalid_windows": invalid_windows, "owner_close_bound_seconds": 10,
            "real_command_before_original_deadline": True,
            "provider_owners_settled_and_current_guest_expired": True,
            "old_input_and_new_operations_denied_no_file_or_workspace_effect": True,
            "no_clock_or_deadline_change": True, "version": self.version})

    def run(self):
        self.setup()
        {"access-policy": self.policy, "access-scope": self.scope,
         "access-expiry": self.expiry}[self.runner.case]()
