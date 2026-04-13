use std::io;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Duration;

use super::config::CAPTURE_FILE;
use super::encoder::{MediaFoundationRuntime, Mp4Encoder};
use super::frame_source::{WinRtApartment, WindowFrameSource};
use super::metadata_writer::{CaptureFrameMetadata, CaptureMetadataWriter};
use super::scale::{CpuBgraScaler, FrameScaler};
use super::timing::{CaptureTimeMapper, FrameDecimator};

pub(super) fn run_capture(
    hwnd: isize,
    log_dir: &Path,
    stop_rx: Receiver<()>,
    ready_tx: Sender<io::Result<()>>,
) -> io::Result<()> {
    let setup = CaptureRuntime::start(hwnd, log_dir);
    let mut runtime = match setup {
        Ok(runtime) => {
            let _ = ready_tx.send(Ok(()));
            runtime
        }
        Err(error) => {
            let _ = ready_tx.send(Err(clone_io_error(&error)));
            return Err(error);
        }
    };

    let mut frame_index = 0u64;

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        let Some(frame) = runtime.frame_source.try_next_frame()? else {
            thread::sleep(Duration::from_millis(5));
            continue;
        };

        if !runtime.decimator.should_emit(frame.system_relative_time) {
            continue;
        }

        let output = runtime.scaler.scale_to_output(
            &frame.bgra,
            frame.content_size.width,
            frame.content_size.height,
            frame.row_pitch,
        );

        runtime.encoder.write_frame(&output, frame_index)?;
        let utc_timestamp = runtime
            .time_mapper
            .utc_timestamp_for_system_relative(frame.system_relative_time);
        runtime.metadata_writer.write(&CaptureFrameMetadata::new(
            frame_index,
            frame.system_relative_time,
            utc_timestamp,
            frame.content_size.width,
            frame.content_size.height,
        ))?;

        frame_index += 1;
    }

    runtime.metadata_writer.flush()?;
    runtime.encoder.finalize()
}

struct CaptureRuntime {
    _winrt: WinRtApartment,
    _mf: MediaFoundationRuntime,
    metadata_writer: CaptureMetadataWriter,
    frame_source: WindowFrameSource,
    scaler: CpuBgraScaler,
    encoder: Mp4Encoder,
    time_mapper: CaptureTimeMapper,
    decimator: FrameDecimator,
}

impl CaptureRuntime {
    fn start(hwnd: isize, log_dir: &Path) -> io::Result<Self> {
        let winrt = WinRtApartment::init()?;
        let mf = MediaFoundationRuntime::init()?;
        let metadata_writer = CaptureMetadataWriter::create(log_dir)?;
        let frame_source = WindowFrameSource::start(hwnd)?;
        let encoder = Mp4Encoder::create(&log_dir.join(CAPTURE_FILE))?;
        let time_mapper = CaptureTimeMapper::capture_now()?;
        let decimator = FrameDecimator::new();

        Ok(Self {
            _winrt: winrt,
            _mf: mf,
            metadata_writer,
            frame_source,
            scaler: CpuBgraScaler,
            encoder,
            time_mapper,
            decimator,
        })
    }
}

fn clone_io_error(error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), error.to_string())
}
