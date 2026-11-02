#!/usr/bin/env python3
"""Package verified desktop/headless pins without invoking a compiler."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import shutil
import stat
import tarfile
import zipfile

from pin_artifact import pin_artifact

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    with Path(path).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def current_build(path, expected):
    receipt = json.loads(path.read_text())
    assert receipt['exit'] == 0 and receipt['sha256'] == expected
    assert sha(receipt['pin']) == expected
    selected = json.loads((path.parent / 'selected-dependency-inputs.json').read_text())
    for name, digest in selected['selectedRepositoryInputs'].items():
        assert sha(ROOT / name) == digest, ('changed selected source', name)
    for directory, names in selected.get('selectedDirectories', {}).items():
        actual = sorted(str(p.relative_to(ROOT)) for p in (ROOT / directory).rglob('*') if p.is_file())
        assert actual == names, ('changed selected directory', directory)
    return receipt, selected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    args = parser.parse_args()
    plan = json.loads(args.plan.read_text())
    assert plan['cargo_invocations'] == 0
    assert plan['packager_sha256'] == sha(__file__)
    assert plan['pin_helper_sha256'] == sha(ROOT / 'scripts/pin_artifact.py')
    assert plan['store_budget_bytes'] <= 39 * 1024**3
    assert plan['compressed_budget_bytes'] <= 200 * 1024**2
    output = Path(plan['output'])
    assert not output.exists()
    prepared = []
    # Validate the entire batch before producing any archive.
    for package in plan['packages']:
        build_path = Path(package['build_receipt'])
        build, selected = current_build(build_path, package['source_sha256'])
        members = {}
        for item in package['files']:
            source = Path(item['source'])
            name = item['name']
            assert name and not Path(name).is_absolute() and '..' not in Path(name).parts
            assert name not in members and stat.S_ISREG(source.lstat().st_mode)
            assert sha(source) == item['sha256'], name
            members[name] = dict(item, bytes=source.stat().st_size)
        release = {
            'kind': package['kind'], 'platform': build['platform'], 'profile': build['profile'],
            'sdk_source_sha256': build['sha256'], 'build_receipt_sha256': sha(build_path),
            'selected_repository_inputs': selected['selectedRepositoryInputs'],
            'compiler_invoked_by_packaging': False,
            'files': {name: {k: item[k] for k in ['sha256', 'bytes', 'mode']} for name, item in members.items()},
            'limits': package['limits'],
        }
        extras = {'release.json': (json.dumps(release, indent=2) + '\n').encode(),
                  'README.txt': package['readme'].encode()}
        assert not members.keys() & extras.keys()
        prepared.append((package, members, extras))
    assert shutil.disk_usage(ROOT).free - 2 * plan['compressed_budget_bytes'] > 10 * 1024**3
    output.mkdir(mode=0o700)
    results = []
    total = 0
    for package, members, extras in prepared:
        archive = output / package['archive']
        assert archive.parent == output
        if archive.suffix == '.zip':
            with zipfile.ZipFile(archive, 'x', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as bundle:
                for name, item in sorted(members.items()):
                    info = zipfile.ZipInfo(name)
                    info.compress_type = zipfile.ZIP_DEFLATED
                    info.external_attr = (stat.S_IFREG | item['mode']) << 16
                    with bundle.open(info, 'w') as target, Path(item['source']).open('rb') as source:
                        shutil.copyfileobj(source, target)
                for name, data in sorted(extras.items()):
                    bundle.writestr(name, data)
        else:
            assert archive.name.endswith('.tar.gz')
            with archive.open('xb') as raw, gzip.GzipFile(fileobj=raw, mode='wb', mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode='w|') as bundle:
                    for name, item in sorted(members.items()):
                        info = tarfile.TarInfo(name)
                        info.size, info.mode = item['bytes'], item['mode']
                        with Path(item['source']).open('rb') as source:
                            bundle.addfile(info, source)
                    for name, data in sorted(extras.items()):
                        info = tarfile.TarInfo(name)
                        info.size, info.mode = len(data), 0o644
                        bundle.addfile(info, io.BytesIO(data))
        total += archive.stat().st_size
        assert total <= plan['compressed_budget_bytes'], 'compressed batch exceeds declared budget'
        expected = {name: item['sha256'] for name, item in members.items()}
        expected.update({name: hashlib.sha256(data).hexdigest() for name, data in extras.items()})
        # Independently read the exported bytes, including every executable.
        if archive.suffix == '.zip':
            with zipfile.ZipFile(archive) as bundle:
                assert sorted(bundle.namelist()) == sorted(expected)
                for name, digest in expected.items():
                    with bundle.open(name) as source:
                        assert hashlib.file_digest(source, 'sha256').hexdigest() == digest, name
        else:
            with tarfile.open(archive, 'r:gz') as bundle:
                entries = bundle.getmembers()
                assert sorted(m.name for m in entries) == sorted(expected)
                assert all(m.isfile() for m in entries)
                for entry in entries:
                    with bundle.extractfile(entry) as source:
                        assert hashlib.file_digest(source, 'sha256').hexdigest() == expected[entry.name], entry.name
                    if entry.name in members:
                        assert entry.mode == members[entry.name]['mode']
        for item in members.values():
            assert sha(item['source']) == item['sha256'], 'input changed while packaging'
        destination = ROOT / '.artifacts/runs' / plan['pin_directory'] / archive.name
        pinned = pin_artifact(archive, destination, max_bytes=plan['store_budget_bytes'])
        result = dict(kind=package['kind'], pin=str(pinned), sha256=sha(pinned), bytes=pinned.stat().st_size,
                      members=expected, build_receipt=package['build_receipt'],
                      source_sha256=package['source_sha256'], compiler_invoked=False)
        results.append(result)
        (output / (package['kind'] + '.json')).write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({k: result[k] for k in ['kind', 'pin', 'sha256', 'bytes']}), flush=True)
    (output / 'result.json').write_text(json.dumps(dict(status='pass', packages=results,
        compressed_bytes=total, plan_sha256=sha(args.plan), cargo_invocations=0), indent=2) + '\n')


if __name__ == '__main__':
    main()
