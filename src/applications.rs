use crate::platform::{RawWindow, collect_raw_windows, process_name_from_process_id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppWindow {
    pub hwnd: isize,
    pub title: String,
    pub process_id: u32,
    pub process_name: Option<String>,
}

pub fn list_running_applications() -> windows::core::Result<Vec<AppWindow>> {
    let raw_windows = collect_raw_windows()?;
    Ok(filter_app_windows(raw_windows))
}

fn filter_app_windows(raw_windows: Vec<RawWindow>) -> Vec<AppWindow> {
    raw_windows
        .into_iter()
        .filter_map(|w| {
            if !w.visible {
                return None;
            }

            let title = w.title.trim().to_string();
            if title.is_empty() {
                return None;
            }

            Some(AppWindow {
                hwnd: w.hwnd,
                title,
                process_id: w.process_id,
                process_name: process_name_from_process_id(w.process_id),
            })
        })
        .collect()
}
