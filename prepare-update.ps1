[CmdletBinding()]
param(
    [string] $SourceArtifact = (Join-Path $PSScriptRoot 'target\release\aitierlist.exe'),
    [Parameter(Mandatory)][string] $OutputDirectory,
    [string] $Version,
    [string] $SigningKeyFile = (Join-Path $env:LOCALAPPDATA 'blockitall-update\signing\aitierlist.key'),
    [string] $UpdaterRoot = (Join-Path (Split-Path $PSScriptRoot -Parent) 'blockitall-update'),
    [string] $UpxPath = 'C:\Users\kaltsit\AppData\Local\Microsoft\WinGet\Packages\UPX.UPX_Microsoft.Winget.Source_8wekyb3d8bbwe\upx-5.2.1-win64\upx.exe'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$line = Select-String -LiteralPath (Join-Path $PSScriptRoot 'Cargo.toml') `
    -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if ($null -eq $line) { throw 'could not read package version' }
$manifestVersion = $line.Matches[0].Groups[1].Value
if ([string]::IsNullOrWhiteSpace($Version)) { $Version = $manifestVersion }
elseif ($Version -cne $manifestVersion) { throw "requested version $Version does not match package version $manifestVersion" }
$commit = (& git -C $PSScriptRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'could not read source commit' }
$tree = (& git -C $PSScriptRoot rev-parse 'HEAD^{tree}').Trim()
if ($LASTEXITCODE -ne 0) { throw 'could not read source tree' }
. (Join-Path $UpdaterRoot 'scripts\prepare-upx-release.ps1')
$root = $PSScriptRoot
$smoke = {
    param([string] $PackedArtifact)
    $settings = Join-Path ([System.IO.Path]::GetTempPath()) ('aitierlist-smoke-' + [guid]::NewGuid().ToString('N') + '.json')
    $cache = Join-Path $root 'assets\aa-snapshot.json'
    $process = $null
    try {
        Copy-Item -LiteralPath (Join-Path ([System.Environment]::GetFolderPath('LocalApplicationData')) 'aitierlist\settings.json') `
            -Destination $settings
        $start = [System.Diagnostics.ProcessStartInfo]::new()
        $start.FileName = $PackedArtifact
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        foreach ($argument in @('--dump-table', "--settings=$settings", "--cache=$cache")) {
            $start.ArgumentList.Add($argument)
        }
        $process = [System.Diagnostics.Process]::Start($start)
        if (-not $process.WaitForExit(300000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw 'the packed executable smoke run timed out'
        }
        if ($process.ExitCode -ne 0) { throw "the packed executable smoke run failed (exit $($process.ExitCode))" }
    }
    finally {
        if ($null -ne $process) { $process.Dispose() }
        Remove-Item -LiteralPath $settings -Force -ErrorAction SilentlyContinue
    }
}
New-BlockItAllUpxRelease -AppId 'aitierlist' -Version $Version `
    -SourceArtifact ([System.IO.Path]::GetFullPath($SourceArtifact)) `
    -FileName "aitierlist-$Version-portable-x64.exe" `
    -SigningKeyFile $SigningKeyFile `
    -PublisherBinary (Join-Path $UpdaterRoot 'target\release\blockitall-update.exe') `
    -OutputDirectory ([System.IO.Path]::GetFullPath($OutputDirectory)) `
    -UpxPath $UpxPath -ExpectedUpxSha256 'd20ebe0b7b22b6be968c8c34be61f94ddea12cb11462e2cec27f548ef9574df8' `
    -CompressionProfile Best -NativePolicyProfile WindowsDesktop `
    -RequiredStampPrefix 'AITIERLIST_VERSION=' `
    -ExpectedCommit $commit -ExpectedTree $tree -PackedCompatibilityCheck $smoke
