#!/usr/bin/env python3
"""Windows-issued forwarding guests with actual exit, revoke and expiry effects."""
import argparse
from pathlib import Path
import subprocess
import time

from check_forwarding import Node
from check_forwarding_platform_guards import GuardRunner, RemoteTarget


class GuestPlatformRunner(GuardRunner):
    def __init__(self, args):
        super().__init__(args)
        self.freeze_source(__file__)
        self.freeze_source(Path(__file__).with_name('check_forwarding_platform_guards.py'))

    def guest(self, windows, target, label, seconds):
        remote_path = windows.directory + '\\' + label + '-invitation.json'
        invitation = self.private / (label + '-invitation.json')
        windows.call('guest_issue', provider=windows.address, service=target.protocol + '_forward',
                     seconds=seconds, path=remote_path)
        result = subprocess.run(['scp', '-q', 'laptop:' + remote_path.replace('\\', '/'), str(invitation)],
                                capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, 'owned invitation control transfer failed'
        invitation.chmod(0o600)
        guest = Node(self, label, 'join-guest', invitation)
        guest.call('forward_start', ok=False, address=windows.address, protocol=target.protocol,
                   target=target.address, bind='[::1]:0')
        state = guest.call('guest_redeem')
        assert state['stage'] == 'GuestActive'
        assert state['scope']['guest'] == guest.identity and state['scope']['provider'] == windows.identity
        assert state['scope']['service'] == target.protocol + '_forward'
        assert guest.identity not in [row['node'] for row in windows.call('status')['members']]
        return guest, state, invitation

    def exchanges(self, linux, windows):
        for protocol in ['tcp', 'udp']:
            target = RemoteTarget(windows, protocol, windows.targets[protocol], 0)
            direct = target.control('target_probe', protocol=protocol, length=37, seed=23)
            assert direct['exact'] and direct['length'] == 37
            windows.call('forward_enable', protocol=protocol, targets=[target.address])
            guest, state, invitation = self.guest(windows, target, protocol + '-exit', 240)
            duplicate = Node(self, protocol + '-duplicate', 'join-guest', invitation)
            duplicate.call('guest_redeem', ok=False)
            guest.call('forward_start', ok=False, address=windows.address,
                       protocol='udp' if protocol == 'tcp' else 'tcp', target=target.address, bind='[::1]:0')
            guest.call('forward_enable', ok=False, protocol=protocol, targets=[target.address])
            guest.call('ping', ok=False, address=windows.address)
            # The deliberate member-protocol rejection uses the shared peer
            # contact budget. Respect its existing first/second cooldown before
            # asserting the separately authorized forwarding operation.
            time.sleep(2.05)
            with self.flow(guest, windows, target) as (forward, app):
                before = self.exchange(app, target)
                ended = guest.call('guest_end')
                assert ended['ended_locally'] and all(row['saved'] for row in ended['delivery'])
                self.joined_provider(windows)
                self.no_effect(app, target, before)
            guest.call('forward_start', ok=False, address=windows.address, protocol=protocol,
                       target=target.address, bind='[::1]:0')
            self.record('guest', 'windows_issuer_exit_' + protocol, {'identity_bound': True,
                'not_ordinary_member': True, 'other_service_denied': True, 'not_provider': True,
                'member_protocol_denied': True, 'actual_allowed_flow': True, 'exit_no_later_effect': True})

            revoked, state, _ = self.guest(windows, target, protocol + '-revoke', 180)
            with self.flow(revoked, windows, target) as (forward, app):
                before = self.exchange(app, target)
                windows.call('guest_revoke', invitation=state['scope']['invitation'])
                self.joined_provider(windows)
                self.no_effect(app, target, before)
            self.record('guest', 'windows_issuer_revoke_' + protocol, {'actual_allowed_flow': True,
                'provider_joined': True, 'no_later_target_effect': True})

            expiring, state, _ = self.guest(windows, target, protocol + '-expiry', 45)
            expires = state['scope']['expires']
            with self.flow(expiring, windows, target) as (forward, app):
                assert time.time() < expires
                before = self.exchange(app, target)
                assert time.time() < expires
                self.until(lambda: time.time() > expires + .1 and
                           expiring.call('guest_status')['stage'] == 'GuestExpired',
                           'actual guest deadline has not expired', 90)
                self.joined_provider(windows)
                # Preserve the previously observed host-clock rollback premise.
                # The provider already had to stop at the first true expiry.
                self.until(lambda: time.time() > expires + 20 and
                           expiring.call('guest_status')['stage'] == 'GuestExpired',
                           'stable expiry observation window unavailable', 90)
                wall_before = time.time()
                self.no_effect(app, target, before)
                wall_after = time.time()
                assert wall_before > expires and wall_after > expires
            expiring.call('forward_start', ok=False, address=windows.address, protocol=protocol,
                          target=target.address, bind='[::1]:0')
            assert time.time() > expires
            self.record('guest', 'windows_issuer_expiry_' + protocol, {'expires': expires,
                'wall_before': wall_before, 'wall_after': wall_after, 'actual_success_before': True,
                'provider_joined_at_first_expiry': True, 'no_later_target_effect': True,
                'new_access_denied': True})
            windows.call('forward_disable', protocol=protocol)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--source-receipt', required=True)
    parser.add_argument('--windows-sdk', required=True)
    parser.add_argument('--windows-sha', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.case, args.loopback, args.reuse_windows_tcp = 'platform-guests', '::1', None
    GuestPlatformRunner(args).run()


if __name__ == '__main__':
    main()
