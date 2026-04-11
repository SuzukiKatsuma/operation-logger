use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetWindowThreadProcessId, WindowFromPoint,
};

use crate::AppWindow;

use super::model::{
    ClientPoint, MouseInputEvent, MouseMoveEvent, RawMouseEvent, RawMouseEventKind,
    ResolvedMouseEvent,
};

#[derive(Debug, Clone, Copy)]
pub struct TargetResolver {
    target_hwnd: isize,
    target_process_id: u32,
}

impl TargetResolver {
    pub fn new(target: &AppWindow) -> Self {
        Self {
            target_hwnd: target.hwnd,
            target_process_id: target.process_id,
        }
    }

    pub fn resolve(&self, event: RawMouseEvent) -> Option<ResolvedMouseEvent> {
        if !self.is_target_process(event.screen_position.x, event.screen_position.y) {
            return None;
        }

        let position = self.client_position(event.screen_position.x, event.screen_position.y)?;

        match event.kind {
            RawMouseEventKind::Move => Some(ResolvedMouseEvent::Move(MouseMoveEvent { position })),
            RawMouseEventKind::Input {
                button,
                kind,
                delta,
            } => Some(ResolvedMouseEvent::Input(MouseInputEvent {
                position,
                button,
                kind,
                delta,
            })),
        }
    }

    fn is_target_process(&self, x: i32, y: i32) -> bool {
        // SAFETY: WindowFromPoint only consumes the POINT value and may return a
        // null HWND, which is checked before further use.
        let hwnd = unsafe { WindowFromPoint(POINT { x, y }) };
        if hwnd.0.is_null() {
            return false;
        }

        // SAFETY: hwnd is non-null and owned by the OS. GetAncestor may still
        // return null, which is handled by falling back to hwnd.
        let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
        let process_hwnd = if root.0.is_null() { hwnd } else { root };

        let mut process_id = 0u32;
        // SAFETY: process_hwnd is an OS-provided HWND, and process_id is a valid
        // out pointer for this call.
        unsafe {
            GetWindowThreadProcessId(process_hwnd, Some(&mut process_id as *mut u32));
        }

        process_id == self.target_process_id
    }

    fn client_position(&self, x: i32, y: i32) -> Option<ClientPoint> {
        let mut point = POINT { x, y };
        let hwnd = HWND(self.target_hwnd as *mut c_void);

        // SAFETY: hwnd is the selected AppWindow handle captured from Windows.
        // point is a valid mutable POINT for ScreenToClient to update.
        if unsafe { ScreenToClient(hwnd, &mut point).as_bool() } {
            Some(ClientPoint {
                x: point.x,
                y: point.y,
            })
        } else {
            None
        }
    }
}
