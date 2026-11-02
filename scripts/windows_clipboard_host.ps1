param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$BinarySha256,
    [Parameter(Mandatory=$true)][string]$ControllerSha256,
    [Parameter(Mandatory=$true)][string]$HelperSha256,
    [Parameter(Mandatory=$true)][string]$RunDirectory,
    [ValidateSet('create','join','join-guest')][string]$Action = 'create',
    [string]$Invitation,
    [string]$Resources
)
# Local preparation only until applicable authorization. Run in the user's
# current unlocked interactive Windows desktop, never as an SSH/Session0 host.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$utf8 = New-Object System.Text.UTF8Encoding($false)
$client = $null
$clientStarted = $false
$phase = 'preflight'
$clipboard = $null
$errors = $null
$script:lastSdkRead = $null
$restored = $false
$allTerminal = $false
$ownsDirectory = $false
$cleanupErrors = New-Object System.Collections.Generic.List[string]
$runId = [Guid]::NewGuid().ToString('N')
$report = @{status='failed'; runId=$runId; scope='interactive host lifecycle and requested assertions; external multi-node qualification required'; interactive=$false; clipboardRestored=$false; originalClipboardExported=$false}

function Write-JsonAtomic([string]$Path, $Value) {
    $temporary = $Path + '.pending'
    [IO.File]::WriteAllText($temporary, (ConvertTo-Json -Depth 64 -Compress -InputObject $Value), $utf8)
    Move-Item -LiteralPath $temporary -Destination $Path
}
function Marker($Request) {
    if ($Request.label -notmatch '^[A-Za-z0-9_-]{1,64}$') { throw 'invalid_owned_marker_label' }
    $unicode = [string][char]0x4e2d + [string][char]0x6587 + [char]::ConvertFromUtf32(0x1f642)
    $text = 'M117 owned Windows clipboard ' + $runId + ' ' + $Request.label + ' Unicode' + $unicode
    $size = [int]$Request.bytes
    $prefix = $utf8.GetByteCount($text)
    if ($size -lt $prefix -or $size -gt 1048577) { throw 'invalid_owned_marker_bound' }
    $text += 'x' * ($size - $prefix)
    return $text
}
function Read-Sdk {
    $budget = [Diagnostics.Stopwatch]::StartNew()
    while ($budget.Elapsed.TotalSeconds -lt 90) {
        if ($null -ne $script:lastSdkRead -and -not $script:lastSdkRead.IsCompleted) { throw 'owned_sdk_reader_pending' }
        $script:lastSdkRead = $client.StandardOutput.ReadLineAsync()
        $line = $script:lastSdkRead
        $remaining = [Math]::Max(1, [int](90000 - $budget.Elapsed.TotalMilliseconds))
        if (-not $line.Wait($remaining)) { throw 'owned_sdk_response_timeout' }
        if ($null -eq $line.Result) { throw 'owned_sdk_exited_before_response' }
        $value = ConvertFrom-Json -InputObject $line.Result
        if (-not ($value.PSObject.Properties.Name -contains 'event') -or $value.event -eq 'ready') { return $value }
    }
    throw 'owned_sdk_response_timeout'
}
function Call-Sdk($Command) {
    $allowed = @('status','export_recovery_card','acknowledge_recovery_saved','issue_precode','request',
        'clipboard_enable','clipboard_disable','clipboard_start','clipboard_stop','clipboard_status',
        'service_status','service_rule','service_sync','guest_issue','guest_redeem','guest_status','guest_end',
        'guest_revoke','ping','files_enable','files_status','file_send','file_approve','file_reject','stop')
    if ($Command.op -notin $allowed) { throw 'unsupported_owned_sdk_operation' }
    # All effect/save paths belong to this fresh private run, never user files.
    foreach ($field in @('path','directory')) {
        if ($Command.PSObject.Properties.Name -contains $field) {
            $full = [IO.Path]::GetFullPath($Command.$field)
            if (-not $full.StartsWith($RunDirectory + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
                throw 'sdk_path_outside_owned_runtime'
            }
        }
    }
    $client.StandardInput.WriteLine((ConvertTo-Json -Depth 64 -Compress -InputObject $Command))
    $client.StandardInput.Flush()
    $answer = Read-Sdk
    if ($answer.id -ne $Command.id) { throw 'sdk_response_id_mismatch' }
    return $answer
}

try {
    $Binary = [IO.Path]::GetFullPath($Binary)
    $RunDirectory = [IO.Path]::GetFullPath($RunDirectory)
    $helper = Join-Path $PSScriptRoot 'windows_clipboard_atomic.cs'
    if ((Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $ControllerSha256) { throw 'controller_pin_mismatch' }
    if ((Get-FileHash -LiteralPath $helper -Algorithm SHA256).Hash.ToLowerInvariant() -ne $HelperSha256) { throw 'clipboard_helper_pin_mismatch' }
    Add-Type -Path $helper
    if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA' -or -not [Environment]::UserInteractive -or
        -not [OwnedInteractiveDesktop]::Current()) {
        throw 'actual_interactive_sta_desktop_required'
    }
    if ((Get-FileHash -LiteralPath $Binary -Algorithm SHA256).Hash.ToLowerInvariant() -ne $BinarySha256) { throw 'sdk_pin_mismatch' }
    if ((Test-Path -LiteralPath $RunDirectory) -or (Test-Path -LiteralPath ($RunDirectory + '.result.json'))) {
        throw 'fresh_owned_runtime_and_result_required'
    }
    [void](New-Item -ItemType Directory -Path $RunDirectory)
    $ownsDirectory = $true
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true, $false)
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl.SetOwner($identity)
    $rule = New-Object Security.AccessControl.FileSystemAccessRule($identity,'FullControl','ContainerInherit,ObjectInherit','None','Allow')
    $acl.AddAccessRule($rule)
    Set-Acl -LiteralPath $RunDirectory -AclObject $acl
    [IO.File]::WriteAllText((Join-Path $RunDirectory '.owner'), $BinarySha256, $utf8)
    [void](New-Item -ItemType Directory -Path (Join-Path $RunDirectory 'requests'))
    [void](New-Item -ItemType Directory -Path (Join-Path $RunDirectory 'responses'))
    # Win32 adapter's public enable additionally verifies the visible window
    # station and current input desktop. This snapshot never reaches receipts.
    $phase = 'private_snapshot'
    $clipboard = New-Object OwnedClipboard
    $initial = @{label='initial';bytes=128}
    $initialText = Marker $initial
    $clipboard.Begin($initialText)
    if (-not $clipboard.EqualsText($initialText)) { throw 'independent_system_clipboard_unavailable' }
    $report.interactive = $true
    $phase = 'sdk_start'
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $Binary
    $state = Join-Path $RunDirectory 'state'
    $start.Arguments = $Action + ' "' + $state + '"'
    if ($Action -ne 'create') {
        if (-not $Invitation) { throw 'fresh_private_invitation_required' }
        $Invitation = [IO.Path]::GetFullPath($Invitation)
        # Invitation is deployed beside the runtime; it is never printed.
        if ([IO.Path]::GetDirectoryName($Invitation) -ne [IO.Path]::GetDirectoryName($RunDirectory)) { throw 'invitation_outside_owned_deployment' }
        $start.Arguments += ' "' + $Invitation + '"'
    }
    if ($Resources) {
        $Resources = [IO.Path]::GetFullPath($Resources)
        if ([IO.Path]::GetDirectoryName($Resources) -ne [IO.Path]::GetDirectoryName($RunDirectory)) { throw 'resources_outside_owned_deployment' }
        $start.Arguments += ' "' + $Resources + '"'
    }
    $start.UseShellExecute = $false
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $client = New-Object Diagnostics.Process
    $client.StartInfo = $start
    [void]$client.Start()
    $clientStarted = $true
    $report.ownedSdkPid = $client.Id
    $errors = $client.StandardError.ReadToEndAsync()
    $ready = Read-Sdk
    if ($ready.event -ne 'ready') { throw 'owned_sdk_startup_failed' }
    if ($Action -eq 'create') {
        $saved = Call-Sdk ([pscustomobject]@{id=0;op='export_recovery_card';path=(Join-Path $RunDirectory 'card')})
        if (-not $saved.ok) { throw 'owned_recovery_save_failed' }
        $saved = Call-Sdk ([pscustomobject]@{id=1;op='acknowledge_recovery_saved'})
        if (-not $saved.ok) { throw 'owned_recovery_ack_failed' }
    }
    Write-JsonAtomic (Join-Path $RunDirectory 'ready.json') @{ready=$ready;runId=$runId;interactive=$true;ownsFreshState=$true;clipboardMarkerOnly=$true}
    $phase = 'protocol'
    $sequence = 1
    $finish = $false
    $budget = [Diagnostics.Stopwatch]::StartNew()
    while (-not $finish -and $budget.Elapsed.TotalMinutes -lt 20) {
        $requestPath = Join-Path $RunDirectory ('requests/{0:d6}.json' -f $sequence)
        if (-not (Test-Path -LiteralPath $requestPath)) { Start-Sleep -Milliseconds 50; continue }
        $request = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($requestPath, $utf8))
        if ($request.id -ne $sequence) { throw 'mailbox_sequence_mismatch' }
        $value = $null
        switch ($request.op) {
            'copy' { $text = Marker $request; $clipboard.WriteText($text); $value = @{matches=$clipboard.EqualsText($text)} }
            'copy_empty' { $clipboard.WriteText(''); $value = @{matches=$clipboard.EqualsText('')} }
            'expect' { $text = Marker $request; $value = @{matches=$clipboard.EqualsText($text)} }
            'expect_empty' { $value = @{matches=$clipboard.EqualsText('')} }
            'sdk' { $value = Call-Sdk $request.command; if ($value.ok -ne $request.expectedSdkOk) { throw 'sdk_result_assertion_failed' } }
            'finish' { $finish = $true; $value = @{finished=$true} }
            default { throw 'unsupported_mailbox_operation' }
        }
        if ($request.op -in @('copy','copy_empty','expect','expect_empty') -and -not $value.matches) {
            throw 'independent_system_clipboard_assertion_failed'
        }
        Write-JsonAtomic (Join-Path $RunDirectory ('responses/{0:d6}.json' -f $sequence)) @{id=$sequence;ok=$true;result=$value}
        $sequence++
    }
    if (-not $finish) { throw 'owned_interactive_controller_deadline' }
    $report.status = 'pass'
} catch {
    # Fixed classification only; exception messages/clipboard objects are private.
    $report.failure = 'interactive_controller_failed'
    $report.failure_stage = $phase
    $cause = $_.Exception
    while ($null -ne $cause.InnerException) { $cause = $cause.InnerException }
    # Fixed native-helper codes only; never serialize an arbitrary exception
    # message because it can include private clipboard or invitation values.
    if ($cause.Message -match '^(clipboard_[a-z_]+|prior_clipboard_format_unsupported|interactive_desktop_required)$') {
        $report.failure_code = $cause.Message
    }
} finally {
    $processJoined, $stderrJoined, $stdoutJoined = $false, $false, $false
    if ($clientStarted) {
        try {
            if (-not $client.HasExited) {
                $stopped = Call-Sdk ([pscustomobject]@{id=2147483647;op='stop'})
                if (-not $stopped.ok) { $cleanupErrors.Add('owned_sdk_stop_rejected') }
            }
        } catch { $cleanupErrors.Add('owned_sdk_stop_failed') }
        try { $client.StandardInput.Close() } catch { $cleanupErrors.Add('owned_sdk_stdin_close_failed') }
        try {
            $processJoined = $client.WaitForExit(25000)
            if (-not $processJoined -or $client.ExitCode -ne 0) { $cleanupErrors.Add('owned_sdk_exit_not_clean') }
        } catch { $cleanupErrors.Add('owned_sdk_wait_failed') }
        try {
            $stderrJoined = $null -ne $errors -and $errors.Wait(3000)
            if (-not $stderrJoined) { $cleanupErrors.Add('owned_sdk_stderr_not_joined') }
        } catch { $cleanupErrors.Add('owned_sdk_stderr_join_failed') }
        try {
            $stdoutJoined = $null -eq $script:lastSdkRead -or $script:lastSdkRead.Wait(3000)
            if (-not $stdoutJoined) { $cleanupErrors.Add('owned_sdk_stdout_not_joined') }
        } catch { $cleanupErrors.Add('owned_sdk_stdout_join_failed') }
        $allTerminal = $processJoined -and $stderrJoined -and $stdoutJoined
    } else { $allTerminal = $true }
    if ($null -ne $clipboard -and $clipboard.HasSnapshot -and $allTerminal) {
        try {
            $restored = $clipboard.RestoreIfOwned()
            $report.restoreDisposition = if ($restored) { 'direct_controller_revision_restored' } else { 'later_or_unproven_revision_preserved' }
        } catch { $cleanupErrors.Add('prior_clipboard_restore_failed') }
    }
    if ($null -ne $clipboard) {
        try { $clipboard.Dispose() } catch { $cleanupErrors.Add('private_clipboard_snapshot_dispose_failed') }
    }
    # Every cleanup operation is isolated so an owner-file read or removal
    # failure cannot suppress the final failed receipt attempt.
    if ($ownsDirectory -and $allTerminal) {
        try {
            if ([IO.File]::ReadAllText((Join-Path $RunDirectory '.owner'),$utf8) -ne $BinarySha256) { throw 'private_owner_mismatch' }
            Remove-Item -LiteralPath $RunDirectory -Recurse -Force
        } catch { $cleanupErrors.Add('owned_private_state_removal_failed') }
    }
    try { $report.privateStateRemoved = -not (Test-Path -LiteralPath $RunDirectory) }
    catch { $cleanupErrors.Add('owned_private_state_observation_failed'); $report.privateStateRemoved = $false }
    $report.clipboardRestored = $restored
    $report.allOwnedProcessesTerminal = $allTerminal
    $report.stderrReaderJoined = $stderrJoined
    $report.stdoutReaderJoined = $stdoutJoined
    $report.cleanupErrors = @($cleanupErrors)
    if ($cleanupErrors.Count -gt 0 -or -not $report.privateStateRemoved) { $report.status = 'failed' }
    if ($ownsDirectory) {
        try { Write-JsonAtomic ($RunDirectory + '.result.json') $report }
        catch { $report.status = 'failed'; [Console]::Error.WriteLine('owned final receipt publication failed') }
    } else { [Console]::Error.WriteLine('interactive host precondition failed; no runtime claimed') }
}
if ($report.status -ne 'pass') { exit 1 }
