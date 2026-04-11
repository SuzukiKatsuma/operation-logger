use std::collections::BTreeSet;

use super::model::ControllerSnapshot;

const ANALOG_NAMES: [&str; 6] = [
    "axis_left_x",
    "axis_left_y",
    "axis_right_x",
    "axis_right_y",
    "trigger_left",
    "trigger_right",
];

const DUALSENSE_USB_REPORT_ID: u8 = 0x01;
const DUALSENSE_USB_MIN_REPORT_LEN: usize = 11;
const DUALSENSE_USB_ANALOG_OFFSET: usize = 1;
const DUALSENSE_USB_BUTTON_OFFSET: usize = 8;

const DUALSENSE_BLUETOOTH_REPORT_ID: u8 = 0x31;
const DUALSENSE_BLUETOOTH_MIN_REPORT_LEN: usize = 12;
const DUALSENSE_BLUETOOTH_ANALOG_OFFSET: usize = 2;
const DUALSENSE_BLUETOOTH_BUTTON_OFFSET: usize = 9;

const DPAD_VALUE_MASK: u8 = 0x0f;
const DPAD_NEUTRAL: u8 = 0x08;
const FIRST_BUTTON_BIT: u8 = 4;
const LAST_BUTTON_BIT: u8 = 7;
const FIRST_NAMED_BUTTON: u8 = 1;
const NEXT_BUTTON_BYTE_COUNT: usize = 2;
const BITS_PER_BYTE: u8 = 8;

pub(super) struct HidMapper;

impl HidMapper {
    pub(super) fn new() -> Self {
        Self
    }

    pub(super) fn map_report(&mut self, report: &[u8]) -> Option<ControllerSnapshot> {
        let layout = ReportLayout::from_report(report)?;
        if report.len() <= layout.button_offset {
            return None;
        }

        let mut analog_values = Vec::new();

        for (index, name) in ANALOG_NAMES.iter().enumerate() {
            if let Some(value) = report.get(layout.analog_offset + index) {
                analog_values.push(((*name).to_string(), i32::from(*value)));
            }
        }

        let pressed_buttons = pressed_buttons(report, layout.button_offset);

        Some(ControllerSnapshot {
            pressed_buttons,
            analog_values,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReportLayout {
    analog_offset: usize,
    button_offset: usize,
}

impl ReportLayout {
    fn from_report(report: &[u8]) -> Option<Self> {
        let report_id = report.first().copied()?;

        match report_id {
            DUALSENSE_USB_REPORT_ID if report.len() >= DUALSENSE_USB_MIN_REPORT_LEN => Some(Self {
                analog_offset: DUALSENSE_USB_ANALOG_OFFSET,
                button_offset: DUALSENSE_USB_BUTTON_OFFSET,
            }),
            DUALSENSE_BLUETOOTH_REPORT_ID if report.len() >= DUALSENSE_BLUETOOTH_MIN_REPORT_LEN => {
                Some(Self {
                    analog_offset: DUALSENSE_BLUETOOTH_ANALOG_OFFSET,
                    button_offset: DUALSENSE_BLUETOOTH_BUTTON_OFFSET,
                })
            }
            _ => None,
        }
    }
}

fn pressed_buttons(report: &[u8], offset: usize) -> BTreeSet<String> {
    let mut buttons = BTreeSet::new();
    let Some(first) = report.get(offset).copied() else {
        return buttons;
    };

    match first & DPAD_VALUE_MASK {
        0 => {
            buttons.insert("dpad_up".to_string());
        }
        1 => {
            buttons.insert("dpad_up".to_string());
            buttons.insert("dpad_right".to_string());
        }
        2 => {
            buttons.insert("dpad_right".to_string());
        }
        3 => {
            buttons.insert("dpad_down".to_string());
            buttons.insert("dpad_right".to_string());
        }
        4 => {
            buttons.insert("dpad_down".to_string());
        }
        5 => {
            buttons.insert("dpad_down".to_string());
            buttons.insert("dpad_left".to_string());
        }
        6 => {
            buttons.insert("dpad_left".to_string());
        }
        7 => {
            buttons.insert("dpad_up".to_string());
            buttons.insert("dpad_left".to_string());
        }
        DPAD_NEUTRAL..=u8::MAX => {}
    }

    for bit in FIRST_BUTTON_BIT..=LAST_BUTTON_BIT {
        if first & (1 << bit) != 0 {
            let button_number = FIRST_NAMED_BUTTON + (bit - FIRST_BUTTON_BIT);
            buttons.insert(format!("button_{button_number:02}"));
        }
    }

    let mut next_button = FIRST_NAMED_BUTTON + (LAST_BUTTON_BIT - FIRST_BUTTON_BIT) + 1;
    for byte in report.iter().skip(offset + 1).take(NEXT_BUTTON_BYTE_COUNT) {
        for bit in 0..BITS_PER_BYTE {
            if byte & (1 << bit) != 0 {
                buttons.insert(format!("button_{next_button:02}"));
            }
            next_button += 1;
        }
    }

    buttons
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_dualsense_usb_style_report_to_generic_names() {
        let mut mapper = HidMapper::new();
        let report = [
            DUALSENSE_USB_REPORT_ID,
            10,
            20,
            30,
            40,
            50,
            60,
            0,
            0b0011_0010,
            0b0000_0001,
            0,
        ];

        let snapshot = mapper.map_report(&report).unwrap();

        assert!(snapshot.pressed_buttons.contains("dpad_right"));
        assert!(snapshot.pressed_buttons.contains("button_01"));
        assert!(snapshot.pressed_buttons.contains("button_02"));
        assert!(snapshot.pressed_buttons.contains("button_05"));
        assert!(
            snapshot
                .analog_values
                .contains(&("axis_left_x".to_string(), 10))
        );
        assert!(
            snapshot
                .analog_values
                .contains(&("trigger_right".to_string(), 60))
        );
    }

    #[test]
    fn maps_dualsense_bluetooth_style_report_to_generic_names() {
        let mut mapper = HidMapper::new();
        let report = [
            DUALSENSE_BLUETOOTH_REPORT_ID,
            0,
            11,
            21,
            31,
            41,
            51,
            61,
            0,
            0b0101_1000,
            0,
            0b0000_0010,
        ];

        let snapshot = mapper.map_report(&report).unwrap();

        assert!(!snapshot.pressed_buttons.contains("dpad_up"));
        assert!(snapshot.pressed_buttons.contains("button_01"));
        assert!(snapshot.pressed_buttons.contains("button_03"));
        assert!(snapshot.pressed_buttons.contains("button_14"));
        assert!(
            snapshot
                .analog_values
                .contains(&("axis_left_x".to_string(), 11))
        );
        assert!(
            snapshot
                .analog_values
                .contains(&("trigger_right".to_string(), 61))
        );
    }

    #[test]
    fn rejects_unknown_reports_instead_of_guessing_layout() {
        let mut mapper = HidMapper::new();
        let report = [0x99, 10, 20, 30, 40, 50, 60, 0, 0b1111_1111, 0b1111_1111, 0];

        assert!(mapper.map_report(&report).is_none());
    }

    #[test]
    fn rejects_short_dualsense_reports() {
        let mut mapper = HidMapper::new();
        let report = [0x01, 10, 20, 30];

        assert!(mapper.map_report(&report).is_none());
    }

    #[test]
    fn rejects_empty_reports() {
        let mut mapper = HidMapper::new();

        assert!(mapper.map_report(&[]).is_none());
    }
}
