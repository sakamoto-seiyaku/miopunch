#!/usr/bin/env python3
"""Run the owned Node/listener fault supplement using frozen test artifacts."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time

from check_build_cache import compiled_artifacts

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'scope': 'Owned loopback SDK/generation/disconnect supplement; not public facility or physical-device qualification',
               'public_traffic': False, 'passed': False, 'checks': [], 'artifacts': []}

    def save():
        (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')

    def source_hashes():
        sources = sorted(set(ROOT.glob('src/**/*.rs')) | set(ROOT.glob('vendor/**/*.rs'))
            | set(ROOT.glob('vendor/**/Cargo.toml')) | set(ROOT.glob('tests/**/*.rs'))
            | {p for p in ROOT.glob('tests/fixtures/**/*') if p.is_file()} | {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock',
            ROOT / '.cargo/config.toml', ROOT / 'browser-probe/Cargo.toml',
            ROOT / 'scripts/cargo_cache.py', ROOT / 'scripts/check_build_cache.py',
            ROOT / 'scripts/check_contact_listener.py', ROOT / 'tests/contact_listener.rs',
            ROOT / 'tests/support/contact_broker.rs'})
        return {str(path.relative_to(ROOT)): digest(path) for path in sources}

    def run(name, command, budget, environment=None):
        started = time.monotonic()
        argv = command if command[:2] == ['rtk', 'proxy'] else ['rtk', 'proxy', *command]
        with (output / f'{name}.stdout.log').open('w') as stdout, (output / f'{name}.stderr.log').open('w') as stderr:
            child = subprocess.Popen(argv, cwd=ROOT, stdout=stdout,
                                     stderr=stderr, env=environment, start_new_session=True)
            terminated = forced = False
            try:
                code = child.wait(timeout=budget)
            except (subprocess.TimeoutExpired, KeyboardInterrupt):
                terminated = True
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    forced = True
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()
                code = child.returncode
            receipt['checks'].append({'name': name, 'argv': argv,
                'exit': code, 'guard_seconds': budget, 'seconds': time.monotonic() - started,
                'terminated_by_guard': terminated, 'forced_kill': forced})
            save()
            if code != 0 and environment is None:
                raise RuntimeError(f'{name} exit {code}; raw logs retained')
            return code

    try:
        receipt['source_head'] = subprocess.check_output(['rtk', 'proxy', 'git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        receipt['source_before'] = source_hashes()
        artifacts = {}
        with compiled_artifacts(output, 'build',
                ['test', '--locked', '--offline', '-j', '2', '--features', 'rtc-probe',
                 '--lib', '--test', 'contact_listener', '--no-run'],
                source_hashes, receipt['source_before'], run, 600) as entries:
            receipt['source_after'] = source_hashes()
            receipt['source_changed_during_build'] = []
            for value in entries:
                if value.get('profile', {}).get('test') and value.get('executable'):
                    target = value['target']['name']
                    if target in ['miopunch_native_probe', 'contact_listener']:
                        frozen = output / f'{target}.test-bin'
                        shutil.copy2(value['executable'], frozen)
                        artifacts[target] = frozen
                        receipt['artifacts'].append({'target': target, 'built_path': value['executable'],
                            'frozen_path': str(frozen), 'sha256': digest(frozen), 'bytes': frozen.stat().st_size})
            assert set(artifacts) == {'miopunch_native_probe', 'contact_listener'}, 'test artifacts missing'
            save()
        save()
        run('generation', [str(artifacts['miopunch_native_probe']), 'contact_listener::tests::', '--nocapture'], 30)
        run('tls-negative', [str(artifacts['miopunch_native_probe']),
            'mqtt_transport::tls_tests::android_public_mqtt_roots_reject_an_untrusted_server_certificate',
            '--exact', '--nocapture'], 10)
        run('maintained-node', [str(artifacts['contact_listener']), '--nocapture'], 30)
        receipt['passed'] = True
    except (Exception, KeyboardInterrupt) as error:
        receipt['failure'] = str(error) or 'cancelled'
    finally:
        receipt['completed_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
        save()
    print(json.dumps({'passed': receipt['passed'], 'public_traffic': False, 'output': str(output)}))
    raise SystemExit(0 if receipt['passed'] else 1)


if __name__ == '__main__':
    main()
