#!/usr/bin/env python3
"""Offline checks for the current M82 route qualification predicate."""

import importlib.util
import copy
import hashlib
import ipaddress
import struct
import tempfile
import android_pair_packet_witness as packet_witness
import host_pixel_route_receipt_current as host_routes
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "android_pair_route_receipt_current", ROOT / "scripts/android_pair_route_receipt_current.py"
)
collector = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(collector)
CONTROLLER_SPEC = importlib.util.spec_from_file_location(
    "check_unified_admission_current", ROOT / "scripts/check_unified_admission_current.py"
)
controller = importlib.util.module_from_spec(CONTROLLER_SPEC)
assert CONTROLLER_SPEC.loader is not None
CONTROLLER_SPEC.loader.exec_module(controller)


def endpoint(role: str, *, native: bool, mark: bool) -> dict:
    return {
        "role": role,
        "qualification": {
            "activeNotVpnPhysicalNetwork": True,
            "selectedPairObserved": True,
            "selectedPairNonRelay": True,
            "selectedRouteToActualInterface": True if native else False,
            "nativeSocketTupleOwned": True if native else None,
            "nativeMarkObservedAndCorrelated": mark if native else None,
            "browserSocketUidUnknown": not native,
            "browserProtectUnknown": not native,
        },
        "network": {
            "activeValidatedNetwork": True,
            "activeNotVpnPhysicalNetwork": True,
        },
        "routeLookup": {"performed": True},
        "actualSocketUid": {"observed": False},
        "socketMark": {"observed": mark if native else False},
    }


class CurrentRouteQualificationTests(unittest.TestCase):
    def test_android_route_output_with_explicit_from_is_correlated(self):
        selected = {"localAddress": "192.0.2.7", "localPort": 40000,
                    "peerAddress": "198.51.100.9", "peerPort": 41000}
        facts = collector._parse_route("198.51.100.9 from 192.0.2.7 dev wlan1 table 1048 uid 10180\n cache", selected)
        self.assertTrue(facts["sourceMatches"])
        self.assertTrue(facts["destinationMatches"])

    def test_owned_fd_parser_accepts_real_ls_long_listing_and_keeps_duplicates(self):
        listing = "\n".join([
            "total 0",
            "lrwx------ 1 js js 64 Oct 8 01:48 7 -> socket:[12345]",
            "lrwx------ 1 js js 64 Oct 8 01:48 8 -> socket:[12345]",
            "lrwx------ 1 root root 64 2026-10-08 01:48 9 -> socket:[67890]",
            "lr-x------ 1 js js 64 Oct 8 01:48 10 -> /tmp/socket:[11111]",
        ])
        self.assertEqual(collector._fd_inodes(listing), {12345: [7, 8], 67890: [9]})

    def test_host_route_requires_exact_source_destination_and_default_interface(self):
        selected = {"localAddress": "192.0.2.7", "localPort": 40000,
                    "peerAddress": "198.51.100.9", "peerPort": 41000}
        route = {"dst": selected["peerAddress"], "from": selected["localAddress"], "dev": "eth2"}
        self.assertTrue(host_routes.route_matches([route], selected, {"eth2"}))
        for field, value in (("dst", "198.51.100.8"), ("from", "192.0.2.8"),
                             ("dev", "eth1"), ("type", "blackhole")):
            self.assertFalse(host_routes.route_matches([dict(route, **{field: value})], selected, {"eth2"}))
        self.assertFalse(host_routes.route_matches([dict(route, dev="wg0")], selected, {"wg0"}))
        self.assertFalse(host_routes.route_matches([route, route], selected, {"eth2"}))

    def test_host_observer_requires_packet_witness_before_reading_processes(self):
        with self.assertRaises(ValueError):
            host_routes.collect({"endpoints": []}, None, None)

    def test_nat_pair_correlation_requires_matching_ice_payloads_both_ways(self):
        native_tuple = {"localAddress": "192.0.2.7", "localPort": 40000,
                        "peerAddress": "198.51.100.9", "peerPort": 41000}
        browser_tuple = {"localAddress": "192.0.2.9", "localPort": 41000,
                         "peerAddress": "203.0.113.7", "peerPort": 40000}
        def member(selected):
            return {"sameOriginal": True, "alive": True, "memberGate": True,
                    "selectedRtc": True, "localTuple": selected}
        request = {"endpoints": [
            {"role": "native", "model": collector.NATIVE_MODEL, "transport": "usb",
             "actor": {"kind": "native", "ownedDirectory": "/data/local/tmp/miopunch-pair-" + "a" * 32},
             "selectedTuple": native_tuple, "currentM": member(native_tuple),
             "selectedPair": {"local": {"address": "192.0.2.7", "port": 40000, "typ": "host"},
                              "remote": {"address": "198.51.100.9", "port": 41000, "typ": "srflx"}}},
            {"role": "browser", "model": collector.BROWSER_MODEL, "transport": "usb",
             "actor": {"kind": "chrome", "package": "com.android.chrome"},
             "selectedTuple": browser_tuple, "currentM": member(browser_tuple),
             "selectedPair": {
                 "local": {"address": "198.51.100.9", "port": 41000, "typ": "srflx",
                           "relatedAddress": "192.0.2.9", "relatedPort": 41000},
                 "remote": {"address": "203.0.113.7", "port": 40000, "typ": "prflx"}},
             "pcStats": {"source": "browser-owned-selected-pc", "selected": True,
                         "sameOriginal": True, "alive": True, "memberGate": True,
                         "localTuple": browser_tuple, "peerTuple": browser_tuple,
                         "pingCount": 1, "pongCount": 1}},
        ]}
        self.assertFalse(collector.validate_request(request)["pairCorrelation"])
        def ice_request(tag):
            def attribute(kind, data):
                return struct.pack("!HH", kind, len(data)) + data + b"\0" * (-len(data) % 4)
            attributes = attribute(6, b"one:two") + attribute(8, b"\0" * 20) + attribute(0x8028, b"\0" * 4)
            return struct.pack("!HHI", 1, len(attributes), 0x2112A442) + tag * 12 + attributes
        def packet(source, source_port, destination, destination_port, payload):
            udp = struct.pack("!HHHH", source_port, destination_port, len(payload) + 8, 0) + payload
            ip = struct.pack("!BBHHHBBH4s4s", 0x45, 0, len(udp) + 20, 0, 0, 64, 17, 0,
                             ipaddress.ip_address(source).packed, ipaddress.ip_address(destination).packed)
            return b"\0" * 12 + b"\x08\x00" + ip + udp
        def pcap(packets):
            header = struct.pack("<IHHIIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1)
            return header + b"".join(struct.pack("<IIII", 1, i, len(v), len(v)) + v
                                      for i, v in enumerate(packets))
        n, b = ice_request(b"n"), ice_request(b"b")
        native_packets = [packet("192.0.2.7", 40000, "198.51.100.9", 41000, n),
                          packet("198.51.100.9", 41000, "192.0.2.7", 40000, b)]
        browser_packets = [packet("203.0.113.7", 40000, "192.0.2.9", 41000, n),
                           packet("192.0.2.9", 41000, "203.0.113.7", 40000, b)]
        with tempfile.TemporaryDirectory() as folder:
            paths = {}
            for role, packets in (("native", native_packets), ("browser", browser_packets)):
                path = Path(folder) / (role + ".pcap")
                path.write_bytes(pcap(packets))
                paths[role] = {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            request["packetWitness"] = paths
            self.assertEqual(packet_witness.derive_missing_peer_address(
                paths, native_tuple, browser_tuple["localAddress"],
                browser_tuple["localPort"], browser_tuple["peerPort"]), "203.0.113.7")
            path = Path(paths["browser"]["path"])
            path.write_bytes(pcap(browser_packets + [
                packet("203.0.113.8", 40000, "192.0.2.9", 41000, n)]))
            paths["browser"]["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
            self.assertIsNone(packet_witness.derive_missing_peer_address(
                paths, native_tuple, browser_tuple["localAddress"],
                browser_tuple["localPort"], browser_tuple["peerPort"]))
            path.write_bytes(pcap(browser_packets))
            paths["browser"]["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
            self.assertTrue(collector.validate_request(request)["pairCorrelation"])
            path = Path(paths["browser"]["path"])
            browser_packets[0] = packet("203.0.113.7", 40000, "192.0.2.9", 41000, ice_request(b"x"))
            path.write_bytes(pcap(browser_packets))
            with self.assertRaises(ValueError):
                collector.validate_request(request)
            paths["browser"]["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
            self.assertFalse(collector.validate_request(request)["pairCorrelation"])

    def test_chrome_stats_pair_requires_candidate_and_transport_id_links(self):
        stats = {
            "selectedPair": {"id": "pair", "state": "succeeded", "nominated": True,
                             "localCandidateId": "local", "remoteCandidateId": "remote"},
            "transport": {"selectedCandidatePairId": "pair"},
            "localCandidate": {"id": "local", "address": "203.0.113.7", "port": 40000,
                               "candidateType": "srflx", "protocol": "udp",
                               "relatedAddress": "192.0.2.7", "relatedPort": 41000},
            "remoteCandidate": {"id": "remote", "address": "", "ip": "198.51.100.9", "port": 42000,
                                "candidateType": "prflx", "protocol": "udp"},
        }
        pair = controller._selected_pair({"bootstrap": {"rtc": {"stats": stats}}})
        self.assertIsNotNone(pair)
        self.assertEqual(pair["local"]["baseAddress"], "192.0.2.7")
        self.assertEqual(pair["remote"]["candidateType"], "prflx")
        self.assertEqual(pair["remote"]["address"], "198.51.100.9")
        for field, key in (("transport", "selectedCandidatePairId"),
                           ("selectedPair", "remoteCandidateId")):
            bad = copy.deepcopy(stats)
            bad[field][key] = "unrelated"
            self.assertIsNone(controller._selected_pair({"stats": bad}))

    def test_live_member_ids_remain_local_to_each_endpoint(self):
        native = {"stableId": 9001, "memberAuthorized": True,
                  "newProtocol": True, "remoteId": "browser-node"}
        browser = {"memberAuthorized": True, "newProtocol": True,
                   "expectedPeerMatched": True, "ordinaryHandle": True,
                   "bootstrap": {"memberStableId": 42},
                   "coordinator": {"stableId": 42}}
        self.assertEqual(controller._live_member_local_ids(native, browser, "browser-node"),
                         (9001, 42))

        wrong_peer = dict(native, remoteId="another-browser")
        with self.assertRaises(controller.CurrentGuardError):
            controller._live_member_local_ids(wrong_peer, browser, "browser-node")
        denied = dict(browser, expectedPeerMatched=False)
        with self.assertRaises(controller.CurrentGuardError):
            controller._live_member_local_ids(native, denied, "browser-node")
        mismatched = copy.deepcopy(browser)
        mismatched["coordinator"]["stableId"] = 43
        with self.assertRaises(controller.CurrentGuardError):
            controller._live_member_local_ids(native, mismatched, "browser-node")

    def test_route_summary_keeps_missing_observation_unqualified(self):
        self.assertEqual(controller._route_summary(None), {})
        self.assertEqual(controller._route_summary({"summary": None}), {})
        self.assertEqual(controller._route_summary({"summary": {"physicalDataQualified": True}}),
                         {"physicalDataQualified": True})

    def test_m82_prelude_disables_m83_secret_abort_fixture(self):
        prelude = controller._m82_prelude()
        self.assertIn('})({"abortSecretOnce":false});', prelude)
        self.assertIn('})("observe");', prelude)
        self.assertIn("mio82-rtc-progress", prelude)
        self.assertIn("createOffer", prelude)
        self.assertIn("setRemoteDescription", prelude)
        self.assertNotIn("localDescription.sdp", prelude)

    def test_report_finalization_preserves_first_failure_and_cleanup(self):
        class Runtime:
            @staticmethod
            def browser_cleanup_succeeded(value):
                return value.get("ok") is True

        report = {"passed": False, "normalTlsVerified": True,
                  "routeObservation": None,
                  "firstFailure": {"phase": "sdk:complete_admission",
                                   "failureClass": "SdkReplyFailure",
                                   "failure": "material commit failed"},
                  "cleanup": {"nativeQualified": False,
                              "native": {"failureClass": "CloseError"},
                              "browser": {"ok": False}}}
        controller._finalize_m82_report(report, Runtime())
        self.assertEqual(report["firstFailure"]["phase"], "sdk:complete_admission")
        self.assertEqual(report["cleanup"]["native"]["failureClass"], "CloseError")
        self.assertFalse(report["passed"])
        self.assertFalse(report["physicalDataQualified"])

    def test_report_finalization_accepts_qualified_route_summary(self):
        class Runtime:
            @staticmethod
            def browser_cleanup_succeeded(value):
                return value.get("ok") is True

        report = {"passed": True, "normalTlsVerified": True,
                  "routeObservation": {"summary": {"physicalDataQualified": True}},
                  "cleanup": {"nativeQualified": True, "browser": {"ok": True}}}
        controller._finalize_m82_report(report, Runtime())
        self.assertTrue(report["passed"])
        self.assertTrue(report["physicalDataQualified"])

    def test_source_entry_uses_repository_root_key(self):
        digest = "a" * 64
        name, value = controller._source_entry(
            {"Cargo.toml": digest, "vendor/iroh/Cargo.toml": "b" * 64}, "Cargo.toml")
        self.assertEqual(name, "Cargo.toml")
        self.assertEqual(value, digest)

    def test_source_entry_rejects_nested_only_and_root_aliases(self):
        with self.assertRaises(controller.CurrentGuardError):
            controller._source_entry({"vendor/iroh/Cargo.toml": "a" * 64}, "Cargo.toml")
        with self.assertRaises(controller.CurrentGuardError):
            controller._source_entry({
                "Cargo.toml": "a" * 64,
                str(ROOT / "Cargo.toml"): "b" * 64,
            }, "Cargo.toml")

    def test_android_connectivity_parser_uses_selected_records(self):
        connectivity = """Active default network: 640

Current Networks:
  NetworkAgentInfo{network{640} ni{WIFI CONNECTED} LinkProperties{{InterfaceName: wlan1}} nc{{[ Transports: WIFI Capabilities: NOT_VPN&VALIDATED&INTERNET ]}}
  NetworkAgentInfo{network{641} ni{VPN CONNECTED} LinkProperties{{InterfaceName: tun0}} nc{{[ Transports: VPN Capabilities: INTERNET ]}}

Network Requests:
"""
        facts = collector._agent_facts(connectivity)
        self.assertTrue(facts["activeDefaultObserved"])
        self.assertTrue(facts["defaultAgentObserved"])
        self.assertTrue(facts["defaultTransportWifi"])
        self.assertTrue(facts["defaultValidated"])
        self.assertTrue(facts["defaultNotVpnCapability"])
        self.assertEqual(facts["currentVpnAgentCount"], 1)

    def test_v3_observer_requires_live_pair_and_exact_base_tuple(self):
        reply = {
            "currentReplyABI": "member-socket-mark/v3",
            "originalM": {"stableId": 7, "sameOriginal": True, "alive": True,
                           "memberGate": True, "selectedRtc": True},
            "selectedPair": {
                "local": {"address": "203.0.113.7", "port": 40000,
                           "candidateType": "srflx", "protocol": "udp",
                           "relatedAddress": "192.0.2.7", "relatedPort": 40000},
                "remote": {"address": "198.51.100.9", "port": 41000,
                            "candidateType": "prflx", "protocol": "udp"},
            },
            "socketTuple": {"localAddress": "192.0.2.7", "localPort": 40000,
                            "peerAddress": "198.51.100.9", "peerPort": 41000},
            "observation": {"observed": True, "matchingFdCount": 1,
                            "duplicatesClosed": True,
                            "socketEvidence": {"operation": "getsockopt(SO_MARK)",
                                "success": True, "value": 0, "pid": 7,
                                "localAddress": "192.0.2.7", "localPort": 40000,
                                "inode": 77}},
            "after": {"stableId": 7, "sameOriginal": True, "alive": True,
                       "memberGate": True, "selectedRtc": True,
                       "bindingUnchanged": True},
        }
        pair, selected, observation, gap = controller._native_live_observation(reply, 7)
        self.assertIsNone(gap)
        self.assertEqual(selected["localAddress"], "192.0.2.7")
        self.assertTrue(pair["nonRelay"])
        self.assertTrue(observation["socketEvidence"]["success"])

        bad = dict(reply)
        bad["socketTuple"] = dict(reply["socketTuple"], localPort=40001)
        self.assertEqual(controller._native_live_observation(bad, 7)[3],
                         "native-selected-pair-tuple-mismatch")

    def test_v3_observer_requires_live_state(self):
        reply = {"currentReplyABI": "member-socket-mark/v3",
                 "tupleCorrelation": {"observed": True}}
        self.assertEqual(controller._native_live_observation(reply, 7)[3],
                         "native-live-observer-state-mismatch")

    def test_explicit_evidence_qualifies_without_browser_socket_uid(self):
        report = {
            "receipt": "/tmp/private.json",
            "commandExit": 0,
            "pairCorrelation": True,
            "endpoints": [endpoint("native", native=True, mark=True),
                          endpoint("browser", native=False, mark=False)],
        }
        summary = collector._public_summary(report)
        self.assertTrue(summary["physicalDataQualified"])
        self.assertTrue(summary["nativeMarkObservedAndCorrelated"])
        self.assertEqual(summary["actualSocketUidObservedCount"], 0)

    def test_missing_native_mark_stays_unqualified_but_browser_unknown_is_allowed(self):
        report = {
            "receipt": "/tmp/private.json",
            "commandExit": 0,
            "pairCorrelation": True,
            "endpoints": [endpoint("native", native=True, mark=False),
                          endpoint("browser", native=False, mark=False)],
        }
        self.assertFalse(collector._public_summary(report)["physicalDataQualified"])

    def test_route_parser_requires_selected_destination_and_source(self):
        selected = {"localAddress": "192.0.2.4", "localPort": 40000,
                    "peerAddress": "198.51.100.7", "peerPort": 40001}
        parsed = collector._parse_route(
            "198.51.100.7 via 192.0.2.1 dev wlan0 src 192.0.2.4 uid 2000\n", selected
        )
        self.assertTrue(parsed["destinationMatches"])
        self.assertTrue(parsed["sourceMatches"])
        self.assertEqual(parsed["interface"], "wlan0")
        self.assertFalse(collector._parse_route(
            "198.51.100.8 via 192.0.2.1 dev wlan0 src 192.0.2.4 uid 2000\n", selected
        )["destinationMatches"])


if __name__ == "__main__":
    unittest.main()
