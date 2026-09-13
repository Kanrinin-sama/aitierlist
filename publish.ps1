[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $PreparedReleaseDirectory,
    [string] $RemoteHost = 'kanrinin',
    [string] $WebRoot = '/opt/homebrew/var/www/files/aitierlist',
    [string] $UpdaterRoot = (Join-Path (Split-Path $PSScriptRoot -Parent) 'blockitall-update'),
    [switch] $Force
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$prepared = [System.IO.Path]::GetFullPath($PreparedReleaseDirectory)
if (-not (Test-Path -LiteralPath $prepared -PathType Container)) {
    throw "prepared release directory does not exist: $prepared"
}
$publisher = Join-Path $UpdaterRoot 'target\release\blockitall-update.exe'
if (-not (Test-Path -LiteralPath $publisher -PathType Leaf)) {
    throw "shared publisher does not exist: $publisher"
}
$arguments = @('publish', '--prepared-dir', $prepared, '--app-id', 'aitierlist',
    '--public-key', (Join-Path $PSScriptRoot 'public-key.json'), '--ssh-host', $RemoteHost,
    '--remote-app-root', $WebRoot, '--require-upx')
if ($Force) { $arguments += '--allow-identical' }
& $publisher @arguments
if ($LASTEXITCODE -ne 0) { throw "blockitall-update publish failed (exit $LASTEXITCODE)" }
