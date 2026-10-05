"""Public receipt privacy and qualification scope, without any device access."""
import unittest
from copy import deepcopy
from pathlib import Path
import tempfile

from check_browser_relay_capability import (
    digest, native_artifact_receipt, native_cleanup_succeeded, native_failure_diagnostics, public, sources_match,
    browser_relay_observations, native_relay_observations, relay_observation,
)


class PublicEvidence(unittest.TestCase):
    def test_skipped_runtime_is_explicit_and_private_material_never_survives(self):
        value = {"rtcCapability": {"supported": None, "permitted": False},
                 "members": [{"identityMatched": {"browser": True, "native": True}}],
                 "id": "00" * 32, "binding": {"opaque": [1, 2, 3]}, "secret": [42]}
        self.assertEqual(public(value), {
            "rtcCapability": {"supported": None, "permitted": False},
            "members": [{"identityMatched": {"browser": True, "native": True}}],
        })

    def test_external_build_input_matches_and_mutation_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            tool = Path(directory) / "compiler"
            tool.write_bytes(b"compiler version one")
            frozen = {"external:" + str(tool): digest(tool)}
            self.assertTrue(sources_match(frozen))
            tool.write_bytes(b"compiler version two")
            self.assertFalse(sources_match(frozen))

    def test_stripped_deployment_requires_link_to_original_compiled_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            compiled = Path(directory) / "compiled"
            deployed = Path(directory) / "deploy"
            compiled.write_bytes(b"debug symbols + executable")
            deployed.write_bytes(b"executable")
            deployment = {"compiledPath": str(compiled), "compiledSha256": digest(compiled),
                          "deployedPath": str(deployed), "deployedSha256": digest(deployed),
                          "transformation": {"exit": 0}}
            build = {"artifacts": {str(compiled): digest(compiled)},
                     "deployment": {"fixture": deployment}}
            self.assertEqual(native_artifact_receipt(build, deployed), deployment)
            build["artifacts"][str(compiled)] = "unrelated build"
            with self.assertRaises(RuntimeError):
                native_artifact_receipt(build, deployed)

    def test_deployed_binary_mutation_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "fixture"
            binary.write_bytes(b"original")
            build = {"artifacts": {str(binary): digest(binary)}}
            binary.write_bytes(b"changed")
            with self.assertRaises(RuntimeError):
                native_artifact_receipt(build, binary)

    def test_private_diagnostic_aliases_and_opaque_contact_routes_are_removed(self):
        self.assertEqual(public({"private_key": [1, 2], "sdp": "private", "claimed": "peer",
                                 "paths": ["CustomAddr(opaque-contact-address)"],
                                 "bindingHash": "private", "expectedPeerMatched": True}),
                         {"expectedPeerMatched": True})

    def test_process_exit_does_not_hide_a_late_sdk_socket(self):
        cleanup = {"forced": False, "commandExit": 0, "ownedRemoteProcessAbsent": True,
                   "ownedRemoteProcessAbsentLate": True, "localReadersEnded": True,
                   "ownedDirectoryRemoved": True,
                   "sdkRetirement": {"activeRtcAssociations": 0, "activeRtcPumps": 0,
                                     "socketsBefore": 0, "socketsImmediate": 1, "socketsLate": 1}}
        self.assertFalse(native_cleanup_succeeded(cleanup))
        cleanup["sdkRetirement"]["socketsLate"] = 0
        self.assertTrue(native_cleanup_succeeded(cleanup))

    def test_false_exit_or_retirement_counts_never_qualify_owned_cleanup(self):
        valid = {"forced": False, "commandExit": 0, "ownedRemoteProcessAbsent": True,
                 "ownedRemoteProcessAbsentLate": True, "localReadersEnded": True,
                 "ownedDirectoryRemoved": True,
                 "sdkRetirement": {"activeRtcAssociations": 0, "activeRtcPumps": 0,
                                   "socketsBefore": 0, "socketsLate": 0}}
        for field in ("commandExit", "activeRtcAssociations", "activeRtcPumps",
                      "socketsBefore", "socketsLate"):
            with self.subTest(field=field):
                observed = deepcopy(valid)
                target = observed if field == "commandExit" else observed["sdkRetirement"]
                target[field] = False
                self.assertFalse(native_cleanup_succeeded(observed))

    def test_retirement_counts_must_be_observed_and_nonnegative(self):
        valid = {"forced": False, "commandExit": 0, "ownedRemoteProcessAbsent": True,
                 "ownedRemoteProcessAbsentLate": True, "localReadersEnded": True,
                 "ownedDirectoryRemoved": True,
                 "sdkRetirement": {"activeRtcAssociations": 0, "activeRtcPumps": 0,
                                   "socketsBefore": 0, "socketsLate": 0}}
        for field in ("activeRtcAssociations", "activeRtcPumps", "socketsBefore", "socketsLate"):
            for value in (-1, None, "0"):
                with self.subTest(field=field, value=value):
                    observed = deepcopy(valid)
                    observed["sdkRetirement"][field] = value
                    self.assertFalse(native_cleanup_succeeded(observed))
            with self.subTest(field=field, missing=True):
                observed = deepcopy(valid)
                del observed["sdkRetirement"][field]
                self.assertFalse(native_cleanup_succeeded(observed))

    def test_real_numeric_zero_qualifies_but_original_process_failure_exits_do_not(self):
        observed = {"forced": False, "commandExit": 0, "ownedRemoteProcessAbsent": True,
                    "ownedRemoteProcessAbsentLate": True, "localReadersEnded": True,
                    "ownedDirectoryRemoved": True,
                    "sdkRetirement": {"activeRtcAssociations": 0, "activeRtcPumps": 0,
                                      "socketsBefore": 0, "socketsLate": 0}}
        self.assertTrue(native_cleanup_succeeded(observed))
        for exit_code in (-15, 1, None):
            with self.subTest(exit_code=exit_code):
                observed["commandExit"] = exit_code
                self.assertFalse(native_cleanup_succeeded(observed))

    def test_native_fatal_errors_preserve_cause_but_not_identifiers_or_private_payloads(self):
        self.assertEqual(native_failure_diagnostics([
            'Error: InvalidInput peer=' + 'ab' * 32,
            'Error: secret=private fixture material',
            'a=candidate:raw private SDP', 'unrelated arbitrary log',
        ]), ['Error: InvalidInput peer=<identifier>', '<private diagnostic omitted>'])

    def test_relay_dto_retains_actual_zero_but_never_defaults_unreached_fields(self):
        result = relay_observation({'tag': 'DEBUG-mio76-relay', 'phase': 'begin',
            'localCandidateCount': 0, 'memberOpen': True, 'rawPeerId': 'private'})
        self.assertEqual(result, {'tag': 'DEBUG-mio76-relay', 'phase': 'begin',
            'localCandidateCount': 0, 'memberOpen': True})
        self.assertNotIn('remoteCandidateCount', result)
        self.assertNotIn('freshAck', result)
        self.assertEqual(browser_relay_observations({}), {'getterAvailable': False, 'observations': None})

    def test_relay_count_rejects_boolean_and_native_parser_preserves_only_agreed_fields(self):
        with self.assertRaises(RuntimeError):
            relay_observation({'tag': 'DEBUG-mio76-relay', 'phase': 'begin', 'localCandidateCount': False})
        result = native_relay_observations([
            '[DEBUG-mio76-relay] {"tag":"DEBUG-mio76-relay","phase":"exchange","remoteCandidateCount":1,"rawSDP":"private"}',
            '[DEBUG-mio76-relay] invalid JSON',
        ])
        self.assertEqual(result['malformedReceipts'], 1)
        self.assertEqual(result['observations'], [{'tag': 'DEBUG-mio76-relay', 'phase': 'exchange',
                                                 'remoteCandidateCount': 1}])

    def test_probe_timeline_keeps_actual_zero_and_false_without_inventing_ack(self):
        values = {'tag': 'DEBUG-mio76-relay', 'phase': 'probe-progress',
                  'startedUtcMs': 1791186200000, 'elapsedMs': 1000,
                  'callerRemainingMs': 5000, 'stageRemainingMs': 0,
                  'probeWatermark': 4, 'exactProbePathPresent': True,
                  'probePathSelected': False, 'probePathRttMicros': 0,
                  'probePingTx': 1, 'probePingRx': 0, 'probeAckTx': 0, 'probeAckRx': 0,
                  'probePathAckTx': 1, 'probePathAckRx': 0,
                  'probeDatagramsTx': 1, 'probeDatagramsRx': 0,
                  'probeBytesTx': 1200, 'probeBytesRx': 0, 'probeLostPackets': 0}
        self.assertEqual(relay_observation({**values, 'rawPeerId': 'private'}), values)
        absent = relay_observation({'tag': 'DEBUG-mio76-relay', 'phase': 'probe-progress',
                                   'exactProbePathPresent': False})
        self.assertNotIn('probePingTx', absent)
        self.assertNotIn('freshAck', absent)

    def test_probe_timeline_rejects_wrong_types_and_preserves_native_malformed_count(self):
        for field, value in [('probePingTx', False), ('startedUtcMs', -1),
                             ('callerRemainingMs', None), ('exactProbePathPresent', 0)]:
            with self.subTest(field=field), self.assertRaises(RuntimeError):
                relay_observation({'tag': 'DEBUG-mio76-relay', 'phase': 'probe-progress',
                                   field: value})
        result = native_relay_observations([
            '[DEBUG-mio76-relay] {"tag":"DEBUG-mio76-relay","phase":"probe-progress","probePingTx":0}',
            '[DEBUG-mio76-relay] {"tag":"DEBUG-mio76-relay","phase":"probe-progress","probeAckRx":false}',
        ])
        self.assertEqual(result['malformedReceipts'], 1)
        self.assertEqual(result['observations'][0]['probePingTx'], 0)

    def test_home_watcher_preserves_observed_zero_while_absent_getter_remains_unknown(self):
        result = browser_relay_observations({'relayChecks': [], 'relayHome': {
            'publishedCount': 1, 'connectedCount': 0, 'errorCount': 0, 'rawAddress': 'private'}})
        self.assertEqual(result['relayHome'], {'publishedCount': 1, 'connectedCount': 0,
                                               'errorCount': 0})
        self.assertNotIn('relayHome', browser_relay_observations({'relayChecks': []}))
        with self.assertRaises(RuntimeError):
            browser_relay_observations({'relayChecks': [], 'relayHome': {'publishedCount': False}})


if __name__ == "__main__":
    unittest.main()
