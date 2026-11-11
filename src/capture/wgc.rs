use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender, TrySendError};
use std::thread;
use std::time::Duration;

use super::config::{CAPTURE_FILE, FRAME_DURATION_100NS, TARGET_FPS};
use super::encoder::{MediaFoundationRuntime, Mp4Encoder};
use super::frame_source::{CapturedFrame, WinRtApartment, WindowFrameSource};
use super::metadata_writer::{CaptureFrameMetadata, CaptureMetadataWriter};
use super::scale::{CpuBgraScaler, FrameScaler};
use super::timeline::{FrameTimeline, TimelineStats};
use super::timing::CaptureTimeMapper;

enum CaptureEvent {
    Begin(i64, CaptureTimeMapper),
    Frame(CapturedFrame),
    Idle(i64),
    Stop {
        at: i64,
        received: u64,
        dropped: u64,
    },
}

pub(super) fn run_capture(
    hwnd: isize,
    log_dir: &Path,
    stop_rx: Receiver<()>,
    ready_tx: Sender<io::Result<()>>,
) -> io::Result<()> {
    // Bound memory and isolate capture from H.264 writes. Encoder overload drops
    // incoming observations (reported below), never video output slots.
    let (event_tx, event_rx) = mpsc::sync_channel(4);
    let (encoder_ready_tx, encoder_ready_rx) = mpsc::channel();
    let output_dir = log_dir.to_path_buf();
    let output_thread = thread::spawn(move || {
        // These guards outlive the output and are released on their owning thread.
        let setup = (|| {
            let winrt = WinRtApartment::init()?;
            let mf = MediaFoundationRuntime::init()?;
            let output = CaptureOutput::create(&output_dir)?;
            Ok((winrt, mf, output))
        })();
        match setup {
            Ok((_winrt, _mf, mut output)) => {
                let _ = encoder_ready_tx.send(Ok(()));
                output.run(event_rx, &output_dir)
            }
            Err(error) => {
                let _ = encoder_ready_tx.send(Err(clone_io_error(&error)));
                Err(error)
            }
        }
    });

    let capture_result = (|| {
        encoder_ready_rx
            .recv()
            .map_err(|_| io::Error::other("encoder exited during startup"))??;
        let _winrt = WinRtApartment::init()?;
        let mapper = CaptureTimeMapper::capture_now()?;
        let (mut source, started_at) = WindowFrameSource::start(hwnd)?;
        event_tx
            .send(CaptureEvent::Begin(started_at, mapper))
            .map_err(|_| io::Error::other("encoder disconnected"))?;
        let _ = ready_tx.send(Ok(()));

        let mut received = 0;
        let mut dropped = 0;
        let result = (|| {
            loop {
                match stop_rx.try_recv() {
                    Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                let event = match source.try_next_frame()? {
                    Some(frame) => {
                        received += 1;
                        CaptureEvent::Frame(frame)
                    }
                    None => {
                        CaptureEvent::Idle(CaptureTimeMapper::now_100ns()? - FRAME_DURATION_100NS)
                    }
                };
                // Idle watermarks lag by a slot so queued WGC frames can drain
                // before those output slots close. No video time is discarded.
                let idle = matches!(event, CaptureEvent::Idle(_));
                match event_tx.try_send(event) {
                    Ok(()) => {}
                    Err(TrySendError::Full(CaptureEvent::Frame(_))) => dropped += 1,
                    Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => {
                        return Err(io::Error::other("encoder disconnected"));
                    }
                }
                if idle {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            Ok(())
        })();
        // Freeze time BEFORE closing WGC, draining the queue or finalizing MP4.
        let stopped_at = CaptureTimeMapper::now_100ns()?;
        let close_result = source.stop();
        let send_result = event_tx
            .send(CaptureEvent::Stop {
                at: stopped_at,
                received,
                dropped,
            })
            .map_err(|_| io::Error::other("encoder disconnected"));
        result.and(close_result).and(send_result)
    })();
    if let Err(error) = &capture_result {
        let _ = ready_tx.send(Err(clone_io_error(error)));
    }
    drop(event_tx);
    let output_result = output_thread
        .join()
        .map_err(|_| io::Error::other("encoder thread panicked"))?;
    capture_result.and(output_result)
}

struct OutputFrame {
    bgra: Vec<u8>,
    metadata: CaptureFrameMetadata,
}

struct CaptureOutput {
    encoder: Mp4Encoder,
    metadata_writer: CaptureMetadataWriter,
}

impl CaptureOutput {
    fn create(log_dir: &Path) -> io::Result<Self> {
        Ok(Self {
            encoder: Mp4Encoder::create(&log_dir.join(CAPTURE_FILE))?,
            metadata_writer: CaptureMetadataWriter::create(log_dir)?,
        })
    }

    fn write(&mut self, index: u64, frame: &OutputFrame) -> io::Result<()> {
        self.encoder.write_frame(&frame.bgra, index)?;
        let mut metadata = frame.metadata.clone();
        metadata.frame_index = index;
        self.metadata_writer.write(&metadata)
    }

    fn run(&mut self, events: Receiver<CaptureEvent>, log_dir: &Path) -> io::Result<()> {
        let Ok(CaptureEvent::Begin(started_at, mapper)) = events.recv() else {
            return Ok(());
        };
        let mut timeline = FrameTimeline::new(started_at);
        let scaler = CpuBgraScaler;
        for event in events {
            match event {
                CaptureEvent::Frame(frame) => {
                    let output = OutputFrame {
                        bgra: scaler.scale_to_output(
                            &frame.bgra,
                            frame.content_size.width,
                            frame.content_size.height,
                            frame.row_pitch,
                        ),
                        metadata: CaptureFrameMetadata::new(
                            0,
                            frame.system_relative_time,
                            mapper.utc_timestamp_for_system_relative(frame.system_relative_time),
                            frame.content_size.width,
                            frame.content_size.height,
                            frame.window_x,
                            frame.window_y,
                        ),
                    };
                    timeline.accept(frame.system_relative_time, output, |index, frame| {
                        self.write(index, frame)
                    })?;
                }
                CaptureEvent::Idle(at) => {
                    timeline.advance_until(at, |index, frame| self.write(index, frame))?
                }
                CaptureEvent::Stop {
                    at,
                    received,
                    dropped,
                } => {
                    timeline.advance_until(at, |index, frame| self.write(index, frame))?;
                    self.metadata_writer.flush()?;
                    let finalize_result = if timeline.stats.output_frame_count == 0 {
                        Err(io::Error::other(
                            "capture stopped without a usable frame; no valid video was produced",
                        ))
                    } else {
                        self.encoder.finalize()
                    };
                    let mut metadata = CaptureSessionMetadata::new(
                        started_at, at, &mapper, &timeline, received, dropped,
                    );
                    metadata.video_finalized = finalize_result.is_ok();
                    serde_json::to_writer_pretty(
                        File::create(log_dir.join("capture_session_metadata.json"))?,
                        &metadata,
                    )
                    .map_err(io::Error::other)?;
                    return finalize_result;
                }
                CaptureEvent::Begin(..) => return Err(io::Error::other("duplicate capture start")),
            }
        }
        Err(io::Error::other("capture ended without a stop boundary"))
    }
}

#[derive(serde::Serialize)]
struct CaptureSessionMetadata<'a> {
    capture_started_at_utc: String,
    capture_stopped_at_utc: String,
    video_zero_at_utc: String,
    capture_started_at_system_relative_100ns: i64,
    capture_stopped_at_system_relative_100ns: i64,
    elapsed_100ns: i64,
    elapsed_seconds: f64,
    target_fps: u32,
    video_duration_seconds: f64,
    video_finalized: bool,
    first_frame_system_relative_time: Option<i64>,
    first_frame_utc: Option<String>,
    initial_padding_policy: &'static str,
    received_frame_count: u64,
    queue_dropped_frame_count: u64,
    effective_capture_fps: f64,
    effective_adopted_fps: f64,
    #[serde(flatten)]
    stats: &'a TimelineStats,
}

impl<'a> CaptureSessionMetadata<'a> {
    fn new<T>(
        start: i64,
        stop: i64,
        mapper: &CaptureTimeMapper,
        timeline: &'a FrameTimeline<T>,
        received: u64,
        dropped: u64,
    ) -> Self {
        let elapsed_100ns = (stop - start).max(0);
        let elapsed_seconds = elapsed_100ns as f64 / 10_000_000.0;
        let fps = |count| {
            if elapsed_seconds > 0.0 {
                count as f64 / elapsed_seconds
            } else {
                0.0
            }
        };
        Self {
            capture_started_at_utc: mapper.utc_timestamp_for_system_relative(start),
            capture_stopped_at_utc: mapper.utc_timestamp_for_system_relative(stop),
            video_zero_at_utc: mapper.utc_timestamp_for_system_relative(start),
            capture_started_at_system_relative_100ns: start,
            capture_stopped_at_system_relative_100ns: stop,
            elapsed_100ns,
            elapsed_seconds,
            target_fps: TARGET_FPS,
            video_duration_seconds: timeline.stats.output_frame_count as f64 / TARGET_FPS as f64,
            video_finalized: false,
            first_frame_system_relative_time: timeline.first_frame_time_100ns,
            first_frame_utc: timeline
                .first_frame_time_100ns
                .map(|time| mapper.utc_timestamp_for_system_relative(time)),
            initial_padding_policy: "backfill_with_first_observation_preserving_source_timestamp",
            received_frame_count: received,
            queue_dropped_frame_count: dropped,
            effective_capture_fps: fps(received),
            effective_adopted_fps: fps(timeline.stats.adopted_frame_count),
            stats: &timeline.stats,
        }
    }
}

fn clone_io_error(error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), error.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    use windows::Win32::Media::MediaFoundation::{
        MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,
        MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM, MFCreateAttributes,
        MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Video, MFVideoFormat_RGB32,
    };
    use windows::core::PCWSTR;

    use super::*;
    use crate::capture::frame_source::ContentSize;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "operation-logger-capture-test-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn synthetic_frame(time: i64, width: u32) -> CapturedFrame {
        CapturedFrame {
            bgra: vec![96; width as usize * 2 * 4],
            row_pitch: width as usize * 4,
            system_relative_time: time,
            content_size: ContentSize { width, height: 2 },
            window_x: -(width as i32) * 100,
            window_y: width as i32 * 50,
        }
    }

    // Exercise the actual H.264/MP4 writer and decoder, not a mock frame sink.
    fn decoded_timestamps(path: &Path) -> Vec<i64> {
        let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: URL is NUL-terminated and alive; reader and media type are valid
        // COM objects, and all ReadSample out-pointers refer to local storage.
        unsafe {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1).unwrap();
            let attributes = attributes.unwrap();
            // Enable the YUV-to-RGB32 conversion required by the decode check.
            attributes
                .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
                .unwrap();
            let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attributes).unwrap();
            let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
            let media_type = MFCreateMediaType().unwrap();
            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .unwrap();
            media_type
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                .unwrap();
            reader
                .SetCurrentMediaType(stream, None, &media_type)
                .unwrap();
            let mut times = Vec::new();
            for _ in 0..1000 {
                let mut flags = 0;
                let mut timestamp = 0;
                let mut sample = None;
                reader
                    .ReadSample(
                        stream,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut timestamp),
                        Some(&mut sample),
                    )
                    .unwrap();
                if let Some(sample) = sample {
                    assert_eq!(sample.GetSampleDuration().unwrap(), FRAME_DURATION_100NS);
                    times.push(timestamp);
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    return times;
                }
            }
            panic!("decoder did not reach end of stream");
        }
    }

    #[test]
    fn mp4_csv_and_session_metadata_match_including_stop_padding() {
        let _winrt = WinRtApartment::init().unwrap();
        let _mf = MediaFoundationRuntime::init().unwrap();
        let directory = TestDirectory::new();
        let mapper = CaptureTimeMapper::capture_now().unwrap();
        let start = CaptureTimeMapper::now_100ns().unwrap();
        let d = FRAME_DURATION_100NS;
        let (tx, rx) = mpsc::channel();
        tx.send(CaptureEvent::Begin(start, mapper)).unwrap();
        for (offset, width) in [(0, 2), (d, 3), (4 * d, 4)] {
            tx.send(CaptureEvent::Frame(synthetic_frame(start + offset, width)))
                .unwrap();
        }
        tx.send(CaptureEvent::Stop {
            at: start + 6 * d,
            received: 3,
            dropped: 0,
        })
        .unwrap();
        drop(tx);
        let mut output = CaptureOutput::create(&directory.0).unwrap();
        // The encoder also enforces consecutive indices, including duplicates.
        assert!(output.encoder.write_frame(&[], 1).is_err());
        output.run(rx, &directory.0).unwrap();
        drop(output);

        let csv = fs::read_to_string(directory.0.join("capture_metadata.csv")).unwrap();
        let rows: Vec<Vec<&str>> = csv
            .lines()
            .skip(1)
            .map(|row| row.split(',').collect())
            .collect();
        let times = decoded_timestamps(&directory.0.join(CAPTURE_FILE));
        assert_eq!(rows.len(), 6);
        assert_eq!(times.len(), rows.len());
        let source_offsets = [0, d, d, d, 4 * d, 4 * d];
        let source_widths = [2, 3, 3, 3, 4, 4];
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row[0].parse::<usize>().unwrap(), index);
            assert_eq!(times[index], index as i64 * d);
            assert_eq!(
                row[1].parse::<i64>().unwrap(),
                start + source_offsets[index]
            );
            assert_eq!(
                row[2],
                mapper.utc_timestamp_for_system_relative(start + source_offsets[index])
            );
            assert_eq!(row[3].parse::<u32>().unwrap(), source_widths[index]);
            assert_eq!(row[4], "2");
            assert_eq!(row.len(), 7);
            assert_eq!(
                row[5].parse::<i32>().unwrap(),
                -(source_widths[index] as i32) * 100
            );
            assert_eq!(
                row[6].parse::<i32>().unwrap(),
                source_widths[index] as i32 * 50
            );
        }
        // Identical source fields identify a duplicate and trace it to the first
        // output row with that same real observation timestamp.
        assert_eq!(rows[1][1..], rows[2][1..]);
        assert_eq!(rows[1][1..], rows[3][1..]);
        assert_eq!(rows[4][1..], rows[5][1..]);
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(directory.0.join("capture_session_metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["output_frame_count"], times.len());
        assert_eq!(manifest["video_finalized"], true);
        assert_eq!(manifest["adopted_frame_count"], 3);
        assert_eq!(manifest["duplicated_frame_count"], 3);
        assert_eq!(manifest["initial_padding_frame_count"], 0);
        assert_eq!(manifest["elapsed_100ns"], 6 * d);
        assert_eq!(
            manifest["video_zero_at_utc"],
            mapper.utc_timestamp_for_system_relative(start)
        );
        assert_eq!(
            manifest["capture_stopped_at_utc"],
            mapper.utc_timestamp_for_system_relative(start + 6 * d)
        );
    }

    #[test]
    fn initial_padding_is_explicit_and_does_not_fabricate_timestamps() {
        let _winrt = WinRtApartment::init().unwrap();
        let _mf = MediaFoundationRuntime::init().unwrap();
        let directory = TestDirectory::new();
        let mapper = CaptureTimeMapper::capture_now().unwrap();
        let start = CaptureTimeMapper::now_100ns().unwrap();
        let d = FRAME_DURATION_100NS;
        let (tx, rx) = mpsc::channel();
        tx.send(CaptureEvent::Begin(start, mapper)).unwrap();
        tx.send(CaptureEvent::Idle(start + d)).unwrap();
        tx.send(CaptureEvent::Frame(synthetic_frame(start + 2 * d + 1, 2)))
            .unwrap();
        tx.send(CaptureEvent::Stop {
            at: start + 4 * d,
            received: 1,
            dropped: 0,
        })
        .unwrap();
        drop(tx);
        let mut output = CaptureOutput::create(&directory.0).unwrap();
        output.run(rx, &directory.0).unwrap();
        drop(output);
        let csv = fs::read_to_string(directory.0.join("capture_metadata.csv")).unwrap();
        let rows: Vec<_> = csv.lines().skip(1).collect();
        assert_eq!(rows.len(), 4);
        let first_source_fields = rows[0].split_once(',').unwrap().1;
        for row in &rows {
            assert_eq!(row.split_once(',').unwrap().1, first_source_fields);
            assert_eq!(
                row.split(',').collect::<Vec<_>>()[3..],
                ["2", "2", "-200", "100"]
            );
            assert_eq!(
                row.split(',').nth(1).unwrap().parse::<i64>().unwrap(),
                start + 2 * d + 1
            );
        }
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(directory.0.join("capture_session_metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["initial_padding_frame_count"], 3);
        assert_eq!(
            manifest["first_frame_system_relative_time"],
            start + 2 * d + 1
        );
        assert_eq!(manifest["adopted_frame_count"], 1);
        assert_eq!(manifest["duplicated_frame_count"], 3);
    }

    #[test]
    fn no_frames_produces_diagnostics_instead_of_invented_observations() {
        let _winrt = WinRtApartment::init().unwrap();
        let _mf = MediaFoundationRuntime::init().unwrap();
        let directory = TestDirectory::new();
        let mapper = CaptureTimeMapper::capture_now().unwrap();
        let start = CaptureTimeMapper::now_100ns().unwrap();
        let (tx, rx) = mpsc::channel();
        tx.send(CaptureEvent::Begin(start, mapper)).unwrap();
        tx.send(CaptureEvent::Stop {
            at: start + FRAME_DURATION_100NS,
            received: 0,
            dropped: 0,
        })
        .unwrap();
        drop(tx);
        let mut output = CaptureOutput::create(&directory.0).unwrap();
        assert!(output.run(rx, &directory.0).is_err());
        drop(output);
        let csv = fs::read_to_string(directory.0.join("capture_metadata.csv")).unwrap();
        assert_eq!(csv.lines().count(), 1);
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(directory.0.join("capture_session_metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["output_frame_count"], 0);
        assert_eq!(manifest["video_finalized"], false);
        assert!(manifest["first_frame_system_relative_time"].is_null());
        assert!(manifest["first_frame_utc"].is_null());
    }

    #[test]
    fn ten_seconds_of_mp4_at_normal_and_half_capture_rate() {
        let _winrt = WinRtApartment::init().unwrap();
        let _mf = MediaFoundationRuntime::init().unwrap();
        let mapper = CaptureTimeMapper::capture_now().unwrap();
        let start = CaptureTimeMapper::now_100ns().unwrap();
        let d = FRAME_DURATION_100NS;
        for (step, stop_offset) in [(1, 0), (2, 0), (1, d / 3), (2, d / 3)] {
            let directory = TestDirectory::new();
            let (tx, rx) = mpsc::channel();
            tx.send(CaptureEvent::Begin(start, mapper)).unwrap();
            for index in (0..100).step_by(step) {
                tx.send(CaptureEvent::Frame(synthetic_frame(start + index * d, 2)))
                    .unwrap();
            }
            tx.send(CaptureEvent::Stop {
                at: start + 100 * d + stop_offset,
                received: 100 / step as u64,
                dropped: 0,
            })
            .unwrap();
            drop(tx);
            let mut output = CaptureOutput::create(&directory.0).unwrap();
            output.run(rx, &directory.0).unwrap();
            drop(output);
            let times = decoded_timestamps(&directory.0.join(CAPTURE_FILE));
            let count = if stop_offset == 0 { 100 } else { 101 };
            assert_eq!(times.len(), count);
            let duration = times.last().copied().unwrap() + d;
            assert!(duration >= 100 * d + stop_offset);
            assert!(duration - (100 * d + stop_offset) < d);
            let csv = fs::read_to_string(directory.0.join("capture_metadata.csv")).unwrap();
            assert_eq!(csv.lines().skip(1).count(), times.len());
            let manifest: serde_json::Value = serde_json::from_slice(
                &fs::read(directory.0.join("capture_session_metadata.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(manifest["adopted_frame_count"], 100 / step);
            assert_eq!(manifest["duplicated_frame_count"], count - 100 / step);
            assert_eq!(manifest["video_duration_seconds"], count as f64 / 10.0);
        }
    }
}
