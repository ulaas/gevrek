# Build gevrek as it is in this tree, install it for you, and start it: the
# loop for trying it by hand.
#
#   pwsh scripts\dev-run.ps1            build (release), install, start again
#   pwsh scripts\dev-run.ps1 -Quick     a debug build: quicker to build, slower to run,
#                                       and with the "nightly" icon, so it is told apart
#   pwsh scripts\dev-run.ps1 -NoStart   build and install only
#   pwsh scripts\dev-run.ps1 -Stop      stop gevrek (every window is put back)
#   pwsh scripts\dev-run.ps1 -Back      put the build before this one back and start it
#
# The programs go to %USERPROFILE%\.local\bin (`cargo xtask install`), which
# is where `gevrek` on your PATH is found; the build that was there before is
# kept in %USERPROFILE%\.local\lib\gevrek\previous. A gevrek that runs is
# stopped first (its stop puts every window back) and started again with the
# new build. Your configuration (%APPDATA%\gevrek) is not touched.
#
# This is the main instance: your own gevrek. The desktop checks use an
# instance of their own and do not meet it, but do not run them while it runs.
[CmdletBinding()]
param(
    [switch]$Quick,
    [switch]$NoStart,
    [switch]$Stop,
    [switch]$Back
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$prefix = Join-Path $env:USERPROFILE '.local'
$bin = Join-Path $prefix 'bin'
$previous = Join-Path $prefix 'lib\gevrek\previous'
$gevrek = Join-Path $bin 'gevrek.exe'
$programs = @('gevrek.exe', 'gevrek-wm.exe', 'gevrek-studio.exe', 'gevrek-shell.exe')

function Stop-Gevrek {
    if (Test-Path $gevrek) {
        & $gevrek stop all 2>&1 | Out-Null
    }
    # The settings program is a window of its own: asked to close, too.
    Get-Process gevrek-studio -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and $_.Path.StartsWith($bin, [StringComparison]::OrdinalIgnoreCase) } |
        ForEach-Object { [void]$_.CloseMainWindow() }
    # Until the programs of this install are gone (their files can then be replaced).
    foreach ($try in 1..40) {
        $left = Get-Process gevrek, gevrek-wm, gevrek-shell, gevrek-studio -ErrorAction SilentlyContinue |
            Where-Object { $_.Path -and $_.Path.StartsWith($bin, [StringComparison]::OrdinalIgnoreCase) }
        if (-not $left) { return }
        Start-Sleep -Milliseconds 250
    }
    throw "these still run from ${bin}: $(($left | ForEach-Object ProcessName) -join ', '). Close them and run this again."
}

function Start-Gevrek {
    & $gevrek start all
    Start-Sleep -Seconds 2
    & $gevrek status
    Write-Host ''
    Write-Host 'Running. Settings: gevrek-studio    Health: gevrek health    Stop: pwsh scripts\dev-run.ps1 -Stop'
}

if ($Stop) {
    Stop-Gevrek
    Write-Host 'gevrek is stopped.'
    return
}

if ($Back) {
    foreach ($program in $programs) {
        if (-not (Test-Path (Join-Path $previous $program))) { throw "there is no build before this one in $previous" }
    }
    Stop-Gevrek
    foreach ($program in $programs) { Copy-Item -Force (Join-Path $previous $program) (Join-Path $bin $program) }
    Write-Host "The build before is back: $((& $gevrek --version) | Select-Object -First 1)"
    if (-not $NoStart) { Start-Gevrek }
    return
}

# Built first, while the old one still runs: a build that fails leaves it running.
Push-Location $root
try {
    $build = @('build', '--bins', '-p', 'gevrek', '-p', 'gevrek-studio', '-p', 'gevrek-shell')
    if (-not $Quick) { $build += '--release' }
    & cargo @build
    if ($LASTEXITCODE -ne 0) { throw 'the build failed; what was installed is as it was (and still runs, if it ran)' }
    Stop-Gevrek
    $install = @('xtask', 'install', '--prefix', $prefix)
    if ($Quick) { $install += '--debug' }
    & cargo @install
    if ($LASTEXITCODE -ne 0) { throw 'the install failed' }
} finally {
    Pop-Location
}
$commit = (& git -C $root rev-parse --short HEAD)
$dirty = if (& git -C $root status --porcelain) { ', with changes not committed' } else { '' }
Write-Host ''
Write-Host "Installed: $((& $gevrek --version) | Select-Object -First 1), of commit $commit$dirty, in $bin"
if (-not $NoStart) { Start-Gevrek } else { Write-Host 'Not started (-NoStart). Start it with: gevrek start all' }
