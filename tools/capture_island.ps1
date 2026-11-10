param(
  [string]$OutFile = "$env:TEMP\isle-capture.png",
  [int]$Left = 1516,
  [int]$Top = 0,
  [int]$Width = 500,
  [int]$Height = 110
)
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$b = New-Object System.Drawing.Bitmap $Width, $Height
$g = [System.Drawing.Graphics]::FromImage($b)
$g.CopyFromScreen($Left, $Top, 0, 0, $b.Size)
$b.Save($OutFile, [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output "saved $OutFile"
