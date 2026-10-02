#!/usr/bin/env python3
"""Ordered ordinary-member data relay acceptance and affected short regression."""
import argparse
import json
import os
import socket
from pathlib import Path
import re
import subprocess
import sys
import time

CASES = {'first': 'explicitly_enabled_member_relay_carries_a_real_end_to_end_business_session', 'priority': 'disabling_data_relay_stops_new_forwarding_and_the_only_inner_path_fails', 'default': 'ordinary_member_data_service_is_disabled_by_default_and_bootstrap_is_insufficient', 'security': 'b_has_separate_outer_tls_keys_and_cannot_substitute_the_inner_endpoint_identity', 'backup-disabled': 'disabling_member_relay_keeps_the_original_connection_and_stream_on_an_allowed_backup', 'backup-disconnected': 'disconnected_b_keeps_the_original_connection_and_stream_on_an_allowed_backup', 'backup-removed': 'removing_b_retires_its_routes_and_preserves_the_original_ac_stream_on_an_allowed_backup', 'backup-interrupted': 'interrupted_member_circuit_keeps_the_original_ac_stream_on_an_allowed_backup', 'expiry': 'expired_data_routes_cannot_revive_after_reenabling_the_service', 'one-hop': 'an_inner_member_relay_connection_cannot_be_used_as_another_relay_outer_link', 'routes': 'forged_destinations_and_retired_tokens_cannot_redirect_member_data', 'bounds': 'malformed_and_over_budget_member_frames_have_bounded_forwarding', 'restriction': 'explicit_consumer_data_relay_restriction_blocks_the_route_at_actual_send', 'signal-isolation': 'live_signaling_service_never_becomes_business_fallback_when_b_disables_data_relay'}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--case', choices=CASES)
    parser.add_argument('--suite', action='store_true')
    parser.add_argument('--start-at', choices=['irc','default-build','fork-format'], help='resume only remaining suite checks after earlier receipts passed')
    args = parser.parse_args()
    if args.start_at and not args.suite: parser.error('start-at requires suite')
    if args.case and args.suite:
        parser.error('case and suite are separate modes')
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'checks': [], 'passed': False,
        'environment': 'native Linux IPv4 loopback; independent persisted nodes; real signed membership/TLS/QUIC; TCP outer links; controlled UDP backup',
        'skipped': 'Previously validated unchanged slow NAT matrices, per standing user instruction'}
    owned = []
    def run(name, command, expected=None, environment=None):
        print(f'Starting {name}', flush=True)
        started = time.monotonic()
        with (output/(name+'.log')).open('w') as log:
            result = subprocess.run(['rtk', 'proxy', *command], cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT)
        raw = (output/(name+'.log')).read_text()
        passed = sum(map(int, re.findall(r'test result: ok\. (\d+) passed', raw)))
        receipt['checks'].append({'name': name, 'argv': ['rtk','proxy',*command], 'exit': result.returncode, 'seconds': round(time.monotonic()-started,3), 'reported_passed': passed, 'log': name+'.log'})
        (output/'results.json').write_text(json.dumps(receipt,indent=2)+'\n')
        if result.returncode or (expected is not None and passed != expected):
            print(raw[-6000:], flush=True)
            raise RuntimeError(f'{name} failed or executed unexpected number of tests')
        return raw
    try:
        if not args.start_at:
            # Build and discover the actual current test binary before exact selections.
            raw = run('build', ['cargo','test','--locked','--test','member_relay','--features','member-signaling-test-hooks','--no-run','--message-format=json'])
            binary = None
            for line in raw.splitlines():
                try: event = json.loads(line)
                except ValueError: continue
                if event.get('reason') == 'compiler-artifact' and event.get('target',{}).get('name') == 'member_relay':
                    binary = event.get('executable') or binary
            if not binary: raise RuntimeError('no member-relay test executable')
            listed = subprocess.check_output(['rtk','proxy',binary,'--list'],cwd=root,text=True)
            for name, case in CASES.items():
                if args.case and args.case != name: continue
                if f'{case}: test' not in listed: raise RuntimeError(f'absent case {case}')
                run(name,[binary,case,'--exact','--nocapture','--test-threads=1'],1)
        if args.suite:
            if not args.start_at:
                run('affected-short', ['python','-B','scripts/check_path_maintenance.py','--output',str(output/'path-maintenance-short'),'--suite'])
            if args.start_at in (None, 'irc'):
                base = root/'tests/fixtures/irc/ergo.yaml'
                limited = output/'limited-irc.yaml'
                text = base.read_text()
                marker = 'fakelag:\n  enabled: false'
                if marker not in text: raise RuntimeError('IRC rate fixture changed')
                limited.write_text(text.replace(marker,'fakelag:\n  enabled: true',1))
                addresses = []
                for purpose, config in [('usable',base),('limited',limited)]:
                    name = f'miopunch48-irc-{purpose}-{time.time_ns()}'
                    owned.append(name)
                    run('start-irc-'+purpose, ['docker','run','--detach','--name',name,'--publish','127.0.0.1::6667','--entrypoint','/ircd-bin/ergo','--mount',f'type=bind,src={config},dst=/ircd/ircd.yaml,readonly','ergochat/ergo@sha256:cf50544214cf196d25913553a451b766101edce114b5e005a7c063da578851a7','run'])
                    address = subprocess.check_output(['rtk','proxy','docker','port',name,'6667/tcp'],text=True).strip()
                    host, port = address.rsplit(':',1)
                    deadline = time.monotonic()+10
                    while True:
                        try:
                            with socket.create_connection((host,int(port)),timeout=1): break
                        except OSError:
                            if time.monotonic() > deadline: raise
                            time.sleep(.1)
                    addresses.append(address)
                receipt['irc_servers'] = addresses
                environment = dict(os.environ, MIOPUNCH_IRC_SERVER=addresses[0], MIOPUNCH_IRC_LIMITED_SERVER=addresses[1])
                run('irc', ['cargo','test','--locked','--test','irc_bootstrap','--','--nocapture','--test-threads=1'], 10, environment)
            if args.start_at in (None,'irc','default-build'):
                run('default-build', ['cargo','check','--locked','--test','member_relay'])
            run('fork-format', ['rustfmt','--check','--edition','2024','--config','skip_children=true','vendor/iroh/src/socket.rs','vendor/iroh/src/socket/transports.rs','vendor/iroh/src/socket/remote_map.rs','vendor/iroh/src/socket/remote_map/remote_state.rs'])
        receipt['passed'] = True
    except KeyboardInterrupt: receipt['interrupted'] = True
    except Exception as error: receipt['error'] = str(error)
    finally:
        receipt['cleanup'] = []
        for name in owned:
            result = subprocess.run(['rtk','proxy','docker','rm','--force','--volumes',name],capture_output=True,text=True)
            receipt['cleanup'].append({'owned':name,'exit':result.returncode})
            receipt['passed'] &= result.returncode == 0
        (output/'results.json').write_text(json.dumps(receipt,indent=2)+'\n')
    print(f'{"PASS" if receipt["passed"] else "FAIL"}: {output}',flush=True)
    return 0 if receipt['passed'] else 1

if __name__ == '__main__': sys.exit(main())
