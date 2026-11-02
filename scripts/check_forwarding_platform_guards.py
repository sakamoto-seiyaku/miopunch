#!/usr/bin/env python3
"""Real Windows provider scope, ACL, stop, disable and reopen effects."""
import argparse
from contextlib import contextmanager
from pathlib import Path
import socket
import time

from check_forwarding_platform import PlatformRunner


class RemoteTarget:
    def __init__(self, windows, protocol, address, slot):
        self.windows, self.protocol, self.address, self.slot = windows, protocol, address, slot

    def control(self, kind, **fields):
        return self.windows.control(kind, target_slot=self.slot, **fields)

    def prepare(self, length=37, seed=23):
        self.control('expect', protocol=self.protocol, length=length, seed=seed)

    def count(self):
        state = self.control('target_status')
        assert not state['errors']
        return state['tcp_bytes'] if self.protocol == 'tcp' else state['udp_datagrams']

    def exact(self, length=37):
        state = self.control('target_status')
        assert state[self.protocol + '_case_exact'] and not state['errors']
        assert (state['tcp_case_bytes'] == length if self.protocol == 'tcp' else state['udp_case_datagrams'] == 1)


class GuardRunner(PlatformRunner):
    def __init__(self, args):
        super().__init__(args)
        self.freeze_source(__file__)

    def until(self, predicate, label, seconds=75):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.1)
        raise AssertionError(label)

    @contextmanager
    def flow(self, consumer, provider, target):
        forward = consumer.call('forward_start', address=provider.address, protocol=target.protocol,
                                target=target.address, bind='[::1]:0')
        forward['stopped'] = False
        host, port = forward['listen'].rsplit(':', 1)
        app = socket.socket(socket.AF_INET6, socket.SOCK_STREAM if target.protocol == 'tcp' else socket.SOCK_DGRAM)
        app.settimeout(75)
        primary = None
        try:
            app.connect((host.strip('[]'), int(port)))
            yield forward, app
        except BaseException as failure:
            primary = failure
            raise
        finally:
            app.close()
            if not forward['stopped']:
                try:
                    self.stop(consumer, forward)
                except BaseException as failure:
                    self.record('controller', 'flow_finally_stop_failed', {'type': type(failure).__name__})
                    if primary is None:
                        raise

    def stop(self, consumer, forward):
        result = consumer.call('forward_stop', task=forward['id'])
        forward['stopped'] = True
        assert result['active_flows'] == 0

    def exchange(self, app, target):
        target.prepare()
        data = bytes((index + 23) % 251 for index in range(37))
        app.sendall(data) if target.protocol == 'tcp' else app.send(data)
        response = b''
        while len(response) < len(data):
            part = app.recv(65536)
            assert part
            response += part
            if target.protocol == 'udp':
                break
        assert response == data
        target.exact()
        return target.count()

    def no_effect(self, app, target, before):
        app.settimeout(1)
        try:
            app.send(b'owned-after-revocation')
            assert app.recv(65536) == b'', 'retired flow returned business data'
        except (ConnectionResetError, ConnectionRefusedError, BrokenPipeError, socket.timeout):
            pass
        assert target.count() == before, 'retired or unauthorized flow reached target'

    def joined_provider(self, windows):
        def joined():
            state = windows.call('forward_status')
            return state['active_flows'] == 0 and state['provider_workers'] == 0
        self.until(joined, 'provider flows not joined after revocation', 10)

    def exchanges(self, linux, windows):
        extra = windows.control('target_create')
        assert extra['slot'] == 1
        retained_policies = {}
        for protocol in ['tcp', 'udp']:
            service = protocol + '_forward'
            allowed = RemoteTarget(windows, protocol, windows.targets[protocol], 0)
            forbidden = RemoteTarget(windows, protocol, extra[protocol + '_target'], 1)
            for target in [allowed, forbidden]:
                probe = target.control('target_probe', protocol=protocol, length=37, seed=23)
                assert probe['exact'] and probe['length'] == 37
            windows.call('forward_enable', protocol=protocol, targets=[allowed.address])
            before = forbidden.count()
            with self.flow(linux, windows, forbidden) as (forward, app):
                self.no_effect(app, forbidden, before)
                self.until(lambda: next(row for row in linux.call('forward_status')['listeners']
                    if row['id'] == forward['id'])['counters']['failed'] > 0,
                    'unauthorized target attempt did not settle')
                assert forbidden.count() == before
            self.joined_provider(windows)
            self.record('guards', 'exact_target_scope_' + protocol, {'both_targets_independently_live': True,
                        'forbidden_target_unchanged': True, 'actual_failed_flow': True})
            time.sleep(2.05)  # Existing denied-contact backoff, not a retry.

            deny = dict(members=True, guests=True, allowed=[], denied=[linux.identity])
            with self.flow(linux, windows, allowed) as (forward, app):
                before = self.exchange(app, allowed)
                revision = windows.call('service_rule', provider=windows.identity, service=service,
                                        previous=None, rule=deny)['revision']
                self.joined_provider(windows)
                self.no_effect(app, allowed, before)
            self.record('guards', 'effective_acl_stops_' + protocol, {'actual_flow_precondition': True,
                        'provider_joined': True, 'no_later_target_effect': True})

            # The authorized remote editor must first obtain the exact predecessor.
            linux.call('service_sync', address=windows.address)
            policy = next(row for row in linux.call('service_status')['policies']
                          if row['provider'] == windows.identity and row['service'] == service)
            assert policy['revision'] == revision and policy['rule'] == deny
            default = dict(members=True, guests=True, allowed=[], denied=[])
            restored = linux.call('service_rule', provider=windows.identity, service=service,
                                  previous=revision, rule=default)['revision']
            linux.call('service_sync', address=windows.address)
            policy = next(row for row in windows.call('service_status')['policies']
                          if row['provider'] == windows.identity and row['service'] == service)
            assert policy['revision'] == restored and policy['rule'] == default
            with self.flow(linux, windows, allowed) as (forward, app):
                before = self.exchange(app, allowed)
                windows.call('forward_enable', protocol=protocol, targets=[forbidden.address])
                self.joined_provider(windows)
                self.no_effect(app, allowed, before)
            self.record('guards', 'scope_replacement_stops_' + protocol, {'restored_same_operation': True,
                        'provider_joined': True, 'old_target_unchanged': True})

            with self.flow(linux, windows, forbidden) as (forward, app):
                before = self.exchange(app, forbidden)
                self.stop(linux, forward)
                self.joined_provider(windows)
                self.no_effect(app, forbidden, before)
            self.record('guards', 'explicit_stop_' + protocol, {'listener_and_flows_joined': True,
                        'independent_target_unchanged_after_stop': True})
            with self.flow(linux, windows, forbidden) as (forward, app):
                before = self.exchange(app, forbidden)
                windows.call('forward_disable', protocol=protocol)
                self.joined_provider(windows)
                self.no_effect(app, forbidden, before)
            self.record('guards', 'disable_' + protocol, {'provider_joined': True,
                        'independent_target_unchanged_after_disable': True})
            retained_policies[service] = windows.call('service_rule', provider=windows.identity,
                service=service, previous=restored, rule=deny)['revision']

        reopened = windows.control('restart')
        assert reopened['status']['node'] == windows.identity
        windows.address = reopened['address']
        state = windows.call('forward_status')
        assert state['active_flows'] == state['provider_workers'] == 0 and not state['listeners']
        assert all(not row['enabled'] and row['targets'] is None for row in state['services'])
        policies = windows.call('service_status')['policies']
        for service, revision in retained_policies.items():
            policy = next(row for row in policies if row['provider'] == windows.identity and row['service'] == service)
            assert policy['revision'] == revision and policy['rule']['denied'] == [linux.identity]
        self.record('guards', 'windows_reopen', {'identity_retained': True, 'both_acls_retained': True,
                    'no_automatic_provider_or_listener': True, 'zero_owned_flows': True})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--source-receipt', required=True)
    parser.add_argument('--windows-sdk', required=True)
    parser.add_argument('--windows-sha', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.case, args.loopback, args.reuse_windows_tcp = 'platform-guards', '::1', None
    GuardRunner(args).run()


if __name__ == '__main__':
    main()
