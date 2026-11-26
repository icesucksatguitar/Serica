<#
.SYNOPSIS
    Convenience wrapper for the Serica Browser build script (build.py).

.DESCRIPTION
    Every way to build, try out and ship Serica. Run with no parameters for a
    menu. See build.py (or `python build.py --help`) for details.

.PARAMETER test
    Test run: build an isolated "Serica Test" copy (own profile, no auto-updates or UI patches) and open it

.PARAMETER fresh
    With -test: start from an empty test profile

.PARAMETER dev
    Dev mode: tauri dev with hot reload, on the test profile

.PARAMETER bump
    Version bump increment: 'major', 'minor', 'patch', or 'none' (default: 'none')

.PARAMETER clean
    Clean build artifacts before building

.PARAMETER bundle
    Also build the signed MSI + auto-update files into release/github (a full release)

.PARAMETER publishRelease
    Upload what -bundle staged to the GitHub release (a draft unless -live)

.PARAMETER live
    With -publishRelease: publish it and mark it Latest right away

.PARAMETER uiPatch
    Ship a React/CSS-only fix as a signed UI patch: no installer, no reinstall

.PARAMETER notes
    With -uiPatch: one line shown to users

.PARAMETER publish
    With -uiPatch: upload it so running copies pick it up

.PARAMETER force
    With -uiPatch: ship even though Rust code changed

.PARAMETER status
    Show what's built and staged

.PARAMETER run
    Launch the local build afterwards (uses your real profile)

.EXAMPLE
    .\build.ps1                                   # menu
    .\build.ps1 -test                             # try the current code
    .\build.ps1 -bump patch -bundle               # full release, then -publishRelease
    .\build.ps1 -uiPatch -notes "Smoother sidebar" -publish
#>

[CmdletBinding()]
param (
    [switch]$test,
    [switch]$fresh,
    [switch]$dev,

    [ValidateSet("major", "minor", "patch", "none")]
    [string]$bump,
    [switch]$clean,
    [switch]$bundle,
    [switch]$publishRelease,
    [switch]$live,
    [switch]$run,

    [switch]$uiPatch,
    [string]$notes,
    [switch]$publish,
    [switch]$force,

    [switch]$status
)

$pythonArgs = @((Join-Path $PSScriptRoot "build.py"))
if ($test) { $pythonArgs += "--test" }
if ($fresh) { $pythonArgs += "--fresh" }
if ($dev) { $pythonArgs += "--dev" }
if ($bump) { $pythonArgs += @("--bump", $bump) }
if ($clean) { $pythonArgs += "--clean" }
if ($bundle) { $pythonArgs += "--bundle" }
if ($publishRelease) { $pythonArgs += "--publish-release" }
if ($live) { $pythonArgs += "--live" }
if ($run) { $pythonArgs += "--run" }
if ($uiPatch) { $pythonArgs += "--ui-patch" }
if ($PSBoundParameters.ContainsKey('notes')) { $pythonArgs += @("--notes", $notes) }
if ($publish) { $pythonArgs += "--publish" }
if ($force) { $pythonArgs += "--force" }
if ($status) { $pythonArgs += "--status" }

python @pythonArgs
exit $LASTEXITCODE
