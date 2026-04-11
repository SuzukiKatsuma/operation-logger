use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use super::model::KeyboardInputEvent;
use crate::logger::common::{csv, time};

pub(super) struct KeyboardCsvWriter {
    writer: BufWriter<File>,
}

impl KeyboardCsvWriter {
    pub(super) fn new(log_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(log_dir)?;

        let input_file = File::create(log_dir.join("keyboard_input.csv"))?;
        let mut writer = Self {
            writer: BufWriter::new(input_file),
        };

        writer.write_headers()?;
        Ok(writer)
    }

    pub(super) fn write_input(&mut self, event: &KeyboardInputEvent) -> io::Result<()> {
        writeln!(
            self.writer,
            "{},{},{},{},{},{}",
            time::utc_timestamp_millis(),
            event.virtual_key,
            event.scan_code,
            csv::escape(&event.key_name),
            event.kind.as_csv_value(),
            event.is_injected
        )
    }

    pub(super) fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    fn write_headers(&mut self) -> io::Result<()> {
        writeln!(
            self.writer,
            "timestamp,virtual_key,scan_code,key_name,event,is_injected"
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::super::model::{KeyboardInputEvent, KeyboardInputKind};
    use super::*;

    #[test]
    fn writes_keyboard_input_csv() {
        let root = test_root("keyboard-input");
        fs::create_dir_all(&root).unwrap();

        let mut writer = KeyboardCsvWriter::new(&root).unwrap();
        writer
            .write_input(&KeyboardInputEvent {
                virtual_key: 65,
                scan_code: 30,
                key_name: "A".to_string(),
                kind: KeyboardInputKind::Down,
                is_injected: false,
            })
            .unwrap();
        writer.flush().unwrap();

        let csv = fs::read_to_string(root.join("keyboard_input.csv")).unwrap();
        let lines: Vec<_> = csv.lines().collect();

        assert_eq!(
            lines[0],
            "timestamp,virtual_key,scan_code,key_name,event,is_injected"
        );
        assert!(lines[1].contains(",65,30,A,keydown,false"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn escapes_keyboard_key_names_for_csv() {
        let root = test_root("keyboard-input-escape");
        fs::create_dir_all(&root).unwrap();

        let mut writer = KeyboardCsvWriter::new(&root).unwrap();
        writer
            .write_input(&KeyboardInputEvent {
                virtual_key: 188,
                scan_code: 51,
                key_name: "Comma, \"quoted\"".to_string(),
                kind: KeyboardInputKind::Up,
                is_injected: true,
            })
            .unwrap();
        writer.flush().unwrap();

        let csv = fs::read_to_string(root.join("keyboard_input.csv")).unwrap();
        let lines: Vec<_> = csv.lines().collect();

        assert!(lines[1].contains(",188,51,\"Comma, \"\"quoted\"\"\",keyup,true"));

        let _ = fs::remove_dir_all(root);
    }

    fn test_root(name: &str) -> PathBuf {
        PathBuf::from("target")
            .join("test-keyboard-csv-writer")
            .join(format!("pid-{}-{}", std::process::id(), name))
    }
}
