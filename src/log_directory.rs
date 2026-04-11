use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::applications::AppWindow;
use crate::platform::local_timestamp_for_directory_name;

const OPERATION_LOGS_DIR: &str = "OperationLogs";

pub fn create_operation_log_directory(app: &AppWindow) -> io::Result<PathBuf> {
    let root = operation_logs_root()?;
    let timestamp = local_timestamp_for_directory_name();
    create_operation_log_directory_in(&root, &timestamp, app)
}

fn operation_logs_root() -> io::Result<PathBuf> {
    let user_profile = env::var_os("USERPROFILE").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "USERPROFILE is not set; cannot locate the Documents directory",
        )
    })?;

    Ok(PathBuf::from(user_profile)
        .join("Documents")
        .join(OPERATION_LOGS_DIR))
}

fn build_operation_log_directory_path(root: &Path, timestamp: &str, app: &AppWindow) -> PathBuf {
    let process_name = app.process_name.as_deref().unwrap_or("unknown");
    root.join(format!(
        "{}_{}",
        timestamp,
        sanitize_path_component(process_name)
    ))
}

fn create_operation_log_directory_in(
    root: &Path,
    timestamp: &str,
    app: &AppWindow,
) -> io::Result<PathBuf> {
    let path = build_operation_log_directory_path(root, timestamp, app);
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn sanitize_path_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();

    let trimmed = sanitized.trim_matches([' ', '.']);
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn creates_operation_log_directory() {
        let root = test_root("create");
        let app = AppWindow {
            hwnd: 1,
            title: "Test App".to_string(),
            process_id: 123,
            process_name: Some("test-app.exe".to_string()),
        };

        let path = create_operation_log_directory_in(&root, "2026-04-13_012345", &app).unwrap();

        assert!(path.is_dir());
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("2026-04-13_012345_test-app.exe")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sanitizes_process_name_for_directory_name() {
        let root = test_root("sanitize");
        let app = AppWindow {
            hwnd: 1,
            title: "Test App".to_string(),
            process_id: 123,
            process_name: Some(r#"bad<>:"/\|?*.exe"#.to_string()),
        };

        let path = create_operation_log_directory_in(&root, "2026-04-13_012345", &app).unwrap();

        assert!(path.is_dir());
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("2026-04-13_012345_bad_________.exe")
        );

        let _ = fs::remove_dir_all(root);
    }

    fn test_root(name: &str) -> PathBuf {
        PathBuf::from("target")
            .join("test-operation-logs")
            .join(format!("pid-{}-{}", std::process::id(), name))
    }
}
