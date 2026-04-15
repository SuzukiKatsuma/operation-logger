use std::collections::{BTreeSet, HashMap};

// Approximates Unity Input System's defaultDeadzoneMin (0.125) for 8-bit (0-255) stick data.
// We treat absolute distance from center (128) <= 16 as deadzone (practical approximation, not a strict physical model).
const STICK_DEADZONE_THRESHOLD: i32 = 16;
// Uses XInput's XINPUT_GAMEPAD_TRIGGER_THRESHOLD (30) directly for trigger deadzone.
const TRIGGER_DEADZONE_THRESHOLD: i32 = 30;
const STICK_CENTER_VALUE: i32 = 128;
const ANALOG_MIN_VALUE: i32 = 0;
const ANALOG_MAX_VALUE: i32 = 255;
const ANALOG_QUANTIZATION_BINS: i32 = 32;
const ANALOG_QUANTIZATION_BIN_SIZE: i32 =
    (ANALOG_MAX_VALUE - ANALOG_MIN_VALUE + 1) / ANALOG_QUANTIZATION_BINS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ControllerButtonEvent {
    pub device_id: String,
    pub button: String,
    pub kind: ControllerButtonEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ControllerButtonEventKind {
    Down,
    Up,
}

impl ControllerButtonEventKind {
    pub(super) fn as_csv_value(self) -> &'static str {
        match self {
            Self::Down => "keydown",
            Self::Up => "keyup",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ControllerAnalogEvent {
    pub device_id: String,
    pub control: String,
    pub value: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ControllerSnapshot {
    pub pressed_buttons: BTreeSet<String>,
    pub analog_values: Vec<(String, i32)>,
}

#[derive(Debug, Default)]
pub(super) struct ControllerState {
    pressed_buttons: BTreeSet<String>,
    analog_values: HashMap<String, i32>,
}

#[derive(Debug, Default)]
pub(super) struct ControllerEvents {
    pub button_events: Vec<ControllerButtonEvent>,
    pub analog_events: Vec<ControllerAnalogEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalogControlKind {
    Stick,
    Trigger,
    Other,
}

impl AnalogControlKind {
    fn from_control(control: &str) -> Self {
        match control {
            "axis_left_x" | "axis_left_y" | "axis_right_x" | "axis_right_y" => Self::Stick,
            "trigger_left" | "trigger_right" => Self::Trigger,
            _ => Self::Other,
        }
    }
}

fn apply_deadzone(control: &str, raw_value: i32) -> i32 {
    match AnalogControlKind::from_control(control) {
        AnalogControlKind::Stick => {
            if (raw_value - STICK_CENTER_VALUE).abs() <= STICK_DEADZONE_THRESHOLD {
                // Stick neutral input maps to center (128), so deadzone values snap back to center.
                STICK_CENTER_VALUE
            } else {
                raw_value
            }
        }
        AnalogControlKind::Trigger => {
            if raw_value <= TRIGGER_DEADZONE_THRESHOLD {
                ANALOG_MIN_VALUE
            } else {
                raw_value
            }
        }
        AnalogControlKind::Other => raw_value,
    }
}

fn quantize_analog_value(value: i32) -> i32 {
    let clamped = value.clamp(ANALOG_MIN_VALUE, ANALOG_MAX_VALUE);
    if clamped == ANALOG_MAX_VALUE {
        return ANALOG_MAX_VALUE;
    }

    (clamped / ANALOG_QUANTIZATION_BIN_SIZE) * ANALOG_QUANTIZATION_BIN_SIZE
}

fn preprocess_analog_value(control: &str, raw_value: i32) -> i32 {
    quantize_analog_value(apply_deadzone(control, raw_value))
}

impl ControllerState {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn diff(
        &mut self,
        device_id: &str,
        snapshot: ControllerSnapshot,
    ) -> ControllerEvents {
        let mut events = ControllerEvents::default();

        for button in snapshot
            .pressed_buttons
            .difference(&self.pressed_buttons)
            .cloned()
        {
            events.button_events.push(ControllerButtonEvent {
                device_id: device_id.to_string(),
                button,
                kind: ControllerButtonEventKind::Down,
            });
        }

        for button in self
            .pressed_buttons
            .difference(&snapshot.pressed_buttons)
            .cloned()
        {
            events.button_events.push(ControllerButtonEvent {
                device_id: device_id.to_string(),
                button,
                kind: ControllerButtonEventKind::Up,
            });
        }

        let mut next_analog_values = HashMap::with_capacity(snapshot.analog_values.len());

        for (control, raw_value) in &snapshot.analog_values {
            let processed_value = preprocess_analog_value(control, *raw_value);

            if self.analog_values.get(control) != Some(&processed_value) {
                events.analog_events.push(ControllerAnalogEvent {
                    device_id: device_id.to_string(),
                    control: control.clone(),
                    value: processed_value,
                });
            }

            next_analog_values.insert(control.clone(), processed_value);
        }

        self.pressed_buttons = snapshot.pressed_buttons;
        self.analog_values = next_analog_values;

        events
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RawControllerReport {
    pub device_handle: isize,
    pub report: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_button_edges_and_changed_analog_values() {
        let mut state = ControllerState::new();
        let first = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::from(["button_01".to_string()]),
                analog_values: vec![("axis_x".to_string(), 127)],
            },
        );

        assert_eq!(first.button_events.len(), 1);
        assert_eq!(first.button_events[0].kind, ControllerButtonEventKind::Down);
        assert_eq!(first.analog_events.len(), 1);
        assert_eq!(first.analog_events[0].value, 120);

        let second = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("axis_x".to_string(), 127)],
            },
        );

        assert_eq!(second.button_events.len(), 1);
        assert_eq!(second.button_events[0].kind, ControllerButtonEventKind::Up);
        assert!(second.analog_events.is_empty());
    }

    #[test]
    fn does_not_emit_events_when_state_is_unchanged() {
        let mut state = ControllerState::new();
        let snapshot = ControllerSnapshot {
            pressed_buttons: BTreeSet::from(["button_01".to_string(), "dpad_up".to_string()]),
            analog_values: vec![("axis_left_x".to_string(), 128)],
        };

        let first = state.diff("rawhid_0001", snapshot.clone());
        assert_eq!(first.button_events.len(), 2);
        assert_eq!(first.analog_events.len(), 1);

        let second = state.diff("rawhid_0001", snapshot);
        assert!(second.button_events.is_empty());
        assert!(second.analog_events.is_empty());
    }

    #[test]
    fn emits_only_changed_analog_values() {
        let mut state = ControllerState::new();
        let _ = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![
                    ("axis_left_x".to_string(), 128),
                    ("axis_left_y".to_string(), 200),
                ],
            },
        );

        let events = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![
                    ("axis_left_x".to_string(), 128),
                    ("axis_left_y".to_string(), 208),
                ],
            },
        );

        assert!(events.button_events.is_empty());
        assert_eq!(events.analog_events.len(), 1);
        assert_eq!(events.analog_events[0].control, "axis_left_y");
        assert_eq!(events.analog_events[0].value, 208);
    }

    #[test]
    fn emits_down_and_up_edges_for_multiple_buttons() {
        let mut state = ControllerState::new();
        let _ = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::from(["button_01".to_string(), "button_02".to_string()]),
                analog_values: vec![],
            },
        );

        let events = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::from(["button_02".to_string(), "button_03".to_string()]),
                analog_values: vec![],
            },
        );

        assert_eq!(events.button_events.len(), 2);
        assert!(events.button_events.iter().any(|event| {
            event.button == "button_01" && event.kind == ControllerButtonEventKind::Up
        }));
        assert!(events.button_events.iter().any(|event| {
            event.button == "button_03" && event.kind == ControllerButtonEventKind::Down
        }));
    }

    #[test]
    fn does_not_emit_stick_event_for_small_center_movement_within_deadzone() {
        let mut state = ControllerState::new();
        let _ = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("axis_left_x".to_string(), 128)],
            },
        );

        let events = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("axis_left_x".to_string(), 140)],
            },
        );

        assert!(events.analog_events.is_empty());
    }

    #[test]
    fn does_not_emit_trigger_event_for_small_value_within_deadzone() {
        let mut state = ControllerState::new();
        let _ = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("trigger_left".to_string(), 0)],
            },
        );

        let events = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("trigger_left".to_string(), 20)],
            },
        );

        assert!(events.analog_events.is_empty());
    }

    #[test]
    fn does_not_emit_event_when_values_stay_in_same_quantized_bin() {
        let mut state = ControllerState::new();
        let _ = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("axis_x".to_string(), 65)],
            },
        );

        let events = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("axis_x".to_string(), 71)],
            },
        );

        assert!(events.analog_events.is_empty());
    }

    #[test]
    fn emits_event_when_value_moves_to_different_quantized_bin() {
        let mut state = ControllerState::new();
        let _ = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("axis_x".to_string(), 71)],
            },
        );

        let events = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("axis_x".to_string(), 72)],
            },
        );

        assert_eq!(events.analog_events.len(), 1);
        assert_eq!(events.analog_events[0].value, 72);
    }

    #[test]
    fn emits_trigger_event_when_value_exits_deadzone() {
        let mut state = ControllerState::new();
        let _ = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("trigger_right".to_string(), 10)],
            },
        );

        let events = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("trigger_right".to_string(), 80)],
            },
        );

        assert_eq!(events.analog_events.len(), 1);
        assert_eq!(events.analog_events[0].value, 80);
    }
}
