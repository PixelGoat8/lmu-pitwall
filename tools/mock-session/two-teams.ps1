<#
.SYNOPSIS
  A fake race with two cars of ours, for trying the hub's teams without the game.

.DESCRIPTION
  Generates one race seen from two drivers' PCs (Anna on one car, Bruno on
  another), replays each on a Pitwall of its own, and starts a hub with two
  teams, "Ufficiale" (Anna) and "Clienti" (Bruno). Everything lives in a
  temporary folder: your own settings, history and teams are not touched.

  The data is NOT real telemetry: see make_session.py.

  Ports: 9181 and 9182 for the drivers, 9183 for the hub.

.EXAMPLE
  .\tools\mock-session\two-teams.ps1            start, and open the hub
  .\tools\mock-session\two-teams.ps1 -Stop      stop everything
#>
param(
  [string]$Exe = "$PSScriptRoot\..\..\bridge\target\release\lmu-pitwall.exe",
  [int]$Minutes = 30,
  [switch]$Stop,
  [switch]$NoBrowser
)

$ErrorActionPreference = 'Stop'
$ports = 9181, 9182, 9183

if ($Stop) {
  foreach ($p in $ports) {
    try { Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$p/api/shutdown" -TimeoutSec 2 | Out-Null } catch {}
  }
  Write-Host 'Stopped.'
  return
}

if (-not (Test-Path $Exe)) { throw "Pitwall not found at $Exe (build it, or pass -Exe)" }
foreach ($p in $ports) {
  $tcp = New-Object System.Net.Sockets.TcpClient
  $busy = $false
  try { $busy = $tcp.ConnectAsync('127.0.0.1', $p).Wait(300) } catch { } finally { $tcp.Dispose() }
  if ($busy) { throw "Port $p is in use. Already running? Stop it with -Stop." }
}
$py = Get-Command python, python3, py -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $py) { throw 'Python not found (needed to generate the fake race).' }

$dir = Join-Path $env:TEMP 'pitwall-two-teams'
if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
New-Item -ItemType Directory -Force "$dir\anna", "$dir\bruno", "$dir\hub" | Out-Null

# One race (same seed), two points of view: car 2 is Anna's, car 3 Bruno's.
$gen = Join-Path $PSScriptRoot 'make_session.py'
$sec = $Minutes * 60
& $py.Source $gen "$dir\anna.pwrec" --seed 7 --car 2 --me Anna --team "Anna:0-$sec" --duration $sec --lap-time 60 --pit-every 8 | Out-Null
& $py.Source $gen "$dir\bruno.pwrec" --seed 7 --car 3 --me Bruno --team "Bruno:0-$sec" --duration $sec --lap-time 60 --pit-every 8 | Out-Null

# The hub's teams, as the Team panel would have saved them.
@{
  teams = @(
    @{ id = 'ufficiale'; name = 'Ufficiale'; color = '#3987e5'; follow = $null
       sources = @(@{ id = 'anna'; label = 'Anna'; url = 'ws://127.0.0.1:9181/?token=prova' }) },
    @{ id = 'clienti'; name = 'Clienti'; color = '#d95926'; follow = $null
       sources = @(@{ id = 'bruno'; label = 'Bruno'; url = 'ws://127.0.0.1:9182/?token=prova' }) }
  )
} | ConvertTo-Json -Depth 5 | ForEach-Object { [IO.File]::WriteAllText("$dir\hub\teams.json", $_) }  # UTF-8 without BOM

# No AI calls from a fake race: the key is kept from these processes only.
$key = $env:ANTHROPIC_API_KEY
$env:ANTHROPIC_API_KEY = $null
try {
  $common = '--no-browser', '--keep-alive'
  Start-Process $Exe -WindowStyle Hidden -ArgumentList (@('--data-dir', "`"$dir\anna`"", '--ws-port', 9181, '--token', 'prova', '--replay', "`"$dir\anna.pwrec`"") + $common)
  Start-Process $Exe -WindowStyle Hidden -ArgumentList (@('--data-dir', "`"$dir\bruno`"", '--ws-port', 9182, '--token', 'prova', '--replay', "`"$dir\bruno.pwrec`"") + $common)
  Start-Process $Exe -WindowStyle Hidden -ArgumentList (@('--hub', '--data-dir', "`"$dir\hub`"", '--ws-port', 9183) + $common)
} finally {
  $env:ANTHROPIC_API_KEY = $key
}

Start-Sleep -Seconds 3
if (-not $NoBrowser) { Start-Process 'http://localhost:9183/' }
Write-Host "Hub: http://localhost:9183/  (two teams: Ufficiale, Clienti)"
Write-Host "Data in $dir. Stop with: $($MyInvocation.MyCommand.Path) -Stop"
