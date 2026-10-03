#!/usr/bin/env python3
"""Bounded real browser/native IRC-WSS/native TLS slow-datagram qualification."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time

from browser_mqtt_trust import certificates, private_nss

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=['irc-datagram', 'irc-restricted-handshake', 'irc-mixed-retry'], default='irc-datagram')
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--wasm-bindgen', default=shutil.which('wasm-bindgen') or '/tmp/miopunch-wasm-tools/bin/wasm-bindgen')
    parser.add_argument('--caller-budget-ms', type=int, choices=[12000,45000], default=12000)
    parser.add_argument('--mode', choices=['retry', 'delayed-first'], default='retry')
    args = parser.parse_args()
    protocol_budget = 60
    cleanup_reserve = 10  # process-group termination (3), log (2), removal (5)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'case': args.case, 'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
               'passed': False, 'checks': [], 'protocol_budget_seconds': protocol_budget,
               'protocol_work_budget_seconds': protocol_budget - cleanup_reserve,
               'cleanup_reserve_seconds': cleanup_reserve,
               'phase_budget_seconds': 15, 'public_traffic': False, 'cleanup': []}
    owned = None
    owned_mqtt = None
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
            run('native-build', ['cargo', 'build', '--locked', '--offline', '-j', '2', '--bin', 'browser_irc_restricted_native' if args.case != 'irc-datagram' else 'browser_irc_datagram'], 600)
            run('wasm-build', ['cargo', 'build', '--locked', '--offline', '-j', '2', '-p', 'miopunch-browser-probe', '--target', 'wasm32-unknown-unknown'], 600)
            run('wasm-bindgen', [args.wasm_bindgen, '--target', 'web', '--out-dir', 'browser-probe/pkg', 'target/wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm'], 60)
        with tempfile.TemporaryDirectory(prefix='mio-rtc-irc-') as fixture_name:
            fixture = Path(fixture_name)
            started = time.monotonic()
            protocol_started = started
            signal.setitimer(signal.ITIMER_REAL, 15)
            ca, leaf, leaf_key = certificates(fixture)
            private_nss(fixture / 'nssdb', ca)
            receipt['certificate'] = {'normal_verification': True, 'private_trust_only': True,
                                      'leaf_pem_sha256': hashlib.sha256(leaf.read_bytes()).hexdigest()}
            owned = f'miopunch-rtc-irc-{time.time_ns()}'
            base = (ROOT / 'tests/fixtures/irc/ergo.yaml').read_text()
            assert 'fakelag:\n  enabled: false' in base
            configured = base.replace('    :6667: {}', '    :6697:\n      tls:\n        cert: /fixture/leaf.pem\n        key: /fixture/leaf.key\n    :8097:\n      websocket: true\n      tls:\n        cert: /fixture/leaf.pem\n        key: /fixture/leaf.key')
            configured = configured.replace('fakelag:\n  enabled: false', 'fakelag:\n  enabled: true')
            configured = configured.replace('path: ircd.db', 'path: /tmp/irc-owned.db')
            configured = configured.replace('motd: ergo.motd', 'motd: /ircd-bin/ergo.motd')
            config = fixture / 'ergo.yaml'
            config.write_text(configured)
            image = 'ergochat/ergo@sha256:cf50544214cf196d25913553a451b766101edce114b5e005a7c063da578851a7'
            subprocess.run(['rtk', 'proxy', 'docker', 'run', '--detach', '--name', owned,
                            '--publish', '127.0.0.1::6697', '--publish', '127.0.0.1::8097',
                            '--mount', f'type=bind,src={fixture},target=/fixture,readonly',
                            '--entrypoint', '/ircd-bin/ergo', image, 'run', '--conf', '/fixture/ergo.yaml'],
                           check=True, capture_output=True, text=True, timeout=15)
            addresses = {}
            for port in (6697, 8097):
                address = subprocess.check_output(['rtk', 'proxy', 'docker', 'port', owned, f'{port}/tcp'], text=True, timeout=5).strip()
                host, forwarded = address.rsplit(':', 1)
                until = time.monotonic() + 10
                while True:
                    try:
                        with socket.create_connection((host, int(forwarded)), timeout=1):
                            break
                    except OSError:
                        if time.monotonic() >= until:
                            raise
                        time.sleep(0.1)
                addresses[str(port)] = address
            receipt['owned_ergo'] = {'addresses': addresses, 'container': owned, 'image': image,
                                     'fakelag': {'enabled': True, 'window_seconds': 1, 'burst': 5, 'messages_per_window': 2}}
            broker_arguments = []
            if args.case == 'irc-mixed-retry':
                owned_mqtt = f'miopunch-irc-mqtt-retry-{time.time_ns()}'
                subprocess.run(['rtk', 'proxy', 'docker', 'run', '--detach', '--name', owned_mqtt,
                                '--publish', '127.0.0.1::1883', '--mount',
                                f'type=bind,src={ROOT / "scripts/mosquitto-test.conf"},target=/mosquitto/config/mosquitto.conf,readonly',
                                'eclipse-mosquitto:2'], check=True, capture_output=True, text=True, timeout=10)
                broker = subprocess.check_output(['rtk', 'proxy', 'docker', 'port', owned_mqtt, '1883/tcp'], text=True, timeout=3).strip()
                host, forwarded = broker.rsplit(':', 1)
                until = time.monotonic() + 5
                while True:
                    try:
                        with socket.create_connection((host, int(forwarded)), timeout=1): break
                    except OSError:
                        if time.monotonic() >= until: raise
                        time.sleep(0.1)
                receipt['owned_mqtt'] = {'container': owned_mqtt, 'address': broker, 'image': 'eclipse-mosquitto:2'}
                broker_arguments = ['--broker', broker, '--mode', args.mode]
            receipt['fixture_setup_seconds'] = round(time.monotonic() - started, 3)
            remaining = protocol_budget - cleanup_reserve - (time.monotonic() - protocol_started)
            if remaining <= 0:
                raise TimeoutError('IRC protocol work budget exhausted before browser start')
            signal.setitimer(signal.ITIMER_REAL, remaining)
            # Compilation is separately recorded. Fixture provisioning and the
            # complete protocol run and cleanup share one 60-second attempt.
            # Reserve termination/removal time before running the browser worker.
            run(args.case, ['node', {'irc-restricted-handshake':'browser-probe/harness/run-irc-restricted.mjs', 'irc-mixed-retry':'browser-probe/harness/run-irc-mixed-retry.mjs', 'irc-datagram':'browser-probe/harness/run-irc-datagram.mjs'}[args.case], '--case', args.case,
                                 '--evidence', str(output), '--fixture', str(fixture), '--native', addresses['6697'], '--wss', addresses['8097'], '--caller-budget-ms', str(args.caller_budget_ms), *broker_arguments], remaining)
            receipt['passed'] = json.loads((output / f'{args.case}.json').read_text())['passed']
    except (Exception, KeyboardInterrupt) as error:
        receipt['failure'] = str(error) or 'cancelled'
        receipt['passed'] = False
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        cleanup_containers = [name for name in [owned, owned_mqtt] if name]
        for cleanup_container in cleanup_containers:
            cleanup = {'container': cleanup_container}

            def remaining_cleanup(cap, reserve=0):
                remaining = protocol_budget - (time.monotonic() - protocol_started) - reserve
                if remaining <= 0:
                    raise TimeoutError('IRC total attempt cleanup deadline')
                return min(cap, remaining)

            # Logging is optional diagnostics. Its failure must never skip removal.
            try:
                with (output / ('ergo-server.log' if cleanup_container == owned else 'mqtt-server.log')).open('w') as log:
                    result = subprocess.run(['rtk', 'proxy', 'docker', 'logs', cleanup_container],
                                            stdout=log, stderr=subprocess.STDOUT,
                                            timeout=remaining_cleanup(2 / len(cleanup_containers), reserve=5))
                    cleanup['log_exit'] = result.returncode
            except (Exception, KeyboardInterrupt) as error:
                cleanup['log_error'] = str(error) or 'cancelled during log collection'
            try:
                result = subprocess.run(['rtk', 'proxy', 'docker', 'rm', '--force', '--volumes', cleanup_container],
                                        capture_output=True, text=True, timeout=remaining_cleanup(5 / len(cleanup_containers)))
                code = result.returncode
            except (Exception, KeyboardInterrupt) as error:
                code = str(error)
            cleanup['exit'] = code
            receipt['cleanup'].append(cleanup)
            receipt['passed'] &= code == 0
        receipt['completed_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
        receipt['total_seconds'] = round(time.monotonic() - task_started, 3)
        if protocol_started is not None:
            receipt['protocol_total_seconds'] = round(time.monotonic() - protocol_started, 3)
            if receipt['protocol_total_seconds'] > protocol_budget:
                receipt['passed'] = False
                receipt['failure'] = 'IRC total protocol and cleanup budget exceeded'
        save()
    print(f'{"PASS" if receipt["passed"] else "FAIL"} {args.case}: {output}')
    raise SystemExit(0 if receipt['passed'] else 1)


if __name__ == '__main__':
    main()
