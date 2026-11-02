#!/usr/bin/env python3
"""One ordinary real WASM content request, closed at its owned gateway."""
import argparse
import asyncio
import base64
import hashlib
import json
from pathlib import Path
import secrets
import ssl
import time
from urllib.request import HTTPSHandler, ProxyHandler, build_opener

from check_web_browser import BrowserFrame, BrowserRunner
from check_web_service import OwnedNode
from forwarding_local_resources import configure_local_resources
from web_browser_runtime import BrowserWebRequests, WebBrowser
from web_content_gateway import ContentGateway
from web_process_owner import register_process
from web_protocol_target import WebTarget


REPO = Path('/home/js/Git/miopunch')
INSTALLED_CORE = REPO / 'browser-probe/harness/node_modules/playwright-core/lib/coreBundle.js'
CAPTURED_CORE = REPO / '.artifacts/runs/m119-browser-current-5/controller-sources/browser-probe/harness/node_modules/playwright-core/lib/coreBundle.js'
CORE_SHA = '549070af3acabb3efcc4f55bfe6210f9f7c2fcf633cf7eaa59bfe60719969171'


class PendingBrowserRunner(BrowserRunner):
    def freeze_source(self, path):
        path = Path(path).resolve()
        if path != INSTALLED_CORE.resolve():
            return super().freeze_source(path)
        for source in [path, CAPTURED_CORE]:
            assert source.stat().st_size == 3477183
            assert hashlib.sha256(source.read_bytes()).hexdigest() == CORE_SHA
        previous = self.controller_sources.get(str(path))
        assert previous in {None, CORE_SHA}, 'installed core changed during run'
        self.controller_sources[str(path)] = CORE_SHA
        (self.output / 'controller-start.json').write_text(json.dumps(self.controller_sources, indent=2) + '\n')
        reference = {'installed_path': str(path), 'retained_full_source_path': str(CAPTURED_CORE),
                     'bytes': 3477183, 'sha256': CORE_SHA, 'full_source_preserved': True,
                     'new_reference_only_no_existing_path_mutation': True}
        (self.output / 'source-references.json').write_text(json.dumps(reference, indent=2) + '\n')

    def __init__(self, args):
        super().__init__(args)
        for name in ['check_web_browser.py', 'check_web_container.py', 'web_browser_runtime.py', 'web_asyncio_observer.py']:
            self.freeze_source(Path(__file__).with_name(name))

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = WebTarget(self, tls=True)
        verification = ssl.create_default_context(cafile=target.certificate)
        with build_opener(ProxyHandler({}), HTTPSHandler(context=verification)).open(target.url, timeout=3) as response:
            assert response.read() == b'owned M119 web target'
        asyncio.run(target.socket_prerequisite())
        self.record('controller', 'independent_browser_target_precondition', {
            'https_exact': True, 'normal_tls_validation': True,
            'real_websocket_handshake_and_bidirectional_message': True})
        provider = OwnedNode(self, 'provider')
        provider.call('web_enable', targets=[target.url], trust_roots_pem=[target.trust_root_pem])
        holder = {}
        requests = BrowserWebRequests(provider.identity)
        request = requests.request

        callbacks = {
            'open': lambda url, headers: holder['browser'].call('open_web_socket', provider.identity, url, json.dumps(headers)),
            'send': lambda identifier, message: holder['browser'].call('send_web_socket', identifier, json.dumps(message)),
            'read': lambda identifier: holder['browser'].call('read_web_socket', identifier, 100),
            'close': lambda identifier: holder['browser'].call('close_web_socket', identifier),
        }
        gateway = ContentGateway(target.url + 'page', request, self.private, self.nodes,
                                 socket_callbacks=callbacks, browser_origins=True, http_callbacks=requests.callbacks)
        gateway.record = lambda operation, result: self.record('controller', operation, result)
        browser = WebBrowser(self, self.args, gateway)
        holder['browser'] = browser
        requests.bind(browser)
        opened = browser.call('open')
        assert not opened['browserSandboxDisabledByHarness']
        invitation = self.private / 'browser-invitation'
        provider.call('issue_precode', path=str(invitation), seconds=600, max_uses=1)
        status = browser.call('start', 'join', self.configuration, invitation.read_text())
        assert status['id'] != provider.identity
        assert browser.call('request_join', False) == 'Complete'
        browser.call('sync_services', provider.identity)
        self.record('controller', 'independent_wasm_admission_precondition', {
            'distinct_identity': True, 'real_admission_complete': True, 'real_signed_service_sync': True})
        response = request(dict(url=target.url, method='GET', headers=[], body=''))
        assert response['status'] == 200 and base64.b64decode(response['body']) == b'owned M119 web target'
        self.record('controller', 'public_browser_web_delivery', {'target_observed': True, 'response_exact': True})
        browser.call('content_open', gateway.bootstrap)
        frame = BrowserFrame(browser)
        canary = secrets.token_hex(16)
        assert browser.call('management_prepare', canary)['saved']
        self.until(lambda: (frame.evaluate('({ready:document.readyState,fetch:window.fixtureFetch||null})') or {}).get('fetch')
                   == 'owned M119 fetched resource', 'actual content script/fetch did not complete')
        self.until(lambda: browser.call('web_status')['active_operations'] == 0
                   and provider.call('web_status')['active_flows'] == 0, 'ordinary bootstrap owners did not settle')
        assert target.requests.count('/pending') == 0 and not target.pending_release.is_set()
        assert frame.evaluate("(window.pendingFetchOutcome=null,fetch('/pending').then(r=>{window.pendingFetchOutcome=!r.ok;},()=>{window.pendingFetchOutcome=true;}),true)")
        self.until(target.pending_started.is_set, 'real content did not reach pending upstream HTTP')
        assert target.requests.count('/pending') == 1 and not target.pending_release.is_set()
        live = browser.call('owned_web_liveness')
        assert live == {'nodePresent': True, 'browserConnected': True, 'pageOpen': True, 'contentFramePresent': True}, live
        assert browser.process.poll() is None and frame.evaluate('window.pendingFetchOutcome===null')
        self.record('controller', 'actual_browser_pending_before_gateway_close', {
            'one_target_pending_request': True, 'pending_target_not_released': True, 'liveness': live,
            'browser_host_original_pid': browser.process.pid, 'no_node_or_renderer_close_requested': True,
            'binding': 'real BrowserWebRequests begin/result/cancel Interface; no node cancellation substitute'})
        started = time.monotonic()
        try:
            cleanup = gateway.close()
        except BaseException:
            self.record('controller', 'content_gateway_early_close_failure', getattr(gateway, 'cleanup', {}))
            raise
        self.record('controller', 'content_gateway_early_close', cleanup)
        self.nodes.remove(gateway)
        close_seconds = time.monotonic() - started
        self.record('controller', 'browser_gateway_close_returned', {
            'close_seconds': close_seconds, 'original_close_limit': 5,
            'pending_target_not_released': not target.pending_release.is_set(),
            'browser_host_still_alive': browser.process.poll() is None})
        assert close_seconds < 5, 'pending content close exceeded owner bound'
        self.until(lambda: browser.call('web_status')['active_operations'] == 0
                   and provider.call('web_status')['active_flows'] == 0, 'content close left module owners')
        self.until(lambda: frame.evaluate('window.pendingFetchOutcome===true'),
                   'content received false success after pending close')
        live = browser.call('owned_web_liveness')
        assert all(live.values()) and not target.pending_release.is_set()
        assert all(browser.call('management_check', canary).values())
        self.record('controller', 'actual_content_pending_close', {
            'platform': frame.platform, 'target_observed_pending': True, 'close_seconds': time.monotonic()-started,
            'all_module_owners_joined': True, 'liveness_after_gateway_close': live,
            'pending_target_not_released': True})
        self.exercise_request_ownership(browser, provider, target, status['id'])

    def exercise_request_ownership(self, browser, provider, target, identity):
        def arguments(path='/'):
            return json.dumps(dict(url=target.url.rstrip('/') + path, method='GET', headers=[], body=''))

        def result(identifier):
            return browser.call('web_request_result', identifier)

        def response(identifier):
            holder = {}
            def completed():
                value = result(identifier)
                if not value['pending']:
                    holder.update(value['response'])
                    return True
                return False
            self.until(completed, 'owned request handle never completed')
            assert holder['status'] == 200 and base64.b64decode(holder['body']) == b'owned M119 web target'
            return holder

        # A fresh independently observed response on the very same admitted node
        # distinguishes per-request cancellation from disconnect/reopen.
        before = target.requests.count('/after-gateway-close')
        returned = browser.call('request_web', provider.identity, arguments('/after-gateway-close'))
        assert returned['status'] == 200 and base64.b64decode(returned['body']) == b'owned M119 web target'
        assert target.requests.count('/after-gateway-close') == before + 1
        assert browser.call('status')['id'] == identity
        self.record('controller', 'same_node_request_after_gateway_cancel', {
            'identity_unchanged': True, 'independent_target_observed': True, 'response_exact': True})

        pending_before = target.requests.count('/pending')
        first = browser.call('begin_web_request', provider.identity, arguments('/pending'))['id']
        other = browser.call('begin_web_request', provider.identity, arguments('/pending'))['id']
        assert first != other
        self.until(lambda: target.requests.count('/pending') == pending_before + 2,
                   'two separate public HTTP owners did not reach the independent target')
        assert result(first)['pending'] and result(other)['pending']
        started = time.monotonic()
        assert browser.call('cancel_web_request', first) == {'cancelled': True, 'joined': True}
        assert time.monotonic() - started < 5
        browser.call('web_request_result', first, ok=False)
        browser.call('cancel_web_request', first, ok=False)
        browser.call('web_request_result', '18446744073709551615', ok=False)
        browser.call('cancel_web_request', '18446744073709551615', ok=False)
        assert result(other)['pending']
        assert browser.call('web_status')['active_operations'] == 1
        assert browser.call('web_status')['request_handles'] == 1
        socket = browser.call('open_web_socket', provider.identity, target.socket_url, '[]')['id']
        assert browser.call('read_web_socket', socket, 2000)['message'] == {'type': 'text', 'text': 'owned M119 ready'}
        browser.call('web_request_result', socket, ok=False)
        browser.call('cancel_web_request', socket, ok=False)
        browser.call('send_web_socket', socket, json.dumps({'type': 'text', 'text': 'M119 separate owner'}))
        assert browser.call('read_web_socket', socket, 2000)['message'] == {'type': 'text', 'text': 'owned-target:M119 separate owner'}
        assert browser.call('close_web_socket', socket) == {'closed': True, 'failed': False}
        assert result(other)['pending']
        complete = browser.call('begin_web_request', provider.identity, arguments('/complete-while-other-pending'))['id']
        response(complete)
        browser.call('web_request_result', complete, ok=False)
        browser.call('cancel_web_request', complete, ok=False)
        assert result(other)['pending']
        assert browser.call('cancel_web_request', other) == {'cancelled': True, 'joined': True}
        self.until(lambda: browser.call('web_status')['active_operations'] == 0
                   and provider.call('web_status')['active_flows'] == 0, 'exact request cancellation retained SDK owners')
        assert browser.call('web_status')['request_handles'] == 0
        assert not target.pending_release.is_set()
        self.record('controller', 'exact_request_owner_cancel_and_stale_handles', {
            'one_cancel_did_not_cancel_other': True, 'unknown_and_consumed_handles_rejected': True,
            'cross_kind_handle_did_not_cancel_socket': True, 'response_consumed_once': True,
            'node_identity_unchanged': browser.call('status')['id'] == identity})

        denied_target = WebTarget(self)
        with build_opener(ProxyHandler({})).open(denied_target.url, timeout=3) as direct:
            assert direct.read() == b'owned M119 web target'
        denied_before = list(denied_target.requests)
        assert denied_before == ['/']
        denied_request = json.dumps(dict(url=denied_target.url, method='GET', headers=[], body=''))
        denied = browser.call('begin_web_request', provider.identity, denied_request)['id']
        self.until(lambda: browser.call('web_status')['active_operations'] == 0,
                   'provider-rejected HTTP owner did not settle')
        assert browser.call('web_status')['request_handles'] == 1
        browser.call('web_request_result', denied, ok=False)
        browser.call('web_request_result', denied, ok=False)
        browser.call('cancel_web_request', denied, ok=False)
        assert browser.call('web_status')['request_handles'] == 0
        assert denied_target.requests == denied_before and browser.call('status')['id'] == identity
        self.record('controller', 'failed_request_result_consumed_once', {
            'current_provider_origin_rule_rejected': True, 'denied_target_no_effect': True,
            'denied_target_independently_live_exact': True,
            'error_releases_only_its_handle': True, 'same_node_remains_usable': True})

        # Completed ordinary responses occupy the same fixed eight slots until
        # consumed. This is a handle-limit check with no pressured transport.
        maximum = browser.call('web_status')['max_request_handles']
        assert maximum == 8
        completed = [browser.call('begin_web_request', provider.identity, arguments('/retained-%d' % index))['id']
                     for index in range(maximum)]
        self.until(lambda: browser.call('web_status')['active_operations'] == 0,
                   'ordinary completed owners did not settle')
        assert browser.call('web_status')['request_handles'] == maximum
        before = len(target.requests)
        browser.call('begin_web_request', provider.identity, arguments('/must-not-reach-full-handle-target'), ok=False)
        assert len(target.requests) == before
        for identifier in completed:
            response(identifier)
            browser.call('web_request_result', identifier, ok=False)
        assert browser.call('web_status')['request_handles'] == 0
        pending_before = target.requests.count('/pending')
        stopped = browser.call('begin_web_request', provider.identity, arguments('/pending'))['id']
        discarded = browser.call('begin_web_request', provider.identity, arguments('/discarded-by-stop'))['id']
        self.until(lambda: target.requests.count('/pending') == pending_before + 1
                   and browser.call('web_status')['active_operations'] == 1
                   and browser.call('web_status')['request_handles'] == 2,
                   'stop precondition did not retain one pending and one completed request')
        started = time.monotonic()
        browser.call('stop_web')
        assert time.monotonic() - started < 5
        browser.call('web_request_result', stopped, ok=False)
        browser.call('cancel_web_request', stopped, ok=False)
        browser.call('web_request_result', discarded, ok=False)
        status = browser.call('web_status')
        assert status['active_operations'] == status['request_handles'] == status['socket_handles'] == 0
        self.until(lambda: provider.call('web_status')['active_flows'] == 0,
                   'stopped Web request retained the provider owner')
        assert not target.pending_release.is_set()
        self.record('controller', 'stop_web_joins_pending_and_clears_retained_requests', {
            'pending_owner_joined': True, 'completed_handle_discarded': True,
            'old_handles_stale': True, 'node_still_open': True})
        assert browser.call('status')['id'] == identity
        final = browser.call('request_web', provider.identity, arguments('/after-exact-owner-checks'))
        assert final['status'] == 200 and base64.b64decode(final['body']) == b'owned M119 web target'
        assert '/after-exact-owner-checks' in target.requests and not target.pending_release.is_set()
        self.until(lambda: browser.call('web_status')['active_operations'] == 0
                   and provider.call('web_status')['active_flows'] == 0, 'final Web operations did not settle')
        status = browser.call('web_status')
        assert status['request_handles'] == status['socket_handles'] == 0
        status = provider.call('web_status')
        assert status['request_handles'] == status['socket_handles'] == 0
        self.record('controller', 'retained_request_limit_and_same_node_followup', {
            'completed_handles_bounded': maximum, 'overflow_no_target_effect': True,
            'all_results_consumed_once': True, 'same_node_followup_exact': True})


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--source-receipt', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--relay-binary', type=Path, required=True)
    parser.add_argument('--relay-receipt', type=Path, required=True)
    parser.add_argument('--package', type=Path, required=True)
    parser.add_argument('--browser-receipt', type=Path, required=True)
    parser.add_argument('--chromium', type=Path, required=True)
    parser.add_argument('--case', default='pending-close')
    PendingBrowserRunner(parser.parse_args()).run()
