#!/usr/bin/env python3
"""Package a verified current WASM pin and the static platform entry; no build."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tarfile

from pin_artifact import ROOT, pin_artifact


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['wasm-receipt','plan','output']:
        parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    receipt=args.wasm_receipt.resolve()
    build=json.loads(receipt.read_text())
    plan=json.loads(args.plan.read_text())
    assert build['exit']==0 and build['bindgenExit']==0
    assert sha(receipt)==plan['wasm_receipt_sha256']
    assert hashlib.sha256(json.dumps(build['artifacts'],sort_keys=True).encode()).hexdigest()==build['packageSHA256']
    selected=json.loads(receipt.with_name('selected-dependency-inputs.json').read_text())['selectedRepositoryInputs']
    assert selected and all(sha(ROOT/name)==digest for name,digest in selected.items()),'selected WASM source changed'
    source=ROOT/'platform/browser'
    inputs={str(p.relative_to(ROOT)):sha(p) for p in source.rglob('*') if p.is_file()}
    inputs['scripts/package_browser_host.py']=sha(Path(__file__).resolve())
    inputs['scripts/pin_artifact.py']=sha(ROOT/'scripts/pin_artifact.py')
    assert inputs==plan['entry_inputs']
    used=sum(p.stat().st_blocks*512 for p in (ROOT/'.artifacts/sha256').rglob('*') if p.is_file())
    assert used==plan['allocated_before'] and used+plan['archive_budget']<plan['store_cap']
    for name,digest in build['artifacts'].items():
        assert sha(receipt.parent/'pkg'/name)==digest,name
    output=args.output.resolve()
    output.mkdir(parents=True,exist_ok=False)
    site=output/'site'
    site.mkdir()
    for name,digest in build['artifacts'].items():
        destination=site/'pkg'/name
        destination.parent.mkdir(parents=True,exist_ok=True)
        os.link(receipt.parent/'pkg'/name,destination)
    for path in source.rglob('*'):
        if path.is_file():
            destination=site/path.relative_to(source)
            destination.parent.mkdir(parents=True,exist_ok=True)
            destination.write_bytes(path.read_bytes())
            destination.chmod(0o444)
    files={str(p.relative_to(site)):sha(p) for p in site.rglob('*') if p.is_file()}
    release={'entry':'miopunch browser platform host','original_wasm_receipt':str(receipt),'original_wasm_receipt_sha256':sha(receipt),
        'wasm_package_sha256':build['packageSHA256'],'selected_repository_inputs':selected,'entry_inputs':inputs,
        'files':files,'source_current':True,'cargo_invoked':False,'bindgen_invoked':False,'native_proxy':False}
    (site/'release.json').write_text(json.dumps(release,indent=2)+'\n')
    (site/'release.json').chmod(0o444)
    archive=output/'package.tar.gz'
    with tarfile.open(archive,'w:gz') as tar:
        for path in sorted(site.rglob('*')):
            if path.is_file():
                tar.add(path,arcname=str(path.relative_to(site)),recursive=False)
    assert archive.stat().st_size<=plan['archive_budget']
    assert all(sha(ROOT/name)==digest for name,digest in selected.items())
    assert all(sha(ROOT/name)==digest for name,digest in inputs.items())
    pin=pin_artifact(archive,ROOT/'.artifacts/runs'/output.name/'miopunch-browser.tar.gz',max_bytes=plan['store_cap'])
    report={'status':'packaged','pin':str(pin),'sha256':sha(pin),'bytes':pin.stat().st_size,'site':str(site),
        'release_sha256':sha(site/'release.json'),'SDK_reused':True,'SDK_package_sha256':build['packageSHA256'],
        'original_receipt':str(receipt),'selected_current_inputs':len(selected),'entry_inputs':inputs,'runtime_qualified':False}
    (output/'package.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:v for k,v in report.items() if k!='entry_inputs'}))


if __name__=='__main__':main()
