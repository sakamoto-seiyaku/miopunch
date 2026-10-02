#!/usr/bin/env python3
"""Real pinned IRCd acceptance in order; optional affected short regressions."""
import argparse
import json
import os
import pathlib
import signal
import socket
import subprocess
import time

IMAGE = 'ergochat/ergo@sha256:cf50544214cf196d25913553a451b766101edce114b5e005a7c063da578851a7'
CASES = [
    ('first', 'real_irc_replaces_mqtt_at_the_same_native_connection_entry'),
    ('priority', 'a_missing_fragment_of_the_real_long_handshake_cannot_release_a_connection'),
    ('reassembly', 'incomplete_public_envelopes_have_bounded_reassembly_and_expire'),
    ('duplicates-injection', 'duplicate_fragments_and_injected_ciphertext_do_not_impersonate_or_leak'),
    ('purpose', 'reachable_irc_cannot_release_business_without_a_permitted_data_path'),
    ('identity-membership', 'an_irc_route_cannot_substitute_tls_identity_or_unapproved_membership'),
    ('cancel', 'canceled_handshake_releases_the_attempt_and_a_later_contact_succeeds'),
    ('reconnect', 'disconnected_irc_withdraws_routes_and_fresh_binding_preserves_identity'),
    ('rate-limit', 'actual_server_rate_limiting_returns_bounded_unavailability'),
    ('resources', 'saved_irc_disable_and_a_failed_service_preserve_other_resource_choices'),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--short', action='store_true')
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'checks': [], 'passed': False, 'image': IMAGE,
               'environment': 'Linux real Ergo 2.7.0, loopback UDP; isolated offline kernel data-cut',
               'skipped': 'Already verified unchanged slow NAT matrices, per user policy'}
    owned = []

    def save():
        (output / 'results.json').write_text(json.dumps(receipt, indent=2) + '\n')

    def execute(name, command, environment=None, timeout=300):
        print(f'Starting {name}', flush=True)
        start = time.monotonic()
        with (output / (name + '.log')).open('w') as log:
            result = subprocess.run(['rtk', 'proxy', *command], cwd=root, env=environment,
                                    stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
        receipt['checks'].append({'name': name, 'argv': command, 'exit': result.returncode,
                                  'seconds': round(time.monotonic() - start, 3),
                                  'log': name + '.log'})
        save()
        if result.returncode:
            raise RuntimeError(f'{name} failed: read {output / (name + ".log")}')

    def interrupted(_signal, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    try:
        base = root / 'tests/fixtures/irc/ergo.yaml'
        limited = output / 'limited.yaml'
        text = base.read_text()
        marker = 'fakelag:\n  enabled: false'
        if marker not in text:
            raise RuntimeError('rate-limit fixture marker changed')
        limited.write_text(text.replace(marker, 'fakelag:\n  enabled: true', 1))
        addresses = []
        for purpose, config in [('usable', base), ('limited', limited)]:
            name = f'miopunch45-{purpose}-{time.time_ns()}'
            owned.append(name)
            execute('start-' + purpose, ['docker', 'run', '--detach', '--name', name,
                    '--publish', '127.0.0.1::6667', '--entrypoint', '/ircd-bin/ergo',
                    '--mount', f'type=bind,src={config},dst=/ircd/ircd.yaml,readonly',
                    IMAGE, 'run'])
            address = subprocess.check_output(['rtk', 'proxy', 'docker', 'port', name, '6667/tcp'], text=True).strip()
            host, port = address.rsplit(':', 1)
            deadline = time.monotonic() + 10
            while True:
                try:
                    with socket.create_connection((host, int(port)), timeout=1):
                        break
                except OSError:
                    if time.monotonic() > deadline:
                        raise
                    time.sleep(.1)
            addresses.append(address)
        receipt['servers'] = addresses
        environment = dict(os.environ, MIOPUNCH_IRC_SERVER=addresses[0],
                           MIOPUNCH_IRC_LIMITED_SERVER=addresses[1])
        for name, selector in CASES:
            execute(name, ['cargo', 'test', '--locked', '--test', 'irc_bootstrap', selector,
                           '--', '--exact', '--nocapture', '--test-threads=1'], environment)
        execute('build-worker', ['cargo', 'test', '--locked', '--test', 'irc_bootstrap',
                                '--no-run', '--message-format=json'])
        executables = [item['executable'] for line in (output / 'build-worker.log').read_text().splitlines()
                       if line.startswith('{') for item in [json.loads(line)]
                       if item.get('executable') and item.get('target', {}).get('name') == 'irc_bootstrap']
        worker = pathlib.Path(executables[-1]).resolve().relative_to(root)
        execute('cut-image', ['docker', 'build', '--file', 'scripts/irc_lab.Dockerfile',
                             '--tag', 'miopunch45-irc-lab:ergo270', '.'])
        cut_name = f'miopunch45-cut-{time.time_ns()}'
        owned.append(cut_name)
        execute('data-cut', ['docker', 'run', '--rm', '--name', cut_name, '--privileged',
                            '--network', 'none', '--mount', f'type=bind,src={root},dst=/repo,readonly',
                            '--mount', f'type=bind,src={output},dst=/evidence',
                            'miopunch45-irc-lab:ergo270', 'python3', '/repo/scripts/irc_data_cut_lab.py',
                            str(pathlib.Path('/repo') / worker)], timeout=60)
        owned.remove(cut_name)  # --rm completed successfully
        if args.short:
            execute('affected-short', ['python', '-B', 'scripts/check_mqtt_bootstrap_short.py',
                                      '--output', str(output / 'mqtt-short'), '--suite'])
            execute('member-short', ['cargo', 'test', '--locked', '--test', 'member_forward',
                                    '--features', 'member-signaling-test-hooks', '--', '--test-threads=1'])
            execute('all-current-hooks', ['cargo', 'check', '--locked', '--workspace', '--all-targets',
                                         '--features', 'nat-lab-test-hooks,replica-test-hooks,member-signaling-test-hooks'])
        receipt['passed'] = True
    except KeyboardInterrupt:
        receipt['interrupted'] = True
    except (RuntimeError, subprocess.TimeoutExpired, OSError) as error:
        receipt['error'] = str(error)
    finally:
        receipt['cleanup'] = []
        for name in owned:
            with (output / (name + '-server.log')).open('w') as log:
                subprocess.run(['rtk', 'proxy', 'docker', 'logs', name], stdout=log, stderr=subprocess.STDOUT)
            result = subprocess.run(['rtk', 'proxy', 'docker', 'rm', '--force', '--volumes', name], capture_output=True, text=True)
            receipt['cleanup'].append({'owned_container': name, 'exit': result.returncode})
            receipt['passed'] &= result.returncode == 0
        save()
    print(f'{"PASS" if receipt["passed"] else "FAIL"} IRC checks: {output}', flush=True)
    raise SystemExit(0 if receipt['passed'] else 1)


if __name__ == '__main__':
    main()
