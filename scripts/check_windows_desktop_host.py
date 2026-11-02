#!/usr/bin/env python3
"""Real Windows interactive Tauri <-> Linux SDK lifecycle and file acceptance."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import uuid

from check_web_service import WebRunner, OwnedNode
from check_forwarding import isolated_environment
from web_process_owner import register_process
from windows_interactive import OPTIONS, quote


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class InteractiveOwner:
    def __init__(self,runner,root,manifest_sha,controller_sha):
        self.runner=runner
        self.root=root
        self.streams=[(runner.output/('interactive.'+suffix)).open('w') for suffix in ['stdout','stderr']]
        source=Path(__file__).resolve().parent
        code='import sys,json;from pathlib import Path;sys.path.insert(0,'+repr(str(source))+');from windows_interactive import run_task;p=Path('+repr(str(runner.output))+');print(json.dumps(run_task(p,'+repr(root)+','+repr(runner.runtime+'/python/pythonw.exe')+','+repr(['-B',root+'/controller.py','--root',root,'--manifest-sha',manifest_sha])+','+repr(controller_sha)+')))'
        self.process=subprocess.Popen([sys.executable,'-B','-c',code],env=isolated_environment(),stdout=self.streams[0],stderr=self.streams[1])
        self.owner=None
        runner.nodes.append(self)
        self.owner=register_process(self.process,runner.output/'interactive-owner.jsonl')
    def read(self,name):
        text=self.runner.remote("if(Test-Path -LiteralPath "+quote(self.root+'/evidence/'+name+'.json')+"){[IO.File]::ReadAllText("+quote(self.root+'/evidence/'+name+'.json')+")}",name+'-read')
        return json.loads(text) if text.strip() else None
    def wait(self,name,seconds=90):
        deadline=time.monotonic()+seconds
        while time.monotonic()<deadline:
            value=self.read(name)
            if value is not None:return value
            result=self.read('result')
            if result is not None:
                self.runner.record('windows','early_result',result)
                raise AssertionError('Windows controller ended before '+name)
            assert self.process.poll() is None,'original interactive controller exited early'
            time.sleep(.1)
        raise TimeoutError('Windows phase '+name)
    def close(self):
        if hasattr(self,'cleanup'):return self.cleanup
        self.process.wait(timeout=100)
        for stream in self.streams:stream.close()
        if self.owner:self.owner.close()
        self.cleanup={'owner':'windows-interactive-task','exit':self.process.returncode}
        assert self.process.returncode==0,self.cleanup
        return self.cleanup
    def terminal(self):
        return self.process.poll() is not None and (self.owner is None or self.owner.pidfd is None)


class WindowsDesktopRunner(WebRunner):
    def remote(self,body,label):
        self.remote_sequence=getattr(self,'remote_sequence',0)+1
        body="$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue';"+body
        encoded=base64.b64encode(body.encode('utf-16le')).decode()
        assert len(encoded)<6000,'remote control command exceeds bounded transport'
        result=subprocess.run(['rtk','proxy','ssh',*OPTIONS,'laptop','powershell.exe','-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',encoded],capture_output=True,text=True)
        prefix=self.output/('%03d-%s'%(self.remote_sequence,label))
        prefix.with_suffix('.stdout').write_text(result.stdout)
        prefix.with_suffix('.stderr').write_text(result.stderr)
        assert result.returncode==0,(label,result.stderr)
        return result.stdout
    def transfer(self,source,destination,label):
        result=subprocess.run(['rtk','proxy','scp',*OPTIONS,str(source),'laptop:'+destination],capture_output=True,text=True)
        (self.output/(label+'.scp.stderr')).write_text(result.stderr)
        assert result.returncode==0,result.stderr
        self.remote("if((Get-FileHash -LiteralPath "+quote(destination)+" -Algorithm SHA256).Hash.ToLowerInvariant() -ne "+quote(sha(source))+"){throw 'deployed file changed'}",label+'-hash')
    def scenario(self):
        root=Path(__file__).resolve().parents[1]
        deployment=json.loads(self.args.windows_deployment.read_text())
        prior=json.loads(self.args.prior_startup.read_text())
        assert prior['status']=='pass' and not prior['cleanup_errors'] and prior['main_exit']==0 and prior['startup_before']==prior['startup_after']
        self.runtime=self.args.windows_runtime
        remote='C:/Users/stati/AppData/Local/Temp/m121-'+uuid.uuid4().hex[:8]
        resources={'defaults':{'revision':2,'resources':[]},'replacement':[],'disabled':[],
            'disabled_uses':['Observation','Lookup','Signaling','DataRelay'],'lan_discovery':'Disabled','port_mapping':'Disabled'}
        self.native_configuration=self.private/'resources.json'
        self.native_configuration.write_text(json.dumps(resources))
        provider=OwnedNode(self,'provider')
        received=self.private/'received'
        provider.call('files_enable',directory=str(received))
        invitation=self.private/'invitation.json'
        provider.call('issue_precode',path=str(invitation),seconds=600,max_uses=1)
        for filename in ['desktop_windows_lifecycle.py','check_desktop_windows_startup.py','windows_interactive.py']:
            self.freeze_source(root/'scripts'/filename)
        retained=self.args.retained_root
        body="$base=Split-Path "+quote(self.runtime)+";$total=(@(Get-ChildItem -LiteralPath $base -Recurse -File -Force)|Measure-Object Length -Sum).Sum;"
        for value in retained:
            body+="$total+=(@(Get-ChildItem -LiteralPath "+quote(value)+" -Recurse -File -Force)|Measure-Object Length -Sum).Sum;"
        body+="if($total+402653184 -gt 1879048192){throw 'combined retained deployment budget'};if(Test-Path -LiteralPath "+quote(remote)+"){throw 'fresh root collision'};New-Item -ItemType Directory -Path "+quote(remote+'/app files')+"|Out-Null;@{retainedBytes=$total;newReservation=402653184;combinedBudget=1879048192;root="+quote(remote)+"}|ConvertTo-Json -Compress"
        preflight=json.loads(next(l for l in self.remote(body,'preflight').splitlines() if l.startswith('{')))
        manifest={'provider_address':provider.address,'resources':resources,'binary_sha256':deployment['deployment_sha256'],
            'loader_sha256':deployment['loader_sha256'],'helper_sha256':sha(root/'scripts/check_desktop_windows_startup.py'),
            'prior_startup_sha256':sha(self.args.prior_startup),'invitation_sha256':sha(invitation),
            'linux_file_sha256':hashlib.sha256(bytes(i%251 for i in range(65539))).hexdigest()}
        manifest_path=self.private/'manifest.json'
        manifest_path.write_text(json.dumps(manifest))
        info={'root':remote,'remote_root_owned':True,'preflight':preflight,'deployment':deployment,'manifest_sha256':sha(manifest_path),
              'business_transport':'SDK over actual Windows/Linux network; SSH control/invitation/evidence only'}
        (self.output/'remote-deployment.json').write_text(json.dumps(info,indent=2)+'\n')
        for filename,digest in [('miopunch-desktop.exe',manifest['binary_sha256']),('WebView2Loader.dll',manifest['loader_sha256'])]:
            original=self.args.windows_artifact_root+'/app files/'+filename
            self.remote("if((Get-FileHash -LiteralPath "+quote(original)+" -Algorithm SHA256).Hash.ToLowerInvariant() -ne "+quote(digest)+"){throw 'reused deployment hash'};New-Item -ItemType HardLink -Path "+quote(remote+'/app files/'+filename)+" -Value "+quote(original)+"|Out-Null",'reuse-'+filename)
        for source,name in [(root/'scripts/desktop_windows_lifecycle.py','controller.py'),(root/'scripts/check_desktop_windows_startup.py','startup_helpers.py'),
                            (self.args.prior_startup,'prior-startup.json'),(invitation,'invitation.json'),(manifest_path,'manifest.json')]:
            self.transfer(source,remote+'/'+name,name)
        task=InteractiveOwner(self,remote,sha(manifest_path),sha(root/'scripts/desktop_windows_lifecycle.py'))
        primary=None
        try:
            ready=task.wait('background-ready')
            assert ready['identity']!=provider.identity and ready['window_destroyed']
            self.until(lambda:any(t['id']==ready['windows_to_linux_task'] and t['state']=='complete' for t in provider.call('files_status')['tasks']),
                       'actual Windows-to-Linux receiver not complete',seconds=20)
            files=list(received.iterdir())
            assert len(files)==1 and files[0].read_bytes()==bytes(i%239 for i in range(8193))
            outgoing=self.private/'linux-to-windows.bin'
            outgoing.write_bytes(bytes(i%251 for i in range(65539)))
            sent=provider.call('file_send',address=ready['address'],path=str(outgoing))['task']
            self.until(lambda:any(t['id']==sent and t['state']=='complete' for t in provider.call('files_status')['tasks']),
                       'actual Linux-to-background-Windows sender incomplete',seconds=25)
            observed=task.wait('background-file')
            assert observed['sha256']==manifest['linux_file_sha256'] and observed['bytes']==65539
            result=task.wait('result')
            assert result['status']=='pass' and not result['cleanup_errors'] and result['startup_before']==result['startup_after'],result
            task.close()
            self.record('observer','real_windows_desktop_chain',{'windows_to_linux_saved_bytes':8193,'windows_to_linux_sha256':sha(files[0]),
                'linux_to_windows_saved_bytes':65539,'linux_to_windows_sha256':observed['sha256'],'same_process_reopen':result['same_process_window_reopen'],
                'same_identity_restart':result['same_identity_complete_after_restart'],'silent_launch':result['silent_launch'],
                'actual_registered_command':result['actual_registered_command_parsed'],'actual_login_trigger':False})
        except BaseException as failure:
            primary=failure
            raise
        finally:
            try:
                task.close()
                result=subprocess.run(['rtk','proxy','scp','-r',*OPTIONS,'laptop:'+remote+'/evidence',str(self.output/'windows-evidence')],capture_output=True,text=True)
                assert result.returncode==0,result.stderr
            except BaseException as failure:
                if primary:primary.add_note('Independent Windows task settlement: '+str(failure))
                else:raise


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['binary','source-receipt','output','windows-deployment','prior-startup']:
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--sha256',required=True)
    parser.add_argument('--windows-runtime',required=True)
    parser.add_argument('--windows-artifact-root',required=True)
    parser.add_argument('--retained-root',action='append',required=True)
    parser.set_defaults(case='windows_desktop')
    WindowsDesktopRunner(parser.parse_args()).run()
