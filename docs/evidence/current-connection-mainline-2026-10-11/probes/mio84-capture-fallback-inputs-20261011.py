"""Retain the actual compiler inputs of the just-run, frozen fallback test."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
ROOT = Path('/home/js/Git/miopunch')
HERE = Path('/tmp/mio84-current-integration-20261011')
OUT = HERE / 'entry-fallback'
context = json.loads((HERE / 'build-context.json').read_text())
utility = Path('/tmp/mio81-final-peer-assertion-sdk-root-20261007/build-current-sdk.py')
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert sha(utility) == context['utilitySha256']
spec = importlib.util.spec_from_file_location('m84_fallback_capture', utility)
sdk = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = sdk
spec.loader.exec_module(sdk)
raw = json.loads((OUT / 'receipt.json').read_text())
cache = json.loads((OUT / 'build.cache.json').read_text())
assert raw['passed'] and raw['commandExit'] == 0 and cache['exit'] == 0
assert cache['artifactPinningFinishedUnderLease']
assert raw['sourcesBeforeBuild'] == raw['sourcesAfterBuild'] == raw['sourcesAfterRun']
assert raw['sourcesFrozenDuringBuild'] and raw['sourcesFrozenDuringRun'] and raw['artifactFrozenDuringRun']
assert sdk.current_sources() == context['sources']
assert not (OUT / 'selected-dependency-inputs.json').exists()
artifacts = sdk.parse_cargo_artifacts(OUT / 'build.stdout.log')
selected = sdk.capture_inputs(artifacts, ROOT / 'target', OUT, context['sources'])
sdk.verify_captured_graph(selected, OUT, context['sources'])
assert sha(raw['artifact']['frozenPath']) == raw['artifact']['sha256']
summary = {'rawReceiptSha256': sha(OUT / 'receipt.json'), 'cacheReceiptSha256': sha(OUT / 'build.cache.json'),
    'selectedManifestSha256': sha(OUT / 'selected-dependency-inputs.json'),
    'actualSelectedInputCount': len(selected['inputs']), 'actualSourceCopyCount': len(selected['copiedRepositoryInputs']),
    'actualDepfileCount': len(selected['depfiles']), 'sourceCopyAndInputHashesCurrent': True,
    'artifactPinnedBeforeRunUnderOriginalLease': True, 'compilerCaptureAfterRun': True,
    'rawReceiptsUnmodified': True, 'freshArtifacts': cache['freshArtifacts'], 'rebuiltArtifacts': cache['rebuiltArtifacts']}
(OUT / 'actual-input-capture.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary))
