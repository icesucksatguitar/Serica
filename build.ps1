<#
.SYNOPSIS
    Convenience wrapper for the Neutrino Browser build script (build.py).

.DESCRIPTION
    Builds, versions, and stages the Rust (Tauri v2) release into release/rust.

.PARAMETER bump
    Version bump increment: 'major', 'minor', 'patch', or 'none' (default: 'none')

.PARAMETER clean
    Clean build artifacts before building

.PARAMETER run
    Launch the executable after building

.EXAMPLE
    .\build.ps1 -bump none
    .\build.ps1 -bump patch -clean
#>

[CmdletBinding()]
param (
    [ValidateSet("major", "minor", "patch", "none")]
    [string]$bump = "none",

    [switch]$clean,
    [switch]$run
)

$scriptPath = Join-Path $PSScriptRoot "build.py"
$pythonArgs = @($scriptPath, "--bump", $bump)

if ($clean) {
    $pythonArgs += "--clean"
}
if ($run) {
    $pythonArgs += "--run"
}

python @pythonArgs
exit $LASTEXITCODE
