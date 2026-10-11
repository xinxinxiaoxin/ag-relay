# Generates ag-relay app icons: an "AG" monogram on a rounded square.
# Outputs into src-tauri/icons/: icon.ico, icon.png, 32x32.png, 128x128.png, 128x128@2x.png
# Usage: powershell -ExecutionPolicy Bypass -File scripts/make-icons.ps1
# ASCII only on purpose (PowerShell 5.1 reads BOM-less files as ANSI).

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$iconsDir = Join-Path $PSScriptRoot "..\src-tauri\icons"
New-Item -ItemType Directory -Force -Path $iconsDir | Out-Null

function New-BaseBitmap([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap($size, $size)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality

    $g.Clear([System.Drawing.Color]::Transparent)

    # rounded-rect background (10% inset)
    $inset = [Math]::Max(1, [int]($size * 0.04))
    $w = $size - 2 * $inset
    $rect = New-Object System.Drawing.Rectangle($inset, $inset, $w, $w)
    $radius = [int]($w * 0.22)
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = 2 * $radius
    $path.AddArc($rect.X, $rect.Y, $d, $d, 180, 90)
    $path.AddArc($rect.Right - $d, $rect.Y, $d, $d, 270, 90)
    $path.AddArc($rect.Right - $d, $rect.Bottom - $d, $d, $d, 0, 90)
    $path.AddArc($rect.X, $rect.Bottom - $d, $d, $d, 90, 90)
    $path.CloseFigure()

    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
        $rect,
        [System.Drawing.Color]::FromArgb(255, 30, 41, 59),
        [System.Drawing.Color]::FromArgb(255, 12, 74, 110),
        55.0)
    $g.FillPath($brush, $path)

    # "AG" monogram, centered
    $fontSize = [float]($size * 0.46)
    $font = New-Object System.Drawing.Font("Segoe UI", $fontSize,
        [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
    $fmt = New-Object System.Drawing.StringFormat
    $fmt.Alignment = [System.Drawing.StringAlignment]::Center
    $fmt.LineAlignment = [System.Drawing.StringAlignment]::Center
    $textRect = New-Object System.Drawing.RectangleF($rect.X, ($rect.Y - $size * 0.01), $rect.Width, $rect.Height)
    $g.DrawString("AG", $font, [System.Drawing.Brushes]::White, $textRect, $fmt)

    $g.Dispose()
    return $bmp
}

function Save-Resized($baseBmp, [int]$size, [string]$path) {
    $bmp = New-Object System.Drawing.Bitmap($size, $size)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.DrawImage($baseBmp, (New-Object System.Drawing.Rectangle(0, 0, $size, $size)))
    $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Output "wrote $path ($size px)"
}

$base = New-BaseBitmap 256

$png256 = Join-Path $iconsDir "icon.png"
$base.Save($png256, [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output "wrote $png256 (256 px)"
Save-Resized $base 256 (Join-Path $iconsDir "128x128@2x.png")
Save-Resized $base 128 (Join-Path $iconsDir "128x128.png")
Save-Resized $base 32 (Join-Path $iconsDir "32x32.png")

# --- pack the 256px PNG into a single-entry .ico (PNG-compressed entry, Vista+) ---
$icoPath = Join-Path $iconsDir "icon.ico"
$pngBytes = [System.IO.File]::ReadAllBytes($png256)
$ms = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($ms)
$bw.Write([UInt16]0)      # reserved
$bw.Write([UInt16]1)      # type: icon
$bw.Write([UInt16]1)      # image count
$bw.Write([Byte]0)        # width 256 -> 0
$bw.Write([Byte]0)        # height 256 -> 0
$bw.Write([Byte]0)        # palette
$bw.Write([Byte]0)        # reserved
$bw.Write([UInt16]1)      # color planes
$bw.Write([UInt16]32)     # bits per pixel
$bw.Write([UInt32]$pngBytes.Length)
$bw.Write([UInt32]22)     # data offset: 6-byte header + 16-byte entry
$bw.Write($pngBytes)
$bw.Flush()
[System.IO.File]::WriteAllBytes($icoPath, $ms.ToArray())
$bw.Dispose(); $ms.Dispose()
Write-Output "wrote $icoPath ($($pngBytes.Length) byte png payload)"

$base.Dispose()
Write-Output "ICONS-DONE"
