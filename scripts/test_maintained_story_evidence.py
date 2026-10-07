"""Offline receipt-boundary checks; these never qualify a live Node story."""
import copy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_maintained_browser_native as evidence
import check_unified_maintained as active_evidence


def ready():
    return {
        "maintenanceOwnedByNode": True,
        "maintenanceStatus": {"contact_listener": {
            "phase": "Ready", "generation": 1, "ready_routes": 1,
            "starts": 1, "withdrawals": 0}},
        "listenerTraffic": {"driver_active": 1,
                            "protocol": {"connack": 0, "suback_codes": [0]}},
        "relayOnline": True, "normalPublicTrust": True,
    }


def closed():
    return {
        "forced": False, "commandExit": 0, "ownedRemoteProcessAbsent": True,
        "ownedRemoteProcessAbsentLate": True, "localReadersEnded": True,
        "ownedDirectoryRemoved": True,
        "sdkRetirement": {
            "activeRtcAssociations": 0, "activeRtcPumps": 0,
            "socketsBefore": 0, "socketsLate": 0,
            "maintenanceOwnedByNode": True,
            "maintenanceStatus": {"state": "Stopped", "contact_listener": {
                "phase": "Stopped", "ready_routes": 0}},
            "listenerTraffic": {"driver_active": 0, "queued_packets": 0},
        },
    }


class ReceiptBoundary(unittest.TestCase):
    def test_independent_endpoint_cannot_substitute_for_maintained_node(self):
        value = ready()
        value.pop("maintenanceOwnedByNode")
        with self.assertRaises(RuntimeError):
            evidence.require_maintained_ready(value)

    def test_actual_node_listener_and_relay_receipt_is_required(self):
        evidence.require_maintained_ready(ready())
        for key in ("maintenanceStatus", "listenerTraffic", "relayOnline", "normalPublicTrust"):
            value = ready()
            value.pop(key)
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                evidence.require_maintained_ready(value)

    def test_boolean_cannot_stand_in_for_actual_driver_count(self):
        value = ready()
        value["listenerTraffic"]["driver_active"] = True
        with self.assertRaises(RuntimeError):
            evidence.require_maintained_ready(value)

    def test_ready_signaling_is_not_an_ordinary_rtc_path(self):
        with self.assertRaises(RuntimeError):
            evidence.require_selected_rtc({"paths": [{"selected": True, "isSignal": True}]})
        evidence.require_selected_rtc({"paths": [{"selected": True, "isRtc": True,
                                                   "isSignal": False, "isRelay": False}]})

    def test_missing_driver_cleanup_count_is_not_zero(self):
        self.assertTrue(evidence.native_cleanup_succeeded(closed()))
        for key in ("driver_active", "queued_packets"):
            value = closed()
            value["sdkRetirement"]["listenerTraffic"].pop(key)
            self.assertFalse(evidence.native_cleanup_succeeded(value))
        value = closed()
        value["sdkRetirement"]["listenerTraffic"]["queued_packets"] = 1
        self.assertFalse(evidence.native_cleanup_succeeded(value))

    def test_forced_or_late_live_process_cleanup_is_rejected(self):
        for key, changed in (("forced", True), ("ownedRemoteProcessAbsentLate", False)):
            value = closed()
            value[key] = changed
            self.assertFalse(evidence.native_cleanup_succeeded(value))

    def test_active_maintenance_source_delta_cannot_be_called_inactive(self):
        binding = {"inactiveCurrentSourceDeltas": {"src/network_maintenance.rs": {}}}
        with patch.object(evidence, "validate_build_sources", return_value=copy.deepcopy(binding)):
            with self.assertRaises(RuntimeError):
                evidence.validate_maintained_build_sources(Path("unused-receipt"))


def active_commit_dto():
    # Offline shape fixture only, reflecting the actual Browser active adapter
    # and common coordinator DTO. It is not an SDK/runtime success receipt.
    return {
        "bootstrap": {"switched": True, "finalized": True, "freshAck": True,
                      "finalReleaseFreshAck": True, "stableUnchanged": True,
                      "bindingUnchanged": True},
        "coordinator": {"event": "active-improvement-committed", "version": 1,
                        "freshAck": True, "bothReady": True, "stableId": 17},
    }


class ActiveCommitBoundary(unittest.TestCase):
    def test_actual_active_dto_needs_no_bootstrap_both_confirmed(self):
        value = active_commit_dto()
        self.assertNotIn("bothConfirmed", value["bootstrap"])
        active_evidence.require_active_commit_proof(value, 17)

    def test_prepared_or_unproved_adapter_cannot_qualify(self):
        for field in ("switched", "finalized", "freshAck", "finalReleaseFreshAck",
                      "stableUnchanged", "bindingUnchanged"):
            for changed in (False, None):
                value = active_commit_dto()
                value["bootstrap"][field] = changed
                with self.subTest(field=field, changed=changed), self.assertRaises(RuntimeError):
                    active_evidence.require_active_commit_proof(value, 17)

    def test_common_bilateral_commit_is_required_for_the_original_m(self):
        for field, changed in (("event", "unified-path-ready"), ("freshAck", False),
                               ("bothReady", False), ("stableId", 18)):
            value = active_commit_dto()
            value["coordinator"][field] = changed
            with self.subTest(field=field), self.assertRaises(RuntimeError):
                active_evidence.require_active_commit_proof(value, 17)
        value = active_commit_dto()
        value.pop("coordinator")
        value["bootstrap"]["bothConfirmed"] = True
        with self.assertRaises(RuntimeError):
            active_evidence.require_active_commit_proof(value, 17)

    def test_absent_original_id_cannot_match_absent_commit_id(self):
        value = active_commit_dto()
        value["coordinator"].pop("stableId")
        with self.assertRaises(RuntimeError):
            active_evidence.require_active_commit_proof(value, None)


if __name__ == "__main__":
    unittest.main()
