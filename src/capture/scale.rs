use super::config::{BYTES_PER_PIXEL_BGRA, OUTPUT_HEIGHT, OUTPUT_WIDTH};
use super::layout::fit_to_360p;

pub(super) trait FrameScaler {
    fn scale_to_output(
        &self,
        src: &[u8],
        src_width: u32,
        src_height: u32,
        src_row_pitch: usize,
    ) -> Vec<u8>;
}

pub(super) struct CpuBgraScaler;

impl FrameScaler for CpuBgraScaler {
    fn scale_to_output(
        &self,
        src: &[u8],
        src_width: u32,
        src_height: u32,
        src_row_pitch: usize,
    ) -> Vec<u8> {
        let mut dst = vec![0u8; (OUTPUT_WIDTH * OUTPUT_HEIGHT) as usize * BYTES_PER_PIXEL_BGRA];
        if src_width == 0 || src_height == 0 {
            return dst;
        }

        let layout = fit_to_360p(src_width, src_height);

        for y in 0..layout.height {
            let src_y = map_src_y(y, layout.height, src_height);
            for x in 0..layout.width {
                let src_x = (x as u64 * src_width as u64 / layout.width as u64) as usize;
                let src_index = src_y * src_row_pitch + src_x * BYTES_PER_PIXEL_BGRA;
                let dst_x = (layout.x + x) as usize;
                let dst_y = (layout.y + y) as usize;
                let dst_index = (dst_y * OUTPUT_WIDTH as usize + dst_x) * BYTES_PER_PIXEL_BGRA;
                if src_index + BYTES_PER_PIXEL_BGRA <= src.len() {
                    dst[dst_index..dst_index + BYTES_PER_PIXEL_BGRA]
                        .copy_from_slice(&src[src_index..src_index + BYTES_PER_PIXEL_BGRA]);
                }
            }
        }

        dst
    }
}

fn map_src_y(dst_y: u32, mapped_height: u32, src_height: u32) -> usize {
    let normalized = (dst_y as u64 * src_height as u64 / mapped_height as u64) as usize;
    // WGC -> staging readback is treated as bottom-up rows for this path.
    src_height as usize - 1 - normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_frame_keeps_output_size() {
        let src = vec![255u8; 4 * 4 * BYTES_PER_PIXEL_BGRA];
        let scaled = CpuBgraScaler.scale_to_output(&src, 4, 4, 4 * BYTES_PER_PIXEL_BGRA);
        assert_eq!(
            scaled.len(),
            (OUTPUT_WIDTH * OUTPUT_HEIGHT) as usize * BYTES_PER_PIXEL_BGRA
        );
    }

    #[test]
    fn scale_handles_row_pitch_padding() {
        let src_row_pitch = 8 * BYTES_PER_PIXEL_BGRA;
        let src = vec![128u8; src_row_pitch * 4];
        let scaled = CpuBgraScaler.scale_to_output(&src, 4, 4, src_row_pitch);
        assert_eq!(
            scaled.len(),
            (OUTPUT_WIDTH * OUTPUT_HEIGHT) as usize * BYTES_PER_PIXEL_BGRA
        );
    }

    #[test]
    fn scale_zero_source_dimensions_returns_black_frame() {
        let src = vec![255u8; BYTES_PER_PIXEL_BGRA];
        let scaled = CpuBgraScaler.scale_to_output(&src, 0, 0, BYTES_PER_PIXEL_BGRA);
        assert_eq!(
            scaled.len(),
            (OUTPUT_WIDTH * OUTPUT_HEIGHT) as usize * BYTES_PER_PIXEL_BGRA
        );
        assert!(scaled.iter().all(|value| *value == 0));
    }

    #[test]
    fn scale_flips_bottom_up_source_to_top_down_output() {
        let src_width = 2;
        let src_height = 2;
        let src_row_pitch = src_width as usize * BYTES_PER_PIXEL_BGRA;

        // Bottom-up input rows:
        // row0 (memory first): bottom row => blue (B=200)
        // row1: top row => red (R=150)
        let src = vec![
            200, 0, 0, 255, 200, 0, 0, 255, 0, 0, 150, 255, 0, 0, 150, 255,
        ];
        let scaled = CpuBgraScaler.scale_to_output(&src, src_width, src_height, src_row_pitch);
        let layout = fit_to_360p(src_width, src_height);

        let top_left = ((layout.y as usize * OUTPUT_WIDTH as usize) + layout.x as usize)
            * BYTES_PER_PIXEL_BGRA;
        let bottom_left = (((layout.y + layout.height - 1) as usize * OUTPUT_WIDTH as usize)
            + layout.x as usize)
            * BYTES_PER_PIXEL_BGRA;

        assert_eq!(scaled[top_left], 0);
        assert_eq!(scaled[top_left + 2], 150);
        assert_eq!(scaled[bottom_left], 200);
        assert_eq!(scaled[bottom_left + 2], 0);
    }
}
