#!/usr/bin/env python3
"""Build and run a formal MQTT entry in an owned offline dual-NAT container."""
import argparse
import json
import pathlib
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--worker', type=pathlib.Path)
    parser.add_argument('--case', choices=['positive', 'domain_stun', 'domain_stun_diagnostic', 'stun_udp', 'blocked', 'simultaneous_cancel', 'endpoint_dependent', 'udp_fallback', 'simultaneous', 'mixed_tcp', 'wrong_target', 'private_bind', 'cancel_attempt', 'observer_failure', 'candidate_denied'], default='positive')
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'case': args.case, 'commands': [], 'passed': False}

    def execute(command, name):
        start = time.monotonic()
        with (output / name).open('w') as log:
            result = subprocess.run(['rtk', 'proxy', *command], cwd=root, stdout=log, stderr=subprocess.STDOUT)
        receipt['commands'].append({'argv': command, 'exit': result.returncode, 'seconds': round(time.monotonic()-start, 3), 'log': name})
        (output / 'results.json').write_text(json.dumps(receipt, indent=2)+'\n')
        if result.returncode:
            raise SystemExit(f'FAIL exit={result.returncode}; read {output / name}')

    if args.worker:
        worker = args.worker.resolve().relative_to(root)
    else:
        execute(['cargo', 'test', '--locked', '--test', 'tcp_punch', '--no-run', '--message-format=json'], 'build.log')
        executables = [item['executable'] for line in (output/'build.log').read_text().splitlines() if line.startswith('{') for item in [json.loads(line)] if item.get('executable') and item.get('target', {}).get('name') == 'tcp_punch']
        worker = pathlib.Path(executables[-1]).resolve().relative_to(root)
    if args.case in ['udp_fallback', 'mixed_tcp', 'wrong_target', 'private_bind', 'cancel_attempt', 'observer_failure', 'candidate_denied']:
        execute(['cargo', 'build', '--locked', '--bin', 'udp_observation_probe'], 'qad-build.log')
    execute(['docker', 'run', '--rm', '--name', f'miopunch43-{time.time_ns()}', '--privileged', '--network', 'none', '--mount', f'type=bind,src={root},target=/repo,readonly', '--mount', f'type=bind,src={output},target=/evidence', 'miopunch-nat61-lab:bookworm', 'python3', '/repo/scripts/tcp_punch_lab.py', str(pathlib.Path('/repo')/worker), '/repo/target/debug/udp_observation_probe', '/evidence', args.case], 'lab.log')
    receipt['passed'] = True
    (output/'results.json').write_text(json.dumps(receipt, indent=2)+'\n')
    print(f'PASS TCP formal native punch: {output}')


if __name__ == '__main__':
    main()
