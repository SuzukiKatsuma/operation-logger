use std::io;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;

use super::config::FRAME_DURATION_100NS;

const HUNDRED_NS_PER_SECOND: i128 = 10_000_000;
const FILETIME_UNIX_EPOCH_100NS: i128 = 116_444_736_000_000_000;

#[derive(Debug, Clone, Copy)]
pub(super) struct CaptureTimeMapper {
    system_relative_anchor_100ns: i64,
    utc_anchor_filetime_100ns: i128,
}

impl CaptureTimeMapper {
    pub(super) fn capture_now() -> io::Result<Self> {
        Ok(Self {
            system_relative_anchor_100ns: qpc_as_100ns_now()?,
            utc_anchor_filetime_100ns: precise_utc_filetime_100ns_now(),
        })
    }

    pub(super) fn utc_timestamp_for_system_relative(
        &self,
        system_relative_time_100ns: i64,
    ) -> String {
        let delta = system_relative_time_100ns as i128 - self.system_relative_anchor_100ns as i128;
        let frame_filetime_100ns = self.utc_anchor_filetime_100ns + delta;
        format_filetime_utc_millis(frame_filetime_100ns)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FrameDecimator {
    next_emit_at_100ns: Option<i64>,
}

impl FrameDecimator {
    pub(super) fn new() -> Self {
        Self {
            next_emit_at_100ns: None,
        }
    }

    pub(super) fn should_emit(&mut self, system_relative_time_100ns: i64) -> bool {
        match self.next_emit_at_100ns {
            None => {
                self.next_emit_at_100ns = Some(system_relative_time_100ns + FRAME_DURATION_100NS);
                true
            }
            Some(next_emit_at_100ns) if system_relative_time_100ns < next_emit_at_100ns => false,
            Some(mut next_emit_at_100ns) => {
                while next_emit_at_100ns <= system_relative_time_100ns {
                    next_emit_at_100ns += FRAME_DURATION_100NS;
                }
                self.next_emit_at_100ns = Some(next_emit_at_100ns);
                true
            }
        }
    }
}

fn qpc_as_100ns_now() -> io::Result<i64> {
    let mut frequency = 0i64;
    let mut counter = 0i64;

    // SAFETY: The pointers refer to valid writable i64 storage.
    unsafe { QueryPerformanceFrequency(&mut frequency) }.map_err(windows_error_to_io)?;
    // SAFETY: The pointer refers to valid writable i64 storage.
    unsafe { QueryPerformanceCounter(&mut counter) }.map_err(windows_error_to_io)?;

    if frequency <= 0 {
        return Err(io::Error::other("QPC frequency is invalid"));
    }

    let counter = counter as i128;
    let frequency = frequency as i128;
    let value = counter
        .checked_mul(HUNDRED_NS_PER_SECOND)
        .ok_or_else(|| io::Error::other("QPC conversion overflow"))?
        / frequency;

    i64::try_from(value).map_err(|_| io::Error::other("QPC timestamp conversion overflow"))
}

fn precise_utc_filetime_100ns_now() -> i128 {
    // SAFETY: The API returns current UTC FILETIME by value with no pointer preconditions.
    let filetime = unsafe { GetSystemTimePreciseAsFileTime() };
    filetime_to_i128(filetime)
}

fn filetime_to_i128(filetime: FILETIME) -> i128 {
    ((filetime.dwHighDateTime as i128) << 32) | filetime.dwLowDateTime as i128
}

fn format_filetime_utc_millis(filetime_100ns: i128) -> String {
    let unix_millis = (filetime_100ns - FILETIME_UNIX_EPOCH_100NS).div_euclid(10_000);
    format_unix_millis_utc(unix_millis)
}

fn format_unix_millis_utc(unix_millis: i128) -> String {
    let unix_seconds = unix_millis.div_euclid(1_000);
    let millis = unix_millis.rem_euclid(1_000) as u32;

    let days = unix_seconds.div_euclid(86_400) as i64;
    let seconds_of_day = unix_seconds.rem_euclid(86_400) as u32;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
        year, month, day, hour, minute, second, millis
    )
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };

    (year as i32, month as u32, day as u32)
}

fn windows_error_to_io(error: windows::core::Error) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_unix_epoch_millis() {
        assert_eq!(format_unix_millis_utc(0), "1970-01-01T00:00:00.000");
    }

    #[test]
    fn format_known_utc_millis() {
        assert_eq!(
            format_unix_millis_utc(86_401_234),
            "1970-01-02T00:00:01.234"
        );
    }

    #[test]
    fn reconstruct_utc_from_system_relative_delta() {
        let anchor = CaptureTimeMapper {
            system_relative_anchor_100ns: 1_000_000,
            utc_anchor_filetime_100ns: FILETIME_UNIX_EPOCH_100NS + 5_000 * 10_000,
        };

        assert_eq!(
            anchor.utc_timestamp_for_system_relative(1_000_000 + 20_000),
            "1970-01-01T00:00:05.002"
        );
    }

    #[test]
    fn frame_decimator_emits_on_interval() {
        let mut decimator = FrameDecimator::new();

        assert!(decimator.should_emit(1_000));
        assert!(!decimator.should_emit(1_000 + FRAME_DURATION_100NS / 2));
        assert!(decimator.should_emit(1_000 + FRAME_DURATION_100NS));
        assert!(decimator.should_emit(1_000 + FRAME_DURATION_100NS * 2 + 50));
    }
}
