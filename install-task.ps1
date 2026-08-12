# Register (or remove) the scheduled task that starts focal-desk at logon.
#
# Needs administrator rights, because the task runs with highest privileges
# so it can also move windows belonging to elevated processes — an admin
# terminal, for instance. The script re-launches itself elevated if needed,
# so you can start it from an ordinary terminal and just accept the prompt.
#
#   powershell -ExecutionPolicy Bypass -File install-task.ps1
#   powershell -ExecutionPolicy Bypass -File install-task.ps1 -Remove

[CmdletBinding()]
param(
    # Remove the task instead of creating it.
    [switch]$Remove,
    # Path to the built executable. Defaults to the release build beside this script.
    [string]$Exe = (Join-Path $PSScriptRoot 'target\release\focal-desk.exe')
)

$TaskName = 'focal-desk'

# True when this process already holds administrator rights.
function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
}

# Re-launch this script elevated, forwarding the arguments we were given.
function Invoke-Elevated {
    $argList = @('-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"")
    if ($Remove) { $argList += '-Remove' }
    $argList += @('-Exe', "`"$Exe`"")
    Write-Host 'Needs administrator rights - accept the prompt.' -ForegroundColor Yellow
    Start-Process -FilePath 'powershell.exe' -Verb RunAs -ArgumentList $argList
}

if (-not (Test-Admin)) { Invoke-Elevated; return }

if ($Remove) {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    Write-Host "Removed scheduled task '$TaskName'." -ForegroundColor Green
    Write-Host 'focal-desk will no longer start at logon. Any running instance keeps running.'
    return
}

if (-not (Test-Path $Exe)) {
    Write-Error "No executable at $Exe. Run 'cargo build --release' first."
    return
}

$me = "$env:USERDOMAIN\$env:USERNAME"

$action = New-ScheduledTaskAction -Execute $Exe
# Give the shell and the display topology a moment to settle before we
# start rearranging windows.
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $me
$trigger.Delay = 'PT15S'
# Interactive matters: a task in session 0 cannot touch user windows.
$principal = New-ScheduledTaskPrincipal -UserId $me -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -StartWhenAvailable -MultipleInstances IgnoreNew `
    -ExecutionTimeLimit (New-TimeSpan -Seconds 0)

Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger `
    -Principal $principal -Settings $settings -Force `
    -Description 'focal-priority window layout for the 8K desk panel. Writes focal-desk.log beside the executable; right-click the tray icon to quit.' | Out-Null

Write-Host "Registered scheduled task '$TaskName'." -ForegroundColor Green
Write-Host "  runs:  $Exe"
Write-Host '  when:  at logon, 15s delay, elevated, in your interactive session'
Write-Host ''
Write-Host 'Start it now without rebooting:   Start-ScheduledTask focal-desk'
Write-Host 'Remove it later:                  install-task.ps1 -Remove'
