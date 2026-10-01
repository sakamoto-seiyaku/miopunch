#!/usr/bin/env python3
"""Reproduce MIOPUNCH-37 and affected admission/credential regressions with immutable receipts."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import time
import tarfile
import shutil


ROOT = Path(__file__).resolve().parents[1]
SOURCES = [
    "Cargo.toml", "Cargo.lock", "src/lib.rs", "src/management_auth.rs",
    "src/management_store.rs", "src/native_network.rs", "src/genesis_recovery.rs",
    "src/management_recovery.rs", "tests/management_recovery.rs",
    "src/daily_management.rs", "src/manual_admission.rs", "src/precode.rs",
    "src/admission_gate.rs", "src/offline_replica.rs",
    "tests/revocation.rs", "tests/admission_gate.rs", "tests/precode.rs",
    "tests/manual_admission.rs", "tests/admission_completion.rs",
    "scripts/check_admission_rotation.py", "scripts/check_management_recovery.py", "scripts/mosquitto-test.conf",
]
SOURCES = sorted(set(SOURCES + [
    str(path.relative_to(ROOT))
    for directory in ["src", "tests", "browser-probe/src"]
    for path in (ROOT / directory).rglob("*.rs")
] + ["browser-probe/Cargo.toml"]))


# Unchanged NAT algorithms remain covered by frozen MIOPUNCH-60–63 evidence.
# Rotation changes credential gates; the two relevant invalidation cases below
# run explicitly. Other previously verified NAT cases remain omitted by user policy.
NAT_EXCLUSIONS = {
    "tests/legacy_multi_socket.rs": [
        "a_provisional_winner_closes_when_cancelled_and_cannot_be_used_by_another_connection",
        "a_real_target_send_error_does_not_truncate_following_candidates_or_random_work",
        "all_256_physical_listeners_close_on_cancel_and_rounds_do_not_accumulate",
        "authenticated_winner_keeps_its_actual_source_ip_despite_packets_to_another_local_ip",
        "cross_socket_round_packets_are_ignored_and_network_change_invalidates_pending_bindings",
        "discarded_winner_paths_release_descriptors_while_original_control_connection_lives",
        "every_physical_sender_isolates_packet_ttl_and_preserves_source_when_ttl_degrades",
        "interrupted_listener_setup_preserves_actual_partial_costs_and_closes_bindings",
        "nonprimary_winner_returns_through_its_physical_socket_on_the_original_connection",
        "os_bind_failures_keep_exact_attempt_count_and_release_partial_pool",
        "same_round_authenticated_receipt_cannot_move_to_another_physical_socket"
    ],
    "tests/legacy_nat_runtime.rs": [
        "assisted_only_fallback_receives_without_an_outbound_target_and_sender_keeps_lan_first",
        "cancelled_or_revoked_round_has_no_peer_budget_or_learning_feedback",
        "complete_scheduler_preserves_original_tls_connection_business_and_one_feedback",
        "dropping_attempt_during_sender_delay_spends_nothing_and_cannot_learn",
        "exhaustion_stops_full_phase_and_existing_authorized_control_remains_usable",
        "existing_verified_path_cannot_learn_when_budget_was_denied_or_credential_changed",
        "fallback_retains_a_freshly_acquired_candidate_for_its_phase",
        "formal_direct_candidates_precede_selector_and_keep_original_identity",
        "formal_disabled_udp_blocks_business_before_stream_creation",
        "formal_entry_observes_selects_executes_and_confirms_original_business",
        "formal_entry_permission_and_complete_budget_stop_before_execution",
        "formal_entry_without_s_uses_local_assisted_basic_attempt_and_original_business",
        "formal_ipv6_direct_requires_no_ipv4_observation_or_punching",
        "isolated_functional_cpu_cost_supplement",
        "low_ttl_and_unsupported_ttl_do_not_change_parallel_connection_business",
        "old_prediction_age_does_not_expire_a_legacy_plan_but_network_change_does",
        "ordinary_qad_refresh_keeps_the_physical_generation_but_real_network_change_invalidates_it",
        "receiver_can_win_before_delay_without_sending_a_probe_and_lease_closes",
        "scope_and_network_changes_during_delay_stop_before_submission",
        "stale_round_and_foreign_connection_authentication_cannot_select_a_winner",
        "two_real_qad_observers_public_preference_original_business_and_learning",
        "winner_handoff_waits_for_peer_inside_the_original_deadline_and_rechecks_scope",
        "wrong_gate_and_empty_fallback_cannot_disclose_candidates_or_spend_peer_budget",
        "wrong_network_and_old_round_are_rejected_before_snapshot_disclosure_or_selection"
    ],
    "tests/legacy_nat_scheduler.rs": [
        "legacy_range_is_inclusive_and_clips_to_valid_udp_ports",
        "random_listener_pair_has_the_original_thirty_second_read_extension",
        "random_sampling_keeps_legacy_bounds_ten_draw_skip_and_per_call_uniqueness",
        "sender_assisted_first_and_only_adjacent_duplicates_compact"
    ],
    "tests/udp_observation.rs": [
        "changed_nat_mapping_replaces_the_old_candidate_after_network_change",
        "destination_dependent_nat_reports_mapping_variance",
        "disabled_observation_endpoint_never_contacts_the_observer",
        "failed_periodic_observation_withdraws_the_old_udp_candidate",
        "mixed_observation_and_data_relay_configuration_fails_at_bind",
        "relay_disabled_observes_the_real_quic_socket_through_a_controlled_nat"
    ]
}


def cargo_tests(targets, selector=None):
    command = ["cargo", "test", "--locked", "--features", "replica-test-hooks"]
    for target in targets:
        command.extend(["--test", target])
    if selector:
        command.append(selector)
    return command + ["--", "--test-threads=1", "--nocapture"]


def checks(affected_rotation=False, recovery_fixes=False):
    selected = [
        ("rotation-suite", cargo_tests(["admission_rotation"])),
        ("affected-admission-suite", cargo_tests(["admission_gate", "manual_admission", "native_network", "management_store", "daily_management"])),
        ("raw-signed-response", cargo_tests(["admission_completion"], "signed_old_response_cannot_restore_locally_verified_removal")),
        ("raw-duplicate-receipt", cargo_tests(["admission_completion"], "duplicate_save_ack_does_not_consume_twice_or_prove_recipient_persistence")),
        ("root-recovery-entry", cargo_tests(["management_recovery"], "ordinary_contact_requires_root_proof_but_no_secret_history_or_old_approval")),
        ("root-recovery-rejoin", cargo_tests(["management_recovery"], "completed_recovery_after_effective_removal_can_rejoin_with_fresh_ordinary_approval")),
        ("genesis-recovery-entry", cargo_tests(["genesis_recovery"], "pinned_root_credential_recovers_from_ordinary_member_without_secret_or_old_admin")),
        ("nat-credential-invalidation", cargo_tests(["legacy_nat_runtime"], "existing_verified_path_cannot_learn_when_budget_was_denied_or_credential_changed")),
        ("nat-delayed-scope-invalidation", cargo_tests(["legacy_nat_runtime"], "scope_and_network_changes_during_delay_stop_before_submission")),
        ("browser-wasm", ["cargo", "check", "--locked", "-p", "miopunch-browser-probe", "--target", "wasm32-unknown-unknown"]),
        ("workspace-typecheck", ["cargo", "check", "--locked", "--workspace", "--tests"]),
        ("fault-build-typecheck", ["cargo", "check", "--locked", "--workspace", "--tests", "--features", "replica-test-hooks"]),
        ("format", ["cargo", "fmt", "--check"]),
    ]
    if recovery_fixes:
        return [("predecessor-recovery", cargo_tests(["admission_rotation"], "missed_predecessor_completion_recovers_when_initiator_starts_next_rotation")),
                ("original-provider-recovery", cargo_tests(["admission_rotation"], "original_")), *selected[-4:]]
    return [selected[0], *selected[-4:]] if affected_rotation else selected


def hashes():
    return {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in SOURCES}


def utc():
    return datetime.now(timezone.utc).isoformat()


def owned_resource_absent(kind, name):
    inspected = subprocess.run(["rtk", "proxy", "docker", kind, "inspect", name],
                               cwd=ROOT, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True)
    # Docker uses "No such container" but "no such volume" on this daemon.
    return {"kind": kind, "name": name, "actual_exit": inspected.returncode,
            "output": inspected.stdout,
            "absent": inspected.returncode != 0
            and f"no such {kind}" in inspected.stdout.lower()
            and name.lower() in inspected.stdout.lower()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path,
                        help="New evidence directory; existing results are never overwritten")
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--affected-rotation", action="store_true",
                        help="Revalidate the rotation protocol after a scoped rotation-only fix; prior unchanged admission/NAT checks remain separate evidence")
    selection.add_argument("--recovery-fixes", action="store_true",
                           help="Verify exact predecessor and original-provider recovery seams after a generation-guard fix")
    args = parser.parse_args()
    rotation_only = args.affected_rotation or args.recovery_fixes
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {"baseline": "a6e384ca9d19595721a1b373ebde47feb21efade",
               "started_utc": utc(), "source_sha256": hashes(), "commands": [],
               "nat_exclusions": NAT_EXCLUSIONS if rotation_only else {path: [name for name in names if name not in {"existing_verified_path_cannot_learn_when_budget_was_denied_or_credential_changed", "scope_and_network_changes_during_delay_stop_before_submission"}] for path, names in NAT_EXCLUSIONS.items()},
               "verification_scope": "predecessor-and-original-provider-regressions" if args.recovery_fixes else "rotation-only-regression" if args.affected_rotation else "rotation-and-affected-admission-credential",
               "prior_unchanged_checks_not_rerun": rotation_only,
               "executables": []}
    container = None
    success = False
    # Persist the manifest before even creating the broker, not only after
    # returning from Cargo. Recovery can inspect it if this runner disappears.
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    (output / "prestart-source-sha256.json").write_text(json.dumps(receipt["source_sha256"], indent=2) + "\n")
    with tarfile.open(output / "source-snapshot.tar.gz", "w:gz") as archive:
        for name in SOURCES:
            archive.add(ROOT / name, arcname=name, recursive=False)
    try:
        environment = os.environ.copy()
        if not rotation_only:
            container = subprocess.check_output([
                "rtk", "proxy", "docker", "run", "--detach",
                "--publish", "127.0.0.1::1883", "--mount",
                f"type=bind,source={ROOT / 'scripts/mosquitto-test.conf'},target=/mosquitto/config/mosquitto.conf,readonly",
                "eclipse-mosquitto:2",
            ], cwd=ROOT, text=True).strip()
            receipt["owned_container_id"] = container
            inspection = json.loads(subprocess.check_output([
                "rtk", "proxy", "docker", "inspect", container
            ], cwd=ROOT, text=True))[0]
            receipt["owned_anonymous_volumes"] = [mount["Name"] for mount in inspection["Mounts"] if mount["Type"] == "volume"]
            (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
            address = subprocess.check_output([
                "rtk", "proxy", "docker", "port", container, "1883/tcp"
            ], cwd=ROOT, text=True).strip()
            host, port = address.rsplit(":", 1)
            deadline = time.monotonic() + 10
            while True:
                try:
                    with socket.create_connection((host, int(port)), timeout=1):
                        break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.1)
            receipt["mqtt_broker"] = address
            receipt["broker_tcp_preflight"] = True
            environment = os.environ.copy()
            environment["MIOPUNCH_MQTT_BROKER"] = address
        for name, command in checks(args.affected_rotation, args.recovery_fixes):
            check = {"name": name, "argv": ["rtk", "proxy", *command], "started_utc": utc()}
            receipt["commands"].append(check)
            print(f"Starting {name}; raw log: {output / (name + '.log')}", flush=True)
            (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
            started = time.monotonic()
            with (output / (name + ".log")).open("w") as log:
                result = subprocess.run(check["argv"], cwd=ROOT, env=environment,
                                        stdout=log, stderr=subprocess.STDOUT)
            raw_log = (output / (name + ".log")).read_text()
            for binary in re.findall(r"Running [^\n]+\((target/debug/deps/[^) ]+)\)", raw_log):
                path = ROOT / binary
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                frozen = Path("/tmp/mio37-frozen-executables") / digest / path.name
                frozen.parent.mkdir(parents=True, exist_ok=True)
                if not frozen.exists():
                    shutil.copyfile(path, frozen)
                    frozen.chmod(0o500)
                assert hashlib.sha256(frozen.read_bytes()).hexdigest() == digest
                receipt["executables"].append({"command": name, "path": binary, "sha256": digest, "frozen_path": str(frozen)})
            check.update(exit_code=result.returncode, finished_utc=utc(),
                         wall_seconds=time.monotonic() - started)
            # Save each actual exit boundary before parsing or broader checks.
            (output / (name + "-exit.json")).write_text(json.dumps(check, indent=2) + "\n")
            receipt["source_unchanged_through_checks"] = hashes() == receipt["source_sha256"]
            (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
            print(f"Finished {name}: exit {result.returncode}", flush=True)
            if result.returncode or not receipt["source_unchanged_through_checks"]:
                raise RuntimeError(f"{name} failed or source changed")
        suite = "\n".join((output / (name + ".log")).read_text() for name, _ in checks(args.affected_rotation, args.recovery_fixes) if "cargo" in _ and "test" in _)
        totals = [tuple(map(int, row)) for row in re.findall(
            r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", suite)]
        receipt["suite_totals"] = dict(zip(["passed", "failed", "ignored"],
                                            [sum(row[i] for row in totals) for i in range(3)]))
        receipt["broker_tests_not_skipped"] = "SKIPPED" not in suite
        capture = re.search(r"MIO30_MQTT_CIPHERTEXT[^\n]*", suite)
        receipt["actual_mqtt_capture"] = capture.group(0) if capture else None
        if not totals or not receipt["broker_tests_not_skipped"] or (not rotation_only and not capture):
            raise RuntimeError("affected suite lacks required real-broker evidence")
        success = True
    finally:
        if container:
            cleanup = subprocess.run(["rtk", "proxy", "docker", "rm", "--force", "--volumes", container],
                                     cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                     text=True)
            receipt["owned_container_cleanup_exit"] = cleanup.returncode
            receipt["owned_container_cleanup_output"] = cleanup.stdout
            receipt["owned_container_removed"] = cleanup.returncode == 0
            receipt["owned_resource_absence"] = {}
            for kind, names in [("container", [container]), ("volume", receipt.get("owned_anonymous_volumes", []))]:
                for owned_name in names:
                    receipt["owned_resource_absence"][owned_name] = owned_resource_absent(kind, owned_name)
            receipt["owned_container_removed"] = receipt["owned_container_removed"] and all(item["absent"] for item in receipt["owned_resource_absence"].values())
        receipt["finished_utc"] = utc()
        receipt["success"] = success and (container is None or receipt.get("owned_container_removed", False))
        receipt["owned_broker_created_for_this_run"] = container is not None
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: receipt.get(key) for key in ["success", "suite_totals", "actual_mqtt_capture", "owned_container_removed"]}), flush=True)
    return 0 if receipt["success"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
