import hashlib, json, os, re, sys
from pathlib import Path
os.umask(0o077)
ROOT = Path('/home/js/Git/miopunch')
P = Path(sys.argv[1])
OUT = Path(sys.argv[2])
OUT.mkdir(parents=True, exist_ok=True)
source = (ROOT/'docs/evidence/irc-base64-saferate-2026-10-11/probe-sources/verify-final-evidence.py').read_text()
# Reuse the prior full-story assertions and scalar allowlist, with explicit
# current input/output directories. Stop before the old artifact writer.
source = source[source.index('sha=lambda'):source.index("(OUT/'final-public-story.json').write_text")]
exec(compile(source, '<prior-full-sdk-assertions>', 'exec'))
assert r['request']['elapsedMs'] < r['sdkCallerBudgetMs']
assert r['base64WireQualified'] and r['wireObservation']['invalidBase64'] == 0
gate = r['request']['diagnostics']['gate']
stats = gate['stats']
cuts = {}
for direction in ('tx', 'rx'):
    match = re.search('frame_'+direction+r': FrameStats \{([^}]+)\}', stats)
    assert match
    cuts[direction] = {k: int(v) for k,v in re.findall(r'(\w+): (\d+)', match[1])}
value.update(
    wireObservation=r['wireObservation'],
    gateSnapshot={
        'cut': 'BrowserGate after the last confirmation represented by this implementation; older legacy cuts precede the unified coordinator',
        'legacyAgreementMs': gate['agreement_elapsed_ms'],
        'confirmationMs': gate.get('confirmation_elapsed_ms'),
        'agreementMode': gate.get('agreement_mode', 'legacy-stage-plus-unified'),
        'legacyAgreementExecuted': gate.get('legacy_agreement_executed', True),
        'frameCounters': cuts,
        'meaning': 'Transmission counters include retransmissions; these are not distinct CID counts.'
    },
    defaultProfileQualified=False,
    budgetQualification='The measured caller and attempt budgets are explicit. Assembly8s, queue30s and fallback600ms remain experimental overrides; this is not default-profile certification.'
)
if 'confirmation' in P.name:
    assert gate['agreement_elapsed_ms'] == 0
    assert gate['legacy_agreement_executed'] is False
    assert gate['agreement_mode'] == 'unified-session/1'
    assert gate['confirmation_elapsed_ms'] >= 0
    assert gate['coordinator']['dataReleased'] is False
end = r['request']['diagnostics']['request']
end_frames = {}
end_datagrams = {}
for direction in ('tx','rx'):
    match = re.search('frame_'+direction+r': FrameStats \{([^}]+)\}', end['stats'])
    assert match
    end_frames[direction] = {k:int(v) for k,v in re.findall(r'(\w+): (\d+)',match[1])}
    match = re.search('udp_'+direction+r': UdpStats \{([^}]+)\}',end['stats'])
    assert match
    end_datagrams[direction] = {k:int(v) for k,v in re.findall(r'(\w+): (\d+)',match[1])}
value['requestEndSnapshot'] = {
    'cut':'After the actual full initial admission request returns AwaitingApproval, for every implementation.',
    'logicalQuicDatagrams':end_datagrams,
    'frameCounters':end_frames,
    'note':'Noq UdpStats are logical QUIC datagram counters here; physical transport is protected IRC. Frame transmissions include repeats.'
}
value['requestTiming'] = {
    'handshakeAndHooksAndCoordinatorMs':end['handshake_ms'],
    'cumulativeExchangeMs':end['exchange_ms'],
    'finalAdmissionExchangeRemainderMs':end['exchange_ms']-end['handshake_ms'],
    'attemptMs':end['attempt_elapsed_ms'],
    'remainingCallerMs':end['remaining_caller_ms'],
    'note':'Cumulative SDK milestones, not independently measured pure TLS or server latency. Gate confirmation timing is nested.'
}
(OUT/(P.name+'.json')).write_text(json.dumps(value,indent=2)+'\n')
print(json.dumps({'case':P.name,'qualified':True,'requestMs':value['initialIrcRequestMs'],'wholeWallMs':value['wholeWallMs'],'legacyAgreementMs':gate['agreement_elapsed_ms'],'confirmationMs':gate.get('confirmation_elapsed_ms'),'wireSent':r['wireObservation']['sentFragments'],'wireReceived':r['wireObservation']['receivedFragments']}))
