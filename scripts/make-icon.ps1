# Generates the source application icon (1024x1024 PNG) used by `tauri icon`.
#
# Requires .NET System.Drawing (available on Windows PowerShell).
# Run:  powershell -ExecutionPolicy Bypass -File scripts/make-icon.ps1

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$size = 1024
$out = Join-Path $PSScriptRoot "..\src-tauri\icons\app-icon.png"
$outDir = Split-Path $out -Parent
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$bmp = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g.Clear([System.Drawing.Color]::Transparent)

# --- Rounded background with a diagonal gradient -------------------------
$radius = 224
$path = New-Object System.Drawing.Drawing2D.GraphicsPath
$d = $radius * 2
$path.AddArc(0, 0, $d, $d, 180, 90)
$path.AddArc($size - $d, 0, $d, $d, 270, 90)
$path.AddArc($size - $d, $size - $d, $d, $d, 0, 90)
$path.AddArc(0, $size - $d, $d, $d, 90, 90)
$path.CloseFigure()

$rect = New-Object System.Drawing.Rectangle(0, 0, $size, $size)
$brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    $rect,
    [System.Drawing.Color]::FromArgb(255, 23, 43, 130),
    [System.Drawing.Color]::FromArgb(255, 88, 140, 255),
    [System.Drawing.Drawing2D.LinearGradientMode]::ForwardDiagonal)
$g.FillPath($brush, $path)

# --- Soft glow in the upper right ----------------------------------------
$glow = New-Object System.Drawing.Drawing2D.GraphicsPath
$glow.AddEllipse(430, -60, 720, 720)
$gb = New-Object System.Drawing.Drawing2D.PathGradientBrush($glow)
$gb.CenterColor = [System.Drawing.Color]::FromArgb(70, 255, 255, 255)
$gb.SurroundColors = @([System.Drawing.Color]::FromArgb(0, 255, 255, 255))
$g.FillPath($gb, $glow)

# --- Open book ------------------------------------------------------------
$white = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(250, 255, 255, 255))
$pageLeft = New-Object System.Drawing.Drawing2D.GraphicsPath
$pageLeft.AddBezier(196, 330, 300, 300, 400, 330, 502, 386)
$pageLeft.AddLine(502, 386, 502, 744)
$pageLeft.AddBezier(502, 744, 400, 690, 300, 662, 196, 690)
$pageLeft.CloseFigure()

$pageRight = New-Object System.Drawing.Drawing2D.GraphicsPath
$pageRight.AddBezier(828, 330, 724, 300, 624, 330, 522, 386)
$pageRight.AddLine(522, 386, 522, 744)
$pageRight.AddBezier(522, 744, 624, 690, 724, 662, 828, 690)
$pageRight.CloseFigure()

$g.FillPath($white, $pageLeft)
$g.FillPath($white, $pageRight)

# Spine shadow
$spine = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(36, 20, 40, 90))
$g.FillRectangle($spine, 494, 372, 36, 380)

# --- Text lines on the pages ---------------------------------------------
$line = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(120, 40, 70, 160))
$g.FillRectangle($line, 262, 452, 178, 26)
$g.FillRectangle($line, 262, 520, 178, 26)
$g.FillRectangle($line, 262, 588, 118, 26)
$g.FillRectangle($line, 584, 452, 178, 26)
$g.FillRectangle($line, 584, 520, 178, 26)
$g.FillRectangle($line, 584, 588, 118, 26)

# --- AI spark (four-point star) ------------------------------------------
$spark = New-Object System.Drawing.Drawing2D.GraphicsPath
$cx = 742; $cy = 300; $r1 = 108; $r2 = 34
$spark.AddPolygon(@(
    (New-Object System.Drawing.PointF($cx, ($cy - $r1))),
    (New-Object System.Drawing.PointF(($cx + $r2), ($cy - $r2))),
    (New-Object System.Drawing.PointF(($cx + $r1), $cy)),
    (New-Object System.Drawing.PointF(($cx + $r2), ($cy + $r2))),
    (New-Object System.Drawing.PointF($cx, ($cy + $r1))),
    (New-Object System.Drawing.PointF(($cx - $r2), ($cy + $r2))),
    (New-Object System.Drawing.PointF(($cx - $r1), $cy)),
    (New-Object System.Drawing.PointF(($cx - $r2), ($cy - $r2)))
))
$sparkBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 255, 214, 102))
$g.FillPath($sparkBrush, $spark)

$small = New-Object System.Drawing.Drawing2D.GraphicsPath
$cx2 = 852; $cy2 = 156; $r3 = 56; $r4 = 18
$small.AddPolygon(@(
    (New-Object System.Drawing.PointF($cx2, ($cy2 - $r3))),
    (New-Object System.Drawing.PointF(($cx2 + $r4), ($cy2 - $r4))),
    (New-Object System.Drawing.PointF(($cx2 + $r3), $cy2)),
    (New-Object System.Drawing.PointF(($cx2 + $r4), ($cy2 + $r4))),
    (New-Object System.Drawing.PointF($cx2, ($cy2 + $r3))),
    (New-Object System.Drawing.PointF(($cx2 - $r4), ($cy2 + $r4))),
    (New-Object System.Drawing.PointF(($cx2 - $r3), $cy2)),
    (New-Object System.Drawing.PointF(($cx2 - $r4), ($cy2 - $r4)))
))
$g.FillPath($sparkBrush, $small)

$g.Dispose()
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

Write-Output "wrote $out ($((Get-Item $out).Length) bytes)"
