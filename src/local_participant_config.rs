use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session_metadata::utc_timestamp_millis;

const CONFIG_DIR_NAME: &str = "operation-logger";
const CONFIG_FILE_NAME: &str = "config.json";
const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalParticipantConfig {
    pub schema_version: u32,
    pub local_participant_id: String,
    pub created_at_utc: String,
}

pub fn load_or_create_local_participant_config() -> io::Result<LocalParticipantConfig> {
    let config_dir = config_dir()?;
    load_or_create_local_participant_config_in(&config_dir)
}

fn config_dir() -> io::Result<PathBuf> {
    let appdata = env::var_os("APPDATA").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "APPDATA is not set; cannot locate operation-logger config directory",
        )
    })?;

    Ok(PathBuf::from(appdata).join(CONFIG_DIR_NAME))
}

fn load_or_create_local_participant_config_in(
    config_dir: &Path,
) -> io::Result<LocalParticipantConfig> {
    let path = config_dir.join(CONFIG_FILE_NAME);
    if !path.exists() {
        return create_new_config(config_dir, &path);
    }

    match read_config(&path) {
        Ok(config) => Ok(config),
        Err(_) => {
            fs::create_dir_all(config_dir)?;
            backup_invalid_config(&path)?;
            create_new_config(config_dir, &path)
        }
    }
}

fn read_config(path: &Path) -> io::Result<LocalParticipantConfig> {
    let json = fs::read_to_string(path)?;
    let config: LocalParticipantConfig = serde_json::from_str(&json).map_err(io::Error::other)?;
    validate_config(&config)?;
    Ok(config)
}

fn validate_config(config: &LocalParticipantConfig) -> io::Result<()> {
    if config.schema_version != SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported local participant config schema_version",
        ));
    }

    Uuid::parse_str(&config.local_participant_id)
        .map(|_| ())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn create_new_config(config_dir: &Path, path: &Path) -> io::Result<LocalParticipantConfig> {
    fs::create_dir_all(config_dir)?;

    let config = LocalParticipantConfig {
        schema_version: SCHEMA_VERSION,
        local_participant_id: Uuid::new_v4().to_string(),
        created_at_utc: utc_timestamp_millis(),
    };

    write_config(path, &config)?;
    Ok(config)
}

fn write_config(path: &Path, config: &LocalParticipantConfig) -> io::Result<()> {
    let json = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    file.write_all(json.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()
}

fn backup_invalid_config(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let backup_path = invalid_config_backup_path(path);
    fs::rename(path, backup_path)
}

fn invalid_config_backup_path(path: &Path) -> PathBuf {
    let timestamp = backup_timestamp();
    let mut backup_path = path.with_file_name(format!("config.invalid-{timestamp}.json"));
    let mut index = 1;

    while backup_path.exists() {
        backup_path = path.with_file_name(format!("config.invalid-{timestamp}-{index}.json"));
        index += 1;
    }

    backup_path
}

fn backup_timestamp() -> String {
    let value = utc_timestamp_millis();
    format!(
        "{}{}{}_{}{}{}",
        &value[0..4],
        &value[5..7],
        &value[8..10],
        &value[11..13],
        &value[14..16],
        &value[17..19]
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn creates_config_when_missing() {
        let dir = test_root("missing");
        let _ = fs::remove_dir_all(&dir);

        let config = load_or_create_local_participant_config_in(&dir).unwrap();

        assert_eq!(config.schema_version, 1);
        assert!(Uuid::parse_str(&config.local_participant_id).is_ok());
        assert!(dir.join(CONFIG_FILE_NAME).is_file());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn reuses_existing_local_participant_id() {
        let dir = test_root("reuse");
        let _ = fs::remove_dir_all(&dir);

        let first = load_or_create_local_participant_config_in(&dir).unwrap();
        let second = load_or_create_local_participant_config_in(&dir).unwrap();

        assert_eq!(second.local_participant_id, first.local_participant_id);
        assert_eq!(second.created_at_utc, first.created_at_utc);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn backs_up_invalid_config_and_creates_new_one() {
        let dir = test_root("invalid");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(CONFIG_FILE_NAME), "{ invalid json").unwrap();

        let config = load_or_create_local_participant_config_in(&dir).unwrap();

        assert!(Uuid::parse_str(&config.local_participant_id).is_ok());
        assert!(dir.join(CONFIG_FILE_NAME).is_file());
        let backups = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("config.invalid-")
            })
            .count();
        assert_eq!(backups, 1);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn treats_invalid_uuid_as_invalid_config() {
        let dir = test_root("invalid-uuid");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let config = LocalParticipantConfig {
            schema_version: 1,
            local_participant_id: "not-a-uuid".to_string(),
            created_at_utc: "2026-04-27T07:20:31.123".to_string(),
        };
        write_config(&dir.join(CONFIG_FILE_NAME), &config).unwrap();

        let recreated = load_or_create_local_participant_config_in(&dir).unwrap();

        assert!(Uuid::parse_str(&recreated.local_participant_id).is_ok());
        assert_ne!(recreated.local_participant_id, "not-a-uuid");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn treats_invalid_schema_version_as_invalid_config() {
        let dir = test_root("invalid-schema");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let config = LocalParticipantConfig {
            schema_version: 999,
            local_participant_id: Uuid::new_v4().to_string(),
            created_at_utc: "2026-04-27T07:20:31.123".to_string(),
        };
        write_config(&dir.join(CONFIG_FILE_NAME), &config).unwrap();

        let recreated = load_or_create_local_participant_config_in(&dir).unwrap();

        assert_eq!(recreated.schema_version, 1);
        assert_ne!(recreated.local_participant_id, config.local_participant_id);

        let _ = fs::remove_dir_all(dir);
    }

    fn test_root(name: &str) -> PathBuf {
        PathBuf::from("target")
            .join("test-local-participant-config")
            .join(format!("pid-{}-{}", std::process::id(), name))
    }
}
