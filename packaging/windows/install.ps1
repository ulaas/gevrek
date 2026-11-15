# Install gevrek for the person who runs this, or put a newer build over an
# older one. No administrator is needed and none is asked for.
#
#   powershell -ExecutionPolicy Bypass -File install.ps1
#
# What it does:
#   - copies the programs beside this script to %LOCALAPPDATA%\Programs\gevrek
#     (the build that was there is kept in its folder `previous`);
#   - puts that folder on the person's PATH, so `gevrek` is a command;
#   - makes entries in the Start menu (the settings program; start; stop);
#   - tells Windows how to remove it (Settings, Apps, Installed apps).
# A gevrek that is running is stopped first and started again afterwards.
# The configuration (%APPDATA%\gevrek) is not touched.
#
# Updating: unpack the newer package and run its install.ps1.
#
# Nothing here needs the network.
[CmdletBinding()]
param(
    # Where the programs go.
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\gevrek'),
    # Where the Start menu's entries go.
    [string]$StartMenu = (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\gevrek'),
    # The part of the registry that holds the person's settings (another
    # one is given by the check of this script, which writes nothing real).
    [string]$RegistryRoot = 'HKCU:',
    [switch]$NoPath,
    [switch]$NoShortcuts,
    [switch]$NoUninstallEntry,
    # Leave a running gevrek alone (its programs are replaced under it and
    # it runs the old build until it is started again).
    [switch]$NoStop
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$programs = @('gevrek.exe', 'gevrek-wm.exe', 'gevrek-studio.exe', 'gevrek-shell.exe')
# Programs an older build had and this one has not.
$retired = @('gevrek-editor.exe')

foreach ($program in $programs) {
    if (-not (Test-Path (Join-Path $here $program))) {
        throw "$program is not beside this script: run install.ps1 from the unpacked package"
    }
}

function Version-Of([string]$file) {
    if (-not (Test-Path $file)) { return $null }
    try { return ((& $file --version) 2>$null | Select-Object -First 1) } catch { return $null }
}

$installed = Join-Path $Prefix 'gevrek.exe'
$before = Version-Of $installed
$new = Version-Of (Join-Path $here 'gevrek.exe')
if ($before) { Write-Host "Installed: $before" } else { Write-Host 'gevrek is not installed yet.' }
Write-Host "This package: $new"

# A gevrek that runs is stopped, and remembered.
$wasRunning = $false
if ((-not $NoStop) -and (Test-Path $installed)) {
    $status = (& $installed status) 2>$null | Out-String
    if ($status -match 'wm\s*:\s*running') {
        $wasRunning = $true
        Write-Host 'gevrek is running: stopping it for the update.'
        & $installed stop all | Out-Null
        Start-Sleep -Seconds 2
    }
}

New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
$previous = Join-Path $Prefix 'previous'
New-Item -ItemType Directory -Force -Path $previous | Out-Null
foreach ($program in $programs) {
    $to = Join-Path $Prefix $program
    if (Test-Path $to) {
        # What ran until now stays at hand. A program that is running
        # cannot be written over, but it can be moved aside.
        Move-Item -Force -Path $to -Destination (Join-Path $previous $program)
    }
    Copy-Item -Force -Path (Join-Path $here $program) -Destination $to
}
foreach ($program in $retired) {
    $old = Join-Path $Prefix $program
    if (Test-Path $old) { Remove-Item -Force $old; Write-Host "Removed $program (no longer part of gevrek)." }
}
# The way out, beside the programs.
foreach ($file in @('uninstall.ps1', 'README.txt')) {
    if (Test-Path (Join-Path $here $file)) { Copy-Item -Force (Join-Path $here $file) (Join-Path $Prefix $file) }
}
Write-Host "The programs are in $Prefix (the build before in $previous)."

if (-not $NoPath) {
    $key = Join-Path $RegistryRoot 'Environment'
    if (-not (Test-Path $key)) { New-Item -Path $key -Force | Out-Null }
    # (Read as it is written: with its %NAMES% not filled in.)
    $path = (Get-Item $key).GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    $parts = @($path -split ';' | Where-Object { $_ -ne '' })
    if ($parts -notcontains $Prefix) {
        $parts += $Prefix
        Set-ItemProperty -Path $key -Name 'Path' -Value ($parts -join ';') -Type ExpandString
        if ($RegistryRoot -eq 'HKCU:') {
            # Programs that are open are told (a new terminal has it).
            $signature = '[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr w, uint m, UIntPtr a, string b, uint f, uint t, out UIntPtr r);'
            $user32 = Add-Type -MemberDefinition $signature -Name 'GevrekTell' -Namespace 'Gevrek' -PassThru
            $result = [UIntPtr]::Zero
            [void]$user32::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 2000, [ref]$result)
        }
        Write-Host "$Prefix is on your PATH now: a new terminal knows the command gevrek."
    }
}

if (-not $NoShortcuts) {
    New-Item -ItemType Directory -Force -Path $StartMenu | Out-Null
    $shell = New-Object -ComObject WScript.Shell
    $entries = @(
        @{ Name = 'gevrek settings'; Target = (Join-Path $Prefix 'gevrek-studio.exe'); Arguments = ''; Words = "gevrek's settings program" },
        @{ Name = 'gevrek (start)'; Target = (Join-Path $Prefix 'gevrek.exe'); Arguments = 'start all'; Words = 'Start the window manager and the shell' },
        @{ Name = 'gevrek (stop)'; Target = (Join-Path $Prefix 'gevrek.exe'); Arguments = 'stop all'; Words = 'Stop gevrek; every window is put back' }
    )
    foreach ($entry in $entries) {
        $shortcut = $shell.CreateShortcut((Join-Path $StartMenu ($entry.Name + '.lnk')))
        $shortcut.TargetPath = $entry.Target
        $shortcut.Arguments = $entry.Arguments
        $shortcut.WorkingDirectory = $Prefix
        $shortcut.Description = $entry.Words
        $shortcut.Save()
    }
    Write-Host 'The Start menu has gevrek settings, gevrek (start) and gevrek (stop).'
}

if (-not $NoUninstallEntry) {
    $key = Join-Path $RegistryRoot 'Software\Microsoft\Windows\CurrentVersion\Uninstall\gevrek'
    New-Item -Path $key -Force | Out-Null
    $remove = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "' + (Join-Path $Prefix 'uninstall.ps1') + '"'
    Set-ItemProperty -Path $key -Name 'DisplayName' -Value 'gevrek'
    Set-ItemProperty -Path $key -Name 'DisplayVersion' -Value ("$new" -replace '^gevrek\s*', '')
    Set-ItemProperty -Path $key -Name 'Publisher' -Value 'gevrek'
    Set-ItemProperty -Path $key -Name 'InstallLocation' -Value $Prefix
    Set-ItemProperty -Path $key -Name 'DisplayIcon' -Value (Join-Path $Prefix 'gevrek-studio.exe')
    Set-ItemProperty -Path $key -Name 'UninstallString' -Value $remove
    Set-ItemProperty -Path $key -Name 'NoModify' -Value 1 -Type DWord
    Set-ItemProperty -Path $key -Name 'NoRepair' -Value 1 -Type DWord
}

if ($wasRunning) {
    Write-Host 'Starting gevrek again.'
    & $installed start all | Out-Null
}
Write-Host ''
if ($before) { Write-Host "Updated: $before -> $new" } else { Write-Host "Installed: $new" }
Write-Host 'Start it with: gevrek start all      At sign-in: gevrek enable-autostart'
