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
import os
import subprocess
import threading
import time
from concurrent.futures import ThreadPoolExecutor

from check_client_onboarding import OnboardingRunner, Host
from host_native_runtime import group_absent, group_snapshot

ROOT = Path(__file__).resolve().parents[1]
RELAY = "https://aps1-1.relay.n0.iroh.link./"
CONFIGURATION = {"rtc_enabled": False, "relays": [RELAY]}
PRIVATE_OPERATIONS = {"invitation", "precode_invitation", "pairing_information", "export_recovery_card", "member_call", "begin_management_session", "issue_guest", "prepare_node_replacement"}


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

    def wait(self, pending, ok=True, expected_error=None):
        identifier, actor, operation = pending
        deadline = time.monotonic() + 75
        with self.condition:
            while identifier not in self.responses:
                if self.process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("owned browser operation ended without response: " + operation)
                self.condition.wait(timeout=0.1)
            response = self.responses.pop(identifier)
        public = {key: value for key, value in response.items() if key != "privateError"}
        if operation in getattr(self, "private_operations", PRIVATE_OPERATIONS):
            public.pop("result", None)
            public["privateReturn"] = response["ok"]
        self.runner.record(actor, operation, public)
        if not response["ok"]:
            # Preserve exact failures privately before any subsequent getter.
            with (self.runner.output / "private-browser-errors.jsonl").open("a") as out:
                out.write(json.dumps(dict(actor=actor, operation=operation, **response)) + "\n")
        assert response["ok"] == ok, dict(actor=actor, operation=operation, failureClass=response.get("failureClass"))
        if expected_error is not None:
            assert expected_error in response.get("privateError", ""), public
        return response.get("result")

    def call(self, actor, operation, *args, ok=True, expected_error=None):
        return self.wait(self.send(actor, operation, *args), ok=ok, expected_error=expected_error)

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
    def native_configuration(self, remote):
        if self.args.suite not in ["progress", "observation", "receipts", "device-names", "member-management", "departure", "removal-rejoin", "precode-revocation"]:
            return super().native_configuration(remote)
        config = dict(defaults=dict(revision=2, resources=[]),
                      replacement=[dict(url=self.args.relay, uses=["DataRelay"], limitation="onboarding test relay")],
                      disabled=[], disabled_uses=["Observation", "Lookup", "Signaling"],
                      lan_discovery="Disabled", port_mapping="Disabled",
                      maintenance=dict(contact_timeout_ms=10000),
                      udp_direct_disabled=True, tcp_direct_disabled=True)
        return self.write_private("progress-resources.json", json.dumps(config).encode(), remote)

    def progress_configuration(self):
        return dict(CONFIGURATION, relays=[self.args.relay], maintenance=dict(contact_timeout_ms=10000))

    def actor_call(self, actor, wasm, native, *args, ok=True, expected_error=None, **fields):
        if isinstance(actor, str):
            return self.browser.call(actor, wasm, *args, ok=ok, expected_error=expected_error)
        return actor.call(native, ok=ok, **fields)

    def actor_identity(self, actor):
        return self.browser.call(actor, "id") if isinstance(actor, str) else actor.node

    def actor_snapshot(self, actor):
        return self.actor_call(actor, "state_snapshot", "state_snapshot")

    def close_actor(self, actor):
        if isinstance(actor, str):
            assert self.browser.call(actor, "close_page")
        else:
            actor.stop()

    def reopen_actor(self, actor):
        if isinstance(actor, str):
            assert self.browser.call(actor, "reopen_page")
            self.browser.call(actor, "start", "start", "{}", None)
            return actor
        return Host(self, actor.label + "-reopen", False, "open", actor.directory)

    def member_management(self):
        for browser_issuer in (False, True):
            self.management_propagation(browser_issuer)

    def precode_revocation(self):
        browser = self.browser
        config = self.progress_configuration()
        issuer = self.creator(False, "revoke-precode-issuer")

        def issue(name, seconds, uses):
            path = self.private_path(name, False)
            issuer.call("issue_precode", path=path, seconds=seconds, max_uses=uses)
            return path

        def applicant(name, path):
            invitation = Path(path).read_text()
            return browser.start(name, "join", invitation, config)["id"]

        def uses(path):
            return issuer.call("precode_status", path=path)["uses"]

        def history(node):
            return [row for row in issuer.call("admission_history") if row["applicant"] == node]

        def reject(name, path):
            node = applicant(name, path)
            browser.call(name, "request_join", False, ok=False, expected_error="by peer")
            state = browser.call(name, "status")
            assert state["stage"] == "AwaitingApproval" and state["admission_attempt"]["progress"] == "Created"
            assert history(node) == []
            self.close_actor(name)

        revoked = issue("revocable-precode.json", 3600, None)
        retained = issue("retained-precode.json", 3600, 1)
        ordinary = self.private_path("revoke-precode-ordinary.json", False)
        issuer.call("invite", path=ordinary)
        issuer.call("revoke_precode", ok=False, path=ordinary)
        other = self.start("revoke-precode-other-admin", False, "join", ordinary)
        assert other.call("request")["stage"] == "AwaitingApproval"
        request, = issuer.call("requests")["requests"]
        assert request["applicant"] == other.node
        issuer.call("approve", **request)
        assert other.call("request")["stage"] == "Complete"
        proposed = issuer.call("genesis_propose", action="appoint-administrator", target=other.node)
        assert issuer.call("genesis_confirm", operation=proposed["operation"]["id"])["effective_locally"]
        other.call("service_sync", address=issuer.address)
        status = other.call("status")
        assert status["network"] == issuer.call("status")["network"]
        assert any(row["node"] == other.node and row["role"] == "admin" for row in status["members"])
        other.call("revoke_precode", ok=False, path=revoked)
        other.stop()
        member = "revoke-precode-member"
        member_id = applicant(member, revoked)
        assert browser.call(member, "request_join", False) == "Complete"
        admitted, = history(member_id)
        assert admitted["decision"] == "ReceiptReceived"
        self.close_actor(member)

        waiting = "revoke-precode-pending"
        waiting_id = applicant(waiting, revoked)
        assert browser.call(waiting, "request_join", True) == "PendingMaterials"
        pending, = history(waiting_id)
        assert pending["decision"] == "ApprovedAwaitingReceipt" and pending["authorization"]
        assert browser.call(waiting, "status")["admission_attempt"] == {
            "request_id": pending["request_id"], "progress": "Authorized"}
        self.close_actor(waiting)
        assert uses(revoked) == 2 and uses(retained) == 0

        # A separate uncapped code makes expiry independent of exhausted quota.
        lifetime = 20
        expiring = issue("short-lived-precode.json", lifetime, None)
        expiry = time.monotonic() + lifetime
        assert issuer.call("revoke_precode", path=revoked) == {"revoked": True}
        issuer = self.reopen(issuer)
        applicant("retained-precode-before-expiry", expiring)
        assert browser.call("retained-precode-before-expiry", "request_join", False) == "Complete"
        reject("revoke-precode-fresh", revoked)
        assert uses(revoked) == 2 and history(waiting_id) == [pending]

        self.reopen_actor(member)
        assert browser.call(member, "id") == member_id
        assert browser.call(member, "status")["stage"] == "Complete"
        browser.call(member, "sync_services", issuer.node)
        assert history(member_id) == [admitted]
        self.reopen_actor(waiting)
        assert browser.call(waiting, "id") == waiting_id
        assert browser.call(waiting, "request_join", False) == "Complete"
        assert browser.call(waiting, "status")["admission_attempt"] == {
            "request_id": pending["request_id"], "progress": "Complete"}
        assert history(waiting_id) == [dict(pending, decision="ReceiptReceived")]
        assert uses(revoked) == 2

        applicant("retained-precode-member", retained)
        assert browser.call("retained-precode-member", "request_join", False) == "Complete"
        reject("retained-precode-quota", retained)
        assert uses(retained) == 1

        time.sleep(max(0, expiry - time.monotonic()))
        reject("retained-precode-expiry", expiring)
        assert uses(expiring) == 1
        self.record("precode-revocation", "precode_revocation_qualified", dict(
            designatedNodeOnly=True, freshRedemptionDeniedAfterReopen=True,
            existingMembershipUsable=True, sameDeliveryContinuedWithoutExtraUse=True,
            otherCodeUsableUntilQuota=True, otherTimeOnlyCodeUsableUntilExpiry=True, manualApprovalStillRequired=True))

    def removal_rejoin(self, browser_removed):
        browser = self.browser
        prefix = "removal-browser" if browser_removed else "removal-native"
        call, identity, snapshot = self.actor_call, self.actor_identity, self.actor_snapshot
        close, reopen = self.close_actor, self.reopen_actor

        def request(actor, authorization_only=False, ok=True, expected_error=None):
            if isinstance(actor, str):
                return browser.call(actor, "request_join", authorization_only, ok=ok, expected_error=expected_error)
            result = actor.call("request", authorization_only=authorization_only, ok=ok)
            return result["stage"] if ok else None

        def join(creator, suffix, wasm):
            name = prefix + "-" + suffix
            invitation = (browser.call(creator, "invitation") if isinstance(creator, str)
                          else self.native_invitation(creator, name + "-invite"))
            if wasm:
                config = self.progress_configuration()
                if browser_removed:
                    config["maintenance"]["exploration_interval_ms"] = 60000
                browser.start(name, "join", invitation, config)
                actor = name
            else:
                path = self.write_private(name + "-join", invitation.encode(), False)
                actor = self.start(name, False, "join", path)
            assert request(actor) == "AwaitingApproval"
            node = identity(actor)
            entry = next(row for row in snapshot(creator)["pending_admissions"] if row["applicant"] == node)
            approve(creator, node, entry["request_id"])
            assert request(actor) == "Complete"
            return actor

        def wait_for(read, predicate):
            deadline = time.monotonic() + 55
            while True:
                actual = read()
                if predicate(actual):
                    return actual
                assert time.monotonic() < deadline, actual
                time.sleep(0.5)

        def roles(actor):
            return {row["node"]: row["role"] for row in snapshot(actor)["status"]["members"]}

        def sign(action, target):
            proposed = call(creator, "propose_device_operation", "genesis_propose", action, target,
                            action=action, target=target)
            operation = proposed["operation"]["id"]
            if proposed["requires_genesis"]:
                applied = call(creator, "confirm_device_operation", "genesis_confirm", operation, operation=operation)
            else:
                applied = call(creator, "execute_device_operation", "device_execute", operation, operation=operation)
            assert applied["effective_locally"]

        def approve(provider, node, request_id):
            call(provider, "approve_admission", "approve", node, request_id,
                 applicant=node, request_id=request_id)

        def code(name, seconds, uses):
            if isinstance(creator, str):
                return browser.call(creator, "precode_invitation", int(time.time()) + seconds, uses)
            creator.call("issue_precode", path=self.private_path(name, False), seconds=seconds, max_uses=uses)
            return self.read_private(name, False).decode()

        def code_uses(invitation, name):
            if isinstance(creator, str):
                return browser.call(creator, "precode_usage", material(invitation)["precode"])
            return creator.call("precode_status", path=self.private_path(name, False))["uses"]

        def reject_precode():
            if isinstance(creator, str):
                received = browser.send(creator, "serve_progress_admission", True)
                assert browser.call(creator, "wait_progress_admission", received[0])
                request(removed, ok=False)
                browser.wait(received, ok=False, expected_error="admission rejected or materials incomplete")
            else:
                request(removed, ok=False, expected_error="by peer")

        def status(actor):
            return snapshot(actor)["status"]

        def history(request_id):
            return next((row for row in snapshot(creator)["admission_history"]
                         if row["applicant"] == removed_id and row["request_id"] == request_id), None)

        def select_rejoin(invitation, name):
            path = None if isinstance(removed, str) else self.write_private(
                prefix + "-" + name, invitation.encode(), False)
            call(removed, "new_rejoin_attempt", "new_rejoin_attempt", invitation, path=path)

        def holder_operation(wasm, native, ok=True, expected_error=None):
            return call(removed, wasm, native, holder_id, ok=ok, expected_error=expected_error,
                        address=dict(id=holder_id, addrs=[{"Relay": self.args.relay}]))

        def member_file():
            name = "rejoined.bin"
            payload = bytes(index % 251 for index in range(2048))
            if isinstance(removed, str):
                destination = self.output / (prefix + "-received")
                destination.mkdir()
                holder.call("files_enable", directory=str(destination))
                browser.call(removed, "file_make", name, len(payload))
                task = browser.call(removed, "file_send_source", holder_id, name)
            else:
                browser.call(holder, "enable_files")
                path = self.write_private(name, payload, False)
                address = dict(id=holder_id, addrs=[{"Relay": self.args.relay}])
                task = removed.call("file_send", address=address, path=path)["task"]

            def receipt():
                status = call(holder, "file_status", "files_status")
                return next((row["receipt"] for row in status["tasks"]
                             if row["id"] == task and row["state"] == "complete"), None)

            result = wait_for(receipt, bool)
            assert result["size"] == len(payload)
            if isinstance(holder, str):
                assert result["saved_name"] in browser.call(holder, "file_list")
            else:
                assert (destination / result["saved_name"]).read_bytes() == payload

        if browser_removed:
            creator = self.creator(False, prefix)
        else:
            creator = prefix + "-creator"
            browser.start(creator, "create", configuration=self.progress_configuration())
            browser.call(creator, "export_recovery_card")
            browser.call(creator, "acknowledge_recovery_saved")
        removed = join(creator, "removed", browser_removed)
        holder = join(creator, "holder", not browser_removed)
        removed_id, holder_id = map(identity, (removed, holder))
        wait_for(lambda: roles(removed), lambda state: state.get(holder_id) == "member")
        initial = status(removed)
        old_attempt = initial["admission_attempt"]
        assert history(old_attempt["request_id"])["decision"] == "ReceiptReceived"
        holder_operation("sync_services", "service_sync")
        holder_operation("check_membership_removal", "check_membership_removal", ok=False)
        close(removed)
        sign("appoint-administrator", holder_id)
        sign("demote-administrator", holder_id)
        sign("remove-member", removed_id)
        later = join(creator, "after-removal", False)
        later_id = identity(later)
        if not browser_removed:
            # Make the post-removal record available before shutting A down.
            browser.call(holder, "sync_services", identity(creator))
        wait_for(lambda: roles(holder), lambda state: state.get(holder_id) == "member" and
                 removed_id not in state and later_id in state)
        close(later)
        close(creator)
        close(holder)

        removed = reopen(removed)
        assert identity(removed) == removed_id
        if browser_removed:
            holder_operation("check_membership_removal", "check_membership_removal", ok=False,
                             expected_error="temporarily unreachable")
        else:
            assert holder_operation("check_membership_removal", "check_membership_removal") == {
                "stage": "TemporarilyUnreachable"}
        offline = status(removed)
        assert offline["stage"] == "Complete" and offline["admission_attempt"] == old_attempt
        holder = reopen(holder)
        if browser_removed:
            holder_operation("sync_services", "service_sync", ok=False)
            assert status(removed)["stage"] == "Complete"
            assert holder_operation("check_membership_removal", "check_membership_removal") is True
        wait_for(lambda: status(removed)["stage"], lambda stage: stage == "Removed")
        ended = status(removed)
        assert not ended["automatic_retry"] and ended["admission_attempt"] == old_attempt
        assert ended["records"] == initial["records"] + 3
        assert removed_id not in roles(removed) and later_id not in roles(removed)
        holder_operation("sync_services", "service_sync", ok=False)
        request(removed, ok=False)
        close(removed)
        removed = reopen(removed)
        assert status(removed)["stage"] == "Removed"

        call(removed, "new_manual_attempt", "new_manual_attempt")
        created = status(removed)["admission_attempt"]
        new_id = created["request_id"]
        assert created["progress"] == "Created" and new_id != old_attempt["request_id"]
        request(removed, ok=False)
        close(removed)
        removed = reopen(removed)
        assert identity(removed) == removed_id and status(removed)["admission_attempt"] == created
        assert status(removed)["stage"] == "AwaitingApproval"
        creator = reopen(creator)
        assert request(removed) == "AwaitingApproval"
        old = history(old_attempt["request_id"])
        assert old["decision"] == "ReceiptReceived" and old["authorization"]
        assert history(new_id)["decision"] == "AwaitingApproval"
        approve(creator, removed_id, new_id)
        assert request(removed, authorization_only=True) == "PendingMaterials"
        close(removed)
        removed = reopen(removed)
        assert status(removed)["stage"] == "PendingMaterials"
        assert status(removed)["admission_attempt"] == dict(request_id=new_id, progress="Authorized")
        assert request(removed) == "Complete"
        assert status(removed)["admission_attempt"] == dict(request_id=new_id, progress="Complete")
        joined = history(new_id)
        assert joined["decision"] == "ReceiptReceived" and joined["authorization"] != old["authorization"]
        assert history(old_attempt["request_id"]) == old

        wait_for(lambda: roles(holder), lambda state: state.get(removed_id) == "member")
        close(removed)
        if browser_removed:
            # C never saw this ordinary holder's Add. It has to learn B's
            # membership from the same signed proof, with its old B offline.
            close(holder)
            holder = join(creator, "unknown-holder", False)
            holder_id = identity(holder)
        sign("remove-member", removed_id)
        valid_name = prefix + "-new-precode"
        valid = code(valid_name, 3600, 2)
        expired_by = time.time() + 4
        expired_name = prefix + "-expired-precode"
        expired = code(expired_name, 3, 1)
        wait_for(lambda: roles(holder), lambda state: removed_id not in state)
        close(creator)
        removed = reopen(removed)
        if browser_removed:
            assert holder_id not in roles(removed)
            assert holder_operation("check_membership_removal", "check_membership_removal") is True
            assert roles(removed)[holder_id] == "member"
        wait_for(lambda: status(removed)["stage"], lambda stage: stage == "Removed")
        creator = reopen(creator)
        time.sleep(max(0, expired_by - time.time()))
        select_rejoin(expired, "try-expired")
        reject_precode()
        expired_attempt = status(removed)["admission_attempt"]
        assert status(removed)["stage"] == "AwaitingApproval" and expired_attempt["progress"] == "Created"
        assert history(expired_attempt["request_id"]) is None and code_uses(expired, expired_name) == 0
        select_rejoin(valid, "try-valid")
        precode_id = status(removed)["admission_attempt"]["request_id"]
        assert precode_id not in (new_id, expired_attempt["request_id"])
        close(removed)
        removed = reopen(removed)
        assert status(removed)["admission_attempt"] == dict(request_id=precode_id, progress="Created")
        assert request(removed, authorization_only=True) == "PendingMaterials"
        close(removed)
        removed = reopen(removed)
        assert identity(removed) == removed_id and status(removed)["stage"] == "PendingMaterials"
        assert status(removed)["admission_attempt"] == dict(request_id=precode_id, progress="Authorized")
        assert request(removed) == "Complete"
        assert code_uses(valid, valid_name) == 1
        assert history(precode_id)["decision"] == "ReceiptReceived" and history(new_id) == joined
        wait_for(lambda: roles(holder), lambda state: state.get(removed_id) == "member")
        call(removed, "set_device_display_name", "set_device_display_name", "Rejoined", display_name="Rejoined")
        holder_operation("sync_services", "service_sync")
        assert next(row.get("display_name") for row in status(holder)["members"] if row["node"] == removed_id) == "Rejoined"
        member_file()
        close(removed)
        sign("remove-member", removed_id)
        wait_for(lambda: roles(holder), lambda state: removed_id not in state)
        close(creator)
        removed = reopen(removed)
        if browser_removed:
            assert holder_operation("check_membership_removal", "check_membership_removal") is True
        wait_for(lambda: status(removed)["stage"], lambda stage: stage == "Removed")
        creator = reopen(creator)
        select_rejoin(expired, "new-selection")
        select_rejoin(valid, "old-precode-replay")
        replay_id = status(removed)["admission_attempt"]["request_id"]
        reject_precode()
        assert code_uses(valid, valid_name) == 1 and status(removed)["stage"] == "AwaitingApproval"
        assert history(replay_id) is None
        assert removed_id not in roles(creator)
        close(creator)
        close(removed)
        close(holder)
        self.record(prefix, "removal_rejoin", dict(
            holderWasOrdinary=True, unavailableRetainedGrant=True, onlyCausalHistory=True,
            oldMemberSessionDenied=True, bothRejoinsDurable=True, sameIdentityNewRequests=True,
            providerKeptOlderReceipts=True, expiredCodeDenied=True, freshCodeUsedOnce=True,
            oldCodeCannotRejoinAgain=True, unknownHolderVerified=browser_removed,
            fullSyncAndMemberFileAfterRejoin=True))

    def departures(self):
        browser = self.browser
        config = self.progress_configuration()

        call, identity, snapshot = self.actor_call, self.actor_identity, self.actor_snapshot
        close, reopen = self.close_actor, self.reopen_actor

        def members(actor):
            return {row["node"]: row["role"] for row in snapshot(actor)["status"]["members"]}

        def wait_for(read, predicate):
            deadline = time.monotonic() + 60
            while True:
                current = read()
                if predicate(current):
                    return current
                assert time.monotonic() < deadline, current
                time.sleep(0.5)

        def admit(creator, name, wasm):
            invite = (browser.call(creator, "invitation") if isinstance(creator, str)
                      else self.native_invitation(creator, name + "-invite"))
            if wasm:
                browser.start(name, "join", invite, config)
                actor = name
            else:
                path = self.write_private(name + "-join", invite.encode(), False)
                actor = self.start(name, False, "join", path)
            stage = lambda: (browser.call(actor, "request_join", False)
                             if wasm else actor.call("request")["stage"])
            assert stage() == "AwaitingApproval"
            node = identity(actor)
            entry = next(row for row in snapshot(creator)["pending_admissions"] if row["applicant"] == node)
            call(creator, "approve_admission", "approve", node, entry["request_id"],
                 applicant=node, request_id=entry["request_id"])
            assert stage() == "Complete"
            return actor

        for browser_leaves in (True, False):
            prefix = "departure-browser" if browser_leaves else "departure-native"
            creator = self.creator(False, prefix)
            leaving = admit(creator, prefix + "-leaving", browser_leaves)
            holder = admit(creator, prefix + "-holder", not browser_leaves)
            late = admit(creator, prefix + "-late", False)
            leaving_id, holder_id, late_id = map(identity, (leaving, holder, late))
            wait_for(lambda: members(leaving), lambda state: holder_id in state and late_id in state)
            wait_for(lambda: members(holder), lambda state: leaving_id in state and late_id in state)
            wait_for(lambda: members(late), lambda state: leaving_id in state and holder_id in state)
            holder_address = dict(id=holder_id, addrs=[{"Relay": self.args.relay}])
            call(leaving, "sync_services", "service_sync", holder_id, address=holder_address)
            close(late)
            close(holder)
            if browser_leaves:
                # Only A can obtain this predecessor before it leaves. The
                # first Native recipient and late Native peer start without it.
                proposed = creator.call("genesis_propose", action="appoint-administrator", target=late_id)
                assert creator.call("genesis_confirm", operation=proposed["operation"]["id"])["effective_locally"]
                wait_for(lambda: members(leaving), lambda state: state.get(late_id) == "admin")
            close(creator)

            proposed = call(leaving, "propose_device_operation", "genesis_propose", "leave", leaving_id,
                            action="leave", target=leaving_id)
            assert not proposed["requires_genesis"] and not proposed["effective_locally"]
            operation = proposed["operation"]["id"]
            applied = call(leaving, "execute_device_operation", "device_execute", operation, operation=operation)
            assert applied["effective_locally"] and applied["record"]
            assert applied["first_delivery"] == {"stage": "pending"}
            record = applied["record"]

            def departure():
                state = snapshot(leaving)
                assert leaving_id not in {row["node"] for row in state["status"]["members"]}
                assert state["status"]["stage"] == "Removed"
                entry = next(row for row in state["device_operations"]["operations"] if row["operation"]["id"] == operation)
                assert entry["record"] == record and entry["effective_locally"]
                return entry["first_delivery"]

            assert departure() == {"stage": "pending"}
            close(leaving)
            leaving = reopen(leaving)
            assert identity(leaving) == leaving_id and departure() == {"stage": "pending"}

            if not browser_leaves:
                # The receiver has the upload, but its real IndexedDB
                # transaction is still outstanding. Disconnect before it
                # sends the receipt, then require a duplicate to confirm it.
                close(leaving)
                holder = reopen(holder)
                assert browser.call(holder, "hold_admission_commit")
                leaving = reopen(leaving)
                assert browser.call(holder, "wait_admission_commit", 45000)
                assert departure() == {"stage": "pending"}
                close(leaving)
                assert browser.call(holder, "release_admission_commit")
                wait_for(lambda: members(holder), lambda state: leaving_id not in state)
                close(holder)
                leaving = reopen(leaving)
                assert departure() == {"stage": "pending"}
                holder = reopen(holder)
            else:
                holder = reopen(holder)
            assert wait_for(departure, lambda delivery: delivery["stage"] == "confirmed") == {
                "stage": "confirmed", "recipient": holder_id,
            }
            assert leaving_id not in members(holder)
            if browser_leaves:
                assert members(holder)[late_id] == "admin"
            call(leaving, "sync_services", "service_sync", holder_id, ok=False, address=holder_address)
            close(leaving)
            close(holder)
            leaving = reopen(leaving)
            assert departure() == {"stage": "confirmed", "recipient": holder_id}
            close(leaving)
            holder = reopen(holder)
            assert identity(holder) == holder_id and leaving_id not in members(holder)
            late = reopen(late)
            assert identity(late) == late_id
            wait_for(lambda: members(late), lambda state: leaving_id not in state)
            close(late)
            late = reopen(late)
            assert leaving_id not in members(late) and members(late)[holder_id] == "member"
            if browser_leaves:
                assert members(late)[late_id] == "admin"
            close(holder)
            close(late)
            self.record(prefix, "departure_delivery", dict(localExit=True, pendingAfterReopen=True,
                sameRecordConfirmed=True, memberConnectionDenied=True, holderReopened=True,
                lostReceiptRetried=not browser_leaves, confirmationDurable=True,
                departingNodeOffline=True, lateMemberReopened=True))

        administrator = "departure-administrator"
        browser.start(administrator, "create", configuration=config)
        browser.call(administrator, "export_recovery_card")
        browser.call(administrator, "acknowledge_recovery_saved")
        peer = admit(administrator, administrator + "-peer", True)
        administrator_id, peer_id = map(identity, (administrator, peer))
        accepted = browser.send(peer, "member_accept", 10000)
        browser.call(administrator, "member_connect", peer_id)
        browser.wait(accepted)
        reading = browser.send(peer, "data_accept")
        browser.call(administrator, "data_open")
        browser.call(administrator, "data_write", 1, 7)
        browser.wait(reading)
        assert browser.call(peer, "data_read", 7) == dict(length=1, matches=True)
        proposed = browser.call(administrator, "propose_device_operation", "leave", administrator_id)
        assert proposed["requires_genesis"] and not proposed["effective_locally"]
        operation = proposed["operation"]["id"]
        browser.call(administrator, "execute_device_operation", operation, ok=False)
        assert snapshot(administrator)["status"]["stage"] == "Complete"
        approved = browser.call(administrator, "confirm_device_operation", operation)
        assert approved["effective_locally"] and approved["first_delivery"] == {"stage": "pending"}
        assert browser.call(administrator, "member_call", "is_closed", [])
        browser.call(administrator, "member_connect", peer_id, ok=False)
        wait_for(lambda: members(peer), lambda state: administrator_id not in state)
        assert wait_for(
            lambda: snapshot(administrator)["device_operations"]["operations"][0]["first_delivery"],
            lambda delivery: delivery["stage"] == "confirmed",
        ) == {"stage": "confirmed", "recipient": peer_id}
        close(administrator)
        close(peer)
        self.record("departure-administrator", "genesis_departure", dict(
            requiresPerActionGenesis=True, activeMemberSessionClosed=True, newMemberConnectionDenied=True))

    def management_propagation(self, browser_issuer):
        browser = self.browser
        prefix = "management-" + ("browser-issuer" if browser_issuer else "native-issuer")
        config = self.progress_configuration()

        call, identity, snapshot = self.actor_call, self.actor_identity, self.actor_snapshot
        close, reopen = self.close_actor, self.reopen_actor

        def roles(actor):
            return {entry["node"]: entry["role"] for entry in snapshot(actor)["status"]["members"]}

        def expect_roles(actor, expected):
            deadline = time.monotonic() + 55
            while True:
                actual = roles(actor)
                if all(actual.get(node) == role for node, role in expected.items()):
                    return
                assert time.monotonic() < deadline, (expected, actual)
                time.sleep(0.5)

        def request(provider, suffix, wasm):
            name = prefix + "-" + suffix
            if isinstance(provider, str):
                invitation = browser.call(provider, "invitation")
            else:
                invitation = self.native_invitation(provider, name + "-invite")
            if wasm:
                browser.start(name, "join", invitation, config)
                actor = name
            else:
                path = self.write_private(name + "-join", invitation.encode(), False)
                actor = self.start(name, False, "join", path)
            assert request_stage(actor) == "AwaitingApproval"
            entry = next(row for row in snapshot(provider)["pending_admissions"] if row["applicant"] == identity(actor))
            return actor, entry

        def request_stage(actor):
            return browser.call(actor, "request_join", False) if isinstance(actor, str) else actor.call("request")["stage"]

        def approve(provider, entry, ok=True):
            applicant, request_id = entry["applicant"], entry["request_id"]
            call(provider, "approve_admission", "approve", applicant, request_id,
                 ok=ok, applicant=applicant, request_id=request_id)

        def admit(provider, suffix, wasm):
            actor, entry = request(provider, suffix, wasm)
            approve(provider, entry)
            assert request_stage(actor) == "Complete"
            return actor

        def signed(action, target):
            proposed = call(issuer, "propose_device_operation", "genesis_propose", action, target,
                            action=action, target=target)
            assert proposed["requires_genesis"] and not proposed["effective_locally"]
            op = proposed["operation"]["id"]
            result = call(issuer, "confirm_device_operation", "genesis_confirm", op, operation=op)
            assert result["effective_locally"] and result["record"]

        def browser_link(left, right):
            pending = browser.send(left, "member_accept", 10000)
            browser.call(right, "member_connect", identity(left))
            browser.wait(pending)

        def communicate(left, right):
            if not isinstance(left, str):
                self.exchange(left, right)
                return
            browser_link(left, right)
            pending = browser.send(left, "member_call", "serve_exchange", [])
            message = "MioCube management propagation"
            assert browser.call(right, "member_call", "exchange", [message]) == browser.wait(pending) == message
            for actor in (left, right):
                browser.call(actor, "member_call", "close", [])

        if browser_issuer:
            issuer = prefix
            browser.start(issuer, "create", configuration=config)
            browser.call(issuer, "export_recovery_card")
            browser.call(issuer, "acknowledge_recovery_saved")
        else:
            issuer = self.creator(False, prefix)
        holder = admit(issuer, "holder", not browser_issuer)
        late = admit(issuer, "late", not browser_issuer)
        removed = admit(issuer, "removed", not browser_issuer)
        holder_id, late_id, removed_id = map(identity, (holder, late, removed))
        expect_roles(holder, {late_id: "member", removed_id: "member"})
        expect_roles(late, {holder_id: "member", removed_id: "member"})

        signed("appoint-administrator", holder_id)
        expect_roles(holder, {holder_id: "admin"})
        approved = admit(holder, "approved-by-appointed", not browser_issuer)
        approved_id = identity(approved)
        # Anchor the grant before demotion so the unrelated positive control
        # does not depend on a concurrent, deauthorized administrator grant.
        expect_roles(issuer, {approved_id: "member"})
        pending, entry = request(holder, "pending-before-demotion", browser_issuer)
        signed("demote-administrator", holder_id)
        expect_roles(holder, {holder_id: "member"})
        approve(holder, entry, ok=False)
        assert snapshot(pending)["status"]["stage"] == "AwaitingApproval"
        close(pending)
        communicate(holder, late)

        expect_roles(late, {holder_id: "member"})
        close(late)
        if isinstance(holder, str):
            browser_link(holder, removed)
            opening = browser.send(holder, "data_accept")
            browser.call(removed, "data_open")
            browser.call(removed, "data_write", 1, 7)
            browser.wait(opening)
            assert browser.call(holder, "data_read", 7) == dict(length=1, matches=True)
            reading = browser.send(holder, "data_read", 7)
        else:
            holder.call("terminal_enable", directory=self.path(prefix + "-terminal", False))
            workspace = removed.call("terminal_create", address=holder.address)["workspace"]
            attachment = removed.call("terminal_attach", address=holder.address, workspace=workspace)["id"]
            command = b"stty -echo; printf 'MIO187_'; printf 'OPEN\\n'\n"
            removed.call("terminal_send", attachment=attachment, message=dict(type="bytes", data=list(command)))
            output = bytearray()
            deadline = time.monotonic() + 10
            while b"MIO187_OPEN" not in output:
                assert time.monotonic() < deadline, output
                event = removed.call("terminal_read", attachment=attachment, timeout_ms=500)
                output.extend(event.get("message", {}).get("data", []))

        # The offline peer lacks both the preceding grant and the removal that
        # needs it. The issuer then grants the late peer an administrator role.
        signed("appoint-administrator", removed_id)
        signed("remove-member", removed_id)
        signed("appoint-administrator", late_id)
        expected = {holder_id: "member", late_id: "admin", removed_id: None}
        expect_roles(holder, expected)
        if isinstance(holder, str):
            browser.wait(reading, ok=False)
            assert browser.call(holder, "member_call", "is_closed", [])
            browser.call(removed, "member_connect", holder_id, ok=False)
        else:
            deadline = time.monotonic() + 10
            while holder.call("terminal_status")["attachment_process_owners"]:
                assert time.monotonic() < deadline
                time.sleep(0.1)
            # The old handle can still deliver output buffered before removal.
            deadline = time.monotonic() + 10
            while True:
                end = removed.call("terminal_read", attachment=attachment, timeout_ms=500)
                if not end["attached"]:
                    break
                assert time.monotonic() < deadline, end
            assert not end["attached"] and end.get("failed"), end
            removed.call("ping", address=holder.address, ok=False)
        close(removed)
        assert roles(holder)[approved_id] == "member"
        communicate(holder, approved)
        close(approved)

        close(issuer)
        close(holder)
        holder = reopen(holder)
        assert all(roles(holder).get(node) == role for node, role in expected.items())
        late = reopen(late)
        expect_roles(late, expected)
        communicate(holder, late)
        close(holder)
        close(late)
        late = reopen(late)
        assert all(roles(late).get(node) == role for node, role in expected.items())
        close(late)
        self.record(prefix, "automatic_member_management", dict(appointmentAllowedApproval=True,
            demotionDeniedApproval=True, ordinaryCommunicationRetained=True, openStreamRevoked=True,
            freshMemberConnectionRejected=True, ordinaryHolderReopened=True, signerUnavailable=True,
            dependentRecordsAppliedByLatePeer=True, durableAfterReopen=True))

    def device_names(self):
        self.browser_applicant_display_name()
        self.native_applicant_display_name()
        self.record("device-names", "display_names_qualified", dict(
            nativeAndWasmDefaults=True, sameRequestAfterOfflineRename=True,
            matchingNamesDistinctIdentities=True, admittedPeersCommunicate=True,
            independentReaderAfterOriginClosed=True, idOnlyDataReopens=True,
            sameIdentityReplacementRetainsName=True, idOnlyBackupRestores=True))

    @staticmethod
    def member_by_id(snapshot, node):
        return next(member for member in snapshot["status"]["members"] if member["node"] == node)

    def browser_applicant_display_name(self):
        b, actor, reader = self.browser, "browser-named-applicant", "browser-name-reader"
        provider = self.creator(False, "native-name-provider")
        assert provider.call("status")["display_name"] == os.uname().nodename
        invitation = self.native_invitation(provider, "name-invitation")
        config = self.progress_configuration()
        started = b.start(actor, "join", invitation, dict(config, host_device_name="Browser host"))
        assert started.get("display_name") == "Browser host"
        b.call(actor, "set_device_display_name", "Blue Cube")
        assert b.call(actor, "request_join", False) == "AwaitingApproval"
        request, = provider.call("state_snapshot")["pending_admissions"]
        assert request["applicant"] == started["id"] and request["display_name"] == "Blue Cube"
        assert provider.call("requests")["requests"] == [request]
        original = b.call(actor, "status")["admission_attempt"]

        provider.stop()
        b.call(actor, "set_device_display_name", "Common Cube")
        b.call(actor, "reload")
        after = b.call(actor, "start", "start", json.dumps(dict(config, host_device_name="Changed host")), None)
        assert after["display_name"] == "Common Cube" and after["admission_attempt"] == original
        provider = Host(self, "native-name-provider-pending", False, "open", provider.directory)
        before = provider.call("state_snapshot")
        assert before["pending_admissions"] == [request]
        with ThreadPoolExecutor(max_workers=1) as waiter:
            observing = waiter.submit(provider.call, "observe_state", after=before["cursor"])
            assert b.call(actor, "request_join", False) == "AwaitingApproval"
            renamed = observing.result(timeout=15)
        assert renamed["pending_admissions"] == [dict(request, display_name="Common Cube")]
        history, = provider.call("admission_history")
        assert history["request_id"] == request["request_id"] and history["display_name"] == "Common Cube"
        assert b.call(actor, "status")["admission_attempt"] == original
        provider.call("approve", applicant=started["id"], request_id=request["request_id"])
        assert b.call(actor, "request_join", False) == "Complete"
        assert self.member_by_id(provider.call("state_snapshot"), started["id"]) == {
            "node": started["id"], "role": "member", "display_name": "Common Cube"}

        # This browser has no host-supplied name and sends the original ID-only request.
        reader_id = b.start(reader, "join", invitation, config)["id"]
        assert b.call(reader, "request_join", False) == "AwaitingApproval"
        unnamed, = provider.call("requests")["requests"]
        assert unnamed == {"applicant": reader_id, "request_id": unnamed["request_id"]}
        assert reader_id != started["id"]
        provider.call("approve", applicant=reader_id, request_id=request["request_id"], ok=False)
        provider.call("approve", **unnamed)
        assert b.call(reader, "request_join", False) == "Complete"
        provider = self.reopen(provider)
        assert "display_name" not in self.member_by_id(provider.call("state_snapshot"), reader_id)
        assert "display_name" not in b.reopen(reader)
        b.call(reader, "set_device_display_name", "Common Cube")
        b.call(reader, "sync_services", provider.node)
        b.call(actor, "sync_services", provider.node)
        snapshot = provider.call("state_snapshot")
        assert {entry["node"] for entry in snapshot["status"]["members"]
                if entry.get("display_name") == "Common Cube"} == {reader_id, started["id"]}
        accepting = b.send(reader, "member_accept", 10000)
        b.call(actor, "member_connect", reader_id)
        b.wait(accepting)
        reading = b.send(reader, "member_call", "serve_exchange", [])
        assert b.call(actor, "member_call", "exchange", ["Two Common Cubes"]) == b.wait(reading) == "Two Common Cubes"
        b.call(actor, "member_call", "close", [])
        b.call(reader, "member_call", "close", [])

        # The origin can save offline, then the reader learns from a different
        # member after the origin closes again.
        provider.stop()
        b.call(reader, "close_page")
        b.call(actor, "set_device_display_name", "Desk Cube")
        assert b.reopen(actor)["display_name"] == "Desk Cube"
        provider = Host(self, "native-name-provider-relay", False, "open", provider.directory)
        assert self.member_by_id(provider.call("state_snapshot"), started["id"])["display_name"] == "Common Cube"
        b.call(actor, "sync_services", provider.node)
        b.call(actor, "close_page")
        provider = self.reopen(provider)
        b.call(reader, "reopen_page")
        b.call(reader, "start", "start", "{}", None)
        assert self.member_by_id(b.call(reader, "state_snapshot"), started["id"])["display_name"] == "Common Cube"
        b.call(reader, "sync_services", provider.node)
        b.reopen(reader)
        assert self.member_by_id(b.call(reader, "state_snapshot"), started["id"]) == {
            "node": started["id"], "role": "member", "display_name": "Desk Cube"}
        b.call(actor, "reopen_page")
        b.call(actor, "start", "start", "{}", None)
        capsule = b.call(actor, "prepare_node_replacement")
        backup = self.write_private("browser-named-backup", capsule.encode(), False)
        restored = Host(self, "native-restored-browser-name", False, "restore",
                        self.path("restored-browser-name-state", False), backup)
        assert restored.node == started["id"]
        assert restored.ready["status"]["stage"] == "RestoringAuthorization"
        assert self.reopen(restored).call("status")["display_name"] == "Desk Cube"

    def native_applicant_display_name(self):
        b, provider = self.browser, "browser-name-provider"
        provider_id = b.start(provider, "create", configuration=dict(
            self.progress_configuration(), host_device_name="Browser provider"))["id"]
        b.call(provider, "export_recovery_card")
        b.call(provider, "acknowledge_recovery_saved")
        invitation = b.call(provider, "invitation")
        path = self.write_private("native-name-invitation", invitation.encode(), False)
        origin = self.start("native-named-applicant", False, "join", path)
        assert origin.call("status")["display_name"] == os.uname().nodename
        origin.call("set_device_display_name", display_name="Native Cube")
        assert origin.call("request")["stage"] == "AwaitingApproval"
        request, = b.call(provider, "state_snapshot")["pending_admissions"]
        assert request["applicant"] == origin.node and request["display_name"] == "Native Cube"
        b.call(provider, "close_page")
        origin.call("set_device_display_name", display_name="Other Cube")
        origin = self.reopen(origin)
        assert origin.call("status")["display_name"] == "Other Cube"
        b.call(provider, "reopen_page")
        b.call(provider, "start", "start", "{}", None)
        before = b.call(provider, "state_snapshot")
        assert before["pending_admissions"] == [request]
        observing = b.send(provider, "observe_state", before["cursor"])
        assert origin.call("request")["stage"] == "AwaitingApproval"
        assert b.wait(observing)["pending_admissions"] == [dict(request, display_name="Other Cube")]
        history, = b.call(provider, "admission_history")
        assert history["request_id"] == request["request_id"] and history["display_name"] == "Other Cube"
        b.call(provider, "approve_admission", origin.node, request["request_id"])
        assert origin.call("request")["stage"] == "Complete"
        assert self.member_by_id(b.call(provider, "state_snapshot"), origin.node) == {
            "node": origin.node, "role": "member", "display_name": "Other Cube"}

        reader = self.start("native-name-reader", False, "join", path)
        reader.call("set_device_display_name", display_name="Other Cube")
        assert reader.call("request")["stage"] == "AwaitingApproval"
        pending, = b.call(provider, "state_snapshot")["pending_admissions"]
        assert pending["applicant"] == reader.node != origin.node
        b.call(provider, "approve_admission", reader.node, pending["request_id"])
        assert reader.call("request")["stage"] == "Complete"
        assert {entry["node"] for entry in b.call(provider, "state_snapshot")["status"]["members"]
                if entry.get("display_name") == "Other Cube"} == {reader.node, origin.node}
        provider_address = dict(id=provider_id, addrs=[{"Relay": self.args.relay}])
        origin.call("service_sync", address=provider_address)
        reader.call("service_sync", address=provider_address)
        self.exchange(origin, reader)

        b.call(provider, "close_page")
        reader.stop()
        origin.call("set_device_display_name", display_name="Travel Cube")
        origin = self.reopen(origin)
        b.call(provider, "reopen_page")
        b.call(provider, "start", "start", "{}", None)
        origin.call("service_sync", address=provider_address)
        origin.stop()
        b.reopen(provider)
        reader = Host(self, "native-name-reader-reopened", False, "open", reader.directory)
        assert self.member_by_id(reader.call("state_snapshot"), origin.node)["display_name"] == "Other Cube"
        reader.call("service_sync", address=provider_address)
        reader = self.reopen(reader)
        assert self.member_by_id(reader.call("state_snapshot"), origin.node) == {
            "node": origin.node, "role": "member", "display_name": "Travel Cube"}
        origin = Host(self, "native-name-replacement-source", False, "open", origin.directory)
        backup = self.private_path("native-named-backup", False)
        prepared = origin.call("replacement_prepare", path=backup)
        assert prepared["saved"] and prepared["source_stopped"]
        origin.process.stdin.close()
        assert origin.process.wait(timeout=20) == 0
        origin.stderr.close()
        capsule = json.loads(Path(backup).read_text())
        legacy = dict(capsule, node={key: value for key, value in capsule["node"].items() if key != "device_names"})
        legacy_actor = "browser-id-only-replacement"
        restored = b.start(legacy_actor, "restore_node_replacement", json.dumps(legacy), {})
        assert restored["id"] == origin.node and "display_name" not in restored
        b.call(legacy_actor, "close_page")

        target = "browser-named-replacement"
        restored = b.start(target, "restore_node_replacement", json.dumps(capsule), {
            "host_device_name": "Different host"})
        assert restored["id"] == origin.node and restored["display_name"] == "Travel Cube"
        completed = json.loads(b.call(target, "complete_node_replacement", reader.node))
        assert completed["ready"] and b.reopen(target)["display_name"] == "Travel Cube"
        receipt = self.write_private("named-replacement-receipt", json.dumps(completed["receipt"]).encode(), False)
        finalize = subprocess.run([str(self.linux), "finalize-replacement", origin.directory, receipt],
                                  capture_output=True, text=True, timeout=30, check=True)
        retired = json.loads(finalize.stdout)
        assert retired["source_retired"] and "device-names.json" in retired["removed_material_files"]

    def serve_progress_admission(self, actor, precode):
        pending = self.browser.send(actor, "serve_progress_admission", precode)
        self.browser.call(actor, "wait_progress_admission", pending[0])
        return pending

    def online_admission_observation(self):
        b = self.browser
        creator = "browser-online-provider"
        b.start(creator, "create", configuration=self.progress_configuration())
        b.call(creator, "export_recovery_card")
        b.call(creator, "acknowledge_recovery_saved")
        invitation = b.call(creator, "invitation")
        code = b.call(creator, "precode_invitation", int(time.time()) + 3600, 1)
        manual_path = self.write_private("online-manual-invitation", invitation.encode(), False)
        code_path = self.write_private("online-precode-invitation", code.encode(), False)
        first = self.start("native-online-first", False, "join", manual_path)
        second = self.start("native-online-second", False, "join", manual_path)
        granted = self.start("native-online-precode", False, "join", code_path)
        initial = b.call(creator, "state_snapshot")
        assert initial["pending_admissions"] == initial["admission_history"] == []
        with ThreadPoolExecutor(max_workers=3) as peers:
            manual = [peers.submit(node.call, "request") for node in (first, second)]
            redeem = peers.submit(granted.call, "request")
            assert redeem.result()["stage"] == "Complete"
            operation = b.call(creator, "propose_device_operation", "appoint-administrator", granted.node)["operation"]["id"]
            assert b.call(creator, "confirm_device_operation", operation)["effective_locally"]
            assert [work.result()["stage"] for work in manual] == ["AwaitingApproval"] * 2
        observed = b.call(creator, "observe_state", initial["cursor"])
        assert observed == b.call(creator, "state_snapshot")
        assert {entry["applicant"] for entry in observed["pending_admissions"]} == {first.node, second.node}
        assert {first.node: "AwaitingApproval", second.node: "AwaitingApproval", granted.node: "ReceiptReceived"} == {
            entry["applicant"]: entry["decision"] for entry in observed["admission_history"]}
        assert self.member_by_id(observed, granted.node)["role"] == "admin"
        assert any(item["id"] == granted.node and item["administrator"] for item in b.call(creator, "control_state")["members"])
        assert observed["device_operations"] == b.call(creator, "device_operations")
        authorization = next(entry["authorization"] for entry in observed["admission_history"] if entry["applicant"] == granted.node)
        assert authorization in b.call(creator, "admission_material_receipt")["records"]
        assert b.call(creator, "precode_usage", material(code)["precode"]) == 1

        # Only a rejection changes this snapshot. It must wake an already
        # pending observation without another request, receipt or role change.
        assert b.call(creator, "begin_observation", observed["cursor"])
        first_request = next(entry for entry in observed["pending_admissions"] if entry["applicant"] == first.node)
        b.call(creator, "reject_admission", first.node, first_request["request_id"])
        rejected = b.call(creator, "finish_observation")
        assert [entry["applicant"] for entry in rejected["pending_admissions"]] == [second.node]
        assert next(entry["decision"] for entry in rejected["admission_history"] if entry["applicant"] == first.node) == "Rejected"

        offline = self.start("native-online-offline", False, "join", manual_path)
        changing = self.start("native-online-code-change", False, "join", code_path)
        fresh_code = b.call(creator, "precode_invitation", int(time.time()) + 3600, 1)
        fresh_path = self.write_private("online-precode-fresh", fresh_code.encode(), False)
        assert b.call(creator, "close_page")
        before_code_change = changing.call("state_snapshot")
        with ThreadPoolExecutor(max_workers=2) as requests:
            failure = requests.submit(changing.call, "request", ok=False, path=fresh_path)
            offline.call("request", ok=False)
            failure.result()
        changed = changing.call("observe_state", after=before_code_change["cursor"])
        assert changed["status"]["admission_attempt"]["progress"] == "Created"
        assert changed["status"]["admission_attempt"]["request_id"] != before_code_change["status"]["admission_attempt"]["request_id"]
        assert offline.call("status")["admission_attempt"]["progress"] == "Created"
        # Reopen the same saved attempt independently of the caller's relay
        # cooldown for the now-finished offline exchange.
        offline = self.reopen(offline)
        assert b.call(creator, "reopen_page")
        b.call(creator, "start", "start", json.dumps(self.progress_configuration()), None)
        reopened = b.call(creator, "state_snapshot")
        assert reopened["cursor"] != rejected["cursor"]
        assert {key: value for key, value in reopened.items() if key != "cursor"} == {
            key: value for key, value in rejected.items() if key != "cursor"}
        waiting = b.send(creator, "observe_state", reopened["cursor"])
        assert offline.call("request")["stage"] == "AwaitingApproval"
        assert {entry["applicant"] for entry in b.wait(waiting)["pending_admissions"]} == {second.node, offline.node}

        member = "wasm-online-member"
        member_id = b.start(member, "join", invitation, self.progress_configuration())["id"]
        assert b.call(member, "request_join", False) == "AwaitingApproval"
        entry = next(item for item in b.call(creator, "state_snapshot")["pending_admissions"] if item["applicant"] == member_id)
        b.call(creator, "approve_admission", member_id, entry["request_id"])
        assert b.call(member, "request_join", False) == "Complete"
        accept = b.send(creator, "member_accept", 10000)
        b.call(member, "member_connect", b.call(creator, "id"))
        b.wait(accept)
        message = "MioCube online admission"
        reading = b.send(creator, "member_call", "serve_exchange", [])
        assert b.call(member, "member_call", "exchange", [message]) == b.wait(reading) == message
        for actor in [creator, member]:
            b.call(actor, "member_call", "close", [])
        remote = b.call(member, "propose_device_operation", "appoint-administrator", member_id)["operation"]["id"]
        creator_id = b.call(creator, "id")
        assert b.call(member, "request_device_confirmation", remote, creator_id)["stage"] == "awaiting_confirmation"
        assert any(item["operation"]["id"] == remote for item in b.call(creator, "device_operations")["operations"])
        assert b.call(creator, "confirm_device_operation", remote)["effective_locally"]
        assert b.call(member, "collect_device_result", remote, creator_id)["effective_locally"]

        draining = self.start("native-online-draining", False, "join", manual_path)
        assert b.call(creator, "hold_admission_commit")
        with ThreadPoolExecutor(max_workers=1) as peer:
            request = peer.submit(draining.call, "request", ok=False)
            assert b.call(creator, "wait_admission_commit")
            closing = b.send(creator, "close")
            try:
                request.result(timeout=15)
                # The dispatcher closed the exchange, but the write it started
                # is still active. Another owner must not obtain the lease yet.
                assert b.call(creator, "duplicate_start") == {"rejected": True}
            finally:
                b.call(creator, "release_admission_commit")
            b.wait(closing)
        b.call(creator, "reload")
        b.call(creator, "start", "start", json.dumps(self.progress_configuration()), None)
        durable = b.call(creator, "state_snapshot")
        assert any(item["applicant"] == draining.node for item in durable["pending_admissions"])

        provider = self.creator(False, "native-online-provider")
        invite = self.native_invitation(provider, "browser-online-invitation")
        code = self.native_invitation(provider, "browser-online-precode", precode=True)
        browser_first, browser_second, browser_granted = "wasm-online-first", "wasm-online-second", "wasm-online-granted"
        for name, received in [(browser_first, invite), (browser_second, invite), (browser_granted, code)]:
            b.start(name, "join", received, self.progress_configuration())
        initial = provider.call("state_snapshot")
        assert initial["pending_admissions"] == initial["admission_history"] == []
        requests = [b.send(name, "request_join", False) for name in (browser_first, browser_second, browser_granted)]
        assert [b.wait(request) for request in requests] == ["AwaitingApproval", "AwaitingApproval", "Complete"]
        ids = [b.call(name, "id") for name in (browser_first, browser_second, browser_granted)]
        op = provider.call("genesis_propose", action="appoint-administrator", target=ids[2])["operation"]["id"]
        assert provider.call("genesis_confirm", operation=op)["effective_locally"]
        observed = provider.call("observe_state", after=initial["cursor"])
        assert observed == provider.call("state_snapshot")
        assert {entry["applicant"] for entry in observed["pending_admissions"]} == set(ids[:2])
        assert observed["pending_admissions"] == provider.call("requests")["requests"]
        assert observed["admission_history"] == provider.call("admission_history")
        assert {ids[0]: "AwaitingApproval", ids[1]: "AwaitingApproval", ids[2]: "ReceiptReceived"} == {
            entry["applicant"]: entry["decision"] for entry in observed["admission_history"]}
        assert self.member_by_id(observed, ids[2])["role"] == "admin"
        assert observed["device_operations"] == provider.call("device_operations")

        # Observe one more admission over a running NativeClient. The incoming
        # exchange is the only operation that can advance the snapshot.
        browser_late = "wasm-online-late"
        late_id = b.start(browser_late, "join", invite, self.progress_configuration())["id"]
        with ThreadPoolExecutor(max_workers=1) as waiter:
            waiting = waiter.submit(provider.call, "observe_state", after=observed["cursor"])
            assert b.call(browser_late, "request_join", False) == "AwaitingApproval"
            arrived = waiting.result(timeout=15)
        assert {entry["applicant"] for entry in arrived["pending_admissions"]} == set(ids[:2] + [late_id])
        provider = self.reopen(provider)
        assert provider.call("observe_state", after=arrived["cursor"]) == provider.call("state_snapshot")
        self.record("observation", "online_admission_qualified", dict(nativeAndWasmProviders=True,
                    concurrentManualAndPrecode=True, lateAndWaitingObservers=True, roleAndOperationState=True,
                    singleChangeDelivered=True, pageClosedOffline=True, durableReopen=True, joinedStoreClose=True))

    def browser_admission_progress(self):
        provider = self.creator(False, "native-progress-provider")
        invitation = self.native_invitation(provider, "browser-progress-invitation")
        name = "browser-progress-applicant"
        b = self.browser
        initial = b.start(name, "join", invitation, self.progress_configuration())
        assert initial["admission_attempt"]["progress"] == "Created"
        assert provider.call("requests")["requests"] == []
        assert b.call(name, "request_join", False) == "AwaitingApproval"
        sent = b.reopen(name)["admission_attempt"]
        first, = provider.call("requests")["requests"]
        assert sent == dict(request_id=first["request_id"], progress="Delivered")
        assert first["applicant"] == initial["id"]
        assert sent["request_id"] == initial["admission_attempt"]["request_id"]
        assert b.call(name, "request_join", False) == "AwaitingApproval"
        assert provider.call("requests")["requests"] == [first]
        provider.stop()
        b.call(name, "request_join", False, ok=False, expected_error="timed out")
        b.call(name, "block_relay", True)
        assert b.reopen(name)["admission_attempt"] == sent
        assert b.call(name, "path_diagnostics")["relayHome"]["connectedCount"] == 0
        b.call(name, "reload")
        offline = b.call(name, "start", "start", json.dumps({"relays": []}), None)
        assert offline["admission_attempt"] == sent
        b.call(name, "block_relay", False)
        b.call(name, "reload")
        restarted = b.call(name, "start", "start", json.dumps(self.progress_configuration()), None)
        assert restarted["admission_attempt"] == sent
        provider = Host(self, "native-progress-provider-reopened", False, "open", provider.directory)
        assert provider.call("admission_history") == [dict(first, decision="AwaitingApproval", authorization=None)]
        provider.call("reject", **first)
        assert b.call(name, "request_join", False) == "Rejected"
        assert b.reopen(name)["admission_attempt"] == dict(sent, progress="Rejected")
        assert b.call(name, "request_join", False) == "Rejected"
        assert provider.call("requests")["requests"] == []

        b.call(name, "new_manual_attempt")
        retry = b.call(name, "status")
        assert retry["id"] == initial["id"] and retry["admission_attempt"]["progress"] == "Created"
        assert b.call(name, "request_join", False) == "AwaitingApproval"
        second, = provider.call("requests")["requests"]
        assert second["applicant"] == first["applicant"] and second["request_id"] != first["request_id"]
        assert second["request_id"] == retry["admission_attempt"]["request_id"]
        provider.call("approve", **second)
        assert b.call(name, "request_join", True) == "PendingMaterials"
        assert b.reopen(name)["admission_attempt"] == dict(request_id=second["request_id"], progress="Authorized")
        history = provider.call("admission_history")
        assert len(history) == 2
        authorization = history[0]["authorization"]
        assert authorization
        assert history == [dict(second, decision="ApprovedAwaitingReceipt", authorization=authorization),
                           dict(first, decision="Rejected", authorization=None)]
        assert provider.call("requests")["requests"] == []

        assert b.call(name, "request_join", False) == "Complete"
        assert b.reopen(name)["admission_attempt"] == dict(request_id=second["request_id"], progress="Complete")
        assert b.call(name, "request_admission", material(invitation)["invitation"]) == "Complete"
        assert b.call(name, "status")["secret_saved"] is True
        assert b.call(name, "complete_admission") == "Complete"
        history[0]["decision"] = "ReceiptReceived"
        provider = self.reopen(provider)
        assert provider.call("admission_history") == history
        operation = provider.call("genesis_propose", action="remove-member", target=second["applicant"])["operation"]["id"]
        assert provider.call("device_execute", operation=operation)["effective_locally"]
        b.call(name, "request_admission", material(invitation)["invitation"], ok=False)
        provider = self.reopen(provider)
        assert provider.call("admission_history") == history
        assert provider.call("requests")["requests"] == []
        b.call(name, "close")
        return provider

    def native_admission_progress(self):
        b = self.browser
        creator = "browser-progress-provider"
        b.start(creator, "create", configuration=self.progress_configuration())
        b.call(creator, "export_recovery_card")
        b.call(creator, "acknowledge_recovery_saved")
        invitation = b.call(creator, "invitation")
        path = self.write_private("native-progress-invitation", invitation.encode(), False)
        applicant = self.start("native-progress-applicant", False, "join", path)
        Path(path).unlink()
        created = applicant.call("status")["admission_attempt"]
        assert created["progress"] == "Created"
        assert b.call(creator, "admission_history") == []

        b.call(creator, "fail_next_save")
        serving = self.serve_progress_admission(creator, False)
        applicant.call("request", ok=False)
        b.wait(serving, ok=False)
        assert applicant.call("status")["admission_attempt"] == created
        b.reopen(creator)
        assert b.call(creator, "admission_history") == []
        serving = self.serve_progress_admission(creator, False)
        assert applicant.call("request")["stage"] == "AwaitingApproval"
        assert b.wait(serving) == "AwaitingApproval"
        sent = dict(created, progress="Delivered")
        applicant = self.reopen(applicant)
        assert applicant.call("status")["admission_attempt"] == sent
        first = dict(applicant=applicant.node, request_id=created["request_id"], authorization=None,
                     decision="AwaitingApproval", display_name=applicant.call("status")["display_name"])
        b.reopen(creator)
        assert b.call(creator, "admission_history") == [first]
        b.call(creator, "close_page")
        applicant.call("request", ok=False)
        applicant = self.reopen(applicant)
        assert applicant.call("status")["admission_attempt"] == sent
        b.call(creator, "reopen_page")
        b.call(creator, "start", "start", json.dumps(self.progress_configuration()), None)
        b.call(creator, "reject_admission", applicant.node, created["request_id"])
        serving = self.serve_progress_admission(creator, False)
        assert applicant.call("request")["stage"] == "Rejected"
        assert b.wait(serving) == "Rejected"
        applicant = self.reopen(applicant)
        assert applicant.call("status")["admission_attempt"] == dict(created, progress="Rejected")
        assert applicant.call("request")["stage"] == "Rejected"
        directory = Path(applicant.directory)
        mode = directory.stat().st_mode
        directory.chmod(0o500)
        try:
            if not os.access(directory, os.W_OK):
                applicant.call("new_manual_attempt", ok=False)
        finally:
            directory.chmod(mode)
        assert applicant.call("status")["admission_attempt"] == dict(created, progress="Rejected")
        applicant.call("new_manual_attempt")
        retry = applicant.call("status")["admission_attempt"]
        assert retry["request_id"] != created["request_id"] and retry["progress"] == "Created"

        serving = self.serve_progress_admission(creator, False)
        assert applicant.call("request")["stage"] == "AwaitingApproval"
        assert b.wait(serving) == "AwaitingApproval"
        second = dict(applicant=applicant.node, request_id=retry["request_id"], authorization=None,
                      decision="AwaitingApproval", display_name=applicant.call("status")["display_name"])
        first["decision"] = "Rejected"
        assert b.call(creator, "admission_history") == [second, first]
        second["authorization"] = b.call(creator, "approve_admission", applicant.node, retry["request_id"])
        second["decision"] = "ApprovedAwaitingReceipt"
        serving = self.serve_progress_admission(creator, False)
        assert applicant.call("request", authorization_only=True)["stage"] == "PendingMaterials"
        b.wait(serving, ok=False)
        applicant = self.reopen(applicant)
        assert applicant.call("status")["admission_attempt"] == dict(retry, progress="Authorized")
        assert b.call(creator, "admission_history") == [second, first]
        serving = self.serve_progress_admission(creator, False)
        assert applicant.call("request")["stage"] == "Complete"
        assert b.wait(serving) == "Complete"
        applicant = self.reopen(applicant)
        assert applicant.call("status")["admission_attempt"] == dict(retry, progress="Complete")
        b.reopen(creator)
        second["decision"] = "ReceiptReceived"
        assert b.call(creator, "admission_history") == [second, first]

        session = b.call(creator, "begin_management_session")
        b.call(creator, "manage", session, "remove-member", applicant.node)
        b.call(creator, "end_management_session", session)
        serving = self.serve_progress_admission(creator, False)
        applicant.call("request", ok=False)
        b.wait(serving, ok=False)
        b.reopen(creator)
        assert b.call(creator, "admission_history") == [second, first]
        applicant.stop()
        return creator

    def admission_progress(self):
        native = self.browser_admission_progress()
        browser = self.native_admission_progress()
        b = self.browser
        invitation = self.native_invitation(native, "browser-progress-precode", precode=True)
        name = "browser-progress-precode-applicant"
        b.start(name, "join", invitation, self.progress_configuration())
        assert b.call(name, "request_join", True) == "PendingMaterials"
        authorized = b.reopen(name)["admission_attempt"]
        assert authorized["progress"] == "Authorized"
        assert b.call(name, "request_join", False) == "Complete"
        assert b.call(name, "status")["admission_attempt"] == dict(authorized, progress="Complete")
        assert native.call("precode_status", path=self.path("browser-progress-precode", False))["uses"] == 1

        invitation = b.call(browser, "precode_invitation", int(time.time()) + 3600, 1)
        path = self.write_private("native-progress-precode", invitation.encode(), False)
        applicant = self.start("native-progress-precode-applicant", False, "join", path)
        Path(path).unlink()
        serving = self.serve_progress_admission(browser, True)
        assert applicant.call("request", authorization_only=True)["stage"] == "PendingMaterials"
        b.wait(serving, ok=False)
        applicant = self.reopen(applicant)
        authorized = applicant.call("status")["admission_attempt"]
        assert authorized["progress"] == "Authorized"
        serving = self.serve_progress_admission(browser, True)
        assert applicant.call("request")["stage"] == "Complete"
        assert b.wait(serving) == "Complete"
        assert applicant.call("status")["admission_attempt"] == dict(authorized, progress="Complete")
        assert b.call(browser, "precode_usage", material(invitation)["precode"]) == 1

        name = "browser-progress-raw-applicant"
        invitation = material(self.native_invitation(native, "browser-progress-raw-invitation"))["invitation"]
        b.start(name, "start", configuration=self.progress_configuration())
        assert b.call(name, "request_admission", invitation) == "AwaitingApproval"
        sent = b.reopen(name)["admission_attempt"]
        assert sent["progress"] == "Delivered"
        pending, = native.call("requests")["requests"]
        assert pending["request_id"] == sent["request_id"]
        native.call("approve", **pending)
        assert b.call(name, "request_join", True) == "PendingMaterials"
        assert b.reopen(name)["admission_attempt"] == dict(sent, progress="Authorized")
        assert b.call(name, "request_join", False) == "Complete"
        self.record("progress", "admission_progress_qualified", dict(
            nativeAndWasmBothRoles=True, durableDeliveryBeforeConfirmation=True,
            originalReadAfterReopen=True, explicitNewRequestAfterRejection=True,
            invalidatedGrantStopsDelivery=True, precodeUsedOnceOnEachProvider=True))

    def admission_receipts(self):
        b = self.browser
        browser_provider = "browser-receipt-provider"
        configuration = self.progress_configuration()
        b.start(browser_provider, "create", configuration=configuration)
        b.call(browser_provider, "export_recovery_card")
        b.call(browser_provider, "acknowledge_recovery_saved")

        def expect_interrupted(host):
            assert host.process.wait(timeout=10) == 86
            host.process.stdin.close()
            host.stderr.close()
            self.hosts.remove(host)
            self.record(host.label, "expected_interruption", {"exit": 86})

        def reopen_native(host):
            reopened = Host(self, host.label + "-after-interruption", False, "open", host.directory)
            assert reopened.node == host.node
            return reopened

        def reopen_browser(actor):
            assert b.call(actor, "reopen_page")
            return b.call(actor, "start", "start", json.dumps(configuration), None)

        def wait_checkpoint(host, message):
            deadline = time.monotonic() + 30
            log = self.output / (host.label + ".stderr")
            while message not in log.read_text():
                assert time.monotonic() < deadline and host.process.poll() is None, message
                time.sleep(0.1)
            self.record(host.label, "protocol_checkpoint", {"committed": True})

        def wait_browser_receipt(actor, request_id):
            deadline = time.monotonic() + 10
            while True:
                receipt = b.call(actor, "path_diagnostics")["admissionReceipt"]
                if receipt is not None:
                    assert receipt == dict(request_id=request_id, status="confirmed")
                    return
                assert time.monotonic() < deadline
                time.sleep(0.1)

        def native_saved_before_send(label):
            code = b.call(browser_provider, "precode_invitation", int(time.time()) + 3600, 1)
            path = self.write_private(label + "-invite.json", code.encode(), False)
            applicant = self.start(label, False, "join", path)
            assert applicant.call("request", authorization_only=True)["stage"] == "PendingMaterials"
            history = b.call(browser_provider, "admission_history")
            entry = next(item for item in history if item["applicant"] == applicant.node)
            assert entry["decision"] == "ApprovedAwaitingReceipt" and entry["authorization"]
            assert applicant.call("interrupt_admission", after="secret_saved")["armed"]
            try:
                applicant.call("request")
            except RuntimeError as interruption:
                assert "exited before response" in str(interruption)
            expect_interrupted(applicant)
            assert b.call(browser_provider, "admission_history") == history
            assert b.call(browser_provider, "precode_usage", material(code)["precode"]) == 1
            return applicant, entry, code

        native_applicant, awaiting, code = native_saved_before_send("native-receipt-applicant")
        browser_records = b.call(browser_provider, "admission_material_receipt")["records"]
        assert b.call(browser_provider, "close_page")
        native_applicant = reopen_native(native_applicant)
        joined = dict(request_id=awaiting["request_id"], progress="Complete")
        assert native_applicant.ready["status"]["stage"] == "Complete"
        assert native_applicant.ready["status"]["admission_attempt"] == joined
        assert native_applicant.call("interrupt_admission", after="observe_receipt_timeout")["armed"]
        wait_checkpoint(native_applicant, "admission receipt timed out")
        assert native_applicant.call("status")["admission_attempt"] == joined
        native_applicant.stop()
        reopen_browser(browser_provider)
        assert b.call(browser_provider, "admission_history") == [awaiting]
        assert b.call(browser_provider, "close_page")
        native_applicant = reopen_native(native_applicant)
        assert native_applicant.call("interrupt_admission", after="observe_receipt_settled")["armed"]
        reopen_browser(browser_provider)
        wait_checkpoint(native_applicant, "admission receipt settled")
        received = dict(awaiting, decision="ReceiptReceived")
        assert b.call(browser_provider, "admission_history") == [received]
        assert b.call(browser_provider, "admission_material_receipt")["records"] == browser_records
        assert b.call(browser_provider, "precode_usage", material(code)["precode"]) == 1
        assert native_applicant.call("status")["stage"] == "Complete"
        assert native_applicant.call("status")["admission_attempt"] == joined
        native_applicant.stop()
        assert b.call(browser_provider, "close_page")
        reopen_browser(browser_provider)
        assert b.call(browser_provider, "admission_history") == [received]

        native_provider = self.creator(False, "native-receipt-provider")
        browser_applicant = "browser-receipt-applicant"
        code_name = "browser-receipt-precode.json"
        native_code = self.native_invitation(native_provider, code_name, precode=True)
        saved = b.start(browser_applicant, "join", native_code, configuration)
        assert b.call(browser_applicant, "request_join", True) == "PendingMaterials"
        approved, = native_provider.call("admission_history")
        assert approved["applicant"] == saved["id"]
        assert approved["decision"] == "ApprovedAwaitingReceipt" and approved["authorization"]
        native_records = native_provider.call("status")["records"]
        assert native_provider.call("interrupt_admission", after="receipt_saved_before_reply")["armed"]
        b.call(browser_applicant, "request_join", False, ok=False)
        expect_interrupted(native_provider)
        joined = dict(request_id=approved["request_id"], progress="Complete")
        assert b.call(browser_applicant, "status")["stage"] == "Complete"
        assert b.call(browser_applicant, "status")["admission_attempt"] == joined
        assert b.call(browser_applicant, "close_page")
        native_provider = reopen_native(native_provider)
        received_native = dict(approved, decision="ReceiptReceived")
        assert native_provider.call("admission_history") == [received_native]
        assert native_provider.call("interrupt_admission", after="observe_receipt_saved")["armed"]
        reopened = reopen_browser(browser_applicant)
        assert reopened["stage"] == "Complete" and reopened["admission_attempt"] == joined
        wait_checkpoint(native_provider, "admission receipt committed")
        wait_browser_receipt(browser_applicant, approved["request_id"])
        assert native_provider.call("requests")["requests"] == []
        assert native_provider.call("status")["records"] == native_records
        assert native_provider.call("precode_status", path=self.path(code_name, False))["uses"] == 1
        native_provider = self.reopen(native_provider)
        assert native_provider.call("admission_history") == [received_native]
        assert b.call(browser_applicant, "status")["admission_attempt"] == joined

        # A repeated material save leaves the main snapshot unchanged. Hold its
        # real IndexedDB commit through the socket deadline, then verify the
        # same running browser schedules recovery after storage completes.
        assert b.reopen(browser_applicant)["admission_attempt"] == joined
        assert b.call(browser_applicant, "path_diagnostics")["admissionReceipt"] is None
        assert b.call(browser_applicant, "request_precode", material(native_code)["precode"]) == "Complete"
        assert b.call(browser_applicant, "hold_admission_commit")
        completion = b.send(browser_applicant, "complete_admission")
        try:
            assert b.call(browser_applicant, "wait_admission_commit")
            time.sleep(configuration["maintenance"]["contact_timeout_ms"] / 1000)
        finally:
            b.call(browser_applicant, "release_admission_commit")
        b.wait(completion, ok=False, expected_error="timed out")
        assert b.call(browser_applicant, "status")["admission_attempt"] == joined
        wait_browser_receipt(browser_applicant, approved["request_id"])
        assert native_provider.call("admission_history") == [received_native]

        stale, old, stale_code = native_saved_before_send("native-receipt-stale")
        session = b.call(browser_provider, "begin_management_session")
        b.call(browser_provider, "manage", session, "remove-member", stale.node)
        b.call(browser_provider, "end_management_session", session)
        assert b.call(browser_provider, "close_page")
        stale = reopen_native(stale)
        assert stale.call("interrupt_admission", after="observe_receipt_settled")["armed"]
        reopen_browser(browser_provider)
        wait_checkpoint(stale, "admission receipt settled")
        assert b.call(browser_provider, "admission_history") == [old, received]
        assert b.call(browser_provider, "precode_usage", material(stale_code)["precode"]) == 1
        assert b.call(browser_provider, "state_snapshot")["pending_admissions"] == []
        self.record("receipts", "admission_receipts_qualified", dict(
            nativeAndWasmBothRoles=True, nativeSavedBeforeSend=True,
            providerSavedBeforeFinalReply=True, timeoutPreservedLocalAdmission=True,
            reopenedProvidersRetainOriginalResult=True, invalidGrantStopsReceipt=True,
            noDuplicateGrantOrRedemption=True, inFlightBrowserSaveSurvivesDeadline=True))

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
        assert original["recovery_exported"] is False and original["recovery_saved"] is False
        assert b.call(name, "duplicate_start") == dict(rejected=True)
        b.call(name, "invitation", ok=False)
        b.call(name, "begin_management_session", ok=False)
        b.call(name, "acknowledge_recovery_saved", ok=False)
        b.call(name, "fail_next_save")
        b.call(name, "export_recovery_card", ok=False)
        b.call(name, "acknowledge_recovery_saved", ok=False)
        assert b.call(name, "status") == original
        assert b.reopen(name) == original
        b.call(name, "acknowledge_recovery_saved", ok=False)
        plain = b.call(name, "export_recovery_card")
        exported = dict(original, recovery_exported=True)
        assert b.reopen(name) == exported
        b.call(name, "invitation", ok=False)
        assert b.call(name, "export_recovery_card") == plain
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
        assert b.reopen(name) == exported
        b.call(name, "acknowledge_recovery_saved")
        b.call(name, "acknowledge_recovery_saved")
        saved = b.call(name, "status")
        assert saved["stage"] == "Complete" and saved["id"] == original["id"]
        assert saved["recovery_exported"] is True and saved["recovery_saved"] is True
        assert saved["network"] == original["network"] and saved["genesis"] == original["genesis"]
        assert b.reopen(name) == saved
        applicant = name + "-recovery-ordinary-applicant"
        pending = b.start(applicant, "join", b.call(name, "invitation"))
        assert pending["stage"] == "AwaitingApproval"
        assert pending["recovery_exported"] is False and pending["recovery_saved"] is True
        assert b.reopen(applicant) == pending
        b.call(applicant, "close")
        direct = name + "-recovery-direct"
        b.start(direct, "create")
        b.call(direct, "export_recovery_card")
        assert b.reopen(direct)["recovery_exported"] is True
        b.call(direct, "acknowledge_recovery_saved")
        assert b.call(direct, "status")["stage"] == "Complete"
        b.call(direct, "close")
        self.record(name, "browser_recovery_qualified", dict(actualIndexedDB=True, independentCreator=True,
                    optionalEncryption=True, failedSaveNotAcknowledged=True, originalCredentialRoundtrip=True,
                    wrongPasswordAndTamperRejected=True, explicitSavedGateDurable=True,
                    allStagesReopened=True, reexportSameCredential=True,
                    ordinaryApplicantUnaffected=True))
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
            if self.args.suite == "recovery":
                self.recovery(False, "native-creator").stop()
                self.browser_creator("browser-creator")
            elif self.args.suite == "progress":
                self.admission_progress()
            elif self.args.suite == "observation":
                self.online_admission_observation()
            elif self.args.suite == "receipts":
                self.admission_receipts()
            elif self.args.suite == "device-names":
                self.device_names()
            elif self.args.suite == "member-management":
                self.member_management()
            elif self.args.suite == "departure":
                self.departures()
            elif self.args.suite == "removal-rejoin":
                for browser_removed in (True, False):
                    self.removal_rejoin(browser_removed)
            elif self.args.suite == "precode-revocation":
                self.precode_revocation()
            else:
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
    parser.add_argument("--suite", choices=["all", "core", "pairing", "continuation", "recovery", "progress", "observation", "receipts", "device-names", "member-management", "departure", "removal-rejoin", "precode-revocation"], default="all")
    parser.add_argument("--relay", default=RELAY, help="explicit relay for suites that take a dedicated relay")
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    args = parser.parse_args()
    build = json.loads(args.build_receipt.read_text())
    assert build["exit"] == 0 and build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"]
    actual = {p.relative_to(args.package).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
              for p in args.package.rglob("*") if p.is_file()}
    assert actual == build["artifacts"]
    args.windows = args.windows_sha = None
    args.local_only, args.failure_only = True, False
    args.focus = "recovery" if args.suite == "recovery" else "all"
    runner = BrowserOnboardingRunner(args)
    runner.args = args
    runner.record("runner", "browser_package", dict(package=str(args.package),
                  packageSHA256=build["packageSHA256"], buildReceipt=str(args.build_receipt)))
    runner.run()


if __name__ == "__main__":
    main()
