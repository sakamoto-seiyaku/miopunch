#!/usr/bin/env python3
"""Build the demo host once; verify and reuse the existing browser SDK/relay."""
import hashlib
import argparse
import json
from pathlib import Path
import shutil
import shlex
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = Path('/tmp/miopunch-client-demo-20261021')
sys.path.insert(0, str(ROOT / 'scripts'))
import cargo_cache as cache
from pin_artifact import pin_artifact, sha256

BROWSER_BUILD = Path('/tmp/miopunch-m103-20261019/browser-final')


def verify_map(entries):
    failed = [str(path) for path, digest in entries.items()
              if not Path(path).is_file() or sha256(Path(path)) != digest]
    if failed:
        raise RuntimeError('source/lock/artifact precondition failed: ' + ', '.join(failed[:12]))


def browser_pin():
    receipt = json.loads((BROWSER_BUILD / 'receipt.json').read_text())
    inputs = json.loads((BROWSER_BUILD / 'selected-dependency-inputs.json').read_text())
    if receipt['exit'] != 0 or not receipt['qualifiedForFrozenRun'] or not receipt['sourceFrozenDuringBuild']:
        raise RuntimeError('browser originating build is unqualified')
    verify_map(inputs['inputs'])
    verify_map(receipt['artifacts'])
    context = json.loads((BROWSER_BUILD.parent / 'browser-final-context.json').read_text())
    verify_map({ROOT / path: context['sources'][path] for path in
                ('Cargo.lock', 'Cargo.toml', 'browser-probe/Cargo.toml', '.cargo/config.toml')})
    return dict(originatingReceipt=str(BROWSER_BUILD / 'receipt.json'),
                receiptSha256=sha256(BROWSER_BUILD / 'receipt.json'),
                selectedInputManifestSha256=sha256(BROWSER_BUILD / 'selected-dependency-inputs.json'),
                verifiedInputCount=len(inputs['inputs']), artifacts=receipt['artifacts'], pkg=str(BROWSER_BUILD / 'pkg'))


def source_map():
    names = subprocess.check_output(['rtk', 'proxy', 'rg', '--files', '--hidden',
        'src', 'vendor', 'browser-probe/src', '-g', '*.rs', '-g', '*.toml', '-g', 'build.rs', '-g', '*.js'], cwd=ROOT, text=True).splitlines()
    names += ['Cargo.lock', 'Cargo.toml', '.cargo/config.toml', 'browser-probe/Cargo.toml',
              'scripts/cargo_cache.py', 'scripts/pin_artifact.py', 'demo/build.py']
    return {str(ROOT / name): sha256(ROOT / name) for name in sorted(set(names))}


def build(rebuild_relay=False):
    browser = browser_pin()
    relay_receipt = Path('/tmp/miopunch-inner-web-prototype-20261021/builds/1792554324203992859-79d385b97db3/receipt.json')
    relay = json.loads(relay_receipt.read_text())
    relay_path = Path(relay['programs']['browser-probe-native'])
    relay_record = dict(path=str(relay_path), sha256=relay['artifacts'][str(relay_path)], originatingReceipt=str(relay_receipt))
    latest = OUTPUT / 'artifacts.json'
    if latest.exists():
        candidate = json.loads(latest.read_text())['relay']
        if candidate.get('selectedInputManifest'):
            relay_record = candidate
            relay_path = Path(candidate['path'])
            relay_receipt = Path(candidate['originatingReceipt'])
    verify_map({str(relay_path): relay_record['sha256']})
    relay_graph = json.loads((relay_receipt.parent / 'graph.json').read_text())
    # The relay's selected source tree has not changed. Ignore the unrelated
    # throwaway binary/driver when checking its originating receipt.
    relay_inputs = {path: digest for path, digest in relay_graph['sources'].items()
                    if path not in (str(ROOT / 'src/bin/prototype_inner_web.rs'), str(ROOT / 'browser-probe/prototype-web/run.py'))}
    if relay_record.get('selectedInputManifest'):
        selected = Path(relay_record['selectedInputManifest'])
        verify_map({str(selected): relay_record['selectedInputManifestSHA256']})
        relay_inputs = json.loads(selected.read_text())
    relay_delta = {path: dict(origin=digest, current=sha256(Path(path)) if Path(path).is_file() else None)
                   for path, digest in relay_inputs.items()
                   if not Path(path).is_file() or sha256(Path(path)) != digest}
    if not rebuild_relay:
        verify_map(relay_inputs)
    sources = source_map()
    for path in [* (ROOT / 'demo/host').glob('*.rs'), ROOT / 'demo/build.py']:
        sources[str(path)] = sha256(path)
    toolchain = subprocess.check_output(['rtk', 'proxy', 'rustc', '--version', '--verbose'], cwd=ROOT, text=True)
    if not toolchain.startswith('rustc 1.98.1 '):
        raise RuntimeError('demo requires the inventoried Rust toolchain')
    targets = ['miopunch_demo', 'browser-probe-native'] if rebuild_relay else ['miopunch_demo']
    graph = dict(sources=sources, toolchain=toolchain, platform='x86_64-unknown-linux-gnu', profile='dev (committed settings)', features=['rtc-probe'], targets=targets)

    def selected_relay(receipt):
        if 'browser-probe-native' not in receipt.get('programs', {}):
            return relay_record
        pin = receipt['programs']['browser-probe-native']
        selected = Path(receipt['selectedInputManifests']['browser-probe-native'])
        verify_map({str(selected): receipt['selectedInputManifestHashes']['browser-probe-native']})
        verify_map(json.loads(selected.read_text()))
        return dict(path=pin, sha256=receipt['artifacts'][pin], originatingReceipt=receipt['originatingReceipt'],
                    selectedInputManifest=str(selected), selectedInputManifestSHA256=sha256(selected))
    compiler = dict(graph, sources={k:v for k,v in sources.items() if k.endswith(('.rs','.toml','.lock'))})
    compiler.pop('targets')
    graph_id = hashlib.sha256(json.dumps(compiler, sort_keys=True).encode()).hexdigest()
    for path in sorted((OUTPUT / 'builds').glob('*/receipt.json')):
        receipt = json.loads(path.read_text())
        if receipt.get('graphId') == graph_id and receipt.get('exit') == 0 and all(target in receipt.get('programs', {}) for target in targets):
            verify_map(receipt['artifacts'])
            receipt['reused'] = True
            return dict(browser=browser, native=receipt, relay=selected_relay(receipt))
    directory = OUTPUT / 'builds' / (str(time.time_ns()) + '-' + graph_id[:12])
    directory.mkdir(parents=True)
    (directory / 'graph.json').write_text(json.dumps(graph, indent=2))
    plan = dict(graphId=graph_id, graph=str(directory / 'graph.json'), browserReuse=browser, relayReuse=None if rebuild_relay else relay_record,
                relayRebuild=bool(rebuild_relay), relaySourceDelta=relay_delta, budget=dict(cacheAndNewPinsBytes=2*1024**3, artifactStoreBytes=32*1024**3), disk=cache.disk_state(ROOT / 'target'))
    (directory / 'plan.json').write_text(json.dumps(plan, indent=2))
    print(json.dumps(plan), flush=True)
    args=['build','--locked','--offline','--features','rtc-probe','--jobs','2']
    for target in targets:
        args += ['--bin', target]
    command=cache.cargo_command(args,ROOT/'target')
    with cache.cache_lease(ROOT/'target') as before:
        verify_map(sources);browser_pin();verify_map({str(relay_path): relay_record['sha256']})
        if not rebuild_relay:
            verify_map(relay_inputs)
        for relative in ['src/bin/miopunch_demo.rs','demo/host/services.rs','demo/build.py','Cargo.toml','Cargo.lock','.cargo/config.toml','scripts/cargo_cache.py','scripts/pin_artifact.py']:
            dest=directory/'source-snapshots'/relative;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/relative,dest)
        with (directory/'cargo.jsonl').open('w') as out,(directory/'cargo.stderr.log').open('w') as err:
            result=subprocess.run(command,cwd=ROOT,env=cache.build_environment(ROOT/'target'),stdout=out,stderr=err)
        messages=[json.loads(line) for line in (directory/'cargo.jsonl').read_text().splitlines() if line.startswith('{')]
        artifacts=[m for m in messages if m.get('reason')=='compiler-artifact']
        receipt=dict(graphId=graph_id,originatingReceipt=str(directory/'receipt.json'),command=command,exit=result.returncode,freshArtifacts=sum(m['fresh'] for m in artifacts),rebuiltArtifacts=sum(not m['fresh'] for m in artifacts),rebuiltTargets=[dict(package=m['package_id'],name=m['target']['name']) for m in artifacts if not m['fresh']],diskBefore=before,diskAfter=cache.disk_state(ROOT/'target'))
        (directory/'receipt.json').write_text(json.dumps(receipt,indent=2))
        if result.returncode:
            raise RuntimeError('build failed; see '+str(directory/'cargo.jsonl'))
        verify_map(sources)
        programs = {}; hashes = {}; manifests = {}; manifest_hashes = {}
        for target in targets:
            candidates=[m for m in artifacts if m['target']['name']==target and m.get('executable')]
            if len(candidates)!=1:raise RuntimeError('expected one executable for '+target)
            program=Path(candidates[0]['executable'])
            pin=pin_artifact(program,ROOT/'.artifacts/runs/client-demo'/directory.name/target)
            depfile = program.with_suffix('.d')
            shutil.copy2(depfile,directory/(target+'.d'))
            line = depfile.read_text().replace('\\\n', '').splitlines()[0]
            dependencies = shlex.split(line.split(': ', 1)[1])
            inputs = {}
            for value in dependencies + ['Cargo.lock', 'Cargo.toml', '.cargo/config.toml']:
                path = Path(value)
                if not path.is_absolute(): path = ROOT / path
                if not path.is_file(): raise RuntimeError('selected input missing: '+str(path))
                inputs[str(path)] = sha256(path)
            selected = directory/(target+'-selected-inputs.json')
            selected.write_text(json.dumps(inputs,indent=2))
            programs[target]=str(pin); hashes[str(pin)]=sha256(pin)
            manifests[target]=str(selected); manifest_hashes[target]=sha256(selected)
        receipt.update(programs=programs,artifacts=hashes,selectedInputManifests=manifests,
                       selectedInputManifestHashes=manifest_hashes,sourceUnchangedAfterBuild=True)
        (directory/'receipt.json').write_text(json.dumps(receipt,indent=2))
    unexpected=[m for m in receipt['rebuiltTargets'] if not m['package'].startswith('path+file://'+str(ROOT)+'#')]
    if unexpected:raise RuntimeError('review unexpected dependency rebuilds before runtime: '+json.dumps(unexpected))
    return dict(browser=browser,native=receipt,relay=selected_relay(receipt))


if __name__=='__main__':
    OUTPUT.mkdir(parents=True,exist_ok=True)
    parser = argparse.ArgumentParser()
    parser.add_argument('--rebuild-relay', action='store_true', help='Explicitly compile both native programs after their shared SDK sources change')
    result=build(parser.parse_args().rebuild_relay)
    (OUTPUT/'artifacts.json').write_text(json.dumps(result,indent=2))
    print(json.dumps({k:v for k,v in result['native'].items() if k in ('originatingReceipt','freshArtifacts','rebuiltArtifacts','rebuiltTargets','reused')}))
