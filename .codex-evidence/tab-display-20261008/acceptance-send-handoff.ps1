param([Parameter(Mandatory=$true)][string]$Executable,[string]$TaskDir='G:/DevLocal/BuildTemp/iterate-desktop/tab-display-20261008',[switch]$LeaveOpen,[switch]$ObserveOnly,[string]$LegacyTargetExecutable,[ValidateSet('success','failure','failure-after-edit')][string]$Scenario='success')
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
 [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
 public static void CtrlTab(){
  var inputs=new Input[]{
   new Input{type=1,data=new Data{key=new Key{vk=17}}},
   new Input{type=1,data=new Data{key=new Key{vk=9}}},
   new Input{type=1,data=new Data{key=new Key{vk=9,flags=2}}},
   new Input{type=1,data=new Data{key=new Key{vk=17,flags=2}}}
  };
  if(SendInput(4,inputs,Marshal.SizeOf(typeof(Input)))!=4)throw new Exception("Ctrl+Tab input injection failed");
 }
 public static void PlainTab(){
  var inputs=new Input[]{new Input{type=1,data=new Data{key=new Key{vk=9}}},new Input{type=1,data=new Data{key=new Key{vk=9,flags=2}}}};
  if(SendInput(2,inputs,Marshal.SizeOf(typeof(Input)))!=2)throw new Exception("Tab input injection failed");
 }
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
 [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr hwnd,uint flags);
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

public sealed class TabTransitionMonitor {
 [DllImport("winmm.dll")] static extern uint timeBeginPeriod(uint period);
 [DllImport("winmm.dll")] static extern uint timeEndPeriod(uint period);
 public sealed class Sample { public double milliseconds; public bool source_visible,target_visible; public long foreground; }
 readonly IntPtr source,target;
 readonly System.Diagnostics.Stopwatch clock=System.Diagnostics.Stopwatch.StartNew();
 readonly List<Sample> samples=new List<Sample>();
 readonly System.Threading.Thread thread;
 volatile bool running=true;
 public TabTransitionMonitor(long a,long b) {
  timeBeginPeriod(1);
  source=new IntPtr(a);target=new IntPtr(b);
  thread=new System.Threading.Thread(()=> {
   while(running) {
    samples.Add(new Sample { milliseconds=clock.Elapsed.TotalMilliseconds,
      source_visible=TabNative.IsWindowVisible(source),target_visible=TabNative.IsWindowVisible(target),foreground=TabNative.GetForegroundWindow().ToInt64() });
    System.Threading.Thread.Sleep(2);
   }
  });
  thread.IsBackground=true;thread.Start();
 }
 public Sample[] Stop(){running=false;thread.Join();timeEndPeriod(1);return samples.ToArray();}
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
$report=@{result='Blocked';steps=@();transitions=@();artifact=$Executable;sha256=(Get-FileHash $Executable).Hash;root=$root}
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
 $stable=@{rect=$null}
 WaitFor {
  $rect=$element.Current.BoundingRectangle.ToString()
  if($stable.rect-eq $rect){return $true}
  $stable.rect=$rect
  return $null
 } 5 | Out-Null
 $r=$element.Current.BoundingRectangle
 $point=[TabNative+Point]::new([int]($r.X+$r.Width/2),[int]($r.Y+$r.Height/2))
 $under=[TabNative]::GetAncestor([TabNative]::WindowFromPoint($point),2)
 $underPid=[uint32]0
 [void][TabNative]::GetWindowThreadProcessId($under,[ref]$underPid)
 if($processes.Id-notcontains $underPid -or $under-ne [TabNative]::GetForegroundWindow()){
  WriteJson "$root/click-blocked.json" @{name=$element.Current.Name;rect=$r.ToString();under=$under.ToInt64();under_pid=$underPid;foreground=[TabNative]::GetForegroundWindow().ToInt64()}
  throw 'Physical click is blocked by another window; no click issued'
 }
 [void][TabNative]::SetCursorPos($point.x,$point.y)
 [TabNative]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
 [TabNative]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
 Start-Sleep -Milliseconds 150
}
function Focus($hwnd,$edit){
 [void][TabNative]::SetWindowPos([IntPtr]$hwnd,[IntPtr]::Zero,0,0,0,0,0x13)
 $edge=$edit.Current.BoundingRectangle
 [void][TabNative]::SetCursorPos([int]($edge.Right-8),[int]($edge.Y+$edge.Height/2))
 [TabNative]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
 [TabNative]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
 Start-Sleep -Milliseconds 150
 [void][TabNative]::SetForegroundWindow([IntPtr]$hwnd)
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
 $launchExecutable=if($i-eq 1 -and $LegacyTargetExecutable){$LegacyTargetExecutable}else{$Executable}
 $p=Start-Process $launchExecutable -ArgumentList @('--mcp-request',$request) -PassThru -WindowStyle Hidden -RedirectStandardOutput "$root/stdout-$i.log" -RedirectStandardError "$root/stderr-$i.log"
 $script:processes+=$p
 $hwnd=WaitFor {$h=[TabNative]::Window($p.Id);if($h-ne [IntPtr]::Zero){$h.ToInt64()}}
 $script:windows+=$hwnd
 WaitFor {(ReadTabGroup).tabs | Where-Object {$_.pid-eq $p.Id}} | Out-Null
}
function SettingsMode($hwnd,$mode){
 Focus $hwnd (WaitFor {Editor $hwnd})
 Click (WaitFor {Named $hwnd '打开设置'})
 Start-Sleep -Milliseconds 600
 if(-not (Named $hwnd '返回聊天') -and (Named $hwnd '打开设置')){
  Click (Named $hwnd '打开设置')
 }
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
 $stable=@{position=$null}
 $backButton=WaitFor {
  if(([bool](TabControl $hwnd 'Tab acceptance 0'))-ne ($mode-eq '标签页')){return $null}
  $button=Named $hwnd '返回聊天'
  if(-not $button){return $null}
  $position=$button.Current.BoundingRectangle.ToString()
  if($stable.position-eq $position){return $button}
  $stable.position=$position
  return $null
 }
 Click $backButton
 WaitFor {Editor $hwnd} | Out-Null
}
function Screenshot($hwnd,$file){
 $r=(Root $hwnd).Current.BoundingRectangle
 $bitmap=[Drawing.Bitmap]::new([int]$r.Width,[int]$r.Height)
 $graphics=[Drawing.Graphics]::FromImage($bitmap)
 try{$graphics.CopyFromScreen([int]$r.X,[int]$r.Y,0,0,$bitmap.Size);$bitmap.Save($file,[Drawing.Imaging.ImageFormat]::Png)}finally{$graphics.Dispose();$bitmap.Dispose()}
}

function SwitchMeasured($from,$to,[switch]$Keyboard){
 Assert ([TabNative]::GetForegroundWindow().ToInt64()-eq $windows[$from]) 'Switch source is not foreground; do not click another window'
 $button=WaitFor {TabControl $windows[$from] "Tab acceptance $to"}
 $monitor=[TabTransitionMonitor]::new($windows[$from],$windows[$to])
 try {
  if($Keyboard){[TabNative]::CtrlTab()}else{Click $button}
  WaitFor {[TabNative]::IsWindowVisible([IntPtr]$windows[$to]) -and (VisibleCount)-eq 1} | Out-Null
  WaitFor {[TabNative]::GetForegroundWindow().ToInt64()-eq $windows[$to]} 5 | Out-Null
 } finally {$samples=$monitor.Stop()}
 $transitionIndex=@($report.transitions).Count
 WriteJson "$root/transition-$transitionIndex.json" @($samples)
 $gaps=@($samples|Where-Object {-not $_.source_visible -and -not $_.target_visible})
 $gapMs=0.0
 if($gaps.Count){$gapMs=$gaps[-1].milliseconds-$gaps[0].milliseconds+2}
 $report.transitions+=@{from=$from;to=$to;input=$(if($Keyboard){'Ctrl+Tab'}else{'click'});samples=$samples.Count;exposed_samples=$gaps.Count;exposed_ms=$gapMs;result=$(if($gaps.Count){'Fail'}else{'Pass'})}
 Assert ($samples.Count-ge 5) 'Transition monitor did not collect enough samples'
 if(-not $ObserveOnly){Assert ($gaps.Count-eq 0) "Tab switch $from->$to exposed the desktop for $gapMs ms"}
}

$deliveryOwner=$null
try {
 $configPath="$root/config/config.json"
 $config=[IO.File]::ReadAllText($configPath)|ConvertFrom-Json
 $config.ui_config.window_config|Add-Member -NotePropertyName popup_display_mode -NotePropertyValue 'tabs'
 WriteJson $configPath $config
 $deliveryPath="$root/delivery-state"
 [IO.File]::WriteAllText($deliveryPath,'w',$utf8)
 $deliveryOwner=[IO.FileStream]::new("$deliveryPath.lock",[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::ReadWrite)
 $deliveryOwner.Lock(0,[long]::MaxValue)
 $env:ITERATE_DELIVERY_FILE=$deliveryPath
 Launch 0
 WaitFor {[TabNative]::IsWindowVisible([IntPtr]$windows[0])}|Out-Null
 $edit=WaitFor {Editor $windows[0]}
 $workArea=[Windows.Forms.Screen]::PrimaryScreen.WorkingArea
 $ownedRect=(Root $windows[0]).Current.BoundingRectangle
 $testX=[Math]::Max($workArea.Left,$workArea.Right-[int]$ownedRect.Width-24)
 [void][TabNative]::SetWindowPos([IntPtr]$windows[0],[IntPtr]::Zero,$testX,($workArea.Top+46),0,0,0x15)
 Focus $windows[0] $edit
 [TabNative]::Type('send-handoff-diagnostic')
 WaitFor {(Value $edit)-eq 'send-handoff-diagnostic'} 5|Out-Null
 Remove-Item -LiteralPath Env:ITERATE_DELIVERY_FILE
 Launch 1
 WaitFor {(ReadTabGroup).tabs.Count-eq 2}|Out-Null
 Assert (-not [TabNative]::IsWindowVisible([IntPtr]$windows[1])) 'Background target unexpectedly visible'
 Focus $windows[0] (Editor $windows[0])
 $sendButton=WaitFor {Named $windows[0] '发送'}
 $monitor=[TabTransitionMonitor]::new($windows[0],$windows[1])
 try {
  Click $sendButton
  WaitFor {Test-Path "$root/response-0.json"} 10|Out-Null
  Assert ([IO.File]::ReadAllText($deliveryPath)-eq 'w') 'Fixture ACK was not deferred'
  Assert (-not $processes[0].HasExited -and (ReadTabGroup).tabs.Count-eq 2) 'Source completed before real ACK'
  Assert ((ReadTabGroup).active_pid-eq $processes[1].Id) 'Existing successor did not take over during pending ACK'
  Assert ([TabNative]::IsWindowVisible([IntPtr]$windows[1]) -and [TabNative]::GetForegroundWindow().ToInt64()-eq $windows[1]) 'Existing target did not show and receive foreground before ACK'
  $report.steps+=@{step='existing-target-ready-before-ack-source-still-owned';result='Pass'}
  if($Scenario-ne 'failure'){
   $targetEditor=WaitFor {Editor $windows[1]}
   Focus $windows[1] $targetEditor
   [TabNative]::Type('target-edited-before-ack')
   WaitFor {(Value $targetEditor)-eq 'target-edited-before-ack'} 5|Out-Null
   $r=$targetEditor.Current.BoundingRectangle
   [void][TabNative]::SetCursorPos([int]($r.Right-12),[int]($r.Y+$r.Height/2))
   $report.steps+=@{step='can-edit-existing-target-and-move-mouse-before-ack';result='Pass'}
  }
  Start-Sleep -Milliseconds 650
  if($Scenario-eq 'success'){
   [IO.File]::WriteAllText($deliveryPath,'r',$utf8)
   WaitFor {$processes[0].HasExited -and (ReadTabGroup).tabs.Count-eq 1} 10|Out-Null
   Assert ((ReadTabGroup).active_pid-eq $processes[1].Id) 'ACK skipped selected successor'
   Assert ([TabNative]::GetForegroundWindow().ToInt64()-eq $windows[1]) 'ACK disturbed target foreground'
   Assert ((Value (Editor $windows[1]))-eq 'target-edited-before-ack') 'ACK reset target draft'
   $report.steps+=@{step='ack-removes-only-source-target-draft-and-focus-preserved';result='Pass'}
  }else{
   [IO.File]::WriteAllText($deliveryPath,'f',$utf8)
   if($Scenario-eq 'failure'){
    WaitFor {(ReadTabGroup).active_pid-eq $processes[0].Id -and (VisibleCount)-eq 1 -and [TabNative]::IsWindowVisible([IntPtr]$windows[0])} 10|Out-Null
    Assert ((Value (Editor $windows[0]))-eq 'send-handoff-diagnostic') 'Failure lost source draft'
    Assert ([TabNative]::GetForegroundWindow().ToInt64()-eq $windows[0]) 'Failure did not restore source foreground'
    $report.steps+=@{step='failed-send-restores-source-draft-and-tab';result='Pass'}
   }else{
    Start-Sleep -Milliseconds 750
    Assert ((ReadTabGroup).active_pid-eq $processes[1].Id -and [TabNative]::GetForegroundWindow().ToInt64()-eq $windows[1]) 'Failure took over the edited peer'
    Assert ((Value (Editor $windows[1]))-eq 'target-edited-before-ack') 'Failure changed peer draft'
    $report.steps+=@{step='failed-send-respects-edited-peer-focus';result='Pass'}
   }
   Assert (-not $processes[0].HasExited -and (ReadTabGroup).tabs.Count-eq 2) 'Failure discarded source request'
  }
 }finally{$samples=$monitor.Stop()}
 WriteJson "$root/send-transition.json" @($samples)
 $response=[IO.File]::ReadAllText("$root/response-0.json")|ConvertFrom-Json
 Assert ($response.user_input-eq 'send-handoff-diagnostic' -and $response.metadata.actual_request_id-eq 'tabs-acceptance-0') 'Reply request ownership changed'
 $hidden=@($samples|Where-Object {-not $_.source_visible})
 $shown=@($samples|Where-Object target_visible)
 $gaps=@($samples|Where-Object {-not $_.source_visible -and -not $_.target_visible})
 $targetFocus=@($samples|Where-Object {$_.foreground-eq $windows[1]})
 $hideTime=if($hidden.Count){$hidden[0].milliseconds}else{$null}
 $showTime=if($shown.Count){$shown[0].milliseconds}else{$null}
 $report.send_probe=@{scenario=$Scenario;source_pid=$processes[0].Id;target_pid=$processes[1].Id;source_hwnd=$windows[0];target_hwnd=$windows[1];samples=$samples.Count;source_first_hidden_ms=$hideTime;target_first_shown_ms=$showTime;hidden_to_shown_ms=$(if($null-ne $hideTime -and $null-ne $showTime){$showTime-$hideTime}else{$null});both_hidden_samples=$gaps.Count;target_foreground_samples=$targetFocus.Count;target_became_foreground=[bool]$targetFocus.Count;request_id=$response.metadata.actual_request_id;target_process_preserved=(-not $processes[1].HasExited);legacy_target_sha256=$(if($LegacyTargetExecutable){(Get-FileHash $LegacyTargetExecutable).Hash}else{$null})}
 Assert ($gaps.Count-eq 0) 'Sending exposed a both-hidden window gap'
 Assert ($shown.Count-gt 0 -and $hidden.Count-gt 0 -and $showTime-le $hideTime) 'Source hid before existing target appeared'
 if($Scenario-eq 'failure-after-edit'){
  SwitchMeasured 1 0
  Assert ((Value (Editor $windows[0]))-eq 'send-handoff-diagnostic') 'Returning to failed source did not preserve draft'
  $report.steps+=@{step='failed-source-remains-selectable-with-original-draft';result='Pass'}
 }
 $report.result='Pass'
 $report.steps+=@{step='continuous-native-visibility-request-id-and-target-process-preserved';result='Pass'}
}catch{
 $report.result='Fail';$report.error=$_.Exception.ToString();$report.location=$_.ScriptStackTrace
 if($samples){WriteJson "$root/send-transition.json" @($samples)}
 $foreground=[TabNative]::GetForegroundWindow();$foregroundPid=[uint32]0
 [void][TabNative]::GetWindowThreadProcessId($foreground,[ref]$foregroundPid)
 $report.foreground=@{hwnd=$foreground.ToInt64();pid=$foregroundPid;owned_pids=@($processes.Id);owned_windows=@($windows)}
 foreach($h in $windows){if([TabNative]::IsWindowVisible([IntPtr]$h)){try{Screenshot $h "$root/failure-screen-$h.png"}catch{}}}
}finally{
 WriteJson "$root/result.json" $report
 foreach($p in $processes){if(-not $p.HasExited){Stop-Process -Id $p.Id -ErrorAction SilentlyContinue}}
 if($deliveryOwner){$deliveryOwner.Dispose()}
 $report|ConvertTo-Json -Depth 15
}
if($report.result-ne 'Pass'){exit 1}