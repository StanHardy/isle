param(
  [Parameter(Mandatory = $true)][string]$Action,
  [int]$X,
  [int]$Y,
  [int]$Clicks = 1,
  [int]$Delta = -120
)
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class MouseHelper {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, int dwData, UIntPtr dwExtraInfo);
  public const uint MOVE = 0x0001;
  public const uint LEFTDOWN = 0x0002;
  public const uint LEFTUP = 0x0004;
  public const uint WHEEL = 0x0800;
}
'@
switch ($Action) {
  "move" {
    [void][MouseHelper]::SetCursorPos($X, $Y)
    Write-Output "moved to $X,$Y"
  }
  "click" {
    [void][MouseHelper]::SetCursorPos($X, $Y)
    Start-Sleep -Milliseconds 120
    for ($i = 0; $i -lt $Clicks; $i++) {
      [MouseHelper]::mouse_event([MouseHelper]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
      [MouseHelper]::mouse_event([MouseHelper]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
      Start-Sleep -Milliseconds 80
    }
    Write-Output "clicked $X,$Y x$Clicks"
  }
  "scroll" {
    [void][MouseHelper]::SetCursorPos($X, $Y)
    Start-Sleep -Milliseconds 100
    [MouseHelper]::mouse_event([MouseHelper]::WHEEL, 0, 0, $Delta, [UIntPtr]::Zero)
    Write-Output "scrolled $Delta at $X,$Y"
  }
  default { Write-Output "unknown action" }
}
