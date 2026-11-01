<#
.SYNOPSIS
  Downloads the pinned libmpv dev build (shinchiro/mpv-winbuild-cmake) into third_party/mpv/.

.DESCRIPTION
  Verifies the archive SHA-256 before extracting. shinchiro only keeps recent releases on GitHub,
  so when the pinned release disappears: pick a new one, update $Version/$Url/$Sha256 below,
  and record the bump in docs/build-windows.md.

.PARAMETER Archive
  Use an already downloaded archive instead of downloading it (hash is still verified).

.PARAMETER Force
  Re-extract even if third_party/mpv/ already matches the pinned version.
#>
param(
  [string]$Archive,
  [switch]$Force
)

$ErrorActionPreference = 'Stop'

$Version = '20261021-git-e471496c56'
$Url     = "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20261021/mpv-dev-x86_64-$Version.7z"
$Sha256  = 'efc719eff3dc5954b4a5efc800632808bcec01cc9034940dd14e5eb7fe9ea633'

$Root    = Split-Path -Parent $PSScriptRoot
$Dest    = Join-Path $Root 'third_party\mpv'
$Stamp   = Join-Path $Dest 'VERSION'

if (-not $Force -and (Test-Path $Stamp) -and ((Get-Content $Stamp -Raw).Trim() -eq $Version)) {
  Write-Host "libmpv $Version already present in $Dest"
  exit 0
}

$SevenZip = @(
  (Get-Command 7z -ErrorAction SilentlyContinue).Source,
  "$env:ProgramFiles\7-Zip\7z.exe",
  "${env:ProgramFiles(x86)}\7-Zip\7z.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $SevenZip) { throw '7-Zip not found (install it or add 7z to PATH).' }

$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) "peeby-mpv-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  if (-not $Archive) {
    $Archive = Join-Path $Tmp 'mpv-dev.7z'
    Write-Host "Downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $Archive -UseBasicParsing
  }

  $Actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
  if ($Actual -ne $Sha256) { throw "SHA-256 mismatch: expected $Sha256, got $Actual" }

  if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
  New-Item -ItemType Directory -Path $Dest | Out-Null
  & $SevenZip x $Archive "-o$Dest" -y | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "7-Zip failed with exit code $LASTEXITCODE" }

  Set-Content -Path $Stamp -Value $Version -NoNewline
  Write-Host "libmpv $Version extracted to $Dest"
}
finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
