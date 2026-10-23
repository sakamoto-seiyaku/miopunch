#!/usr/bin/env python3
"""Real public onboarding operations over explicit immutable client pins.

SSH deploys/controls Windows and transfers only private invitation/backup test
materials. Member PING/PONG runs in the actual SDK, never through SSH. This
driver never compiles; receipts contain only secret-free responses and outcomes.
"""
import argparse
import base64
import json
from pathlib import Path
import time

from check_native_client import Host, Runner, quote


class OnboardingRunner(Runner):
    def __init__(self, args):
        super().__init__(args)
        self.private = set()
        self.focus = args.focus

    def private_path(self, name, remote):
        self.private.add(name)
        return self.path(name, remote)

    def write_private(self, name, value, remote):
        path = self.output / name
        path.write_bytes(value)
        path.chmod(0o600)
        self.private.add(name)
        return self.transfer(name, False, remote)

    def read_private(self, name, remote):
        self.transfer(name, remote, False)
        return (self.output / name).read_bytes()

    def recovery(self, remote, prefix):
        creator = self.start(prefix, remote)
        original = creator.call("status")
        assert original["stage"] == "AwaitingRecoverySave"
        creator.call("invite", ok=False, path=self.private_path(prefix + "-invite.json", remote))
        creator.call("acknowledge_recovery_saved", ok=False)
        creator.call("export_recovery_card", ok=False, path=self.remote_root if remote else str(self.output))
        creator.call("acknowledge_recovery_saved", ok=False)
        assert creator.call("status") == original
        plain_name, encrypted_name = prefix + "-card.json", prefix + "-encrypted-card.json"
        plain = self.private_path(plain_name, remote)
        encrypted = self.private_path(encrypted_name, remote)
        password = self.write_private(prefix + "-password.json", b"M113 private test passphrase", remote)
        wrong = self.write_private(prefix + "-wrong-password.json", b"different test passphrase", remote)
        creator.call("export_recovery_card", path=plain)
        creator.call("export_recovery_card", path=encrypted, password_path=password)
        expected = {key: original[key] for key in ["network", "genesis"]}
        assert creator.call("inspect_recovery_card", path=plain) == expected
        assert creator.call("inspect_recovery_card", path=encrypted, password_path=password) == expected
        creator.call("inspect_recovery_card", ok=False, path=encrypted, password_path=wrong)
        changed = bytearray(self.read_private(encrypted_name, remote))
        changed[70] = ord("A") if changed[70] != ord("A") else ord("B")
        tampered = self.write_private(prefix + "-tampered-card.json", changed, remote)
        creator.call("inspect_recovery_card", ok=False, path=tampered, password_path=password)
        # External credential format: 32-byte root seed plus the original Create
        # pin. Never publish these bytes or the private backup's digest.
        raw = self.read_private(plain_name, remote).decode()
        assert raw.startswith("miopunch-recovery:1:")
        credential = base64.urlsafe_b64decode(raw.split(":1:", 1)[1] + "==")
        assert len(credential) == 64 and credential[32:].hex() == original["genesis"]
        assert creator.call("status") == original
        creator = self.reopen(creator)
        creator.call("invite", ok=False, path=self.private_path(prefix + "-invite.json", remote))
        creator.call("acknowledge_recovery_saved")
        creator.call("acknowledge_recovery_saved")
        saved = creator.call("status")
        assert saved["stage"] == "Complete" and saved["recovery_saved"]
        assert all(saved[key] == original[key] for key in ["node", "network", "genesis", "records"])
        creator = self.reopen(creator)
        assert creator.call("status") == saved
        self.record(prefix, "recovery_qualified", dict(plain=True, optional_encryption=True,
                    wrong_password_rejected=True, ciphertext_change_rejected=True,
                    canonical_create_roundtrip=True, explicit_save_durable=True))
        return creator

    def creator(self, remote, prefix):
        creator = self.start(prefix, remote)
        creator.call("export_recovery_card", path=self.private_path(prefix + "-card.json", remote))
        creator.call("acknowledge_recovery_saved")
        return creator

    def rejection(self, creator, join_remote, prefix):
        name = prefix + "-invite.json"
        creator.call("invite", path=self.private_path(name, creator.remote), text=True)
        assert self.read_private(name, creator.remote).startswith(b"miopunch://invite/")
        invitation = self.transfer(name, creator.remote, join_remote)
        applicant = self.start(prefix + "-applicant", join_remote, "join", invitation)
        assert applicant.node != creator.node
        assert applicant.call("request", path=invitation)["stage"] == "AwaitingApproval"
        first = creator.call("requests")["requests"]
        assert len(first) == 1 and first[0]["applicant"] == applicant.node
        assert applicant.call("request", path=invitation)["stage"] == "AwaitingApproval"
        assert creator.call("requests")["requests"] == first
        before = creator.call("status")["records"]
        creator.call("reject", **first[0])
        creator.call("reject", **first[0])
        creator.call("approve", ok=False, **first[0])
        assert creator.call("requests")["requests"] == []
        for _ in range(2):
            assert applicant.call("request", path=invitation) == dict(stage="Rejected", automatic_retry=False)
        applicant = self.reopen(applicant)
        assert applicant.ready["status"]["stage"] == "Rejected"
        assert applicant.call("request", path=invitation)["stage"] == "Rejected"
        assert creator.call("status")["records"] == before
        assert creator.call("requests")["requests"] == []
        applicant.call("new_manual_attempt", path=invitation)
        assert applicant.call("request", path=invitation)["stage"] == "AwaitingApproval"
        second = creator.call("requests")["requests"]
        assert len(second) == 1 and second[0]["applicant"] == first[0]["applicant"]
        assert second[0]["request_id"] != first[0]["request_id"]
        creator.call("approve", **second[0])
        approved_count = creator.call("status")["records"]
        creator.call("approve", **second[0])
        assert creator.call("status")["records"] == approved_count == before + 1
        assert applicant.call("request", path=invitation, authorization_only=True)["stage"] == "PendingMaterials"
        applicant = self.reopen(applicant)
        assert applicant.call("request", path=invitation)["stage"] == "Complete"
        assert applicant.call("request", path=invitation)["stage"] == "Complete"
        assert creator.call("status")["records"] == approved_count
        self.exchange(creator, applicant)
        applicant = self.reopen(applicant)
        self.exchange(creator, applicant)
        applicant.stop()
        self.record(prefix, "rejection_qualified", dict(retry_stopped=True, rejected_attempt_durable=True,
                    explicit_fresh_attempt=True, approval_idempotent=True, saved_before_complete=True,
                    offline_membership_retained=True))

    def precode(self, creator, join_remote, prefix):
        name = prefix + "-precode.json"
        creator.call("issue_precode", path=self.private_path(name, creator.remote), seconds=3600, max_uses=1)
        invitation = self.transfer(name, creator.remote, join_remote)
        applicant = self.start(prefix + "-applicant", join_remote, "join", invitation)
        assert applicant.call("request", path=invitation, authorization_only=True)["stage"] == "PendingMaterials"
        assert creator.call("precode_status", path=self.path(name, creator.remote))["uses"] == 1
        count = creator.call("status")["records"]
        applicant = self.reopen(applicant)
        assert applicant.call("request", path=invitation)["stage"] == "Complete"
        assert applicant.call("request", path=invitation)["stage"] == "Complete"
        assert creator.call("precode_status", path=self.path(name, creator.remote))["uses"] == 1
        assert creator.call("status")["records"] == count
        assert creator.call("requests")["requests"] == []
        self.exchange(creator, applicant)
        self.reopen(applicant).stop()

    def pairing(self, creator, join_remote, prefix):
        receiver = self.start(prefix + "-receiver", join_remote, "pair")
        wrong_receiver = self.start(prefix + "-wrong-receiver", join_remote, "pair")
        pairing_name, wrong_name = prefix + "-pairing.json", prefix + "-wrong-pairing.json"
        receiver.call("pairing_information", path=self.private_path(pairing_name, join_remote))
        wrong_receiver.call("pairing_information", path=self.private_path(wrong_name, join_remote))
        pairing = self.transfer(pairing_name, join_remote, creator.remote)
        invitation_name = prefix + "-invite.json"
        creator.call("invite", path=self.private_path(invitation_name, creator.remote))
        invitation = self.path(invitation_name, creator.remote)
        wrong_anchor = json.loads(self.read_private(invitation_name, creator.remote))
        value = wrong_anchor["invitation"]
        wrong_anchor["invitation"] = value[:64] + ("a" if value[64] != "a" else "b") + value[65:]
        wrong_anchor_path = self.write_private(prefix + "-wrong-anchor.json", json.dumps(wrong_anchor).encode(), creator.remote)
        creator.call("deliver_invitation", ok=False, pairing_path=pairing, invitation_path=wrong_anchor_path)
        assert receiver.call("pairing_status") == dict(invitation_saved=False)
        receiver.call("finish_pairing", ok=False, path=self.private_path(prefix + "-premature-invite.json", join_remote))
        impostor = self.creator(creator.remote, prefix + "-impostor")
        impostor.call("deliver_invitation", ok=False, pairing_path=pairing, invitation_path=invitation)
        assert receiver.call("pairing_status") == dict(invitation_saved=False)
        # Preserve the genuine private capability but substitute a different
        # real receiver's authenticated identity and routes. That receiver must
        # reject; merely knowing where to dial is not delivery authorization.
        original = json.loads(self.read_private(pairing_name, join_remote))
        substituted = dict(original)
        substituted["address"] = json.loads(self.read_private(wrong_name, join_remote))["address"]
        changed_pairing = self.write_private(prefix + "-substituted-pairing.json", json.dumps(substituted).encode(), creator.remote)
        creator.call("deliver_invitation", ok=False, pairing_path=changed_pairing, invitation_path=invitation)
        assert wrong_receiver.call("pairing_status") == dict(invitation_saved=False)
        assert receiver.call("pairing_status") == dict(invitation_saved=False)
        if receiver.remote:
            self.remote("New-Item -ItemType Directory -Path " + quote(receiver.directory) + " | Out-Null")
        else:
            Path(receiver.directory).mkdir()
        creator.call("deliver_invitation", ok=False, pairing_path=pairing, invitation_path=invitation)
        assert receiver.call("pairing_status") == dict(invitation_saved=False)
        if receiver.remote:
            self.remote("Remove-Item -LiteralPath " + quote(receiver.directory))
        else:
            Path(receiver.directory).rmdir()
        creator.call("deliver_invitation", pairing_path=pairing, invitation_path=invitation)
        assert receiver.call("pairing_status") == dict(invitation_saved=True)
        creator.call("deliver_invitation", pairing_path=pairing, invitation_path=invitation)
        changed = json.loads(self.read_private(invitation_name, creator.remote))
        changed["version"] = 1 if changed["version"] == 2 else 2
        changed_invitation = self.write_private(prefix + "-substituted-invite.json", json.dumps(changed).encode(), creator.remote)
        creator.call("deliver_invitation", ok=False, pairing_path=pairing, invitation_path=changed_invitation)
        creator.call("deliver_invitation", pairing_path=pairing, invitation_path=invitation)
        received_name = prefix + "-received-invite.json"
        received_path = self.private_path(received_name, join_remote)
        result = receiver.call("finish_pairing", path=received_path)
        assert result["status"]["node"] == receiver.node
        assert result["status"]["stage"] == "AwaitingApproval"
        receiver.address = result["address"]
        receiver.ready = dict(event="ready", **result)
        assert receiver.call("request", path=received_path)["stage"] == "AwaitingApproval"
        pending = creator.call("requests")["requests"]
        assert len(pending) == 1 and pending[0]["applicant"] == receiver.node
        creator.call("approve", **pending[0])
        assert receiver.call("request", path=received_path)["stage"] == "Complete"
        self.exchange(creator, receiver)
        self.reopen(receiver).stop()
        wrong_receiver.stop()
        impostor.stop()
        self.record(prefix, "pairing_qualified", dict(wrong_sender_rejected=True,
                    wrong_network_invitation_rejected_before_delivery=True,
                    substituted_peer_and_routes_rejected=True, substituted_material_rejected=True,
                    failed_publication_not_acknowledged=True,
                    same_delivery_retry=True, received_identity_preserved=True,
                    ordinary_admission_still_required=True))

    def resource_policy(self, creator, join_remote, prefix):
        relay = "https://aps1-1.relay.n0.iroh.link./"
        base = dict(defaults=dict(revision=1, resources=[dict(url=relay,
                    uses=["Observation", "DataRelay"], limitation="owned contact-policy fixture")]),
                    replacement=None, disabled=[], disabled_uses=[], lan_discovery="Disabled")
        sender_config = dict(base, disabled_uses=["DataRelay", "Signaling"])
        mqtt = dict(logical_id="m113-disabled-mqtt", native_url="mqtt://mqtt.example.invalid:1883/", wss_url="wss://mqtt.example.invalid/mqtt")
        irc = dict(logical_id="m113-disabled-irc", native_url="irc://irc.example.invalid:6667/", wss_url="wss://irc.example.invalid/", channel="#m113-disabled")
        sender_config["defaults"] = dict(revision=1, resources=[*base["defaults"]["resources"],
            *[dict(url=entry["native_url"], uses=["Signaling", "Lookup"], limitation="disabled mapped fixture") for entry in [mqtt, irc]],
            dict(url="irc://raw.example.invalid:6667/", uses=["Signaling", "Lookup"], limitation="disabled native-only fixture")])
        sender_config["mqtt_facilities"] = [mqtt]
        sender_config["irc_facilities"] = [irc]
        sender_path = self.write_private(prefix + "-sender-resources.json", json.dumps(sender_config).encode(), creator.remote)
        sender = Host(self, prefix + "-sender", creator.remote, "create",
                      self.path(prefix + "-sender-state", creator.remote), configuration=sender_path)
        sender.call("export_recovery_card", path=self.private_path(prefix + "-sender-card.json", creator.remote))
        sender.call("acknowledge_recovery_saved")
        name = prefix + "-disabled-contact.json"
        sender.call("invite", path=self.private_path(name, creator.remote))
        contact = json.loads(self.read_private(name, creator.remote))
        assert contact["bootstrap"] == dict(relays=[], mqtt=[], irc=[], native_signaling=[])
        assert not any("Relay" in route for route in contact["address"]["addrs"])
        sender.stop()

        # Exercise exact, DNS-absolute and implicit-default-port spellings for
        # both mapped transports. A disabled mapping must reserve all of them.
        for protocol, facility, dotted, implicit in [
            ("mqtt", mqtt, "mqtt://mqtt.example.invalid.:1883/", "mqtt://mqtt.example.invalid/"),
            ("irc", irc, "irc://irc.example.invalid.:6667/", "irc://irc.example.invalid/"),
        ]:
            for spelling, selected in [("exact", facility["native_url"]), ("absolute", dotted), ("default-port", implicit)]:
                for mode, disabled_alias in [("wss", facility["wss_url"]), ("logical", facility["logical_id"])]:
                    label = prefix + "-" + protocol + "-" + spelling + "-" + mode
                    config = dict(base, defaults=dict(revision=1, resources=[*base["defaults"]["resources"],
                        dict(url=selected, uses=["Signaling", "Lookup"], limitation="mapped fixture")]),
                        disabled=[[disabled_alias, "Signaling"]], **{protocol + "_facilities": [facility]})
                    cfg = self.write_private(label + "-resources.json", json.dumps(config).encode(), creator.remote)
                    mapped = Host(self, label, creator.remote, "create", self.path(label + "-state", creator.remote), configuration=cfg)
                    mapped.call("export_recovery_card", path=self.private_path(label + "-card.json", creator.remote))
                    mapped.call("acknowledge_recovery_saved")
                    invitation_name = label + "-invite.json"
                    mapped.call("invite", path=self.private_path(invitation_name, creator.remote))
                    offered = json.loads(self.read_private(invitation_name, creator.remote))
                    assert offered["bootstrap"][protocol] == []
                    assert offered["bootstrap"]["native_signaling"] == []
                    mapped.stop()

            for mapped_allowed in [True, False]:
                label = prefix + "-" + protocol + ("-allowed-mapping" if mapped_allowed else "-allowed-raw")
                selected = dotted if mapped_allowed else facility["native_url"]
                config = dict(base, defaults=dict(revision=1, resources=[*base["defaults"]["resources"],
                    dict(url=selected, uses=["Signaling", "Lookup"], limitation="permitted fixture")]))
                if mapped_allowed:
                    config[protocol + "_facilities"] = [facility]
                cfg = self.write_private(label + "-resources.json", json.dumps(config).encode(), creator.remote)
                allowed = Host(self, label, creator.remote, "create", self.path(label + "-state", creator.remote), configuration=cfg)
                allowed.call("export_recovery_card", path=self.private_path(label + "-card.json", creator.remote))
                allowed.call("acknowledge_recovery_saved")
                name = label + "-invite.json"
                allowed.call("invite", path=self.private_path(name, creator.remote))
                offered = json.loads(self.read_private(name, creator.remote))
                if mapped_allowed:
                    assert len(offered["bootstrap"][protocol]) == 1
                    assert offered["bootstrap"]["native_signaling"] == []
                else:
                    assert offered["bootstrap"][protocol] == []
                    assert selected in offered["bootstrap"]["native_signaling"]
                allowed.stop()

            for selected, disabled_native in [(facility["native_url"], implicit), (implicit, facility["native_url"])]:
                label = prefix + "-" + protocol + "-native-default-denial-" + str(selected == implicit)
                config = dict(base, defaults=dict(revision=1, resources=[*base["defaults"]["resources"],
                    dict(url=selected, uses=["Signaling", "Lookup"], limitation="disabled native origin fixture")]),
                    disabled=[[disabled_native, "Signaling"]])
                cfg = self.write_private(label + "-resources.json", json.dumps(config).encode(), creator.remote)
                denied = Host(self, label, creator.remote, "create", self.path(label + "-state", creator.remote), configuration=cfg)
                denied.call("export_recovery_card", path=self.private_path(label + "-card.json", creator.remote))
                denied.call("acknowledge_recovery_saved")
                name = label + "-invite.json"
                denied.call("invite", path=self.private_path(name, creator.remote))
                assert json.loads(self.read_private(name, creator.remote))["bootstrap"]["native_signaling"] == []
                denied.stop()

        # A complete invitation cannot override an explicit empty replacement,
        # a disabled use/origin, or the user's disabled direct transport.
        name = prefix + "-offered-invite.json"
        creator.call("invite", path=self.private_path(name, creator.remote))
        offered = json.loads(self.read_private(name, creator.remote))
        invitation = self.transfer(name, creator.remote, join_remote)
        for mode in ["replacement", "disabled-use", "disabled-origin"]:
            denied = dict(base, udp_direct_disabled=True, tcp_direct_disabled=True)
            if mode == "replacement":
                denied["replacement"] = []
            elif mode == "disabled-use":
                denied["disabled_uses"] = ["DataRelay", "Signaling"]
            else:
                denied["disabled"] = [[url, "DataRelay"] for url in offered["bootstrap"]["relays"]]
                denied["disabled_uses"] = ["Signaling"]
            configuration = self.write_private(prefix + "-" + mode + ".json", json.dumps(denied).encode(), join_remote)
            applicant = Host(self, prefix + "-" + mode, join_remote, "join",
                self.path(prefix + "-" + mode + "-state", join_remote), invitation, configuration)
            applicant.call("request", ok=False, path=invitation)
            assert applicant.call("status")["stage"] == "AwaitingApproval"
            applicant = self.reopen(applicant)
            applicant.call("request", ok=False, path=invitation)
            assert applicant.call("status")["records"] == 0
            applicant.stop()
            assert creator.call("requests")["requests"] == []
        self.imported_resource_policy(creator, join_remote, prefix + "-import")
        self.record(prefix, "contact_policy_qualified", dict(senderDisabledUsesNotOffered=True,
                    disabledWssAndLogicalMappingsNotExportedRaw=True,
                    explicitEmptyReplacementRetained=True, disabledUsesRetained=True,
                    disabledOriginRetained=True, fallbackDidNotReenable=True, durableReopen=True))

    def imported_resource_policy(self, creator, join_remote, prefix):
        name = prefix + "-source.json"
        creator.call("invite", path=self.private_path(name, creator.remote))
        source = json.loads(self.read_private(name, creator.remote))
        for protocol, facility, implicit in [
            ("mqtt", dict(logical_id="m113-import-mqtt", native_url="mqtt://mqtt.example.invalid:1883/",
                          wss_url="wss://mqtt.example.invalid/mqtt"), "mqtt://mqtt.example.invalid/"),
            ("irc", dict(logical_id="m113-import-irc", native_url="irc://irc.example.invalid:6667/",
                         wss_url="wss://irc.example.invalid/", channel="#m113-import"), "irc://irc.example.invalid/"),
        ]:
            for mode in ["wss", "logical", "no-extra-permission", "permitted"]:
                label = prefix + "-" + protocol + "-" + mode
                offered = json.loads(json.dumps(source))
                offered["bootstrap"][protocol] = [facility]
                invitation = self.write_private(label + "-invite.json", json.dumps(offered).encode(), join_remote)
                selected = facility["native_url"] if mode == "permitted" else implicit
                resources = [dict(url=selected, uses=["Signaling", "Lookup"], limitation="explicit import allow-list"),
                             *[dict(url=url, uses=["DataRelay"], limitation="explicit bootstrap relay")
                               for url in offered["bootstrap"]["relays"]]]
                disabled = [[facility["wss_url"] if mode == "wss" else facility["logical_id"], "Signaling"]] if mode in ["wss", "logical"] else []
                config = dict(defaults=dict(revision=1, resources=[]), replacement=resources,
                              disabled=disabled, disabled_uses=[], lan_discovery="Disabled")
                cfg = self.write_private(label + "-resources.json", json.dumps(config).encode(), join_remote)
                applicant = Host(self, label, join_remote, "join", self.path(label + "-state", join_remote), invitation, cfg)
                state_path = applicant.directory + "/resources.json"
                saved = json.loads(self.remote("Get-Content -Raw -LiteralPath " + quote(state_path))) if join_remote else json.loads(Path(state_path).read_text())
                assert saved[protocol + "_facilities"] == [facility]
                assert [dict(entry, uses=sorted(entry["uses"])) for entry in saved["replacement"]] == [
                    dict(entry, uses=sorted(entry["uses"])) for entry in resources]
                assert saved["disabled"] == disabled
                assert applicant.call("status")["stage"] == "AwaitingApproval"
                assert applicant.call("request", path=invitation)["stage"] == "AwaitingApproval"
                pending = creator.call("requests")["requests"]
                assert len(pending) == 1 and pending[0]["applicant"] == applicant.node
                creator.call("approve", **pending[0])
                assert applicant.call("request", path=invitation)["stage"] == "Complete"
                self.exchange(creator, applicant)
                applicant = self.reopen(applicant)
                state = json.loads(self.remote("Get-Content -Raw -LiteralPath " + quote(state_path))) if join_remote else json.loads(Path(state_path).read_text())
                assert state == saved
                applicant.stop()
                if mode != "permitted":
                    for suffix in ["", "-reopen"]:
                        assert protocol.upper() + "_RESOURCE_UNAVAILABLE" not in (self.output / (label + suffix + ".stderr")).read_text()
                self.record(label, "import_policy_qualified", dict(mappingMetadataRetained=True,
                            explicitReplacementAndDenialRetained=True, noSignalingPermissionGrantedByMetadata=mode != "permitted",
                            independentApprovalRequired=True, durableReopen=True))

    def cleanup(self):
        results = super().cleanup()
        for name in self.private:
            try:
                (self.output / name).unlink(missing_ok=True)
            except Exception as error:
                results.append(dict(host="host", action="remove_private_export", error=type(error).__name__))
        return results

    def run(self):
        passed = False
        try:
            self.deploy()
            if self.focus == "all":
                self.invalid_invitation(False)
                if not self.local_only:
                    self.invalid_invitation(True)
            for creator_remote in ([False] if self.local_only else [False, True]):
                prefix = "windows-creates" if creator_remote else "linux-creates"
                creator = self.recovery(creator_remote, prefix) if self.focus in ["all", "recovery"] else self.creator(creator_remote, prefix)
                join_remote = not creator_remote if not self.local_only else False
                if self.focus in ["all", "admission"]:
                    self.rejection(creator, join_remote, prefix + "-manual")
                    self.precode(creator, join_remote, prefix + "-precode")
                if self.focus in ["all", "pairing"]:
                    self.pairing(creator, join_remote, prefix + "-pair")
                if self.focus in ["all", "resources"]:
                    self.resource_policy(creator, join_remote, prefix + "-resources")
                creator.stop()
            passed = True
        finally:
            cleanup = self.cleanup()
            passed = passed and all("error" not in item for item in cleanup)
            (self.output / "result.json").write_text(json.dumps(dict(passed=passed, focus=self.focus,
                cleanup=cleanup, private_exports_removed=not any((self.output / name).exists() for name in self.private)), indent=2))
        assert passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--windows", type=Path)
    parser.add_argument("--windows-sha")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--local-only", action="store_true")
    parser.add_argument("--focus", choices=["all", "recovery", "admission", "pairing", "resources"], default="all")
    args = parser.parse_args()
    args.failure_only = False
    if not args.local_only and not (args.windows and args.windows_sha):
        parser.error("Windows pin/hash required for actual two-device qualification")
    OnboardingRunner(args).run()


if __name__ == "__main__":
    main()
