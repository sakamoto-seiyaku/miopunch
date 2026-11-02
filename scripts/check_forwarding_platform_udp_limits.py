#!/usr/bin/env python3
"""One legal IPv6 UDP payload above the SDK wire bound at each receive entry.

Only fresh owned loopback application/target sockets receive 65,509 bytes.
Independent counters must prove one drop and continued normal use of the same
association. This does not inject malformed SDK records or use public relays.
"""
import argparse
from pathlib import Path
import socket

from check_forwarding import Target
from check_forwarding_platform_boundaries import BoundaryRunner, payload


class ReplyTarget(Target):
    def __init__(self, runner):
        self.large_reply = None
        self.large_replies = 0
        super().__init__(runner, 'udp', prefix=b'')

    def run(self):
        try:
            while not self.stop.is_set():
                try:
                    data, address = self.socket.recvfrom(65536)
                except socket.timeout:
                    continue
                self.received.append(data)
                if self.large_reply is not None:
                    assert self.socket.sendto(self.large_reply, address) == len(self.large_reply)
                    self.large_replies += 1
                assert self.socket.sendto(data, address) == len(data)
        except BaseException as failure:
            self.errors.append(type(failure).__name__)


class LimitRunner(BoundaryRunner):
    def __init__(self, args):
        super().__init__(args)
        self.freeze_source(__file__)
        self.freeze_source(Path(__file__).with_name('check_forwarding_platform_boundaries.py'))

    @staticmethod
    def listener(node, identifier):
        return next(row for row in node.call('forward_status')['listeners'] if row['id'] == identifier)

    @staticmethod
    def provider_flow(node, target):
        return next(row for row in node.call('forward_status')['flows']
                    if row['provider'] and row['protocol'] == 'udp' and row['target'] == target)

    def exchanges(self, linux, windows):
        large = payload(65509, 13)
        normal = payload(37, 23)
        # Establish both OS premises before testing SDK classification.
        direct = windows.control('target_probe', protocol='udp', length=len(large), seed=13)
        assert direct['length'] == len(large) and direct['exact']
        target = ReplyTarget(self)
        with self.application(target.address, 'udp') as app:
            assert app.send(large) == len(large) and app.recv(65536) == large
        self.record('boundary', 'actual_ipv6_65509_premises', {'windows_direct_exact': True,
                    'linux_direct_exact': True, 'bytes': len(large)})

        windows.call('forward_enable', protocol='udp', targets=[windows.targets['udp']])
        forwarding = linux.call('forward_start', address=windows.address, protocol='udp',
                                target=windows.targets['udp'], bind='[::1]:0')
        with self.application(forwarding['listen'], 'udp') as app:
            windows.control('expect', protocol='udp', length=37, seed=23)
            assert app.send(normal) == len(normal) and app.recv(65536) == normal
            before = windows.control('target_status')['udp_datagrams']
            assert self.listener(linux, forwarding['id'])['counters']['dropped_datagrams'] == 0
            assert app.send(large) == len(large)
            self.until(lambda: self.listener(linux, forwarding['id'])['counters']['dropped_datagrams'] == 1,
                       'Linux consumer did not count one local oversized datagram')
            assert self.listener(linux, forwarding['id'])['running']
            assert windows.control('target_status')['udp_datagrams'] == before
            assert app.send(normal) == len(normal) and app.recv(65536) == normal
            assert windows.control('target_status')['udp_datagrams'] == before + 1
            self.record('boundary', 'linux_consumer_drop', {'dropped': 1, 'listener_live': True,
                        'same_socket_normal_after': True, 'no_oversized_target_effect': True})

            assert self.provider_flow(windows, windows.targets['udp'])['dropped_datagrams'] == 0
            windows.control('expect_udp_reply', length=37, seed=23, response_length=len(large),
                            response_seed=13, echo_after_reply=True)
            assert app.send(normal) == len(normal) and app.recv(65536) == normal
            self.until(lambda: self.provider_flow(windows, windows.targets['udp'])['dropped_datagrams'] == 1,
                       'Windows provider did not count one target reply drop')
            state = windows.control('target_status')
            assert state['udp_case_exact'] and state['udp_case_replies'] == 1
            assert state['udp_case_datagrams'] == 1
            self.record('boundary', 'windows_provider_reply_drop', {'target_sent_once': True,
                        'dropped': 1, 'same_association_valid_reply_exact': True})
        linux.call('forward_stop', task=forwarding['id'])
        self.until(lambda: windows.call('forward_status')['active_flows'] == 0, 'Windows provider not joined')

        linux.call('forward_enable', protocol='udp', targets=[target.address])
        forwarding = windows.call('forward_start', address=linux.address, protocol='udp',
                                  target=target.address, bind='[::1]:0')
        client = windows.control('client_open', protocol='udp', address=forwarding['listen'])['client']
        try:
            assert windows.control('client_exchange', client=client, length=37, seed=23)['exact']
            before = len(target.received)
            assert self.listener(windows, forwarding['id'])['counters']['dropped_datagrams'] == 0
            sent = windows.control('client_send_datagram', client=client, length=len(large), seed=13)
            assert sent['sent'] == len(large) and sent['delivery_confirmed'] is False
            self.until(lambda: self.listener(windows, forwarding['id'])['counters']['dropped_datagrams'] == 1,
                       'Windows consumer did not count one local oversized datagram')
            assert self.listener(windows, forwarding['id'])['running']
            assert len(target.received) == before
            assert windows.control('client_exchange', client=client, length=37, seed=23)['exact']
            assert target.received[before:] == [normal]
            self.record('boundary', 'windows_consumer_drop', {'dropped': 1, 'listener_live': True,
                        'same_socket_normal_after': True, 'no_oversized_target_effect': True})

            assert self.provider_flow(linux, target.address)['dropped_datagrams'] == 0
            target.large_reply = large
            before = len(target.received)
            assert windows.control('client_exchange', client=client, length=37, seed=23)['exact']
            self.until(lambda: self.provider_flow(linux, target.address)['dropped_datagrams'] == 1,
                       'Linux provider did not count one target reply drop')
            assert target.received[before:] == [normal] and target.large_replies == 1
            self.record('boundary', 'linux_provider_reply_drop', {'target_sent_once': True,
                        'dropped': 1, 'same_association_valid_reply_exact': True})
        finally:
            windows.control('client_close', client=client)
        windows.call('forward_stop', task=forwarding['id'])
        self.until(lambda: linux.call('forward_status')['active_flows'] == 0, 'Linux provider not joined')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--source-receipt', required=True)
    parser.add_argument('--windows-sdk', required=True)
    parser.add_argument('--windows-sha', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.case, args.loopback, args.reuse_windows_tcp = 'platform-udp-limits', '::1', None
    LimitRunner(args).run()


if __name__ == '__main__':
    main()
