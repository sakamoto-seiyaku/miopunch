param(
    [Parameter(Mandatory=$true)][string]$Manifest,
    [Parameter(Mandatory=$true)][string]$RunRoot,
    [ValidateSet('create','join','join-guest')][string]$Mode='create',
    [string]$Invitation,
    [switch]$Mailbox,
    [switch]$TerminalConsumer
)
# Concrete private technical host, never invoked by the local parser check.
$ErrorActionPreference='Stop'
[Console]::InputEncoding=New-Object Text.UTF8Encoding($false)
[Console]::OutputEncoding=New-Object Text.UTF8Encoding($false)
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'actual Windows required' }
$plan=Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json
$deployment=[IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Manifest))
$exe=Join-Path $deployment 'miopunch-client.exe'
$helper=Join-Path $deployment 'windows_forwarding_sockets.cs'
foreach ($entry in @(@($exe,$plan.files.exe),@($PSCommandPath,$plan.files.host),@($helper,$plan.files.helper))) {
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $entry[0]).Hash.ToLowerInvariant() -ne $entry[1]) { throw 'frozen input hash mismatch' }
}
$RunRoot=[IO.Path]::GetFullPath($RunRoot)
$temp=[IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')+'\'
if (-not $RunRoot.StartsWith($temp,[StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $RunRoot)) { throw 'fresh current-user TEMP runtime required' }
if ($Mode -ne 'create') {
    $Invitation=[IO.Path]::GetFullPath($Invitation)
    if (-not $Invitation.StartsWith($deployment.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'only fresh deployment invitation accepted' }
}
$final=Join-Path $deployment 'runtime.result.json'
if (Test-Path -LiteralPath $final) { throw 'final receipt already exists' }
Add-Type -Path $helper
$runId=[Guid]::NewGuid().ToString('N')
$ownerPath=Join-Path $RunRoot 'owner.json'
$resources=Join-Path $RunRoot 'resources.json'
$nodeDirectory=Join-Path $RunRoot 'node'
$sdk=$null; $read=$null; $sockets=$null; $secondary=$null; $spawn=$null
$listeners=@{}; $failures=New-Object 'Collections.Generic.List[string]'
$signalLog=Join-Path $deployment 'runtime-owner.jsonl'
$script:mailSequence=0
function ReadControl {
    if (-not $Mailbox) { return [Console]::ReadLine() }
    $script:mailSequence++
    if ($script:mailSequence -gt 4096) { throw 'technical controller request bound' }
    $path=Join-Path $RunRoot ('requests/{0:d6}.json' -f $script:mailSequence)
    $clock=[Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath $path)) {
        if ($clock.Elapsed.TotalSeconds -ge 120) { throw 'owned control idle deadline' }
        Start-Sleep -Milliseconds 30
    }
    if ((Get-Item -LiteralPath $path).Length -gt 65536) { throw 'technical controller input bound' }
    return [IO.File]::ReadAllText($path,[Text.Encoding]::UTF8)
}
function Event([object]$value) {
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes(($value | ConvertTo-Json -Depth 12 -Compress)+"`n")
    $stream=[IO.File]::Open($signalLog,[IO.FileMode]::Append,[IO.FileAccess]::Write,[IO.FileShare]::Read)
    try { $stream.Write($bytes,0,$bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
}
function Quote([string]$value) {
    if ($value.Contains('"') -or $value.Contains("`r") -or $value.Contains("`n") -or $value.EndsWith('\')) { throw 'unsupported owned argument' }
    return '"'+$value+'"'
}
function StartSdk([string]$action) {
    $values=@($action,$nodeDirectory)
    if ($action -in @('join','join-guest')) { $values+=@($Invitation,$resources) }
    if ($action -eq 'create') { $values+=@($resources) }
    $info=New-Object Diagnostics.ProcessStartInfo
    $info.FileName=$exe; $info.Arguments=($values | ForEach-Object { Quote $_ }) -join ' '
    $info.UseShellExecute=$false; $info.RedirectStandardInput=$true; $info.RedirectStandardOutput=$true; $info.CreateNoWindow=$true
    foreach ($key in @('DISPLAY','WAYLAND_DISPLAY','WAYLAND_SOCKET','XAUTHORITY','MIR_SOCKET','DBUS_SESSION_BUS_ADDRESS','DBUS_SYSTEM_BUS_ADDRESS','SESSION_MANAGER','HTTP_PROXY','HTTPS_PROXY','ALL_PROXY')) { $info.EnvironmentVariables.Remove($key) }
    $script:sdk=New-Object Diagnostics.Process
    $script:sdk.StartInfo=$info
    if (-not $script:sdk.Start()) { throw 'SDK start failed' }
    $script:spawn=@{pid=$script:sdk.Id;start_ticks=$script:sdk.StartTime.ToUniversalTime().Ticks;exe=$exe;exe_sha256=$plan.files.exe}
    Event @{phase='registered';owner=$script:spawn;run_id=$runId}
    return ReadSdk $null
}
function ReadSdk($expected) {
    $clock=[Diagnostics.Stopwatch]::StartNew()
    while ($clock.Elapsed.TotalSeconds -lt 80) {
        $script:read=$script:sdk.StandardOutput.ReadLineAsync()
        if (-not $script:read.Wait([Math]::Max(1,80000-[int]$clock.ElapsedMilliseconds))) { throw 'SDK response deadline' }
        $line=$script:read.Result
        if ($null -eq $line) { throw 'SDK exited before response' }
        if ($line.Length -gt 65536) { throw 'SDK response bound' }
        $value=$line | ConvertFrom-Json
        if ($null -eq $expected) { if ($value.event -eq 'ready') { return $value } }
        elseif ($value.id -eq $expected) { return $value }
    }
    throw 'SDK response deadline'
}
function StopSdk {
    if ($null -eq $script:sdk) { return }
    if (-not $script:sdk.HasExited) {
        try { $script:sdk.StandardInput.WriteLine('{"id":999999999,"op":"stop"}'); $script:sdk.StandardInput.Flush(); $script:sdk.StandardInput.Close() } catch { $failures.Add('SDKStopWrite') }
        if (-not $script:sdk.WaitForExit(25000)) { throw 'owned SDK still running; retain private state' }
    }
    if ($script:sdk.ExitCode -ne 0) { $failures.Add('SDKNonzeroExit') }
    if ($null -ne $script:read -and -not $script:read.IsCompleted -and -not $script:read.Wait(2000)) { $failures.Add('SDKReaderNotJoined') }
    Event @{phase='terminal';owner=$script:spawn;exit=$script:sdk.ExitCode;run_id=$runId}
}
$allowed=@('status','address','export_recovery_card','acknowledge_recovery_saved','invite','issue_precode','request','requests','approve','service_status','service_rule','service_sync','guest_issue','guest_redeem','guest_status','guest_end','guest_revoke','forward_enable','forward_disable','forward_start','forward_stop','forward_cancel','forward_status')
if ($TerminalConsumer) {
    $allowed += @('terminal_create','terminal_list','terminal_end','terminal_attach','terminal_send','terminal_read','terminal_detach','terminal_cancel')
}

try {
    New-Item -ItemType Directory -Path $RunRoot | Out-Null
    $acl=New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true,$false)
    $who=[Security.Principal.WindowsIdentity]::GetCurrent().Name
    $rule=New-Object Security.AccessControl.FileSystemAccessRule($who,'FullControl','ContainerInherit,ObjectInherit','None','Allow')
    $acl.AddAccessRule($rule)
    [IO.Directory]::SetAccessControl($RunRoot,$acl)
    [IO.File]::WriteAllText($ownerPath,(@{run_id=$runId;exe_sha256=$plan.files.exe} | ConvertTo-Json -Compress),(New-Object Text.UTF8Encoding($false)))
    if ($Mailbox) { New-Item -ItemType Directory -Path (Join-Path $RunRoot 'requests') | Out-Null }
    # No public facility, observation, discovery or fallback is enabled by this host.
    [IO.File]::WriteAllText($resources,(@{defaults=@{revision=2;resources=@()};replacement=@();disabled=@();disabled_uses=@('Observation','Lookup','Signaling','DataRelay');lan_discovery='Disabled';port_mapping='Disabled'} | ConvertTo-Json -Depth 8 -Compress),(New-Object Text.UTF8Encoding($false)))
    if ($Mode -ne 'create') {
        $privateInvitation=Join-Path $RunRoot 'invitation.json'
        [IO.File]::Copy($Invitation,$privateInvitation,$false)
        $Invitation=$privateInvitation
    }
    $sockets=New-Object MioForwardSockets
    $ready=StartSdk $Mode
    @{event='ready';run_id=$runId;sdk=$ready;tcp_target=$sockets.TcpAddress;udp_target=$sockets.UdpAddress;external_resources=$false} | ConvertTo-Json -Depth 32 -Compress | Write-Output
    $count=0
    Event @{phase='control_wait';run_id=$runId;next_id=1}
    while ($null -ne ($line=ReadControl)) {
        $count++; if ($count -gt 4096 -or $line.Length -gt 65536) { throw 'technical controller input bound' }
        $packet=$line | ConvertFrom-Json
        Event @{phase='control_received';run_id=$runId;id=$packet.id;kind=$packet.kind}
        try {
            $targetSockets=$sockets
            if ($packet.PSObject.Properties.Name -contains 'target_slot') {
                if ([int]$packet.target_slot -eq 1 -and $null -ne $secondary) { $targetSockets=$secondary }
                elseif ([int]$packet.target_slot -ne 0) { throw 'unknown owned target slot' }
            }
            switch ($packet.kind) {
                'sdk' {
                    $command=$packet.command
                    if ($command.op -notin $allowed) { throw 'operation outside selected technical host scope' }
                    if ($command.PSObject.Properties.Name -contains 'path') {
                        $path=[IO.Path]::GetFullPath($command.path)
                        if (-not $path.StartsWith($RunRoot+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'SDK path outside fresh runtime' }
                    }
                    $command | Add-Member -MemberType NoteProperty -Name id -Value $packet.id -Force
                    $sdk.StandardInput.WriteLine(($command | ConvertTo-Json -Depth 32 -Compress)); $sdk.StandardInput.Flush()
                    $result=ReadSdk $packet.id
                    if ($command.op -eq 'forward_start' -and $result.ok) { $listeners[[string]$result.result.id]=[string]$result.result.listen }
                    if ($command.op -eq 'forward_stop' -and $result.ok) { $listeners.Remove([string]$command.task) }
                }
                'target_create' {
                    if ($null -ne $secondary) { throw 'owned target slot bound' }
                    $secondary=New-Object MioForwardSockets
                    $result=@{slot=1;tcp_target=$secondary.TcpAddress;udp_target=$secondary.UdpAddress}
                }
                'expect' { $targetSockets.Expect($packet.protocol,[int]$packet.length,[int]$packet.seed); $result=@{armed=$true} }
                'expect_eof_reply' { $targetSockets.ExpectEofReply([int]$packet.length,[int]$packet.seed,[int]$packet.response_length,[int]$packet.response_seed); $result=@{armed=$true;reply_only_after_eof=$true} }
                'expect_udp_reply' { $targetSockets.ExpectUdpReply([int]$packet.length,[int]$packet.seed,[int]$packet.response_length,[int]$packet.response_seed,[bool]$packet.echo_after_reply); $result=@{armed=$true} }
                'target_status' { $result=$targetSockets.Snapshot() }
                'target_probe' {
                    $address=if ($packet.protocol -eq 'tcp') { $targetSockets.TcpAddress } elseif ($packet.protocol -eq 'udp') { $targetSockets.UdpAddress } else { throw 'unknown protocol' }
                    $targetSockets.Expect($packet.protocol,[int]$packet.length,[int]$packet.seed)
                    Event @{phase='target_expected';run_id=$runId;id=$packet.id}
                    $client=$targetSockets.OpenClient($packet.protocol,$address)
                    Event @{phase='target_client_open';run_id=$runId;id=$packet.id}
                    try { $result=$targetSockets.Exchange($client,[int]$packet.length,[int]$packet.seed) } finally { $targetSockets.CloseClient($client) }
                }
                'client_open' {
                    if ([string]$packet.address -notin $listeners.Values) { throw 'only actual SDK listener addresses accepted' }
                    $result=@{client=$sockets.OpenClient($packet.protocol,$packet.address)}
                }
                'client_exchange' { $result=$sockets.Exchange([int]$packet.client,[int]$packet.length,[int]$packet.seed) }
                'client_send_datagram' { $result=$sockets.SendDatagram([int]$packet.client,[int]$packet.length,[int]$packet.seed) }
                'client_halfclose_exchange' { $result=$sockets.ExchangeHalfClosed([int]$packet.client,[int]$packet.length,[int]$packet.seed,[int]$packet.response_length,[int]$packet.response_seed) }
                'target_probe_eof_reply' {
                    $sockets.ExpectEofReply([int]$packet.length,[int]$packet.seed,[int]$packet.response_length,[int]$packet.response_seed)
                    $client=$sockets.OpenClient('tcp',$sockets.TcpAddress)
                    try { $result=$sockets.ExchangeHalfClosed($client,[int]$packet.length,[int]$packet.seed,[int]$packet.response_length,[int]$packet.response_seed) } finally { $sockets.CloseClient($client) }
                }
                'client_close' { $sockets.CloseClient([int]$packet.client); $result=@{closed=$true} }
                'restart' { StopSdk; $result=StartSdk 'open'; $listeners=@{} }
                'close' { break }
                default { throw 'unknown technical operation' }
            }
            if ($packet.kind -eq 'close') { break }
            @{id=$packet.id;ok=$true;result=$result} | ConvertTo-Json -Depth 32 -Compress | Write-Output
            Event @{phase='control_replied';run_id=$runId;id=$packet.id}
        } catch {
            $cause=$_.Exception.GetBaseException()
            $socketError=if($cause -is [Net.Sockets.SocketException]){[int]$cause.SocketErrorCode}else{$null}
            @{id=$packet.id;ok=$false;error='technical_operation_failed';causeType=$cause.GetType().FullName;socketError=$socketError} | ConvertTo-Json -Compress | Write-Output
        }
    }
} catch { $failures.Add('ControllerFailure') }
finally {
    try { StopSdk } catch { $failures.Add('SDKCleanup') }
    try { if ($null -ne $sockets) { $sockets.Dispose() } } catch { $failures.Add('SocketCleanup') }
    try { if ($null -ne $secondary) { $secondary.Dispose() } } catch { $failures.Add('SecondarySocketCleanup') }
    $socketSnapshot=$null
    try {
        if ($null -ne $sockets) {
            $socketSnapshot=$sockets.Snapshot()
            foreach ($failure in @($socketSnapshot.errors)) { $failures.Add('SocketHelper:'+ [string]$failure) }
        }
    } catch { $failures.Add('SocketSnapshot') }
    $secondarySnapshot=$null
    try {
        if ($null -ne $secondary) {
            $secondarySnapshot=$secondary.Snapshot()
            foreach ($failure in @($secondarySnapshot.errors)) { $failures.Add('SecondarySocketHelper:'+ [string]$failure) }
        }
    } catch { $failures.Add('SecondarySocketSnapshot') }
    $terminal=($null -eq $sdk -or $sdk.HasExited) -and ($null -eq $sockets -or ($null -ne $socketSnapshot -and $socketSnapshot.terminal)) -and ($null -eq $secondary -or ($null -ne $secondarySnapshot -and $secondarySnapshot.terminal))
    $report=@{status='host_closed';run_id=$runId;all_owned_terminal=$terminal;socket_snapshot=$socketSnapshot;secondary_socket_snapshot=$secondarySnapshot;failures=@($failures.ToArray());actual_windows=$true;clipboard_accessed=$false;external_resources=$false;private_removed=$false}
    try {
        if ($terminal -and $failures.Count -eq 0 -and (Get-Content -LiteralPath $ownerPath -Raw | ConvertFrom-Json).run_id -eq $runId) {
            Remove-Item -LiteralPath $RunRoot -Recurse
            $report.private_removed=$true
        }
    } catch { $failures.Add('PrivateCleanup'); $report.failures=@($failures.ToArray()) }
    try { [IO.File]::WriteAllText($final,($report | ConvertTo-Json -Depth 12 -Compress),(New-Object Text.UTF8Encoding($false))) } catch { $failures.Add('FinalReceipt') }
    $report.failures=@($failures.ToArray())
    $report | ConvertTo-Json -Depth 12 -Compress | Write-Output
}
if (-not $terminal -or $failures.Count -ne 0) { exit 1 }
