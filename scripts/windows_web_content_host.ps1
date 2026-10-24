#Requires -Version 5.1
<# Content-only technical host; prepared source, not actual Windows qualification.
The caller owns the provider-bound gateway, closes this host's original stdin
on its closing event, cancels/joins the gateway and SDK, and retains failures. #>
param([Parameter(Mandatory=$true)][string]$OptionsPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Windows host only' }
if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') { throw 'Run with -STA' }
if (-not [Environment]::Is64BitProcess) { throw 'A pinned x64 runtime and host process are required' }
[Console]::InputEncoding = New-Object Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
$raw = Get-Content -LiteralPath $OptionsPath -Raw
$options = $raw | ConvertFrom-Json
foreach ($item in @($options.coreAssembly, $options.formsAssembly, $options.loader, $options.runtimeExecutable, $options.hostAssembly)) {
    if ($item.sha256 -notmatch '^[0-9a-f]{64}$' -or
        (Get-FileHash -LiteralPath $item.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $item.sha256) { throw 'Prepared host pin mismatch' }
}
$gateway = [Uri]$options.gateway
if ($gateway.Scheme -ne 'https' -or $gateway.Host -ne '127.0.0.1' -or $gateway.UserInfo) { throw 'Owned HTTPS loopback gateway required' }
if ($options.gatewayCertificateSHA256 -notmatch '^[0-9a-f]{64}$') { throw 'Exact public gateway leaf pin required' }
if (Test-Path -LiteralPath $options.profile) { throw 'A fresh empty content profile is required' }
foreach ($entry in Get-ChildItem Env:WEBVIEW2_*) { throw 'Inherited WebView2 override is not allowed in this technical host' }
New-Item -ItemType Directory -Path $options.profile | Out-Null
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Web.Extensions
Add-Type -Path $options.coreAssembly.path
Add-Type -Path $options.formsAssembly.path
Add-Type -Path $options.hostAssembly.path
$code = [Miopunch.WebContentHost]::Run($raw)
exit $code
