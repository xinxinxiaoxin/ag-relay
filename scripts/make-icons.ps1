param(
    [string]$Source = (Join-Path $PSScriptRoot "..\src\assets\brand-original.jpg")
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$iconsDir = Join-Path $PSScriptRoot "..\src-tauri\icons"
$assetsDir = Join-Path $PSScriptRoot "..\src\assets"
$sourceImage = [System.Drawing.Image]::FromFile((Resolve-Path $Source))

function New-BaseBitmap([bool]$small) {
    $size = 1024
    $image = New-Object System.Drawing.Bitmap($size, $size)
    $g = [System.Drawing.Graphics]::FromImage($image)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $crop = if ($small) {
        New-Object System.Drawing.RectangleF(($sourceImage.Width * 0.195), ($sourceImage.Height * 0.035), ($sourceImage.Width * 0.64), ($sourceImage.Height * 0.64))
    } else {
        New-Object System.Drawing.RectangleF(0, 0, $sourceImage.Width, $sourceImage.Height)
    }
    $g.DrawImage($sourceImage, (New-Object System.Drawing.RectangleF(0, 0, $size, $size)), $crop, [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose()

    $bmp = New-Object System.Drawing.Bitmap($size, $size)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = 384
    $path.AddArc(0, 0, $d, $d, 180, 90)
    $path.AddArc(($size - $d), 0, $d, $d, 270, 90)
    $path.AddArc(($size - $d), ($size - $d), $d, $d, 0, 90)
    $path.AddArc(0, ($size - $d), $d, $d, 90, 90)
    $path.CloseFigure()
    $brush = New-Object System.Drawing.TextureBrush($image)
    $g.FillPath($brush, $path)
    $brush.Dispose()
    $path.Dispose()
    $g.Dispose()
    $image.Dispose()
    return $bmp
}

function Get-Resized($base, [int]$size) {
    $bmp = New-Object System.Drawing.Bitmap($size, $size)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.DrawImage($base, (New-Object System.Drawing.Rectangle(0, 0, $size, $size)))
    $g.Dispose()
    return $bmp
}

function Save-Resized($base, [int]$size, [string]$path) {
    $bmp = Get-Resized $base $size
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Output "wrote $path ($size px)"
}

function New-TrayBitmap {
    $bmp = New-Object System.Drawing.Bitmap(1024, 1024)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $g.Clear([System.Drawing.Color]::Transparent)
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = 256
    $path.AddArc(0, 0, $d, $d, 180, 90)
    $path.AddArc(768, 0, $d, $d, 270, 90)
    $path.AddArc(768, 768, $d, $d, 0, 90)
    $path.AddArc(0, 768, $d, $d, 90, 90)
    $path.CloseFigure()
    $g.FillPath([System.Drawing.Brushes]::Black, $path)
    $font = New-Object System.Drawing.Font("Segoe UI", 560, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
    $format = New-Object System.Drawing.StringFormat
    $format.FormatFlags = [System.Drawing.StringFormatFlags]::NoWrap
    $format.Alignment = [System.Drawing.StringAlignment]::Center
    $format.LineAlignment = [System.Drawing.StringAlignment]::Center
    $g.DrawString("AG", $font, [System.Drawing.Brushes]::White, (New-Object System.Drawing.RectangleF(0, -15, 1024, 1024)), $format)
    $format.Dispose()
    $font.Dispose()
    $path.Dispose()
    $g.Dispose()
    return $bmp
}

$full = New-BaseBitmap $false
$small = New-BaseBitmap $true
$tray = New-TrayBitmap
try {
    Save-Resized $full 256 (Join-Path $iconsDir "icon.png")
    Save-Resized $full 256 (Join-Path $iconsDir "128x128@2x.png")
    Save-Resized $full 128 (Join-Path $iconsDir "128x128.png")
    Save-Resized $small 32 (Join-Path $iconsDir "32x32.png")
    Save-Resized $tray 32 (Join-Path $iconsDir "tray.png")
    Save-Resized $full 256 (Join-Path $assetsDir "brand.png")
    Save-Resized $small 128 (Join-Path $assetsDir "brand-small.png")

    $sizes = @(16, 20, 24, 32, 48, 64, 96, 128, 256)
    $entries = foreach ($size in $sizes) {
        $base = if ($size -le 32) { $small } else { $full }
        $bmp = Get-Resized $base $size
        $stream = New-Object System.IO.MemoryStream
        $bmp.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
        [PSCustomObject]@{ Size = $size; Bytes = $stream.ToArray() }
        $stream.Dispose()
        $bmp.Dispose()
    }
    $ico = New-Object System.IO.MemoryStream
    $writer = New-Object System.IO.BinaryWriter($ico)
    $writer.Write([UInt16]0)
    $writer.Write([UInt16]1)
    $writer.Write([UInt16]$entries.Count)
    $offset = 6 + 16 * $entries.Count
    foreach ($entry in $entries) {
        $dimension = if ($entry.Size -eq 256) { 0 } else { $entry.Size }
        $writer.Write([Byte]$dimension)
        $writer.Write([Byte]$dimension)
        $writer.Write([Byte]0)
        $writer.Write([Byte]0)
        $writer.Write([UInt16]1)
        $writer.Write([UInt16]32)
        $writer.Write([UInt32]$entry.Bytes.Length)
        $writer.Write([UInt32]$offset)
        $offset += $entry.Bytes.Length
    }
    foreach ($entry in $entries) { $writer.Write([byte[]]$entry.Bytes) }
    $writer.Flush()
    [System.IO.File]::WriteAllBytes((Join-Path $iconsDir "icon.ico"), $ico.ToArray())
    $writer.Dispose()
    $ico.Dispose()
    Write-Output "ICONS-DONE"
} finally {
    $full.Dispose()
    $small.Dispose()
    $tray.Dispose()
    $sourceImage.Dispose()
}
