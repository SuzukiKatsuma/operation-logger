use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;

const CAPTURE_METADATA_FILE: &str = "capture_metadata.csv";
const CAPTURE_METADATA_HEADER: &str =
    "frame_index,system_relative_time,utc_timestamp,content_width,content_height\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CaptureFrameMetadata {
    pub(super) frame_index: u64,
    pub(super) system_relative_time: i64,
    pub(super) utc_timestamp: String,
    pub(super) content_width: u32,
    pub(super) content_height: u32,
}

impl CaptureFrameMetadata {
    pub(super) fn new(
        frame_index: u64,
        system_relative_time: i64,
        utc_timestamp: String,
        content_width: u32,
        content_height: u32,
    ) -> Self {
        Self {
            frame_index,
            system_relative_time,
            utc_timestamp,
            content_width,
            content_height,
        }
    }

    pub(super) fn to_csv_row(&self) -> String {
        format!(
            "{},{},{},{},{}",
            self.frame_index,
            self.system_relative_time,
            self.utc_timestamp,
            self.content_width,
            self.content_height
        )
    }
}

pub(super) struct CaptureMetadataWriter {
    writer: BufWriter<File>,
}

impl CaptureMetadataWriter {
    pub(super) fn create(log_dir: &Path) -> io::Result<Self> {
        let path = log_dir.join(CAPTURE_METADATA_FILE);
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(CAPTURE_METADATA_HEADER.as_bytes())?;
        writer.flush()?;
        Ok(Self { writer })
    }

    pub(super) fn write(&mut self, metadata: &CaptureFrameMetadata) -> io::Result<()> {
        writeln!(self.writer, "{}", metadata.to_csv_row())
    }

    pub(super) fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_row_matches_capture_schema() {
        let metadata =
            CaptureFrameMetadata::new(3, 123456, "2026-04-12T19:05:44.116".to_string(), 1920, 1080);

        assert_eq!(
            metadata.to_csv_row(),
            "3,123456,2026-04-12T19:05:44.116,1920,1080"
        );
    }

    #[test]
    fn metadata_rows_keep_content_size_per_frame() {
        let first =
            CaptureFrameMetadata::new(7, 1000, "2026-04-12T19:05:44.116".to_string(), 1280, 720);
        let resized =
            CaptureFrameMetadata::new(8, 2000, "2026-04-12T19:05:44.216".to_string(), 1920, 1080);

        assert_eq!(first.content_width, 1280);
        assert_eq!(first.content_height, 720);
        assert_eq!(resized.content_width, 1920);
        assert_eq!(resized.content_height, 1080);
    }
}
