$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$root = $PSScriptRoot
$url = 'http://' + (Get-Content -Raw -LiteralPath (Join-Path $root 'config.json') | ConvertFrom-Json).listen
$process = $null
try {
    $state = Invoke-RestMethod -Uri ($url + '/api/status') -TimeoutSec 2
    if ($state.local.hostState -in @('stopped', 'failed')) {
        $null = Invoke-RestMethod -Uri ($url + '/api/start') -Method Post -Headers @{Origin=$url} -TimeoutSec 30
    }
} catch {
    $arguments = 'run "' + (Join-Path $root 'node') + '" "' + (Join-Path $root 'config.json') + '"'
    $process = Start-Process -FilePath (Join-Path $root 'miopunch-network-demo.exe') -ArgumentList $arguments -WorkingDirectory $root -RedirectStandardOutput (Join-Path $root 'stdout.log') -RedirectStandardError (Join-Path $root 'stderr.log') -PassThru
    $process.Id | Set-Content -LiteralPath (Join-Path $root 'process.pid')
    for ($i = 0; $i -lt 60; $i++) {
        if ($process.HasExited) { throw 'Network demo exited; inspect stderr.log' }
        try { $null = Invoke-RestMethod -Uri ($url + '/api/status') -TimeoutSec 1; break } catch { Start-Sleep -Milliseconds 500 }
    }
}
$edge = @(
    (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'),
    (Join-Path $env:ProgramFiles 'Microsoft\Edge\Application\msedge.exe')
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if ($null -ne $edge) {
    $uiArguments = '--app=' + $url + '/ --user-data-dir="' + (Join-Path $root 'ui-profile') + '" --no-first-run --no-default-browser-check'
    Start-Process -FilePath $edge -ArgumentList $uiArguments
} else {
    Start-Process ($url + '/')
}
if ($null -ne $process) { $process.WaitForExit() }
