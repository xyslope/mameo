//! 名前付きMutexで二重起動を防ぎ、既存の設定ウィンドウを復元・前面化する。
use std::ffi::c_void;
use std::io;

type Handle = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(attributes: *const c_void, owner: i32, name: *const u16) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetLastError() -> u32;
    fn GetCurrentThreadId() -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> Handle;
    fn ShowWindowAsync(window: Handle, command: i32) -> i32;
    fn IsIconic(window: Handle) -> i32;
    fn SetForegroundWindow(window: Handle) -> i32;
    fn BringWindowToTop(window: Handle) -> i32;
    fn GetForegroundWindow() -> Handle;
    fn GetWindowThreadProcessId(window: Handle, process: *mut u32) -> u32;
    fn AllowSetForegroundWindow(process: u32) -> i32;
    fn AttachThreadInput(source: u32, target: u32, attach: i32) -> i32;
    fn PeekMessageW(
        message: *mut NativeMessage,
        window: Handle,
        min: u32,
        max: u32,
        remove: u32,
    ) -> i32;
}

#[repr(C)]
struct NativeMessage {
    window: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point: [i32; 2],
}

unsafe fn focus_window(window: Handle) {
    if SetForegroundWindow(window) != 0 {
        return;
    }
    // トレイ等からの起動でも、前面スレッドの入力キューに一時的に接続してフォーカスを移す。
    // 二つ目のプロセスにはまだGUIキューがないため、PeekMessageWで先に作る。
    let mut message: NativeMessage = std::mem::zeroed();
    PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, 0);
    let foreground = GetForegroundWindow();
    let foreground_thread = GetWindowThreadProcessId(foreground, std::ptr::null_mut());
    let current_thread = GetCurrentThreadId();
    let target_thread = GetWindowThreadProcessId(window, std::ptr::null_mut());
    let attached = foreground_thread != 0
        && foreground_thread != current_thread
        && AttachThreadInput(current_thread, foreground_thread, 1) != 0;
    let target_attached = target_thread != 0
        && target_thread != current_thread
        && target_thread != foreground_thread
        && AttachThreadInput(current_thread, target_thread, 1) != 0;
    BringWindowToTop(window);
    SetForegroundWindow(window);
    if target_attached {
        AttachThreadInput(current_thread, target_thread, 0);
    }
    if attached {
        AttachThreadInput(current_thread, foreground_thread, 0);
    }
}

pub struct SingleInstance(Handle);

impl SingleInstance {
    pub fn acquire() -> io::Result<Option<Self>> {
        let instance = Self::try_acquire("Local\\mameo_config_single_instance_mutex")?;
        if instance.is_none() {
            activate_existing_window();
        }
        Ok(instance)
    }

    fn try_acquire(name: &str) -> io::Result<Option<Self>> {
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            // CreateMutexW成功直後のエラー値を、他のWin32呼び出しより先に読む。
            let error = GetLastError();
            if handle.is_null() {
                return Err(io::Error::from_raw_os_error(error as i32));
            }
            if error == 183 {
                CloseHandle(handle);
                return Ok(None);
            }
            Ok(Some(Self(handle)))
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn activate_existing_window() {
    let title: Vec<u16> = "mameo 設定".encode_utf16().chain(Some(0)).collect();
    // 起動直後にもう一度開いた場合、最初のウィンドウの生成を待つ。
    for _ in 0..50 {
        unsafe {
            let window = FindWindowW(std::ptr::null(), title.as_ptr());
            if !window.is_null() {
                let mut process = 0;
                GetWindowThreadProcessId(window, &mut process);
                AllowSetForegroundWindow(process);
                ShowWindowAsync(window, if IsIconic(window) != 0 { 9 } else { 5 });
                focus_window(window);
                if GetForegroundWindow() == window {
                    return;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_is_rejected_and_handle_is_released() {
        let name = format!("Local\\mameo_config_test_{}", std::process::id());
        let first = SingleInstance::try_acquire(&name).unwrap().unwrap();
        assert!(SingleInstance::try_acquire(&name).unwrap().is_none());
        drop(first);
        assert!(SingleInstance::try_acquire(&name).unwrap().is_some());
    }
}
