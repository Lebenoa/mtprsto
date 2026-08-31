# Generates a real 800x800 JPEG used by the sweep's upload_profile_photo
# test (a fake payload is rejected with PHOTO_CROP_SIZE_SMALL).
Add-Type -AssemblyName System.Drawing
$out = Join-Path $env:TEMP 'mtprsto_sweep_avatar.jpg'
$bmp = New-Object System.Drawing.Bitmap 800, 800
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.Clear([System.Drawing.Color]::FromArgb(90, 160, 240))
$font = New-Object System.Drawing.Font('Arial', 48)
$g.DrawString('mtprsto', $font, [System.Drawing.Brushes]::White, 220, 360)
$g.Dispose()
$codec = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() |
    Where-Object { $_.MimeType -eq 'image/jpeg' }
$parms = New-Object System.Drawing.Imaging.EncoderParameters 1
$parms.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter([System.Drawing.Imaging.Encoder]::Quality, 85L)
$bmp.Save($out, $codec, $parms)
$bmp.Dispose()
"saved $((Get-Item $out).Length) bytes to $out"
