<#
.SYNOPSIS
  Renders the Peeby logo (docs/logo/onglets.html) into crates/ui/assets/logo.png and
  crates/peeby/assets/peeby.ico.

.DESCRIPTION
  Microsoft Edge (headless, transparent background) screenshots the page at 1024 px; every size is
  then scaled down from it with GDI+, halving first so the small ones keep their detail. The icon
  holds the same nine sizes as before (16 to 256), each as a PNG. See docs/logo/README.md.
#>
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$Root = Split-Path -Parent $PSScriptRoot
$Page = Join-Path $Root 'docs\logo\onglets.html'
$Png  = Join-Path $Root 'crates\ui\assets\logo.png'
$Ico  = Join-Path $Root 'crates\peeby\assets\peeby.ico'
$IcoSizes = 16, 20, 24, 32, 40, 48, 64, 128, 256

$Edge = @(
  "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
  "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $Edge) { throw 'Microsoft Edge not found' }

$Work = Join-Path ([IO.Path]::GetTempPath()) "peeby-logo-$PID"
New-Item -ItemType Directory -Force $Work | Out-Null
try {
  $Shot = Join-Path $Work 'logo-1024.png'
  $Url = ([Uri](Resolve-Path $Page).Path).AbsoluteUri
  Start-Process -FilePath $Edge -Wait -NoNewWindow -ArgumentList @(
    '--headless=new', '--disable-gpu', '--hide-scrollbars', '--allow-file-access-from-files',
    '--force-device-scale-factor=1', '--default-background-color=00000000',
    "--user-data-dir=$Work\profile", '--virtual-time-budget=3000',
    '--window-size=1024,1024', "--screenshot=$Shot", $Url
  )
  if (-not (Test-Path $Shot)) { throw 'Edge did not save the screenshot' }

  $Source = [System.Drawing.Bitmap]::new($Shot)
  # The drawing must fit with a margin: the border pixels are transparent.
  foreach ($i in 0..1023) {
    foreach ($p in @(@($i, 0), @($i, 1023), @(0, $i), @(1023, $i))) {
      if ($Source.GetPixel($p[0], $p[1]).A -ne 0) { throw "Not transparent at $($p -join ','): the drawing is clipped" }
    }
  }

  # $Source at $Size px: halved with bicubic filtering until the last step.
  function Scale([System.Drawing.Bitmap]$Image, [int]$Size) {
    $current = $Image
    while ($current.Width -gt 2 * $Size) {
      $current = Resize $current ([int]($current.Width / 2))
    }
    return Resize $current $Size
  }
  function Resize([System.Drawing.Bitmap]$Image, [int]$Size) {
    $out = [System.Drawing.Bitmap]::new($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($out)
    $g.InterpolationMode = 'HighQualityBicubic'
    $g.PixelOffsetMode = 'HighQuality'
    $g.CompositingMode = 'SourceCopy'
    $attributes = [System.Drawing.Imaging.ImageAttributes]::new()
    $attributes.SetWrapMode('TileFlipXY')
    $g.DrawImage($Image, [System.Drawing.Rectangle]::new(0, 0, $Size, $Size), 0, 0, $Image.Width, $Image.Height, 'Pixel', $attributes)
    $g.Dispose()
    return $out
  }
  function PngBytes([System.Drawing.Bitmap]$Image) {
    $stream = [IO.MemoryStream]::new()
    $Image.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    return , $stream.ToArray()
  }

  [IO.File]::WriteAllBytes($Png, (PngBytes (Scale $Source 256)))

  # ICO: header, one 16-byte entry per size, then the PNGs.
  $images = foreach ($size in $IcoSizes) { , (PngBytes (Scale $Source $size)) }
  $file = [IO.MemoryStream]::new()
  $w = [IO.BinaryWriter]::new($file)
  $w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$IcoSizes.Count)
  $offset = 6 + 16 * $IcoSizes.Count
  for ($i = 0; $i -lt $IcoSizes.Count; $i++) {
    $side = if ($IcoSizes[$i] -ge 256) { 0 } else { $IcoSizes[$i] }
    $w.Write([byte]$side); $w.Write([byte]$side); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32)
    $w.Write([uint32]$images[$i].Length); $w.Write([uint32]$offset)
    $offset += $images[$i].Length
  }
  foreach ($bytes in $images) { $w.Write($bytes) }
  $w.Flush()
  [IO.File]::WriteAllBytes($Ico, $file.ToArray())
  $Source.Dispose()
  Write-Host "Wrote $Png and $Ico"
} finally {
  Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
