# Try install.ps1 and uninstall.ps1 of an unpacked package without touching
# what is the person's: the programs go to a folder of this check's own, the
# Start menu's entries to another, and what would be written to the person's
# registry is written under a key of this check's own
# (HKCU:\Software\gevrek-install-check), which is removed at the end. A
# gevrek that is running is left alone.
#
#   pwsh -NoProfile -File packaging\windows\check.ps1 target\package\gevrek-...-windows-x64
param([Parameter(Mandatory = $true)][string]$Package)
$ErrorActionPreference = 'Stop'
$Package = (Resolve-Path $Package).Path
$root = Join-Path ([IO.Path]::GetTempPath()) ("gevrek-install-check-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
$prefix = Join-Path $root 'programs'
$menu = Join-Path $root 'menu'
$registry = 'HKCU:\Software\gevrek-install-check'
$programs = @('gevrek.exe', 'gevrek-wm.exe', 'gevrek-studio.exe', 'gevrek-shell.exe')
$failed = 0
function Said([string]$what, [bool]$holds) {
    if ($holds) { Write-Host "ok    $what" } else { Write-Host "FAIL  $what"; $script:failed++ }
}
function Path-Parts { @((Get-Item "$registry\Environment").GetValue('Path', '', 'DoNotExpandEnvironmentNames') -split ';') }

New-Item -ItemType Directory -Force -Path $root | Out-Null
if (Test-Path $registry) { Remove-Item -Recurse -Force $registry }
New-Item -Path "$registry\Environment" -Force | Out-Null
# A PATH as a person has one: with a name that is filled in when it is used.
Set-ItemProperty -Path "$registry\Environment" -Name 'Path' -Value 'C:\one;%USERPROFILE%\two' -Type ExpandString
try {
    $common = @{ Prefix = $prefix; StartMenu = $menu; RegistryRoot = $registry; NoStop = $true }
    $first = & (Join-Path $Package 'install.ps1') @common 6>&1 | Out-String
    Said 'a first install says so' ($first -match 'Installed: gevrek')
    Said 'the five programs are there' (@($programs | Where-Object { Test-Path (Join-Path $prefix $_) }).Count -eq 5)
    Said 'the way out is beside them' ((Test-Path (Join-Path $prefix 'uninstall.ps1')) -and (Test-Path (Join-Path $prefix 'README.txt')))
    $parts = Path-Parts
    Said 'the folder is on the PATH, after what was there, which is as it was' (($parts -join ';') -eq "C:\one;%USERPROFILE%\two;$prefix")
    $links = @(Get-ChildItem $menu -Filter *.lnk)
    $shell = New-Object -ComObject WScript.Shell
    $targets = @($links | ForEach-Object { $shell.CreateShortcut($_.FullName).TargetPath })
    Said 'the Start menu has three entries that start the installed programs' (($links.Count -eq 3) -and (@($targets | Where-Object { $_.StartsWith($prefix) }).Count -eq 3))
    $key = "$registry\Software\Microsoft\Windows\CurrentVersion\Uninstall\gevrek"
    $entry = Get-ItemProperty $key
    Said 'Windows is told how to remove it' (($entry.DisplayName -eq 'gevrek') -and ($entry.UninstallString -match 'uninstall\.ps1') -and ($entry.InstallLocation -eq $prefix) -and ($entry.DisplayVersion -match '^\d'))
    $version = (& (Join-Path $prefix 'gevrek.exe') --version) | Select-Object -First 1
    Said "the installed gevrek runs ($version)" ($version -match '^gevrek ')

    # The same package again: an update.
    $second = & (Join-Path $Package 'install.ps1') @common 6>&1 | Out-String
    Said 'a second install is an update' ($second -match 'Updated: gevrek')
    Said 'the build before is kept' (@($programs | Where-Object { Test-Path (Join-Path $prefix "previous\$_") }).Count -eq 5)
    Said 'the folder is on the PATH once' (@(Path-Parts | Where-Object { $_ -eq $prefix }).Count -eq 1)

    # Removed by the copy that was installed.
    $removed = & (Join-Path $prefix 'uninstall.ps1') -Prefix $prefix -StartMenu $menu -RegistryRoot $registry -NoStop 6>&1 | Out-String
    Said 'the uninstall says it is removed' ($removed -match 'gevrek is removed')
    Said 'the programs are gone' (-not (Test-Path $prefix))
    Said 'the Start menu entries are gone' (-not (Test-Path $menu))
    Said 'the PATH is as it was before the install' (((Path-Parts) -join ';') -eq 'C:\one;%USERPROFILE%\two')
    Said 'Windows no longer lists it' (-not (Test-Path $key))
} finally {
    if (Test-Path $registry) { Remove-Item -Recurse -Force $registry }
    if (Test-Path $root) { Remove-Item -Recurse -Force $root }
}
if ($failed -gt 0) { Write-Host "$failed failed"; exit 1 }
Write-Host 'all passed'
