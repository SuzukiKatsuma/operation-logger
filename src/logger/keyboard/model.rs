#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct KeyboardKeyId {
    pub virtual_key: u32,
    pub scan_code: u32,
    pub is_extended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyboardInputKind {
    Down,
    Up,
}

impl KeyboardInputKind {
    pub(super) fn as_csv_value(self) -> &'static str {
        match self {
            Self::Down => "keydown",
            Self::Up => "keyup",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KeyboardInputEvent {
    pub virtual_key: u32,
    pub scan_code: u32,
    pub key_name: String,
    pub kind: KeyboardInputKind,
    pub is_injected: bool,
}

impl KeyboardInputEvent {
    pub(super) fn from_raw(raw: RawKeyboardEvent) -> Self {
        Self {
            virtual_key: raw.key.virtual_key,
            scan_code: raw.key.scan_code,
            key_name: key_name(raw.key),
            kind: raw.kind,
            is_injected: raw.is_injected,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RawKeyboardEvent {
    pub key: KeyboardKeyId,
    pub kind: KeyboardInputKind,
    pub is_injected: bool,
}

pub(super) fn key_name(key: KeyboardKeyId) -> String {
    if let Some(name) = known_key_name(key) {
        return name.to_string();
    }

    windows_key_name(key).unwrap_or_else(|| format!("VK_{:02X}", key.virtual_key))
}

fn known_key_name(key: KeyboardKeyId) -> Option<&'static str> {
    match (key.virtual_key, key.scan_code, key.is_extended) {
        (0xA0, _, _) => Some("LShift"),
        (0xA1, _, _) => Some("RShift"),
        (0x10, 0x2A, _) => Some("LShift"),
        (0x10, 0x36, _) => Some("RShift"),
        (0xA2, _, _) => Some("LCtrl"),
        (0xA3, _, _) => Some("RCtrl"),
        (0x11, 0x1D, false) => Some("LCtrl"),
        (0x11, 0x1D, true) => Some("RCtrl"),
        (0xA4, _, _) => Some("LAlt"),
        (0xA5, _, _) => Some("RAlt"),
        (0x12, 0x38, false) => Some("LAlt"),
        (0x12, 0x38, true) => Some("RAlt"),
        (0x20, _, _) => Some("Space"),
        (0x1B, _, _) => Some("Esc"),
        (0x0D, _, _) => Some("Enter"),
        (0x09, _, _) => Some("Tab"),
        (0x08, _, _) => Some("Backspace"),
        _ => None,
    }
}

fn windows_key_name(key: KeyboardKeyId) -> Option<String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyNameTextW;

    if key.scan_code == 0 {
        return None;
    }

    let mut lparam = (key.scan_code << 16) as i32;
    if key.is_extended {
        lparam |= 1 << 24;
    }

    let mut buffer = [0u16; 64];
    // SAFETY: The buffer is valid for writes of its own length. lparam is built from
    // the scan-code/extended-key bits expected by GetKeyNameTextW.
    let len = unsafe { GetKeyNameTextW(lparam, &mut buffer) };
    if len <= 0 {
        return None;
    }

    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}
