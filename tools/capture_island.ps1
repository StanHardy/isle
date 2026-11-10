param([string]$OutFile = "$env:TEMP\isle-capture.png")
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$b = New-Object System.Drawing.Bitmap 500, 110
$g = [System.Drawing.Graphics]::FromImage($b)
$g.CopyFromScreen(1516, 0, 0, 0, $b.Size)
$b.Save($OutFile, [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output "saved $OutFile"
