#!/usr/bin/env python3
"""Own two TLS targets and reject a real exclusive-file collision before loops."""
import argparse
import json
import os
from pathlib import Path
import ssl
from urllib.request import HTTPSHandler, ProxyHandler, build_opener

from check_web_service import HttpTarget, WebRunner


class ConstructorRunner(WebRunner):
    def scenario(self):
        first, second = HttpTarget(self, tls=True), HttpTarget(self, tls=True)
        assert first.certificate != second.certificate
        assert first.label == 'target:1' and second.label == 'target:2'

        def live(target):
            trust = ssl.create_default_context(cafile=target.certificate)
            with build_opener(ProxyHandler({}), HTTPSHandler(context=trust)).open(target.url, timeout=3) as response:
                assert response.read() == b'owned M119 web target'

        live(first)
        live(second)
        assert first.requests == second.requests == ['/']
        self.record('controller', 'two_TLS_targets_same_runner', {
            'distinct_exclusive_certificate_files': True, 'both_direct_HTTPS_exact': True})

        def created():
            rows = [json.loads(line) for line in (self.output / 'events.jsonl').read_text().splitlines()]
            return [(row['result']['owner'], row['result']['loop_id']) for row in rows
                    if row['operation'] == 'owned_asyncio' and row['result']['phase'] == 'created']

        before = created()
        assert len(before) == len(self.targets) == 2
        collision = self.private / 'target-3-key.pem'
        descriptor = os.open(collision, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, 'wb') as owned:
            owned.write(b'owned constructor-exclusive-file precondition')
        try:
            HttpTarget(self, tls=True)
        except FileExistsError:
            pass
        else:
            raise AssertionError('TLS constructor overwrote an existing owned exclusive file')
        assert collision.read_bytes() == b'owned constructor-exclusive-file precondition'
        assert created() == before and len(self.targets) == 2
        live(first)
        live(second)
        assert first.requests == second.requests == ['/', '/']
        self.record('controller', 'exclusive_certificate_failure_before_loop', {
            'actual_O_EXCL_FileExistsError': True, 'failed_target_created_no_loop': True,
            'registered_targets_unchanged': True, 'prior_targets_still_live_exact': True,
            'no_SDK_or_credentials_or_monkeypatch': True})


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--source-receipt', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--case', default='target-construction')
    ConstructorRunner(parser.parse_args()).run()
