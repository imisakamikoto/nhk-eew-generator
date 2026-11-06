# Keep the EEW generator standby process alive forever.
# Launched by a Scheduled Task at user logon; restart the child if it ever exits.
# ASCII-only on purpose: PowerShell 5.1 reads .ps1 with the system ANSI code page,
# so no non-ASCII characters are embedded here (paths are derived from $PSScriptRoot).

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\release\eew-generator.exe'
$dir = Join-Path $root 'output\auto'
$log = Join-Path $dir 'supervisor.log'
New-Item -ItemType Directory -Force -Path $dir | Out-Null

function Write-Log([string]$msg) {
    $line = '{0} {1}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $msg
    Add-Content -LiteralPath $log -Value $line -Encoding UTF8
}

if (-not (Test-Path -LiteralPath $exe)) {
    Write-Log "FATAL exe not found: $exe - run cargo build --release first"
    exit 1
}

Set-Location -LiteralPath $root
Write-Log "supervisor started pid=$PID exe=$exe"

# Single instance: the watchdog trigger and the logon trigger may overlap,
# and an extra supervisor would spawn a second generator (port drift, duplicate PNGs).
$created = $false
$mutex = New-Object System.Threading.Mutex($true, 'Local\EEW-Generator-Standby', [ref]$created)
if (-not $created) {
    Write-Log "another supervisor already holds the lock; exiting pid=$PID"
    exit 0
}

$restarts = 0
while ($true) {
    $start = Get-Date
    $code = -1
    try {
        $p = Start-Process -FilePath $exe -ArgumentList '--auto', '--interval', '3' `
             -WorkingDirectory $root -WindowStyle Hidden -PassThru -Wait
        $code = $p.ExitCode
    } catch {
        Write-Log "launch failed: $($_.Exception.Message)"
    }
    $ran = [int]((Get-Date) - $start).TotalSeconds
    $restarts++

    # fast crash loop -> back off harder
    $delay = 5
    if ($ran -lt 15) { $delay = 30 }
    Write-Log "child exited code=$code after ${ran}s; restart #$restarts in ${delay}s"
    Start-Sleep -Seconds $delay
}
