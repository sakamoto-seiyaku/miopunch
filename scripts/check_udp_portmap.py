#!/usr/bin/env python3
"""Build and run one owned, offline kernel gateway lab; never run slow NAT matrices."""
import argparse
import json
import pathlib
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--case', default='positive', choices=['positive', 'expired', 'changed', 'restart', 'renewed', 'disabled', 'allocation-timeout', 'allocation-cancel', 'renewal-address-failure'])
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'case': args.case, 'commands': []}

    def execute(command, name):
        started = time.monotonic()
        with (output / name).open('w') as out:
            result = subprocess.run(['rtk', 'proxy', *command], cwd=root, text=True, stdout=out, stderr=subprocess.STDOUT)
        receipt['commands'].append({'argv': command, 'exit': result.returncode, 'seconds': round(time.monotonic() - started, 3), 'log': name})
        (output / 'results.json').write_text(json.dumps(receipt, indent=2) + '\n')
        if result.returncode:
            raise SystemExit(f'FAIL exit={result.returncode}; read {output / name}')

    execute(['cargo', 'test', '--locked', '--test', 'udp_portmap', '--no-run', '--message-format=json'], 'build.log')
    executables = [item['executable'] for line in (output / 'build.log').read_text().splitlines() if line.startswith('{') for item in [json.loads(line)] if item.get('executable') and item.get('target', {}).get('name') == 'udp_portmap']
    binary = pathlib.Path(executables[-1]).resolve().relative_to(root)
    execute(['docker', 'run', '--rm', '--name', f'miopunch39-{time.time_ns()}', '--privileged', '--network', 'none', '--mount', f'type=bind,src={root},target=/repo,readonly', '--mount', f'type=bind,src={output},target=/evidence', 'miopunch-nat61-lab:bookworm', 'python3', '/repo/scripts/udp_portmap_lab.py', 'lab', str(pathlib.Path('/repo') / binary), '/evidence', args.case], 'lab.log')
    receipt['passed'] = True
    (output / 'results.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(f'PASS {args.case}: real gateway + formal node evidence={output}')


if __name__ == '__main__':
    main()
