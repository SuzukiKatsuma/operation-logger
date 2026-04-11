use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::{BOOL, PWSTR};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawWindow {
    pub(crate) hwnd: isize,
    pub(crate) visible: bool,
    pub(crate) title: String,
    pub(crate) process_id: u32,
}

pub(crate) fn collect_raw_windows() -> windows::core::Result<Vec<RawWindow>> {
    let mut out = Vec::<RawWindow>::new();

    // SAFETY: enum_windows_proc expects lparam to be a valid pointer to this
    // Vec<RawWindow>. EnumWindows calls the callback synchronously before out
    // is moved or dropped.
    let enum_result = unsafe {
        EnumWindows(
            Some(enum_windows_proc),
            LPARAM((&mut out as *mut Vec<RawWindow>) as isize),
        )
    };

    if let Err(error) = enum_result {
        if error.code().is_err() {
            return Err(error);
        }
    }

    Ok(out)
}

pub(crate) fn process_name_from_process_id(process_id: u32) -> Option<String> {
    if process_id == 0 {
        return None;
    }

    // SAFETY: OpenProcess is called with query-only access and a plain process
    // id. The returned handle is checked and later closed exactly once.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()? };
    let mut buffer = vec![0u16; 32_768];
    let mut size = buffer.len() as u32;

    // SAFETY: buffer is valid writable UTF-16 storage of length size. The handle
    // is valid because OpenProcess succeeded.
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    };

    // SAFETY: handle came from OpenProcess above and is not used after this call.
    let _ = unsafe { CloseHandle(handle) };

    result.ok()?;

    let image_path = String::from_utf16_lossy(&buffer[..size as usize]);
    Path::new(&image_path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

pub(crate) fn local_timestamp_for_directory_name() -> String {
    // SAFETY: GetLocalTime writes no Rust memory and returns SYSTEMTIME by value.
    // It has no preconditions beyond running on Windows.
    let now = unsafe { GetLocalTime() };

    format!(
        "{:04}-{:02}-{:02}_{:02}{:02}{:02}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    )
}

// SAFETY: This function is called by EnumWindows using the system ABI. lparam
// must be the Vec<RawWindow> pointer supplied by collect_raw_windows.
unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: collect_raw_windows passes a pointer to a live Vec<RawWindow> as
    // lparam, and EnumWindows invokes this callback synchronously.
    let out = unsafe { &mut *(lparam.0 as *mut Vec<RawWindow>) };

    // SAFETY: hwnd is provided by EnumWindows and may be queried with
    // IsWindowVisible during the callback.
    let visible = unsafe { IsWindowVisible(hwnd).as_bool() };

    // SAFETY: hwnd is provided by EnumWindows. A zero length is handled below.
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    let title = if len > 0 {
        let mut buffer = vec![0u16; (len + 1) as usize];
        // SAFETY: buffer has len + 1 UTF-16 code units, enough for the window
        // text plus terminator according to GetWindowTextLengthW.
        let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        String::from_utf16_lossy(&buffer[..copied as usize])
    } else {
        String::new()
    };

    let mut process_id = 0u32;
    // SAFETY: hwnd is provided by EnumWindows, and process_id is a valid local
    // out pointer for the duration of the call.
    let _thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id as *mut u32)) };

    out.push(RawWindow {
        hwnd: hwnd.0 as isize,
        visible,
        title,
        process_id,
    });

    BOOL(1)
}
