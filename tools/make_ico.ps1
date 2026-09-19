Add-Type -AssemblyName System.Drawing

function Convert-PngToIco {
    param(
        [Parameter(Mandatory=$true)][string]$PngPath,
        [Parameter(Mandatory=$true)][string]$IcoPath
    )

    $src = [System.Drawing.Bitmap]::FromFile((Resolve-Path $PngPath).Path)
    $sizes = @(16, 32, 48, 64, 128, 256)

    $msList = @()
    foreach ($sz in $sizes) {
        $bmp = New-Object System.Drawing.Bitmap $sz, $sz
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
        $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $g.DrawImage($src, 0, 0, $sz, $sz)
        $g.Dispose()

        $ms = New-Object System.IO.MemoryStream
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        $msList += $ms
    }
    $src.Dispose()

    $targetFile = [System.IO.Path]::GetFullPath($IcoPath)
    $fs = [System.IO.File]::Create($targetFile)
    $bw = New-Object System.IO.BinaryWriter $fs

    # ICONDIR
    $bw.Write([uint16]0) # reserved
    $bw.Write([uint16]1) # type 1 = icon
    $bw.Write([uint16]$sizes.Count)

    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $sz = $sizes[$i]
        $w = if ($sz -ge 256) { [byte]0 } else { [byte]$sz }
        $h = if ($sz -ge 256) { [byte]0 } else { [byte]$sz }
        $bw.Write($w)
        $bw.Write($h)
        $bw.Write([byte]0) # colors
        $bw.Write([byte]0) # reserved
        $bw.Write([uint16]1) # planes
        $bw.Write([uint16]32) # bpp
        $len = [uint32]$msList[$i].Length
        $bw.Write($len)
        $bw.Write([uint32]$offset)
        $offset += $len
    }

    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $bytes = $msList[$i].ToArray()
        $bw.Write($bytes)
        $msList[$i].Dispose()
    }

    $bw.Dispose()
    $fs.Dispose()
    Write-Host "Generated $IcoPath successfully."
}

Convert-PngToIco -PngPath "target\send.png" -IcoPath "crates\sendmecongo-send\app.ico"
Convert-PngToIco -PngPath "target\recv.png" -IcoPath "crates\sendmecongo-recv\app.ico"
