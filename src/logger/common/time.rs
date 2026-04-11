use windows::Win32::System::SystemInformation::GetSystemTime;

pub(in crate::logger) fn utc_timestamp_millis() -> String {
    // SAFETY: GetSystemTime writes no Rust memory and returns a SYSTEMTIME by value.
    // It has no preconditions beyond running on Windows.
    let now = unsafe { GetSystemTime() };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond, now.wMilliseconds
    )
}
