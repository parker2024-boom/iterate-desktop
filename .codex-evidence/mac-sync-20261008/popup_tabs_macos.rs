//! AppKit presentation for existing popup processes. Notifications only wake
//! readers; request ownership and selection remain in the locked tab group.
use core_foundation::{base::{CFType, TCFType}, dictionary::{CFDictionary, CFDictionaryRef}, number::CFNumber, string::CFString};
use objc::{class, msg_send, sel, sel_impl, runtime::Object};
use serde::{Deserialize, Serialize};
use std::{ffi::CString, sync::mpsc, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct FocusGuard { source_pid: u32, input_counter: u32 }
impl FocusGuard {
    pub(super) fn new(source_pid: u32) -> Self { Self { source_pid, input_counter: input_counter() } }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" { fn CGEventSourceCounterForEventType(state: i32, event: u32) -> u32; }
unsafe extern "C" {
    fn notify_register_file_descriptor(name: *const libc::c_char, fd: *mut i32, flags: i32, token: *mut i32) -> u32;
    fn notify_post(name: *const libc::c_char) -> u32;
    fn notify_cancel(token: i32) -> u32;
}
pub(super) fn input_counter() -> u32 { unsafe { CGEventSourceCounterForEventType(0, u32::MAX) } }
fn name(pid: u32) -> CString { CString::new(format!("dev.iterate.popup-tabs.{}.{}", unsafe { libc::getuid() }, pid)).unwrap() }
pub(super) fn wake(pid: u32) { unsafe { notify_post(name(pid).as_ptr()); } }
pub(super) struct Wake { fd: i32, token: i32 }
impl Wake {
    pub(super) fn new() -> Option<Self> {
        let (mut fd, mut token) = (-1, 0);
        (unsafe { notify_register_file_descriptor(name(std::process::id()).as_ptr(), &mut fd, 0, &mut token) } == 0)
            .then_some(Self { fd, token })
    }
    pub(super) fn wait(&self) {
        let mut poll = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut poll, 1, 300) } > 0 && poll.revents & libc::POLLIN != 0 {
            let mut token = 0u32;
            unsafe { libc::read(self.fd, &mut token as *mut u32 as *mut libc::c_void, 4); }
        }
    }
}
impl Drop for Wake { fn drop(&mut self) { unsafe { notify_cancel(self.token); libc::close(self.fd); } } }

pub(super) fn visible(pid: u32) -> bool {
    let Some(windows) = core_graphics::window::copy_window_info(1, 0) else { return false };
    windows.iter().any(|item| {
        let row: CFDictionary<CFString, CFType> = unsafe { TCFType::wrap_under_get_rule(*item as CFDictionaryRef) };
        let number = |key: &str| row.find(&CFString::new(key)).and_then(|n| n.downcast::<CFNumber>()).and_then(|n| n.to_i64());
        // AppKit also publishes an application menu bar at layer zero. It can
        // survive orderOut, so only a full popup can confirm the handoff.
        let popup = row.find(&CFString::new("kCGWindowBounds"))
            .filter(|v| v.type_of() == CFDictionary::<CFString, CFType>::type_id())
            .map(|v| unsafe { CFDictionary::<CFString, CFType>::wrap_under_get_rule(v.as_CFTypeRef() as CFDictionaryRef) })
            .is_some_and(|bounds| ["Width", "Height"].iter().all(|key| {
                bounds.find(&CFString::new(key)).and_then(|n| n.downcast::<CFNumber>())
                    .and_then(|n| n.to_f64()).is_some_and(|size| size > 100.0)
            }));
        number("kCGWindowOwnerPID") == Some(pid as i64)
            && number("kCGWindowLayer").is_some_and(|layer| (0..=5).contains(&layer)) && popup
    })
}

fn on_main<T: Send + 'static>(window: &tauri::WebviewWindow, action: impl FnOnce(*mut Object) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let target = window.clone();
    let (tx, rx) = mpsc::channel();
    window.run_on_main_thread(move || {
        let result = target.ns_window().map_err(|e| e.to_string()).and_then(|ptr| action(ptr as *mut Object));
        let _ = tx.send(result);
    }).map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(2)).map_err(|e| format!("Mac 标签窗口未确认: {e}"))?
}
fn frontmost_pid() -> u32 {
    unsafe {
        let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let application: *mut Object = msg_send![workspace, frontmostApplication];
        if application.is_null() { 0 } else { msg_send![application, processIdentifier] }
    }
}
pub(super) fn foreground(window: &tauri::WebviewWindow) -> bool {
    on_main(window, |ns| Ok(frontmost_pid() == std::process::id() && unsafe { msg_send![ns, isKeyWindow] })).unwrap_or(false)
}
pub(super) fn show(window: &tauri::WebviewWindow, focus: bool, tabs: bool, guard: Option<FocusGuard>) -> Result<(), String> {
    on_main(window, move |ns| {
        let focus = focus && guard.as_ref().is_none_or(|guard| frontmost_pid() == guard.source_pid && input_counter() == guard.input_counter);
        unsafe {
            let _: () = msg_send![ns, setAnimationBehavior: if tabs { 2isize } else { 0isize }];
            if focus {
                let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app, activateIgnoringOtherApps: true];
                let _: () = msg_send![ns, makeKeyAndOrderFront: std::ptr::null_mut::<Object>()];
            } else { let _: () = msg_send![ns, orderFrontRegardless]; }
        }
        Ok(())
    })
}
pub(super) fn disable_animation(window: &tauri::WebviewWindow) -> Result<(), String> {
    on_main(window, |ns| { unsafe { let _: () = msg_send![ns, setAnimationBehavior: 2isize]; } Ok(()) })
}
pub(super) fn hide(window: &tauri::WebviewWindow) -> Result<(), String> {
    on_main(window, |ns| { unsafe {
        let _: () = msg_send![ns, setAnimationBehavior: 2isize];
        let _: () = msg_send![ns, orderOut: std::ptr::null_mut::<Object>()];
    } Ok(()) })
}
