#!/usr/bin/env python3
"""Explicit Linux cdylib generation, with a pin taken under the shared cache lease."""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import time
import tomllib

import cargo_cache

ROOT = Path(__file__).resolve().parents[1]


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(path):
    with Path(path).open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()


def sources():
    paths = {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / '.cargo/config.toml',
             Path(__file__).resolve(), ROOT / 'scripts/cargo_cache.py'}
    for directory in ['src', 'browser-probe', 'vendor']:
        paths.update(p for p in (ROOT / directory).rglob('*') if p.is_file()
                     and not {'target', '.git', '__pycache__', 'node_modules'}.intersection(p.parts))
    return {str(p.relative_to(ROOT)): sha(p) for p in sorted(paths) if p.is_file()}


def tools():
    result = {}
    for name in ['cargo', 'rustc']:
        selected = subprocess.check_output(['rtk', 'proxy', 'rustup', 'which', name], text=True).strip()
        stable = subprocess.check_output(['rtk', 'proxy', 'rustup', 'which', '--toolchain', 'stable', name], text=True).strip()
        require(selected == stable, 'use the stable toolchain')
        result[name] = {'path': selected, 'sha256': sha(selected),
                        'version': subprocess.check_output(['rtk', 'proxy', name, '-vV'], text=True)}
    return result


def capture_inputs(artifacts, target, output, before):
    """Retain actual compiler input provenance before releasing the lease."""
    depfiles, inputs = {}, {}
    for row in artifacts:
        for filename in row['filenames']:
            path = Path(filename)
            candidates = [path.with_suffix('.d')]
            if path.name.startswith('lib'):
                candidates.append(path.with_name(path.name[3:]).with_suffix('.d'))
            for depfile in candidates:
                key = str(depfile)
                if not depfile.is_file() or key in depfiles:
                    continue
                depfiles[key] = sha(depfile)
                dest = output / 'depfiles' / depfile.relative_to(target)
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(depfile, dest)
                first = depfile.read_text().replace('\\\n', ' ').splitlines()[0]
                require(': ' in first, 'invalid compiler depfile')
                for item in shlex.split(first.split(': ', 1)[1]):
                    source = Path(item)
                    source = (source if source.is_absolute() else ROOT / source).resolve()
                    if source.is_file():
                        inputs[str(source)] = sha(source)
    copied = {}
    for name, digest in inputs.items():
        path = Path(name)
        if path.is_relative_to(ROOT) and not path.is_relative_to(target):
            relative = str(path.relative_to(ROOT))
            if relative in before:
                require(before[relative] == digest, 'selected source changed')
            dest = output / 'selected-source-copies' / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, dest)
            copied[relative] = digest
    save(output / 'selected-dependency-inputs.json', {'inputs': inputs, 'depfiles': depfiles,
         'copiedRepositoryInputs': copied})
    return len(inputs)


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def prepare(args):
    require(sys.platform == 'linux', 'this entry qualifies Linux ELF output only')
    require(Path.cwd().resolve() == ROOT, 'run from the repository root')
    for package in ['iroh', 'iroh-relay']:
        manifest = ROOT / 'vendor' / package / 'Cargo.toml'
        require(tomllib.loads(manifest.read_text())['lib']['crate-type'] == ['lib'], 'artifact generation invariant failed')
    protected = {}
    for receipt in args.preserve_receipt:
        receipt = receipt.resolve()
        report = json.loads(receipt.read_text())
        require(report['exit'] == 0 and report['qualifiedForFrozenRun'], 'artifact generation invariant failed')
        protected[str(receipt)] = sha(receipt)
        for path, digest in report['artifacts'].items():
            artifact = Path(path)
            require(artifact.is_absolute() and sha(artifact) == digest, 'artifact generation invariant failed')
            protected[path] = digest
    args.output.mkdir(parents=True, exist_ok=False)
    command = ['rustc', '--locked', '--offline', '-p', args.package, '--lib',
               '--crate-type', 'cdylib', '--jobs', str(args.jobs)]
    save(args.output / 'context.json', {'sources': sources(), 'tools': tools(),
         'protectedArtifacts': protected, 'command': command, 'package': args.package,
         'driverSha256': sha(__file__)})


def guard(context):
    require(Path.cwd().resolve() == ROOT, 'artifact generation invariant failed')
    require(sha(__file__) == context['driverSha256'], 'builder changed')
    require(sources() == context['sources'], 'source, lock or configuration changed')
    require(tools() == context['tools'], 'toolchain changed')
    require(all(sha(p) == digest for p, digest in context['protectedArtifacts'].items()), 'protected artifact changed')


def run(output):
    context_path = output / 'context.json'
    context = json.loads(context_path.read_text())
    guard(context)
    require(not (output / 'receipt.json').exists(), 'use a new receipt directory')
    target = cargo_cache.target_directory(ROOT)
    require(target == ROOT / 'target', 'artifact generation invariant failed')
    with cargo_cache.cache_lease(target) as disk:
        guard(context)
        name = context['package'].replace('-', '_')
        # These unversioned outputs can have been overwritten by another graph
        # whose separate fingerprint still says Fresh. Keep their exact bytes,
        # then require Cargo to regenerate this leaf without deleting rlibs or
        # changing flags, profiles, sources or dependency caches.
        previous = {}
        for artifact in [target / 'debug' / ('lib' + name + '.so'),
                         target / 'debug/deps' / ('lib' + name + '.so')]:
            if not artifact.is_file():
                continue
            require(not artifact.is_symlink(), 'unexpected symlink at the cdylib cache output')
            digest = sha(artifact)
            dest = output / 'previous-cache-output' / (digest + '.so')
            dest.parent.mkdir(exist_ok=True)
            if not dest.exists():
                shutil.copy2(artifact, dest)
            require(sha(dest) == digest, 'artifact generation invariant failed')
            previous[str(artifact)] = {'sha256': digest, 'retainedCopy': str(dest)}
        for path, value in previous.items():
            require(sha(path) == value['sha256'], 'artifact generation invariant failed')
            Path(path).unlink()
        save(output / 'previous-cache-output.json', previous)
        guard(context)
        command = cargo_cache.cargo_command(context['command'], target)
        save(output / 'preconditions.json', {'sourcesLocksToolsAndProtectedArtifactsPassedBeforeCargo': True,
             'contextSha256': sha(context_path), 'command': command, 'diskBefore': disk})
        started = time.monotonic()
        with (output / 'cargo.jsonl').open('w') as log, (output / 'cargo.stderr.log').open('w') as err:
            result = subprocess.run(command, cwd=ROOT, env=cargo_cache.build_environment(target), stdout=log, stderr=err)
        report = {'exit': result.returncode, 'cargoSeconds': time.monotonic() - started,
                  'qualifiedForFrozenRun': False, 'artifacts': {}}
        save(output / 'receipt.json', report)
        guard(context)
        records = [json.loads(line) for line in (output / 'cargo.jsonl').read_text().splitlines() if line.strip()]
        artifacts = [row for row in records if row.get('reason') == 'compiler-artifact']
        report.update(freshArtifacts=sum(row['fresh'] for row in artifacts),
                      rebuiltArtifacts=sum(not row['fresh'] for row in artifacts),
                      sourceFrozenDuringBuild=True, protectedArtifactsUnchanged=True)
        if result.returncode == 0:
            selected = [row for row in artifacts if row['target']['name'] == name
                        and any(Path(p).name == 'lib' + name + '.so' for p in row['filenames'])]
            require(selected and all(not row['fresh'] for row in selected), 'requested cdylib was not regenerated')
            candidates = {Path(p) for row in artifacts if row['target']['name'] == name
                          for p in row['filenames'] if Path(p).name == 'lib' + name + '.so'}
            require(candidates, 'Cargo succeeded without the requested cdylib')
            pinned = output / 'pinned'
            pinned.mkdir()
            for artifact in sorted(candidates):
                dest = pinned / artifact.name
                shutil.copy2(artifact, dest)
                require(sha(artifact) == sha(dest), 'artifact generation invariant failed')
                report['artifacts'][str(dest)] = sha(dest)
            # Pin before releasing the lease: an unversioned Cargo output is mutable.
            report['qualifiedForFrozenRun'] = True
            report['pinnedUnderSharedLease'] = True
            report['unversionedLeafRegenerated'] = True
            report['actualSelectedInputCount'] = capture_inputs(artifacts, target, output, context['sources'])
        guard(context)
        report['diskAfter'] = cargo_cache.disk_state(target)
        save(output / 'receipt.json', report)
        print(json.dumps(report))
        return result.returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='operation', required=True)
    prep = sub.add_parser('prepare')
    prep.add_argument('--package', choices=['iroh', 'iroh-relay'], required=True)
    prep.add_argument('--output', type=Path, required=True)
    prep.add_argument('--preserve-receipt', type=Path, action='append', default=[])
    prep.add_argument('--jobs', type=int, choices=range(1, 9), default=2)
    execute = sub.add_parser('run')
    execute.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output = args.output.resolve()
    if args.operation == 'prepare':
        prepare(args)
        return 0
    return run(args.output)


if __name__ == '__main__':
    raise SystemExit(main())
