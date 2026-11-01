<#
.SYNOPSIS
  Downloads the pinned yt-dlp and JavaScript runtimes into third_party/yt-dlp/, so nothing has to
  be installed on the host.

.DESCRIPTION
  Every archive is checked against its pinned SHA-256 before being extracted. Layout:
    third_party/yt-dlp/yt-dlp.exe          yt-dlp single-file build: it updates itself (`yt-dlp -U`),
                                           which the "onedir" build cannot do
    third_party/yt-dlp/deno.exe            Deno, the JS runtime yt-dlp needs for YouTube (~110 MB)
    third_party/yt-dlp/VERSION             pins of this script, to skip a re-download
  QuickJS-NG (2 MB) was measured and rejected: 5.6 s per JS challenge against 0.8 s for Deno.
  The release bundle ships the same layout under tools/ next to the executable.

  To bump a pin: take the SHA-256 from the release (yt-dlp: SHA2-256SUMS; Deno:
  the asset "digest" of the GitHub releases API) and update $Pins below.

.PARAMETER Force
  Download again even if third_party/yt-dlp/ already matches the pins.
#>
param(
  [switch]$Force
)

$ErrorActionPreference = 'Stop'

$Pins = @(
  @{
    Name    = 'yt-dlp 2026.10.26'
    Url     = 'https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp.exe'
    Sha256  = 'f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d'
    Rename  = 'yt-dlp.exe'
  },
  @{
    Name    = 'deno 2.9.8'
    Url     = 'https://github.com/denoland/deno/releases/download/v2.9.8/deno-x86_64-pc-windows-msvc.zip'
    Sha256  = 'cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf'
    Extract = '.'
  }
)

$Root    = Split-Path -Parent $PSScriptRoot
$Dest    = Join-Path $Root 'third_party\yt-dlp'
$Stamp   = Join-Path $Dest 'VERSION'
$Version = ($Pins | ForEach-Object { $_.Name }) -join ', '

if (-not $Force -and (Test-Path $Stamp) -and ((Get-Content $Stamp -Raw).Trim() -eq $Version)) {
  Write-Host "$Version already present in $Dest"
  exit 0
}

$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) "peebtube-tools-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
  New-Item -ItemType Directory -Path $Dest | Out-Null

  foreach ($Pin in $Pins) {
    $File = Join-Path $Tmp ([System.IO.Path]::GetFileName($Pin.Url))
    Write-Host "Downloading $($Pin.Name): $($Pin.Url)"
    Invoke-WebRequest -Uri $Pin.Url -OutFile $File -UseBasicParsing

    $Actual = (Get-FileHash -Algorithm SHA256 $File).Hash.ToLowerInvariant()
    if ($Actual -ne $Pin.Sha256) {
      throw "$($Pin.Name): SHA-256 mismatch, expected $($Pin.Sha256), got $Actual"
    }

    if ($Pin.Extract) {
      Expand-Archive -Path $File -DestinationPath (Join-Path $Dest $Pin.Extract) -Force
    } else {
      Copy-Item $File (Join-Path $Dest $Pin.Rename)
    }
  }

  $YtDlp = Join-Path $Dest 'yt-dlp.exe'
  if (-not (Test-Path $YtDlp)) { throw "yt-dlp.exe not found in the archive (expected $YtDlp)" }

  Set-Content -Path $Stamp -Value $Version -NoNewline
  Write-Host "$Version extracted to $Dest"
}
finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
