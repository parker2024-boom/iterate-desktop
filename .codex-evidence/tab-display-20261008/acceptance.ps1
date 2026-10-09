param([Parameter(Mandatory=$true)][string]$Executable,[string]$TaskDir='G:/DevLocal/BuildTemp/iterate-desktop/tab-display-20261008',[switch]$LeaveOpen)
$ErrorActionPreference='Stop'
New-Item -ItemType Directory -Path "$TaskDir/temp" -Force | Out-Null
$env:TEMP="$TaskDir/temp"
$env:TMP=$env:TEMP
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Windows.Forms,System.Drawing
Add-Type @'
using System;
using System.Collections.Generic;
using System.Text;
using System.Runtime.InteropServices;
public static class TabNative {
 [StructLayout(LayoutKind.Sequential)] public struct Key {public ushort vk,scan;public uint flags,time;public UIntPtr extra;}
 [StructLayout(LayoutKind.Explicit,Size=32)] public struct Data {[FieldOffset(0)]public Key key;}
 [StructLayout(LayoutKind.Sequential)] public struct Input {public uint type;public Data data;}
 [DllImport("user32.dll")] public static extern uint SendInput(uint count,Input[] inputs,int size);
 public static void Type(string value){
  var inputs=new List<Input>();
  foreach(char c in value){foreach(uint flags in new uint[]{4,6})inputs.Add(new Input{type=1,data=new Data{key=new Key{scan=c,flags=flags}}});}
  if(SendInput((uint)inputs.Count,inputs.ToArray(),Marshal.SizeOf(typeof(Input)))!=inputs.Count)throw new Exception("Unicode input injection failed");
 }
 public delegate bool EnumProc(IntPtr hwnd, IntPtr data);
 [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, IntPtr data);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
 [StructLayout(LayoutKind.Sequential)] public struct Point {public int x,y;public Point(int px,int py){x=px;y=py;}}
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd,IntPtr after,int x,int y,int cx,int cy,uint flags);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd,StringBuilder name,int size);
 public static IntPtr Window(int target) {
  IntPtr result=IntPtr.Zero;
  EnumWindows((h,d)=>{uint pid;GetWindowThreadProcessId(h,out pid);
   var name=new StringBuilder(128);GetClassName(h,name,128);
   if(pid==target && name.ToString()=="Tauri Window"){result=h;return false;}return true;},IntPtr.Zero);
  return result;
 }
}
'@
$root=Join-Path $TaskDir ('native-acceptance-'+(Get-Date -Format 'HHmmss'))
New-Item -ItemType Directory -Path $root,"$root/config","$root/home","$root/temp" | Out-Null
$env:TEMP="$root/temp"
$env:TMP=$env:TEMP
$env:ITERATE_CONFIG_DIR="$root/config"
$env:ITERATE_CROSS_DEVICE_DIR="$root/cross-device"
$env:ITERATE_CONVERSATION_STATE_FILE="$root/conversation.json"
$env:WEBVIEW2_USER_DATA_FOLDER="$root/webview"
$env:ITERATE_LOG_FILE="$root/popup.log"
$env:CODEX_HOME="$root/home"
$utf8=[Text.UTF8Encoding]::new($false)
function WriteJson($path,$value){[IO.File]::WriteAllText($path,($value|ConvertTo-Json -Depth 30),$utf8)}
WriteJson "$root/config/config.json" @{ui_config=@{always_on_top=$false;window_config=@{focus_popup_on_show=$false;fixed_width=600;fixed_height=900;fixed=$true;popup_placement='left'}};custom_prompt_config=@{enabled=$false;prompts=@()};reply_config=@{copy_submission_to_clipboard=$false}}
$processes=@()
$windows=@()
$report=@{result='Blocked';steps=@();artifact=$Executable;sha256=(Get-FileHash $Executable).Hash;root=$root}
function Assert($condition,$message){if(-not $condition){throw $message}}
function WaitFor([scriptblock]$probe,[int]$seconds=45){
 $end=(Get-Date).AddSeconds($seconds)
 do {$value=& $probe;if($value){return $value};Start-Sleep -Milliseconds 200}while((Get-Date)-lt $end)
 throw 'Timed out waiting for observable UI state'
}
function Root($hwnd){[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$hwnd)}
function Elements($hwnd){(Root $hwnd).FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)}
function Named($hwnd,$name){@(Elements $hwnd | Where-Object {$_.Current.Name -eq $name -and -not $_.Current.IsOffscreen})[0]}
function Editor($hwnd){
 $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Edit)
 $edits=(Root $hwnd).FindAll([Windows.Automation.TreeScope]::Descendants,$condition)
 @($edits|Where-Object {-not $_.Current.IsOffscreen -and $_.Current.IsEnabled}|Sort-Object {$_.Current.BoundingRectangle.Height} -Descending)[0]
}
function Value($edit){($edit.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)).Current.Value}
function Click($element){
 Assert ($null-ne $element) 'Required visible control not found'
 $r=$element.Current.BoundingRectangle
 [void][TabNative]::SetCursorPos([int]($r.X+$r.Width/2),[int]($r.Y+$r.Height/2))
 [TabNative]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
 [TabNative]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
 Start-Sleep -Milliseconds 150
}
function Focus($hwnd,$edit){
 [void][TabNative]::SetWindowPos([IntPtr]$hwnd,[IntPtr]::Zero,0,0,0,0,0x13)
 Click $edit
 try {WaitFor {[TabNative]::GetForegroundWindow().ToInt64()-eq $hwnd} 5 | Out-Null}catch{
 $r=$edit.Current.BoundingRectangle;$foreground=[TabNative]::GetForegroundWindow();$ownerPid=[uint32]0;[void][TabNative]::GetWindowThreadProcessId($foreground,[ref]$ownerPid)
 WriteJson "$root/focus-failure.json" @{target=$hwnd;foreground=$foreground.ToInt64();foreground_pid=$ownerPid;edit_rect=$r.ToString();under_cursor=[TabNative]::WindowFromPoint([TabNative+Point]::new([int]($r.X+$r.Width/2),[int]($r.Y+$r.Height/2))).ToInt64()}
 Screenshot $hwnd "$root/focus-failure.png"
 throw 'Test setup could not focus owned popup'
 }
}
function ReadTabGroup{if(Test-Path "$root/temp/iterate_popup_tabs.json"){Get-Content "$root/temp/iterate_popup_tabs.json" -Raw -Encoding UTF8 | ConvertFrom-Json}}
function TabControl($hwnd,$title){@(Elements $hwnd | Where-Object {$_.Current.Name -match [regex]::Escape($title) -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::TabItem -and -not $_.Current.IsOffscreen})[0]}
function VisibleCount{ @($windows | Where-Object {[TabNative]::IsWindowVisible([IntPtr]$_)}).Count }
function Launch($i){
 $request="$root/request-$i.json"
 WriteJson $request @{id="tabs-acceptance-$i";message="Tab mode acceptance message $i";is_markdown=$true;predefined_options=@();project_path=$root;conversation_title="Tab acceptance $i"}
 $env:ITERATE_RESPONSE_FILE="$root/response-$i.json"
 $p=Start-Process $Executable -ArgumentList @('--mcp-request',$request) -PassThru -WindowStyle Hidden -RedirectStandardOutput "$root/stdout-$i.log" -RedirectStandardError "$root/stderr-$i.log"
 $script:processes+=$p
 $hwnd=WaitFor {$h=[TabNative]::Window($p.Id);if($h-ne [IntPtr]::Zero){$h.ToInt64()}}
 $script:windows+=$hwnd
 WaitFor {(ReadTabGroup).tabs | Where-Object {$_.pid-eq $p.Id}} | Out-Null
}
function SettingsMode($hwnd,$mode){
 Focus $hwnd (WaitFor {Editor $hwnd})
 Click (WaitFor {Named $hwnd '打开设置'})
 WaitFor {Named $hwnd '返回聊天'} | Out-Null
 for($attempt=0;$attempt-lt 15 -and -not (Named $hwnd '窗口设置');$attempt++){
  $r=(Root $hwnd).Current.BoundingRectangle
  [void][TabNative]::SetCursorPos([int]($r.X+$r.Width/2),[int]($r.Y+$r.Height/2))
  [TabNative]::mouse_event(0x800,0,0,4294966696,[UIntPtr]::Zero)
  Start-Sleep -Milliseconds 150
 }
 Click (WaitFor {Named $hwnd '窗口设置'})
 Click (WaitFor {Named $hwnd $mode})
 WaitFor {(ReadTabGroup).mode-eq $(if($mode-eq '标签页'){'tabs'}else{'windows'})} | Out-Null
 Click (WaitFor {Named $hwnd '返回聊天'})
 WaitFor {Editor $hwnd} | Out-Null
}
function Screenshot($hwnd,$file){
 $r=(Root $hwnd).Current.BoundingRectangle
 $bitmap=[Drawing.Bitmap]::new([int]$r.Width,[int]$r.Height)
 $graphics=[Drawing.Graphics]::FromImage($bitmap)
 try{$graphics.CopyFromScreen([int]$r.X,[int]$r.Y,0,0,$bitmap.Size);$bitmap.Save($file,[Drawing.Imaging.ImageFormat]::Png)}finally{$graphics.Dispose();$bitmap.Dispose()}
}
try {
 Launch 0
 WaitFor {[TabNative]::IsWindowVisible([IntPtr]$windows[0])} | Out-Null
 $first=WaitFor {Editor $windows[0]}
 Focus $windows[0] $first
 $initialRect=(Root $windows[0]).Current.BoundingRectangle
 [TabNative]::Type('abcdef')
 WaitFor {(Value $first)-eq 'abcdef'} 5 | Out-Null
 [Windows.Forms.SendKeys]::SendWait('{LEFT}{LEFT}{LEFT}')
 Launch 1
 WaitFor {(VisibleCount)-eq 2} | Out-Null
 Assert ((VisibleCount)-eq 2) 'Default separate-window mode did not show both popups'
 Assert ([TabNative]::GetForegroundWindow().ToInt64()-eq $windows[0]) 'Separate-window arrival stole foreground'
 $report.steps+=@{step='default-separate-windows';result='Pass'}
 SettingsMode $windows[0] '标签页'
 WaitFor {(VisibleCount)-eq 1} | Out-Null
 Assert ((ReadTabGroup).active_pid-eq $processes[0].Id) 'Enabling tabs did not keep the window used to change setting'
 Assert ((Value (Editor $windows[0]))-eq 'abcdef') 'Mode setting lost original draft'
 $rect=(Root $windows[0]).Current.BoundingRectangle
 Assert ($rect.Width-eq $initialRect.Width -and $rect.Height-eq $initialRect.Height) 'Enabling tabs changed existing popup size'
 $saved=Get-Content "$root/config/config.json" -Raw -Encoding UTF8 | ConvertFrom-Json
 Assert ($saved.ui_config.window_config.popup_display_mode-eq 'tabs') 'Display choice did not persist'
 $report.steps+=@{step='enable-tabs-through-settings-and-persist';result='Pass';width=$rect.Width;height=$rect.Height}
 Focus $windows[0] (Editor $windows[0])
 [Windows.Forms.SendKeys]::SendWait('{END}{LEFT}{LEFT}{LEFT}')
 Launch 2
 WaitFor {(ReadTabGroup).tabs.Count-eq 3} | Out-Null
 Start-Sleep -Milliseconds 800
 Assert ((VisibleCount)-eq 1) 'Background arrival showed an extra window'
 Assert ([TabNative]::GetForegroundWindow().ToInt64()-eq $windows[0]) 'Background tab stole foreground'
 [TabNative]::Type('X')
 WaitFor {(Value (Editor $windows[0]))-eq 'abcXdef'} 5 | Out-Null
 WriteJson "$root/controls.json" @(Elements $windows[0]|ForEach-Object {@{name=$_.Current.Name;type=$_.Current.ControlType.ProgrammaticName;offscreen=$_.Current.IsOffscreen}})
 $report.steps+=@{step='background-tab-keeps-window-focus-caret';result='Pass';draft='abcXdef'}
 Click (WaitFor {TabControl $windows[0] 'Tab acceptance 1'})
 WaitFor {[TabNative]::IsWindowVisible([IntPtr]$windows[1]) -and (VisibleCount)-eq 1} | Out-Null
 WaitFor {[TabNative]::GetForegroundWindow().ToInt64()-eq $windows[1]} 5 | Out-Null
 $second=WaitFor {Editor $windows[1]}
 Focus $windows[1] $second
 [TabNative]::Type('second-draft')
 WaitFor {(Value $second)-eq 'second-draft'} 5 | Out-Null
 Click (WaitFor {TabControl $windows[1] 'Tab acceptance 0'})
 WaitFor {[TabNative]::IsWindowVisible([IntPtr]$windows[0]) -and (VisibleCount)-eq 1} | Out-Null
 Assert ((Value (Editor $windows[0]))-eq 'abcXdef') 'Returning to first tab lost draft'
 Start-Sleep -Milliseconds 500
 Screenshot $windows[0] "$root/tabs.png"
 Click (WaitFor {TabControl $windows[0] 'Tab acceptance 1'})
 WaitFor {[TabNative]::IsWindowVisible([IntPtr]$windows[1])} | Out-Null
 Assert ((Value (Editor $windows[1]))-eq 'second-draft') 'Returning to second tab lost draft'
 $report.steps+=@{step='tab-switch-independent-drafts';result='Pass'}
 Click (WaitFor {Named $windows[1] '发送'})
 WaitFor {Test-Path "$root/response-1.json"} | Out-Null
 $response=Get-Content "$root/response-1.json" -Raw -Encoding UTF8 | ConvertFrom-Json
 Assert ($response.user_input-eq 'second-draft') 'Reply did not match selected draft'
 Assert ($response.metadata.actual_request_id-eq 'tabs-acceptance-1') 'Reply went to wrong request'
 WaitFor {(ReadTabGroup).tabs.Count-eq 2 -and [TabNative]::IsWindowVisible([IntPtr]$windows[0])} | Out-Null
 Assert ((Value (Editor $windows[0]))-eq 'abcXdef') 'Sending other tab changed original draft'
 $report.steps+=@{step='send-isolated-and-next-tab-visible';result='Pass';actual_request_id=$response.metadata.actual_request_id}
 SettingsMode $windows[0] '分窗'
 WaitFor {(VisibleCount)-eq 2} | Out-Null
 Assert ((Value (Editor $windows[0]))-eq 'abcXdef') 'Returning to separate windows lost draft'
 $report.steps+=@{step='return-to-separate-windows';result='Pass'}
 SettingsMode $windows[0] '标签页'
 Launch 3
 WaitFor {(ReadTabGroup).tabs.Count-eq 3} | Out-Null
 Assert ((ReadTabGroup).mode-eq 'tabs' -and (ReadTabGroup).active_pid-eq $processes[0].Id) 'New process did not load saved tab mode'
 Assert (-not [TabNative]::IsWindowVisible([IntPtr]$windows[3])) 'Saved mode failed for newly launched popup'
 $report.steps+=@{step='new-process-loads-saved-choice';result='Pass'}
 $report.result='Pass'
}catch{
 $report.result='Fail';$report.error=$_.Exception.ToString();$report.location=$_.ScriptStackTrace
 if(Test-Path "$root/temp/iterate_popup_tabs.json"){$report.group=ReadTabGroup}
 foreach($h in $windows){try{WriteJson "$root/failure-controls-$h.json" @(Elements $h | ForEach-Object {@{name=$_.Current.Name;type=$_.Current.ControlType.ProgrammaticName;offscreen=$_.Current.IsOffscreen}})}catch{}}
}finally{
 WriteJson "$root/result.json" $report
 if(-not $LeaveOpen -or $report.result-ne 'Pass'){foreach($p in $processes){if(-not $p.HasExited){Stop-Process -Id $p.Id -ErrorAction SilentlyContinue}}}
 $report|ConvertTo-Json -Depth 15
}
if($report.result-ne 'Pass'){exit 1}
