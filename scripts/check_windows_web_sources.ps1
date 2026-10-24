param([Parameter(Mandatory=$true)][string]$Source, [Parameter(Mandatory=$true)][string]$Output)
$ErrorActionPreference = 'Stop'
$tokens = $null; $parseErrors = $null
[Management.Automation.Language.Parser]::ParseFile($Source, [ref]$tokens, [ref]$parseErrors) | Out-Null
$report = @{ powerShellParseErrors=@($parseErrors | ForEach-Object { @{ error=$_.ErrorId; line=$_.Extent.StartLineNumber } });
    powerShellVersion=$PSVersionTable.PSVersion.ToString(); hostExecuted=$false; WindowsExecuted=$false;
    WebView2Loaded=$false; scope='local AST parse only; no assembly load, host, SDK or network initialization' }
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Output -Encoding UTF8
if ($parseErrors.Count -ne 0) { exit 1 }
