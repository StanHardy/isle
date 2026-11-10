Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase
$pt = New-Object System.Windows.Point(1906, 14)
$el = [System.Windows.Automation.AutomationElement]::FromPoint($pt)
if (-not $el) { Write-Output "no element at point"; exit }
Write-Output ("hit: name=[" + $el.Current.Name + "] class=[" + $el.Current.ClassName + "] type=" + $el.Current.ControlType.ProgrammaticName + " framework=" + $el.Current.FrameworkId)
# Walk up to the top-level ancestor and report its native handle.
$walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
$cur = $el
$depth = 0
while ($cur -and $depth -lt 12) {
    $hwnd = $cur.Current.NativeWindowHandle
    $name = $cur.Current.Name
    $cls = $cur.Current.ClassName
    if ($hwnd -and $hwnd -ne 0) {
        $procId = 0
        $sig = '
using System;
using System.Runtime.InteropServices;
public class GTP { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid); }
'
        Add-Type -TypeDefinition $sig
        $procId = 0
        [void][GTP]::GetWindowThreadProcessId([IntPtr]$hwnd, [ref]$procId)
        $proc = (Get-Process -Id $procId -ErrorAction SilentlyContinue).ProcessName
        Write-Output ("ancestor depth=$depth hwnd=$hwnd class=[$cls] name=[$name] proc=$proc")
        break
    }
    $cur = $walker.GetParent($cur)
    $depth++
}
