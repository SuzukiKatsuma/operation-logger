use std::collections::{BTreeSet, HashMap};

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
    pub raw_value: i32,
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

        for (control, raw_value) in &snapshot.analog_values {
            if self.analog_values.get(control) != Some(raw_value) {
                events.analog_events.push(ControllerAnalogEvent {
                    device_id: device_id.to_string(),
                    control: control.clone(),
                    raw_value: *raw_value,
                });
            }
        }

        self.pressed_buttons = snapshot.pressed_buttons;
        self.analog_values = snapshot.analog_values.into_iter().collect();

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
                analog_values: vec![("axis_x".to_string(), 128)],
            },
        );

        assert_eq!(first.button_events.len(), 1);
        assert_eq!(first.button_events[0].kind, ControllerButtonEventKind::Down);
        assert_eq!(first.analog_events.len(), 1);

        let second = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![("axis_x".to_string(), 128)],
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
                    ("axis_left_y".to_string(), 129),
                ],
            },
        );

        let events = state.diff(
            "rawhid_0001",
            ControllerSnapshot {
                pressed_buttons: BTreeSet::new(),
                analog_values: vec![
                    ("axis_left_x".to_string(), 128),
                    ("axis_left_y".to_string(), 130),
                ],
            },
        );

        assert!(events.button_events.is_empty());
        assert_eq!(events.analog_events.len(), 1);
        assert_eq!(events.analog_events[0].control, "axis_left_y");
        assert_eq!(events.analog_events[0].raw_value, 130);
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
}
