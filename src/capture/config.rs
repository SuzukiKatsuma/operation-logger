pub(super) const CAPTURE_FILE: &str = "capture.mp4";
pub(super) const OUTPUT_WIDTH: u32 = 640;
pub(super) const OUTPUT_HEIGHT: u32 = 360;
pub(super) const TARGET_FPS: u32 = 10;
pub(super) const FRAME_DURATION_100NS: i64 = 10_000_000 / TARGET_FPS as i64;
pub(super) const VIDEO_BITRATE: u32 = 1_000_000;
pub(super) const BYTES_PER_PIXEL_BGRA: usize = 4;
pub(super) const CAPTURE_BUFFER_COUNT: i32 = 2;
