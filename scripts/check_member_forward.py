#!/usr/bin/env python3
"""Real one-hop member signaling acceptance; optional affected short regressions."""
import argparse
import json
import pathlib
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--short', action='store_true')
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'checks': [], 'passed': False,
               'environment': 'Linux loopback; three real persistent members; no NAT success claim',
               'skipped': 'Previously validated unchanged slow NAT matrices, per user policy'}
    selectors = [
        ('first', 'member_without_data_relay_forwards_protected_control_then_ac_uses_a_direct_path'),
        ('priority', 'signaling_only_cannot_release_business_without_another_permitted_ac_path'),
        ('duplicates', 'duplicated_opaque_datagrams_cannot_reveal_private_control_or_release_extra_connections'),
        ('registration', 'repeated_registration_does_not_add_forwarding_routes_or_confuse_member_identity'),
        ('expiry', 'expired_forwarding_leases_remove_routes_without_ending_the_member_links'),
        ('interruption', 'stopping_a_forwarding_stream_fails_ac_control_and_keeps_ab_usable'),
        ('identity', 'a_forwarding_route_cannot_substitute_the_inner_tls_target_identity'),
        ('scope', 'an_admitted_bootstrap_session_does_not_grant_member_forwarding'),
        ('endpoint-close', 'closing_the_endpoint_releases_its_forwarding_lease_without_waiting_for_expiry'),
    ]
    checks = [(name, ['cargo', 'test', '--locked', '--test', 'member_forward',
                     '--features', 'member-signaling-test-hooks', selector,
                     '--', '--exact', '--nocapture', '--test-threads=1'])
              for name, selector in selectors]
    if args.short:
        checks += [
            ('affected-short', ['python', '-B', 'scripts/check_mqtt_bootstrap_short.py',
                                '--output', str(output/'mqtt-short'), '--suite']),
            ('all-test-hooks', ['cargo', 'check', '--locked', '--workspace', '--all-targets',
                               '--features', 'nat-lab-test-hooks,replica-test-hooks,member-signaling-test-hooks']),
            ('default-member-build', ['cargo', 'check', '--locked', '--test', 'member_forward']),
        ]
    try:
        for name, command in checks:
            print(f'Starting {name}', flush=True)
            started = time.monotonic()
            with (output/(name+'.log')).open('w') as log:
                result = subprocess.run(['rtk', 'proxy', *command], cwd=root,
                                        stdout=log, stderr=subprocess.STDOUT)
            receipt['checks'].append({'name': name, 'argv': command, 'exit': result.returncode,
                                      'seconds': round(time.monotonic()-started, 3),
                                      'log': name+'.log'})
            (output/'results.json').write_text(json.dumps(receipt, indent=2)+'\n')
            if result.returncode:
                break
        else:
            receipt['passed'] = True
    except KeyboardInterrupt:
        receipt['interrupted'] = True
    finally:
        (output/'results.json').write_text(json.dumps(receipt, indent=2)+'\n')
    print(f'{"PASS" if receipt["passed"] else "FAIL"} member forwarding checks: {output}')
    raise SystemExit(0 if receipt['passed'] else 1)


if __name__ == '__main__':
    main()
