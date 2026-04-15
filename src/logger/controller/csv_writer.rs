use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use super::model::{ControllerAnalogEvent, ControllerButtonEvent};
use crate::logger::common::{csv, time};

pub(super) struct ControllerCsvWriter {
    button_writer: BufWriter<File>,
    analog_writer: BufWriter<File>,
}

impl ControllerCsvWriter {
    pub(super) fn new(log_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(log_dir)?;

        let button_file = File::create(log_dir.join("controller_button_input.csv"))?;
        let analog_file = File::create(log_dir.join("controller_analog_input.csv"))?;
        let mut writer = Self {
            button_writer: BufWriter::new(button_file),
            analog_writer: BufWriter::new(analog_file),
        };

        writer.write_headers()?;
        Ok(writer)
    }

    pub(super) fn write_button(&mut self, event: &ControllerButtonEvent) -> io::Result<()> {
        writeln!(
            self.button_writer,
            "{},{},{},{}",
            time::utc_timestamp_millis(),
            csv::escape(&event.device_id),
            csv::escape(&event.button),
            event.kind.as_csv_value()
        )
    }

    pub(super) fn write_analog(&mut self, event: &ControllerAnalogEvent) -> io::Result<()> {
        writeln!(
            self.analog_writer,
            "{},{},{},{}",
            time::utc_timestamp_millis(),
            csv::escape(&event.device_id),
            csv::escape(&event.control),
            event.value
        )
    }

    pub(super) fn flush(&mut self) -> io::Result<()> {
        self.button_writer.flush()?;
        self.analog_writer.flush()
    }

    fn write_headers(&mut self) -> io::Result<()> {
        writeln!(self.button_writer, "timestamp,device_id,button,event")?;
        writeln!(self.analog_writer, "timestamp,device_id,control,value")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::logger::controller::model::{ControllerButtonEvent, ControllerButtonEventKind};

    #[test]
    fn writes_controller_csv_files() {
        let root = PathBuf::from("target")
            .join("test-controller-csv")
            .join(format!("pid-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();

        let mut writer = ControllerCsvWriter::new(&root).unwrap();
        writer
            .write_button(&ControllerButtonEvent {
                device_id: "rawhid_0001".to_string(),
                button: "button_01".to_string(),
                kind: ControllerButtonEventKind::Down,
            })
            .unwrap();
        writer
            .write_analog(&ControllerAnalogEvent {
                device_id: "rawhid_0001".to_string(),
                control: "axis_left_x".to_string(),
                value: 127,
            })
            .unwrap();
        writer.flush().unwrap();

        let button_csv = fs::read_to_string(root.join("controller_button_input.csv")).unwrap();
        let analog_csv = fs::read_to_string(root.join("controller_analog_input.csv")).unwrap();

        assert_eq!(
            button_csv.lines().next(),
            Some("timestamp,device_id,button,event")
        );
        assert!(button_csv.contains(",rawhid_0001,button_01,keydown"));
        assert_eq!(
            analog_csv.lines().next(),
            Some("timestamp,device_id,control,value")
        );
        assert!(analog_csv.contains(",rawhid_0001,axis_left_x,127"));

        let _ = fs::remove_dir_all(root);
    }
}
