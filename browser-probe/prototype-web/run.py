#!/usr/bin/env python3
"""THROWAWAY: build once, reuse pinned SDKs, run the real inner-Web prototype."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUTPUT = Path('/tmp/miopunch-inner-web-prototype-20261021')
BROWSER_BUILD = Path('/tmp/miopunch-m103-20261019/browser-final')
sys.path.insert(0, str(ROOT / 'scripts'))
import cargo_cache as cache
from pin_artifact import pin_artifact, sha256


def verify_map(entries):
    failures = [str(path) for path, digest in entries.items()
                if not Path(path).is_file() or sha256(Path(path)) != digest]
    if failures:
        raise RuntimeError('source/lock/artifact precondition failed: ' + ', '.join(failures[:12]))


def browser_pin():
    receipt = json.loads((BROWSER_BUILD / 'receipt.json').read_text())
    inputs = json.loads((BROWSER_BUILD / 'selected-dependency-inputs.json').read_text())
    if receipt['exit'] != 0 or not receipt['qualifiedForFrozenRun'] or not receipt['sourceFrozenDuringBuild']:
        raise RuntimeError('browser originating build is unqualified')
    verify_map(inputs['inputs'])
    verify_map(receipt['artifacts'])
    # Cargo.lock and both package manifests belong to this frozen SDK graph.
    context = json.loads((BROWSER_BUILD.parent / 'browser-final-context.json').read_text())
    verify_map({ROOT / path: context['sources'][path] for path in
                ('Cargo.lock', 'Cargo.toml', 'browser-probe/Cargo.toml', '.cargo/config.toml')})
    return {'originatingReceipt': str(BROWSER_BUILD / 'receipt.json'),
            'receiptSha256': sha256(BROWSER_BUILD / 'receipt.json'),
            'selectedInputManifestSha256': sha256(BROWSER_BUILD / 'selected-dependency-inputs.json'),
            'verifiedInputCount': len(inputs['inputs']), 'artifacts': receipt['artifacts'],
            'pkg': str(BROWSER_BUILD / 'pkg')}


def source_map():
    listed = subprocess.check_output(['rtk', 'proxy', 'rg', '--files', '--hidden',
        'src', 'vendor', 'browser-probe/src', '-g', '*.rs', '-g', '*.toml', '-g', 'build.rs', '-g', '*.js'], cwd=ROOT, text=True).splitlines()
    listed += ['Cargo.lock', 'Cargo.toml', '.cargo/config.toml', 'browser-probe/Cargo.toml',
               'scripts/cargo_cache.py', 'scripts/pin_artifact.py', 'browser-probe/prototype-web/run.py']
    return {str(ROOT / name): sha256(ROOT / name) for name in sorted(set(listed))}


def native_pin(output, browser):
    sources = source_map()
    toolchain = subprocess.check_output(['rtk', 'proxy', 'rustc', '--version', '--verbose'], cwd=ROOT, text=True)
    graph = {'sources': sources, 'toolchain': toolchain, 'platform': 'x86_64-unknown-linux-gnu',
             'profile': 'dev (repository settings)', 'features': ['rtc-probe'],
             'targets': ['prototype_inner_web', 'browser-probe-native']}
    def compiler_graph(value):
        # Build tooling is frozen for provenance, but changing the pin runner
        # does not change the Rust graph or justify compiling it again.
        value = dict(value)
        value['sources'] = {path: digest for path, digest in value['sources'].items()
                            if path not in {str(ROOT / 'scripts/cargo_cache.py'),
                                            str(ROOT / 'scripts/pin_artifact.py'),
                                            str(ROOT / 'browser-probe/prototype-web/run.py')}}
        return value
    graph_id = hashlib.sha256(json.dumps(compiler_graph(graph), sort_keys=True).encode()).hexdigest()
    for receipt_path in sorted((output / 'builds').glob('*/receipt.json')):
        receipt = json.loads(receipt_path.read_text())
        previous_graph = json.loads((receipt_path.parent / 'graph.json').read_text())
        if (receipt.get('exit') == 0 and receipt.get('artifacts') and
                compiler_graph(previous_graph) == compiler_graph(graph)):
            verify_map(receipt['artifacts'])
            return {'originatingReceipt': str(receipt_path), 'graphId': graph_id, 'reused': True,
                    'programs': receipt['programs'], 'artifacts': receipt['artifacts']}
    build = output / 'builds' / f'{time.time_ns()}-{graph_id[:12]}'
    build.mkdir(parents=True)
    (build / 'graph.json').write_text(json.dumps(graph, indent=2) + '\n')
    plan = {'nativeGraph': str(build / 'graph.json'), 'browserReuse': browser,
            'diskBudget': {'incrementalCacheAndPinsBytes': 2 * 1024**3, 'artifactStoreMaxBytes': 32 * 1024**3},
            'baselineDisk': cache.disk_state(ROOT / 'target')}
    (build / 'preconditions.json').write_text(json.dumps(plan, indent=2) + '\n')
    arguments = ['build', '--locked', '--offline', '--features', 'rtc-probe', '--jobs', '2',
                 '--bin', 'prototype_inner_web', '--bin', 'browser-probe-native']
    report = {'graphId': graph_id, 'command': cache.cargo_command(arguments, ROOT / 'target'), 'exit': None}
    with cache.cache_lease(ROOT / 'target') as before:
        verify_map(sources)
        verify_map(browser['artifacts'])
        browser_pin()  # Abort before Cargo if reused selected source inputs changed.
        snapshots = build / 'source-snapshots'
        for relative in ['src/bin/prototype_inner_web.rs', 'browser-probe/prototype-web/run.py',
                         'Cargo.lock', 'Cargo.toml', '.cargo/config.toml', 'scripts/cargo_cache.py', 'scripts/pin_artifact.py']:
            destination = snapshots / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / relative, destination)
        started = time.monotonic()
        with (build / 'cargo.jsonl').open('w') as stdout, (build / 'cargo.stderr.log').open('w') as stderr:
            result = subprocess.run(report['command'], cwd=ROOT, env=cache.build_environment(ROOT / 'target'), stdout=stdout, stderr=stderr)
        entries = [json.loads(line) for line in (build / 'cargo.jsonl').read_text().splitlines() if line.strip().startswith('{')]
        artifacts = [entry for entry in entries if entry.get('reason') == 'compiler-artifact']
        report.update(exit=result.returncode, seconds=time.monotonic() - started,
                      freshArtifacts=sum(entry['fresh'] for entry in artifacts),
                      rebuiltArtifacts=sum(not entry['fresh'] for entry in artifacts),
                      rebuiltTargets=[{'package': entry['package_id'], 'name': entry['target']['name']} for entry in artifacts if not entry['fresh']],
                      diskBefore=before, diskAfter=cache.disk_state(ROOT / 'target'))
        (build / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
        if result.returncode:
            raise RuntimeError(f'build failed; see {build / "cargo.stderr.log"}')
        verify_map(sources)
        report.update(programs={}, artifacts={})
        for name in graph['targets']:
            chosen = [entry for entry in artifacts if entry['target']['name'] == name and entry.get('executable')]
            if len(chosen) != 1:
                raise RuntimeError(f'expected one selected executable for {name}')
            program = Path(chosen[0]['executable'])
            pin = pin_artifact(program, ROOT / '.artifacts/runs/inner-web-prototype' / build.name / name)
            report['programs'][name] = str(pin)
            report['artifacts'][str(pin)] = sha256(pin)
            depfile = program.with_suffix('.d')
            if depfile.exists():
                shutil.copy2(depfile, build / (name + '.d'))
        report['sourceUnchangedAfterBuild'] = True
        report['diskAfterPin'] = cache.disk_state(ROOT / 'target')
        (build / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'buildReceipt': str(build / 'receipt.json'), 'fresh': report['freshArtifacts'],
                      'rebuilt': report['rebuiltArtifacts'], 'rebuiltTargets': report['rebuiltTargets']}), flush=True)
    unexpected = [entry for entry in report['rebuiltTargets'] if not entry['package'].startswith('path+file://' + str(ROOT) + '#')]
    if unexpected:
        raise RuntimeError('dependency rebuilds require review before runtime: ' + json.dumps(unexpected))
    return {'originatingReceipt': str(build / 'receipt.json'), 'graphId': graph_id, 'reused': False,
            'programs': report['programs'], 'artifacts': report['artifacts']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='automatically exercise the real browser flow and retain evidence')
    parser.add_argument('--output', type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    browser = browser_pin()
    native = native_pin(output, browser)
    run = output / 'runs' / str(time.time_ns())
    run.mkdir(parents=True)
    configuration = {'browser': browser, 'native': native, 'output': str(run)}
    (run / 'inputs.json').write_text(json.dumps(configuration, indent=2) + '\n')
    command = ['rtk', 'proxy', 'node', str(ROOT / 'browser-probe/harness/prototype-inner-web.mjs'), '--inputs', str(run / 'inputs.json')]
    if args.check:
        command.append('--check')
    raise SystemExit(subprocess.call(command, cwd=ROOT))


if __name__ == '__main__':
    main()
