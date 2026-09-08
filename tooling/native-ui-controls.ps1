param(
    [Parameter(Mandatory)][int]$TargetProcessId,
    [ValidateSet('state','restore','drag','double-click','resize','snap-left')][string]$Action = 'state'
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$target = Get-Process -Id $TargetProcessId
if ($target.Path -ne (Join-Path $repo 'target/debug/studio-desktop.exe')) { throw 'Target is not this workspace desktop application.' }
$window = $target.MainWindowHandle
if ($window -eq 0) { throw 'Desktop window is unavailable.' }
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class DatasetUiControl {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; public Point(int x,int y){X=x;Y=y;} }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int n);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a,uint b,bool attach);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h,uint flags);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint dx,uint dy,uint data,UIntPtr extra);
  [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int n);
}
'@
function Read-State {
    $rect = New-Object DatasetUiControl+Rect
    $client = New-Object DatasetUiControl+Rect
    [void][DatasetUiControl]::GetWindowRect($window,[ref]$rect)
    [void][DatasetUiControl]::GetClientRect($window,[ref]$client)
    [pscustomobject]@{ Left=$rect.Left;Top=$rect.Top;Width=$rect.Right-$rect.Left;Height=$rect.Bottom-$rect.Top;ClientWidth=$client.Right;ClientHeight=$client.Bottom;Maximized=[DatasetUiControl]::IsZoomed($window);Minimized=[DatasetUiControl]::IsIconic($window);Foreground=[DatasetUiControl]::GetForegroundWindow() -eq $window;ScreenWidth=[DatasetUiControl]::GetSystemMetrics(0);ScreenHeight=[DatasetUiControl]::GetSystemMetrics(1) }
}
function Focus-Target {
    [void][DatasetUiControl]::SetForegroundWindow($window)
    Start-Sleep -Milliseconds 100
    if ([DatasetUiControl]::GetForegroundWindow() -ne $window) {
        $foregroundProcess = [uint32]0
        $foregroundThread = [DatasetUiControl]::GetWindowThreadProcessId([DatasetUiControl]::GetForegroundWindow(),[ref]$foregroundProcess)
        $currentThread = [DatasetUiControl]::GetCurrentThreadId()
        $attached = [DatasetUiControl]::AttachThreadInput($currentThread,$foregroundThread,$true)
        try { [void][DatasetUiControl]::BringWindowToTop($window); [void][DatasetUiControl]::SetForegroundWindow($window) }
        finally { if($attached){[void][DatasetUiControl]::AttachThreadInput($currentThread,$foregroundThread,$false)} }
        Start-Sleep -Milliseconds 100
    }
    if ([DatasetUiControl]::GetForegroundWindow() -ne $window) { throw 'Target did not receive focus; no keyboard or mouse input was sent.' }
}
function Assert-Point([DatasetUiControl+Point]$point) {
    if ([DatasetUiControl]::GetAncestor([DatasetUiControl]::WindowFromPoint($point),2) -ne $window) { throw 'The input point is covered by another window; no mouse input was sent.' }
}
function Click-Point([DatasetUiControl+Point]$point) {
    Assert-Point $point
    [void][DatasetUiControl]::SetCursorPos($point.X,$point.Y)
    [DatasetUiControl]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    [DatasetUiControl]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
}
$before = Read-State
if ($Action -eq 'restore') { [void][DatasetUiControl]::ShowWindow($window,9) }
elseif ($Action -eq 'drag' -or $Action -eq 'double-click') {
    if ($before.Maximized -or $before.Minimized) { throw 'Restore the target before this test.' }
    Focus-Target
    $point = [DatasetUiControl+Point]::new(800,18)
    [void][DatasetUiControl]::ClientToScreen($window,[ref]$point)
    Assert-Point $point
    if ($Action -eq 'double-click') { Click-Point $point; Start-Sleep -Milliseconds 65; Click-Point $point }
    else {
        [void][DatasetUiControl]::SetCursorPos($point.X,$point.Y)
        [DatasetUiControl]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
        Start-Sleep -Milliseconds 120
        [void][DatasetUiControl]::SetCursorPos($point.X+70,$point.Y+50)
        Start-Sleep -Milliseconds 120
        [DatasetUiControl]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    }
}
elseif ($Action -eq 'resize') {
    if ($before.Maximized -or $before.Minimized) { throw 'Restore the target before resizing.' }
    Focus-Target
    $point = [DatasetUiControl+Point]::new($before.Left+$before.Width-2,$before.Top+$before.Height-2)
    Assert-Point $point
    [void][DatasetUiControl]::SetCursorPos($point.X,$point.Y)
    [DatasetUiControl]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 120
    [void][DatasetUiControl]::SetCursorPos($point.X+64,$point.Y+40)
    Start-Sleep -Milliseconds 120
    [DatasetUiControl]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
}
elseif ($Action -eq 'snap-left') {
    Focus-Target
    [DatasetUiControl]::keybd_event(0x5B,0,0,[UIntPtr]::Zero)
    [DatasetUiControl]::keybd_event(0x25,0,0,[UIntPtr]::Zero)
    [DatasetUiControl]::keybd_event(0x25,0,2,[UIntPtr]::Zero)
    [DatasetUiControl]::keybd_event(0x5B,0,2,[UIntPtr]::Zero)
}
if ($Action -ne 'state') { Start-Sleep -Milliseconds 350 }
[pscustomobject]@{ Action=$Action;Before=$before;After=(Read-State) } | ConvertTo-Json -Depth 4 -Compress
