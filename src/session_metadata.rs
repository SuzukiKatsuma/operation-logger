use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use windows::Win32::System::SystemInformation::GetSystemTime;

use crate::AppWindow;

const SESSION_METADATA_FILE: &str = "session_metadata.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMetadata {
    pub operation_logger_version: String,
    pub is_production_build: bool,
    pub local_participant_id: String,
    pub session_id: String,
    pub started_at_utc: String,
    pub target_app: SessionTargetApp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTargetApp {
    pub title: String,
    pub process_name: String,
}

pub(crate) fn write_session_metadata(
    log_dir: &Path,
    app: &AppWindow,
    local_participant_id: &str,
) -> io::Result<()> {
    let metadata = SessionMetadata {
        operation_logger_version: env!("CARGO_PKG_VERSION").to_string(),
        is_production_build: !cfg!(debug_assertions),
        local_participant_id: local_participant_id.to_string(),
        session_id: Uuid::new_v4().to_string(),
        started_at_utc: utc_timestamp_millis(),
        target_app: SessionTargetApp {
            title: app.title.clone(),
            process_name: app
                .process_name
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        },
    };

    write_session_metadata_json(log_dir, &metadata)
}

fn write_session_metadata_json(log_dir: &Path, metadata: &SessionMetadata) -> io::Result<()> {
    let path = log_dir.join(SESSION_METADATA_FILE);
    let json = serde_json::to_string_pretty(metadata).map_err(io::Error::other)?;

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    file.write_all(json.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()
}

pub(crate) fn utc_timestamp_millis() -> String {
    // SAFETY: GetSystemTime returns the current UTC SYSTEMTIME by value and has
    // no caller-owned pointer preconditions.
    let now = unsafe { GetSystemTime() };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond, now.wMilliseconds
    )
}

#[cfg(test)]
fn is_utc_millis_format(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 23 {
        return false;
    }

    bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[10] == b'T'
        && bytes[11..13].iter().all(u8::is_ascii_digit)
        && bytes[13] == b':'
        && bytes[14..16].iter().all(u8::is_ascii_digit)
        && bytes[16] == b':'
        && bytes[17..19].iter().all(u8::is_ascii_digit)
        && bytes[19] == b'.'
        && bytes[20..23].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn writes_expected_json_fields() {
        let dir = test_root("fields");
        fs::create_dir_all(&dir).unwrap();

        let app = AppWindow {
            hwnd: 1,
            title: "Game Window".to_string(),
            process_id: 999,
            process_name: Some("game.exe".to_string()),
        };

        write_session_metadata(&dir, &app, "8dd7f0c2-6e33-4ed4-a34f-0a5b7fd4b7d8").unwrap();

        let json = fs::read_to_string(dir.join(SESSION_METADATA_FILE)).unwrap();
        let parsed: SessionMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.operation_logger_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(parsed.is_production_build, !cfg!(debug_assertions));
        assert_eq!(
            parsed.local_participant_id,
            "8dd7f0c2-6e33-4ed4-a34f-0a5b7fd4b7d8"
        );
        assert!(uuid::Uuid::parse_str(&parsed.session_id).is_ok());
        assert_eq!(parsed.target_app.title, "Game Window");
        assert_eq!(parsed.target_app.process_name, "game.exe");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn started_at_utc_uses_expected_format() {
        let value = utc_timestamp_millis();
        assert!(is_utc_millis_format(&value));
    }

    #[test]
    fn target_app_falls_back_to_unknown_process_name() {
        let dir = test_root("unknown-process");
        fs::create_dir_all(&dir).unwrap();

        let app = AppWindow {
            hwnd: 1,
            title: "Untitled".to_string(),
            process_id: 7,
            process_name: None,
        };

        write_session_metadata(&dir, &app, "8dd7f0c2-6e33-4ed4-a34f-0a5b7fd4b7d8").unwrap();

        let json = fs::read_to_string(dir.join(SESSION_METADATA_FILE)).unwrap();
        let parsed: SessionMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.target_app.title, "Untitled");
        assert_eq!(parsed.target_app.process_name, "unknown");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn format_validator_rejects_invalid_values() {
        assert!(!is_utc_millis_format("2026/04/13 12:34:56"));
        assert!(!is_utc_millis_format("2026-04-13T12:34:56"));
    }

    fn test_root(name: &str) -> PathBuf {
        PathBuf::from("target")
            .join("test-session-metadata")
            .join(format!("pid-{}-{}", std::process::id(), name))
    }
}
