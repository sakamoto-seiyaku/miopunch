#!/usr/bin/env python3
"""Offline failure-predicate regressions; never starts SDK, ADB, or network actors."""
from contextlib import ExitStack, redirect_stdout
import copy
import gzip
import hashlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import check_unified_native_android as runner

ROOT = Path(__file__).resolve().parents[1]
HISTORICAL = ROOT / "docs/evidence/unified-native-paths-2026-10-05/raw/public/android-first/receipt.json.gz"
ROLES = ("Pixel_6a", "TB322FC")


class TestAndroidNativeSuccessPredicates(unittest.TestCase):
    def setUp(self):
        self.facts = json.loads(gzip.decompress(HISTORICAL.read_bytes()))

    def execute_mock(self, facts):
        """Drive the real run() decision with immutable historical facts and fake owners."""
        class Device:
            def __init__(self, model, route):
                self.model = model
                self.commands = []

            def run(self, *args):
                if args[0] == "pull" and args[1].endswith("target.json"):
                    Path(args[2]).write_text("{}")

            def shell(self, *args):
                return "", ""

        class Actor:
            def __init__(self, device):
                self.device = device
                self.directory = "/owned-mock/" + device.model
                self.process = SimpleNamespace(poll=lambda: 0)
                self.closes = 0

            def deploy(self, *args, **kwargs):
                pass

            def start(self, *args, **kwargs):
                return self

            def send(self, value):
                pass

            def wait_event(self, event, timeout):
                role = self.device.model
                if callable(event):
                    return copy.deepcopy(facts["liveSocketSnapshots"][role])
                if event == "native-ready":
                    return copy.deepcopy(facts["serverReady"])
                key = {"native-result": "results", "native-cleanup": "sdkCleanup",
                       "native-material-reopened": "reopened"}[event]
                return copy.deepcopy(facts[key][role])

            def close(self, *, remove_directory, graceful_seconds):
                self.closes += 1
                if remove_directory:
                    return copy.deepcopy(facts["cleanup"][self.device.model])
                return copy.deepcopy(facts["phases"][self.device.model + "-member"]["cleanup"])

        def phase(actor, args, event, output, name):
            return copy.deepcopy(facts["phases"][name])

        def archive(actor, output, name):
            role = actor.device.model
            key = "terminalRuns" if name.endswith("-terminal") else "phases"
            return copy.deepcopy(facts[key][role if key == "terminalRuns" else role + "-member"])

        with tempfile.TemporaryDirectory(prefix="miopunch79-offline-predicates-") as directory:
            directory = Path(directory)
            binary = directory / "fake-binary-never-executed"
            binary.write_bytes(b"offline mock artifact, never executed")
            artifact = hashlib.sha256(binary.read_bytes()).hexdigest()
            build = directory / "build.json"
            build.write_text(json.dumps({"exit": 0, "qualifiedForFrozenRun": True,
                                         "deployment": {"unified_native_member": {"deployedSha256": artifact}}}))
            facility = directory / "facility.json"
            facility.write_text(json.dumps(facts["facility"]))
            args = SimpleNamespace(output=directory / "result", binary=binary,
                                   build_receipt=build, facility=facility)
            with ExitStack() as stack:
                stack.enter_context(patch.object(runner, "Device", Device))
                stack.enter_context(patch.object(runner, "ReceiptActor", Actor))
                stack.enter_context(patch.object(runner, "phase", phase))
                stack.enter_context(patch.object(runner, "archive_run", archive))
                stack.enter_context(patch.object(runner, "wifi_address", lambda device: facts["physicalBinds"][device.model]))
                stack.enter_context(patch.object(runner, "build_source_binding", lambda value: {"offlineMock": True}))
                stack.enter_context(patch.object(runner, "source_snapshot", lambda: {"offlineMock": "frozen"}))
                stack.enter_context(patch.object(runner, "collect_endpoint", side_effect=AssertionError("network collector forbidden in local predicates")))
                stack.enter_context(redirect_stdout(io.StringIO()))
                exit_code = runner.run(args)
            report = json.loads((args.output / "receipt.json").read_text())
            return exit_code, report

    def require_refused(self, facts):
        code, report = self.execute_mock(facts)
        self.assertEqual(code, 1)
        self.assertIs(report["passed"], False)

    def test_exact_historical_success_and_actual_zero_tcp_listener_contract(self):
        code, report = self.execute_mock(self.facts)
        self.assertEqual(code, 0)
        self.assertIs(report["passed"], True)
        for role in ROLES:
            self.assertEqual(report["sdkCleanup"][role]["socketReleasedImmediately"], [True])
            self.assertEqual(report["sdkCleanup"][role]["tcpListenerReleasedImmediately"], [])

    def test_nonzero_missing_signal_or_wrong_type_exit_is_not_graceful(self):
        for role in ROLES:
            for location in ("member-poll", "member-close", "terminal-poll", "final-close"):
                for value in (1, -9, None, False, 0.0, "0", "missing"):
                    with self.subTest(role=role, location=location, exit=value):
                        facts = copy.deepcopy(self.facts)
                        target = {"member-poll": facts["phases"][role + "-member"],
                                  "member-close": facts["phases"][role + "-member"]["cleanup"],
                                  "terminal-poll": facts["terminalRuns"][role],
                                  "final-close": facts["cleanup"][role]}[location]
                        if value == "missing": target.pop("commandExit")
                        else: target["commandExit"] = value
                        self.require_refused(facts)

    def test_reopened_identity_must_be_explicitly_true_on_each_actor(self):
        for role in ROLES:
            for value in (False, None, 1, "true", "missing"):
                with self.subTest(role=role, identity=value):
                    facts = copy.deepcopy(self.facts)
                    if value == "missing": facts["reopened"][role].pop("identityMatched")
                    else: facts["reopened"][role]["identityMatched"] = value
                    self.require_refused(facts)

    def test_release_arrays_must_be_present_boolean_and_exact_owned_count(self):
        for role in ROLES:
            for key in ("socketReleasedImmediately", "socketReleasedLate",
                        "tcpListenerReleasedImmediately", "tcpListenerReleasedLate"):
                invalid = (None, "true", [1], ["true"], [False], [True, True], "missing")
                if key.startswith("socket"): invalid += ([],)
                else: invalid += ([True],)
                for value in invalid:
                    with self.subTest(role=role, field=key, value=value):
                        facts = copy.deepcopy(self.facts)
                        if value == "missing": facts["sdkCleanup"][role].pop(key)
                        else: facts["sdkCleanup"][role][key] = value
                        self.require_refused(facts)

    def test_owned_udp_and_no_tcp_configuration_evidence_cannot_be_assumed(self):
        for role in ROLES:
            for mutation in ("no-snapshot", "unobserved", "unverified-udp", "unclassified-fd", "missing-config", "tcp-enabled"):
                with self.subTest(role=role, evidence=mutation):
                    facts = copy.deepcopy(self.facts)
                    snapshot = facts["liveSocketSnapshots"][role]
                    if mutation == "no-snapshot": snapshot.clear()
                    elif mutation == "unobserved": snapshot["sockets"]["observed"] = False
                    elif mutation == "unclassified-fd": snapshot["sockets"]["unclassifiedDescriptors"] = [{"fd": 42}]
                    elif mutation == "unverified-udp":
                        for socket in snapshot["sockets"]["entries"]:
                            if socket.get("endpointUdp") is True: socket["dupVerified"] = False
                    elif mutation == "missing-config": facts["phases"][role + "-configure"]["event"].pop("tcpPunchingEnabled")
                    elif mutation == "tcp-enabled": facts["phases"][role + "-configure"]["event"]["tcpPunchingEnabled"] = True
                    self.require_refused(facts)

    def test_additional_observed_owned_udp_requires_another_release_result(self):
        for role in ROLES:
            with self.subTest(role=role):
                facts = copy.deepcopy(self.facts)
                entries = facts["liveSocketSnapshots"][role]["sockets"]["entries"]
                primary = copy.deepcopy(next(value for value in entries if value.get("endpointUdp") is True))
                primary["fd"] += 100
                primary["inode"] = "socket:[offline-additional-owned-bind]"
                primary["localTuple"]["port"] += 1
                entries.append(primary)
                self.require_refused(facts)


if __name__ == "__main__":
    unittest.main()
