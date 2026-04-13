use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Media::MediaFoundation::{
    IMFMediaBuffer, IMFSinkWriter, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_VERSION,
    MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL,
    MFMediaType_Video, MFSTARTUP_FULL, MFShutdown, MFStartup, MFVideoFormat_H264,
    MFVideoFormat_RGB32, MFVideoInterlace_Progressive,
};
use windows::core::{GUID, PCWSTR};

use super::config::{
    BYTES_PER_PIXEL_BGRA, FRAME_DURATION_100NS, OUTPUT_HEIGHT, OUTPUT_WIDTH, TARGET_FPS,
    VIDEO_BITRATE,
};

pub(super) struct Mp4Encoder {
    sink_writer: IMFSinkWriter,
    stream_index: u32,
}

impl Mp4Encoder {
    pub(super) fn create(path: &Path) -> io::Result<Self> {
        let wide_path = path_to_wide(path);
        // SAFETY: wide_path is NUL-terminated and remains alive for the call.
        // No byte stream or custom attributes are supplied.
        let sink_writer =
            unsafe { MFCreateSinkWriterFromURL(PCWSTR(wide_path.as_ptr()), None, None) }
                .map_err(windows_error_to_io)?;

        let output_type = create_video_type(MFVideoFormat_H264, OUTPUT_WIDTH, OUTPUT_HEIGHT)?;
        // SAFETY: output_type is a valid IMFMediaType configured for H.264 video.
        let stream_index =
            unsafe { sink_writer.AddStream(&output_type) }.map_err(windows_error_to_io)?;

        let input_type = create_video_type(MFVideoFormat_RGB32, OUTPUT_WIDTH, OUTPUT_HEIGHT)?;
        // SAFETY: stream_index was returned by AddStream, input_type describes
        // the RGB32 samples written below, and no extra encoding attributes are used.
        unsafe { sink_writer.SetInputMediaType(stream_index, &input_type, None) }
            .map_err(windows_error_to_io)?;
        // SAFETY: Sink writer is fully configured with one stream.
        unsafe { sink_writer.BeginWriting() }.map_err(windows_error_to_io)?;

        Ok(Self {
            sink_writer,
            stream_index,
        })
    }

    pub(super) fn write_frame(&mut self, bgra: &[u8], frame_index: u64) -> io::Result<()> {
        let expected_len =
            (OUTPUT_WIDTH as usize) * (OUTPUT_HEIGHT as usize) * BYTES_PER_PIXEL_BGRA;
        if bgra.len() != expected_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "scaled capture frame has an unexpected byte length",
            ));
        }

        let sample = create_media_sample(bgra)?;
        let sample_time = frame_index as i64 * FRAME_DURATION_100NS;
        // SAFETY: sample is newly created for this frame and not shared while
        // timestamps are being assigned.
        unsafe {
            sample
                .SetSampleTime(sample_time)
                .map_err(windows_error_to_io)?;
            sample
                .SetSampleDuration(FRAME_DURATION_100NS)
                .map_err(windows_error_to_io)?;
            self.sink_writer
                .WriteSample(self.stream_index, &sample)
                .map_err(windows_error_to_io)?;
        }
        Ok(())
    }

    pub(super) fn finalize(self) -> io::Result<()> {
        // SAFETY: Finalize is called once after all samples have been written.
        unsafe { self.sink_writer.Finalize() }.map_err(windows_error_to_io)
    }
}

pub(super) struct MediaFoundationRuntime;

impl MediaFoundationRuntime {
    pub(super) fn init() -> io::Result<Self> {
        // SAFETY: MFStartup initializes process Media Foundation state before
        // creating the sink writer and is balanced by MFShutdown in Drop.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.map_err(windows_error_to_io)?;
        Ok(Self)
    }
}

impl Drop for MediaFoundationRuntime {
    fn drop(&mut self) {
        // SAFETY: This balances the successful MFStartup call in this worker.
        let _ = unsafe { MFShutdown() };
    }
}

fn create_video_type(
    subtype: GUID,
    width: u32,
    height: u32,
) -> io::Result<windows::Win32::Media::MediaFoundation::IMFMediaType> {
    // SAFETY: MFCreateMediaType allocates and returns a new empty media type.
    let media_type = unsafe { MFCreateMediaType() }.map_err(windows_error_to_io)?;
    // SAFETY: All GUID pointers refer to static Media Foundation constants, and
    // the packed UINT64 attributes follow Media Foundation's documented format.
    unsafe {
        media_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(windows_error_to_io)?;
        media_type
            .SetGUID(&MF_MT_SUBTYPE, &subtype)
            .map_err(windows_error_to_io)?;
        media_type
            .SetUINT32(&MF_MT_AVG_BITRATE, VIDEO_BITRATE)
            .map_err(windows_error_to_io)?;
        media_type
            .SetUINT64(&MF_MT_FRAME_SIZE, pack_ratio(width, height))
            .map_err(windows_error_to_io)?;
        media_type
            .SetUINT64(&MF_MT_FRAME_RATE, pack_ratio(TARGET_FPS, 1))
            .map_err(windows_error_to_io)?;
        media_type
            .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_ratio(1, 1))
            .map_err(windows_error_to_io)?;
        media_type
            .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .map_err(windows_error_to_io)?;
    }
    Ok(media_type)
}

fn create_media_sample(
    bgra: &[u8],
) -> io::Result<windows::Win32::Media::MediaFoundation::IMFSample> {
    // SAFETY: Allocates an empty Media Foundation sample.
    let sample = unsafe { MFCreateSample() }.map_err(windows_error_to_io)?;
    // SAFETY: Allocates a memory buffer large enough for one RGB32 frame.
    let buffer = unsafe { MFCreateMemoryBuffer(bgra.len() as u32) }.map_err(windows_error_to_io)?;
    copy_to_media_buffer(&buffer, bgra)?;
    // SAFETY: buffer is a valid IMFMediaBuffer containing this frame.
    unsafe { sample.AddBuffer(&buffer) }.map_err(windows_error_to_io)?;
    Ok(sample)
}

fn copy_to_media_buffer(buffer: &IMFMediaBuffer, bytes: &[u8]) -> io::Result<()> {
    let mut data = std::ptr::null_mut();
    let mut max_len = 0u32;
    // SAFETY: data/max_len are valid out pointers. The buffer is unlocked below
    // after copying at most bytes.len() bytes into it.
    unsafe { buffer.Lock(&mut data, Some(&mut max_len), None) }.map_err(windows_error_to_io)?;
    if max_len < bytes.len() as u32 {
        return Err(io::Error::other("Media Foundation buffer is too small"));
    }

    // SAFETY: Lock returned a writable buffer of at least max_len bytes, and the
    // checked branch above proves bytes.len() fits.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len()) };
    // SAFETY: bytes.len() was verified against the buffer capacity.
    unsafe { buffer.SetCurrentLength(bytes.len() as u32) }.map_err(windows_error_to_io)?;
    // SAFETY: Balances the successful Lock call above.
    unsafe { buffer.Unlock() }.map_err(windows_error_to_io)
}

fn pack_ratio(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

fn path_to_wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn windows_error_to_io(error: windows::core::Error) -> io::Error {
    io::Error::other(error.to_string())
}
