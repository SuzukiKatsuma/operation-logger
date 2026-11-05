//! CFR scheduling independent of WGC, pixel storage, and the encoder.
use std::io;

use super::config::FRAME_DURATION_100NS;

#[derive(Debug, Default, serde::Serialize)]
pub(super) struct TimelineStats {
    pub(super) output_frame_count: u64,
    pub(super) adopted_frame_count: u64,
    pub(super) duplicated_frame_count: u64,
    pub(super) initial_padding_frame_count: u64,
    pub(super) stale_frame_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: i64 = 10_000_000;

    fn simulate(times: &[i64], stop: i64) -> (FrameTimeline<i64>, Vec<(u64, i64)>) {
        let mut timeline = FrameTimeline::new(0);
        let mut frames = Vec::new();
        let mut emit = |index, time: &i64| {
            frames.push((index, *time));
            Ok(())
        };
        for &time in times {
            timeline.accept(time, time, &mut emit).unwrap();
        }
        timeline.advance_until(stop, emit).unwrap();
        (timeline, frames)
    }

    #[test]
    fn ten_fps_for_ten_seconds() {
        let times: Vec<_> = (0..100).map(|i| i * FRAME_DURATION_100NS).collect();
        let (timeline, frames) = simulate(&times, 10 * SECOND);
        assert_eq!(frames.len(), 100);
        assert_eq!(timeline.stats.adopted_frame_count, 100);
        assert_eq!(timeline.stats.duplicated_frame_count, 0);
        for (index, &(output_index, time)) in frames.iter().enumerate() {
            assert_eq!(output_index, index as u64);
            assert_eq!(time, index as i64 * FRAME_DURATION_100NS);
        }
    }

    #[test]
    fn five_fps_still_produces_ten_seconds() {
        let times: Vec<_> = (0..50).map(|i| i * 2 * FRAME_DURATION_100NS).collect();
        let (timeline, frames) = simulate(&times, 10 * SECOND);
        assert_eq!(frames.len(), 100);
        assert_eq!(timeline.stats.adopted_frame_count, 50);
        assert_eq!(timeline.stats.duplicated_frame_count, 50);
        for pair in frames.chunks_exact(2) {
            assert_eq!(pair[0].1, pair[1].1);
        }
    }

    #[test]
    fn one_second_stall_and_stop_are_padded() {
        let (timeline, frames) = simulate(&[0, SECOND, SECOND + FRAME_DURATION_100NS], 2 * SECOND);
        assert_eq!(frames.len(), 20);
        assert!(frames[..10].iter().all(|(_, time)| *time == 0));
        assert_eq!(frames[10].1, SECOND);
        assert!(
            frames[11..]
                .iter()
                .all(|(_, time)| *time == SECOND + FRAME_DURATION_100NS)
        );
        assert_eq!(timeline.stats.adopted_frame_count, 3);
        assert_eq!(timeline.stats.duplicated_frame_count, 17);
    }

    #[test]
    fn irregular_intervals_preserve_observations() {
        let d = FRAME_DURATION_100NS;
        let (timeline, frames) = simulate(&[0, d, 4 * d], 6 * d);
        assert_eq!(
            frames,
            vec![(0, 0), (1, d), (2, d), (3, d), (4, 4 * d), (5, 4 * d)]
        );
        assert_eq!(timeline.stats.adopted_frame_count, 3);
        assert_eq!(timeline.stats.duplicated_frame_count, 3);
    }

    #[test]
    fn new_frame_never_moves_back_to_an_earlier_slot() {
        let d = FRAME_DURATION_100NS;
        let (_, frames) = simulate(&[0, d + 1, 3 * d - 1], 4 * d);
        assert_eq!(frames, vec![(0, 0), (1, 0), (2, d + 1), (3, 3 * d - 1)]);
        for &(index, captured_at) in &frames {
            assert!(captured_at <= index as i64 * d);
        }
    }

    #[test]
    fn idle_ticks_progress_without_new_frames() {
        let mut timeline = FrameTimeline::new(0);
        let mut frames = Vec::new();
        timeline.accept(0, 42, |_, _| Ok(())).unwrap();
        for tick in 1..=10 {
            timeline
                .advance_until(tick * FRAME_DURATION_100NS, |index, frame| {
                    frames.push((index, *frame));
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(frames.len(), 10);
        assert_eq!(frames.last(), Some(&(9, 42)));
    }

    #[test]
    fn initial_backfill_keeps_first_real_timestamp() {
        let d = FRAME_DURATION_100NS;
        let (timeline, frames) = simulate(&[2 * d + 1, 4 * d], 6 * d);
        assert_eq!(
            frames.iter().map(|f| f.1).collect::<Vec<_>>(),
            vec![2 * d + 1, 2 * d + 1, 2 * d + 1, 2 * d + 1, 4 * d, 4 * d]
        );
        assert_eq!(timeline.first_frame_time_100ns, Some(2 * d + 1));
        assert_eq!(timeline.stats.initial_padding_frame_count, 3);
        assert_eq!(
            timeline.stats.adopted_frame_count + timeline.stats.duplicated_frame_count,
            6
        );
    }

    #[test]
    fn no_observation_does_not_invent_a_capture_timestamp() {
        let (timeline, frames) = simulate(&[], SECOND);
        assert!(frames.is_empty());
        assert_eq!(timeline.first_frame_time_100ns, None);
    }

    #[test]
    fn duration_rounds_up_by_less_than_one_slot() {
        let d = FRAME_DURATION_100NS;
        for stop in [1, d - 1, d, d + 1, 10 * SECOND, 10 * SECOND + 1] {
            let (_, frames) = simulate(&[0], stop);
            let duration = frames.len() as i64 * d;
            assert!(duration >= stop);
            assert!(duration - stop < d);
        }
    }

    #[test]
    fn nonzero_origin_and_stale_observations() {
        let start = 123_456_789;
        let d = FRAME_DURATION_100NS;
        let mut timeline = FrameTimeline::new(start);
        let mut frames = Vec::new();
        let mut emit = |index, time: &i64| {
            frames.push((index, *time));
            Ok(())
        };
        timeline.accept(start - 1, start - 1, &mut emit).unwrap();
        timeline.accept(start, start, &mut emit).unwrap();
        timeline.accept(start, start, &mut emit).unwrap();
        timeline.advance_until(start + 2 * d, emit).unwrap();
        assert_eq!(frames, vec![(0, start), (1, start)]);
        assert_eq!(timeline.stats.stale_frame_count, 2);
    }

    #[test]
    fn failed_output_does_not_advance_frame_index() {
        let mut timeline = FrameTimeline::new(0);
        timeline.accept(0, (), |_, _| Ok(())).unwrap();
        assert!(
            timeline
                .advance_until(1, |_, _| Err(io::Error::other("write failed")))
                .is_err()
        );
        assert_eq!(timeline.stats.output_frame_count, 0);
        assert_eq!(timeline.stats.adopted_frame_count, 0);
    }
}

pub(super) struct FrameTimeline<T> {
    start_100ns: i64,
    current: Option<(i64, T, bool)>,
    pub(super) first_frame_time_100ns: Option<i64>,
    pub(super) stats: TimelineStats,
}

impl<T> FrameTimeline<T> {
    pub(super) fn new(start_100ns: i64) -> Self {
        Self {
            start_100ns,
            current: None,
            first_frame_time_100ns: None,
            stats: TimelineStats::default(),
        }
    }

    pub(super) fn accept(
        &mut self,
        captured_at_100ns: i64,
        frame: T,
        mut emit: impl FnMut(u64, &T) -> io::Result<()>,
    ) -> io::Result<()> {
        if captured_at_100ns < self.start_100ns
            || self
                .current
                .as_ref()
                .is_some_and(|(time, _, _)| captured_at_100ns <= *time)
        {
            self.stats.stale_frame_count += 1;
            return Ok(());
        }
        // A later observation must never replace the image in an earlier slot.
        self.advance_until(captured_at_100ns, &mut emit)?;
        self.current = Some((captured_at_100ns, frame, false));
        if self.first_frame_time_100ns.is_none() {
            self.first_frame_time_100ns = Some(captured_at_100ns);
            // Explicit initial backfill: retain the first observation's REAL time.
            self.advance_until(captured_at_100ns, emit)?;
        }
        Ok(())
    }

    /// Emit slots whose start is strictly before the boundary (stop is exclusive).
    pub(super) fn advance_until(
        &mut self,
        boundary_100ns: i64,
        mut emit: impl FnMut(u64, &T) -> io::Result<()>,
    ) -> io::Result<()> {
        let Some((captured_at, frame, used)) = self.current.as_mut() else {
            return Ok(());
        };
        loop {
            let index = self.stats.output_frame_count;
            let slot = self.start_100ns as i128 + index as i128 * FRAME_DURATION_100NS as i128;
            if slot >= boundary_100ns as i128 {
                break;
            }
            emit(index, frame)?;
            if *used {
                self.stats.duplicated_frame_count += 1;
            } else {
                self.stats.adopted_frame_count += 1;
                *used = true;
            }
            if slot < *captured_at as i128 {
                self.stats.initial_padding_frame_count += 1;
            }
            self.stats.output_frame_count += 1;
        }
        Ok(())
    }
}
