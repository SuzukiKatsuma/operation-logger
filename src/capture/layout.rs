use super::config::{OUTPUT_HEIGHT, OUTPUT_WIDTH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OutputLayout {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(super) fn fit_to_360p(src_width: u32, src_height: u32) -> OutputLayout {
    if src_width == 0 || src_height == 0 {
        return OutputLayout {
            x: 0,
            y: 0,
            width: OUTPUT_WIDTH,
            height: OUTPUT_HEIGHT,
        };
    }

    let width_limited_height = OUTPUT_WIDTH as u64 * src_height as u64 / src_width as u64;
    let (width, height) = if width_limited_height <= OUTPUT_HEIGHT as u64 {
        (OUTPUT_WIDTH, width_limited_height.max(1) as u32)
    } else {
        let width = OUTPUT_HEIGHT as u64 * src_width as u64 / src_height as u64;
        (width.max(1) as u32, OUTPUT_HEIGHT)
    };

    OutputLayout {
        x: (OUTPUT_WIDTH - width) / 2,
        y: (OUTPUT_HEIGHT - height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_to_360p_preserves_wide_aspect_ratio() {
        assert_eq!(
            fit_to_360p(1920, 1080),
            OutputLayout {
                x: 0,
                y: 0,
                width: 640,
                height: 360
            }
        );
    }

    #[test]
    fn fit_to_360p_letterboxes_tall_content() {
        assert_eq!(
            fit_to_360p(1080, 1920),
            OutputLayout {
                x: 219,
                y: 0,
                width: 202,
                height: 360
            }
        );
    }

    #[test]
    fn fit_to_360p_centers_square_content() {
        assert_eq!(
            fit_to_360p(1000, 1000),
            OutputLayout {
                x: 140,
                y: 0,
                width: 360,
                height: 360
            }
        );
    }
}
