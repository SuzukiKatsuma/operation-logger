use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWHID, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEHID, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClassInfoW, GetMessageW, MSG, PostThreadMessageW, RegisterClassW, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_INPUT, WM_QUIT, WNDCLASSW,
};
use windows::core::w;

use super::model::RawControllerReport;

static CONTROLLER_REPORT_SENDER: OnceLock<Mutex<Option<Sender<RawControllerReport>>>> =
    OnceLock::new();

const RAW_INPUT_WINDOW_CLASS: windows::core::PCWSTR = w!("OperationLoggerRawInputWindow");

pub(super) struct RawInputSession {
    thread_id: u32,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl RawInputSession {
    pub(super) fn start(sender: Sender<RawControllerReport>) -> io::Result<Self> {
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread = thread::spawn(move || {
            // SAFETY: GetCurrentThreadId has no preconditions and returns the ID
            // of this Raw Input thread.
            let thread_id = unsafe { GetCurrentThreadId() };
            set_sender(Some(sender));

            let result = run_raw_input_thread(thread_id, ready_tx);
            set_sender(None);
            result
        });

        let thread_id = ready_rx
            .recv()
            .map_err(|_| io::Error::other("controller raw input thread exited before startup"))??;

        Ok(Self {
            thread_id,
            thread: Some(thread),
        })
    }

    pub(super) fn stop(mut self) -> io::Result<()> {
        self.request_stop()?;
        self.join()
    }

    fn request_stop(&self) -> io::Result<()> {
        // SAFETY: thread_id is captured from the Raw Input thread after startup.
        // Posting WM_QUIT does not access Rust memory.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }
            .map_err(windows_error_to_io)
    }

    fn join(&mut self) -> io::Result<()> {
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| io::Error::other("controller raw input thread panicked"))?
        } else {
            Ok(())
        }
    }
}

impl Drop for RawInputSession {
    fn drop(&mut self) {
        let _ = self.request_stop();
        let _ = self.join();
    }
}

fn run_raw_input_thread(thread_id: u32, ready_tx: mpsc::Sender<io::Result<u32>>) -> io::Result<()> {
    let hinstance = current_hinstance()?;
    register_window_class(hinstance)?;

    // SAFETY: The class was registered in this process, all string pointers are
    // static, and no lpparam is passed. Failure is handled as an error.
    let hwnd = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            RAW_INPUT_WINDOW_CLASS,
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        )
    } {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let io_error = windows_error_to_io(error);
            let _ = ready_tx.send(Err(io::Error::new(io_error.kind(), io_error.to_string())));
            return Err(io_error);
        }
    };

    if let Err(error) = register_controller_raw_input(hwnd) {
        // SAFETY: hwnd was returned by CreateWindowExW and is still owned by
        // this thread.
        let _ = unsafe { DestroyWindow(hwnd) };
        let _ = ready_tx.send(Err(io::Error::new(error.kind(), error.to_string())));
        return Err(error);
    }

    let _ = ready_tx.send(Ok(thread_id));

    let mut msg = MSG::default();
    loop {
        // SAFETY: msg is valid storage for GetMessageW. We dispatch only
        // messages from this Raw Input thread's queue.
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if result.0 == -1 {
            break;
        }
        if !result.as_bool() {
            break;
        }

        // SAFETY: msg was populated by GetMessageW for this thread.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let _ = unregister_controller_raw_input();
    // SAFETY: hwnd was created on this thread and is destroyed once after the
    // message loop exits.
    unsafe { DestroyWindow(hwnd) }.map_err(windows_error_to_io)
}

// SAFETY: Windows calls this function with the window-procedure ABI for the
// hidden Raw Input window. WM_INPUT lparam is treated as HRAWINPUT only for that
// message and all other messages are forwarded to DefWindowProcW.
unsafe extern "system" fn raw_input_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        // SAFETY: For WM_INPUT, lparam is documented as an HRAWINPUT handle
        // valid for GetRawInputData during message processing.
        if let Some(report) = raw_controller_report(HRAWINPUT(lparam.0 as *mut c_void)) {
            if let Some(sender) = CONTROLLER_REPORT_SENDER.get() {
                if let Ok(guard) = sender.try_lock() {
                    if let Some(sender) = guard.as_ref() {
                        let _ = sender.send(report);
                    }
                }
            }
        }
    }

    // SAFETY: Messages not fully handled here are forwarded unchanged to the
    // default window procedure for this hidden window.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn raw_controller_report(raw_input_handle: HRAWINPUT) -> Option<RawControllerReport> {
    let mut size = 0u32;
    let header_size = size_of::<RAWINPUTHEADER>() as u32;
    // SAFETY: Passing None asks Windows for the required buffer size. size is a
    // valid out pointer, and header_size matches RAWINPUTHEADER.
    let query_result =
        unsafe { GetRawInputData(raw_input_handle, RID_INPUT, None, &mut size, header_size) };
    if query_result == u32::MAX || size == 0 {
        return None;
    }

    let mut buffer = vec![0u8; size as usize];
    // SAFETY: buffer has exactly the byte length requested by the previous
    // GetRawInputData call. Windows writes at most size bytes into it.
    let read_result = unsafe {
        GetRawInputData(
            raw_input_handle,
            RID_INPUT,
            Some(buffer.as_mut_ptr() as *mut c_void),
            &mut size,
            header_size,
        )
    };
    if read_result == u32::MAX || read_result != size {
        return None;
    }

    // SAFETY: buffer was filled by GetRawInputData with a RAWINPUT blob and is
    // at least size_of::<RAWINPUTHEADER>(). Alignment is acceptable for RAWINPUT
    // because Vec<u8> is only byte-aligned; use read_unaligned to avoid relying
    // on pointer alignment.
    let raw = unsafe { std::ptr::read_unaligned(buffer.as_ptr() as *const RAWINPUT) };
    if raw.header.dwType != RIM_TYPEHID.0 {
        return None;
    }

    // SAFETY: dwType was checked as RIM_TYPEHID, so the hid union field is the
    // active RAWINPUT payload variant.
    let hid = unsafe { raw.data.hid };
    let byte_len = hid.dwSizeHid.checked_mul(hid.dwCount)? as usize;
    if byte_len == 0 {
        return None;
    }

    let hid_offset = size_of::<RAWINPUTHEADER>() + size_of::<RAWHID>() - 1;
    if buffer.len() < hid_offset + byte_len {
        return None;
    }

    // SAFETY: hid_offset points to RAWHID::bRawData within the RAWINPUT buffer,
    // and the bounds check above proves byte_len bytes are present.
    let data_ptr = unsafe { buffer.as_ptr().add(hid_offset) };
    // SAFETY: data_ptr and byte_len were bounds-checked against buffer, and the
    // slice is copied immediately into an owned Vec.
    let report = unsafe { std::slice::from_raw_parts(data_ptr, byte_len) }.to_vec();

    Some(RawControllerReport {
        device_handle: raw.header.hDevice.0 as isize,
        report,
    })
}

fn register_controller_raw_input(hwnd: HWND) -> io::Result<()> {
    let devices = [
        raw_input_device(0x01, 0x04, RIDEV_INPUTSINK, hwnd),
        raw_input_device(0x01, 0x05, RIDEV_INPUTSINK, hwnd),
        raw_input_device(0x01, 0x08, RIDEV_INPUTSINK, hwnd),
    ];

    // SAFETY: devices is a valid slice of RAWINPUTDEVICE values for the duration
    // of the call, and cbsize matches the struct type.
    unsafe { RegisterRawInputDevices(&devices, size_of::<RAWINPUTDEVICE>() as u32) }
        .map_err(windows_error_to_io)
}

fn unregister_controller_raw_input() -> io::Result<()> {
    let devices = [
        raw_input_device(0x01, 0x04, RIDEV_REMOVE, HWND::default()),
        raw_input_device(0x01, 0x05, RIDEV_REMOVE, HWND::default()),
        raw_input_device(0x01, 0x08, RIDEV_REMOVE, HWND::default()),
    ];

    // SAFETY: devices is a valid slice of RAWINPUTDEVICE values for the duration
    // of the call, and RIDEV_REMOVE unregisters the usages for this process.
    unsafe { RegisterRawInputDevices(&devices, size_of::<RAWINPUTDEVICE>() as u32) }
        .map_err(windows_error_to_io)
}

fn raw_input_device(
    usage_page: u16,
    usage: u16,
    flags: windows::Win32::UI::Input::RAWINPUTDEVICE_FLAGS,
    hwnd: HWND,
) -> RAWINPUTDEVICE {
    RAWINPUTDEVICE {
        usUsagePage: usage_page,
        usUsage: usage,
        dwFlags: flags,
        hwndTarget: hwnd,
    }
}

fn register_window_class(hinstance: HINSTANCE) -> io::Result<()> {
    let mut existing = WNDCLASSW::default();
    // SAFETY: existing points to valid writable WNDCLASSW storage. The class
    // name is a static PCWSTR.
    if unsafe { GetClassInfoW(Some(hinstance), RAW_INPUT_WINDOW_CLASS, &mut existing).is_ok() } {
        return Ok(());
    }

    // WNDCLASSW is a plain Win32 POD. Zeroed optional handles are valid defaults.
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(raw_input_window_proc),
        hInstance: hinstance,
        lpszClassName: RAW_INPUT_WINDOW_CLASS,
        ..WNDCLASSW::default()
    };

    // SAFETY: class points to a valid WNDCLASSW with a static class name and a
    // window proc using the required system ABI.
    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        let error = windows::core::Error::from_thread();
        if error.code().is_err() {
            return Err(windows_error_to_io(error));
        }
    }

    Ok(())
}

fn current_hinstance() -> io::Result<HINSTANCE> {
    // SAFETY: Passing None requests the current process module handle and does
    // not require a caller-owned pointer.
    let module = unsafe { GetModuleHandleW(None) }.map_err(windows_error_to_io)?;
    Ok(HINSTANCE(module.0))
}

fn set_sender(sender: Option<Sender<RawControllerReport>>) {
    let storage = CONTROLLER_REPORT_SENDER.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = storage.lock() {
        *guard = sender;
    }
}

fn windows_error_to_io(error: windows::core::Error) -> io::Error {
    io::Error::other(error.to_string())
}
