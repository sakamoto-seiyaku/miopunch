#!/usr/bin/env python3
"""Own HWND foreground lifecycle in the user's unlocked Windows session."""
import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time
import traceback


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--root',type=Path,required=True)
    parser.add_argument('--binary-sha',required=True)
    parser.add_argument('--identity',required=True)
    parser.add_argument('--helper-sha',required=True)
    parser.add_argument('--foreground-seconds',type=float,default=65)
    args=parser.parse_args();root=args.root;parent=root.parent
    assert hashlib.sha256((root/'startup_helpers.py').read_bytes()).hexdigest()==args.helper_sha
    sys.path.insert(0,str(root))
    from startup_helpers import windows, wait, startup_values
    output=root/'evidence';output.mkdir()
    verification=root/'verification';verification.mkdir()
    user=ctypes.WinDLL('user32',use_last_error=True)
    user.OpenInputDesktop.argtypes=[ctypes.c_uint32,ctypes.c_bool,ctypes.c_uint32]
    user.OpenInputDesktop.restype=ctypes.c_void_p
    user.GetUserObjectInformationW.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_void_p,ctypes.c_uint32,ctypes.POINTER(ctypes.c_uint32)]
    user.CloseDesktop.argtypes=[ctypes.c_void_p]
    user.GetForegroundWindow.restype=ctypes.c_void_p
    user.SetForegroundWindow.argtypes=[ctypes.c_void_p]
    user.SetForegroundWindow.restype=ctypes.c_bool
    process=None;helper=None;streams=[];sequence=0;quit_requested=False
    report={'status':'failed','clock_start':time.monotonic(),'clipboard_accessed':False,'actual_login_trigger':False}
    before=startup_values()
    def record(name,value):(output/(name+'.json')).write_text(json.dumps(value,indent=2)+'\n',encoding='utf-8')
    def call(action):
        nonlocal sequence,quit_requested
        sequence+=1;p=verification/'request.next';p.write_text(json.dumps({'id':sequence,'action':action}),encoding='utf-8');p.replace(verification/'request.json')
        response=verification/('result-%d.json'%sequence)
        value=wait(lambda:json.loads(response.read_text(encoding='utf-8')) if response.exists() else None,'foreground IPC '+action,seconds=75)
        record('ipc-%d-%s'%(sequence,action),value);assert value['ok'],value
        if action=='quit':quit_requested=True
        return value.get('result',value)
    def own_foreground(label):
        mine=wait(lambda:[w for w in windows(process.pid) if w['visible'] and w['title']=='miopunch host'],label+' visible own HWND')
        hwnd=mine[0]['hwnd']
        requested=bool(user.SetForegroundWindow(ctypes.c_void_p(hwnd)))
        observations=[]
        def focused():
            actual=user.GetForegroundWindow()
            owner=ctypes.c_uint32()
            if actual:user.GetWindowThreadProcessId(ctypes.c_void_p(actual),ctypes.byref(owner))
            value={'own_hwnd':hwnd,'foreground_hwnd':actual,'foreground_pid':owner.value}
            if not observations or observations[-1]!=value:observations.append(value)
            return mine if actual==hwnd else None
        record(label+'-visible',{'windows':mine,'SetForegroundWindow_returned':requested})
        try:
            return wait(focused,label+' actual owned foreground HWND',seconds=args.foreground_seconds)
        finally:
            record(label+'-focus-observations',observations)
    try:
        desktop=user.OpenInputDesktop(0,False,1)
        assert desktop,'input desktop is unavailable'
        try:
            name=ctypes.create_unicode_buffer(256);needed=ctypes.c_uint32()
            assert user.GetUserObjectInformationW(desktop,2,name,ctypes.sizeof(name),ctypes.byref(needed))
            report['input_desktop']=name.value
            assert name.value=='Default','unlocked input desktop required'
        finally:user.CloseDesktop(desktop)
        binary=parent/'app files/miopunch-desktop.exe'
        assert hashlib.sha256(binary.read_bytes()).hexdigest()==args.binary_sha
        streams=[(output/('main.'+suffix)).open('wb') for suffix in ['stdout','stderr']]
        process=subprocess.Popen([str(binary),'--data-dir',str(parent/'node state'),'--verification-dir',str(verification)],stdin=subprocess.DEVNULL,stdout=streams[0],stderr=streams[1])
        handle=int(process._handle);report['original_handle']=handle;report['pid']=process.pid
        events=verification/'events.jsonl'
        wait(lambda:events.exists() and any(json.loads(l)['event']=='webview_ready' for l in events.read_text(encoding='utf-8').splitlines()),'foreground WebView')
        first=own_foreground('initial')
        state=call('state');assert state['node']['result']['node']==args.identity
        call('close')
        wait(lambda:not any(w['visible'] and w['title']=='miopunch host' for w in windows(process.pid)),'actual HWND gone')
        assert process.poll() is None
        helper=subprocess.Popen([str(binary),'--data-dir',str(parent/'node state')],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        helper_handle=int(helper._handle);helper.wait(timeout=20);assert helper.returncode==0
        wait(lambda:len([l for l in events.read_text(encoding='utf-8').splitlines() if json.loads(l)['event']=='webview_ready'])==2,'reopened WebView')
        second=own_foreground('reopened')
        after=call('state');assert after['pid']==state['pid'] and after['node']['result']['node']==args.identity
        assert after['windows_created']==state['windows_created']+1
        assert after['windows_ready']==state['windows_ready']+1
        assert call('quit')['stopped'];process.wait(timeout=20);assert process.returncode==0
        report.update(status='pass',first_foreground=first,reopened_foreground=second,same_process=True,same_identity=True,
            main_exit=process.returncode,helper_exit=helper.returncode,helper_original_handle=helper_handle)
    except BaseException as failure:report.update(failure_type=type(failure).__name__,failure=str(failure),traceback=traceback.format_exc())
    finally:
        errors=[]
        if process is not None and process.poll() is None:
            try:
                if not quit_requested:call('quit')
                process.wait(timeout=20)
            except BaseException as failure:
                errors.append(str(failure))
                if process.poll() is None:
                    assert int(process._handle)==handle;process.terminate();process.wait(timeout=10);errors.append('forced original handle')
        for stream in streams:stream.close()
        if (verification/'events.jsonl').exists():(output/'native-events.jsonl').write_bytes((verification/'events.jsonl').read_bytes())
        report.update(startup_unchanged=startup_values()==before,cleanup_errors=errors,clock_end=time.monotonic())
        if errors or not report['startup_unchanged']:report['status']='failed'
        record('result',report)
    return 0 if report['status']=='pass' else 1


if __name__=='__main__':raise SystemExit(main())
