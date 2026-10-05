#!/usr/bin/env python3
"""Fake-driver route collector checks; no ADB, SDK, public service or STUN call."""
from contextlib import redirect_stderr
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import android_pair_route_receipt as collector


def endpoint(kind="chrome"):
    actor = {"kind": "chrome", "package": "com.android.chrome"} if kind == "chrome" else {
        "kind": "native", "ownedDirectory": "/data/local/tmp/miopunch-pair-" + "a" * 32}
    return {"model": "Pixel_6a" if kind == "chrome" else "TB322FC", "actor": actor,
            "selectedTuple": {"localAddress": "192.168.1.104", "localPort": 53189,
                              "peerAddress": "183.197.94.96", "peerPort": 26729}}


class FakeDevice:
    def __init__(self, *, package_uid=True, rules_exit=0):
        self.commands = []
        self.package_uid, self.rules_exit = package_uid, rules_exit

    def shell(self, *arguments, **_):
        code = 0
        if arguments[:5] == ("ip", "-o", "-4", "addr", "show"):
            text = "1: wlan0 inet 192.168.1.104/24 scope global wlan0\n"
        elif arguments == ("ip", "-4", "rule", "show"):
            text, code = "0: from all lookup local\n", self.rules_exit
        elif arguments[:5] == ("ip", "-4", "route", "show", "table"):
            text = "default via 192.168.1.1 dev wlan0 table 1047\n"
        elif arguments[:4] == ("cmd", "package", "list", "packages"):
            text, code = ("package:com.android.chrome uid:10167\n", 0) if self.package_uid else ("", 17)
        elif arguments[0] == "cat":
            text = "12345\n"
        elif arguments[0] == "sed":
            text = "Uid:\t2000\t2000\t2000\t2000\n"
        elif arguments[:4] == ("ip", "-4", "route", "get"):
            text = "183.197.94.96 from 192.168.1.104 via 192.168.1.1 dev wlan0 uid " + arguments[arguments.index("uid") + 1] + "\n"
        else:
            raise AssertionError("unexpected read-only operation")
        self.commands.append({"command": list(arguments), "exit": code,
                              "cleanup": {"forced": False, "processGroupAbsentImmediately": True}})
        return text, code


class RouteCollectorTests(unittest.TestCase):
    def test_chrome_uses_observed_exact_package_uid_without_inventing_socket_mark(self):
        request = endpoint(); collector.validate_request({"endpoints": [request]})
        device = FakeDevice(); receipt = collector.collect_endpoint(request, device)
        self.assertEqual(receipt["packageUid"], 10167)
        route = next(command["command"] for command in device.commands if command["command"][:4] == ["ip", "-4", "route", "get"])
        self.assertEqual(route, ["ip", "-4", "route", "get", "183.197.94.96", "from", "192.168.1.104", "uid", "10167"])
        self.assertEqual(receipt["socketMark"], {"observed": False, "value": None})
        self.assertFalse(receipt["actualSocketUid"]["observed"])
        self.assertFalse(receipt["physicalWanQualified"])
        self.assertEqual(receipt["errors"], [])

    def test_native_mark_zero_requires_matching_actual_fixture_success_and_owned_pid(self):
        request = endpoint("native")
        evidence = {"operation": "getsockopt(SO_MARK)", "success": True, "value": 0,
                    "pid": 12345, "localAddress": "192.168.1.104", "localPort": 53189}
        request["socketEvidence"] = evidence
        collector.validate_request({"endpoints": [request]})
        device = FakeDevice(); receipt = collector.collect_endpoint(request, device)
        self.assertEqual(receipt["ownedProcessUid"]["effective"], 2000)
        self.assertTrue(receipt["socketMark"]["observed"])
        self.assertEqual(receipt["socketMark"]["value"], 0)
        route = device.commands[-1]["command"]
        self.assertEqual(route[-4:], ["uid", "2000", "mark", "0x0"])
        request["socketEvidence"]["pid"] = 12346
        mismatched = collector.collect_endpoint(request, FakeDevice())
        self.assertFalse(mismatched["socketMark"]["observed"])
        self.assertIsNone(mismatched["socketMark"]["value"])
        self.assertTrue(mismatched["errors"])
        self.assertNotIn("mark", mismatched["commands"][-1]["command"])

    def test_missing_uid_skips_lookup_and_retains_first_real_exit_and_partial_reads(self):
        receipt = collector.collect_endpoint(endpoint(), FakeDevice(package_uid=False, rules_exit=42))
        self.assertTrue(receipt["routeLookupSkipped"])
        self.assertEqual(receipt["errors"][0]["exit"], 42)
        self.assertEqual(receipt["observations"][1]["exit"], 42)
        self.assertEqual(receipt["observations"][3]["exit"], 17)
        self.assertFalse(any(command["command"][:4] == ["ip", "-4", "route", "get"] for command in receipt["commands"]))
        self.assertIn("wlan0", receipt["observations"][2]["stdout"])

    def test_rejects_unowned_reads_and_overwrite_before_device_access(self):
        for replacement in [{"kind": "chrome", "package": "another.package"},
                            {"kind": "native", "ownedDirectory": "/data/user/0/another.package"}]:
            request = endpoint(); request["actor"] = replacement
            with self.assertRaises(ValueError): collector.validate_request({"endpoints": [request]})
        request = endpoint("native"); request["socketEvidence"] = {"operation": "getsockopt(SO_MARK)",
            "success": False, "value": 0, "pid": 12345, "localAddress": "192.168.1.104", "localPort": 53189}
        with self.assertRaises(ValueError): collector.validate_request({"endpoints": [request]})
        with tempfile.TemporaryDirectory(prefix="miopunch-route-collector-local-") as temporary:
            output = Path(temporary) / "existing.json"; output.write_text("original receipt")
            with patch.object(collector, "Device") as device, patch.object(sys, "argv", ["collector",
                    "--request", str(Path(temporary) / "missing.json"), "--output", str(output)]), redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as rejected: collector.main()
                self.assertEqual(rejected.exception.code, 2)
                device.assert_not_called()
            self.assertEqual(output.read_text(), "original receipt")


if __name__ == "__main__":
    unittest.main()
