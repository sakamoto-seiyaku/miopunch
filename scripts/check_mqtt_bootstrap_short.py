#!/usr/bin/env python3
"""Owned real brokers for affected public native/MQTT checks; no NAT matrices."""
import argparse
import json
import os
import pathlib
import signal
import socket
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--selector')
    parser.add_argument('--suite', action='store_true')
    parser.add_argument('--start-at', help='resume a suite at the named check')
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'checks': [], 'passed': False}
    owned = []
    def interrupted(signum, frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    try:
        for _ in range(2):
            name = f'miopunch42-short-{time.time_ns()}'
            command = ['rtk', 'proxy', 'docker', 'run', '--detach', '--name', name, '--publish', '127.0.0.1::1883', '--mount', f'type=bind,src={root / "scripts/mosquitto-test.conf"},target=/mosquitto/config/mosquitto.conf,readonly', 'eclipse-mosquitto:2']
            subprocess.run(command, check=True, capture_output=True, text=True)
            owned.append(name)
            address = subprocess.check_output(['rtk', 'proxy', 'docker', 'port', name, '1883/tcp'], text=True).strip()
            host, port = address.rsplit(':', 1)
            deadline = time.monotonic()+10
            while True:
                try:
                    with socket.create_connection((host, int(port)), timeout=1):
                        break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.1)
            receipt.setdefault('brokers', []).append(address)
        environment = dict(os.environ, MIOPUNCH_MQTT_BROKER=receipt['brokers'][0], MIOPUNCH_MQTT_SECOND_BROKER=receipt['brokers'][1])
        def tests(*targets, selector=None):
            command = ['cargo', 'test', '--locked']
            for target in targets:
                command += ['--test', target]
            if selector:
                command.append(selector)
            return command + ['--', '--test-threads=1', '--nocapture']
        checks = [('tests', tests('mqtt_bootstrap', selector=args.selector))]
        if args.suite:
            if args.selector:
                parser.error('suite and selector are separate verification modes')
            checks += [
                ('resources', tests('resources')),
                ('native-gates-proofs', tests('native_network', 'admission_gate', 'admission_proof')),
                ('mqtt-carrier-and-purpose', tests('mqtt_probe', 'mqtt_upgrade')),
                ('manual-admission', tests('manual_admission')),
                ('offline-replica', tests('offline_replica')),
                ('tcp-direct', tests('tcp_direct')),
                ('formal-node-budget', tests('legacy_nat_runtime', selector='formal_')),
                ('observation-generation', tests('legacy_nat_runtime', selector='ordinary_qad_refresh_keeps_the_physical_generation_but_real_network_change_invalidates_it')),
                ('candidate-scope', tests('legacy_nat_runtime', selector='old_prediction_age_does_not_expire_a_legacy_plan_but_network_change_does')),
                ('library', ['cargo', 'test', '--locked', '--lib', '--', '--test-threads=1']),
                ('all-targets', ['cargo', 'check', '--locked', '--workspace', '--all-targets', '--features', 'nat-lab-test-hooks,replica-test-hooks']),
                ('wasm', ['cargo', 'check', '--locked', '-p', 'miopunch-browser-probe', '--target', 'wasm32-unknown-unknown']),
                ('format', ['cargo', 'fmt', '--check']),
                ('fork-format', ['rustfmt', '--check', '--edition', '2024', '--config', 'skip_children=true', 'vendor/iroh/src/endpoint/connection.rs', 'vendor/iroh/src/endpoint/hooks.rs', 'vendor/iroh/src/socket/remote_map/remote_state.rs']),
            ]
        if args.start_at:
            if not args.suite or args.start_at not in [name for name, _ in checks]:
                parser.error('start-at requires a suite check name')
            checks = checks[next(i for i, (name, _) in enumerate(checks) if name == args.start_at):]
        for name, command in checks:
            print(f'Starting {name}', flush=True)
            started = time.monotonic()
            with (output/(name+'.log')).open('w') as log:
                result = subprocess.run(['rtk', 'proxy', *command], cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT)
            receipt['checks'].append({'argv': command, 'exit': result.returncode, 'seconds': round(time.monotonic()-started, 3), 'log': name+'.log'})
            (output/'results.json').write_text(json.dumps(receipt, indent=2)+'\n')
            if result.returncode:
                break
        else:
            receipt['passed'] = True
    except KeyboardInterrupt:
        receipt['interrupted'] = True
        receipt['passed'] = False
    finally:
        receipt['cleanup'] = []
        for name in owned:
            result = subprocess.run(['rtk', 'proxy', 'docker', 'rm', '--force', '--volumes', name], capture_output=True, text=True)
            receipt['cleanup'].append({'owned_container': name, 'exit': result.returncode})
            receipt['passed'] &= result.returncode == 0
        (output/'results.json').write_text(json.dumps(receipt, indent=2)+'\n')
    print(f'{"PASS" if receipt["passed"] else "FAIL"} short MQTT checks: {output}')
    raise SystemExit(0 if receipt['passed'] else 1)


if __name__ == '__main__':
    main()
