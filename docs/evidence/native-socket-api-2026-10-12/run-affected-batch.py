import json,re,subprocess,sys
from pathlib import Path
HERE=Path(__file__).parent;ROOT=Path('/home/js/Git/miopunch')
names=['legacy_multi_socket','member_path_protocol','unified_connection','unified_admission_story','negotiated_accept_context','tcp_direct_member_preserved','irc_datagram','unified_path_selection','rtc_association','rtc_path_qualification','rtc_invalidation_barrier']
report={'build':'affected-tests-binding-final','cases':{},'passed':False}
for name in names:
 result=subprocess.run(['rtk','proxy','python3',str(HERE/'run-pinned-tests.py'),'affected-tests-binding-final',name,'runtime-binding-'+name,'--','--test-threads=1'],cwd=ROOT,capture_output=True,text=True)
 (HERE/('batch-binding-'+name+'.log')).write_text(result.stdout+result.stderr)
 p=HERE/('runtime-binding-'+name);r=json.loads((p/'receipt.json').read_text()) if (p/'receipt.json').exists() else {'exit':result.returncode,'qualified':False}
 matches=re.findall(r'test result: (\w+)\. (\d+) passed; (\d+) failed;', (p/'stdout.log').read_text() if (p/'stdout.log').exists() else '')
 r['results']=matches;report['cases'][name]=r
 print(json.dumps({'name':name,'qualified':r['qualified'],'results':matches}),flush=True)
 (HERE/'affected-runtime-batch-binding.json').write_text(json.dumps(report,indent=2)+'\n')
 if result.returncode:raise SystemExit(result.returncode)
report['passed']=all(v['qualified'] for v in report['cases'].values());report['totalPassed']=sum(int(x[1]) for v in report['cases'].values() for x in v['results']);(HERE/'affected-runtime-batch-binding.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'passed':report['passed'],'totalPassed':report['totalPassed']}));raise SystemExit(0 if report['passed'] else 1)
