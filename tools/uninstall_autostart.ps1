# Remove the standby Scheduled Task and stop the running supervisor / generator.
# ASCII-only.

$ErrorActionPreference = 'Continue'
$taskName = 'EEW-Generator-Standby'
$root = Split-Path -Parent $PSScriptRoot

if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
    Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
    Write-Host "Removed task: $taskName"
} else {
    Write-Host "Task not found: $taskName"
}

Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -like '*standby.ps1*' } |
    ForEach-Object {
        Write-Host ("Stop supervisor pid=" + $_.ProcessId)
        Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
    }

Get-Process eew-generator -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host ("Stop generator pid=" + $_.Id)
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
}

Write-Host "Done."
