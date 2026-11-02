#!/usr/bin/env python3
"""Real interactive Tauri node, remote SDK files, background/reopen and quit."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import traceback
import winreg


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,required=True)
    parser.add_argument('--manifest-sha',required=True)
    args=parser.parse_args()
    root=args.root
    manifest=root/'manifest.json'
    assert sha(manifest)==args.manifest_sha
    configuration=json.loads(manifest.read_text(encoding='utf-8'))
    assert sha(root/'startup_helpers.py')==configuration['helper_sha256']
    sys.path.insert(0,str(root))
    from startup_helpers import startup_values, read_value, windows, wait, RUN, APPROVED
    output=root/'evidence'
    output.mkdir(exist_ok=False)
    report={'status':'failed','clock_start':time.monotonic(),'clipboard_accessed':False,'WSL_accessed':False,
            'actual_login_trigger':False,'manifest_sha256':args.manifest_sha}
    desktops=[]
    helpers=[]
    before=None
    def record(name,value):
        (output/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    class Desktop:
        def __init__(self,label,background=False):
            self.label=label
            self.verification=root/label
            self.verification.mkdir(exist_ok=False)
            self.sequence=0
            self.quit_requested=False
            self.streams=[(output/(label+'.'+suffix)).open('wb') for suffix in ['stdout','stderr']]
            command=[str(root/'app files/miopunch-desktop.exe'),'--data-dir',str(root/'node state'),
                     '--verification-dir',str(self.verification)]
            if background:command.append('--background')
            self.process=subprocess.Popen(command,stdin=subprocess.DEVNULL,stdout=self.streams[0],stderr=self.streams[1])
            self.handle=int(self.process._handle)
            desktops.append(self)
            record(label+'-owner',{'pid':self.process.pid,'original_handle':self.handle,'argv':command})
        def events(self):
            path=self.verification/'events.jsonl'
            return [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines()] if path.exists() else []
        def event(self,name,count=1):
            def observed():
                assert self.process.poll() is None,'original desktop exited before '+name
                events=[e for e in self.events() if e['event']==name]
                return events if len(events)>=count else None
            return wait(observed,'native event '+name)
        def call(self,action,**fields):
            self.sequence+=1
            temporary=self.verification/'request.next'
            temporary.write_text(json.dumps({'id':self.sequence,'action':action,**fields}),encoding='utf-8')
            temporary.replace(self.verification/'request.json')
            result_path=self.verification/('result-%d.json'%self.sequence)
            def complete():
                if result_path.exists():return json.loads(result_path.read_text(encoding='utf-8'))
                assert self.process.poll() is None,'original desktop exited before IPC result'
            # The pinned SDK default caller budget is 60s; the controller must
            # allow its bounded result to arrive before classifying a timeout.
            result=wait(complete,'WebView IPC '+action,seconds=75)
            record(self.label+'-ipc-%d-%s'%(self.sequence,action),result)
            assert result['ok'],result
            if action=='quit':
                assert result['stopped']
                self.quit_requested=True
            return result.get('result',result)
        def request(self,operation,**fields):
            result=self.call('request',request={'id':self.sequence+1,'op':operation,**fields})
            assert result['ok'],operation
            return result['result']
        def secondary(self):
            return helper([str(root/'app files/miopunch-desktop.exe'),'--data-dir',str(root/'node state')])
        def close(self):
            if hasattr(self,'cleanup'):return self.cleanup
            if self.process.poll() is None and not self.quit_requested:
                self.secondary()
                self.call('autostart',enabled=False)
                self.call('quit')
            self.process.wait(timeout=20)
            assert int(self.process._handle)==self.handle
            for stream in self.streams:stream.close()
            if (self.verification/'events.jsonl').exists():
                (output/(self.label+'-native-events.jsonl')).write_bytes((self.verification/'events.jsonl').read_bytes())
            self.cleanup={'desktop':self.label,'pid':self.process.pid,'exit':self.process.returncode,'normal_quit':self.quit_requested}
            assert self.process.returncode==0 and self.quit_requested,self.cleanup
            return self.cleanup
    def helper(command):
        label='helper-%d'%len(helpers)
        streams=[(output/(label+'.'+suffix)).open('wb') for suffix in ['stdout','stderr']]
        process=subprocess.Popen(command,stdin=subprocess.DEVNULL,stdout=streams[0],stderr=streams[1])
        entry={'process':process,'handle':int(process._handle),'streams':streams,'label':label}
        helpers.append(entry)
        record(label+'-owner',{'pid':process.pid,'original_handle':entry['handle'],'command':command})
        process.wait(timeout=20)
        assert process.returncode==0,'secondary/startup process failed'
        return process.pid
    try:
        assert not ctypes.WinDLL('shell32').IsUserAnAdmin()
        session=ctypes.c_uint32()
        assert ctypes.WinDLL('kernel32').ProcessIdToSessionId(os.getpid(),ctypes.byref(session)) and session.value>0
        report['interactive_session']=session.value
        before=startup_values()
        assert all(v is None for v in before.values())
        prior=root/'prior-startup.json'
        assert sha(prior)==configuration['prior_startup_sha256']
        prior=json.loads(prior.read_text(encoding='utf-8'))
        assert prior['status']=='pass' and prior['main_exit']==0 and not prior['cleanup_errors'] and prior['startup_before']==prior['startup_after']
        report['reused_owned_profile_provenance']=configuration['prior_startup_sha256']
        binary=root/'app files/miopunch-desktop.exe'
        assert sha(binary)==configuration['binary_sha256'] and sha(binary.with_name('WebView2Loader.dll'))==configuration['loader_sha256']
        assert sha(root/'invitation.json')==configuration['invitation_sha256']
        initial=Desktop('initial')
        initial.event('webview_ready')
        assert any(w['visible'] and w['title']=='miopunch host' for w in windows(initial.process.pid))
        assert initial.call('state')['autostart'] is True
        invitation=(root/'invitation.json').read_text(encoding='utf-8')
        ready=initial.call('start',mode={'action':'join','invitation':invitation},resources=configuration['resources'])
        identity=ready['status']['node']
        assert identity!=configuration['provider_address']['id']
        assert initial.request('request',path=str(root/'invitation.json'))['stage']=='Complete'
        assert initial.request('ping',address=configuration['provider_address'])['reply']=='PONG'
        initial.request('service_sync',address=configuration['provider_address'])
        source=root/'windows-to-linux.bin'
        source.write_bytes(bytes(i%239 for i in range(8193)))
        task=initial.request('file_send',address=configuration['provider_address'],path=str(source))['task']
        wait(lambda:any(t['id']==task and t['state']=='complete' for t in initial.request('files_status')['tasks']), 'actual Windows-to-Linux SDK save')
        received=root/'received'
        initial.request('files_enable',directory=str(received))
        state=initial.call('state')
        initial.call('close')
        initial.event('window_destroyed')
        wait(lambda:not any(w['visible'] and w['title']=='miopunch host' for w in windows(initial.process.pid)),'actual own HWND absent')
        assert initial.process.poll() is None
        record('background-ready',{'identity':identity,'address':ready['address'],'main_pid':initial.process.pid,'window_destroyed':True,
                                  'windows_to_linux_task':task})
        def saved_file():
            return [p for p in received.iterdir() if p.is_file() and p.name.endswith('-linux-to-windows.bin')]
        files=wait(saved_file,'actual Linux-to-Windows SDK save while window absent',seconds=60)
        assert len(files)==1 and files[0].stat().st_size==65539 and sha(files[0])==configuration['linux_file_sha256']
        record('background-file',{'file':files[0].name,'bytes':65539,'sha256':sha(files[0]),'window_absent':True,'main_pid':initial.process.pid})
        initial.secondary()
        initial.event('webview_ready',2)
        again=initial.call('state')
        assert again['pid']==state['pid'] and again['node']['result']['node']==identity and again['windows_created']==2
        assert any(w['visible'] and w['title']=='miopunch host' for w in windows(initial.process.pid))
        assert initial.call('autostart',enabled=False) is False and read_value(winreg.HKEY_CURRENT_USER,RUN) is None
        initial.call('quit')
        initial.close()
        silent=Desktop('silent',background=True)
        assert silent.event('node_opened')[-1]['detail']['ok']
        assert not any(e['event']=='window_created' for e in silent.events())
        assert not any(w['visible'] and w['title']=='miopunch host' for w in windows(silent.process.pid))
        assert read_value(winreg.HKEY_CURRENT_USER,RUN) is None
        silent.secondary()
        silent.event('webview_ready')
        reopened=silent.call('state')
        assert reopened['node']['result']['node']==identity and reopened['node']['result']['stage']=='Complete' and reopened['autostart'] is False
        assert silent.request('ping',address=configuration['provider_address'])['reply']=='PONG'
        assert silent.call('autostart',enabled=True) is True
        command=read_value(winreg.HKEY_CURRENT_USER,RUN)['data']
        assert command.startswith('"') and '--background' in command
        helper(command)
        assert silent.process.poll() is None
        assert silent.call('state')['node']['result']['node']==identity
        assert silent.call('autostart',enabled=False) is False
        silent.call('quit')
        silent.close()
        report.update(status='pass',identity=identity,same_process_window_reopen=True,same_identity_complete_after_restart=True,
            actual_registered_command_parsed=True,silent_launch=True,autostart_off_persisted=True,
            windows_to_linux_bytes=8193,linux_to_windows_bytes=65539)
    except BaseException as failure:
        report.update(failure_type=type(failure).__name__,failure=str(failure),traceback=traceback.format_exc())
    finally:
        cleanup=[]
        errors=[]
        for desktop in reversed(desktops):
            try:cleanup.append(desktop.close())
            except BaseException as failure:
                errors.append(str(failure))
                if desktop.process.poll() is None:
                    assert int(desktop.process._handle)==desktop.handle
                    desktop.process.terminate();desktop.process.wait(timeout=10)
                    errors.append('forced original desktop handle; not a clean exit')
        for entry in helpers:
            process=entry['process']
            if process.poll() is None:
                assert int(process._handle)==entry['handle']
                process.terminate();process.wait(timeout=10)
                errors.append('forced original helper handle; not a clean exit')
            for stream in entry['streams']:stream.close()
        if before is not None:
            approved=read_value(winreg.HKEY_CURRENT_USER,APPROVED)
            if before['HKCU:'+APPROVED] is None and approved is not None:
                if approved=={'data':(b'\x02'+b'\0'*11).hex(),'kind':winreg.REG_BINARY}:
                    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,APPROVED,0,winreg.KEY_SET_VALUE) as key:winreg.DeleteValue(key,'miopunch')
                else:errors.append('changed startup approval retained')
            report['startup_before']=before
            report['startup_after']=startup_values()
            if report['startup_after']!=before:errors.append('startup state not restored')
        report.update(cleanup=cleanup,cleanup_errors=errors,clock_end=time.monotonic())
        if errors:report['status']='failed'
        record('result',report)
    return 0 if report['status']=='pass' else 1


if __name__=='__main__':raise SystemExit(main())
