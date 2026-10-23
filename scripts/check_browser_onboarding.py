#!/usr/bin/env python3
"""Exercise public M113 operations on independent Chromium/WASM and native nodes.

The runtime consumes verified immutable artifacts and never invokes Cargo.
Private invitations/cards stay in the owned temporary directory or RPC pipe.
Only redacted outcomes, public identities and counters belong in receipts.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import threading
import time

from check_client_onboarding import OnboardingRunner, Host
from host_native_runtime import group_absent, group_snapshot

ROOT = Path(__file__).resolve().parents[1]
RELAY = "https://aps1-1.relay.n0.iroh.link./"
CONFIGURATION = {"rtc_enabled": False, "relays": [RELAY]}
PRIVATE_OPERATIONS = {"invitation", "precode_invitation", "pairing_information", "export_recovery_card", "member_call", "begin_management_session", "issue_guest"}


def material(invitation):
    if invitation.startswith("miopunch://invite/"):
        return json.loads(base64.urlsafe_b64decode(invitation.split("/invite/", 1)[1] + "=="))
    return json.loads(invitation)


class BrowserControl:
    def __init__(self, runner, args):
        self.runner, self.sequence, self.responses = runner, 0, {}
        self.condition = threading.Condition()
        self.stderr = (runner.output / "private-browser.stderr").open("w")
        command = ["rtk", "proxy", "node", str(getattr(args, "harness", ROOT / "browser-probe/harness/client-onboarding.mjs")),
                   str(args.package.resolve()), str(args.chromium.resolve()), str(getattr(runner, "browser_private", runner.output / "private-browser-profiles"))]
        self.process = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.stderr, text=True, start_new_session=True)
        assert json.loads(self.process.stdout.readline())["event"] == "ready"

        def read():
            for line in self.process.stdout:
                response = json.loads(line)
                with self.condition:
                    self.responses[response["id"]] = response
                    self.condition.notify_all()
        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()

    def send(self, actor, operation, *args):
        self.sequence += 1
        identifier = self.sequence
        self.process.stdin.write(json.dumps(dict(id=identifier, actor=actor, op=operation, args=args)) + "\n")
        self.process.stdin.flush()
        return identifier, actor, operation

    def wait(self, pending, ok=True):
        identifier, actor, operation = pending
        deadline = time.monotonic() + 75
        with self.condition:
            while identifier not in self.responses:
                if self.process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("owned browser operation ended without response: " + operation)
                self.condition.wait(timeout=0.1)
            response = self.responses.pop(identifier)
        public = {key: value for key, value in response.items() if key != "privateError"}
        if operation in PRIVATE_OPERATIONS:
            public.pop("result", None)
            public["privateReturn"] = response["ok"]
        self.runner.record(actor, operation, public)
        if not response["ok"]:
            # Preserve exact failures privately before any subsequent getter.
            with (self.runner.output / "private-browser-errors.jsonl").open("a") as out:
                out.write(json.dumps(dict(actor=actor, operation=operation, **response)) + "\n")
        assert response["ok"] == ok, dict(actor=actor, operation=operation, failureClass=response.get("failureClass"))
        return response.get("result")

    def call(self, actor, operation, *args, ok=True):
        return self.wait(self.send(actor, operation, *args), ok=ok)

    def start(self, actor, action, invitation=None, configuration=None):
        self.call(actor, "open")
        assert self.call(actor, "api") == dict(create=True, join=True, pair=True)
        return self.call(actor, "start", action, json.dumps(CONFIGURATION if configuration is None else configuration), invitation)

    def reopen(self, actor):
        before = self.call(actor, "status")
        self.call(actor, "reload")
        after = self.call(actor, "start", "start", "{}", None)
        assert after == before
        return after

    def close(self):
        before, _ = group_snapshot(self.process.pid)
        receipt = self.call("runner", "close")
        self.process.stdin.close()
        receipt["exit"] = self.process.wait(timeout=20)
        self.reader.join(timeout=2)
        self.stderr.close()
        receipt["readerEnded"] = not self.reader.is_alive()
        receipt["processGroupAbsent"] = group_absent(self.process.pid) is True
        receipt["ownedGroupObservedBefore"] = bool(before)
        assert receipt["exit"] == 0 and receipt["readerEnded"] and receipt["processGroupAbsent"]
        assert all(item["closed"] for item in receipt["cleanup"])
        return receipt


class BrowserOnboardingRunner(OnboardingRunner):
    def native_invitation(self, creator, name, precode=False):
        if precode:
            creator.call("issue_precode", path=self.private_path(name, False), seconds=3600, max_uses=1)
        else:
            creator.call("invite", path=self.private_path(name, False), text=True)
        return self.read_private(name, False).decode()

    def browser_creator(self, name):
        b = self.browser
        original = b.start(name, "create")
        assert original["stage"] == "AwaitingRecoverySave"
        assert b.call(name, "duplicate_start") == dict(rejected=True)
        b.call(name, "invitation", ok=False)
        b.call(name, "begin_management_session", ok=False)
        b.call(name, "acknowledge_recovery_saved", ok=False)
        b.call(name, "fail_next_save")
        b.call(name, "export_recovery_card", ok=False)
        b.call(name, "acknowledge_recovery_saved", ok=False)
        assert b.call(name, "status") == original
        plain = b.call(name, "export_recovery_card")
        encrypted = b.call(name, "export_recovery_card", "M113 owned recovery test")
        expected = {key: original[key] for key in ["network", "genesis"]}
        assert b.call(name, "inspect_recovery_card", plain) == expected
        assert b.call(name, "inspect_recovery_card", encrypted, "M113 owned recovery test") == expected
        b.call(name, "inspect_recovery_card", encrypted, "different password", ok=False)
        changed = encrypted[:70] + ("A" if encrypted[70] != "A" else "B") + encrypted[71:]
        b.call(name, "inspect_recovery_card", changed, "M113 owned recovery test", ok=False)
        credential = base64.urlsafe_b64decode(plain.split(":1:", 1)[1] + "==")
        assert len(credential) == 64 and credential[32:].hex() == original["genesis"]
        self.browser_credentials = getattr(self, "browser_credentials", {})
        self.browser_credentials[name] = (credential[:32].hex(), original["genesis"])
        assert b.reopen(name) == original
        b.call(name, "invitation", ok=False)
        b.call(name, "acknowledge_recovery_saved")
        b.call(name, "acknowledge_recovery_saved")
        saved = b.call(name, "status")
        assert saved["stage"] == "Complete" and saved["id"] == original["id"]
        assert saved["network"] == original["network"] and saved["genesis"] == original["genesis"]
        assert b.reopen(name) == saved
        self.record(name, "browser_recovery_qualified", dict(actualIndexedDB=True, independentCreator=True,
                    optionalEncryption=True, failedSaveNotAcknowledged=True, originalCredentialRoundtrip=True,
                    wrongPasswordAndTamperRejected=True, explicitSavedGateDurable=True))
        return saved

    def browser_applicant(self, creator, name, *, precode=False):
        b = self.browser
        invitation_name = name + "-invitation.json"
        invitation = self.native_invitation(creator, invitation_name, precode)
        original = b.start(name, "join", invitation)
        assert original["id"] != creator.node and original["stage"] == "AwaitingApproval"
        b.call(name, "exchange", "premature", ok=False)
        if not precode:
            assert b.call(name, "request_join", False) == "AwaitingApproval"
            first = creator.call("requests")["requests"]
            assert len(first) == 1 and first[0]["applicant"] == original["id"]
            assert b.call(name, "request_join", False) == "AwaitingApproval"
            assert creator.call("requests")["requests"] == first
            creator.call("reject", **first[0])
            creator.call("reject", **first[0])
            assert b.call(name, "request_join", False) == "Rejected"
            assert b.call(name, "request_join", False) == "Rejected"
            assert creator.call("requests")["requests"] == []
            assert b.reopen(name)["stage"] == "Rejected"
            assert b.call(name, "request_join", False) == "Rejected"
            b.call(name, "new_manual_attempt")
            assert b.call(name, "request_join", False) == "AwaitingApproval"
            second = creator.call("requests")["requests"]
            assert second[0]["request_id"] != first[0]["request_id"]
            creator.call("approve", **second[0])
            creator.call("approve", **second[0])
        assert b.call(name, "request_join", True) == "PendingMaterials"
        pending = b.call(name, "admission_material_receipt")
        assert pending["authorization_saved"] and not pending["secret_saved"]
        count = creator.call("status")["records"]
        assert b.reopen(name)["stage"] == "PendingMaterials"
        assert b.call(name, "request_join", False) == "Complete"
        assert b.call(name, "request_join", False) == "Complete"
        assert creator.call("status")["records"] == count
        complete = b.call(name, "admission_material_receipt")
        assert complete["authorization_saved"] and complete["secret_saved"]
        assert b.reopen(name)["stage"] == "Complete"
        if precode:
            assert creator.call("precode_status", path=self.path(invitation_name, False))["uses"] == 1
        self.record(name, "browser_join_qualified", dict(precode=precode, realNativeProvider=True,
                    rejectedAttemptStops=not precode, explicitFreshRequest=not precode,
                    authorizationBeforeSecret=True, progressRetained=True, offlineMembershipRetained=True,
                    idempotentGrant=True, precodeUsedOnce=precode))

    def native_applicant(self, creator, name, *, precode=False):
        b = self.browser
        invitation = b.call(creator, "precode_invitation", int(time.time()) + 3600, 1) if precode else b.call(creator, "invitation")
        name_file = name + "-invitation.json"
        path = self.write_private(name_file, invitation.encode(), False)
        applicant = self.start(name, False, "join", path)
        assert applicant.ready["status"]["stage"] == "AwaitingApproval"
        if not precode:
            serving = b.send(creator, "serve_admission", False)
            assert applicant.call("request", path=path)["stage"] == "AwaitingApproval"
            assert b.wait(serving) == "AwaitingApproval"
            first = [a for a in b.call(creator, "admission_requests") if not a["approved"] and not a["rejected"]]
            assert len(first) == 1 and first[0]["applicant"] == applicant.node
            for _ in range(2):
                b.call(creator, "reject_admission", applicant.node, first[0]["request"])
            b.call(creator, "approve_admission", applicant.node, first[0]["request"], ok=False)
            serving = b.send(creator, "serve_admission", False)
            assert applicant.call("request", path=path)["stage"] == "Rejected"
            assert b.wait(serving) == "Rejected"
            applicant = self.reopen(applicant)
            assert applicant.call("request", path=path)["stage"] == "Rejected"
            applicant.call("new_manual_attempt", path=path)
            serving = b.send(creator, "serve_admission", False)
            assert applicant.call("request", path=path)["stage"] == "AwaitingApproval"
            assert b.wait(serving) == "AwaitingApproval"
            second = [a for a in b.call(creator, "admission_requests") if not a["approved"] and not a["rejected"]]
            assert len(second) == 1 and second[0]["request"] != first[0]["request"]
            grant = b.call(creator, "approve_admission", applicant.node, second[0]["request"])
            assert b.call(creator, "approve_admission", applicant.node, second[0]["request"]) == grant
        serving = b.send(creator, "serve_admission", precode)
        assert applicant.call("request", path=path, authorization_only=True)["stage"] == "PendingMaterials"
        b.wait(serving, ok=False)  # Deliberate sender-close before AUTH/secret.
        applicant = self.reopen(applicant)
        serving = b.send(creator, "serve_admission", precode)
        assert applicant.call("request", path=path)["stage"] == "Complete"
        assert b.wait(serving) == "Complete"
        if precode:
            assert b.call(creator, "precode_usage", material(invitation)["precode"]) == 1
        self.reopen(applicant).stop()
        self.record(name, "browser_provider_qualified", dict(realNativeApplicant=True, precode=precode,
                    rejectionAndFreshAttempt=not precode, authorizationBeforeSecret=True, durableCompletion=True))

    def resources(self, creator):
        b = self.browser
        invitation = self.native_invitation(creator, "resources-invitation.json")
        b.call("resource-denial", "open")
        denied = {"rtc_enabled": False, "relays": [], "mqtt": [], "irc": []}
        b.call("resource-denial", "start", "join", json.dumps(denied), invitation, ok=False)
        b.call("resource-denial", "start", "start", "{}", None, ok=False)
        restored = b.call("resource-denial", "start", "start", json.dumps(CONFIGURATION), None)
        assert restored["stage"] == "AwaitingApproval"
        assert b.call("resource-denial", "resource_configuration")["relays"] == [RELAY]
        disabled = "https://disabled.example.invalid/"
        mqtt = dict(logical_id="m113-disabled-mqtt", native_url="mqtt://mqtt.example.invalid:1883/", wss_url="wss://mqtt.example.invalid/mqtt")
        irc = dict(logical_id="m113-disabled-irc", native_url="irc://irc.example.invalid:6667/", wss_url="wss://irc.example.invalid/", channel="#m113-disabled")
        b.start("resource-sender", "create", configuration={"rtc_enabled": False, "relays": [RELAY, disabled],
            "disabled": [disabled, mqtt["wss_url"], irc["wss_url"]], "mqtt": [mqtt],
            "irc": [dict(facility=irc, profile=dict(attempt_ms=12000, pacing_ms=10, assembly_ttl_ms=3000, queue_ttl_ms=500))]})
        b.call("resource-sender", "export_recovery_card")
        b.call("resource-sender", "acknowledge_recovery_saved")
        offered = material(b.call("resource-sender", "invitation"))
        assert disabled not in offered["bootstrap"]["relays"]
        assert offered["bootstrap"]["mqtt"] == [] and offered["bootstrap"]["irc"] == []
        assert all(a.get("Relay") != disabled for a in offered["address"]["addrs"])
        self.record("resources", "browser_contact_policy_qualified", dict(explicitEmptyReplacementRetained=True,
                    fallbackDidNotReenable=True, senderDisabledOriginNotOffered=True))

    def browser_exchange(self, creator):
        b = self.browser
        invitation = b.call(creator, "invitation")
        joined = b.start("browser-member", "join", invitation)
        server = b.send(creator, "serve_admission", False)
        assert b.call("browser-member", "request_join", False) == "AwaitingApproval"
        assert b.wait(server) == "AwaitingApproval"
        pending = [a for a in b.call(creator, "admission_requests") if a["applicant"] == joined["id"]]
        b.call(creator, "approve_admission", joined["id"], pending[0]["request"])
        server = b.send(creator, "serve_admission", False)
        assert b.call("browser-member", "request_join", False) == "Complete"
        assert b.wait(server) == "Complete"
        assert b.reopen("browser-member")["id"] == joined["id"]
        accepting = b.send(creator, "member_accept")
        b.call("browser-member", "member_connect", b.call(creator, "id"))
        b.wait(accepting)
        for sender, receiver in [("browser-member", creator), (creator, "browser-member")]:
            serve = b.send(receiver, "member_call", "serve_exchange", [])
            message = "M113 bounded member exchange"
            assert b.call(sender, "member_call", "exchange", [message]) == message
            assert b.wait(serve) == message
        for actor in [creator, "browser-member"]:
            b.call(actor, "member_call", "close", [])
        self.record("browser-member", "real_wasm_member_exchange_qualified", dict(independentIdentities=True,
                    realIndexedDBReopen=True, messages=2, sdkMemberProtocol=True))

    def pairing(self, native_creator, browser_creator):
        b = self.browser
        receiver = b.start("browser-pair", "pair")
        wrong = b.start("browser-pair-wrong", "pair")
        assert receiver["id"] != wrong["id"]
        invitation = self.native_invitation(native_creator, "browser-pair-invitation.json")
        invitation_path = self.path("browser-pair-invitation.json", False)
        pairing = b.call("browser-pair", "pairing_information")
        pairing_path = self.write_private("browser-pair-information.json", pairing.encode(), False)
        b.call("browser-pair", "finish_pairing", ok=False)
        impostor = self.creator(False, "pairing-impostor")
        receiving = b.send("browser-pair", "receive_invitation")
        impostor.call("deliver_invitation", ok=False, pairing_path=pairing_path, invitation_path=invitation_path)
        b.wait(receiving, ok=False)
        assert not b.call("browser-pair", "pairing_status")["invitation_saved"]
        info = json.loads(base64.urlsafe_b64decode(pairing.split("/pair/", 1)[1] + "=="))
        wrong_info = json.loads(base64.urlsafe_b64decode(b.call("browser-pair-wrong", "pairing_information").split("/pair/", 1)[1] + "=="))
        substituted = dict(info, address=wrong_info["address"])
        wrong_path = self.write_private("browser-pair-substituted.json", json.dumps(substituted).encode(), False)
        receiving = b.send("browser-pair-wrong", "receive_invitation")
        native_creator.call("deliver_invitation", ok=False, pairing_path=wrong_path, invitation_path=invitation_path)
        b.wait(receiving, ok=False)
        assert not b.call("browser-pair-wrong", "pairing_status")["invitation_saved"]
        b.call("browser-pair", "fail_next_save")
        receiving = b.send("browser-pair", "receive_invitation")
        native_creator.call("deliver_invitation", ok=False, pairing_path=pairing_path, invitation_path=invitation_path)
        b.wait(receiving, ok=False)
        assert not b.call("browser-pair", "pairing_status")["invitation_saved"]
        for _ in range(2):
            receiving = b.send("browser-pair", "receive_invitation")
            native_creator.call("deliver_invitation", pairing_path=pairing_path, invitation_path=invitation_path)
            b.wait(receiving)
        changed = material(invitation)
        changed["version"] = 1 if changed["version"] == 2 else 2
        changed_path = self.write_private("browser-pair-changed-invitation.json", json.dumps(changed).encode(), False)
        receiving = b.send("browser-pair", "receive_invitation")
        native_creator.call("deliver_invitation", ok=False, pairing_path=pairing_path, invitation_path=changed_path)
        b.wait(receiving, ok=False)
        receiving = b.send("browser-pair", "receive_invitation")
        native_creator.call("deliver_invitation", pairing_path=pairing_path, invitation_path=invitation_path)
        b.wait(receiving)
        pending = b.call("browser-pair", "finish_pairing")
        assert pending["id"] == receiver["id"] and pending["stage"] == "AwaitingApproval"
        assert b.call("browser-pair", "request_join", False) == "AwaitingApproval"
        requests = native_creator.call("requests")["requests"]
        assert len(requests) == 1 and requests[0]["applicant"] == receiver["id"]
        native_creator.call("approve", **requests[0])
        assert b.call("browser-pair", "request_join", False) == "Complete"
        assert b.reopen("browser-pair")["id"] == receiver["id"]

        native_receiver = self.start("native-pair", False, "pair")
        native_receiver.call("pairing_information", path=self.private_path("native-pair-information.json", False))
        native_pairing = self.read_private("native-pair-information.json", False).decode()
        full = b.call(browser_creator, "invitation")
        changed = material(full)
        value = changed["invitation"]
        changed["invitation"] = value[:64] + ("a" if value[64] != "a" else "b") + value[65:]
        b.call(browser_creator, "deliver_invitation", native_pairing, json.dumps(changed), ok=False)
        assert not native_receiver.call("pairing_status")["invitation_saved"]
        b.call(browser_creator, "deliver_invitation", native_pairing, full)
        assert native_receiver.call("pairing_status")["invitation_saved"]
        b.call(browser_creator, "deliver_invitation", native_pairing, full)
        result = native_receiver.call("finish_pairing", path=self.private_path("native-pair-invitation.json", False))
        assert result["status"]["node"] == native_receiver.node and result["status"]["stage"] == "AwaitingApproval"
        native_receiver.address = result["address"]
        server = b.send(browser_creator, "serve_admission", False)
        path = self.path("native-pair-invitation.json", False)
        assert native_receiver.call("request", path=path)["stage"] == "AwaitingApproval"
        assert b.wait(server) == "AwaitingApproval"
        pending = [a for a in b.call(browser_creator, "admission_requests") if a["applicant"] == native_receiver.node]
        b.call(browser_creator, "approve_admission", native_receiver.node, pending[0]["request"])
        server = b.send(browser_creator, "serve_admission", False)
        assert native_receiver.call("request", path=path)["stage"] == "Complete"
        assert b.wait(server) == "Complete"
        self.reopen(native_receiver).stop()
        impostor.stop()
        self.record("pairing", "native_browser_pairing_qualified", dict(nativeToBrowser=True, browserToNative=True,
                    wrongCanonicalNetworkRejected=True,
                    wrongSenderRejected=True, substitutedPeerRoutesRejected=True, substitutedMaterialRejected=True,
                    failedSaveNotAcknowledged=True,
                    sameMaterialRetry=True, durableIdentityPreserved=True, ordinaryApprovalStillRequired=True))

    def new_admin_continuation(self, creator):
        b = self.browser
        administrator = "browser-member"
        target = b.call(administrator, "id")
        session = b.call(creator, "begin_management_session")
        seed, genesis = self.browser_credentials[creator]
        b.call(creator, "elevate_genesis", session, seed, genesis, 12000)
        b.call(creator, "manage", session, "appoint-administrator", target)
        b.call(creator, "end_management_session", session)
        server = b.send(creator, "serve_replica")
        assert b.call(administrator, "sync_replica", b.call(creator, "id"))["complete"]
        b.wait(server)
        assert any(member["id"] == target and member["administrator"] for member in b.call(administrator, "control_state")["members"])
        self.record("fixture", "technical_admin_setup", dict(existingSDKManagementOnly=True,
                    signedReplicaOverSDK=True, newProductGenesisConfirmationNotClaimed=True))

        old = b.call(creator, "precode_invitation", int(time.time())+3600, 1)
        b.start("browser-continuation", "join", old)
        server = b.send(creator, "serve_admission", True)
        assert b.call("browser-continuation", "request_join", True) == "PendingMaterials"
        b.wait(server, ok=False)
        old_authorization = b.call("browser-continuation", "admission_material_receipt")["authorization"]
        assert b.call(creator, "precode_usage", material(old)["precode"]) == 1

        old_native = b.call(creator, "precode_invitation", int(time.time())+3600, 1)
        path = self.write_private("continuation-native-old.json", old_native.encode(), False)
        native = self.start("native-continuation", False, "join", path)
        server = b.send(creator, "serve_admission", True)
        assert native.call("request", path=path, authorization_only=True)["stage"] == "PendingMaterials"
        b.wait(server, ok=False)
        assert b.call(creator, "precode_usage", material(old_native)["precode"]) == 1
        native.stop()
        b.call("browser-continuation", "close")
        server = b.send(creator, "serve_replica")
        assert b.call(administrator, "sync_replica", b.call(creator, "id"))["complete"]
        b.wait(server)
        before = b.call(administrator, "control_state")["records"]
        b.call(creator, "close")  # Actual original issuer is unavailable now.

        fresh = b.call(administrator, "precode_invitation", int(time.time())+3600, 1)
        b.call("browser-continuation", "reload")
        pending = b.call("browser-continuation", "start", "join", "{}", fresh)
        assert pending["stage"] == "PendingMaterials"
        server = b.send(administrator, "serve_admission", True)
        assert b.call("browser-continuation", "request_join", False) == "Complete"
        assert b.wait(server) == "Complete"
        assert b.call("browser-continuation", "admission_material_receipt")["authorization"] == old_authorization
        assert b.call(administrator, "precode_usage", material(fresh)["precode"]) == 1

        fresh_native = b.call(administrator, "precode_invitation", int(time.time())+3600, 1)
        path = self.write_private("continuation-native-new.json", fresh_native.encode(), False)
        resumed = Host(self, "native-continuation-resume", False, "resume", native.directory, path)
        assert resumed.node == native.node and resumed.ready["status"]["stage"] == "PendingMaterials"
        server = b.send(administrator, "serve_admission", True)
        assert resumed.call("request", path=path)["stage"] == "Complete"
        assert b.wait(server) == "Complete"
        assert b.call(administrator, "precode_usage", material(fresh_native)["precode"]) == 1
        assert b.call(administrator, "control_state")["records"] == before
        resumed.stop()
        self.record("continuation", "new_admin_precode_qualified", dict(realNativeAndWasm=True,
                    originalIssuerOffline=True, newCodeConsumedOnce=True, existingValidMembershipRetained=True,
                    noDuplicateGrant=True, originalAuthorizationRetained=True, nodeIdentitiesRetained=True))

    def run(self):
        passed, browser_cleanup = False, None
        try:
            self.browser = BrowserControl(self, self.args)
            creator = self.creator(False, "native-provider")
            if self.args.suite in ["all", "core"]:
                self.browser_applicant(creator, "browser-manual")
                self.browser_applicant(creator, "browser-precode", precode=True)
            self.browser_creator("browser-creator")
            if self.args.suite in ["all", "core"]:
                self.native_applicant("browser-creator", "native-manual")
                self.native_applicant("browser-creator", "native-precode", precode=True)
                self.resources(creator)
            if self.args.suite in ["all", "pairing"]:
                self.pairing(creator, "browser-creator")
            self.browser_exchange("browser-creator")
            if self.args.suite in ["all", "continuation"]:
                self.new_admin_continuation("browser-creator")
            passed = True
        finally:
            if hasattr(self, "browser"):
                try: browser_cleanup = self.browser.close()
                except Exception as error: browser_cleanup = {"error": type(error).__name__}
            cleanup = self.cleanup()
            passed = passed and browser_cleanup is not None and "error" not in browser_cleanup and all("error" not in item for item in cleanup)
            (self.output / "result.json").write_text(json.dumps(dict(passed=passed, cleanup=cleanup,
                browserCleanup=browser_cleanup, privateExportsRemoved=not any((self.output/name).exists() for name in self.private)), indent=2))
        assert passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--suite", choices=["all", "core", "pairing", "continuation"], default="all")
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    args = parser.parse_args()
    build = json.loads(args.build_receipt.read_text())
    assert build["exit"] == 0 and build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"]
    actual = {p.relative_to(args.package).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
              for p in args.package.rglob("*") if p.is_file()}
    assert actual == build["artifacts"]
    args.windows = args.windows_sha = None
    args.local_only, args.failure_only, args.focus = True, False, "all"
    runner = BrowserOnboardingRunner(args)
    runner.args = args
    runner.record("runner", "browser_package", dict(package=str(args.package),
                  packageSHA256=build["packageSHA256"], buildReceipt=str(args.build_receipt)))
    runner.run()


if __name__ == "__main__":
    main()
