# Remove what install.ps1 put on this machine for the person who runs this:
# the programs, the PATH entry, the Start menu's entries, the task that
# starts gevrek at sign-in, and Windows' note of how to remove it.
#
#   powershell -ExecutionPolicy Bypass -File uninstall.ps1
#
# The configuration (%APPDATA%\gevrek) and what gevrek kept of its state
# (%LOCALAPPDATA%\gevrek) stay, unless -Purge is given.
[CmdletBinding()]
param(
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\gevrek'),
    [string]$StartMenu = (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\gevrek'),
    [string]$RegistryRoot = 'HKCU:',
    # Leave a running gevrek and its sign-in task alone (for the check of
    # this script, which removes a copy that was never the person's).
    [switch]$NoStop,
    # The configuration and the state too.
    [switch]$Purge
)
$ErrorActionPreference = 'Stop'
$installed = Join-Path $Prefix 'gevrek.exe'

if ((-not $NoStop) -and (Test-Path $installed)) {
    # Not started at sign-in any more, and not running: every window gevrek
    # hid is put back by its stop.
    try { & $installed disable-autostart | Out-Null } catch { Write-Host "The sign-in task was not removed: $_" }
    try { & $installed stop all | Out-Null } catch { Write-Host "gevrek was not stopped: $_" }
    Start-Sleep -Seconds 2
    # The helper for administrator windows, where it is registered: its
    # task and its copy in Program Files (Windows asks for an administrator).
    $registered = $false
    try { & schtasks.exe /query /tn 'gevrek elevated helper (main)' 2>$null | Out-Null; $registered = ($LASTEXITCODE -eq 0) } catch { }
    if ($registered) {
        try { & $installed elevated remove } catch { Write-Host "The helper's task was not removed: $_" }
    }
}

$key = Join-Path $RegistryRoot 'Environment'
if (Test-Path $key) {
    $path = (Get-Item $key).GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    $parts = @($path -split ';' | Where-Object { $_ -ne '' })
    if ($parts -contains $Prefix) {
        $left = @($parts | Where-Object { $_ -ne $Prefix })
        Set-ItemProperty -Path $key -Name 'Path' -Value ($left -join ';') -Type ExpandString
        Write-Host "$Prefix is off your PATH."
    }
}

if (Test-Path $StartMenu) {
    Remove-Item -Recurse -Force $StartMenu
    Write-Host "The Start menu's entries are gone."
}

$uninstall = Join-Path $RegistryRoot 'Software\Microsoft\Windows\CurrentVersion\Uninstall\gevrek'
if (Test-Path $uninstall) { Remove-Item -Recurse -Force $uninstall }

if (Test-Path $Prefix) {
    # This script may be the copy in that folder: PowerShell has read it,
    # so its file can go with the rest.
    Remove-Item -Recurse -Force $Prefix
    Write-Host "The programs in $Prefix are gone."
}

if ($Purge) {
    foreach ($folder in @((Join-Path $env:APPDATA 'gevrek'), (Join-Path $env:LOCALAPPDATA 'gevrek'))) {
        if (Test-Path $folder) { Remove-Item -Recurse -Force $folder; Write-Host "Removed $folder." }
    }
} else {
    Write-Host 'Your configuration (%APPDATA%\gevrek) is still there; -Purge removes it too.'
}
Write-Host 'gevrek is removed.'
