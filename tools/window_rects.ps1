Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WE8 {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool EnumWindows(WE8Proc cb, IntPtr lp);
  public delegate bool WE8Proc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
}
'@
$lines = New-Object System.Collections.Generic.List[string]
$cb = [WE8+WE8Proc]{ param($h, $l)
  $sb = New-Object System.Text.StringBuilder 64
  [void][WE8]::GetWindowText($h, $sb, 64)
  $pid2 = 0
  [void][WE8]::GetWindowThreadProcessId($h, [ref]$pid2)
  $proc = (Get-Process -Id $pid2 -ErrorAction SilentlyContinue).ProcessName
  if ($proc -eq 'isle') {
    $r = New-Object WE8+RECT
    [void][WE8]::GetWindowRect($h, [ref]$r)
    $lines.Add("proc=isle rect=$($r.L),$($r.T) $(($r.R - $r.L))x$(($r.B - $r.T)) title=[$($sb.ToString())]")
  }
  return $true
}
[void][WE8]::EnumWindows($cb, [IntPtr]::Zero)
$lines | ForEach-Object { Write-Output $_ }
