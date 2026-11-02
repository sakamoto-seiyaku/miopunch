#!/usr/bin/env python3
"""Actual interactive Windows Tauri startup/registration probe; no clipboard.

Runs in one newly registered least-privilege interactive scheduled task. Uses
original Popen handles and a private WebView mailbox. Refuses a writable HKLM
Run key or any preexisting miopunch startup value before launching the app.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import traceback
import winreg

RUN = r"Software\Microsoft\Windows\CurrentVersion\Run"
APPROVED = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read_value(root, path):
    try:
        with winreg.OpenKey(root, path, 0, winreg.KEY_READ) as key:
            data, kind = winreg.QueryValueEx(key, 'miopunch')
            return {'data':data.hex() if isinstance(data,bytes) else data,'kind':kind}
    except FileNotFoundError:
        return None


def startup_values():
    return {root_name+':'+path:read_value(root,path) for root_name,root in [('HKCU',winreg.HKEY_CURRENT_USER),('HKLM',winreg.HKEY_LOCAL_MACHINE)] for path in [RUN,APPROVED]}


def wait(predicate, label, seconds=25):
    deadline = time.monotonic()+seconds
    while time.monotonic()<deadline:
        value=predicate()
        if value:return value
        time.sleep(.05)
    raise TimeoutError(label)


def windows(pid):
    user32=ctypes.WinDLL('user32',use_last_error=True)
    callback=ctypes.WINFUNCTYPE(ctypes.c_int,ctypes.c_void_p,ctypes.c_void_p)
    result=[]
    def visit(hwnd,_):
        actual=ctypes.c_uint32()
        user32.GetWindowThreadProcessId(ctypes.c_void_p(hwnd),ctypes.byref(actual))
        if actual.value==pid:
            title=ctypes.create_unicode_buffer(256)
            user32.GetWindowTextW(ctypes.c_void_p(hwnd),title,len(title))
            result.append({'hwnd':int(hwnd),'visible':bool(user32.IsWindowVisible(ctypes.c_void_p(hwnd))),'title':title.value})
        return True
    user32.EnumWindows(callback(visit),None)
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,required=True)
    parser.add_argument('--binary-sha',required=True)
    parser.add_argument('--loader-sha',required=True)
    args=parser.parse_args()
    root=args.root
    binary=root/'app files/miopunch-desktop.exe'
    loader=binary.with_name('WebView2Loader.dll')
    output=root/'evidence'
    output.mkdir(exist_ok=False)
    verification=root/'verification'
    verification.mkdir(exist_ok=False)
    report={'status':'failed','clock_start':time.monotonic(),'binary_sha256':args.binary_sha,'loader_sha256':args.loader_sha,
        'clipboard_accessed':False,'WSL_accessed':False,'actual_login_trigger':False}
    main_process=None
    startup_process=None
    handles={}
    streams=[]
    sequence=0
    before=None
    def record(name,value):
        (output/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    def call(action,**fields):
        nonlocal sequence
        sequence+=1
        temporary=verification/'request.next'
        temporary.write_text(json.dumps({'id':sequence,'action':action,**fields}),encoding='utf-8')
        temporary.replace(verification/'request.json')
        path=verification/('result-%d.json'%sequence)
        def complete():
            if path.exists():return json.loads(path.read_text(encoding='utf-8'))
            if main_process.poll() is not None:raise RuntimeError('original desktop exited before IPC reply')
        result=wait(complete,'actual WebView IPC '+action)
        record('ipc-%d-%s'%(sequence,action),result)
        assert result['ok'],result
        return result.get('result',result)
    try:
        assert os.name=='nt' and sha(binary)==args.binary_sha and sha(loader)==args.loader_sha
        assert not ctypes.WinDLL('shell32').IsUserAnAdmin(),'interactive token is elevated; current plugin must not write HKLM'
        try:
            key=winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,RUN,0,winreg.KEY_SET_VALUE)
        except PermissionError:
            pass
        else:
            key.Close()
            raise RuntimeError('HKLM startup key writable; refuse current dynamic-registration plugin')
        before=startup_values()
        assert all(value is None for value in before.values()),'preexisting miopunch startup value'
        report['startup_before']=before
        session=ctypes.c_uint32()
        assert ctypes.WinDLL('kernel32').ProcessIdToSessionId(os.getpid(),ctypes.byref(session))
        assert session.value>0,'actual interactive session required'
        report.update(controller_session=session.value,non_elevated=True,hklm_write_denied=True)
        # Record possible application-owned WebView profile roots before launch.
        profiles=[Path(os.environ[name])/'org.miopunch.host' for name in ['APPDATA','LOCALAPPDATA']]
        assert all(not p.exists() for p in profiles),'preexisting application profile must be preserved'
        report['new_profile_candidates']=[str(p) for p in profiles]
        streams=[(output/'main.stdout').open('wb'),(output/'main.stderr').open('wb')]
        command=[str(binary),'--data-dir',str(root/'node state'),'--verification-dir',str(verification)]
        main_process=subprocess.Popen(command,stdin=subprocess.DEVNULL,stdout=streams[0],stderr=streams[1])
        handles['main']=int(main_process._handle)
        report['main']={'pid':main_process.pid,'original_handle':handles['main'],'argv':command}
        ready=verification/'events.jsonl'
        def loaded():
            if main_process.poll() is not None:raise RuntimeError('desktop startup failed: '+str(main_process.returncode))
            return ready.exists() and any(json.loads(line)['event']=='webview_ready' for line in ready.read_text(encoding='utf-8').splitlines())
        wait(loaded,'real Windows WebView readiness')
        observed=wait(lambda:[w for w in windows(main_process.pid) if w['visible'] and w['title']=='miopunch host'],'independent own HWND')
        record('window-observation',{'pid':main_process.pid,'windows':observed})
        assert call('state')['autostart'] is True
        registered=startup_values()
        report['registered']=registered
        assert registered['HKLM:'+RUN] is None and registered['HKLM:'+APPROVED] is None
        command_line=registered['HKCU:'+RUN]['data']
        assert isinstance(command_line,str) and str(binary) in command_line and '--background' in command_line
        startup_out=(output/'startup.stdout').open('wb');startup_err=(output/'startup.stderr').open('wb');streams.extend([startup_out,startup_err])
        # Passing the actual REG_SZ string directly exercises CreateProcessW
        # command-line parsing. This is not a login-trigger qualification.
        startup_process=subprocess.Popen(command_line,stdin=subprocess.DEVNULL,stdout=startup_out,stderr=startup_err)
        handles['startup']=int(startup_process._handle)
        report['startup']={'pid':startup_process.pid,'original_handle':handles['startup'],'registered_command':command_line}
        exit_code=startup_process.wait(timeout=15)
        report['startup']['exit']=exit_code
        assert exit_code==0,'registered startup command did not reach the existing host'
        assert main_process.poll() is None
        native=[json.loads(line) for line in ready.read_text(encoding='utf-8').splitlines()]
        assert any(e['event']=='secondary_launch' and e['detail']['background'] for e in native)
        report['status']='pass'
    except BaseException as failure:
        report.update(failure_type=type(failure).__name__,failure=str(failure),traceback=traceback.format_exc())
    finally:
        cleanup=[]
        if startup_process is not None and startup_process.poll() is None:
            assert int(startup_process._handle)==handles['startup']
            startup_process.terminate();startup_process.wait(timeout=10)
            cleanup.append('forced original startup handle; not clean')
        if main_process is not None and main_process.poll() is None:
            try:
                assert call('autostart',enabled=False) is False
                assert call('quit')['stopped'] is True
                main_process.wait(timeout=20)
            except BaseException as failure:
                cleanup.append('normal desktop cleanup: '+str(failure))
                if main_process.poll() is None:
                    assert int(main_process._handle)==handles['main']
                    main_process.terminate();main_process.wait(timeout=10)
                    cleanup.append('forced original desktop handle; not clean')
        if before is not None:
            # The initial plugin may create its own StartupApproved bytes. Only
            # remove the exact newly-created value, never another registry item.
            current=read_value(winreg.HKEY_CURRENT_USER,APPROVED)
            if before['HKCU:'+APPROVED] is None and current is not None:
                expected={'data':(b'\x02'+b'\0'*11).hex(),'kind':winreg.REG_BINARY}
                if current==expected:
                    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,APPROVED,0,winreg.KEY_SET_VALUE) as key:winreg.DeleteValue(key,'miopunch')
                else:cleanup.append('startup approval changed; retained')
            report['startup_after']=startup_values()
            if report['startup_after']!=before:cleanup.append('startup state not restored')
        for stream in streams:stream.close()
        if (verification/'events.jsonl').exists():
            (output/'native-events.jsonl').write_bytes((verification/'events.jsonl').read_bytes())
        report.update(main_exit=None if main_process is None else main_process.poll(),startup_exit=None if startup_process is None else startup_process.poll(),cleanup_errors=cleanup,clock_end=time.monotonic())
        if cleanup:report['status']='failed'
        record('result',report)
    return 0 if report['status']=='pass' else 1


if __name__=='__main__':
    raise SystemExit(main())
