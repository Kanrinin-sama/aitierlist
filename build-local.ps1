[CmdletBinding()]
param(
    [int] $ShutdownTimeoutSeconds = 30,
    [int] $StartupTimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:OS -ne 'Windows_NT') {
    throw 'build-local.ps1 requires Windows'
}

$root = [System.IO.Path]::GetFullPath($PSScriptRoot)
$cargo = 'C:\Users\kaltsit\.cargo\bin\cargo.exe'
$normalExecutable = Join-Path $root 'target\release\aitierlist.exe'
$allowedExecutables = @(
    $normalExecutable,
    'C:\projects\aitierlist-build-final\release\aitierlist.exe',
    'C:\projects\aitierlist-build-refinements\release\aitierlist.exe'
) | ForEach-Object { [System.IO.Path]::GetFullPath($_) }

if (-not (Test-Path -LiteralPath $cargo -PathType Leaf)) {
    throw "pinned Cargo executable is unavailable: $cargo"
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class AiTierListBuildWindow {
    public delegate bool EnumCallback(IntPtr window, IntPtr parameter);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool EnumWindows(EnumCallback callback, IntPtr parameter);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern int GetClassName(IntPtr window, System.Text.StringBuilder name, int length);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool PostMessage(IntPtr window, uint message, UIntPtr wParam, IntPtr lParam);
}
'@

function Get-AiTierListProcesses {
    @(Get-Process -Name 'aitierlist' -ErrorAction SilentlyContinue | Where-Object {
        try {
            $path = [System.IO.Path]::GetFullPath($_.Path)
            $allowedExecutables -contains $path
        }
        catch {
            $false
        }
    })
}

function Get-AiTierListWindow([int[]] $AllowedProcessIds) {
    $windows = [System.Collections.Generic.List[IntPtr]]::new()
    $callback = [AiTierListBuildWindow+EnumCallback] {
        param([IntPtr] $window, [IntPtr] $parameter)
        [uint32] $processId = 0
        [void] [AiTierListBuildWindow]::GetWindowThreadProcessId($window, [ref] $processId)
        if ($AllowedProcessIds -contains [int] $processId) {
            $className = [System.Text.StringBuilder]::new(256)
            [void] [AiTierListBuildWindow]::GetClassName($window, $className, $className.Capacity)
            if ($className.ToString() -ceq 'AiTierListDesktopWindow') {
                $windows.Add($window)
            }
        }
        $true
    }
    [void] [AiTierListBuildWindow]::EnumWindows($callback, [IntPtr]::Zero)
    if ($windows.Count -gt 1) {
        throw 'more than one allowed AI Tier List desktop control window is active'
    }
    if ($windows.Count -eq 1) { $windows[0] } else { [IntPtr]::Zero }
}

function Get-WindowProcessId([IntPtr] $Window) {
    [uint32] $processId = 0
    [void] [AiTierListBuildWindow]::GetWindowThreadProcessId($Window, [ref] $processId)
    [int] $processId
}

function Send-AiTierListCommand([uint32] $Command, [int[]] $AllowedProcessIds) {
    $window = Get-AiTierListWindow $AllowedProcessIds
    if ($window -eq [IntPtr]::Zero) {
        return $false
    }
    $processId = Get-WindowProcessId $window
    if ($AllowedProcessIds -notcontains $processId) {
        throw "the AI Tier List desktop window belongs to unexpected process $processId"
    }
    if (-not [AiTierListBuildWindow]::PostMessage(
        $window,
        0x0111,
        [UIntPtr]::new([uint64] $Command),
        [IntPtr]::Zero
    )) {
        throw "could not send desktop command $Command to AI Tier List process $processId"
    }
    $true
}

function Wait-ForExit([int[]] $ProcessIds, [int] $TimeoutSeconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($($TimeoutSeconds))
    while ([DateTime]::UtcNow -lt $deadline) {
        $remaining = @(Get-AiTierListProcesses | Where-Object { $ProcessIds -contains $_.Id })
        if ($remaining.Count -eq 0) {
            return
        }
        Start-Sleep -Milliseconds 100
    }
    throw "AI Tier List did not exit within $TimeoutSeconds seconds; the build was not started"
}

function Wait-ForExecutableUnlock([string] $Path, [int] $TimeoutSeconds) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return
    }
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $stream = [System.IO.File]::Open(
                $Path,
                [System.IO.FileMode]::Open,
                [System.IO.FileAccess]::ReadWrite,
                [System.IO.FileShare]::None
            )
            $stream.Dispose()
            return
        }
        catch [System.IO.IOException] {
            Start-Sleep -Milliseconds 100
        }
        catch [System.UnauthorizedAccessException] {
            Start-Sleep -Milliseconds 100
        }
    }
    throw "release executable remained locked after $TimeoutSeconds seconds: $Path"
}

$running = @(Get-AiTierListProcesses)
if ($running.Count -gt 0) {
    $processIds = @($running | ForEach-Object { $_.Id })
    if (-not (Send-AiTierListCommand 3 $processIds)) {
        throw 'a known AI Tier List process is running without its desktop control window'
    }
    Wait-ForExit $processIds $ShutdownTimeoutSeconds
}
Wait-ForExecutableUnlock $normalExecutable $ShutdownTimeoutSeconds

$manifest = Join-Path $root 'Cargo.toml'
$target = Join-Path $root 'target'
Push-Location -LiteralPath $root
try {
    & $cargo fmt --all --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw "cargo fmt failed with exit code $LASTEXITCODE" }
    & $cargo clippy --manifest-path $manifest --all-targets --offline --target-dir $target -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "cargo clippy failed with exit code $LASTEXITCODE" }
    & $cargo build --manifest-path $manifest --release --offline --target-dir $target
    if ($LASTEXITCODE -ne 0) { throw "cargo release build failed with exit code $LASTEXITCODE" }
}
finally {
    Pop-Location
}

$application = Start-Process -FilePath $normalExecutable `
    -WorkingDirectory (Split-Path $normalExecutable -Parent) -WindowStyle Normal -PassThru
$deadline = [DateTime]::UtcNow.AddSeconds($StartupTimeoutSeconds)
while ([DateTime]::UtcNow -lt $deadline) {
    if ($application.HasExited) {
        throw "the newly built AI Tier List exited with code $($application.ExitCode)"
    }
    $window = Get-AiTierListWindow @($application.Id)
    if ($window -ne [IntPtr]::Zero -and (Get-WindowProcessId $window) -eq $application.Id) {
        [void] (Send-AiTierListCommand 1 @($application.Id))
        Write-Output "Built and opened $normalExecutable"
        return
    }
    Start-Sleep -Milliseconds 100
}
throw "the newly built AI Tier List did not create its desktop window within $StartupTimeoutSeconds seconds"
