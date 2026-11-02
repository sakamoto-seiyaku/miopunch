"""One owned, least-privilege, on-demand Windows interactive task over SSH."""
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import uuid

OPTIONS = ['-o','BatchMode=yes','-o','StrictHostKeyChecking=yes','-o','UpdateHostKeys=no',
    '-o','ControlMaster=no','-o','ControlPath=none','-o','ClearAllForwardings=yes',
    '-o','ForwardAgent=no','-o','ForwardX11=no','-o','PermitLocalCommand=no','-o','ConnectTimeout=10']


def quote(value):
    return "'"+str(value).replace("'","''")+"'"


def run_task(output, root, pythonw, arguments, controller_sha256):
    output=Path(output)
    name='miopunch-m121-'+uuid.uuid4().hex
    body=r'''$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$scheduler=New-Object -ComObject Schedule.Service
$scheduler.Connect()
$folder=$scheduler.GetFolder('\')
$definition=$scheduler.NewTask(0)
$definition.Principal.UserId=[Security.Principal.WindowsIdentity]::GetCurrent().Name
$definition.Principal.LogonType=3
$definition.Principal.RunLevel=0
$definition.Settings.Enabled=$true
$definition.Settings.AllowDemandStart=$true
$definition.Settings.DisallowStartIfOnBatteries=$false
$definition.Settings.StopIfGoingOnBatteries=$false
$definition.Settings.ExecutionTimeLimit='PT0S'
$action=$definition.Actions.Create(0)
$action.Path=PYTHON
$action.Arguments=ARGUMENTS
$task=$folder.RegisterTaskDefinition(NAME,$definition,2,$null,$null,3,$null)
$instance=$task.Run($null)
$guid=$instance.InstanceGuid
@{event='started';instance=$guid;name=NAME;interactiveToken=$true;runLevel=0}|ConvertTo-Json -Compress
$clock=[Diagnostics.Stopwatch]::StartNew()
while($clock.Elapsed.TotalSeconds -lt 180){
 try{$instance.Refresh()}catch{
  $cause=$_.Exception
  while($null -ne $cause.InnerException){$cause=$cause.InnerException}
  if($cause.HResult -ne -2147216629){throw}
  break
 }
 if($instance.State -notin @(2,4)){break}
 Start-Sleep -Milliseconds 200
}
if($task.GetInstances(0).Count -ne 0){throw 'original_task_not_terminal_retained'}
$result=$task.LastTaskResult
$folder.DeleteTask(NAME,0)
@{event='terminal';instance=$guid;name=NAME;exit=$result;registrationRemoved=$true}|ConvertTo-Json -Compress
'''.replace('PYTHON',quote(pythonw)).replace('ARGUMENTS',quote(subprocess.list2cmdline(arguments))).replace('NAME',quote(name))
    script=output/'task-controller.ps1'
    script.write_text(body)
    digest=hashlib.sha256(script.read_bytes()).hexdigest()
    transferred=subprocess.run(['rtk','proxy','scp',*OPTIONS,str(script),'laptop:'+root+'/task-controller.ps1'],capture_output=True,text=True)
    (output/'task-scp.stdout').write_text(transferred.stdout)
    (output/'task-scp.stderr').write_text(transferred.stderr)
    assert transferred.returncode==0,transferred.stderr
    command="$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; if((Get-FileHash -LiteralPath "+quote(root+'/controller.py')+" -Algorithm SHA256).Hash.ToLowerInvariant() -ne "+quote(controller_sha256)+"){throw 'controller changed'}; if((Get-FileHash -LiteralPath "+quote(root+'/task-controller.ps1')+" -Algorithm SHA256).Hash.ToLowerInvariant() -ne "+quote(digest)+"){throw 'task controller changed'}; & "+quote(root+'/task-controller.ps1')
    encoded=base64.b64encode(command.encode('utf-16le')).decode()
    assert len(encoded)<6000,'bounded SSH command required'
    (output/'task-launch.json').write_text(json.dumps({'task':name,'root':root,'arguments':arguments,
        'task_controller_sha256':digest,'controller_sha256':controller_sha256,'invocations':1},indent=2)+'\n')
    with (output/'task.stdout').open('w') as stdout,(output/'task.stderr').open('w') as stderr:
        result=subprocess.run(['rtk','proxy','ssh',*OPTIONS,'laptop','powershell.exe','-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',encoded],stdout=stdout,stderr=stderr)
    events=[json.loads(line) for line in (output/'task.stdout').read_text().splitlines() if line.startswith('{')]
    terminal=next((e for e in events if e['event']=='terminal'),None)
    started=next((e for e in events if e['event']=='started'),None)
    record={'transport_exit':result.returncode,'task':name,'started':started,'terminal':terminal}
    (output/'task-result.json').write_text(json.dumps(record,indent=2)+'\n')
    assert result.returncode==0 and started and terminal and terminal['instance']==started['instance'] and terminal['registrationRemoved'],record
    return record
