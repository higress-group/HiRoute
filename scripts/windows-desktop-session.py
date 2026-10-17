#!/usr/bin/env python3
"""Bounded native Desktop acceptance in an existing validation session.

Uses the installed NSIS product, Windows UI Automation and window screenshots.
It never injects browser scripts, creates a login or changes native data roots.
"""
import json
import ctypes
import os
from pathlib import Path
import subprocess
import time


def desktop_state():
    """Read the input desktop in the selected user's actual session; never unlock it."""
    user = ctypes.WinDLL('user32', use_last_error=True)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    user.OpenInputDesktop.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    user.OpenInputDesktop.restype = ctypes.c_void_p
    user.GetThreadDesktop.argtypes = [ctypes.c_ulong]
    user.GetThreadDesktop.restype = ctypes.c_void_p
    user.GetUserObjectInformationW.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_void_p,
                                              ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong)]
    user.CloseDesktop.argtypes = [ctypes.c_void_p]
    user.GetForegroundWindow.restype = ctypes.c_void_p
    def name(handle):
        text = ctypes.create_unicode_buffer(256)
        size = ctypes.c_ulong()
        if not user.GetUserObjectInformationW(handle, 2, text, ctypes.sizeof(text), ctypes.byref(size)):
            raise ctypes.WinError(ctypes.get_last_error())
        return text.value
    current = name(user.GetThreadDesktop(kernel.GetCurrentThreadId()))
    input_desktop = user.OpenInputDesktop(0, False, 1)
    if not input_desktop:
        return dict(thread_desktop=current, input_desktop=None, interactive=False,
                    win32_error=ctypes.get_last_error(), foreground=user.GetForegroundWindow())
    try:
        actual = name(input_desktop)
        return dict(thread_desktop=current, input_desktop=actual, interactive=actual == current == 'Default',
                    foreground=user.GetForegroundWindow())
    finally:
        user.CloseDesktop(input_desktop)


UI = r'''
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class DesktopWindow {
 [StructLayout(LayoutKind.Sequential)] public struct Rect {public int Left,Top,Right,Bottom;}
 [StructLayout(LayoutKind.Sequential)] public struct MenuBar {public uint Size;public Rect Bounds;public IntPtr Menu,Window;public uint Flags;}
 [StructLayout(LayoutKind.Sequential)] public struct Point {public int X,Y;public Point(int x,int y){X=x;Y=y;}}
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window,int command);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint dx,uint dy,uint data,UIntPtr extra);
 [DllImport("user32.dll")] public static extern bool GetMenuBarInfo(IntPtr window,int kind,int item,ref MenuBar info);
 [DllImport("user32.dll")] public static extern int GetMenuItemCount(IntPtr menu);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetMenuString(IntPtr menu,uint item,System.Text.StringBuilder text,int capacity,uint flags);
 [DllImport("user32.dll")] public static extern uint GetMenuState(IntPtr menu,uint item,uint flags);
 [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr window,IntPtr menu,uint item,out Rect rect);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
}
'@
$q=Get-Content -LiteralPath (Join-Path $PSScriptRoot 'ui-request.json') -Raw | ConvertFrom-Json
$p=Get-Process -Id $q.pid -ErrorAction Stop
if($p.Path -ne $q.image){throw 'Desktop image changed'}
$p.Refresh()
if($q.action -eq 'tray-observe') {
 $desktop=[Windows.Automation.AutomationElement]::RootElement
 $tops=@($desktop.FindAll([Windows.Automation.TreeScope]::Children,[Windows.Automation.Condition]::TrueCondition))
 $windows=@($tops | Where-Object {($_.Current.ClassName -in @('Shell_TrayWnd','NotifyIconOverflowWindow','TopLevelWindowForOverflowXamlIsland') -or ($_.Current.ClassName -eq '#32768' -and $_.Current.ProcessId -eq $q.pid)) -and -not $_.Current.IsOffscreen})
 $nodes=@(foreach($w in $windows){foreach($e in $w.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)){
  @{name=$e.Current.Name;type=$e.Current.ControlType.ProgrammaticName;automation_id=$e.Current.AutomationId;offscreen=$e.Current.IsOffscreen;runtime_id=($e.GetRuntimeId() -join '.');rect=@($e.Current.BoundingRectangle.X,$e.Current.BoundingRectangle.Y,$e.Current.BoundingRectangle.Width,$e.Current.BoundingRectangle.Height)}
 }})
 if($nodes.Count -gt 1600){throw 'Notification observation exceeds its bound'}
 $rect=[Windows.Rect]::Empty
 foreach($w in $windows){$rect.Union($w.Current.BoundingRectangle)}
 if($rect.Width -lt 1 -or $rect.Height -lt 1 -or $rect.Width -gt 4096 -or $rect.Height -gt 4096){throw 'Notification area is not capturable'}
 $bitmap=New-Object Drawing.Bitmap ([int]$rect.Width),([int]$rect.Height)
 $graphics=[Drawing.Graphics]::FromImage($bitmap)
 try {$graphics.CopyFromScreen([int]$rect.X,[int]$rect.Y,0,0,$bitmap.Size);$bitmap.Save((Join-Path $PSScriptRoot 'tray.png'),[Drawing.Imaging.ImageFormat]::Png)}
 finally {$graphics.Dispose();$bitmap.Dispose()}
 @{pid=$p.Id;nodes=$nodes;top_windows=@(foreach($w in $tops){@{name=$w.Current.Name;class=$w.Current.ClassName;pid=$w.Current.ProcessId}})}|ConvertTo-Json -Depth 6 -Compress
 exit 0
}
if($q.action -eq 'quit') {
 $desktop=[Windows.Automation.AutomationElement]::RootElement
 $class=[Windows.Automation.AutomationElement]::ClassNameProperty
 $name=[Windows.Automation.AutomationElement]::NameProperty
 $children=[Windows.Automation.TreeScope]::Children
 $descendants=[Windows.Automation.TreeScope]::Descendants
 function NativeQuit {
  $menus=@($desktop.FindAll($children,[Windows.Automation.Condition]::TrueCondition) | Where-Object {$_.Current.ProcessId -eq $q.pid -and $_.Current.ClassName -eq '#32768' -and -not $_.Current.IsOffscreen})
  if($menus.Count -eq 0){return $false}
  if($menus.Count -ne 1){throw 'More than one measured-process popup menu is visible'}
  $window=[IntPtr]$menus[0].Current.NativeWindowHandle
  $bar=[DesktopWindow+MenuBar]::new()
  $bar.Size=[Runtime.InteropServices.Marshal]::SizeOf($bar)
  # Documented OBJID_CLIENT reads the real popup menu; no app command is injected.
  if(-not [DesktopWindow]::GetMenuBarInfo($window,-4,0,[ref]$bar)){throw 'Cannot inspect the owned native popup menu'}
  $count=[DesktopWindow]::GetMenuItemCount($bar.Menu)
  if($count -lt 1 -or $count -gt 32){throw 'Native popup menu exceeds its bound'}
  $items=@(for($i=0;$i -lt $count;$i++) {
   $text=[Text.StringBuilder]::new(512)
   [void][DesktopWindow]::GetMenuString($bar.Menu,[uint32]$i,$text,512,0x400)
   $rect=[DesktopWindow+Rect]::new()
   if(-not [DesktopWindow]::GetMenuItemRect([IntPtr]::Zero,$bar.Menu,[uint32]$i,[ref]$rect)){throw 'Native menu geometry is unavailable'}
   @{name=$text.ToString();index=$i;state=[DesktopWindow]::GetMenuState($bar.Menu,[uint32]$i,0x400);left=$rect.Left;top=$rect.Top;right=$rect.Right;bottom=$rect.Bottom}
  })
  $items | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'menu-native-before.json') -Encoding UTF8
  $choice=@($items | Where-Object {$_.name.Replace('&','') -eq 'Quit'})
  if($choice.Count -ne 1 -or ($choice[0].state -band 3) -ne 0){throw 'One enabled native Quit item is required'}
  $item=$choice[0]
  $point=[DesktopWindow+Point]::new([int](($item.left+$item.right)/2),[int](($item.top+$item.bottom)/2))
  if([DesktopWindow]::WindowFromPoint($point) -ne $window){throw 'The observed native menu is obscured'}
  [void][DesktopWindow]::SetCursorPos($point.X,$point.Y)
  [DesktopWindow]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
  [DesktopWindow]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
  if(-not $p.WaitForExit(10000)){throw 'Native Quit did not exit the measured Desktop'}
  return $true
 }
 if(NativeQuit){@{quit=$true;pid=$p.Id;menu_item='Quit';method='native popup readback and mouse click'}|ConvertTo-Json -Compress;exit 0}
 function QuitItems {
  $windows=$desktop.FindAll($children,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$q.pid))
  $items=@()
  foreach($w in $windows){$items+=@($w.FindAll($descendants,[Windows.Automation.Condition]::TrueCondition))}
  $menuItems=@($items | Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::MenuItem -and -not $_.Current.IsOffscreen} | Sort-Object {($_.GetRuntimeId() -join '.')} -Unique)
  @($menuItems | ForEach-Object {@{name=$_.Current.Name;runtime_id=($_.GetRuntimeId() -join '.')}}) | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'menu-before.json') -Encoding UTF8
  return @($menuItems | Where-Object {$_.Current.Name -eq 'Quit'})
 }
 $quit=@(QuitItems)
 if($quit.Count -eq 0) {
 $bar=$desktop.FindFirst($children,[Windows.Automation.PropertyCondition]::new($class,'Shell_TrayWnd'))
 if($null -eq $bar){throw 'No native notification area'}
 $find=[Windows.Automation.PropertyCondition]::new($name,'HiRoute')
 $icons=@($bar.FindAll($descendants,$find) | Where-Object {-not $_.Current.IsOffscreen})
 $overflowClasses=@('NotifyIconOverflowWindow','TopLevelWindowForOverflowXamlIsland')
 foreach($className in $overflowClasses) {
  $overflow=$desktop.FindFirst($children,[Windows.Automation.PropertyCondition]::new($class,$className))
  if($null -ne $overflow -and -not $overflow.Current.IsOffscreen){$icons+=@($overflow.FindAll($descendants,$find) | Where-Object {-not $_.Current.IsOffscreen})}
 }
 if($icons.Count -eq 0) {
  $chevrons=@($bar.FindAll($descendants,[Windows.Automation.Condition]::TrueCondition) | Where-Object {
   $_.Current.AutomationId -eq 'SystemTrayIcon' -and $_.Current.Name -in @('Show hidden icons','显示隐藏的图标')
  })
  if($chevrons.Count -ne 1){throw 'Observe the notification area before continuing; icon is unavailable'}
  $chevrons[0].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
  Start-Sleep -Milliseconds 500
  foreach($className in $overflowClasses) {
   $overflow=$desktop.FindFirst($children,[Windows.Automation.PropertyCondition]::new($class,$className))
   if($null -ne $overflow -and -not $overflow.Current.IsOffscreen){$icons+=@($overflow.FindAll($descendants,$find) | Where-Object {-not $_.Current.IsOffscreen})}
  }
 }
 if($icons.Count -ne 1){throw 'Exactly one visible HiRoute notification icon is required'}
 $icons[0].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
 Start-Sleep -Milliseconds 500
 if(NativeQuit){@{quit=$true;pid=$p.Id;menu_item='Quit';method='native popup readback and mouse click'}|ConvertTo-Json -Compress;exit 0}
 $quit=@(QuitItems)
 }
 if($quit.Count -ne 1){throw "The measured Desktop exposed $($quit.Count) native Quit menu items; inspect menu-before.json"}
 $quit[0].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
 if(-not $p.WaitForExit(10000)){throw 'Native Quit did not exit the measured Desktop'}
 @{quit=$true;pid=$p.Id;menu_item='Quit'}|ConvertTo-Json -Compress
 exit 0
}
if($p.MainWindowHandle -eq 0){throw 'No visible native Desktop window'}
[void][DesktopWindow]::ShowWindow($p.MainWindowHandle,9)
[void][DesktopWindow]::SetForegroundWindow($p.MainWindowHandle)
Start-Sleep -Milliseconds 500
if([DesktopWindow]::GetForegroundWindow() -ne $p.MainWindowHandle) {
 $r=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle).Current.BoundingRectangle
 $point=[DesktopWindow+Point]::new([int]($r.X+100),[int]($r.Y+14))
 $owner=[uint32]0
 [void][DesktopWindow]::GetWindowThreadProcessId([DesktopWindow]::WindowFromPoint($point),[ref]$owner)
 if($owner -eq $q.pid) {
  [void][DesktopWindow]::SetCursorPos($point.X,$point.Y)
  [DesktopWindow]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
  [DesktopWindow]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
  Start-Sleep -Milliseconds 500
 }
}
if([DesktopWindow]::GetForegroundWindow() -ne $p.MainWindowHandle) {
 # Windows may deny a background observer foreground activation. Use the
 # actual taskbar button for this installed AppUserModelID, without attaching
 # another application's input queue or sending it keyboard events.
 $desktop=[Windows.Automation.AutomationElement]::RootElement
 $bar=$desktop.FindFirst([Windows.Automation.TreeScope]::Children,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ClassNameProperty,'Shell_TrayWnd'))
 if($null -ne $bar) {
  $buttons=@($bar.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty,'Appid: ai.hiroute.desktop')) | Where-Object {-not $_.Current.IsOffscreen -and $_.Current.Name.StartsWith('HiRoute')})
  if($buttons.Count -eq 1) {
   if([DesktopWindow]::GetForegroundWindow() -ne $p.MainWindowHandle) {
    $r=$buttons[0].Current.BoundingRectangle
    if($r.Width -lt 2 -or $r.Height -lt 2){throw 'The HiRoute taskbar button has no visible geometry'}
    $point=[DesktopWindow+Point]::new([int]($r.X+$r.Width/2),[int]($r.Y+$r.Height/2))
    $owner=[uint32]0
    [void][DesktopWindow]::GetWindowThreadProcessId([DesktopWindow]::WindowFromPoint($point),[ref]$owner)
    if($owner -ne $bar.Current.ProcessId){throw 'The observed HiRoute taskbar button is obscured'}
    [void][DesktopWindow]::SetCursorPos($point.X,$point.Y)
    [DesktopWindow]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    [DesktopWindow]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 750
   }
  }
 }
}
if([DesktopWindow]::GetForegroundWindow() -ne $p.MainWindowHandle){
 $foreground=[Windows.Automation.AutomationElement]::FromHandle([DesktopWindow]::GetForegroundWindow())
 @{expected=$p.MainWindowHandle.ToInt64();foreground=@{handle=[DesktopWindow]::GetForegroundWindow().ToInt64();pid=$foreground.Current.ProcessId;name=$foreground.Current.Name;class=$foreground.Current.ClassName}}|ConvertTo-Json -Depth 4|Set-Content -LiteralPath (Join-Path $PSScriptRoot 'focus.json') -Encoding UTF8
 throw 'Native Desktop is not foreground; inspect focus.json'
}
$root=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
function Observe($root) {
 $all=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
 # Some WebView2 fragment roots are reachable by a native accessibility hit
 # test before their host HWND exposes a complete descendant tree.
 $r=$root.Current.BoundingRectangle
 $hit=[Windows.Automation.AutomationElement]::FromPoint([Windows.Point]::new($r.X+80,$r.Y+160))
 if($hit.Current.ProcessId -eq $q.pid -or $hit.Current.FrameworkId -eq 'Chrome') {
  $top=$hit
  for($i=0;$i -lt 40;$i++) {
   if($top.Current.ControlType -eq [Windows.Automation.ControlType]::Document){
    return @($all)+@($top)+@($top.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition))
   }
   $top=[Windows.Automation.TreeWalker]::RawViewWalker.GetParent($top)
   if($null -eq $top -or $top.Equals($root)){break}
  }
 }
 return @($all)
}
$all=@(Observe $root)
if($all.Count -gt 1600){throw 'UI Automation tree exceeds observation bound'}
if($q.action -in @('invoke','text','close','click')) {
 if($q.action -eq 'close') {
  $root.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close()
 } elseif($q.action -eq 'click') {
  $r=$root.Current.BoundingRectangle
  if($q.window -ne $p.MainWindowHandle.ToInt64() -or $q.width -ne $r.Width -or $q.height -ne $r.Height){throw 'Observed window geometry changed'}
  if($q.x -lt 0 -or $q.y -lt 0 -or $q.x -ge $r.Width -or $q.y -ge $r.Height){throw 'Click is outside the observed window'}
  [void][DesktopWindow]::SetCursorPos([int]($r.X+$q.x),[int]($r.Y+$q.y))
  [DesktopWindow]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
  [DesktopWindow]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
 } else {
  $matches=@($all | Where-Object {($_.GetRuntimeId() -join '.') -eq $q.element.runtime_id} | Select-Object -First 1)
  if($matches.Count -ne 1){throw 'Observed element is no longer unique'}
  $e=$matches[0]
  if($e.Current.Name -ne $q.element.name -or $e.Current.ControlType.ProgrammaticName -ne $q.element.type){throw 'Observed element changed'}
  if(-not $e.Current.IsEnabled -or $e.Current.IsOffscreen){throw 'Element is not interactable'}
  if($q.action -eq 'text') {
   $e.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($q.text)
  } else {
   $pattern=$null
   if($e.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)){$pattern.Invoke()}
   elseif($e.TryGetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern,[ref]$pattern)){$pattern.Select()}
   elseif($e.TryGetCurrentPattern([Windows.Automation.TogglePattern]::Pattern,[ref]$pattern)){$pattern.Toggle()}
   elseif($e.TryGetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern,[ref]$pattern)){$pattern.Expand()}
   else {throw 'Element has no supported native action'}
  }
 }
 Start-Sleep -Milliseconds 750
 $p.Refresh()
 if($p.MainWindowHandle -eq 0){@{closed=$true;pid=$p.Id}|ConvertTo-Json -Compress;exit 0}
 $root=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
 $all=@(Observe $root)
}
$nodes=@(foreach($e in $all){
 $c=$e.Current
 @{runtime_id=($e.GetRuntimeId() -join '.');name=$c.Name;type=$c.ControlType.ProgrammaticName;
   automation_id=$c.AutomationId;enabled=$c.IsEnabled;offscreen=$c.IsOffscreen;
   rect=@($c.BoundingRectangle.X,$c.BoundingRectangle.Y,$c.BoundingRectangle.Width,$c.BoundingRectangle.Height)}
})
$r=$root.Current.BoundingRectangle
if($r.Width -lt 100 -or $r.Height -lt 100 -or $r.Width -gt 4096 -or $r.Height -gt 4096){throw 'Window is not capturable'}
$bitmap=New-Object Drawing.Bitmap ([int]$r.Width),([int]$r.Height)
$graphics=[Drawing.Graphics]::FromImage($bitmap)
try {
 $graphics.CopyFromScreen([int]$r.X,[int]$r.Y,0,0,$bitmap.Size)
 $bitmap.Save((Join-Path $PSScriptRoot 'window.png'),[Drawing.Imaging.ImageFormat]::Png)
} finally {$graphics.Dispose();$bitmap.Dispose()}
@{pid=$p.Id;handle=$p.MainWindowHandle.ToInt64();width=$r.Width;height=$r.Height;window=$root.Current.Name;nodes=$nodes}|ConvertTo-Json -Depth 6 -Compress
'''


def installed(helper, product, pm):
    """Admit actual files, not a process name, window title or a marker alone."""
    roots = helper.locations(Path.home(), Path(os.environ['LOCALAPPDATA']))
    facts = helper.inspect_installation(Path.home(), Path(os.environ['LOCALAPPDATA']))
    record = facts.get('marker', {}).get('record', {})
    install = helper.plain(Path(record.get('install_root', '')))
    if (not install.is_absolute() or install.parent != Path.home() / '.local/lib/hiroute'
            or not install.name.endswith(pm['source_revision'][:12])
            or record.get('target') != pm['target']):
        raise ValueError('Installed Desktop does not match the selected product')
    for name in ('hiroute.exe', 'hirouted.exe'):
        if helper.digest(helper.plain(install / name)) != pm['files'][name]['sha256']:
            raise ValueError('Installed product binary changed')
    shell = helper.plain(roots['programs'] / 'hiroute-shell.exe')
    expected = facts.get('shell-package.json', {}).get('record', {}).get('files', {})
    if (helper.digest(shell) != pm['shell_sha256'] or expected.get('hiroute-shell.exe') != pm['shell_sha256']
            or facts.get('shell_inventory_mismatches')):
        raise ValueError('Installed shell differs from measured NSIS member')
    return roots, facts, shell


def ui_action(run, request, shell, pid, element=None, observed=None):
    state = desktop_state()
    (run / 'desktop-state.json').write_text(json.dumps(state, indent=2), encoding='utf-8')
    if not state['interactive']:
        raise RuntimeError('The existing session input desktop is unavailable; unlock it before native UI acceptance')
    query = dict(pid=pid, image=str(shell), action=request['desktop_action'], element=element,
                 text=request.get('text'))
    if request['desktop_action'] == 'click':
        query.update(window=observed['handle'], width=observed['width'], height=observed['height'],
                     x=request['x'], y=request['y'])
    (run / 'ui-request.json').write_text(json.dumps(query), encoding='utf-8')
    script = run / 'ui.ps1'
    script.write_text(UI, encoding='utf-8-sig')
    completed = subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-File', str(script)],
                               capture_output=True, timeout=60, creationflags=0x08000000)
    (run / 'ui.stderr').write_bytes(completed.stderr)
    if completed.returncode:
        raise RuntimeError('Native UI Automation failed; inspect ui.stderr before retrying')
    observed = json.loads(completed.stdout.decode('utf-8-sig'))
    (run / 'ui.json').write_text(json.dumps(observed, ensure_ascii=False, indent=2), encoding='utf-8')
    return observed


def execute(request, run, product, pm, helper, activity, manifest, root):
    if activity.get('administrator_groups') is not False:
        raise ValueError('Native Desktop evidence requires a standard account, not a filtered administrator')
    action = request['desktop_action']
    if action not in ('install', 'launch', 'snapshot', 'invoke', 'text', 'click', 'close', 'quit', 'tray-observe', 'desktop-state', 'cli', 'native-files'):
        raise ValueError('Unknown native Desktop action')
    if action == 'desktop-state':
        return dict(native_entry='read-only input desktop inspection', state=desktop_state())
    if action == 'native-files':
        files = {}
        for relative in ('.codex/config.toml', '.claude/settings.json',
                         '.agents/skills/hiroute-collaboration', '.claude/skills/hiroute-collaboration'):
            path = helper.plain(Path.home() / relative)
            if not path.exists():
                files[relative] = dict(exists=False)
            elif path.is_file():
                if path.stat().st_size > 4 * 1024 * 1024:
                    raise ValueError('Native configuration exceeds evidence bound')
                name = 'native-' + relative.replace('/', '_').lstrip('.')
                (run / name).write_bytes(path.read_bytes())
                files[relative] = dict(exists=True, sha256=helper.digest(path), bytes=path.stat().st_size)
            else:
                files[relative] = dict(exists=True, inventory=helper.inventory(path))
        (run / 'native-files.json').write_text(json.dumps(files, indent=2), encoding='utf-8')
        return dict(native_entry='read-only native configuration inventory', files=files)
    if action == 'install':
        facts = helper.inspect_installation(Path.home(), Path(os.environ['LOCALAPPDATA']))
        previous = request.get('from_product')
        if previous:
            old = root / 'bundles' / previous
            _, _, shell = installed(helper, old, manifest(old))
            if any(Path(p.get('ExecutablePath') or '') == shell for p in activity.get('processes', [])):
                raise ValueError('Quit the old Desktop through its native UI before upgrade')
        elif 'marker' in facts or facts['programs']['exists'] or facts['stable_cli']['exists']:
            raise ValueError('Fresh Desktop installation must not adopt an existing product')
        with (run / 'installer.stdout').open('xb') as out, (run / 'installer.stderr').open('xb') as err:
            done = subprocess.run([str(product / 'installer.exe'), '/S'], stdin=subprocess.DEVNULL,
                                  stdout=out, stderr=err, timeout=150, check=True)
        _, after, _ = installed(helper, product, pm)
        return dict(native_entry='NSIS /S', exit_code=done.returncode, installation=after,
                    upgrade_from=previous, installer_sha256=pm['files']['installer.exe']['sha256'])
    roots, facts, shell = installed(helper, product, pm)
    if action == 'launch':
        child = subprocess.Popen([str(shell)], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL)
        # A second launch uses the product's single-instance protocol. Discover
        # its real window, rather than assuming this new PID owns the WebView.
        time.sleep(4)
        return dict(native_entry='installed Desktop', launched_pid=child.pid,
                    shell_sha256=helper.digest(shell))
    if action == 'cli':
        args = request['cli_args']
        if (not isinstance(args, list) or not args or len(args) > 40
                or any(not isinstance(a, str) or len(a) > 64 * 1024 or '\x00' in a for a in args)
                or args[0] not in ('system', 'service', 'agents', 'plans', 'compute', 'operations', 'console')):
            raise ValueError('Only bounded public product CLI families are accepted')
        environment = os.environ.copy()
        environment.update(request['config']['environment'])
        environment['PATH'] = ';'.join(request['config']['path']) + ';' + environment['PATH']
        done = subprocess.run([str(roots['stable_cli']), *args], capture_output=True, timeout=150,
                              env=environment, creationflags=0x08000000)
        (run / 'cli.stdout').write_bytes(done.stdout)
        (run / 'cli.stderr').write_bytes(done.stderr)
        return dict(native_entry='installed public CLI', exit_code=done.returncode)
    candidates = [p for p in activity.get('processes', []) if Path(p.get('ExecutablePath') or '') == shell]
    if len(candidates) != 1:
        raise ValueError('Exactly one measured installed Desktop process is required')
    pid = candidates[0]['ProcessId']
    element = None
    observed = None
    if action in ('invoke', 'text', 'click'):
        parent = root / 'runs' / request['observation_run']
        old_request = json.loads((parent / 'request.json').read_text(encoding='utf-8'))
        old_result = json.loads((parent / 'result.json').read_text(encoding='utf-8'))
        observed = json.loads((parent / 'ui.json').read_text(encoding='utf-8'))
        if (not old_result.get('passed') or old_request['product'] != request['product']
                or old_request['config']['session_id'] != request['config']['session_id']
                or observed['pid'] != pid or time.time() - (parent / 'ui.json').stat().st_mtime > 600):
            raise ValueError('A recent observation of this exact Desktop process is required')
        if action != 'click':
            matches = [e for e in observed['nodes'] if e['runtime_id'] == request['element']]
            if not matches:
                raise ValueError('Choose one observed native UI element')
            element = matches[0]
    return dict(native_entry='Windows UI Automation', observation=ui_action(run, request, shell, pid, element, observed),
                shell_sha256=helper.digest(shell))
