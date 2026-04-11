#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ClientPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MouseButton {
    Left,
    Right,
    WheelVertical,
    WheelHorizontal,
}

impl MouseButton {
    pub(super) fn as_csv_value(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::WheelVertical => "wheel_v",
            Self::WheelHorizontal => "wheel_h",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MouseInputKind {
    Down,
    Up,
    Wheel,
}

impl MouseInputKind {
    pub(super) fn as_csv_value(self) -> &'static str {
        match self {
            Self::Down => "mousedown",
            Self::Up => "mouseup",
            Self::Wheel => "wheel",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MouseInputEvent {
    pub position: ClientPoint,
    pub button: MouseButton,
    pub kind: MouseInputKind,
    pub delta: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MouseMoveEvent {
    pub position: ClientPoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RawMouseEventKind {
    Move,
    Input {
        button: MouseButton,
        kind: MouseInputKind,
        delta: i32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RawMouseEvent {
    pub screen_position: ScreenPoint,
    pub kind: RawMouseEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResolvedMouseEvent {
    Move(MouseMoveEvent),
    Input(MouseInputEvent),
}
