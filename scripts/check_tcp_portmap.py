#!/usr/bin/env python3
"""Build and run one owned, offline kernel gateway lab; never run slow NAT matrices."""
import argparse
import json
import pathlib
import subprocess
import time


from check_build_cache import CheckBuilds

builds = CheckBuilds()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--strip-debug', action='store_true')
    parser.add_argument('--worker', type=pathlib.Path, help='reuse an already built worker; rebuild after source changes')
    parser.add_argument('--case', default='positive', choices=['positive', 'close-isolation', 'expired', 'changed', 'restart', 'renewed', 'disabled', 'disabled-advanced', 'tcp-disabled-advanced', 'tcp-disabled', 'initial-failure', 'allocation-timeout', 'allocation-cancel', 'renewal-address-failure'])
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'case': args.case, 'commands': []}

    def execute(command, name):
        started = time.monotonic()
        with (output / name).open('w') as out:
            result = builds.run(['rtk', 'proxy', *command], cwd=root, text=True, stdout=out, stderr=subprocess.STDOUT)
        receipt['commands'].append({'argv': command, 'exit': result.returncode, 'seconds': round(time.monotonic() - started, 3), 'log': name})
        (output / 'results.json').write_text(json.dumps(receipt, indent=2) + '\n')
        if result.returncode:
            raise SystemExit(f'FAIL exit={result.returncode}; read {output / name}')

    if args.worker:
        worker = args.worker.resolve()
        if not worker.is_file():
            raise SystemExit('worker executable does not exist')
        binary = worker.relative_to(root)
        receipt['reused_worker'] = str(binary)
    else:
        build = ['cargo', 'rustc', '--locked', '--test', 'tcp_portmap', '--message-format=json']
        execute(build, 'build.log')
        executables = [item['executable'] for line in (output / 'build.log').read_text().splitlines() if line.startswith('{') for item in [json.loads(line)] if item.get('executable') and item.get('target', {}).get('name') == 'tcp_portmap']
        binary = pathlib.Path(executables[-1]).resolve().relative_to(root)
    with builds.deployment(root / binary, strip_debug=args.strip_debug) as deployed:
        binary = deployed.relative_to(root)
        execute(['docker', 'run', '--rm', '--name', f'miopunch41-{time.time_ns()}', '--privileged', '--network', 'none', '--mount', f'type=bind,src={root},target=/repo,readonly', '--mount', f'type=bind,src={output},target=/evidence', 'miopunch-nat61-lab:bookworm', 'python3', '/repo/scripts/tcp_portmap_lab.py', 'lab', str(pathlib.Path('/repo') / binary), '/evidence', args.case], 'lab.log')
    receipt['passed'] = True
    (output / 'results.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(f'PASS {args.case}: real gateway + formal node evidence={output}')


if __name__ == '__main__':
    main()
