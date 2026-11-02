#!/usr/bin/env python3
"""Packaged browser entry, real origin ownership/persistence and duplex files."""
import argparse
import hashlib
import json
from pathlib import Path

from check_web_service import OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_browser_runtime import WebBrowser
from web_process_owner import register_process


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class BrowserHostRunner(WebRunner):
    def scenario(self):
        package = json.loads(self.args.entry_receipt.read_text())
        assert package['status'] == 'packaged' and package['SDK_reused']
        assert sha(Path(package['pin'])) == package['sha256']
        assert Path(package['site']) == self.args.entry_site.resolve()
        release = json.loads((self.args.entry_site/'release.json').read_text())
        assert sha(self.args.entry_site/'release.json') == package['release_sha256']
        assert all(sha(self.args.entry_site/name) == digest for name,digest in release['files'].items())
        root = Path(__file__).resolve().parents[1]
        assert all(sha(root/name) == digest for name,digest in release['selected_repository_inputs'].items())
        assert all(sha(root/name) == digest for name,digest in release['entry_inputs'].items())
        self.freeze_source(root/'scripts/package_browser_host.py')
        configure_local_resources(self, self.args, owner_factory=register_process)
        provider = OwnedNode(self, 'provider')
        browser = WebBrowser(self, self.args)
        opened = browser.call('open')
        assert not opened['browserSandboxDisabledByHarness'] and not opened['automationDefaultsApplied']
        actual = browser.call('entry_release')
        assert actual == release, 'actual browser did not fetch the packaged release'
        invitation = self.private/'invitation'
        provider.call('issue_precode', path=str(invitation), seconds=300, max_uses=1)
        started = browser.call('entry_start', 'join', self.configuration, invitation.read_text())
        identity = started['id']
        assert started['active'] and identity != provider.identity
        assert browser.call('request_join', False) == 'Complete'
        saved = browser.call('entry_snapshot')
        assert saved['id'] == identity and saved['status']['stage'] == 'Complete'
        duplicate = browser.call('entry_duplicate')
        assert duplicate['denied'] and not duplicate['active'] and 'identity already active' in duplicate['reason']
        assert browser.call('entry_snapshot')['id'] == identity
        browser.call('enable_files')
        browser_address = {'id':identity,'addrs':[{'Relay':json.loads(self.configuration)['relays'][0]}]}
        provider.call('service_sync', address=browser_address)
        input_file = self.private/'native-to-browser.bin'
        payload = bytes(i % 251 for i in range(65539))
        input_file.write_bytes(payload)
        task = provider.call('file_send', address=browser_address, path=str(input_file))['task']
        def delivered():
            return next((t for t in provider.call('files_status')['tasks'] if t['id']==task and t['state']=='complete'),None)
        sent = None
        def complete():
            nonlocal sent
            sent = delivered()
            return sent
        self.until(complete, 'native-to-packaged-browser save incomplete', seconds=25)
        name = sent['receipt']['saved_name']
        expected = {'size':len(payload),'sha256':hashlib.sha256(payload).hexdigest()}
        assert browser.call('file_hash', name, False) == expected
        destination = self.private/'native-received'
        provider.call('files_enable', directory=str(destination))
        browser.call('sync_services', provider.identity)
        assert browser.call('file_make', 'browser-to-native.bin', 8193) == {'name':'browser-to-native.bin','size':8193}
        reverse = browser.call('file_send_source', provider.identity, 'browser-to-native.bin')
        self.until(lambda:any(t['id']==reverse and t['state']=='complete' for t in browser.call('file_status')['tasks']),
                   'packaged-browser-to-native save incomplete', seconds=25)
        files = list(destination.iterdir())
        assert len(files)==1 and files[0].read_bytes()==bytes(i % 251 for i in range(8193))
        self.record('observer','independent_duplex_files',{'browser_saved':expected,'native_saved_bytes':8193,
                    'native_saved_sha256':sha(files[0]),'browser_identity_distinct':True,'duplicate_tab_denied':True})
        assert browser.call('close_tab')['closedWithoutSdkClose']
        reopened = browser.call('entry_start', 'open', '{}', None)
        assert reopened['id']==identity and reopened['status']['stage']=='Complete'
        browser.call('sync_services', provider.identity)
        assert browser.call('file_hash',name,False)==expected
        restarted = browser.call('entry_restart_browser')
        assert restarted['oldExit']==0 and restarted['profileRetained'] and not restarted['reopened']['browserSandboxDisabledByHarness']
        after = browser.call('entry_start','open','{}',None)
        assert after['id']==identity and after['status']['stage']=='Complete'
        browser.call('sync_services', provider.identity)
        assert browser.call('file_hash',name,False)==expected
        self.record('observer','packaged_entry_lifecycle',{'page_destroyed_without_SDK_close':True,'same_identity_and_membership':True,
                    'full_browser_process_restarted':True,'saved_file_after_process_restart':expected,
                    'published':False,'PWA_install_tested':False})


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['binary','source-receipt','output','relay-binary','relay-receipt','package','browser-receipt','chromium','entry-site','entry-receipt']:
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--sha256',required=True)
    parser.set_defaults(case='browser_host')
    BrowserHostRunner(parser.parse_args()).run()
