use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::AppWindow;

#[derive(Debug, Clone, Copy)]
pub struct ForegroundResolver {
    target_process_id: u32,
}

impl ForegroundResolver {
    pub fn new(target: &AppWindow) -> Self {
        Self {
            target_process_id: target.process_id,
        }
    }

    pub fn is_target_foreground(&self) -> bool {
        self.foreground_process_id() == Some(self.target_process_id)
    }

    fn foreground_process_id(&self) -> Option<u32> {
        // SAFETY: GetForegroundWindow does not dereference Rust pointers and can
        // return a null HWND, which is handled below.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return None;
        }

        let mut process_id = 0u32;
        // SAFETY: hwnd came from the OS, and process_id points to a valid local
        // u32 for the duration of the call.
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut process_id as *mut u32));
        }

        if process_id == 0 {
            None
        } else {
            Some(process_id)
        }
    }
}
