# Portability check.
#
# Type-checks the backend-neutral crates for Linux from a Windows machine and
# makes sure they do not use the Windows API or the UI/async stacks. Running
# their tests on Linux happens in CI (.github/workflows/ci.yml) or under WSL.
#
# Usage: powershell -File scripts/check-linux.ps1 [-SkipWsl]

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

$target = 'x86_64-unknown-linux-gnu'
$crates = @('gevrek-model', 'gevrek-engine', 'gevrek-zones')
$dirs = @('crates/model', 'crates/engine', 'crates/zones')

if (-not (rustup target list --installed | Select-String -SimpleMatch $target)) {
    rustup target add $target
}

$packageArgs = $crates | ForEach-Object { '-p', $_ }
cargo check @packageArgs --all-targets --target $target
if ($LASTEXITCODE -ne 0) { throw "cargo check for $target failed" }

# The `windows` crate, egui/eframe, or tokio must not appear. `windows::` is
# matched only as a crate path, not the model's own `platform::windows` module.
$forbidden = '(^|[^:A-Za-z_])windows::|\begui\b|\beframe\b|\btokio\b'
$hits = Get-ChildItem -Recurse -Include *.rs -Path $dirs |
    Select-String -Pattern $forbidden |
    Where-Object { $_.Line -notmatch '^\s*//' }
if ($hits) {
    $hits | ForEach-Object { Write-Host "$($_.Path):$($_.LineNumber): $($_.Line.Trim())" }
    throw 'Backend-neutral crates use a forbidden dependency'
}

# The settings program: no Windows API of its own (it may use egui and
# tokio; what it asks of the system goes through gevrek-system).
cargo check -p gevrek-studio --all-targets --target $target
if ($LASTEXITCODE -ne 0) { throw "cargo check of gevrek-studio for $target failed" }
$studioHits = Get-ChildItem -Recurse -Include *.rs -Path 'crates/studio' |
    Select-String -Pattern '(^|[^:A-Za-z_])windows::|gevrek_win32' |
    Where-Object { $_.Line -notmatch '^\s*//' }
if ($studioHits) {
    $studioHits | ForEach-Object { Write-Host "$($_.Path):$($_.LineNumber): $($_.Line.Trim())" }
    throw 'The studio crate uses the Windows API or the Windows backend'
}
# What the shell stands on that needs no library of the system for a check
# (the toolkit, the system's services, the link to the window manager):
# their Linux side is type-checked here too.
$shell = @('gevrek-ui', 'gevrek-system', 'gevrek-link')
$shellArgs = $shell | ForEach-Object { '-p', $_ }
cargo check @shellArgs --all-targets --target $target
if ($LASTEXITCODE -ne 0) { throw "cargo check of the shell's base crates for $target failed" }

# The shell and its host for Wayland (their Wayland library's build script
# asks pkg-config for xkbcommon; the shell wants PipeWire too), the Linux
# backend, the lock screen and the display manager (udev, seat, input, PAM)
# need the system's own libraries to be there even for a check: they are
# checked inside WSL, where one is set up with them (`-SkipWsl` leaves that
# out; without WSL, or without cargo in it, it is said and left out).
$inWsl = 'the shell, its Wayland host, the Linux backend, the lock screen and the display manager'
if ($args -notcontains '-SkipWsl') {
    $wsl = Get-Command wsl.exe -ErrorAction SilentlyContinue
    $hasCargo = $false
    if ($wsl) {
        wsl.exe bash -lc 'command -v cargo >/dev/null 2>&1'
        $hasCargo = $LASTEXITCODE -eq 0
    }
    if ($hasCargo) {
        # (With strokes the other way: the shell inside takes a backslash
        # for itself.)
        $here = (wsl.exe wslpath -a ((Get-Location).Path -replace '\\', '/')).Trim()
        # (A target folder of its own, on the Linux side: the one in the
        # repository is the Windows build's, and slow from inside WSL.)
        wsl.exe bash -lc "cd '$here' && CARGO_TARGET_DIR=`$HOME/.cache/gevrek-target cargo check -p gevrek-shellkit -p gevrek-shell -p gevrek-smithay -p gevrek-lock -p gevrek-dm --all-targets"
        if ($LASTEXITCODE -ne 0) { throw 'cargo check of the Linux-only crates inside WSL failed' }
        Write-Host "OK: $inWsl type-check inside WSL"
    } elseif ($wsl) {
        Write-Host "Not checked: $inWsl (the WSL here has no cargo)"
    } else {
        Write-Host "Not checked: $inWsl (no WSL here)"
    }
}
Write-Host "OK: $($crates -join ', ') build for $target with no Windows/UI/async dependency; gevrek-studio builds for $target with no Windows dependency"
