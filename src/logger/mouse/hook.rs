use std::io;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, HC_ACTION, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW,
    PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_MOUSE_LL,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
    WM_RBUTTONDOWN, WM_RBUTTONUP,
};

use super::model::{MouseButton, MouseInputKind, RawMouseEvent, RawMouseEventKind, ScreenPoint};

static MOUSE_EVENT_SENDER: OnceLock<Mutex<Option<Sender<RawMouseEvent>>>> = OnceLock::new();

pub struct MouseHook {
    thread_id: u32,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl MouseHook {
    pub fn start(sender: Sender<RawMouseEvent>) -> io::Result<Self> {
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread = thread::spawn(move || {
            // SAFETY: GetCurrentThreadId has no preconditions and returns the ID
            // of this newly spawned hook thread.
            let thread_id = unsafe { GetCurrentThreadId() };
            let mut msg = MSG::default();
            // SAFETY: msg points to initialized storage. This creates the
            // thread message queue before other threads post WM_QUIT to it.
            unsafe {
                let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE).as_bool();
            }

            set_sender(Some(sender));

            // SAFETY: mouse_hook_proc has the required system ABI and keeps
            // callback work minimal. A null module handle is valid for WH_MOUSE_LL
            // when installing a global low-level hook from this process.
            let hook =
                match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0) } {
                    Ok(hook) => hook,
                    Err(error) => {
                        set_sender(None);
                        let io_error = windows_error_to_io(error);
                        let _ = ready_tx
                            .send(Err(io::Error::new(io_error.kind(), io_error.to_string())));
                        return Err(io_error);
                    }
                };

            let _ = ready_tx.send(Ok(thread_id));

            loop {
                // SAFETY: msg points to valid storage for the lifetime of the
                // call. The loop exits on WM_QUIT or GetMessageW failure.
                let result = unsafe {
                    windows::Win32::UI::WindowsAndMessaging::GetMessageW(&mut msg, None, 0, 0)
                };
                if result.0 == -1 {
                    break;
                }
                if !result.as_bool() {
                    break;
                }

                // SAFETY: msg was filled by GetMessageW for this thread's queue.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            // SAFETY: hook was returned by SetWindowsHookExW in this thread and
            // has not been unhooked yet.
            let unhook_result = unsafe { UnhookWindowsHookEx(hook) };
            set_sender(None);

            unhook_result.map_err(windows_error_to_io)
        });

        let thread_id = ready_rx
            .recv()
            .map_err(|_| io::Error::other("mouse hook thread exited before startup"))??;

        Ok(Self {
            thread_id,
            thread: Some(thread),
        })
    }

    pub fn stop(mut self) -> io::Result<()> {
        self.request_stop()?;
        self.join()
    }

    fn request_stop(&self) -> io::Result<()> {
        // SAFETY: thread_id is captured from the hook thread after its message
        // queue is created. Posting WM_QUIT does not dereference Rust memory.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }
            .map_err(windows_error_to_io)
    }

    fn join(&mut self) -> io::Result<()> {
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| io::Error::other("mouse hook thread panicked"))?
        } else {
            Ok(())
        }
    }
}

impl Drop for MouseHook {
    fn drop(&mut self) {
        let _ = self.request_stop();
        let _ = self.join();
    }
}

// SAFETY: Windows calls this function with the WH_MOUSE_LL callback ABI. When
// code is HC_ACTION, lparam must point to an MSLLHOOKSTRUCT.
unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: For WH_MOUSE_LL with HC_ACTION, Windows documents lparam as
        // a valid pointer to MSLLHOOKSTRUCT for the duration of the callback.
        let hook_data = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if let Some(event) = raw_mouse_event(wparam.0 as u32, hook_data) {
            if let Some(sender) = MOUSE_EVENT_SENDER.get() {
                if let Ok(guard) = sender.try_lock() {
                    if let Some(sender) = guard.as_ref() {
                        let _ = sender.send(event);
                    }
                }
            }
        }
    }

    // SAFETY: Forwarding to the next hook is required. We pass through the
    // exact callback arguments received from Windows.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn raw_mouse_event(message: u32, hook_data: &MSLLHOOKSTRUCT) -> Option<RawMouseEvent> {
    let screen_position = ScreenPoint {
        x: hook_data.pt.x,
        y: hook_data.pt.y,
    };

    let kind = match message {
        WM_MOUSEMOVE => RawMouseEventKind::Move,
        WM_LBUTTONDOWN => RawMouseEventKind::Input {
            button: MouseButton::Left,
            kind: MouseInputKind::Down,
            delta: 0,
        },
        WM_LBUTTONUP => RawMouseEventKind::Input {
            button: MouseButton::Left,
            kind: MouseInputKind::Up,
            delta: 0,
        },
        WM_RBUTTONDOWN => RawMouseEventKind::Input {
            button: MouseButton::Right,
            kind: MouseInputKind::Down,
            delta: 0,
        },
        WM_RBUTTONUP => RawMouseEventKind::Input {
            button: MouseButton::Right,
            kind: MouseInputKind::Up,
            delta: 0,
        },
        WM_MOUSEWHEEL => RawMouseEventKind::Input {
            button: MouseButton::WheelVertical,
            kind: MouseInputKind::Wheel,
            delta: wheel_delta(hook_data.mouseData),
        },
        WM_MOUSEHWHEEL => RawMouseEventKind::Input {
            button: MouseButton::WheelHorizontal,
            kind: MouseInputKind::Wheel,
            delta: wheel_delta(hook_data.mouseData),
        },
        _ => return None,
    };

    Some(RawMouseEvent {
        screen_position,
        kind,
    })
}

fn wheel_delta(mouse_data: u32) -> i32 {
    ((mouse_data >> 16) as u16 as i16) as i32
}

fn set_sender(sender: Option<Sender<RawMouseEvent>>) {
    let storage = MOUSE_EVENT_SENDER.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = storage.lock() {
        *guard = sender;
    }
}

fn windows_error_to_io(error: windows::core::Error) -> io::Error {
    io::Error::other(error.to_string())
}
