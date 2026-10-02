#!/usr/bin/env python3
"""Ordered real node maintenance checks with an owned broker and short regression."""
import argparse
import json
import os
import pathlib
import socket
import signal
import re
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=pathlib.Path)
    parser.add_argument('--case')
    parser.add_argument('--suite', action='store_true')
    parser.add_argument('--review-checks', action='store_true', help='acceptance plus affected post-review resource/recovery/type/format checks')
    parser.add_argument('--start-at', help='continue only the remaining suite checks')
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'checks': [], 'passed': False, 'cleanup': []}
    name = f'miopunch49-maintenance-{time.time_ns()}'
    owned_names = []
    def interrupted(signum, frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    try:
        subprocess.run(['rtk', 'proxy', 'docker', 'run', '--detach', '--name', name, '--publish', '127.0.0.1::1883', '--mount', f'type=bind,src={root / "scripts/mosquitto-test.conf"},target=/mosquitto/config/mosquitto.conf,readonly', 'eclipse-mosquitto:2'], check=True, capture_output=True, text=True)
        owned_names.append(name)
        broker = subprocess.check_output(['rtk', 'proxy', 'docker', 'port', name, '1883/tcp'], text=True).strip()
        host, port = broker.rsplit(':', 1)
        deadline = time.monotonic() + 10
        while True:
            try:
                with socket.create_connection((host, int(port)), timeout=1):
                    break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.1)
        # A distinct live origin is available to multi-origin regression checks.
        second_name = name + '-second'
        subprocess.run(['rtk', 'proxy', 'docker', 'run', '--detach', '--name', second_name, '--publish', '127.0.0.1::1883', '--mount', f'type=bind,src={root / "scripts/mosquitto-test.conf"},target=/mosquitto/config/mosquitto.conf,readonly', 'eclipse-mosquitto:2'], check=True, capture_output=True, text=True)
        owned_names.append(second_name)
        second_broker = subprocess.check_output(['rtk', 'proxy', 'docker', 'port', second_name, '1883/tcp'], text=True).strip()
        host, port = second_broker.rsplit(':', 1)
        deadline = time.monotonic() + 10
        while True:
            try:
                with socket.create_connection((host, int(port)), timeout=1):
                    break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.1)
        env = dict(os.environ, MIOPUNCH_MQTT_BROKER=broker, MIOPUNCH_MQTT_SECOND_BROKER=second_broker)
        receipt['brokers'] = [broker, second_broker]
        receipt['environment'] = 'native Linux IPv4 loopback, independent identities and SQLite, real TLS/SCRAM/QUIC/LogSync, owned MQTT and controlled UDP failure observers'
        receipt['skipped'] = 'Already verified unchanged slow NAT matrices; unaffected IRC and port-mapping labs, per the standing user test policy'
        cases = [args.case] if args.case else [
            'startup_automatically_recovers_and_saves_history_without_business',
            'continuous_failure_backs_off_then_an_online_member_recovers_without_dial_storm',
            'a_new_contact_wakes_waiting_maintenance_and_verified_contact_survives_restart',
            'conservative_and_active_nodes_observe_distinct_frequency_and_concurrency_budgets',
            'real_business_has_priority_and_shared_identity_guard_prevents_duplicate_dials',
            'partial_links_propagate_new_signed_history_on_the_same_neighbor_session',
            'node_quality_budget_changes_actual_quic_probe_frequency',
            'active_budget_never_enables_disabled_lookup_or_forbidden_data_carriers',
            'stale_saved_contact_expands_to_discovery_and_recovers_an_unknown_member',
            'starting_business_does_not_abort_an_already_admitted_sync_exchange',
            'a_full_neighbor_budget_still_explores_a_new_source_without_requiring_full_mesh',
            'repeated_authenticated_replica_failures_increment_backoff_and_leave_connected_state',
        ]
        checks = [(case, ['cargo', 'test', '--locked', '--features', 'replica-test-hooks', '--test', 'network_maintenance', case, '--', '--exact', '--nocapture']) for case in cases]
        def tests(*targets):
            command = ['cargo', 'test', '--locked', '--features', 'replica-test-hooks,nat-lab-test-hooks,member-signaling-test-hooks']
            for target in targets:
                command += ['--test', target]
            return command + ['--', '--test-threads=1', '--nocapture']
        if args.suite:
            checks += [
                ('resources', tests('resources')),
                ('native-admission', tests('native_network', 'admission_gate', 'admission_proof')),
                ('network-recovery', tests('network_recovery')),
                ('offline-replica', tests('offline_replica')),
                ('path-maintenance', tests('path_maintenance')),
                ('member-relay', tests('member_relay')),
                ('mqtt-purpose', tests('mqtt_bootstrap', 'mqtt_probe', 'mqtt_upgrade', 'unknown_member_bootstrap')),
                ('tcp-direct', tests('tcp_direct')),
                ('workspace-library', ['cargo', 'test', '--locked', '--workspace', '--lib', '--', '--test-threads=1']),
                ('typecheck', ['cargo', 'check', '--locked', '--workspace', '--all-targets', '--features', 'replica-test-hooks,nat-lab-test-hooks,member-signaling-test-hooks']),
                ('wasm', ['cargo', 'check', '--locked', '-p', 'miopunch-browser-probe', '--target', 'wasm32-unknown-unknown']),
                ('format', ['cargo', 'fmt', '--check']),
            ]
        if args.review_checks:
            checks += [('resources', tests('resources')), ('network-recovery', tests('network_recovery')), ('typecheck', ['cargo', 'check', '--locked', '--workspace', '--all-targets', '--features', 'replica-test-hooks,nat-lab-test-hooks,member-signaling-test-hooks']), ('format', ['cargo', 'fmt', '--check'])]
        if args.start_at:
            names = [name for name, _ in checks]
            if not args.suite or args.start_at not in names:
                parser.error('start-at requires a valid suite check')
            checks = checks[names.index(args.start_at):]
        for check, command in checks:
            print(f'Starting {check}', flush=True)
            started = time.monotonic()
            with (output / f'{check}.log').open('w') as log:
                result = subprocess.run(['rtk', 'proxy', *command], cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT)
            receipt['checks'].append({'name': check, 'argv': command, 'exit': result.returncode, 'seconds': round(time.monotonic()-started, 3), 'log': f'{check}.log', 'reported_passed': sum(map(int, re.findall(r'test result: ok\. (\d+) passed', (output / f'{check}.log').read_text())))})
            (output / 'results.json').write_text(json.dumps(receipt, indent=2)+'\n')
            if result.returncode:
                break
            if check in cases:
                raw = (output / f'{check}.log').read_text()
                if 'running 1 test' not in raw or '1 passed' not in raw:
                    raise RuntimeError(f'case did not execute exactly one passing test: {check}')
        else:
            receipt['passed'] = True
    except KeyboardInterrupt:
        receipt['interrupted'] = True
    except Exception as error:
        receipt['error'] = str(error)
    finally:
        for name in owned_names:
            result = subprocess.run(['rtk', 'proxy', 'docker', 'rm', '--force', '--volumes', name], capture_output=True, text=True)
            receipt['cleanup'].append({'owned_container': name, 'exit': result.returncode})
            receipt['passed'] &= result.returncode == 0
        (output / 'results.json').write_text(json.dumps(receipt, indent=2)+'\n')
    print(f'{"PASS" if receipt["passed"] else "FAIL"}: {output}')
    raise SystemExit(0 if receipt['passed'] else 1)


if __name__ == '__main__':
    main()
