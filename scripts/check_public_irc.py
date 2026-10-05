#!/usr/bin/env python3
"""Explicit public TLS/WSS IRC qualification; never installs a default service."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--facility', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--wasm-bindgen', default='/tmp/miopunch-wasm-tools/bin/wasm-bindgen')
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
               'passed': False, 'available': False, 'checks': [], 'public_traffic': True,
               'protocol_budget_seconds': 60, 'cleanup_reserve_seconds': 10,
               'caller_budget_ms': 12000, 'stage_budget_ms': 5000}

    def save():
        (output / 'results.json').write_text(json.dumps(receipt, indent=2) + '\n')

    def run(name, command, budget):
        started = time.monotonic()
        with (output / f'{name}.log').open('w') as log:
            child = subprocess.Popen(['rtk', 'proxy', *command], cwd=ROOT, stdout=log,
                                     stderr=subprocess.STDOUT, start_new_session=True)
            forced = False
            try:
                code = child.wait(timeout=budget)
            except (subprocess.TimeoutExpired, KeyboardInterrupt):
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    forced = True
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()
                code = child.returncode
            receipt['checks'].append({'name': name, 'argv': ['rtk', 'proxy', *command],
                                      'exit': code, 'forced_kill': forced,
                                      'budget_seconds': budget, 'seconds': time.monotonic() - started})
            save()
            if code != 0:
                raise RuntimeError(f'{name} exit {code}; see its log')

    try:
        # Source identity is captured before builds, including the current uncommitted shared work.
        sources = sorted(set(ROOT.glob('src/**/*.rs')) | set(ROOT.glob('browser-probe/src/**/*.rs')) |
                         set(ROOT.glob('browser-probe/src/**/*.js')) |
                         {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'browser-probe/Cargo.toml',
                          Path(__file__).resolve(), ROOT / 'browser-probe/harness/run-public-irc-restricted.mjs',
                          ROOT / 'browser-probe/harness/public-irc-receipt.mjs',
                          ROOT / 'browser-probe/harness/owned-irc-fixture.mjs'})
        receipt['source_sha256'] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
        receipt['source_head'] = subprocess.check_output(['rtk', 'proxy', 'git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        if not args.skip_build:
            run('native-build', ['cargo', 'build', '--locked', '--offline', '-j', '2', '--bin', 'browser_irc_restricted_native'], 600)
            run('wasm-build', ['cargo', 'build', '--locked', '--offline', '-j', '2', '-p', 'miopunch-browser-probe', '--target', 'wasm32-unknown-unknown'], 600)
            run('wasm-bindgen', [args.wasm_bindgen, '--target', 'web', '--out-dir', 'browser-probe/pkg', 'target/wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm'], 60)
        receipt['artifact_sha256'] = {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in [
            'target/debug/browser_irc_restricted_native', 'browser-probe/pkg/miopunch_browser_probe.js',
            'browser-probe/pkg/miopunch_browser_probe_bg.wasm']}
        receipt['source_after_build_sha256'] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
        receipt['source_changed_during_build'] = [name for name, digest in receipt['source_sha256'].items()
                                                if receipt['source_after_build_sha256'].get(name) != digest]
        with tempfile.TemporaryDirectory(prefix='miopunch-public-irc-') as fixture:
            run('public-irc-restricted', ['node', 'browser-probe/harness/run-public-irc-restricted.mjs',
                '--facility', str(args.facility.resolve()), '--output', str(output), '--fixture', fixture], 50)
            result = json.loads((output / 'qualification.json').read_text())
            receipt['passed'] = result['passed']
            receipt['available'] = result['available']
    except (Exception, KeyboardInterrupt) as error:
        receipt['failure'] = str(error) or 'cancelled'
    finally:
        if 'fixture' in locals():
            receipt['private_fixture_removed'] = not Path(fixture).exists()
        receipt['completed_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
        save()
    print(json.dumps({'passed': receipt['passed'], 'available': receipt['available'], 'output': str(output)}))
    raise SystemExit(0 if receipt['passed'] else 1)


if __name__ == '__main__':
    main()
