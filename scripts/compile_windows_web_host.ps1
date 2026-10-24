param([Parameter(Mandatory=$true)][string]$Inputs, [Parameter(Mandatory=$true)][string]$Output)
$ErrorActionPreference = 'Stop'
$inputGraph = Get-Content -LiteralPath $Inputs -Raw | ConvertFrom-Json
foreach ($pin in $inputGraph.files) {
    if ((Get-FileHash -LiteralPath $pin.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $pin.sha256) { throw 'Frozen compile input changed' }
}
Add-Type -Path $inputGraph.roslynCore
Add-Type -Path $inputGraph.roslynCSharp
$references = @($inputGraph.references | ForEach-Object { [Microsoft.CodeAnalysis.MetadataReference]::CreateFromFile($_) })
$parse = [Microsoft.CodeAnalysis.CSharp.CSharpParseOptions]::Default.WithLanguageVersion([Microsoft.CodeAnalysis.CSharp.LanguageVersion]::CSharp5)
$tree = [Microsoft.CodeAnalysis.CSharp.CSharpSyntaxTree]::ParseText([IO.File]::ReadAllText($inputGraph.source), $parse)
$settings = [Microsoft.CodeAnalysis.CSharp.CSharpCompilationOptions]::new([Microsoft.CodeAnalysis.OutputKind]::DynamicallyLinkedLibrary)
$settings = $settings.WithPlatform([Microsoft.CodeAnalysis.Platform]::X64)
$compilation = [Microsoft.CodeAnalysis.CSharp.CSharpCompilation]::Create('Miopunch.WindowsWebContentHost',
    [Microsoft.CodeAnalysis.SyntaxTree[]]@($tree), [Microsoft.CodeAnalysis.MetadataReference[]]$references, $settings)
$assembly = Join-Path $Output 'Miopunch.WindowsWebContentHost.dll'
$stream = [IO.File]::Create($assembly)
try { $emitted = $compilation.Emit($stream) } finally { $stream.Dispose() }
$errors = @($emitted.Diagnostics | Where-Object Severity -eq 'Error' | ForEach-Object { $_.ToString() })
$tokens = $null; $parseErrors = $null
[Management.Automation.Language.Parser]::ParseFile($inputGraph.wrapper, [ref]$tokens, [ref]$parseErrors) | Out-Null
$report = @{ csharpCompiled=$emitted.Success; compileErrors=$errors; powerShellParseErrors=@($parseErrors | ForEach-Object { $_.ErrorId });
    powerShellVersion=$PSVersionTable.PSVersion.ToString(); language='CSharp5'; architecture='x64'; referencesPinned=$true;
    WindowsExecuted=$false; hostAssemblyLoaded=$false; WebView2Instantiated=$false; nativeMethodsInvoked=$false }
$report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $Output 'compiler-result.json') -Encoding UTF8
if (-not $emitted.Success -or $parseErrors.Count -ne 0) { exit 1 }
