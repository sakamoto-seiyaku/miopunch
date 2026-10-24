param(
    [Parameter(Mandatory=$true)][string]$HostScript,
    [Parameter(Mandatory=$true)][string]$Helper,
    [Parameter(Mandatory=$true)][string]$Output
)
# LOCAL static validation only: parse the host as data and compile the helper.
# Never invoke the host AST, load the compiled helper, or call a Win32 method.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$tokens = $null
$failures = $null
[void][Management.Automation.Language.Parser]::ParseFile($HostScript,[ref]$tokens,[ref]$failures)
$report = @{powerShellVersion=$PSVersionTable.PSVersion.ToString(); platform=[Environment]::OSVersion.Platform.ToString();
            hostScriptExecuted=$false; nativeMethodsInvoked=$false; clipboardAccessed=$false;
            powerShellParseErrors=@($failures | ForEach-Object {@{id=$_.ErrorId;line=$_.Extent.StartLineNumber}});
            csharpCompiled=$false; scope='local PowerShell7 parser/C#5 syntax+managed compilation; no Windows runtime qualification'}
try {
    if ($failures.Count -ne 0) { throw 'host_script_parse_failed' }
    $assembly = Join-Path $Output 'helper.dll'
    Add-Type -Path $Helper -OutputAssembly $assembly -CompilerOptions '/langversion:5' -ErrorAction Stop
    # Reading assembly identity parses metadata only; it does not load or run it.
    $report.assemblyIdentity = [Reflection.AssemblyName]::GetAssemblyName($assembly).FullName
    $report.csharpCompiled = $true
} catch {
    $report.failure = 'static_source_validation_failed'
    throw
} finally {
    [IO.File]::WriteAllText((Join-Path $Output 'result.json'), (ConvertTo-Json -Depth 16 -InputObject $report), (New-Object Text.UTF8Encoding($false)))
}
