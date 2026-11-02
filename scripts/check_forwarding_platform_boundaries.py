#!/usr/bin/env python3
"""Cross-platform TCP final replies and ordinary UDP datagram boundaries."""
import argparse
from pathlib import Path
import socket
import time

from check_forwarding import Target
from check_forwarding_completion import EofTarget
from check_forwarding_platform import PlatformRunner


def payload(length, seed):
    return bytes((index + seed) % 251 for index in range(length))


def endpoint(value):
    host, port = value.rsplit(':', 1)
    return host.strip('[]'), int(port)


class BoundaryRunner(PlatformRunner):
    def __init__(self, args):
        super().__init__(args)
        self.freeze_source(__file__)
        self.freeze_source(Path(__file__).with_name('check_forwarding_completion.py'))

    def application(self, address, protocol):
        app = socket.socket(socket.AF_INET6, socket.SOCK_STREAM if protocol == 'tcp' else socket.SOCK_DGRAM)
        app.settimeout(75)
        try:
            app.connect(endpoint(address))
            return app
        except BaseException:
            app.close()
            raise

    def until(self, predicate, message, seconds=10):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.1)
        raise AssertionError(message)

    def exchanges(self, linux, windows):
        request, response = payload(131089, 3), payload(524305, 41)
        fields = dict(length=len(request), seed=3, response_length=len(response), response_seed=41)
        direct = windows.control('target_probe_eof_reply', **fields)
        assert direct['exact'] and direct['eof'] and direct['write_half_closed']
        windows.control('expect_eof_reply', **fields)
        windows.call('forward_enable', protocol='tcp', targets=[windows.targets['tcp']])
        forwarding = linux.call('forward_start', address=windows.address, protocol='tcp',
                                target=windows.targets['tcp'], bind='[::1]:0')
        with self.application(forwarding['listen'], 'tcp') as app:
            app.sendall(request)
            app.shutdown(socket.SHUT_WR)
            received = bytearray()
            while True:
                part = app.recv(65536)
                if not part:
                    break
                received.extend(part)
                assert len(received) <= len(response)
            assert received == response
        state = windows.control('target_status')
        assert state['tcp_case_exact'] and state['tcp_case_bytes'] == len(request)
        assert state['tcp_case_eof_replies'] == 1
        linux.call('forward_stop', task=forwarding['id'])
        self.until(lambda: windows.call('forward_status')['active_flows'] == 0, 'Windows EOF flow not joined')
        self.record('boundary', 'windows_provider_tcp_final_reply', {'request_bytes': len(request),
            'response_bytes': len(response), 'request_exact': True, 'response_exact': True, 'clean_eof': True})

        target = EofTarget(self, response)
        with self.application(target.address, 'tcp') as direct:
            direct.sendall(request)
            direct.shutdown(socket.SHUT_WR)
            received = bytearray()
            while True:
                part = direct.recv(65536)
                if not part:
                    break
                received.extend(part)
            assert received == response and target.received == [request]
        linux.call('forward_enable', protocol='tcp', targets=[target.address])
        forwarding = windows.call('forward_start', address=linux.address, protocol='tcp',
                                  target=target.address, bind='[::1]:0')
        client = windows.control('client_open', protocol='tcp', address=forwarding['listen'])['client']
        try:
            result = windows.control('client_halfclose_exchange', client=client, **fields)
            assert result['exact'] and result['eof'] and result['write_half_closed']
            self.until(lambda: target.completed == 2, 'Linux target final response not complete')
            assert target.received == [request, request]
        finally:
            windows.control('client_close', client=client)
        windows.call('forward_stop', task=forwarding['id'])
        self.record('boundary', 'linux_provider_tcp_final_reply', {'request_bytes': len(request),
            'response_bytes': len(response), 'request_exact': True, 'response_exact': True, 'clean_eof': True})

        windows.call('forward_enable', protocol='udp', targets=[windows.targets['udp']])
        forwarding = linux.call('forward_start', address=windows.address, protocol='udp',
                                target=windows.targets['udp'], bind='[::1]:0')
        with self.application(forwarding['listen'], 'udp') as app:
            for length in [0, 65507]:
                direct = windows.control('target_probe', protocol='udp', length=length, seed=17)
                assert direct['exact'] and direct['length'] == length
                windows.control('expect', protocol='udp', length=length, seed=17)
                data = payload(length, 17)
                assert app.send(data) == length
                assert app.recv(65536) == data
                state = windows.control('target_status')
                assert state['udp_case_exact'] and state['udp_case_datagrams'] == 1
                self.record('boundary', 'windows_provider_udp', {'length': length,
                    'direct_premise': True, 'one_exact_target_datagram': True, 'reply_exact': True})
        linux.call('forward_stop', task=forwarding['id'])

        target = Target(self, 'udp', prefix=b'')
        linux.call('forward_enable', protocol='udp', targets=[target.address])
        forwarding = windows.call('forward_start', address=linux.address, protocol='udp',
                                  target=target.address, bind='[::1]:0')
        client = windows.control('client_open', protocol='udp', address=forwarding['listen'])['client']
        try:
            for length in [0, 65507]:
                data = payload(length, 17)
                with self.application(target.address, 'udp') as direct:
                    assert direct.send(data) == length and direct.recv(65536) == data
                before = len(target.received)
                result = windows.control('client_exchange', client=client, length=length, seed=17)
                assert result['exact'] and result['length'] == length
                assert target.received[before:] == [data]
                self.record('boundary', 'linux_provider_udp', {'length': length,
                    'direct_premise': True, 'one_exact_target_datagram': True, 'reply_exact': True})
        finally:
            windows.control('client_close', client=client)
        windows.call('forward_stop', task=forwarding['id'])
        self.until(lambda: linux.call('forward_status')['active_flows'] == 0, 'Linux boundary flows not joined')
        self.until(lambda: windows.call('forward_status')['active_flows'] == 0, 'Windows boundary flows not joined')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--source-receipt', required=True)
    parser.add_argument('--windows-sdk', required=True)
    parser.add_argument('--windows-sha', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.case, args.loopback, args.reuse_windows_tcp = 'platform-boundaries', '::1', None
    BoundaryRunner(args).run()


if __name__ == '__main__':
    main()
