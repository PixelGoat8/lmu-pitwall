<#
.SYNOPSIS
  Dumps LMU's local REST API (localhost:6397) to JSON files, for docs/lmu-api.md.

.DESCRIPTION
  GET requests only: nothing here changes game state. Run it with LMU open,
  once in the main menu, once in the garage, once on track, so the samples
  show which endpoints only answer inside a session.

  Output: tools/rest-probe/out/<timestamp>-<label>/<endpoint>.json
          plus summary.txt (status, size, top-level keys per endpoint).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools\rest-probe\rest-probe.ps1 -Label garage
#>
param(
    [string]$Label = "sample",
    [int]$Port = 6397
)

# Endpoints known from lmu-pitwall (bridge/src/garage_api.rs) and TinyPedal
# (tinypedal/adapter/lmu_restapi.py). Add new ones here as they are found.
$endpoints = @(
    "/rest/sessions",
    "/rest/sessions/weather",
    "/rest/strategy/usage",
    "/rest/strategy/pitstop-estimate",
    "/rest/garage/UIScreen/RepairAndRefuel",
    "/rest/garage/getPlayerGarageData",
    "/rest/race/car"
)

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$outDir = Join-Path $PSScriptRoot "out\$stamp-$Label"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$summary = @()

foreach ($ep in $endpoints) {
    $url = "http://localhost:$Port$ep"
    $file = Join-Path $outDir (($ep.TrimStart('/') -replace '[/\\]', '_') + ".json")
    try {
        $r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 3
        [System.IO.File]::WriteAllText($file, $r.Content)
        $keys = ""
        try {
            $j = $r.Content | ConvertFrom-Json
            if ($j -is [System.Array]) { $keys = "array[$($j.Count)]" }
            elseif ($j) { $keys = ($j.PSObject.Properties.Name | Select-Object -First 25) -join ", " }
        } catch { $keys = "(not JSON)" }
        $summary += "{0,-48} {1}  {2,8} B  {3}" -f $ep, $r.StatusCode, $r.Content.Length, $keys
    } catch {
        $code = if ($_.Exception.Response) { [int]$_.Exception.Response.StatusCode } else { "ERR" }
        $summary += "{0,-48} {1}  {2}" -f $ep, $code, $_.Exception.Message
    }
}

$summary | Set-Content -Encoding UTF8 (Join-Path $outDir "summary.txt")
$summary
Write-Host "`nSaved to $outDir"
