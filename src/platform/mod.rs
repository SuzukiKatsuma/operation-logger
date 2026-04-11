#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub(crate) use self::windows::{
    RawWindow, collect_raw_windows, local_timestamp_for_directory_name,
    process_name_from_process_id,
};
