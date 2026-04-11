use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use super::model::{ClientPoint, MouseInputEvent, MouseMoveEvent};
use crate::logger::common::time;

pub(super) struct MouseCsvWriter {
    input_writer: BufWriter<File>,
    move_writer: BufWriter<File>,
    last_move_position: Option<ClientPoint>,
}

impl MouseCsvWriter {
    pub(super) fn new(log_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(log_dir)?;

        let input_file = File::create(log_dir.join("mouse_input.csv"))?;
        let move_file = File::create(log_dir.join("mouse_move.csv"))?;

        let mut writer = Self {
            input_writer: BufWriter::new(input_file),
            move_writer: BufWriter::new(move_file),
            last_move_position: None,
        };

        writer.write_headers()?;
        Ok(writer)
    }

    pub(super) fn write_input(&mut self, event: &MouseInputEvent) -> io::Result<()> {
        writeln!(
            self.input_writer,
            "{},{},{},{},{},{}",
            time::utc_timestamp_millis(),
            event.position.x,
            event.position.y,
            event.button.as_csv_value(),
            event.kind.as_csv_value(),
            event.delta
        )
    }

    pub(super) fn write_move(&mut self, event: &MouseMoveEvent) -> io::Result<()> {
        if self.last_move_position == Some(event.position) {
            return Ok(());
        }

        writeln!(
            self.move_writer,
            "{},{},{}",
            time::utc_timestamp_millis(),
            event.position.x,
            event.position.y
        )?;
        self.last_move_position = Some(event.position);

        Ok(())
    }

    pub(super) fn flush(&mut self) -> io::Result<()> {
        self.input_writer.flush()?;
        self.move_writer.flush()
    }

    fn write_headers(&mut self) -> io::Result<()> {
        writeln!(self.input_writer, "timestamp,x,y,button,event,delta")?;
        writeln!(self.move_writer, "timestamp,x,y")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::super::model::{MouseButton, MouseInputKind};
    use super::*;

    #[test]
    fn writes_mouse_input_csv() {
        let root = test_root("mouse-input");
        fs::create_dir_all(&root).unwrap();

        let mut writer = MouseCsvWriter::new(&root).unwrap();
        writer
            .write_input(&MouseInputEvent {
                position: ClientPoint { x: 10, y: 20 },
                button: MouseButton::Left,
                kind: MouseInputKind::Down,
                delta: 0,
            })
            .unwrap();
        writer.flush().unwrap();

        let csv = fs::read_to_string(root.join("mouse_input.csv")).unwrap();
        let lines: Vec<_> = csv.lines().collect();

        assert_eq!(lines[0], "timestamp,x,y,button,event,delta");
        assert!(lines[1].contains(",10,20,left,mousedown,0"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn writes_mouse_move_csv_without_consecutive_duplicate_positions() {
        let root = test_root("mouse-move");
        fs::create_dir_all(&root).unwrap();

        let mut writer = MouseCsvWriter::new(&root).unwrap();
        writer
            .write_move(&MouseMoveEvent {
                position: ClientPoint { x: 10, y: 20 },
            })
            .unwrap();
        writer
            .write_move(&MouseMoveEvent {
                position: ClientPoint { x: 10, y: 20 },
            })
            .unwrap();
        writer
            .write_move(&MouseMoveEvent {
                position: ClientPoint { x: 11, y: 20 },
            })
            .unwrap();
        writer.flush().unwrap();

        let csv = fs::read_to_string(root.join("mouse_move.csv")).unwrap();
        let lines: Vec<_> = csv.lines().collect();

        assert_eq!(lines[0], "timestamp,x,y");
        assert_eq!(lines.len(), 3);
        assert!(lines[1].contains(",10,20"));
        assert!(lines[2].contains(",11,20"));

        let _ = fs::remove_dir_all(root);
    }

    fn test_root(name: &str) -> PathBuf {
        PathBuf::from("target")
            .join("test-mouse-csv-writer")
            .join(format!("pid-{}-{}", std::process::id(), name))
    }
}
