param([string]$TaskDir='G:/DevLocal/BuildTemp/iterate-desktop/popup-title-dnd-20261008',[string]$Executable,[switch]$SkipTitle)
$ErrorActionPreference='Stop'
if(-not $Executable){$Executable="$TaskDir/iterate.exe"}
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Windows.Forms
Add-Type @'
using System;
using System.Collections.Generic;
using System.Text;
using System.Runtime.InteropServices;
public static class PopupNative {
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
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd,IntPtr after,int x,int y,int cx,int cy,uint flags);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd,StringBuilder name,int size);
 public static long[] Order() {
  var result = new List<long>();
  EnumWindows((h,d)=>{if(IsWindowVisible(h))result.Add(h.ToInt64()); return true;},IntPtr.Zero);
  return result.ToArray();
 }
 public static IntPtr Window(int target) {
  IntPtr result=IntPtr.Zero;
  EnumWindows((h,d)=>{uint pid;GetWindowThreadProcessId(h,out pid);
   var name=new StringBuilder(128);GetClassName(h,name,128);
   if(pid==target && IsWindowVisible(h) && name.ToString()=="Tauri Window"){ result=h; return false;}return true;},IntPtr.Zero);
  return result;
 }
}
'@
$root=Join-Path $TaskDir ('acceptance-'+(Get-Date -Format 'HHmmss'))
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
WriteJson "$root/config/config.json" @{ui_config=@{always_on_top=$false;window_config=@{focus_popup_on_show=$false;fixed_width=600;fixed_height=900;window_fixed=$true;popup_placement='left'}};custom_prompt_config=@{enabled=$false;prompts=@()};reply_config=@{copy_submission_to_clipboard=$false}}
[IO.File]::WriteAllText("$root/home/session_index.jsonl",'{"id":"00000000-0000-4000-8000-000000000001","thread_name":"DND acceptance stable session"}'+"`n",$utf8)
$processes=@()
$windows=@()
$report=@{result='Blocked';steps=@();artifact=$Executable;sha256=(Get-FileHash $Executable).Hash;root=$root}
function Assert($condition,$message){if(-not $condition){throw $message}}
function WaitFor([scriptblock]$probe,[int]$seconds=50){
 $end=(Get-Date).AddSeconds($seconds)
 do { $value=& $probe; if($value){return $value}; Start-Sleep -Milliseconds 250 } while((Get-Date)-lt $end)
 throw 'Timed out waiting for observable UI state'
}
function Root($hwnd){[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$hwnd)}
function Editor($hwnd){
 $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Edit)
 $edits=(Root $hwnd).FindAll([Windows.Automation.TreeScope]::Descendants,$condition)
 @($edits|Where-Object {-not $_.Current.IsOffscreen -and $_.Current.IsEnabled}|Sort-Object {$_.Current.BoundingRectangle.Height} -Descending)[0]
}
function Value($edit){($edit.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)).Current.Value}
function Focus($hwnd,$edit){
 [void][PopupNative]::SetWindowPos([IntPtr]$hwnd,[IntPtr]::Zero,0,0,0,0,0x13)
 $rect=$edit.Current.BoundingRectangle
 [void][PopupNative]::SetCursorPos([int]($rect.X+50),[int]($rect.Y+$rect.Height/2))
 [PopupNative]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
 [PopupNative]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
 Start-Sleep -Milliseconds 200
 Assert ([PopupNative]::GetForegroundWindow().ToInt64()-eq $hwnd) 'Test setup could not focus owned popup'
}
try {
 for($i=0;$i -lt 3;$i++){
  $request="$root/request-$i.json"
  WriteJson $request @{id="test-dnd-$i";message="DND acceptance popup $i";is_markdown=$true;predefined_options=@();project_path=$root;conversation_title="Progress step $i";codex_home="$root/home";codex_thread_id='00000000-0000-4000-8000-000000000001';codex_thread_provenance='caller_meta'}
  $env:ITERATE_RESPONSE_FILE="$root/response-$i.json"
  $p=Start-Process $Executable -ArgumentList @('--mcp-request',$request) -PassThru -WindowStyle Hidden -RedirectStandardOutput "$root/stdout-$i.log" -RedirectStandardError "$root/stderr-$i.log"
  $processes+= $p
  $hwnd=WaitFor { $h=[PopupNative]::Window($p.Id); if($h-ne [IntPtr]::Zero){$h.ToInt64()} }
  $windows+= $hwnd
  $edit=WaitFor {Editor $hwnd}
  $names=@((Root $hwnd).FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)|ForEach-Object {$_.Current.Name})
  WriteJson "$root/controls-$i.json" $names
  if(-not $SkipTitle){Assert ($names -contains 'DND acceptance stable session') "Canonical title missing in popup $i"}
  if($i-eq 0){
   $first=$edit
   Focus $hwnd $edit
   [PopupNative]::Type('abcdef')
   WaitFor { (Value $first)-eq 'abcdef' } 5 | Out-Null
   [Windows.Forms.SendKeys]::SendWait('{LEFT}{LEFT}{LEFT}')
  }else{
   $order=[PopupNative]::Order()
   $indices=@($windows|ForEach-Object {[Array]::IndexOf($order,$_ )})
   Assert ([PopupNative]::GetForegroundWindow().ToInt64()-eq $windows[0]) "New popup $i stole focus"
   for($j=1;$j-lt $indices.Count;$j++){Assert ($indices[$j]-gt $indices[$j-1]) 'Existing popup order changed or new popup covered old popup'}
   [PopupNative]::Type('X')
   $expected=if($i-eq 1){'abcXdef'}else{'abcXXdef'}
   WaitFor { (Value $first)-eq $expected } 5 | Out-Null
   $report.steps+=@{step="new-popup-$i";result='Pass';foreground=[PopupNative]::GetForegroundWindow().ToInt64();order=$indices;draft=(Value $first);canonical_title=$true}
  }
 }
 Focus $windows[2] (Editor $windows[2])
 [PopupNative]::Type('new-popup-reply')
 WaitFor { (Value (Editor $windows[2]))-eq 'new-popup-reply' } 5 | Out-Null
 $buttons=(Root $windows[2]).FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Button))
 $sendLabel=[string][char]0x53D1+[char]0x9001
 $send=@($buttons|Where-Object {$_.Current.Name.StartsWith($sendLabel) -and $_.Current.IsEnabled})[0]
 Assert ($null-ne $send) 'Send button not found'
 ($send.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)).Invoke()
 WaitFor {Test-Path "$root/response-2.json"} | Out-Null
 $response=Get-Content "$root/response-2.json" -Raw | ConvertFrom-Json
 Assert ($response.user_input -eq 'new-popup-reply') 'Submitted reply did not match test input'
 Assert ($response.metadata.actual_request_id -eq 'test-dnd-2') 'Submitted reply has wrong request identity'
 Assert ((Value $first)-eq 'abcXXdef') 'Sending new popup changed old draft'
 $report.steps+=@{step='select-edit-send-new-popup';result='Pass';reply=$response.user_input;old_draft=(Value $first)}
 $report.result='Pass'
}catch{
 $report.result='Fail'
 $report.error=$_.Exception.ToString()
 $report.location=$_.ScriptStackTrace
 if($first){$report.draft=Value $first;$report.focused=[Windows.Automation.AutomationElement]::FocusedElement.Current.Name}
}finally{
 WriteJson "$root/result.json" $report
 foreach($p in $processes){if(-not $p.HasExited){Stop-Process -Id $p.Id -ErrorAction SilentlyContinue}}
 $report|ConvertTo-Json -Depth 12
}
if($report.result-ne 'Pass'){exit 1}
