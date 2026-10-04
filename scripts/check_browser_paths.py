#!/usr/bin/env python3
"""Bounded real browser/native MQTT datagram and restricted admission checks."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

from browser_mqtt_trust import certificates, private_nss

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=['mqtt-datagram', 'wss-restricted-handshake', 'irc-datagram', 'rtc-channel', 'relay-member-rtc', 'wss-member-rtc', 'rtc-data-cut', 'rtc-invalidation', 'rtc-member-replica', 'wss-admission-materials', 'rtc-cross-network'], required=True)
    parser.add_argument('--cut-mode', choices=['relay', 'none'], default='relay')
    parser.add_argument('--invalidation-mode', choices=['cancel', 'member', 'secret-standard', 'resource'], default='cancel')
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--wasm-bindgen', default=shutil.which('wasm-bindgen') or '/tmp/miopunch-wasm-tools/bin/wasm-bindgen')
    parser.add_argument('--target-dir', type=Path, default=ROOT / 'target')
    args = parser.parse_args()
    if args.case == 'rtc-cross-network':
        command = ['rtk', 'proxy', sys.executable, str(ROOT / 'scripts/check_browser_rtc_cross_network.py'),
                   '--output', str(args.output), '--target-dir', str(args.target_dir)]
        if args.skip_build:
            command.append('--skip-build')
        os.execvp(command[0], command)
    if args.case in ('rtc-data-cut', 'rtc-invalidation', 'rtc-member-replica', 'wss-admission-materials'):
        command = ['rtk', 'proxy', sys.executable, str(ROOT / 'scripts/check_browser_rtc_cut.py'), '--mode', args.cut_mode,
                   '--case', args.case,
                   '--output', str(args.output), '--target-dir', str(args.target_dir), '--wasm-bindgen', args.wasm_bindgen]
        if args.case == 'rtc-invalidation':
            command.extend(['--invalidation-mode', args.invalidation_mode])
        if args.skip_build:
            command.append('--skip-build')
        os.execvp(command[0], command)
    if args.case == 'irc-datagram':
        if args.target_dir.resolve() != (ROOT / 'target').resolve():
            parser.error('irc-datagram uses the existing repository target directory')
        command = ['rtk', 'proxy', sys.executable, str(ROOT / 'scripts/check_browser_irc.py'),
                   '--case', args.case, '--output', str(args.output),
                   '--wasm-bindgen', args.wasm_bindgen]
        if args.skip_build:
            command.append('--skip-build')
        os.execvp(command[0], command)
    if args.case in ('rtc-channel', 'relay-member-rtc', 'wss-member-rtc'):
        command = ['rtk', 'proxy', sys.executable, str(ROOT / 'scripts/check_browser_rtc.py'),
                   '--case', args.case, '--output', str(args.output), '--target-dir', str(args.target_dir),
                   '--wasm-bindgen', args.wasm_bindgen]
        if args.skip_build:
            command.append('--skip-build')
        os.execvp(command[0], command)
    protocol_budget = 45 if args.case == 'wss-restricted-handshake' else 60
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'case': args.case, 'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
               'passed': False, 'checks': [], 'protocol_budget_seconds': protocol_budget,
               'phase_budget_seconds': 15, 'public_traffic': False, 'cleanup': []}
    owned = None
    task_started = time.monotonic()
    protocol_started = None

    def save():
        (output / 'results.json').write_text(json.dumps(receipt, indent=2) + '\n')

    def run(name, command, budget, environment=None):
        started = time.monotonic()
        with (output / f'{name}.log').open('w') as log:
            process = subprocess.Popen(['rtk', 'proxy', *command], cwd=ROOT,
                                       stdout=log, stderr=subprocess.STDOUT,
                                       env=environment, start_new_session=True)
            try:
                code = process.wait(timeout=budget)
            except (subprocess.TimeoutExpired, KeyboardInterrupt, TimeoutError):
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                code = 'deadline-or-cancel'
        receipt['checks'].append({'name': name, 'argv': command, 'exit': code,
                                  'budget_seconds': budget, 'seconds': round(time.monotonic() - started, 3)})
        save()
        if code != 0:
            raise RuntimeError(f'{name} failed; see {output / (name + ".log")}')

    def cancelled(signum, frame):
        raise KeyboardInterrupt
    def deadline(signum, frame):
        raise TimeoutError('protocol outer or fixture stage deadline')
    signal.signal(signal.SIGTERM, cancelled)
    signal.signal(signal.SIGALRM, deadline)
    try:
        if not args.skip_build:
            run('native-build', ['cargo', 'build', '--locked', '--offline', '-j', '2', '--bin', 'browser_restricted_native' if args.case == 'wss-restricted-handshake' else 'browser_mqtt_datagram'], 600)
            run('wasm-build', ['cargo', 'build', '--locked', '--offline', '-j', '2', '-p', 'miopunch-browser-probe', '--target', 'wasm32-unknown-unknown'], 600)
            run('wasm-bindgen', [args.wasm_bindgen, '--target', 'web', '--out-dir', 'browser-probe/pkg', 'target/wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm'], 60)
        with tempfile.TemporaryDirectory(prefix='mio-rtc-mqtt-') as fixture_name:
            fixture = Path(fixture_name)
            started = time.monotonic()
            protocol_started = started
            signal.setitimer(signal.ITIMER_REAL, 15)
            ca, leaf, leaf_key = certificates(fixture)
            private_nss(fixture / 'nssdb', ca)
            receipt['certificate'] = {'normal_verification': True, 'private_trust_only': True,
                                      'leaf_pem_sha256': hashlib.sha256(leaf.read_bytes()).hexdigest()}
            owned = f'miopunch-rtc-mqtt-{time.time_ns()}'
            subprocess.run(['rtk', 'proxy', 'docker', 'run', '--detach', '--name', owned,
                            '--publish', '127.0.0.1::1883', '--mount',
                            f'type=bind,src={ROOT / "scripts/mosquitto-test.conf"},target=/mosquitto/config/mosquitto.conf,readonly',
                            'eclipse-mosquitto:2'], check=True, capture_output=True, text=True, timeout=15)
            address = subprocess.check_output(['rtk', 'proxy', 'docker', 'port', owned, '1883/tcp'], text=True, timeout=5).strip()
            host, port = address.rsplit(':', 1)
            until = time.monotonic() + 10
            while True:
                try:
                    with socket.create_connection((host, int(port)), timeout=1):
                        break
                except OSError:
                    if time.monotonic() >= until:
                        raise
                    time.sleep(0.1)
            receipt['owned_broker'] = {'address': address, 'container': owned, 'image': 'eclipse-mosquitto:2'}
            receipt['fixture_setup_seconds'] = round(time.monotonic() - started, 3)
            remaining = max(0.001, protocol_budget - (time.monotonic() - protocol_started))
            signal.setitimer(signal.ITIMER_REAL, remaining)
            # Compilation is separately recorded. Fixture provisioning and the
            # complete protocol run share one 60-second attempt budget.
            run(args.case, ['node', 'browser-probe/harness/run-mqtt-datagram.mjs', '--case', args.case,
                                 '--evidence', str(output), '--fixture', str(fixture), '--broker', address], remaining)
            receipt['passed'] = json.loads((output / f'{args.case}.json').read_text())['passed']
    except (Exception, KeyboardInterrupt) as error:
        receipt['failure'] = str(error) or 'cancelled'
        receipt['passed'] = False
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        if owned:
            try:
                result = subprocess.run(['rtk', 'proxy', 'docker', 'rm', '--force', '--volumes', owned],
                                        capture_output=True, text=True, timeout=15)
                code = result.returncode
            except Exception as error:
                code = str(error)
            receipt['cleanup'].append({'container': owned, 'exit': code})
            receipt['passed'] &= code == 0
        receipt['completed_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
        receipt['total_seconds'] = round(time.monotonic() - task_started, 3)
        if protocol_started is not None:
            receipt['protocol_total_seconds'] = round(time.monotonic() - protocol_started, 3)
        save()
    print(f'{"PASS" if receipt["passed"] else "FAIL"} {args.case}: {output}')
    raise SystemExit(0 if receipt['passed'] else 1)


if __name__ == '__main__':
    main()
