#!/usr/bin/env python3
"""Bounded native Desktop acceptance in an existing validation session.

Uses the installed NSIS product, Windows UI Automation and window screenshots.
It never injects browser scripts, creates a login or changes native data roots.
"""
import json
import base64
import hashlib
import ctypes
import os
from pathlib import Path
import re
import subprocess
import time
from urllib.parse import urlsplit


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
if($q.action -in @('invoke','text','close','click','scroll')) {
 if($q.action -eq 'close') {
  $root.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close()
 } elseif($q.action -in @('click','scroll')) {
  $r=$root.Current.BoundingRectangle
  if($q.window -ne $p.MainWindowHandle.ToInt64() -or $q.width -ne $r.Width -or $q.height -ne $r.Height){throw 'Observed window geometry changed'}
  if($q.x -lt 0 -or $q.y -lt 0 -or $q.x -ge $r.Width -or $q.y -ge $r.Height){throw 'Click is outside the observed window'}
  [void][DesktopWindow]::SetCursorPos([int]($r.X+$q.x),[int]($r.Y+$q.y))
  if($q.action -eq 'scroll') {
   if($q.direction -notin @('up','down')){throw 'Invalid scroll direction'}
   $delta=if($q.direction -eq 'down'){-480}else{480}
   $data=[BitConverter]::ToUInt32([BitConverter]::GetBytes([int]$delta),0)
   [DesktopWindow]::mouse_event(0x0800,0,0,$data,[UIntPtr]::Zero)
  } else {
   [DesktopWindow]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
   [DesktopWindow]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
  }
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
    if request['desktop_action'] in ('click', 'scroll'):
        if request['desktop_action'] == 'scroll' and request.get('scroll_direction') not in ('up', 'down'):
            raise ValueError('Scrolling requires an explicit direction')
        query.update(window=observed['handle'], width=observed['width'], height=observed['height'],
                     x=request['x'], y=request['y'], direction=request.get('scroll_direction'))
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


def codex_config_path(home, profile):
    if profile not in (None, 'hiroute'):
        raise ValueError('Only the product-owned Codex profile is accepted')
    return home / '.codex' / ('hiroute.config.toml' if profile else 'config.toml')


def codex_gateway(document, profile):
    codex_config_path(Path.home(), profile)
    selected = document
    target = document.get('model_providers', {}).get('hiroute', {}).get('base_url', '')
    address = urlsplit(target)
    if (selected.get('model_provider') != 'hiroute' or address.scheme != 'http'
            or address.hostname not in ('127.0.0.1', 'localhost', '::1')
            or not address.port or address.username or address.password):
        raise ValueError('Saved Codex configuration must select the local HiRoute Gateway')
    return target


def codex_call_evidence(stdout, expected, directory_token):
    events = [json.loads(line) for line in stdout.decode('utf-8-sig').splitlines() if line.strip()]
    items = [event.get('item', {}) for event in events if event.get('type') == 'item.completed']
    return dict(turn_completed=any(event.get('type') == 'turn.completed' for event in events),
                answer=any(item.get('type') == 'agent_message' and item.get('text', '').strip() == expected for item in items),
                shell_tool=any(item.get('type') == 'command_execution' and item.get('status') == 'completed'
                               and item.get('exit_code') == 0
                               and re.search(r'\bGet-Location\b', item.get('command', ''), re.I)
                               and directory_token in item.get('aggregated_output', '') for item in items))


def codex_process(run, command, environment, stem='codex'):
    try:
        done = subprocess.run(command, cwd=run, stdin=subprocess.DEVNULL, capture_output=True,
                              timeout=150, env=environment, creationflags=0x08000000)
    except subprocess.TimeoutExpired as error:
        (run / (stem + '.stdout')).write_bytes(error.stdout or b'')
        (run / (stem + '.stderr')).write_bytes(error.stderr or b'')
        raise
    (run / (stem + '.stdout')).write_bytes(done.stdout)
    (run / (stem + '.stderr')).write_bytes(done.stderr)
    return done


def codex_thread_id(stdout):
    events = [json.loads(line) for line in stdout.decode('utf-8-sig').splitlines() if line.strip()]
    threads = [event.get('thread_id') for event in events if event.get('type') == 'thread.started']
    if len(threads) != 1 or not isinstance(threads[0], str) or not re.fullmatch(
            r'[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}', threads[0]):
        raise ValueError('A single native Codex session identity is required')
    return threads[0]


def codex_call(run, request, helper):
    environment = os.environ.copy()
    environment.update(request['config']['environment'])
    environment['PATH'] = ';'.join(request['config']['path']) + ';' + environment['PATH']
    if any(environment.get(name) for name in ('CODEX_HOME', 'OPENAI_API_KEY', 'OPENAI_BASE_URL', 'CODEX_API_KEY')):
        raise ValueError('Native acceptance requires saved configuration without auth or routing environment overrides')
    import tomllib  # This optional native scenario requires the configured Python 3.11+ runtime.
    profile = request.get('codex_profile')
    two_turn = request.get('desktop_action') == 'codex-two-turn'
    if two_turn and profile != 'hiroute':
        raise ValueError('Two-turn validation requires the separate HiRoute Codex profile')
    config = helper.plain(codex_config_path(Path.home(), profile))
    if config.stat().st_size > 4 * 1024 * 1024:
        raise ValueError('Native configuration exceeds evidence bound')
    endpoint = codex_gateway(tomllib.loads(config.read_text(encoding='utf-8-sig')), profile)
    binary = helper.plain(Path(environment['HIROUTE_WORKER_CODEX_BINARY']))
    checksum = helper.digest(binary)
    version = subprocess.run([str(binary), '--version'], stdin=subprocess.DEVNULL,
        capture_output=True, timeout=15, env=environment, creationflags=0x08000000)
    (run / 'codex-version.stdout').write_bytes(version.stdout)
    (run / 'codex-version.stderr').write_bytes(version.stderr)
    if version.returncode or not version.stdout.strip() or len(version.stdout) > 1024:
        raise RuntimeError('The configured native client version could not be measured')
    identity = dict(binary_sha256=checksum, client_version=version.stdout.decode('utf-8-sig').strip(),
                    profile=profile, gateway_endpoint=endpoint)
    (run / 'codex-input.json').write_text(json.dumps(identity, indent=2), encoding='utf-8')
    expected = 'HiRoute-Windows-' + request['run_id'] + ('-a:ok' if two_turn else ':ok')
    inference_only = request.get('desktop_action') in ('codex-inference', 'codex-two-turn')
    prompt = (('Do not use any tools or read or modify files. Reply with exactly ' if inference_only else
               'Do not read or modify any files or change directories. Use your shell tool to run '
               'the read-only PowerShell command Get-Location in the current working directory. '
               'Then reply with exactly ') + expected)
    command = [str(binary), 'exec', '--json', '--skip-git-repo-check', '--sandbox', 'read-only',
               '--config', 'approval_policy="never"']
    if profile:
        command += ['--profile', profile]
    command.append(prompt)
    done = (codex_process(run, command, environment, 'codex-a') if two_turn
            else codex_process(run, command, environment))
    evidence = codex_call_evidence(done.stdout, expected, run.name)
    if two_turn:
        if helper.digest(binary) != checksum or done.returncode != 0 or not all(
                evidence[key] for key in ('turn_completed', 'answer')):
            raise RuntimeError('First native Codex turn did not complete; no follow-up was sent')
        thread = codex_thread_id(done.stdout)
        next_expected = 'HiRoute-Windows-' + request['run_id'] + '-b:ok'
        followup = ('The previous reply was correct. Do not use any tools or read or modify files. '
                    'Reply with exactly ' + next_expected)
        resumed = codex_process(run, command[:-1] + ['resume', thread, followup], environment, 'codex-b')
        second = codex_call_evidence(resumed.stdout, next_expected, run.name)
        if (helper.digest(binary) != checksum or resumed.returncode != 0
                or codex_thread_id(resumed.stdout) != thread
                or not all(second[key] for key in ('turn_completed', 'answer'))):
            raise RuntimeError('The resumed native Codex turn did not complete in the same session')
        result = dict(identity, scope='two_turn_inference', native_entry='configured Codex exec and exact-session resume; read-only sandbox',
                      thread_id=thread, first=evidence, second=second, exit_codes=[done.returncode, resumed.returncode])
        (run / 'codex-two-turn.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
        return result
    evidence.update(identity, exit_code=done.returncode,
                    scope='inference_only' if inference_only else 'tool_and_inference',
                    native_entry='configured Codex exec; read-only sandbox')
    filename = 'codex-inference.json' if inference_only else 'codex-call.json'
    (run / filename).write_text(json.dumps(evidence, indent=2), encoding='utf-8')
    required = ('turn_completed', 'answer') if inference_only else ('turn_completed', 'answer', 'shell_tool')
    if (helper.digest(binary) != checksum or done.returncode != 0
            or not all(evidence[key] for key in required)):
        if inference_only:
            raise RuntimeError('Native Codex inference did not complete the turn and exact answer checks; inspect private evidence')
        raise RuntimeError('Native Codex call did not complete the real tool and answer checks; inspect private evidence')
    return evidence


def prepared_installer_replacement(request, pm, helper, activity, manifest, root):
    if (not re.fullmatch(r'[a-f0-9]{32}', request.get('observation_run') or '')
            or not re.fullmatch(r'[a-f0-9]{12}-[a-f0-9]{12}', request.get('from_product') or '')):
        raise ValueError('Installer resume requires explicit inspection and prior product identities')
    observed = root / 'runs' / request['observation_run']
    before = json.loads((observed / 'result.json').read_text(encoding='utf-8'))
    old_request = json.loads((observed / 'request.json').read_text(encoding='utf-8'))
    if (before.get('passed') is not True or before.get('assessment') != 'inspection_only'
            or old_request['product'] != request['product']
            or old_request['config']['session_id'] != request['config']['session_id']
            or time.time() - (observed / 'result.json').stat().st_mtime > 600):
        raise ValueError('Installer resume requires a recent matching installation inspection')
    facts = helper.inspect_installation(Path.home(), Path(os.environ['LOCALAPPDATA']))
    for name in ('marker', 'install-pending.json'):
        if facts.get(name, {}).get('sha256') != before['installation'].get(name, {}).get('sha256'):
            raise ValueError('Installation changed after inspection')
    pending = facts.get('install-pending.json', {}).get('record', {})
    if pending.get('schema') != 'hiroute.windows-shell-transaction/v1' or pending.get('phase') != 'prepared':
        raise ValueError('Only the exact prepared shell installation can be resumed')
    prior = manifest(root / 'bundles' / request['from_product'])
    marker = facts['marker']['record']
    previous = helper.plain(Path(pending.get('previous_package') or ''))
    if (not previous.name.endswith(prior['source_revision'][:12])
            or previous.parent != Path.home() / '.local/lib/hiroute'):
        raise ValueError('The retained previous package does not match the requested product')
    for name in ('hiroute.exe', 'hirouted.exe'):
        if helper.digest(previous / name) != prior['files'][name]['sha256']:
            raise ValueError('The retained previous product changed')
    active=helper.plain(Path(marker['install_root']))
    accepted=prior if active==previous else pm
    if active.parent!=previous.parent or not active.name.endswith(accepted['source_revision'][:12]):
        raise ValueError('The current installation marker is outside the admitted old/current pair')
    for name in ('hiroute.exe','hirouted.exe'):
        if helper.digest(active/name)!=accepted['files'][name]['sha256']:
            raise ValueError('The current installation marker points at a substituted product')
    programs = helper.plain(Path(facts['programs']['path']))
    shell = helper.plain(programs / 'hiroute-shell.exe')
    if (helper.digest(shell) != pm['shell_sha256'] or pending.get('new', {}).get('hiroute-shell.exe') != pm['shell_sha256']
            or any(Path(p.get('ExecutablePath') or '') == shell for p in activity.get('processes', []))):
        raise ValueError('The measured replacement shell must be present and stopped')
    for name in ('hiroute.exe', 'hirouted.exe'):
        path = helper.plain(programs / 'standalone' / name)
        if helper.digest(path) != pm['files'][name]['sha256']:
            raise ValueError('The replacement product changed')
    return facts, programs, shell


def quarantine_generated_uninstaller(programs, state, run, expected, pending, helper):
    if (not re.fullmatch(r'[a-f0-9]{64}', expected or '')
            or 'uninstall.exe' in pending.get('new', {}) or not pending.get('old', {}).get('uninstall.exe')):
        raise ValueError('Only an unregistered generated uninstaller can be quarantined manually')
    source = helper.plain(programs / 'uninstall.exe')
    backup = helper.plain(state / 'install-backups/uninstall.exe')
    if (helper.digest(source) != expected or helper.digest(backup) != pending['old']['uninstall.exe']
            or expected == pending['old']['uninstall.exe']):
        raise ValueError('Generated-file or retained-backup identity changed; no file was moved')
    destination = run / 'unregistered-uninstaller.bin'
    if destination.exists():
        raise ValueError('Quarantine destination already exists')
    source.rename(destination)
    if helper.digest(destination) != expected:
        if not source.exists():
            destination.rename(source)
        raise ValueError('Quarantined generated file changed')
    return dict(native_entry='manual quarantine of an independently verified generated file; not installation acceptance',
                sha256=expected, previous_backup_preserved=True, business_data_untouched=True)


def installer_resume(request, run, pm, helper, activity, manifest, root):
    facts, programs, shell = prepared_installer_replacement(request, pm, helper, activity, manifest, root)
    pending = facts['install-pending.json']['record']
    command = [str(shell), '--installer-install']
    if pending.get('previous_autostart') is False:
        command.append('--no-login-start')
    if facts.get('shell-install.json', {}).get('record', {}).get('tray_enabled') is False:
        command.append('--no-tray')
    done = subprocess.run(command, stdin=subprocess.DEVNULL,
                          capture_output=True, timeout=180, creationflags=0x08000000)
    (run / 'installer-resume.stdout').write_bytes(done.stdout)
    (run / 'installer-resume.stderr').write_bytes(done.stderr)
    (run / 'installer-resume.after.json').write_text(json.dumps(
        helper.inspect_installation(Path.home(), Path(os.environ['LOCALAPPDATA'])),indent=2),encoding='utf-8')
    if done.returncode:
        raise RuntimeError('The real post-install phase failed; inspect installer-resume.stdout before any further action')
    return dict(native_entry='measured installer post-install phase; not a full NSIS result', exit_code=done.returncode)


def probe_tls(run, target):
    import hashlib
    import socket
    import ssl
    host, port = target.get('authority', ''), target.get('port')
    if (target.get('scheme') != 'https' or not re.fullmatch(r'[A-Za-z0-9.-]{1,253}', host)
            or not isinstance(port, int) or not 1 <= port <= 65535):
        raise ValueError('TLS diagnostics require a saved HTTPS DNS target')
    context = ssl.create_default_context()
    context.set_alpn_protocols(['h2', 'http/1.1'])
    result = dict(host=host, port=port, certificate_store=context.cert_store_stats(),
                  authenticated=False, http_request_sent=False, tls_verified=False)
    try:
        with socket.create_connection((host, port), timeout=8) as stream:
            with context.wrap_socket(stream, server_hostname=host) as secured:
                result.update(tls_verified=True, tls_version=secured.version(),
                    alpn=secured.selected_alpn_protocol(),
                    peer_certificate_sha256=hashlib.sha256(secured.getpeercert(binary_form=True)).hexdigest())
    except (OSError, ssl.SSLError) as error:
        result.update(error_type=type(error).__name__, error=str(error)[:1024],
                      verify_code=getattr(error, 'verify_code', None))
    (run / 'tls-probe.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    if not result['tls_verified']:
        raise RuntimeError('TLS verification failed before any authenticated or HTTP request; inspect tls-probe.json')
    return result


def probe_windows_tls(run, target):
    if (target.get('scheme') != 'https'
            or not re.fullmatch(r'[A-Za-z0-9.-]{1,253}', target.get('authority', ''))
            or not isinstance(target.get('port'), int) or not 1 <= target['port'] <= 65535):
        raise ValueError('TLS diagnostics require a saved HTTPS DNS target')
    (run / 'tls-target.json').write_text(json.dumps(target), encoding='utf-8')
    script = run / 'tls-native.ps1'
    script.write_text(r'''
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$q=Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'tls-target.json') | ConvertFrom-Json
$tcp=[Net.Sockets.TcpClient]::new();$ssl=$null
$result=@{tls_verified=$false;authenticated=$false;http_request_sent=$false;stack='Windows SslStream'}
try {
 $connect=$tcp.ConnectAsync([string]$q.authority,[int]$q.port)
 if(-not $connect.Wait(8000)){throw 'TCP connect timeout'}
 $connect.GetAwaiter().GetResult() | Out-Null
 $ssl=[Net.Security.SslStream]::new($tcp.GetStream(),$false)
 $handshake=$ssl.AuthenticateAsClientAsync([string]$q.authority)
 if(-not $handshake.Wait(8000)){throw 'TLS handshake timeout'}
 $handshake.GetAwaiter().GetResult() | Out-Null
 $result.tls_verified=$true;$result.tls_version=$ssl.SslProtocol.ToString()
} catch {$result.error=$_.Exception.GetBaseException().Message}
finally {if($null -ne $ssl){$ssl.Dispose()};$tcp.Dispose()}
$result | ConvertTo-Json -Compress
''', encoding='utf-8-sig')
    done = subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-File', str(script)],
        capture_output=True, timeout=30, creationflags=0x08000000)
    (run / 'tls-native.stderr').write_bytes(done.stderr)
    done.check_returncode()
    result = json.loads(done.stdout.decode('utf-8-sig'))
    (run / 'tls-probe.native.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    if not result.get('tls_verified'):
        raise RuntimeError('Windows system TLS verification failed; inspect tls-probe.native.json')
    return result


def network_probe(request, run, roots):
    done = subprocess.run([str(roots['stable_cli']), 'compute', 'list', '--output', 'json'],
        capture_output=True, timeout=30, creationflags=0x08000000)
    done.check_returncode()
    envelope = json.loads(done.stdout)
    matches = [s for s in envelope.get('data', {}).get('sources', [])
               if s.get('source_id') == request.get('source_id') and s.get('state') == 'ready']
    if envelope.get('status') != 'succeeded' or len(matches) != 1:
        raise ValueError('Select one ready saved source for unauthenticated TLS diagnosis')
    probe = probe_windows_tls if request.get('native_tls') else probe_tls
    return probe(run, matches[0]['target'])


def admit_nsis_install(request, helper, activity, manifest, root):
    facts = helper.inspect_installation(Path.home(), Path(os.environ['LOCALAPPDATA']))
    previous = request.get('from_product')
    interrupted = request.get('recover_product')
    if interrupted:
        if not previous or not re.fullmatch(r'[a-f0-9]{12}-[a-f0-9]{12}', interrupted):
            raise ValueError('Prepared recovery requires explicit previous and interrupted products')
        prepared_installer_replacement({**request, 'product': interrupted},
            manifest(root / 'bundles' / interrupted), helper, activity, manifest, root)
    elif previous:
        old = root / 'bundles' / previous
        _, _, shell = installed(helper, old, manifest(old))
        if any(Path(p.get('ExecutablePath') or '') == shell for p in activity.get('processes', [])):
            raise ValueError('Quit the old Desktop through its native UI before upgrade')
    elif 'marker' in facts or facts['programs']['exists'] or facts['stable_cli']['exists']:
        raise ValueError('Fresh Desktop installation must not adopt an existing product')


# The shell's installer adapter permits one 600-second native install request.
# NSIS also extracts files and runs its prepare/commit hooks outside that request.
# Keep this specific whole-installer watchdog outside that product deadline;
# ordinary public CLI/UI actions retain their existing bounds.
NSIS_INSTALL_TIMEOUT = 720


def nsis_install(installer, run, locations):
    """Wait for the actual NSIS child and retain bounded phase/timing evidence.

    Observing a committed marker is never a substitute for installer exit. On
    timeout stop only our owned NSIS process; its product transaction remains
    available for inspection and normal recovery, without automatic replay.
    """
    observed = {
        'shell': locations['shell_state'] / 'install-pending.json',
        'standalone': locations['data'] / 'service/install-journal.json',
        'installation': locations['data'] / 'standalone.json',
        'outcome': locations['shell_state'] / 'update-outcome.json',
    }
    started = time.monotonic()
    evidence = dict(watchdog_seconds=NSIS_INSTALL_TIMEOUT, events=[])
    signatures = {}

    def record(final=None):
        changed = {}
        for name, path in observed.items():
            try:
                info = path.stat()
                signature = (info.st_mtime_ns, info.st_size)
                if signature == signatures.get(name):
                    continue
                signatures[name] = signature
                value = dict(present=True, bytes=info.st_size)
                if info.st_size <= 1024 * 1024:
                    with path.open('rb') as source:
                        data = source.read(1024 * 1024 + 1)
                    try:
                        row = json.loads(data)
                        value.update({k: row[k] for k in ('phase', 'status')
                                      if isinstance(row.get(k), str) and len(row[k]) <= 64})
                    except (ValueError, TypeError):
                        value['incomplete_read'] = True
                changed[name] = value
            except FileNotFoundError:
                if signatures.get(name) != 'absent':
                    signatures[name] = 'absent'
                    changed[name] = dict(present=False)
            except OSError:
                if signatures.get(name) != 'unavailable':
                    signatures[name] = 'unavailable'
                    changed[name] = dict(unavailable=True)
        if changed and len(evidence['events']) < 256:
            evidence['events'].append(dict(elapsed_seconds=round(time.monotonic()-started, 3),
                                           changes=changed))
        if final is not None:
            evidence.update(final, elapsed_seconds=round(time.monotonic()-started, 3))
        (run / 'installer-process.json').write_text(json.dumps(evidence, indent=2), encoding='utf-8')

    with (run / 'installer.stdout').open('xb') as out, (run / 'installer.stderr').open('xb') as err:
        child = subprocess.Popen([str(installer), '/S'], stdin=subprocess.DEVNULL, stdout=out, stderr=err)
        evidence['pid'] = child.pid
        while True:
            code = child.poll()
            record()
            if code is not None:
                record(dict(exit_code=code, timed_out=False))
                if code:
                    raise subprocess.CalledProcessError(code, [str(installer), '/S'])
                return evidence
            if time.monotonic()-started >= NSIS_INSTALL_TIMEOUT:
                record(dict(timed_out=True))
                child.kill()
                child.wait(timeout=30)
                raise subprocess.TimeoutExpired([str(installer), '/S'], NSIS_INSTALL_TIMEOUT)
            time.sleep(0.5)


def failure_page_languages(run, shell, pid, cause):
    import shutil
    observed=ui_action(run,dict(desktop_action='snapshot'),shell,pid)
    deadline=time.monotonic()+75
    languages=[]
    reasons={'locked':('另一个 HiRoute 实例','Another HiRoute instance'),
             'unsupported_format':('这组数据的格式没有经过验证','This data format has no verified upgrade path'),
             'installer_recovery':('安装尚未完成','Installation is incomplete')}
    for index in range(2):
        while True:
            names='\n'.join(n.get('name','') for n in observed.get('nodes',[]))
            zh,en=reasons[cause]
            language='zh' if zh in names else 'en' if en in names else None
            if language is not None:break
            if time.monotonic()>=deadline:raise ValueError('The native failure page did not expose the current cause')
            time.sleep(1)
            observed=ui_action(run,dict(desktop_action='snapshot'),shell,pid)
        prefix={'locked':'locked','unsupported_format':'format','installer_recovery':'installer-recovery'}[cause]
        shutil.copyfile(run/'window.png',run/(prefix+'-'+language+'.png'))
        shutil.copyfile(run/'ui.json',run/(prefix+'-'+language+'.json'))
        if cause=='installer_recovery':
            if ('SHELL_INSTALL_RECOVERY_REQUIRED' not in names
                    or ('相同版本及构建' if language=='zh' else 'same version and build') not in names
                    or ('寻求支持' if language=='zh' else 'obtain support') not in names
                    or any(n.get('name') in ('重试','Try again') and not n.get('offscreen') for n in observed.get('nodes',[]))
                    or 'RECOVERY.md' in names):
                raise ValueError('The incomplete installation page borrowed storage advice or lost its exact repair/support path')
        languages.append(language)
        if index==0:
            buttons=[n for n in observed['nodes'] if n.get('name') in ('English','中文') and n.get('type')=='ControlType.Button' and not n.get('offscreen')]
            if len(buttons)!=1:raise ValueError('The actual failure-page language control is unavailable')
            observed=ui_action(run,dict(desktop_action='invoke'),shell,pid,buttons[0])
    if set(languages)!={'zh','en'}:raise ValueError('Both native failure-page languages were not observed')
    return languages


def startup_ui_context(request, product, pm, helper, activity, root):
    observation = request.get('observation_run')
    if not re.fullmatch(r'[a-f0-9]{32}',observation or ''):
        raise ValueError('The native lock UI case requires a recent current-product inspection')
    inspected = root/'runs'/observation
    before_request = helper.bounded_json(inspected/'request.json')
    before_result = helper.bounded_json(inspected/'result.json')
    if (before_request.get('operation') != 'inspect' or before_request['product'] != request['product']
            or not before_result.get('passed') or before_result.get('administrator') is not False
            or before_request['config']['user'] != request['config']['user']
            or before_request['config']['session_id'] != request['config']['session_id']
            or time.time()-(inspected/'result.json').stat().st_mtime>3600):
        raise ValueError('The native lock UI inspection is stale or belongs to another product/account')
    roots,facts,shell = installed(helper,product,pm)
    if any(Path(p.get('ExecutablePath') or '') == shell for p in activity.get('processes', [])):
        raise ValueError('Quit the installed Desktop before the bounded native UI case')
    install=Path(facts['marker']['record']['install_root'])
    allowed={install/'hirouted.exe',install/'libexec/cliproxyapi.exe'}
    task=json.loads(facts['definition']['text']).get('name','')
    if (not re.fullmatch(r'HiRoute-[a-f0-9]{64}',task)
            or any(Path(p.get('ExecutablePath') or '') not in allowed for p in activity.get('processes',[]))
            or any(t.get('TaskName')!=task for t in activity.get('tasks',[]))):
        raise ValueError('Preserve unrelated product activity before the bounded native UI fixture')
    if not desktop_state().get('interactive'):
        raise ValueError('The existing input desktop is unavailable; no lock fixture was created')
    return roots,facts,shell


def locked_startup_ui(request, run, product, pm, helper, activity, root, file_use=False):
    """Own one bounded lock fixture, inspect both real languages, then reopen normally."""
    import shutil
    roots,facts,shell = startup_ui_context(request,product,pm,helper,activity,root)
    cli = Path(facts['marker']['record']['install_root'])/'hiroute.exe'
    def command(args,label):
        done=subprocess.run([str(cli),*args],capture_output=True,timeout=150,creationflags=0x08000000)
        (run/(label+'.stdout')).write_bytes(done.stdout);(run/(label+'.stderr')).write_bytes(done.stderr)
        if done.returncode:raise ValueError('The current-product fixture command failed: '+label)
        return json.loads(done.stdout)
    if command(['system','status','--output','json'],'before-system').get('data',{}).get('gateway') != 'ready':
        raise ValueError('The current Gateway must be healthy before the native lock fixture')
    command(['service','stop','--output','json'],'fixture-stop')
    storage=roots['state']/'storage'
    report=helper.plain(storage/'startup-recovery.json')
    report_before=helper.digest(report) if report.exists() else None
    kernel=ctypes.WinDLL('kernel32',use_last_error=True)
    from ctypes import wintypes
    kernel.CreateFileW.argtypes=[wintypes.LPCWSTR,wintypes.DWORD,wintypes.DWORD,ctypes.c_void_p,wintypes.DWORD,wintypes.DWORD,wintypes.HANDLE]
    kernel.CreateFileW.restype=wintypes.HANDLE;kernel.CloseHandle.argtypes=[wintypes.HANDLE]
    target=storage/('live/control.db' if file_use else 'startup.lock')
    handle=kernel.CreateFileW(str(helper.plain(target)),0,0,None,3,0x00200000,None)
    if handle==wintypes.HANDLE(-1).value:raise ctypes.WinError(ctypes.get_last_error())
    child=None
    languages=[]
    failure=None
    try:
        child=subprocess.Popen([str(shell)],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        time.sleep(15)
        languages=failure_page_languages(run,shell,child.pid,'locked')
        offline=command(['service','recovery','--output','json'],'locked-offline-recovery').get('data',{}).get('recovery',{})
        if offline.get('cause')!='locked' or offline.get('action')!='retry_current' or offline.get('backup_batch'):
            raise ValueError('A correctable native file-use prerequisite did not offer current-version retry')
    except Exception as error:
        failure=error
    finally:
        kernel.CloseHandle(handle)
        if child is not None and child.poll() is None:
            try:
                ui_action(run,dict(desktop_action='quit'),shell,child.pid)
                child.wait(timeout=20)
            except Exception:
                if child.poll() is None:child.terminate();child.wait(timeout=20)
    recovery_file=helper.plain(storage/'startup-recovery.json')
    unchanged=(helper.digest(recovery_file) if recovery_file.exists() else None)==report_before
    if not file_use and not unchanged and failure is None:
        failure=ValueError('A non-writer lock failure overwrote the previous recovery report')
    # Follow the visible instruction: release the file use and reopen this same
    # complete version. Keep business readiness separate from the window result.
    reopened=subprocess.Popen([str(shell)],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    try:
        time.sleep(15)
        ui_action(run,dict(desktop_action='snapshot'),shell,reopened.pid)
        shutil.copyfile(run/'window.png',run/'reopened-current.png')
        if command(['system','status','--output','json'],'reopened-system').get('data',{}).get('gateway')!='ready':
            raise ValueError('Reopening the current version did not restore actual Gateway readiness')
    finally:
        if reopened.poll() is None:
            try:ui_action(run,dict(desktop_action='quit'),shell,reopened.pid);reopened.wait(timeout=20)
            except Exception:
                if reopened.poll() is None:reopened.terminate();reopened.wait(timeout=20)
    if failure is not None:raise failure
    return dict(native_entry='real standard-user '+('current-Control file-use' if file_use else 'locked-startup')+' page and current-version reopen',languages=languages,
                recovery_report_unchanged=unchanged,gateway_ready_after_reopen=True)


def execute(request, run, product, pm, helper, activity, manifest, root):
    if activity.get('administrator_groups') is not False:
        raise ValueError('Native Desktop evidence requires a standard account, not a filtered administrator')
    action = request['desktop_action']
    if action not in ('install', 'installer-resume', 'installer-quarantine', 'launch', 'snapshot', 'invoke', 'text', 'click', 'scroll', 'close', 'quit', 'tray-observe', 'desktop-state', 'cli', 'native-files', 'codex-call', 'codex-inference', 'codex-two-turn', 'network-probe', 'recovery-lock-ui', 'recovery-file-use-ui'):
        raise ValueError('Unknown native Desktop action')
    if action == 'recovery-lock-ui':
        return locked_startup_ui(request,run,product,pm,helper,activity,root)
    if action == 'recovery-file-use-ui':
        return locked_startup_ui(request,run,product,pm,helper,activity,root,file_use=True)
    if action == 'desktop-state':
        return dict(native_entry='read-only input desktop inspection', state=desktop_state())
    if action == 'native-files':
        files = {}
        for relative in ('.codex/config.toml', '.codex/hiroute.config.toml', '.claude/settings.json',
                         '.agents/skills/hiroute-collaboration', '.claude/skills/hiroute-collaboration',
                         '.agents/skills/hiroute-management', '.claude/skills/hiroute-management'):
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
        # Compare these before/after an upgrade or restoration without invoking
        # Codex in between. Calls themselves legitimately append native history.
        for relative in ('.codex/sessions','.codex/history.jsonl'):
            path=helper.plain(Path.home()/relative)
            if not path.exists():files[relative]=dict(exists=False)
            elif path.is_dir():files[relative]=dict(exists=True,inventory=helper.inventory(path))
            else:files[relative]=dict(exists=True,sha256=helper.digest(path),bytes=path.stat().st_size)
        (run / 'native-files.json').write_text(json.dumps(files, indent=2), encoding='utf-8')
        return dict(native_entry='read-only native configuration inventory', files=files)
    if action == 'installer-resume':
        return installer_resume(request, run, pm, helper, activity, manifest, root)
    if action == 'installer-quarantine':
        facts, programs, _ = prepared_installer_replacement(request, pm, helper, activity, manifest, root)
        return quarantine_generated_uninstaller(programs, Path(facts['shell_state']['path']), run,
            request.get('generated_uninstaller_sha256'), facts['install-pending.json']['record'], helper)
    if action == 'install':
        admit_nsis_install(request, helper, activity, manifest, root)
        previous = request.get('from_product')
        timing = nsis_install(product / 'installer.exe', run,
                              helper.locations(Path.home(), Path(os.environ['LOCALAPPDATA'])))
        _, after, _ = installed(helper, product, pm)
        return dict(native_entry='NSIS /S', exit_code=timing['exit_code'], installation=after,
                    elapsed_seconds=timing['elapsed_seconds'],
                    upgrade_from=previous, recovery_from=request.get('recover_product'),
                    installer_sha256=pm['files']['installer.exe']['sha256'])
    roots, facts, shell = installed(helper, product, pm)
    if action == 'network-probe':
        return network_probe(request, run, roots)
    if action in ('codex-call', 'codex-inference', 'codex-two-turn'):
        return codex_call(run, request, helper)
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
                or args[0] not in ('system', 'service', 'agents', 'agent', 'plans', 'routing', 'gateway', 'compute', 'operations', 'console', 'sessions')):
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
    if action in ('invoke', 'text', 'click', 'scroll'):
        parent = root / 'runs' / request['observation_run']
        old_request = json.loads((parent / 'request.json').read_text(encoding='utf-8'))
        old_result = json.loads((parent / 'result.json').read_text(encoding='utf-8'))
        observed = json.loads((parent / 'ui.json').read_text(encoding='utf-8'))
        if (not old_result.get('passed') or old_request['product'] != request['product']
                or old_request['config']['session_id'] != request['config']['session_id']
                or observed['pid'] != pid or time.time() - (parent / 'ui.json').stat().st_mtime > 600):
            raise ValueError('A recent observation of this exact Desktop process is required')
        if action not in ('click', 'scroll'):
            matches = [e for e in observed['nodes'] if e['runtime_id'] == request['element']]
            if not matches:
                raise ValueError('Choose one observed native UI element')
            element = matches[0]
    return dict(native_entry='Windows UI Automation', observation=ui_action(run, request, shell, pid, element, observed),
                shell_sha256=helper.digest(shell))
