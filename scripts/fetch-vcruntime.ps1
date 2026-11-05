<#
.SYNOPSIS
  Copies the Visual C++ runtime that peeby.exe links against (VCRUNTIME140.dll,
  VCRUNTIME140_1.dll) from the local Visual Studio into third_party/vcruntime/, for the installer.

.DESCRIPTION
  Rust's MSVC target links the C runtime dynamically, and Windows does not ship it: without these
  DLLs beside the exe, Peeby does not start on a machine without the VC++ redistributable.
  Shipping them app-local keeps "nothing installed on the host" (ADR 0027). Found with vswhere,
  from the newest Visual Studio (or Build Tools) with the C++ workload.
#>

$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $PSScriptRoot
$Dest = Join-Path $Root 'third_party\vcruntime'
$Dlls = @('vcruntime140.dll', 'vcruntime140_1.dll')

$VsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $VsWhere)) { throw "vswhere not found: install Visual Studio Build Tools" }
$Vs = & $VsWhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $Vs) { throw "no Visual Studio with the C++ tools found" }

$Crt = Get-ChildItem -Path (Join-Path $Vs 'VC\Redist\MSVC') -Directory |
  Where-Object { $_.Name -match '^\d+\.' } |
  Sort-Object { [version]$_.Name } -Descending |
  ForEach-Object { Get-ChildItem -Path (Join-Path $_.FullName 'x64') -Directory -Filter 'Microsoft.VC*.CRT' -ErrorAction SilentlyContinue } |
  Select-Object -First 1
if (-not $Crt) { throw "no x64 CRT redistributable under $Vs\VC\Redist\MSVC" }

New-Item -ItemType Directory -Force $Dest | Out-Null
foreach ($Dll in $Dlls) {
  Copy-Item -Path (Join-Path $Crt.FullName $Dll) -Destination $Dest -Force
}
Write-Host "VC++ runtime from $($Crt.FullName) copied to $Dest"
