# Register a Scheduled Task that keeps the EEW generator standby running.
# ASCII-only: paths are derived from this script's own location.
# Run:  powershell -NoProfile -ExecutionPolicy Bypass -File tools\install_autostart.ps1
# Remove: tools\uninstall_autostart.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$taskName = 'EEW-Generator-Standby'
$ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$supervisor = Join-Path $PSScriptRoot 'standby.ps1'
$exe = Join-Path $root 'target\release\eew-generator.exe'

if (-not (Test-Path -LiteralPath $exe)) {
    Write-Host "ERROR: exe not found -> $exe"
    Write-Host "Build it first: cargo build --release"
    exit 1
}

$argument = '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "' + $supervisor + '"'

$action = New-ScheduledTaskAction -Execute $ps -Argument $argument -WorkingDirectory $root

# 1) start at logon (15s delay so the shell is ready)
$tLogon = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$tLogon.Delay = 'PT15S'

# 2) watchdog: re-check every 10 minutes; MultipleInstances=IgnoreNew makes this a no-op
#    while the supervisor is alive, and a restart if it was killed.
$tRepeat = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) `
    -RepetitionInterval (New-TimeSpan -Minutes 10) `
    -RepetitionDuration (New-TimeSpan -Days 3650)

$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -MultipleInstances IgnoreNew `
    -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -RestartCount 999 `
    -RestartInterval (New-TimeSpan -Minutes 1)

$principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" `
    -LogonType Interactive -RunLevel Limited

Register-ScheduledTask -TaskName $taskName `
    -Action $action `
    -Trigger @($tLogon, $tRepeat) `
    -Settings $settings `
    -Principal $principal `
    -Description 'NHK-style EEW image generator: realtime standby (P2P 556) with self-restart supervisor.' `
    -Force | Out-Null

Write-Host "Registered task: $taskName"
$t = Get-ScheduledTask -TaskName $taskName
Write-Host ("Triggers : " + (($t.Triggers | ForEach-Object { $_.CimClass.CimClassName }) -join ', '))
Write-Host ("Action   : " + $t.Actions[0].Execute + ' ' + $t.Actions[0].Arguments)
Write-Host ("Settings : MultipleInstances=" + $t.Settings.MultipleInstances + ' ExecutionTimeLimit=' + $t.Settings.ExecutionTimeLimit)

Start-ScheduledTask -TaskName $taskName
Start-Sleep -Seconds 6
$p = Get-ScheduledTaskInfo -TaskName $taskName
Write-Host ("State    : " + (Get-ScheduledTask -TaskName $taskName).State + " LastResult=" + $p.LastTaskResult)
