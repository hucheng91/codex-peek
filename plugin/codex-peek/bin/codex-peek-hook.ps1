# Codex hook entry (Windows). Usage: codex-peek-hook.ps1 start|stop  (hook JSON on stdin)
param([string]$Action = 'start')
$ErrorActionPreference = 'SilentlyContinue'
$Version = '0.1.0'
$Repo = 'hucheng91/codex-peek'
$Asset = 'codex-peek-windows-x64.exe'
$Data = if ($env:PLUGIN_DATA) { $env:PLUGIN_DATA } else { Join-Path $env:TEMP 'codex-peek-plugin' }
$InputJson = [Console]::In.ReadToEnd()

$Bin = $env:CODEX_PEEK_BIN
if (-not $Bin) {
  $Bin = Join-Path (Join-Path $Data $Version) $Asset
  if (-not (Test-Path $Bin) -and $Action -eq 'start') {
    New-Item -ItemType Directory -Force -Path (Split-Path $Bin) | Out-Null
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/$Repo/releases/download/v$Version/$Asset" -OutFile "$Bin.part"
    if (Test-Path "$Bin.part") { Move-Item -Force "$Bin.part" $Bin }
  }
}
if (-not (Test-Path $Bin)) { exit 0 }

$flag = if ($Action -eq 'stop') { '--hook-stop' } else { '--hook-start' }
$InputJson | & $Bin $flag
exit 0
