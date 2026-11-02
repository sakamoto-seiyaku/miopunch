#!/usr/bin/env python3
"""Arm and observe real user logon startup; never simulate the logon trigger."""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import traceback


class Luid(ctypes.Structure):
    _fields_ = [('low', wintypes.DWORD), ('high', wintypes.LONG)]


class TokenStatistics(ctypes.Structure):
    _fields_ = [('token', Luid), ('authentication', Luid), ('expiry', ctypes.c_longlong),
                ('kind', wintypes.DWORD), ('impersonation', wintypes.DWORD),
                ('charged', wintypes.DWORD), ('available', wintypes.DWORD),
                ('groups', wintypes.DWORD), ('privileges', wintypes.DWORD), ('modified', Luid)]


def sha(path):
    with Path(path).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--manifest-sha', required=True)
    parser.add_argument('--phase', choices=['arm', 'on', 'off'], required=True)
    args = parser.parse_args()
    root = args.root
    assert sha(root/'manifest.json') == args.manifest_sha
    manifest = json.loads((root/'manifest.json').read_text(encoding='utf-8'))
    assert sha(root/'startup_helpers.py') == manifest['helper_sha256']
    assert sha(root/'login_controls.ps1') == manifest['controls_sha256']
    sys.path.insert(0, str(root))
    from startup_helpers import startup_values, windows, wait
    output = root/('evidence-'+args.phase)
    output.mkdir(exist_ok=False)
    report = dict(status='failed', phase=args.phase, clock_start=time.monotonic(),
                  clipboard_accessed=False, WSL_accessed=False, simulated_logon=False)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    security = ctypes.WinDLL('advapi32', use_last_error=True)
    user = ctypes.WinDLL('user32', use_last_error=True)
    kernel.GetCurrentProcess.restype = wintypes.HANDLE
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    security.OpenProcessToken.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(wintypes.HANDLE)]
    security.GetTokenInformation.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
    user.OpenInputDesktop.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    user.OpenInputDesktop.restype = wintypes.HANDLE
    user.GetUserObjectInformationW.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
    user.CloseDesktop.argtypes = [wintypes.HANDLE]
    process = None
    adopted = None
    streams = []
    sequence = 0
    verification = root/'verification-arm'
    quit_requested = False

    def record(name, value):
        (output/(name+'.json')).write_text(json.dumps(value, indent=2)+'\n', encoding='utf-8')

    def authentication(handle):
        token = wintypes.HANDLE()
        assert security.OpenProcessToken(handle, 8, ctypes.byref(token))
        try:
            data = TokenStatistics()
            length = wintypes.DWORD()
            assert security.GetTokenInformation(token, 10, ctypes.byref(data), ctypes.sizeof(data), ctypes.byref(length))
            return '%08x:%08x' % (data.authentication.high & 0xffffffff, data.authentication.low)
        finally:
            kernel.CloseHandle(token)

    def call(action, **fields):
        nonlocal sequence, quit_requested
        sequence += 1
        pending = verification/'request.next'
        pending.write_text(json.dumps(dict(id=sequence, action=action, **fields)), encoding='utf-8')
        pending.replace(verification/'request.json')
        result = verification/('result-%d.json' % sequence)
        value = wait(lambda: json.loads(result.read_text(encoding='utf-8')) if result.exists() else None,
                     'actual arm WebView IPC '+action, seconds=75)
        record('ipc-%d-%s' % (sequence, action), value)
        assert value['ok'], value
        if action == 'quit':
            quit_requested = True
        return value.get('result', value)

    def sdk(operation, **fields):
        value = call('request', request=dict(id=sequence+1, op=operation, **fields))
        assert value['ok'], value
        return value['result']

    def controls(hwnd, action):
        command = ['powershell.exe', '-NoLogo', '-NoProfile', '-NonInteractive', '-STA', '-File',
                   str(root/'login_controls.ps1'), '-Window', str(hwnd), '-Action', action]
        value = subprocess.run(command, capture_output=True, text=True, timeout=30)
        (output/('UI-'+action+'.stdout')).write_text(value.stdout, encoding='utf-8')
        (output/('UI-'+action+'.stderr')).write_text(value.stderr, encoding='utf-8')
        assert value.returncode == 0, 'actual UI Automation '+action+' failed'
        rows = [json.loads(line) for line in value.stdout.splitlines() if line.startswith('{')]
        assert len(rows) == 1 and rows[0]['status'] == 'pass'
        return rows[0]

    try:
        assert os.name == 'nt' and not ctypes.WinDLL('shell32').IsUserAnAdmin()
        session = wintypes.DWORD()
        assert kernel.ProcessIdToSessionId(os.getpid(), ctypes.byref(session)) and session.value > 0
        desktop = user.OpenInputDesktop(0, False, 1)
        assert desktop, 'actual unlocked input desktop required'
        try:
            name = ctypes.create_unicode_buffer(256)
            needed = wintypes.DWORD()
            assert user.GetUserObjectInformationW(desktop, 2, name, ctypes.sizeof(name), ctypes.byref(needed))
            assert name.value == 'Default', 'actual unlocked input desktop required'
            report['input_desktop'] = name.value
        finally:
            user.CloseDesktop(desktop)
        current_auth = authentication(kernel.GetCurrentProcess())
        report.update(interactive_session=session.value, interactive_authentication_id=current_auth)
        binary = root/'app files/miopunch-desktop.exe'
        assert sha(binary) == manifest['binary_sha256']
        assert sha(binary.with_name('WebView2Loader.dll')) == manifest['loader_sha256']
        if args.phase == 'arm':
            before = startup_values()
            assert all(value is None for value in before.values())
            profiles = [Path(os.environ[key])/'org.miopunch.host' for key in ['APPDATA', 'LOCALAPPDATA']]
            assert all(not path.exists() for path in profiles), 'preserve preexisting profiles'
            report.update(startup_before=before, new_profile_candidates=[str(path) for path in profiles])
            verification.mkdir(exist_ok=False)
            streams = [(output/('app.'+suffix)).open('wb') for suffix in ['stdout', 'stderr']]
            process = subprocess.Popen([str(binary), '--data-dir', str(root/'node state'),
                                        '--verification-dir', str(verification)], stdin=subprocess.DEVNULL,
                                       stdout=streams[0], stderr=streams[1])
            report['original_handle'] = int(process._handle)
            events = verification/'events.jsonl'
            wait(lambda: events.exists() and any(json.loads(line)['event'] == 'webview_ready'
                 for line in events.read_text(encoding='utf-8').splitlines()), 'actual arm WebView ready')
            assert call('state')['autostart'] is True
            invitation = root/'invitation.json'
            assert sha(invitation) == manifest['invitation_sha256']
            ready = call('start', mode={'action':'join', 'invitation':invitation.read_text(encoding='utf-8')}, resources=manifest['resources'])
            assert sdk('request', path=str(invitation))['stage'] == 'Complete'
            assert sdk('ping', address=manifest['provider_address'])['reply'] == 'PONG'
            registered = startup_values()
            assert registered[r'HKCU:Software\Microsoft\Windows\CurrentVersion\Run']['data'].startswith('"')
            report.update(identity=ready['status']['node'], address=ready['address'], registered=registered)
            assert call('quit')['stopped']
            assert process.wait(timeout=20) == 0
            assert startup_values() == registered
            report.update(status='pass', armed_for_actual_logon=True, application_exit=0,
                          initial_PING=True, identity_saved_before_logout=True)
        elif args.phase == 'on':
            arm = json.loads((root/'evidence-arm/result.json').read_text(encoding='utf-8'))
            observed = json.loads((root/'startup-observed.json').read_text(encoding='utf-8'))
            assert arm['status'] == 'pass' and current_auth != arm['interactive_authentication_id']
            assert startup_values() == arm['registered']
            adopted = kernel.OpenProcess(0x1000 | 0x100000, False, observed['pid'])
            assert adopted and authentication(adopted) == current_auth
            assert not [window for window in windows(observed['pid']) if window['visible']], 'login must start silently'
            helper = subprocess.Popen([str(binary), '--data-dir', str(root/'node state')],
                                      stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            assert helper.wait(timeout=20) == 0
            mine = wait(lambda: [window for window in windows(observed['pid']) if window['visible'] and window['title'] == 'miopunch host'],
                        'same login owner actual window')
            controls(mine[0]['hwnd'], 'inspect')
            disabled = controls(mine[0]['hwnd'], 'off')
            assert disabled['before'] == 'On' and disabled['after'] == 'Off'
            wait(lambda: all(value is None for value in startup_values().values()), 'actual UI removed startup registration')
            controls(mine[0]['hwnd'], 'quit')
            assert kernel.WaitForSingleObject(adopted, 20000) == 0
            exit_code = wintypes.DWORD()
            assert kernel.GetExitCodeProcess(adopted, ctypes.byref(exit_code)) and exit_code.value == 0
            report.update(status='pass', actual_new_interactive_logon=True, login_owner_pid=observed['pid'],
                          no_initial_window=True, same_process_window_reopen=True, real_UI_autostart_off=True,
                          real_UI_quit=True, adopted_original_handle=int(adopted), application_exit=exit_code.value,
                          startup_after=startup_values(), authenticated_SDK_PING_observed_by_Linux=observed['authenticated_SDK_PING'])
            assert report['authenticated_SDK_PING_observed_by_Linux']
        else:
            on = json.loads((root/'evidence-on/result.json').read_text(encoding='utf-8'))
            assert on['status'] == 'pass' and current_auth != on['interactive_authentication_id']
            assert all(value is None for value in startup_values().values())
            # Observe process absence from the actual new interactive session;
            # never launch the application to manufacture this negative case.
            code = "$ErrorActionPreference='Stop';$items=@(Get-CimInstance Win32_Process|Where-Object{$_.Name -eq 'miopunch-desktop.exe'}|Select-Object ProcessId);ConvertTo-Json -InputObject $items -Compress"
            start = time.monotonic()
            observations = 0
            while time.monotonic()-start < 15:
                value = subprocess.run(['powershell.exe', '-NoLogo', '-NoProfile', '-NonInteractive', '-Command', code], capture_output=True, text=True, timeout=15)
                assert value.returncode == 0 and json.loads(value.stdout.strip()) == []
                observations += 1
                time.sleep(.2)
            report.update(status='pass', actual_new_interactive_logon=True, application_launched_by_probe=False,
                          disabled_no_application_seconds=time.monotonic()-start, process_absence_observations=observations,
                          startup_after=startup_values())
    except BaseException as failure:
        report.update(failure_type=type(failure).__name__, failure=str(failure), traceback=traceback.format_exc())
    finally:
        errors = []
        if process is not None and process.poll() is None:
            try:
                call('autostart', enabled=False)
                if not quit_requested:
                    call('quit')
                assert process.wait(timeout=20) == 0
            except BaseException as failure:
                errors.append('retain original SDK owner for normal settlement: '+str(failure))
        for stream in streams:
            stream.close()
        if adopted:
            kernel.CloseHandle(adopted)
        report.update(cleanup_errors=errors, clock_end=time.monotonic(), SDK_OS_signals=[])
        if errors:
            report['status'] = 'failed'
        record('result', report)
    return 0 if report['status'] == 'pass' else 1


if __name__ == '__main__':
    raise SystemExit(main())
